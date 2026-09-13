//! WAV read/write through `hound` (SPEC-005 §2.2, §2.3, §2.7, §2.8; ADR-004 §8 atomic save).
//!
//! Scope of this ticket (S1-02): 16-bit and 24-bit integer PCM and 32-bit IEEE float, mono or
//! multichannel-downmixed-to-mono on read, mono on write (PowerVoice edits mono only). Wider
//! tolerance (8-bit, 32-bit int, 64-bit float, A-law/µ-law on read; `cue `/`LIST adtl` markers;
//! FLAC; atomic-save cleanup of stale temp files) is deferred — see the ticket report.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use crate::dither::quantize_dithered;
use crate::error::{IoError, Result};

/// On-disk sample encoding (mirrors `hound::SampleFormat`, kept as our own type so callers don't
/// need a direct `hound` dependency just to read it back from [`WavFormat`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    Int,
    Float,
}

/// Container facts of an opened WAV, alongside the `(rate, channels)` [`read_wav`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub sample_format: SampleFormat,
}

/// Bit depth to write (SPEC-005 §2.6): 16/24-bit integer (TPDF-dithered, §2.8) or 32-bit float
/// (bit-exact, never dithered or clipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

impl BitDepth {
    fn hound_spec(self, sample_rate_hz: u32) -> hound::WavSpec {
        let (bits_per_sample, sample_format) = match self {
            BitDepth::Int16 => (16, hound::SampleFormat::Int),
            BitDepth::Int24 => (24, hound::SampleFormat::Int),
            BitDepth::Float32 => (32, hound::SampleFormat::Float),
        };
        hound::WavSpec {
            // PowerVoice documents are mono (PROMPT §2 LOCKED); read_wav downmixes on the way in.
            channels: 1,
            sample_rate: sample_rate_hz,
            bits_per_sample,
            sample_format,
        }
    }
}

/// A WAV file open for streaming, downmixed-to-mono reads ([`read_wav`]).
pub struct WavSource {
    reader: hound::WavReader<BufReader<File>>,
    channels: u16,
    format: WavFormat,
    /// `2^(bits-1)`, the same integer scale `write_wav`'s dither and `vox_testkit::wav` use, so
    /// import and the measuring stick agree bit for bit (SPEC-005 §2.3).
    int_scale: f64,
}

impl WavSource {
    /// The container facts (rate, channels, bit depth, sample format).
    pub fn format(&self) -> WavFormat {
        self.format
    }

    /// Document sample rate (SPEC-005 §2.3: the document rate is the source rate; no resampling
    /// on open).
    pub fn sample_rate_hz(&self) -> u32 {
        self.format.sample_rate_hz
    }

    /// Original channel count (before downmixing).
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Reads up to `buf.len()` mono frames, downmixing multichannel input by averaging
    /// (SPEC-005 §2.4 default). Returns the number of frames written to the front of `buf`; `0`
    /// means the file is exhausted. A trailing frame with fewer than `channels` samples present
    /// (a truncated file, SPEC-005 §2.5) is dropped rather than returned half-summed.
    ///
    /// Non-finite float samples (NaN/±inf) are replaced by `0.0` (SPEC-005 §2.3): a document never
    /// holds a non-finite sample.
    pub fn read_mono(&mut self, buf: &mut [f32]) -> Result<usize> {
        let channels = usize::from(self.channels).max(1);
        let mut filled = 0;
        for slot in buf.iter_mut() {
            let mut sum = 0.0f64;
            let mut got = 0usize;
            for _ in 0..channels {
                match self.next_raw_sample()? {
                    Some(v) => {
                        sum += v;
                        got += 1;
                    }
                    None => break,
                }
            }
            if got < channels {
                break;
            }
            *slot = (sum / channels as f64) as f32;
            filled += 1;
        }
        Ok(filled)
    }

    fn next_raw_sample(&mut self) -> Result<Option<f64>> {
        match self.format.sample_format {
            SampleFormat::Int => match self.reader.samples::<i32>().next() {
                None => Ok(None),
                Some(Ok(v)) => Ok(Some(f64::from(v) / self.int_scale)),
                Some(Err(e)) => Err(IoError::from(e)),
            },
            SampleFormat::Float => match self.reader.samples::<f32>().next() {
                None => Ok(None),
                Some(Ok(v)) => Ok(Some(if v.is_finite() { f64::from(v) } else { 0.0 })),
                Some(Err(e)) => Err(IoError::from(e)),
            },
        }
    }
}

/// Opens `path` for streaming mono reads: `(sample_rate_hz, original_channels, source)`
/// (SPEC-005 §2.2, §2.3). Only 16/24-bit integer PCM and 32-bit IEEE float are supported; any
/// other WAV variant is [`IoError::Unsupported`] (wider tolerance is symphonia's job, T-202).
pub fn read_wav(path: impl AsRef<Path>) -> Result<(u32, u16, WavSource)> {
    let file = File::open(path.as_ref())?;
    let reader = hound::WavReader::new(BufReader::new(file))?;
    let spec = reader.spec();
    let sample_format = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16 | 24) => SampleFormat::Int,
        (hound::SampleFormat::Float, 32) => SampleFormat::Float,
        (format, bits) => {
            return Err(IoError::Unsupported(format!(
                "{bits}-bit {format:?} (S1-02 supports 16/24-bit int and 32-bit float only)"
            )));
        }
    };
    if spec.channels == 0 {
        return Err(IoError::Unsupported("0 channels".into()));
    }
    let format = WavFormat {
        sample_rate_hz: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: spec.bits_per_sample,
        sample_format,
    };
    let int_scale = (1i64 << (spec.bits_per_sample - 1)) as f64;
    Ok((
        format.sample_rate_hz,
        format.channels,
        WavSource {
            reader,
            channels: spec.channels,
            format,
            int_scale,
        },
    ))
}

