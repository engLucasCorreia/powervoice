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
//!
//! H-02: the TPDF dither/quantizer moved to `vox_dsp::dither` (ADR-001 §4); [`wav`] and [`flac`]
//! call through it. [`wav::WavStreamWriter`] additionally lets a save stream through in blocks
//! instead of collecting the whole document into one buffer first.

mod atomic;
pub mod decode;
pub mod downmix;
pub mod encoder;
pub mod error;
pub mod flac;
pub mod mp3;
pub mod wav;

pub use decode::{ChannelProbeResult, DecodeSource, ProbeInfo, TrackFacts, probe, probe_channels};
pub use downmix::{
    ACTIVE_CHANNEL_DBFS, ChannelInfo, ChannelPeak, ChannelPeakAccumulator, DownmixChoice,
    IdenticalChannelsCheck, SILENT_CHANNEL_DBFS, channel_infos, downmix_average, downmix_frame,
    downmix_pick, suggest_silent_channel,
};
pub use encoder::{Encoder, FlacEncoder, Mp3Encoder, WavEncoder};
pub use error::{IoError, Result};
pub use flac::{FlacBitDepth, write_flac};
pub use mp3::{Mp3Settings, encode_mp3, mp3_available};
/// H-20 (SPEC-005 §2.7/§2.8): re-exported so `write_wav`/`write_flac` callers don't need a direct
/// `vox_dsp` dependency just to name the dither mode (mirrors `dither`'s own module doc: `wav` and
/// `flac` already call through `vox_dsp::dither`).
pub use vox_dsp::dither::DitherMode;
pub use wav::{
    BitDepth, SampleFormat, WavFormat, WavMarker, WavMarkersResult, WavSource, WavStreamWriter,
    WriteReport, read_wav, read_wav_markers, read_wav_markers_detailed, wav_has_foreign_metadata,
    write_wav, write_wav_with_markers,
};
