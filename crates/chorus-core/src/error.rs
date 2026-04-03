use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChorusError {
    #[error("provider error ({provider}): {message}")]
    Provider { provider: String, message: String },

    #[error("all providers failed")]
    AllProvidersFailed,

    #[error("validation error: {0}")]
    Validation(String),

    #[error("template not found: {0}")]
    TemplateNotFound(String),

    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("invalid api key")]
    InvalidApiKey,

    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_display() {
        let err = ChorusError::Provider {
            provider: "twilio".into(),
            message: "timeout".into(),
        };
        assert_eq!(err.to_string(), "provider error (twilio): timeout");
    }

    #[test]
    fn all_providers_failed_display() {
        let err = ChorusError::AllProvidersFailed;
        assert_eq!(err.to_string(), "all providers failed");
    }

    #[test]
    fn validation_error_display() {
        let err = ChorusError::Validation("missing phone number".into());
        assert_eq!(err.to_string(), "validation error: missing phone number");
    }

    #[test]
    fn template_not_found_display() {
        let err = ChorusError::TemplateNotFound("welcome".into());
        assert_eq!(err.to_string(), "template not found: welcome");
    }

    #[test]
    fn quota_exceeded_display() {
        let err = ChorusError::QuotaExceeded("monthly limit reached".into());
        assert_eq!(err.to_string(), "quota exceeded: monthly limit reached");
    }

    #[test]
    fn invalid_api_key_display() {
        let err = ChorusError::InvalidApiKey;
        assert_eq!(err.to_string(), "invalid api key");
    }

    #[test]
    fn rate_limited_display() {
        let err = ChorusError::RateLimited {
            retry_after_secs: 30,
        };
        assert_eq!(err.to_string(), "rate limited");
    }

    #[test]
    fn internal_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something broke");
        let err = ChorusError::from(anyhow_err);
        assert!(err.to_string().contains("something broke"));
    }
}
