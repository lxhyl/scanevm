use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("No API key configured. Run: scanevm config set-key <KEY>")]
    NoApiKey,

    #[error("Unknown chain: '{0}'. Run: scanevm chains")]
    UnknownChain(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),
}
