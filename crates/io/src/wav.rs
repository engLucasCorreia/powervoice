//! WAV read/write through `hound` (SPEC-005 §2.2, §2.3, §2.7, §2.8; ADR-004 §8 atomic save).
//!
//! Scope of this ticket (S1-02): 16-bit and 24-bit integer PCM and 32-bit IEEE float, mono or
//! multichannel-downmixed-to-mono on read, mono on write (PowerVoice edits mono only). Wider
//! tolerance (8-bit, 32-bit int, 64-bit float, A-law/µ-law on read; `cue `/`LIST adtl` markers;
//! FLAC; atomic-save cleanup of stale temp files) is deferred — see the ticket report.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use crate::atomic::{finish, temp_path_for};
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
    write_wav_with_markers(path, sample_rate_hz, bits, samples, &[])
}

/// A marker as read from, or to be written to, a WAV `cue `/`LIST adtl` chunk pair (SPEC-005
/// §2.9). S2-03 essential subset: UTF-8 names only (no Windows-1252 read fallback), and a
/// malformed or oversized `cue `/`LIST adtl` layout is treated as "no markers" rather than
/// surfaced as a notice — both are deferred to hardening (ticket report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavMarker {
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: String,
}

/// SPEC-005 §2.9/`cue_max_points`: a `cue ` chunk declaring more points than this is malformed.
const CUE_MAX_POINTS: u32 = 100_000;

/// Same as [`write_wav`], but also appends `cue `/`LIST adtl` chunks for `markers` (SPEC-005
/// §2.9) after the audio, before the atomic rename. `markers.is_empty()` writes neither chunk.
pub fn write_wav_with_markers(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: BitDepth,
    samples: &[f32],
    markers: &[WavMarker],
) -> Result<WriteReport> {
    let path = path.as_ref();
    let spec = bits.hound_spec(sample_rate_hz);
    let tmp = temp_path_for(path);
    let write_result = write_temp(&tmp, spec, bits, samples).and_then(|clipped| {
        append_markers(&tmp, markers)?;
        Ok(clipped)
    });
    let clipped = match write_result {
        Ok(clipped) => clipped,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    // ADR-004 §8: fsync the data, then rename over the target, then fsync the directory. `path`
    // is only ever touched by the rename, so a failure up to here leaves it exactly as it was.
    finish(&tmp, path)?;
    Ok(WriteReport {
        clipped_samples: clipped,
    })
}

/// Reads the `cue `/`LIST adtl` chunks of a WAV file (SPEC-005 §2.9): position (`dwSampleOffset`
/// when `fccChunk = 'data'` and `dwChunkStart = dwBlockStart = 0`, else `dwPosition`), name (the
/// matching `labl`, decoded as UTF-8; empty/whitespace-only or missing becomes "Marker N", 1-based
/// by position), and region length (a matching `ltxt`'s `dwSampleLength`). Ordered by position,
/// then by cue order (a stable sort). Returns an empty list for a file with no `cue ` chunk, and
/// also — quietly, S2-03 scope — for one whose chunk layout doesn't parse: a malformed `cue `/
/// `LIST adtl` never fails the audio open on its account (full notice/error reporting is
/// hardening, deferred).
pub fn read_wav_markers(path: impl AsRef<Path>) -> Result<Vec<WavMarker>> {
    let bytes = std::fs::read(path.as_ref())?;
    Ok(parse_wav_markers(&bytes).unwrap_or_default())
}

/// A byte cursor over one RIFF chunk's body, used by [`parse_wav_markers`].
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.remaining() < n {
            return None;
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Some(slice)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }

    fn tag(&mut self) -> Option<[u8; 4]> {
        self.take(4).map(|b| b.try_into().unwrap())
    }
}

#[derive(Clone, Copy)]
struct RawCue {
    id: u32,
    position: u32,
    fcc_chunk: [u8; 4],
    chunk_start: u32,
    block_start: u32,
    sample_offset: u32,
}

