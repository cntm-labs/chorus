use async_trait::async_trait;
use chorus::email::EmailSender;
use chorus::error::ChorusError;
use chorus::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use serde::Deserialize;

/// Mailgun email provider.
///
/// Supports US and EU regions via configurable `base_url`.
pub struct MailgunEmailSender {
    api_key: String,
    domain: String,
    from: String,
    http_client: reqwest::Client,
    base_url: String,
}

impl MailgunEmailSender {
    /// Create a new Mailgun sender with US region default.
    pub fn new(api_key: String, domain: String, from: String) -> Self {
        Self {
            api_key,
            domain,
            from,
            http_client: reqwest::Client::new(),
            base_url: "https://api.mailgun.net".into(),
        }
    }

    /// Override the base URL (for EU region or testing).
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[derive(Deserialize)]
struct MailgunResponse {
    id: String,
}

#[async_trait]
impl EmailSender for MailgunEmailSender {
    fn provider_name(&self) -> &str {
        "mailgun"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_deref().unwrap_or(&self.from);
        let url = format!("{}/v3/{}/messages", self.base_url, self.domain);

        let form = reqwest::multipart::Form::new()
            .text("from", from.to_string())
            .text("to", msg.to.clone())
            .text("subject", msg.subject.clone())
            .text("html", msg.html_body.clone())
            .text("text", msg.text_body.clone());

        let resp = self
            .http_client
            .post(&url)
            .basic_auth("api", Some(&self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("API error: {}", body),
            });
        }

        let mg_resp: MailgunResponse =
            resp.json().await.map_err(|e| ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("parse error: {}", e),
            })?;

        Ok(SendResult {
            message_id: mg_resp.id,
            provider: "mailgun".to_string(),
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
    fn mailgun_provider_name() {
        let sender = MailgunEmailSender::new(
            "key-xxx".into(),
            "mg.example.com".into(),
            "noreply@example.com".into(),
        );
        assert_eq!(sender.provider_name(), "mailgun");
    }

    #[tokio::test]
    async fn mailgun_send_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/mg.example.com/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "<msg-123>", "message": "Queued"})),
            )
            .mount(&mock_server)
            .await;

        let sender = MailgunEmailSender::new(
            "key-xxx".into(),
            "mg.example.com".into(),
            "noreply@example.com".into(),
        )
        .with_base_url(mock_server.uri());

        let msg = EmailMessage {
            to: "user@test.com".into(),
            subject: "Test".into(),
            html_body: "<p>Hi</p>".into(),
            text_body: "Hi".into(),
            from: None,
        };

        let result = sender.send(&msg).await.unwrap();
        assert_eq!(result.provider, "mailgun");
        assert_eq!(result.message_id, "<msg-123>");
        assert!(matches!(result.status, DeliveryStatus::Sent));
    }

    #[tokio::test]
    async fn mailgun_send_api_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/mg.example.com/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Forbidden"))
            .mount(&mock_server)
            .await;

        let sender = MailgunEmailSender::new(
            "bad-key".into(),
            "mg.example.com".into(),
            "noreply@example.com".into(),
        )
        .with_base_url(mock_server.uri());

        let msg = EmailMessage {
            to: "user@test.com".into(),
            subject: "Test".into(),
            html_body: "<p>Hi</p>".into(),
            text_body: "Hi".into(),
            from: None,
        };

        let err = sender.send(&msg).await.unwrap_err();
        assert!(matches!(err, ChorusError::Provider { .. }));
    }
}
