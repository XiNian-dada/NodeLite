//! Errors intentionally exclude Agent configuration and authentication material.

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("update state is malformed")]
    Json(#[from] serde_json::Error),
    #[error("update policy is malformed")]
    PolicyParse(#[from] toml::de::Error),
    #[error("update policy rejected: {0}")]
    Policy(&'static str),
    #[error("update state requires manual recovery: {0}")]
    Recovery(&'static str),
    #[error("another update transaction is running")]
    Busy,
    #[error("cannot obtain randomness for a private update file")]
    Randomness,
    #[error("signed repository verification failed: {0}")]
    Signature(#[source] Box<tough::error::Error>),
    #[error("update network operation failed")]
    Network(#[from] reqwest::Error),
    #[error("update operation exceeded its deadline")]
    Timeout,
}

pub(crate) type Result<T> = std::result::Result<T, UpdateError>;

impl From<tough::error::Error> for UpdateError {
    fn from(error: tough::error::Error) -> Self {
        Self::Signature(Box::new(error))
    }
}
