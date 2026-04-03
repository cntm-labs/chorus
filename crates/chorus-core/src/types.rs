use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessage {
    pub to: String,
    pub body: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEmailMessage {
    pub to: String,
    pub template_slug: String,
    pub variables: serde_json::Value,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResult {
    pub message_id: String,
    pub provider: String,
    pub channel: Channel,
    pub status: DeliveryStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Sms,
    Email,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Queued,
    Sent,
    Delivered,
    Failed { reason: String },
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Channel::Sms => write!(f, "sms"),
            Channel::Email => write!(f, "email"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_display() {
        assert_eq!(Channel::Sms.to_string(), "sms");
        assert_eq!(Channel::Email.to_string(), "email");
    }

    #[test]
    fn sms_message_serializes() {
        let msg = SmsMessage {
            to: "+66812345678".to_string(),
            body: "Hello".to_string(),
            from: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["to"], "+66812345678");
    }

    #[test]
    fn delivery_status_serializes() {
        let sent = DeliveryStatus::Sent;
        let json = serde_json::to_string(&sent).unwrap();
        assert_eq!(json, "\"sent\"");

        let failed = DeliveryStatus::Failed {
            reason: "timeout".to_string(),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["failed"]["reason"], "timeout");
    }

    #[test]
    fn email_message_serializes() {
        let msg = EmailMessage {
            to: "user@example.com".to_string(),
            subject: "Hello".to_string(),
            html_body: "<p>Hi</p>".to_string(),
            text_body: "Hi".to_string(),
            from: Some("noreply@chorus.dev".to_string()),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["to"], "user@example.com");
        assert_eq!(json["subject"], "Hello");
        assert_eq!(json["from"], "noreply@chorus.dev");
    }

    #[test]
    fn template_email_message_serializes() {
        let msg = TemplateEmailMessage {
            to: "user@example.com".to_string(),
            template_slug: "otp".to_string(),
            variables: serde_json::json!({"code": "123456"}),
            from: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["template_slug"], "otp");
        assert_eq!(json["variables"]["code"], "123456");
        assert!(json["from"].is_null());
    }

    #[test]
    fn send_result_serializes() {
        let result = SendResult {
            message_id: "msg-123".to_string(),
            provider: "twilio".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Delivered,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["message_id"], "msg-123");
        assert_eq!(json["provider"], "twilio");
        assert_eq!(json["channel"], "sms");
        assert_eq!(json["status"], "delivered");
    }

    #[test]
    fn delivery_status_deserializes() {
        let sent: DeliveryStatus = serde_json::from_str("\"queued\"").unwrap();
        assert_eq!(sent, DeliveryStatus::Queued);

        let failed: DeliveryStatus =
            serde_json::from_value(serde_json::json!({"failed": {"reason": "no credit"}}))
                .unwrap();
        assert_eq!(
            failed,
            DeliveryStatus::Failed {
                reason: "no credit".into()
            }
        );
    }

    #[test]
    fn channel_serializes_snake_case() {
        let sms_json = serde_json::to_string(&Channel::Sms).unwrap();
        let email_json = serde_json::to_string(&Channel::Email).unwrap();
        assert_eq!(sms_json, "\"sms\"");
        assert_eq!(email_json, "\"email\"");

        let sms: Channel = serde_json::from_str("\"sms\"").unwrap();
        assert_eq!(sms, Channel::Sms);
    }
}
