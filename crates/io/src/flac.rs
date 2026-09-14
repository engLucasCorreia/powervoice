//! FLAC encode via `flacenc` (ADR-007; SPEC-005 §2.11, §2.14 export [M6]).
//!
//! Mono only (PowerVoice edits mono, PROMPT §2 LOCKED). Bit depth 16 or 24, TPDF-dithered from the
//! f32 samples with the same quantizer [`write_wav`](crate::write_wav) uses (`vox_dsp::dither`),
//! block size 4096 (SPEC-005 §2.11, `flac_block_size`). Atomic write: temp file next to the target,
//! `fdatasync`, rename, directory `fsync` (ADR-004 §8), matching [`crate::wav::write_wav`].
//!
//! **H-14 root cause.** `flacenc` 0.5.1's own output made `flac -t` warn "sample or frame number
//! does not increase correctly" and made `symphonia-bundle-flac` abort with `UnexpectedEof` on its
//! sample-count cross-check (MEMORY.md S4-02/T-202 notes) — but the bitstream's actual audio content
//! was never corrupt (`ffmpeg`/`flac -d` always decoded it bit-exact; see the `flac_repro` finding
//! recorded in the H-14 ticket report). The real defect is in the **STREAMINFO block-size fields**:
//! `flacenc::component::Stream::add_frame` calls `StreamInfo::update_frame_info`, which folds every
//! encoded frame's *actual* block size — including the final, shorter frame — into
//! `min_block_size`/`max_block_size` (`flacenc-0.5.1/src/component/datatype.rs` `update_frame_info`,
//! `Stream::add_frame`). For any track whose length isn't an exact multiple of the block size (i.e.
//! almost every real recording), that leaves `min_block_size < max_block_size`. The reference
//! encoder instead pins both fields to the *configured* block size regardless of the final frame's
//! true length (verified empirically: `flac --blocksize=4096 …` on a 96 000-sample and on a
//! 1 000-sample input both report `minimum blocksize: 4096` / `maximum blocksize: 4096`). Strict
//! decoders rely on `min_block_size == max_block_size` to recognize "fixed blocksize" mode and turn
//! a frame's coded *frame number* into a sample offset via `frame_number * block_size`
//! (`libFLAC/stream_decoder.c` `read_frame_header_`; `symphonia-bundle-flac/src/parser.rs`
//! `calc_sync_info`/`strict_frame_header_check`, `is_fixed = block_len_min == block_len_max`); with
//! `min != max` neither ever locks onto fixed-blocksize mode, so the running sample offset drifts
//! frame by frame until the mismatch trips the header/CRC once decoded position no longer lines up
//! with the container's stated one. Binary-patching just the two `min_block_size` bytes of our own
//! export back to the nominal block size (confirmed via a byte-for-byte STREAMINFO edit during
//! investigation) made both `flac -t` and `symphonia` accept the exact same frame data unchanged —
//! confirming the frame/audio payload was always fine.
//!
//! **Fix (our usage of `flacenc`, no crate patch or replacement needed).** After
//! `encode_with_fixed_block_size` returns its `Stream`, [`write_temp`] resets
//! `stream_info_mut().set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)` to undo that fold-in, matching the
//! reference encoder's convention. This is metadata-only: it does not touch any already-encoded
//! frame's bytes, so it cannot desync the bitstream itself.
//!
//! **Verify-before-rename (SPEC-005 §2.11).** [`write_flac`] now decodes the temp file straight back
//! with [`crate::decode`] (T-202) and compares it, sample for sample, against the quantized values it
//! just wrote (`flac -V`'s own strategy) before the atomic rename; a mismatch (including a
//! `symphonia` decode failure) returns [`IoError::FlacVerifyFailed`]/the propagated decode error, and
//! the temp file is deleted so the target is left untouched.

use std::path::Path;

use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config;
use flacenc::error::Verify;
use flacenc::source::MemSource;

use vox_dsp::dither::quantize_dithered;

use crate::atomic::{finish, temp_path_for};
use crate::decode::DecodeSource;
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
    write_flac_impl(path, sample_rate_hz, bits, samples, |_tmp| {})
}

