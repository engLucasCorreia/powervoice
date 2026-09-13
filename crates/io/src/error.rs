//! Error type of `vox-io`.

use std::io;

/// Result alias used throughout the crate.
pub type Result<T, E = IoError> = std::result::Result<T, E>;

/// Everything that can go wrong reading or writing an audio file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IoError {
    /// Any I/O failure (open, read, write, rename, fsync, …).
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// `hound` rejected the container or a sample.
    #[error("WAV error: {0}")]
    Wav(#[from] hound::Error),
    /// A recognized WAV variant this ticket's scope doesn't cover (SPEC-005 §2.2): only
    /// 16/24-bit integer and 32-bit float are read; only 16/24-bit integer and 32-bit float are
    /// written. Wider tolerance (8-bit, 32-bit int, 64-bit float, A-law/µ-law) is deferred.
    #[error("unsupported WAV format: {0}")]
    Unsupported(String),
    /// An argument outside its documented range (e.g. `bits` not 16 or 24 for a dithered write).
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// `flacenc` rejected the encoder configuration or failed mid-encode (S4-02).
    #[error("FLAC encode error: {0}")]
    FlacEncode(String),
    /// No `libmp3lame` shared library could be loaded (ADR-007 §4, D-013): MP3 export is
    /// unavailable, and the app should show a message rather than crash.
    #[error("MP3 export unavailable: no libmp3lame could be loaded ({0})")]
    Mp3Unavailable(String),
    /// A LAME call failed or returned an error code (S4-02).
    #[error("MP3 encode error: {0}")]
    Mp3Encode(String),
}
