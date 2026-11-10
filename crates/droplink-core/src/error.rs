use thiserror::Error;

/// Result alias used across the DropLink workspace.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("frame too large: {0} bytes")]
    FrameTooLarge(usize),

    #[error("truncated frame")]
    Truncated,

    #[error("protocol version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },

    #[error("invalid file name: {0}")]
    InvalidFileName(String),

    #[error("invalid peer: {0}")]
    InvalidPeer(String),

    #[error("hash mismatch")]
    HashMismatch,

    #[error("cryptographic error: {0}")]
    Crypto(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("timeout")]
    Timeout,

    #[error("transfer refused: {0}")]
    Refused(String),

    #[error("busy")]
    Busy,

    #[error("cancelled")]
    Cancelled,
}