/// What [`write_wav`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteReport {
    /// Input samples whose magnitude exceeded 1.0 (silently clamped; SPEC-005 §2.8). Always 0 for
    /// [`BitDepth::Float32`], which never clips.
    pub clipped_samples: usize,
}

/// Writes mono `samples` to `path` as a WAV at `bits` (SPEC-005 §2.6, §2.8), atomically
/// (ADR-004 §8: a temp file next to `path`, `fdatasync`, rename over the target, directory
/// `fsync`). A crash or an error while writing leaves the old file at `path` untouched, or its
/// absence if there was none — `path` is only ever touched by the final rename.
///
/// 16/24-bit writes apply deterministic, seeded TPDF dither per 4096-sample block, skipping
/// blocks that are already exactly representable at `bits` (grid-exact passthrough: an open→save
/// round trip of an unedited file, and digital silence, come out bit-exact — SPEC-005 §2.8).
/// 32-bit float is written bit-exact, never dithered, and never clips.
pub fn write_wav(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: BitDepth,
    samples: &[f32],
) -> Result<WriteReport> {
    let path = path.as_ref();
    let spec = bits.hound_spec(sample_rate_hz);
    let tmp = temp_path_for(path);
    let write_result = write_temp(&tmp, spec, bits, samples);
    let clipped = match write_result {
        Ok(clipped) => clipped,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    // ADR-004 §8: fsync the data, then rename over the target, then fsync the directory. `path`
    // is only ever touched by the rename, so a failure up to here leaves it exactly as it was.
    fsync_path(&tmp)?;
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fsync_dir(parent)?;
    }
    Ok(WriteReport {
        clipped_samples: clipped,
    })
}

fn write_temp(tmp: &Path, spec: hound::WavSpec, bits: BitDepth, samples: &[f32]) -> Result<usize> {
    let file = File::create(tmp)?;
    let mut writer = hound::WavWriter::new(BufWriter::new(file), spec)?;
    let clipped = match bits {
        BitDepth::Float32 => {
            for &s in samples {
                writer.write_sample(if s.is_finite() { s } else { 0.0 })?;
            }
            0
        }
        BitDepth::Int16 => write_dithered(&mut writer, samples, 16)?,
        BitDepth::Int24 => write_dithered(&mut writer, samples, 24)?,
    };
    writer.finalize()?;
    Ok(clipped)
}

fn write_dithered<W: std::io::Write + std::io::Seek>(
    writer: &mut hound::WavWriter<W>,
    samples: &[f32],
    bits: u32,
) -> Result<usize> {
    let (values, clipped) = quantize_dithered(samples, bits);
    for v in values {
        if bits == 16 {
            writer.write_sample(v as i16)?;
        } else {
            writer.write_sample(v)?;
        }
    }
    Ok(clipped)
}

/// `.<file name>.powervoice-tmp-<pid>`, next to `target` (ADR-004 §8).
fn temp_path_for(target: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(target.file_name().unwrap_or_default());
    name.push(format!(".powervoice-tmp-{}", std::process::id()));
    target.with_file_name(name)
}

fn fsync_path(path: &Path) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_all()
        .map_err(IoError::from)
}

fn fsync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        // The file system journals directory changes itself, and a directory can't be opened as
        // a `File` on Windows (ADR-004 §1, `project::fs_util::sync_dir`).
        let _ = dir;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-io-{tag}-{}-{}",
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
    fn atomic_write_leaves_the_old_file_intact_on_failure() {
        let dir = tmp_dir("atomic-fail");
        let path = dir.join("out.wav");
        std::fs::write(&path, b"OLD-CONTENT").unwrap();

        // Sabotage: put a directory where the temp file needs to be created, so `File::create`
        // fails partway through `write_wav` (a stand-in for a simulated disk/IO failure).
        let tmp = temp_path_for(&path);
        std::fs::create_dir(&tmp).unwrap();

        let err = write_wav(&path, 48_000, BitDepth::Int16, &[0.0; 10]).unwrap_err();
        assert!(matches!(err, IoError::Io(_)));
        assert_eq!(std::fs::read(&path).unwrap(), b"OLD-CONTENT");

        std::fs::remove_dir(&tmp).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_wav_leaves_no_temp_file_on_success() {
        let dir = tmp_dir("atomic-ok");
        let path = dir.join("out.wav");
        write_wav(&path, 48_000, BitDepth::Float32, &[0.1, -0.2, 0.3]).unwrap();
        assert!(path.exists());
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.pop().unwrap(), "out.wav");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unsupported_bit_depths() {
        let dir = tmp_dir("unsupported");
        let path = dir.join("in.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        w.write_sample(0i32).unwrap();
        w.finalize().unwrap();

        assert!(matches!(read_wav(&path), Err(IoError::Unsupported(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
