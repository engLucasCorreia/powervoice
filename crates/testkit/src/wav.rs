//! Read/write f32 WAV via `hound`.

use std::io::{self, Cursor, Read, Write};
use std::path::Path;

use crate::error::TestkitError;

/// On-disk bit depth for WAV writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

impl BitDepth {
    fn spec(self, channels: u16, sample_rate: u32) -> hound::WavSpec {
        let (bits_per_sample, sample_format) = match self {
            BitDepth::Int16 => (16, hound::SampleFormat::Int),
            BitDepth::Int24 => (24, hound::SampleFormat::Int),
            BitDepth::Float32 => (32, hound::SampleFormat::Float),
        };
        hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
        }
    }
}

/// On-disk sample encoding: integer PCM or IEEE float. A `testkit`-owned
/// type (rather than re-exporting `hound::SampleFormat`) so that consumers
/// like `voxedit-cli` don't need a direct dependency on `hound` just to
/// read this one field back from [`WavInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    Int,
    Float,
}

impl From<hound::SampleFormat> for SampleFormat {
    fn from(format: hound::SampleFormat) -> Self {
        match format {
            hound::SampleFormat::Int => SampleFormat::Int,
            hound::SampleFormat::Float => SampleFormat::Float,
        }
    }
}

/// Container format facts `analyze` reports alongside the measurements.
#[derive(Debug, Clone, Copy)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub sample_format: SampleFormat,
}

impl WavInfo {
    pub fn duration_s(&self, num_samples: usize) -> f64 {
        let frames = num_samples as f64 / f64::from(self.channels.max(1));
        frames / f64::from(self.sample_rate)
    }
}

/// Converts one `f32` sample (nominal range `[-1.0, 1.0]`) to a signed
/// integer PCM value with `bits`-bit range, using the *same* scale factor
/// (`2^(bits-1)`) an eventual decoder must divide by -- see [`read_wav`].
/// Using mismatched encode/decode factors (e.g. encoding by `i16::MAX`
/// (32767) but decoding by `2^15` (32768), as an earlier version did)
/// introduces a small systematic bias on every round trip. Values are
/// clamped to the representable range *after* rounding, since `1.0 *
/// 2^(bits-1)` itself is one past the top of the range.
fn encode_int_sample(sample: f32, bits: u32) -> i64 {
    let scale = (1i64 << (bits - 1)) as f32;
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let scaled = (sample * scale).round() as i64;
    scaled.clamp(min, max)
}

fn write_samples<W: Write + io::Seek>(
    writer: &mut hound::WavWriter<W>,
    samples: &[f32],
    bits: BitDepth,
) -> Result<usize, TestkitError> {
    let mut clipped = 0usize;
    match bits {
        BitDepth::Float32 => {
            for &s in samples {
                writer.write_sample(s)?;
            }
        }
        BitDepth::Int16 => {
            for &s in samples {
                if s.abs() > 1.0 {
                    clipped += 1;
                }
                writer.write_sample(encode_int_sample(s, 16) as i16)?;
            }
        }
        BitDepth::Int24 => {
            for &s in samples {
                if s.abs() > 1.0 {
                    clipped += 1;
                }
                writer.write_sample(encode_int_sample(s, 24) as i32)?;
            }
        }
    }
    Ok(clipped)
}

/// Writes interleaved `f32` samples (range `[-1.0, 1.0]`) to a WAV file at
/// `path` with the given bit depth. Returns the number of samples that
/// exceeded full scale and were clamped (always `0` for `Float32`, which
/// never clips).
pub fn write_wav_file<P: AsRef<Path>>(
    path: P,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    bits: BitDepth,
) -> Result<usize, TestkitError> {
    let spec = bits.spec(channels, sample_rate);
    let mut writer = hound::WavWriter::create(path, spec)?;
    let clipped = write_samples(&mut writer, samples, bits)?;
    writer.finalize()?;
    Ok(clipped)
}

/// Encodes interleaved `f32` samples as a complete WAV file in memory (used
/// for `-o -` / stdout, since a WAV header requires a seekable writer to
/// patch chunk sizes on finalize). Returns the bytes and the number of
/// samples that exceeded full scale and were clamped (see
/// [`write_wav_file`]).
pub fn encode_wav_bytes(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    bits: BitDepth,
) -> Result<(Vec<u8>, usize), TestkitError> {
    let spec = bits.spec(channels, sample_rate);
    let mut buf: Vec<u8> = Vec::new();
    let clipped = {
        // `Cursor<&mut Vec<u8>>` implements `Write + Seek` directly (it
        // grows the vec on writes past the end, exactly like
        // `Cursor<Vec<u8>>`), so once `writer` (and the cursor's borrow of
        // `buf`) is dropped at the end of this block, `buf` holds the
        // complete WAV bytes -- no need to smuggle a shared buffer out of
        // `hound::WavWriter::finalize`, which does not hand the writer back.
        let cursor = Cursor::new(&mut buf);
        let mut writer = hound::WavWriter::new(cursor, spec)?;
        let clipped = write_samples(&mut writer, samples, bits)?;
        writer.finalize()?;
        clipped
    };
    Ok((buf, clipped))
}

