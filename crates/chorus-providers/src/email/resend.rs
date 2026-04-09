use async_trait::async_trait;
use chorus::email::EmailSender;
use chorus::error::ChorusError;
use chorus::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use serde::Deserialize;

pub struct ResendEmailSender {
    api_key: String,
    from: String,
    http_client: reqwest::Client,
}

impl ResendEmailSender {
    pub fn new(api_key: String, from: String) -> Self {
        Self {
            api_key,
            from,
            http_client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct ResendResponse {
    id: String,
}

#[async_trait]
impl EmailSender for ResendEmailSender {
    fn provider_name(&self) -> &str {
        "resend"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_deref().unwrap_or(&self.from);

        let payload = serde_json::json!({
            "from": from,
            "to": [msg.to],
            "subject": msg.subject,
            "html": msg.html_body,
            "text": msg.text_body,
        });

        let resp = self
            .http_client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "resend".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "resend".into(),
                message: format!("API error: {}", body),
            });
        }

        let resend_resp: ResendResponse = resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "resend".into(),
            message: format!("parse error: {}", e),
        })?;

        Ok(SendResult {
            message_id: resend_resp.id,
            provider: "resend".to_string(),
            channel: Channel::Email,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_provider_name() {
        let sender = ResendEmailSender::new("re_xxx".into(), "noreply@test.com".into());
        assert_eq!(sender.provider_name(), "resend");
    }
}