/// [`write_flac`]'s body, with a hook that runs on the temp file after it's written but before
/// verify-before-rename checks it — a no-op in production, used by tests to simulate on-disk
/// corruption of the temp file without reaching into `write_flac`'s otherwise-private pipeline.
fn write_flac_impl(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: FlacBitDepth,
    samples: &[f32],
    after_write: impl FnOnce(&Path),
) -> Result<WriteReport> {
    let path = path.as_ref();
    let (values, clipped) = quantize_dithered(samples, bits.bits());
    let tmp = temp_path_for(path);
    if let Err(e) = write_temp(&tmp, sample_rate_hz, bits, &values) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    after_write(&tmp);
    if let Err(e) = verify_temp(&tmp, bits, &values) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    finish(&tmp, path)?;
    Ok(WriteReport {
        clipped_samples: clipped,
    })
}

/// Verify-before-rename (SPEC-005 §2.11): decodes `tmp` back with the T-202 `symphonia` path and
/// checks it is sample-for-sample identical to `expected` (the values just written), like `flac
/// -V`. Called before [`finish`] renames the temp file over the target; on any mismatch (including
/// a decode failure) the target must stay untouched — the caller deletes `tmp` and returns the
/// error rather than proceeding to the rename.
fn verify_temp(tmp: &Path, bits: FlacBitDepth, expected: &[i32]) -> Result<()> {
    let (info, mut source) = DecodeSource::open(tmp)?;
    if info.track.channels.len() != 1 {
        return Err(IoError::FlacVerifyFailed(format!(
            "expected 1 channel, decoded {}",
            info.track.channels.len()
        )));
    }
    // `bits`-bit int -> f32 is `v / 2^(bits-1)` (`crate::decode`'s conversion table); both that
    // division and this inverse multiplication are exact in f32 (power-of-two scaling of an
    // integer that fits the mantissa), so re-quantizing recovers the exact integer symphonia
    // decoded, byte for byte comparable with what flacenc was asked to encode.
    let scale = (1i64 << (bits.bits() - 1)) as f32;
    let mut decoded = Vec::with_capacity(expected.len());
    let mut buf = [0f32; 4096];
    loop {
        let n = source.read_frames(&mut buf)?;
        if n == 0 {
            break;
        }
        decoded.extend(buf[..n].iter().map(|s| (s * scale).round() as i32));
    }
    if decoded.len() != expected.len() {
        return Err(IoError::FlacVerifyFailed(format!(
            "wrote {} samples but decoded {} back",
            expected.len(),
            decoded.len()
        )));
    }
    if let Some(i) = (0..decoded.len()).find(|&i| decoded[i] != expected[i]) {
        return Err(IoError::FlacVerifyFailed(format!(
            "decoded sample {i} ({}) does not match the quantized value written ({})",
            decoded[i], expected[i]
        )));
    }
    Ok(())
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
    let mut stream = flacenc::encode_with_fixed_block_size(&encoder_config, source, BLOCK_SIZE)
        .map_err(|e| IoError::FlacEncode(format!("{e:?}")))?;
    // H-14 root-cause fix: `Stream::add_frame` folds every frame's *actual* block size into
    // STREAMINFO's min/max (`flacenc`'s `update_frame_info`), so a track whose length isn't a
    // multiple of `BLOCK_SIZE` ends up with `min_block_size` set to the shorter final frame's size
    // instead of staying pinned at `BLOCK_SIZE`. Strict decoders (libFLAC, symphonia) require
    // `min_block_size == max_block_size` to treat the stream as fixed-blocksize and correctly turn
    // a frame's coded frame number into a sample offset (`frame_number * block_size`); with them
    // unequal every frame after the first decodes at the wrong offset (see the module doc). This
    // is metadata-only — it doesn't touch any frame already encoded above.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
        .map_err(|e| IoError::FlacEncode(format!("resetting STREAMINFO block sizes: {e:?}")))?;
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

    /// H-14 regression test: `flac -t -w` must accept our export with **no** warnings (not just a
    /// zero exit code — the pre-fix bug exited 0 while still printing "sample or frame number does
    /// not increase correctly" to stderr) across the bit depths/rates the ticket names. Durations
    /// are deliberately *not* a multiple of [`BLOCK_SIZE`] so the final frame is shorter than the
    /// rest — the exact shape that made `flacenc` mis-set STREAMINFO's `min_block_size` (see the
    /// module docs).
    #[test]
    fn flac_t_reports_no_warnings_across_rates_and_bit_depths() {
        if !flac_binary_available() {
            eprintln!("skipping: `flac` reference decoder not installed");
            return;
        }
        let dir = tmp_dir("flac-t-warnings");
        for &rate_hz in &[44_100u32, 48_000, 96_000] {
            for &bits in &[FlacBitDepth::Int16, FlacBitDepth::Int24] {
                let path = dir.join(format!("out-{rate_hz}-{}.flac", bits.bits()));
                let samples = sine(997.0, -20.0, 2.3, rate_hz);
                write_flac(&path, rate_hz, bits, &samples).unwrap();

                let out = Command::new("flac")
                    .args(["-t", "-w"])
                    .arg(&path)
                    .output()
                    .expect("run flac -t");
                let stderr = String::from_utf8_lossy(&out.stderr);
                assert!(
                    out.status.success(),
                    "flac -t failed for {rate_hz} Hz {}-bit: {stderr}",
                    bits.bits()
                );
                assert!(
                    !stderr.contains("WARNING"),
                    "flac -t warned for {rate_hz} Hz {}-bit: {stderr}",
                    bits.bits()
                );
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// H-14 acceptance test: `symphonia` (via [`DecodeSource`], T-202) decodes our export back to
    /// exactly the values [`vox_dsp::dither::quantize_dithered`] wrote — same (deterministic)
    /// dither seed on both sides — for both a grid-exact block (no dither) and a dithered,
    /// non-block-aligned tail. Before the fix this failed with `IoError::Io(UnexpectedEof)`.
    #[test]
    fn symphonia_decodes_our_export_bit_exact_with_the_same_dither_seed() {
        let dir = tmp_dir("symphonia-roundtrip");
        let path = dir.join("out.flac");

        let mut samples = vec![0.0f32; BLOCK_SIZE]; // grid-exact: not dithered
        samples.extend(sine(440.0, -6.0, 1.3, 48_000)); // off-grid tail: dithered
        write_flac(&path, 48_000, FlacBitDepth::Int16, &samples).unwrap();
        let expected = quantize_dithered(&samples, 16).0;

        let (info, mut source) = DecodeSource::open(&path).unwrap();
        assert_eq!(info.track.sample_rate_hz, 48_000);
        assert_eq!(info.track.len_samples, Some(expected.len() as u64));
        assert_eq!(source.channels(), 1);

        let scale = (1i64 << 15) as f32;
        let mut decoded = Vec::with_capacity(expected.len());
        let mut buf = [0f32; BLOCK_SIZE];
        loop {
            let n = source.read_frames(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            decoded.extend(buf[..n].iter().map(|s| (s * scale).round() as i32));
        }
        assert_eq!(decoded, expected);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// SPEC-005 §2.11 acceptance test: a temp file that's corrupted after `flacenc` writes it (but
    /// before verify-before-rename runs) must fail the export — `write_flac` reports the error and
    /// leaves the target and the directory exactly as they were, instead of renaming corrupt bytes
    /// over the target.
    #[test]
    fn corrupted_temp_file_makes_verify_before_rename_fail_the_export() {
        let dir = tmp_dir("verify-corrupt");
        let path = dir.join("out.flac");
        std::fs::write(&path, b"OLD-CONTENT").unwrap();
        let samples = sine(997.0, -20.0, 0.3, 48_000);

        let err = write_flac_impl(&path, 48_000, FlacBitDepth::Int16, &samples, |tmp| {
            let mut bytes = std::fs::read(tmp).unwrap();
            let corrupt_at = bytes.len() - 10;
            bytes[corrupt_at] ^= 0xFF;
            std::fs::write(tmp, &bytes).unwrap();
        })
        .unwrap_err();
        assert!(
            matches!(
                err,
                IoError::FlacVerifyFailed(_)
                    | IoError::Io(_)
                    | IoError::UnrecognizedFormat(_)
                    | IoError::UnsupportedCodec(_)
            ),
            "unexpected error variant for a corrupted temp file: {err:?}"
        );

        // The target is untouched and no temp file was left behind.
        assert_eq!(std::fs::read(&path).unwrap(), b"OLD-CONTENT");
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("out.flac")]);

        std::fs::remove_dir_all(&dir).unwrap();
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
