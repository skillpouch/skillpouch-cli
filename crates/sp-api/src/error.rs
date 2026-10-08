use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The server answered with its error envelope.
    #[error("{message} ({code})")]
    Server {
        code: String,
        status: u16,
        message: String,
        details: Option<Value>,
        retryable: bool,
    },
    /// The server answered with something that isn't the API's envelope.
    #[error("unexpected response from the server (HTTP {status}): {message}")]
    Unexpected { status: u16, message: String },
    #[error("could not reach the server: {0}")]
    Network(#[from] reqwest::Error),
    #[error(transparent)]
    Crypto(#[from] sp_crypto::CryptoError),
}

impl ApiError {
    /// The API error code, e.g. `cli_auth_expired`.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Server { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
