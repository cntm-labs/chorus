use async_trait::async_trait;
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
use chorus_core::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use serde::Deserialize;

pub struct TelnyxSmsSender {
    api_key: String,
    from: Option<String>,
    http_client: reqwest::Client,
    base_url: String,
}

impl TelnyxSmsSender {
    pub fn new(api_key: String, from: Option<String>) -> Self {
        Self {
            api_key,
            from,
            http_client: reqwest::Client::new(),
            base_url: "https://api.telnyx.com".into(),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[derive(Deserialize)]
struct TelnyxResponse {
    data: TelnyxMessageData,
}

#[derive(Deserialize)]
struct TelnyxMessageData {
    id: String,
}

#[derive(Deserialize)]
struct TelnyxStatusResponse {
    data: TelnyxStatusData,
}

#[derive(Deserialize)]
struct TelnyxStatusData {
    to: Vec<TelnyxRecipientStatus>,
}

#[derive(Deserialize)]
struct TelnyxRecipientStatus {
    status: String,
}

#[async_trait]
impl SmsSender for TelnyxSmsSender {
    fn provider_name(&self) -> &str {
        "telnyx"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_ref().or(self.from.as_ref()).ok_or_else(|| {
            ChorusError::Validation("SMS 'from' number is required for Telnyx".to_string())
        })?;

        let payload = serde_json::json!({
            "from": from,
            "to": msg.to,
            "text": msg.body,
        });

        let resp = self
            .http_client
            .post(format!("{}/v2/messages", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("API error: {}", body),
            });
        }

        let telnyx_resp: TelnyxResponse = resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "telnyx".into(),
            message: format!("parse error: {}", e),
        })?;

        Ok(SendResult {
            message_id: telnyx_resp.data.id,
            provider: "telnyx".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Queued,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        let url = format!("{}/v2/messages/{}", self.base_url, message_id);

        let resp = self
            .http_client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("status check failed: {}", body),
            });
        }

        let status_resp: TelnyxStatusResponse =
            resp.json().await.map_err(|e| ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("parse error: {}", e),
            })?;

        let status = status_resp
            .data
            .to
            .first()
            .map(|r| r.status.as_str())
            .unwrap_or("unknown");

        Ok(map_telnyx_status(status))
    }
}

fn map_telnyx_status(status: &str) -> DeliveryStatus {
    match status {
        "delivered" => DeliveryStatus::Delivered,
        "sent" => DeliveryStatus::Delivered,
        "sending_failed" => DeliveryStatus::Failed {
            reason: format!("telnyx status: {}", status),
        },
        "queued" | "sending" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnyx_provider_name() {
        let sender = TelnyxSmsSender::new("fake-key".into(), Some("+1234567890".into()));
        assert_eq!(sender.provider_name(), "telnyx");
    }

    #[tokio::test]
    async fn telnyx_requires_from_number() {
        let sender = TelnyxSmsSender::new("fake-key".into(), None);
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
            map_telnyx_status("delivered"),
            DeliveryStatus::Delivered
        ));
        assert!(matches!(
            map_telnyx_status("sent"),
            DeliveryStatus::Delivered
        ));
    }

    #[test]
    fn map_status_failed() {
        assert!(matches!(
            map_telnyx_status("sending_failed"),
            DeliveryStatus::Failed { .. }
        ));
    }

    #[test]
    fn map_status_sent() {
        assert!(matches!(map_telnyx_status("queued"), DeliveryStatus::Sent));
        assert!(matches!(map_telnyx_status("sending"), DeliveryStatus::Sent));
    }

    #[test]
    fn map_status_unknown_defaults_to_sent() {
        assert!(matches!(
            map_telnyx_status("something_else"),
            DeliveryStatus::Sent
        ));
    }

    #[tokio::test]
    async fn check_status_returns_delivered() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/messages/msg-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "to": [{"status": "delivered"}]
                }
            })))
            .mount(&mock_server)
            .await;

        let sender = TelnyxSmsSender::new("fake-key".into(), Some("+1".into()))
            .with_base_url(mock_server.uri());
        let status = sender.check_status("msg-123").await.unwrap();
        assert!(matches!(status, DeliveryStatus::Delivered));
    }

    #[tokio::test]
    async fn check_status_returns_error_on_failure() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/messages/msg-999"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&mock_server)
            .await;

        let sender = TelnyxSmsSender::new("fake-key".into(), Some("+1".into()))
            .with_base_url(mock_server.uri());
        let result = sender.check_status("msg-999").await;
        assert!(matches!(result, Err(ChorusError::Provider { .. })));
    }
}
