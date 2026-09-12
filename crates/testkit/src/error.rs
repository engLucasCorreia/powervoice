//! Error type for `testkit`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestkitError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAV error: {0}")]
    Wav(#[from] hound::Error),
    #[error("loudness measurement error: {0}")]
    Loudness(#[from] ebur128::Error),
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
}

pub type Result<T> = std::result::Result<T, TestkitError>;
