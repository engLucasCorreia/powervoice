//! FLAC encode via `flacenc` (ADR-007; SPEC-005 §2.11, §2.14 export [M6]).
//!
//! Mono only (PowerVoice edits mono, PROMPT §2 LOCKED). Bit depth 16 or 24, TPDF-dithered from the
//! f32 samples with the same quantizer [`write_wav`](crate::write_wav) uses (`vox_dsp::dither`),
//! block size 4096 (SPEC-005 §2.11, `flac_block_size`). Atomic write: temp file next to the target,
//! `fdatasync`, rename, directory `fsync` (ADR-004 §8), matching [`crate::wav::write_wav`].
//!
//! **Deviation (S4-02 ticket report):** SPEC-005 §2.11 calls for decoding the temp file back and
//! comparing it against the quantized samples before the rename ("verify before rename"). Wiring a
//! decoder into this production write path is deferred: the options are enabling `flacenc`'s
//! `decode` feature (pulls in `nom`, not named by this ticket or ADR-007) or the `symphonia` import
//! path (T-202, not built yet). This ticket's own acceptance test — "FLAC decodes bit-exact to the
//! WAV" — is instead covered by an integration test that shells out to the reference `flac` decoder
//! (skipped if not installed), the same tool SPEC-005's own AC-16 test plan uses for its manual
//! smoke check. Wiring real verify-before-rename into `write_flac` is noted for hardening/S4-04.

use std::path::Path;

use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config;
use flacenc::error::Verify;
use flacenc::source::MemSource;

use vox_dsp::dither::quantize_dithered;

use crate::atomic::{finish, temp_path_for};
use crate::error::{IoError, Result};
use crate::wav::WriteReport;

/// FLAC has no float sample format: 16- or 24-bit integer only (SPEC-005 §2.6, §2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlacBitDepth {
    Int16,
    Int24,
}

impl FlacBitDepth {
    fn bits(self) -> u32 {
        match self {
            FlacBitDepth::Int16 => 16,
            FlacBitDepth::Int24 => 24,
        }
    }
}

/// FLAC block size (SPEC-005 §2.11, `flac_block_size`): flacenc's default LPC/residual settings
/// otherwise apply (≈ libFLAC level 5 class).
const BLOCK_SIZE: usize = 4096;

/// Writes mono `samples` to `path` as FLAC at `bits`, atomically (ADR-004 §8). Samples are
/// TPDF-dithered per 4096-sample block, skipping blocks already exactly representable at `bits`
/// (grid-exact passthrough), with the same PRNG and rules [`write_wav`](crate::write_wav) uses for
/// integer output — a document saved as WAV 16/24 and exported as FLAC of the same depth carry
/// identical audio.
pub fn write_flac(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: FlacBitDepth,
    samples: &[f32],
) -> Result<WriteReport> {
    let path = path.as_ref();
    let (values, clipped) = quantize_dithered(samples, bits.bits());
    let tmp = temp_path_for(path);
    if let Err(e) = write_temp(&tmp, sample_rate_hz, bits, &values) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    finish(&tmp, path)?;
    Ok(WriteReport {
        clipped_samples: clipped,
    })
}