fn parse_wav_markers(bytes: &[u8]) -> Option<Vec<WavMarker>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut cues: Vec<RawCue> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut labels: HashMap<u32, String> = HashMap::new();
    let mut region_lengths: HashMap<u32, u32> = HashMap::new();

    let mut cur = Cursor::new(&bytes[12..]);
    while cur.remaining() >= 8 {
        let id = cur.tag()?;
        let size = cur.u32()? as usize;
        let Some(body) = cur.take(size) else { break };
        if size % 2 == 1 {
            cur.take(1);
        }
        match &id {
            b"cue " => {
                let mut c = Cursor::new(body);
                let Some(count) = c.u32() else { continue };
                if count > CUE_MAX_POINTS {
                    return None;
                }
                for _ in 0..count {
                    let Some(cue_id) = c.u32() else { break };
                    let Some(position) = c.u32() else { break };
                    let Some(fcc_chunk) = c.tag() else { break };
                    let Some(chunk_start) = c.u32() else { break };
                    let Some(block_start) = c.u32() else { break };
                    let Some(sample_offset) = c.u32() else { break };
                    if seen_ids.insert(cue_id) {
                        cues.push(RawCue {
                            id: cue_id,
                            position,
                            fcc_chunk,
                            chunk_start,
                            block_start,
                            sample_offset,
                        });
                    }
                }
            }
            b"LIST" => {
                let mut c = Cursor::new(body);
                if c.tag() != Some(*b"adtl") {
                    continue;
                }
                while c.remaining() >= 8 {
                    let Some(sub_id) = c.tag() else { break };
                    let Some(sub_size) = c.u32().map(|n| n as usize) else {
                        break;
                    };
                    let Some(sub_body) = c.take(sub_size) else {
                        break;
                    };
                    if sub_size % 2 == 1 {
                        c.take(1);
                    }
                    match &sub_id {
                        b"labl" if sub_body.len() >= 4 => {
                            let cue_id = u32::from_le_bytes(sub_body[0..4].try_into().unwrap());
                            let text_bytes = &sub_body[4..];
                            let nul = text_bytes
                                .iter()
                                .position(|&b| b == 0)
                                .unwrap_or(text_bytes.len());
                            labels.insert(cue_id, decode_text(&text_bytes[..nul]));
                        }
                        b"ltxt" if sub_body.len() >= 20 => {
                            let cue_id = u32::from_le_bytes(sub_body[0..4].try_into().unwrap());
                            let sample_length =
                                u32::from_le_bytes(sub_body[4..8].try_into().unwrap());
                            region_lengths.insert(cue_id, sample_length);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let mut markers: Vec<WavMarker> = cues
        .iter()
        .map(|cue| {
            let pos = if cue.fcc_chunk == *b"data" && cue.chunk_start == 0 && cue.block_start == 0 {
                cue.sample_offset
            } else {
                cue.position
            };
            let len = region_lengths.get(&cue.id).copied().unwrap_or(0);
            let name = labels.get(&cue.id).cloned().unwrap_or_default();
            WavMarker {
                pos_samples: u64::from(pos),
                len_samples: u64::from(len),
                name,
            }
        })
        .collect();
    // SPEC-005 §2.9: ordered by position, then cue order — a stable sort preserves the cue-record
    // order (`cues`' push order) for ties.
    markers.sort_by_key(|m| m.pos_samples);
    for (i, marker) in markers.iter_mut().enumerate() {
        if marker.name.trim().is_empty() {
            marker.name = format!("Marker {}", i + 1);
        }
    }
    Some(markers)
}

/// UTF-8 first (SPEC-005 §4.6); this ticket's essential subset skips the Windows-1252 fallback
/// (deferred — never-invalid `from_utf8_lossy` keeps this function total either way).
fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Appends `cue `/`LIST adtl` chunks to the still-open temp file `tmp` (before it is fsynced and
/// renamed into place) and patches the RIFF size at offset 4. A no-op when `markers` is empty
/// (SPEC-005 §2.9: "no markers → no `cue ` and no `LIST adtl` chunk").
fn append_markers(tmp: &Path, markers: &[WavMarker]) -> Result<()> {
    if markers.is_empty() {
        return Ok(());
    }
    let mut buf = Vec::new();
    write_cue_chunk(&mut buf, markers);
    write_adtl_chunk(&mut buf, markers);

    let mut file = OpenOptions::new().read(true).write(true).open(tmp)?;
    file.seek(SeekFrom::End(0))?;
    file.write_all(&buf)?;
    let file_len = file.stream_position()?;
    let riff_size = u32::try_from(file_len - 8)
        .map_err(|_| IoError::InvalidArgument("WAV exceeds the 4 GiB RIFF size limit"))?;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;
    Ok(())
}

/// Writes `cue ` (SPEC-005 §2.9): ids `1..=markers.len()` in position order (the writer always
/// receives markers in canonical, position-sorted order), `fccChunk = 'data'`,
/// `dwChunkStart = dwBlockStart = 0`, so `dwPosition = dwSampleOffset = ` the frame index.
fn write_cue_chunk(buf: &mut Vec<u8>, markers: &[WavMarker]) {
    buf.extend_from_slice(b"cue ");
    let size = 4 + 24 * markers.len();
    buf.extend_from_slice(&(size as u32).to_le_bytes());
    buf.extend_from_slice(&(markers.len() as u32).to_le_bytes());
    for (i, m) in markers.iter().enumerate() {
        let id = (i + 1) as u32;
        let pos = m.pos_samples.min(u64::from(u32::MAX)) as u32;
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&pos.to_le_bytes()); // dwPosition
        buf.extend_from_slice(b"data"); // fccChunk
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwChunkStart
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwBlockStart
        buf.extend_from_slice(&pos.to_le_bytes()); // dwSampleOffset
    }
    // 4 + 24n is always even: no pad byte needed.
}

/// Writes `LIST adtl` (SPEC-005 §2.9): a `labl` per marker (UTF-8 name, NUL-terminated) and an
/// `ltxt` per region (`dwSampleLength`, purpose `'rgn '`).
fn write_adtl_chunk(buf: &mut Vec<u8>, markers: &[WavMarker]) {
    let mut body = Vec::new();
    body.extend_from_slice(b"adtl");
    for (i, m) in markers.iter().enumerate() {
        let id = (i + 1) as u32;
        write_labl(&mut body, id, &m.name);
        if m.len_samples > 0 {
            write_ltxt(&mut body, id, m.len_samples.min(u64::from(u32::MAX)) as u32);
        }
    }
    buf.extend_from_slice(b"LIST");
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&body);
}

fn write_labl(body: &mut Vec<u8>, id: u32, name: &str) {
    let name_bytes = name.as_bytes();
    let mut data = Vec::with_capacity(4 + name_bytes.len() + 1);
    data.extend_from_slice(&id.to_le_bytes());
    data.extend_from_slice(name_bytes);
    data.push(0); // NUL terminator
    body.extend_from_slice(b"labl");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        body.push(0); // pad byte
    }
}

fn write_ltxt(body: &mut Vec<u8>, id: u32, sample_length: u32) {
    body.extend_from_slice(b"ltxt");
    body.extend_from_slice(&20u32.to_le_bytes());
    body.extend_from_slice(&id.to_le_bytes());
    body.extend_from_slice(&sample_length.to_le_bytes());
    body.extend_from_slice(b"rgn "); // dwPurpose (Sound Forge/Audition convention, SPEC-005 §2.9)
    body.extend_from_slice(&0u16.to_le_bytes()); // wCountry
    body.extend_from_slice(&0u16.to_le_bytes()); // wLanguage
    body.extend_from_slice(&0u16.to_le_bytes()); // wDialect
    body.extend_from_slice(&0u16.to_le_bytes()); // wCodePage
    // Fixed 20-byte body (no text): always even, no pad byte needed.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    // --- S2-03: cue/adtl markers ------------------------------------------------------------

    #[test]
    fn cue_marker_round_trip_preserves_positions_names_and_regions() {
        let dir = tmp_dir("cue-round-trip");
        let path = dir.join("markers.wav");
        let markers = vec![
            WavMarker {
                pos_samples: 0,
                len_samples: 0,
                name: "Intro".into(),
            },
            WavMarker {
                pos_samples: 48_000,
                len_samples: 4_800,
                name: "Café — take 2".into(), // UTF-8, SPEC-005 AC-7
            },
            WavMarker {
                pos_samples: 96_000,
                len_samples: 0,
                name: "".into(), // empty name -> "Marker N" on read
            },
        ];
        write_wav_with_markers(&path, 48_000, BitDepth::Int24, &[0.0; 100_000], &markers).unwrap();

        let read_back = read_wav_markers(&path).unwrap();
        assert_eq!(read_back.len(), 3);
        assert_eq!(read_back[0].pos_samples, 0);
        assert_eq!(read_back[0].len_samples, 0);
        assert_eq!(read_back[0].name, "Intro");
        assert_eq!(read_back[1].pos_samples, 48_000);
        assert_eq!(read_back[1].len_samples, 4_800);
        assert_eq!(read_back[1].name, "Café — take 2");
        assert_eq!(read_back[2].pos_samples, 96_000);
        assert_eq!(read_back[2].len_samples, 0);
        assert_eq!(
            read_back[2].name, "Marker 3",
            "an empty name becomes Marker N"
        );

        // Audio is untouched by appending markers.
        let (decoded, _) = read_back_samples(&path);
        assert_eq!(decoded.len(), 100_000);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn read_back_samples(path: &Path) -> (Vec<f32>, WavFormat) {
        let (_, _, mut source) = read_wav(path).unwrap();
        let format = source.format();
        let mut buf = vec![0.0f32; 200_000];
        let n = source.read_mono(&mut buf).unwrap();
        buf.truncate(n);
        (buf, format)
    }

    #[test]
    fn no_markers_writes_no_cue_or_adtl_chunk() {
        let dir = tmp_dir("no-markers");
        let path = dir.join("plain.wav");
        write_wav_with_markers(&path, 48_000, BitDepth::Int16, &[0.0; 10], &[]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!contains(&bytes, b"cue "));
        assert!(!contains(&bytes, b"LIST"));
        assert!(read_wav_markers(&path).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn read_wav_markers_of_a_plain_file_with_no_cue_chunk_is_empty() {
        let dir = tmp_dir("plain-no-markers");
        let path = dir.join("plain.wav");
        write_wav(&path, 48_000, BitDepth::Float32, &[0.1, 0.2, 0.3]).unwrap();
        assert!(read_wav_markers(&path).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_wav_markers_of_a_non_wav_file_is_empty_not_an_error() {
        let dir = tmp_dir("garbage");
        let path = dir.join("garbage.wav");
        std::fs::write(&path, b"not a riff file at all").unwrap();
        assert!(read_wav_markers(&path).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn riff_size_is_patched_to_include_the_appended_chunks() {
        let dir = tmp_dir("riff-size");
        let path = dir.join("markers.wav");
        let markers = vec![WavMarker {
            pos_samples: 10,
            len_samples: 0,
            name: "M".into(),
        }];
        write_wav_with_markers(&path, 48_000, BitDepth::Int16, &[0.0; 100], &markers).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_size, bytes.len() - 8);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
