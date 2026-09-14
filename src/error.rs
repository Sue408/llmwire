#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("buffer limit exceeded")]
    BufferLimitExceeded,
}