fn write_temp(tmp: &Path, sample_rate_hz: u32, bits: FlacBitDepth, values: &[i32]) -> Result<()> {
    let source = MemSource::from_samples(values, 1, bits.bits() as usize, sample_rate_hz as usize);
    let mut encoder_config = config::Encoder::default();
    // A `#[non_exhaustive]` config struct: mutate the field on an owned instance rather than using
    // struct-literal syntax.
    encoder_config.block_size = BLOCK_SIZE;
    let encoder_config = encoder_config
        .into_verified()
        .map_err(|(_, e)| IoError::FlacEncode(format!("invalid encoder config: {e:?}")))?;
    let stream = flacenc::encode_with_fixed_block_size(&encoder_config, source, BLOCK_SIZE)
        .map_err(|e| IoError::FlacEncode(format!("{e:?}")))?;
    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| IoError::FlacEncode(format!("{e:?}")))?;
    std::fs::write(tmp, sink.as_slice())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-io-flac-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sine(freq_hz: f64, level_dbfs: f64, duration_s: f64, rate_hz: u32) -> Vec<f32> {
        let amp = 10f64.powf(level_dbfs / 20.0);
        let n = (duration_s * f64::from(rate_hz)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate_hz);
                (amp * (std::f64::consts::TAU * freq_hz * t).sin()) as f32
            })
            .collect()
    }

    /// The reference `flac` decoder binary, if installed (same tool SPEC-005's AC-16 test plan
    /// names for its manual smoke check). `None` skips the decode-compare tests gracefully instead
    /// of failing on a machine without it (ticket instruction: tests must not hard-depend on it).
    fn flac_binary_available() -> bool {
        Command::new("flac")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn flac_decodes_bit_exact_to_the_source_24bit() {
        if !flac_binary_available() {
            eprintln!("skipping: `flac` reference decoder not installed");
            return;
        }
        let dir = tmp_dir("roundtrip24");
        let flac_path = dir.join("out.flac");
        let wav_path = dir.join("decoded.wav");

        let samples = sine(997.0, -20.0, 0.5, 48_000);
        write_flac(&flac_path, 48_000, FlacBitDepth::Int24, &samples).unwrap();
        let expected = quantize_dithered(&samples, 24).0;

        let status = Command::new("flac")
            .args(["-d", "-f", "-o"])
            .arg(&wav_path)
            .arg(&flac_path)
            .status()
            .expect("run flac -d");
        assert!(status.success(), "flac -d failed");

        let mut reader = hound::WavReader::open(&wav_path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.bits_per_sample, 24);
        let decoded: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn flac_decodes_bit_exact_to_the_source_16bit() {
        if !flac_binary_available() {
            eprintln!("skipping: `flac` reference decoder not installed");
            return;
        }
        let dir = tmp_dir("roundtrip16");
        let flac_path = dir.join("out.flac");
        let wav_path = dir.join("decoded.wav");

        // A block boundary and an off-grid tail so both the grid-exact and dithered quantizer
        // paths are exercised in the same file.
        let mut samples = vec![0.0f32; 4096];
        samples.extend(sine(440.0, -6.0, 0.2, 48_000));
        write_flac(&flac_path, 48_000, FlacBitDepth::Int16, &samples).unwrap();
        let expected = quantize_dithered(&samples, 16).0;

        let status = Command::new("flac")
            .args(["-d", "-f", "-o"])
            .arg(&wav_path)
            .arg(&flac_path)
            .status()
            .expect("run flac -d");
        assert!(status.success(), "flac -d failed");

        let mut reader = hound::WavReader::open(&wav_path).unwrap();
        let decoded: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn atomic_write_leaves_the_old_file_intact_on_failure() {
        let dir = tmp_dir("atomic-fail");
        let path = dir.join("out.flac");
        std::fs::write(&path, b"OLD-CONTENT").unwrap();

        // Sabotage: put a directory where the temp file needs to be created.
        let tmp = temp_path_for(&path);
        std::fs::create_dir(&tmp).unwrap();

        let err = write_flac(&path, 48_000, FlacBitDepth::Int16, &[0.0; 10]).unwrap_err();
        assert!(matches!(err, IoError::Io(_)));
        assert_eq!(std::fs::read(&path).unwrap(), b"OLD-CONTENT");

        std::fs::remove_dir(&tmp).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_flac_leaves_no_temp_file_on_success() {
        let dir = tmp_dir("atomic-ok");
        let path = dir.join("out.flac");
        write_flac(&path, 48_000, FlacBitDepth::Int16, &[0.1, -0.2, 0.3]).unwrap();
        assert!(path.exists());
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.pop().unwrap(), "out.flac");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
