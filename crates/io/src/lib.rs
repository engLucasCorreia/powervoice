//! Decode/encode: WAV, FLAC, MP3, AAC, Vorbis; resampling; dither.
//!
//! S1-02 scope: WAV only (16/24-bit int, 32-bit float), through [`hound`] (ADR-001 §3: codec
//! crates are confined to this crate). See [`wav`] for the reader/writer and [`error`] for
//! [`IoError`].

mod dither;
pub mod error;
pub mod wav;

pub use error::{IoError, Result};
pub use wav::{BitDepth, SampleFormat, WavFormat, WavSource, WriteReport, read_wav, write_wav};
