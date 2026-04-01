use async_trait::async_trait;
use chorus_core::email::EmailSender;
use chorus_core::error::ChorusError;
use chorus_core::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use lettre::message::header::ContentType;
use lettre::message::MultiPart;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

/// AWS SES Email provider using SMTP relay mode.
/// Connects to `email-smtp.{region}.amazonaws.com:587` with SES SMTP credentials.
pub struct SesEmailSender {
    from: String,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SesEmailSender {
    pub fn new(
        access_key: String,
        secret_key: String,
        region: String,
        from: String,
    ) -> Result<Self, ChorusError> {
        let host = format!("email-smtp.{}.amazonaws.com", region);
        let creds = Credentials::new(access_key, secret_key);

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
            .map_err(|e| ChorusError::Provider {
                provider: "ses".into(),
                message: format!("SMTP transport error: {}", e),
            })?
            .credentials(creds)
            .port(587)
            .build();

        Ok(Self { from, transport })
    }
}

#[async_trait]
impl EmailSender for SesEmailSender {
    fn provider_name(&self) -> &str {
        "ses"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_deref().unwrap_or(&self.from);

        let email = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| ChorusError::Validation(format!("invalid from: {}", e)))?,
            )
            .to(msg
                .to
                .parse()
                .map_err(|e| ChorusError::Validation(format!("invalid to: {}", e)))?)
            .subject(&msg.subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(msg.text_body.clone()),
                    )
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(msg.html_body.clone()),
                    ),
            )
            .map_err(|e| ChorusError::Provider {
                provider: "ses".into(),
                message: format!("email build error: {}", e),
            })?;

        self.transport
            .send(email)
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "ses".into(),
                message: format!("SMTP send error: {}", e),
            })?;

        Ok(SendResult {
            message_id: Uuid::new_v4().to_string(),
            provider: "ses".to_string(),
            channel: Channel::Email,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ses_provider_name() {
        let sender = SesEmailSender::new(
            "AKIAIOSFODNN7".into(),
            "secret".into(),
            "us-east-1".into(),
            "noreply@test.com".into(),
        )
        .unwrap();
        assert_eq!(sender.provider_name(), "ses");
    }
}