/// Reads an entire WAV stream (file, or any `Read` source such as stdin)
/// into an interleaved `f32` buffer plus its format info.
pub fn read_wav<R: Read>(reader: R) -> Result<(Vec<f32>, WavInfo), TestkitError> {
    let mut wav = hound::WavReader::new(reader)?;
    let spec = wav.spec();
    let info = WavInfo {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: spec.bits_per_sample,
        sample_format: spec.sample_format.into(),
    };
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => wav.samples::<f32>().collect::<hound::Result<Vec<f32>>>()?,
        hound::SampleFormat::Int => {
            // Same `2^(bits-1)` scale as `encode_int_sample` uses to write,
            // so encode/decode round-trip symmetrically.
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            wav.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<hound::Result<Vec<f32>>>()?
        }
    };
    Ok((samples, info))
}

/// Reads a WAV file at `path` into an interleaved `f32` buffer plus its
/// format info.
pub fn read_wav_file<P: AsRef<Path>>(path: P) -> Result<(Vec<f32>, WavInfo), TestkitError> {
    let file = std::fs::File::open(path)?;
    read_wav(io::BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_float32() {
        let samples = crate::signal::sine(1000.0, -6.0, 0.1, 48_000).unwrap();
        let (bytes, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Float32).unwrap();
        assert_eq!(clipped, 0);
        let (read_back, info) = read_wav(Cursor::new(bytes)).unwrap();
        assert_eq!(info.channels, 1);
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.bits_per_sample, 32);
        assert_eq!(samples.len(), read_back.len());
        for (a, b) in samples.iter().zip(read_back.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn round_trips_int16_within_quantization() {
        let samples = crate::signal::sine(1000.0, -6.0, 0.1, 48_000).unwrap();
        let (bytes, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Int16).unwrap();
        assert_eq!(clipped, 0);
        let (read_back, info) = read_wav(Cursor::new(bytes)).unwrap();
        assert_eq!(info.bits_per_sample, 16);
        for (a, b) in samples.iter().zip(read_back.iter()) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn round_trips_stereo() {
        let samples = crate::signal::sine_stereo(1000.0, -23.0, 0.1, 48_000).unwrap();
        let (bytes, _) = encode_wav_bytes(&samples, 2, 48_000, BitDepth::Float32).unwrap();
        let (read_back, info) = read_wav(Cursor::new(bytes)).unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(read_back.len(), samples.len());
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check, not a tolerance comparison
    fn int16_encode_decode_uses_symmetric_scale() {
        // Full-scale +/-1.0 must round-trip to the true integer extremes
        // (i16::MIN / i16::MAX), and 0.0 must round-trip to exactly 0 --
        // this only holds if encode and decode use the same 2^(bits-1)
        // scale factor.
        let samples = [-1.0f32, 0.0, 1.0, 0.5, -0.5];
        let (bytes, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Int16).unwrap();
        assert_eq!(
            clipped, 0,
            "in-range samples must not be reported as clipped"
        );
        let (read_back, _) = read_wav(Cursor::new(bytes)).unwrap();
        assert!((read_back[0] - (-1.0)).abs() < 1e-4);
        assert_eq!(read_back[1], 0.0);
        assert!((read_back[2] - 1.0).abs() < 1e-4);
        assert!((read_back[3] - 0.5).abs() < 1e-4);
        assert!((read_back[4] - (-0.5)).abs() < 1e-4);
    }

    #[test]
    fn int24_encode_decode_round_trips_within_quantization() {
        let samples = crate::signal::sine(1000.0, -6.0, 0.1, 48_000).unwrap();
        let (bytes, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Int24).unwrap();
        assert_eq!(clipped, 0);
        let (read_back, info) = read_wav(Cursor::new(bytes)).unwrap();
        assert_eq!(info.bits_per_sample, 24);
        for (a, b) in samples.iter().zip(read_back.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn out_of_range_samples_are_reported_as_clipped() {
        let samples = [1.5f32, -2.0, 0.5];
        let (_, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Int16).unwrap();
        assert_eq!(clipped, 2);
        let (_, clipped) = encode_wav_bytes(&samples, 1, 48_000, BitDepth::Float32).unwrap();
        assert_eq!(clipped, 0, "float32 never clips");
    }
}
