use async_trait::async_trait;
use chorus::email::EmailSender;
use chorus::error::ChorusError;
use chorus::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use lettre::message::header::ContentType;
use lettre::message::MultiPart;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

/// Universal SMTP Email provider.
/// Works with any SMTP server: Gmail, SES, Postfix, Mailgun, etc.
pub struct SmtpEmailSender {
    from: String,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpEmailSender {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        from: String,
    ) -> Result<Self, ChorusError> {
        let creds = Credentials::new(username, password);

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
            .map_err(|e| ChorusError::Provider {
                provider: "smtp".into(),
                message: format!("SMTP transport error: {}", e),
            })?
            .credentials(creds)
            .port(port)
            .build();

        Ok(Self { from, transport })
    }
}

#[async_trait]
impl EmailSender for SmtpEmailSender {
    fn provider_name(&self) -> &str {
        "smtp"
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
                provider: "smtp".into(),
                message: format!("email build error: {}", e),
            })?;

        self.transport
            .send(email)
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "smtp".into(),
                message: format!("SMTP send error: {}", e),
            })?;

        Ok(SendResult {
            message_id: Uuid::new_v4().to_string(),
            provider: "smtp".to_string(),
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
    async fn smtp_provider_name() {
        let sender = SmtpEmailSender::new(
            "smtp.gmail.com".into(),
            587,
            "user@gmail.com".into(),
            "password".into(),
            "user@gmail.com".into(),
        )
        .unwrap();
        assert_eq!(sender.provider_name(), "smtp");
    }
}
