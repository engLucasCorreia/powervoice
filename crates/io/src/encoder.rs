//! A common [`Encoder`] trait over WAV/FLAC/MP3 (ticket S4-02), so export code (S4-04) and the CLI
//! `convert` command can pick an encoder by output format without matching on it themselves.

use std::path::Path;

use vox_dsp::dither::DitherMode;

use crate::error::Result;
use crate::flac::{self, FlacBitDepth};
use crate::mp3::{self, Mp3Settings};
use crate::wav::{self, BitDepth, WriteReport};

/// Writes mono `samples` at `sample_rate_hz` to `path`, atomically.
pub trait Encoder {
    fn write(&self, path: &Path, sample_rate_hz: u32, samples: &[f32]) -> Result<WriteReport>;
}

/// WAV output ([`crate::write_wav`]). Always TPDF-dithered (H-20's `DitherMode::None` is a Save
/// As option, not exposed to export/CLI callers of this trait).
#[derive(Debug, Clone, Copy)]
pub struct WavEncoder(pub BitDepth);

impl Encoder for WavEncoder {
    fn write(&self, path: &Path, sample_rate_hz: u32, samples: &[f32]) -> Result<WriteReport> {
        wav::write_wav(path, sample_rate_hz, self.0, DitherMode::Tpdf, samples)
    }
}

/// FLAC output ([`crate::flac::write_flac`]). Always TPDF-dithered, same note as [`WavEncoder`].
#[derive(Debug, Clone, Copy)]
pub struct FlacEncoder(pub FlacBitDepth);

impl Encoder for FlacEncoder {
    fn write(&self, path: &Path, sample_rate_hz: u32, samples: &[f32]) -> Result<WriteReport> {
        flac::write_flac(path, sample_rate_hz, self.0, DitherMode::Tpdf, samples)
    }
}

/// MP3 output via runtime-loaded LAME ([`crate::mp3::encode_mp3`]).
#[derive(Debug, Clone, Copy)]
pub struct Mp3Encoder(pub Mp3Settings);

impl Encoder for Mp3Encoder {
    fn write(&self, path: &Path, sample_rate_hz: u32, samples: &[f32]) -> Result<WriteReport> {
        mp3::encode_mp3(path, sample_rate_hz, samples, self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-io-encoder-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wav_and_flac_encoders_dispatch_through_the_trait() {
        let dir = tmp_dir("dispatch");
        let samples = [0.1f32, -0.2, 0.3, -0.4];

        let wav_path = dir.join("out.wav");
        let encoders: Vec<Box<dyn Encoder>> = vec![
            Box::new(WavEncoder(BitDepth::Int16)),
            Box::new(FlacEncoder(FlacBitDepth::Int16)),
        ];
        let paths = [wav_path.clone(), dir.join("out.flac")];
        for (encoder, path) in encoders.iter().zip(paths.iter()) {
            encoder.write(path, 48_000, &samples).unwrap();
            assert!(path.exists());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
