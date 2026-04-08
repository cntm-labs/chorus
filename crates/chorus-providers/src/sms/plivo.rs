use async_trait::async_trait;
use chorus::error::ChorusError;
use chorus::sms::SmsSender;
use chorus::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use serde::Deserialize;

pub struct PlivoSmsSender {
    auth_id: String,
    auth_token: String,
    from: Option<String>,
    http_client: reqwest::Client,
    base_url: String,
}

impl PlivoSmsSender {
    pub fn new(auth_id: String, auth_token: String, from: Option<String>) -> Self {
        Self {
            auth_id,
            auth_token,
            from,
            http_client: reqwest::Client::new(),
            base_url: "https://api.plivo.com".into(),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[derive(Deserialize)]
struct PlivoResponse {
    message_uuid: Vec<String>,
}

#[derive(Deserialize)]
struct PlivoStatusResponse {
    message_state: String,
}

#[async_trait]
impl SmsSender for PlivoSmsSender {
    fn provider_name(&self) -> &str {
        "plivo"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_ref().or(self.from.as_ref()).ok_or_else(|| {
            ChorusError::Validation("SMS 'from' number is required for Plivo".to_string())
        })?;

        let url = format!("{}/v1/Account/{}/Message/", self.base_url, self.auth_id);

        let payload = serde_json::json!({
            "src": from,
            "dst": msg.to,
            "text": msg.body,
        });

        let resp = self
            .http_client
            .post(&url)
            .basic_auth(&self.auth_id, Some(&self.auth_token))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "plivo".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "plivo".into(),
                message: format!("API error: {}", body),
            });
        }

        let plivo_resp: PlivoResponse = resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "plivo".into(),
            message: format!("parse error: {}", e),
        })?;

        let message_id = plivo_resp
            .message_uuid
            .into_iter()
            .next()
            .unwrap_or_default();

        Ok(SendResult {
            message_id,
            provider: "plivo".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Queued,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        let url = format!(
            "{}/v1/Account/{}/Message/{}/",
            self.base_url, self.auth_id, message_id
        );

        let resp = self
            .http_client
            .get(&url)
            .basic_auth(&self.auth_id, Some(&self.auth_token))
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "plivo".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "plivo".into(),
                message: format!("status check failed: {}", body),
            });
        }

        let status_resp: PlivoStatusResponse =
            resp.json().await.map_err(|e| ChorusError::Provider {
                provider: "plivo".into(),
                message: format!("parse error: {}", e),
            })?;

        Ok(map_plivo_status(&status_resp.message_state))
    }
}

fn map_plivo_status(state: &str) -> DeliveryStatus {
    match state {
        "delivered" => DeliveryStatus::Delivered,
        "failed" | "rejected" => DeliveryStatus::Failed {
            reason: format!("plivo status: {}", state),
        },
        "queued" | "sent" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plivo_provider_name() {
        let sender =
            PlivoSmsSender::new("auth123".into(), "token".into(), Some("+1234567890".into()));
        assert_eq!(sender.provider_name(), "plivo");
    }

    #[tokio::test]
    async fn plivo_requires_from_number() {
        let sender = PlivoSmsSender::new("auth123".into(), "token".into(), None);
        let msg = SmsMessage {
            to: "+66812345678".into(),
            body: "Hi".into(),
            from: None,
        };
        let result = sender.send(&msg).await;
        assert!(matches!(result, Err(ChorusError::Validation(_))));
    }

    #[test]
    fn map_status_delivered() {
        assert!(matches!(
            map_plivo_status("delivered"),
            DeliveryStatus::Delivered
        ));
    }

    #[test]
    fn map_status_failed() {
        assert!(matches!(
            map_plivo_status("failed"),
            DeliveryStatus::Failed { .. }
        ));
        assert!(matches!(
            map_plivo_status("rejected"),
            DeliveryStatus::Failed { .. }
        ));
    }

    #[test]
    fn map_status_sent() {
        assert!(matches!(map_plivo_status("queued"), DeliveryStatus::Sent));
        assert!(matches!(map_plivo_status("sent"), DeliveryStatus::Sent));
    }

    #[test]
    fn map_status_unknown_defaults_to_sent() {
        assert!(matches!(
            map_plivo_status("something_else"),
            DeliveryStatus::Sent
        ));
    }

    #[tokio::test]
    async fn check_status_returns_delivered() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/Account/auth123/Message/uuid-123/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"message_state": "delivered"})),
            )
            .mount(&mock_server)
            .await;

        let sender = PlivoSmsSender::new("auth123".into(), "token".into(), Some("+1".into()))
            .with_base_url(mock_server.uri());
        let status = sender.check_status("uuid-123").await.unwrap();
        assert!(matches!(status, DeliveryStatus::Delivered));
    }

    #[tokio::test]
    async fn check_status_returns_error_on_failure() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/Account/auth123/Message/uuid-999/"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&mock_server)
            .await;

        let sender = PlivoSmsSender::new("auth123".into(), "token".into(), Some("+1".into()))
            .with_base_url(mock_server.uri());
        let result = sender.check_status("uuid-999").await;
        assert!(matches!(result, Err(ChorusError::Provider { .. })));
    }
}
