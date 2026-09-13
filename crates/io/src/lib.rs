//! Decode/encode: WAV, FLAC, MP3, AAC, Vorbis; resampling; dither.
//!
//! S1-02 scope: WAV only (16/24-bit int, 32-bit float), through [`hound`] (ADR-001 §3: codec
//! crates are confined to this crate). See [`wav`] for the reader/writer and [`error`] for
//! [`IoError`].
//!
//! S4-02 scope (export encoders, library part): FLAC ([`flac`], `flacenc`) and MP3 ([`mp3`],
//! runtime-loaded LAME, ADR-007 §4) writers alongside WAV, and the [`Encoder`] trait ([`encoder`])
//! that picks between them by output format. The offline resampler export needs is
//! `vox_dsp::resample::resample_offline` (ADR-001 §3: `rubato` lives only in `dsp`).

mod atomic;
mod dither;
pub mod encoder;
pub mod error;
pub mod flac;
pub mod mp3;
pub mod wav;

pub use encoder::{Encoder, FlacEncoder, Mp3Encoder, WavEncoder};
pub use error::{IoError, Result};
pub use flac::{FlacBitDepth, write_flac};
pub use mp3::{Mp3Settings, encode_mp3, mp3_available};
pub use wav::{
    BitDepth, SampleFormat, WavFormat, WavMarker, WavSource, WriteReport, read_wav,
    read_wav_markers, write_wav, write_wav_with_markers,
};
