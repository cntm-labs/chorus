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

        let failed = DeliveryStatus::Failed { reason: "timeout".to_string() };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["reason"], "timeout");
    }
}
