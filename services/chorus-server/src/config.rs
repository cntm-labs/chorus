/// Server configuration loaded from environment variables.
pub struct Config {
    /// PostgreSQL connection URL.
    pub database_url: String,
    /// Redis connection URL.
    pub redis_url: String,
    /// Bind host address.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Number of concurrent queue workers.
    pub worker_concurrency: usize,

    // SMS providers (global defaults)
    /// Telnyx API key.
    pub telnyx_api_key: Option<String>,
    /// Telnyx sender phone number.
    pub telnyx_from: Option<String>,
    /// Twilio account SID.
    pub twilio_account_sid: Option<String>,
    /// Twilio auth token.
    pub twilio_auth_token: Option<String>,
    /// Twilio sender phone number.
    pub twilio_from: Option<String>,
    /// Plivo auth ID.
    pub plivo_auth_id: Option<String>,
    /// Plivo auth token.
    pub plivo_auth_token: Option<String>,
    /// Plivo sender phone number.
    pub plivo_from: Option<String>,

    // Email providers (global defaults)
    /// Resend API key.
    pub resend_api_key: Option<String>,
    /// AWS SES access key.
    pub ses_access_key: Option<String>,
    /// AWS SES secret key.
    pub ses_secret_key: Option<String>,
    /// AWS SES region.
    pub ses_region: Option<String>,
    /// SMTP server host.
    pub smtp_host: Option<String>,
    /// SMTP server port.
    pub smtp_port: Option<u16>,
    /// SMTP username.
    pub smtp_username: Option<String>,
    /// SMTP password.
    pub smtp_password: Option<String>,
    /// Mailgun API key.
    pub mailgun_api_key: Option<String>,
    /// Mailgun sending domain.
    pub mailgun_domain: Option<String>,
    /// Mailgun base URL (default US, set to https://api.eu.mailgun.net for EU).
    pub mailgun_base_url: Option<String>,
    /// Default sender email address.
    pub from_email: Option<String>,

    /// Shared secret for chorus-mail bounce webhook.
    pub bounce_secret: Option<String>,
}

impl Config {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://chorus:chorus@localhost:5432/chorus".into()),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            worker_concurrency: std::env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(4),

            telnyx_api_key: std::env::var("TELNYX_API_KEY").ok(),
            telnyx_from: std::env::var("TELNYX_FROM").ok(),
            twilio_account_sid: std::env::var("TWILIO_ACCOUNT_SID").ok(),
            twilio_auth_token: std::env::var("TWILIO_AUTH_TOKEN").ok(),
            twilio_from: std::env::var("TWILIO_FROM").ok(),
            plivo_auth_id: std::env::var("PLIVO_AUTH_ID").ok(),
            plivo_auth_token: std::env::var("PLIVO_AUTH_TOKEN").ok(),
            plivo_from: std::env::var("PLIVO_FROM").ok(),

            mailgun_api_key: std::env::var("MAILGUN_API_KEY").ok(),
            mailgun_domain: std::env::var("MAILGUN_DOMAIN").ok(),
            mailgun_base_url: std::env::var("MAILGUN_BASE_URL").ok(),
            resend_api_key: std::env::var("RESEND_API_KEY").ok(),
            ses_access_key: std::env::var("AWS_SES_ACCESS_KEY").ok(),
            ses_secret_key: std::env::var("AWS_SES_SECRET_KEY").ok(),
            ses_region: std::env::var("AWS_SES_REGION").ok(),
            smtp_host: std::env::var("SMTP_HOST").ok(),
            smtp_port: std::env::var("SMTP_PORT").ok().and_then(|p| p.parse().ok()),
            smtp_username: std::env::var("SMTP_USERNAME").ok(),
            smtp_password: std::env::var("SMTP_PASSWORD").ok(),
            from_email: std::env::var("FROM_EMAIL").ok(),

            bounce_secret: std::env::var("BOUNCE_SECRET").ok(),
        }
    }
}
