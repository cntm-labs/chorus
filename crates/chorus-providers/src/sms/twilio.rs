use async_trait::async_trait;
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
use chorus_core::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use serde::Deserialize;

pub struct TwilioSmsSender {
    account_sid: String,
    auth_token: String,
    from: Option<String>,
    http_client: reqwest::Client,
}

impl TwilioSmsSender {
    pub fn new(account_sid: String, auth_token: String, from: Option<String>) -> Self {
        Self {
            account_sid,
            auth_token,
            from,
            http_client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct TwilioResponse {
    sid: String,
}

#[derive(Deserialize)]
struct TwilioStatusResponse {
    status: String,
}

#[async_trait]
impl SmsSender for TwilioSmsSender {
    fn provider_name(&self) -> &str {
        "twilio"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_ref().or(self.from.as_ref()).ok_or_else(|| {
            ChorusError::Validation("SMS 'from' number is required for Twilio".to_string())
        })?;

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.account_sid
        );

        let resp = self
            .http_client
            .post(&url)
            .basic_auth(&self.account_sid, Some(&self.auth_token))
            .form(&[("To", &msg.to), ("From", from), ("Body", &msg.body)])
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "twilio".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "twilio".into(),
                message: format!("API error: {}", body),
            });
        }

        let twilio_resp: TwilioResponse = resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "twilio".into(),
            message: format!("parse error: {}", e),
        })?;

        Ok(SendResult {
            message_id: twilio_resp.sid,
            provider: "twilio".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Queued,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages/{}.json",
            self.account_sid, message_id
        );

        let resp = self
            .http_client
            .get(&url)
            .basic_auth(&self.account_sid, Some(&self.auth_token))
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "twilio".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "twilio".into(),
                message: format!("status check failed: {}", body),
            });
        }

        let status_resp: TwilioStatusResponse =
            resp.json().await.map_err(|e| ChorusError::Provider {
                provider: "twilio".into(),
                message: format!("parse error: {}", e),
            })?;

        Ok(map_twilio_status(&status_resp.status))
    }
}

fn map_twilio_status(status: &str) -> DeliveryStatus {
    match status {
        "delivered" => DeliveryStatus::Delivered,
        "sent" => DeliveryStatus::Delivered,
        "failed" | "undelivered" => DeliveryStatus::Failed {
            reason: format!("twilio status: {}", status),
        },
        "queued" | "accepted" | "sending" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twilio_provider_name() {
        let sender =
            TwilioSmsSender::new("AC123".into(), "token".into(), Some("+1234567890".into()));
        assert_eq!(sender.provider_name(), "twilio");
    }

    #[tokio::test]
    async fn twilio_requires_from_number() {
        let sender = TwilioSmsSender::new("AC123".into(), "token".into(), None);
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
            map_twilio_status("delivered"),
            DeliveryStatus::Delivered
        ));
        assert!(matches!(
            map_twilio_status("sent"),
            DeliveryStatus::Delivered
        ));
    }

    #[test]
    fn map_status_failed() {
        assert!(matches!(
            map_twilio_status("failed"),
            DeliveryStatus::Failed { .. }
        ));
        assert!(matches!(
            map_twilio_status("undelivered"),
            DeliveryStatus::Failed { .. }
        ));
    }

    #[test]
    fn map_status_sent() {
        assert!(matches!(map_twilio_status("queued"), DeliveryStatus::Sent));
        assert!(matches!(
            map_twilio_status("accepted"),
            DeliveryStatus::Sent
        ));
        assert!(matches!(map_twilio_status("sending"), DeliveryStatus::Sent));
    }

    #[test]
    fn map_status_unknown_defaults_to_sent() {
        assert!(matches!(
            map_twilio_status("something_else"),
            DeliveryStatus::Sent
        ));
    }
}
