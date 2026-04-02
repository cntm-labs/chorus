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
