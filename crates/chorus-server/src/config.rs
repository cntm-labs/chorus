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
        }
    }
}
