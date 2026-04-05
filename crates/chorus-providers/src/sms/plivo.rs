use async_trait::async_trait;
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
use chorus_core::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use serde::Deserialize;

pub struct PlivoSmsSender {
    auth_id: String,
    auth_token: String,
    from: Option<String>,
    http_client: reqwest::Client,
}

impl PlivoSmsSender {
    pub fn new(auth_id: String, auth_token: String, from: Option<String>) -> Self {
        Self {
            auth_id,
            auth_token,
            from,
            http_client: reqwest::Client::new(),
        }
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

        let url = format!("https://api.plivo.com/v1/Account/{}/Message/", self.auth_id);

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
            "https://api.plivo.com/v1/Account/{}/Message/{}/",
            self.auth_id, message_id
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
}
