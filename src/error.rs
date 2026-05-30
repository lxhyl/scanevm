use thiserror::Error;

pub type Result<T> = std::result::Result<T, ScanevmError>;

/// All errors surfaced to the user. Each variant maps to a distinct process
/// exit code (see [`ScanevmError::exit_code`]) so scripts and agents can branch
/// on the failure class — e.g. back off and retry on `RateLimited` (4) but give
/// up on `ApiKeyMissing` (2).
#[derive(Debug, Error)]
pub enum ScanevmError {
    #[error("No API key configured. Run: scanevm config set-key <KEY>")]
    ApiKeyMissing,

    #[error("Invalid API key. Check the key, or set a new one: scanevm config set-key <KEY>")]
    InvalidApiKey,

    #[error("Unknown chain: '{0}'. Run: scanevm chains")]
    UnknownChain(String),

    #[error("Invalid input: {0}")]
    BadInput(String),

    #[error("{}", match .retry_after {
        Some(s) => format!("Rate limit reached. Try again in ~{s}s (or use an API Pro plan)."),
        None => "Rate limit reached. Slow down requests, or use an API Pro plan.".to_string(),
    })]
    RateLimited { retry_after: Option<u64> },

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Contract source code is not verified for {0}")]
    NotVerified(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("{0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Data error: {0}")]
    Json(#[from] serde_json::Error),
}

impl ScanevmError {
    /// Stable, documented exit code per failure class.
    ///
    /// | code | meaning                                               |
    /// |------|-------------------------------------------------------|
    /// | 2    | usage/config (missing key, unknown chain, bad input)  |
    /// | 3    | network/transport failure                             |
    /// | 4    | rate limited (retryable)                              |
    /// | 5    | invalid API key (auth)                                |
    /// | 6    | other upstream API error                              |
    /// | 7    | not found (no such block / tx)                        |
    /// | 8    | contract source not verified                          |
    /// | 9    | local I/O error                                       |
    /// | 70   | internal data/decoding error                          |
    pub fn exit_code(&self) -> i32 {
        use ScanevmError::*;
        match self {
            ApiKeyMissing | UnknownChain(_) | BadInput(_) | Config(_) => 2,
            Network(_) => 3,
            RateLimited { .. } => 4,
            InvalidApiKey => 5,
            ApiError(_) => 6,
            NotFound(_) => 7,
            NotVerified(_) => 8,
            Io(_) => 9,
            Json(_) => 70,
        }
    }

    /// Convert a `reqwest::Error` into a sanitized [`ScanevmError`].
    ///
    /// IMPORTANT: this deliberately never includes the request URL. The API key
    /// travels as the `apikey` query parameter, and `reqwest::Error`'s `Display`
    /// embeds the full URL — so printing it verbatim would leak the key to
    /// stderr / CI logs. We classify the error and emit a key-free message.
    pub fn from_http(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            ScanevmError::Network("request timed out".to_string())
        } else if e.is_connect() {
            ScanevmError::Network("could not connect to the API endpoint".to_string())
        } else if let Some(status) = e.status() {
            ScanevmError::Network(format!("HTTP {}", status.as_u16()))
        } else if e.is_decode() {
            ScanevmError::Network("could not read the API response".to_string())
        } else {
            ScanevmError::Network("request failed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct_per_class() {
        assert_eq!(ScanevmError::ApiKeyMissing.exit_code(), 2);
        assert_eq!(ScanevmError::UnknownChain("x".into()).exit_code(), 2);
        assert_eq!(ScanevmError::BadInput("x".into()).exit_code(), 2);
        assert_eq!(ScanevmError::Network("x".into()).exit_code(), 3);
        assert_eq!(
            ScanevmError::RateLimited { retry_after: None }.exit_code(),
            4
        );
        assert_eq!(ScanevmError::InvalidApiKey.exit_code(), 5);
        assert_eq!(ScanevmError::ApiError("x".into()).exit_code(), 6);
        assert_eq!(ScanevmError::NotFound("x".into()).exit_code(), 7);
        assert_eq!(ScanevmError::NotVerified("x".into()).exit_code(), 8);
    }

    #[test]
    fn rate_limited_message_mentions_retry_after() {
        let e = ScanevmError::RateLimited {
            retry_after: Some(12),
        };
        assert!(e.to_string().contains("12s"));
    }
}
