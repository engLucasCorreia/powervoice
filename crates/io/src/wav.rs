//! WAV read/write through `hound` (SPEC-005 §2.2, §2.3, §2.7, §2.8; ADR-004 §8 atomic save).
//!
//! Scope of this ticket (S1-02): 16-bit and 24-bit integer PCM and 32-bit IEEE float, mono or
//! multichannel-downmixed-to-mono on read, mono on write (PowerVoice edits mono only). Wider
//! tolerance (8-bit, 32-bit int, 64-bit float, A-law/µ-law on read; `cue `/`LIST adtl` markers;
//! FLAC; atomic-save cleanup of stale temp files) is deferred — see the ticket report.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vox_dsp::dither::{DitherMode, StreamingQuantizer};

use crate::atomic::{finish, temp_path_for};
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
/// round trip of an unedited file, and digital silence, come out bit-exact — SPEC-005 §2.8), or
/// plain rounding with no added noise when `dither` is [`DitherMode::None`] (H-20). 32-bit float
/// is written bit-exact, never dithered (whatever `dither` is), and never clips.
pub fn write_wav(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: BitDepth,
    dither: DitherMode,
    samples: &[f32],
) -> Result<WriteReport> {
    write_wav_with_markers(path, sample_rate_hz, bits, dither, samples, &[])
}

/// A marker as read from, or to be written to, a WAV `cue `/`LIST adtl` chunk pair (SPEC-005
/// §2.9). UTF-8 names only (no Windows-1252 read fallback — S2-03 essential-subset deviation,
/// unrelated to H-72). H-72 surfaces a malformed `cue `/`LIST adtl` layout, and out-of-range cue
/// points, as `notice.open.markers_unreadable`/`notice.open.markers_out_of_range`
/// ([`WavMarkersResult`], `read_wav_markers_detailed`) — the audio still opens with no markers
/// (or with the bad ones dropped) either way, only the file's own reader (`document.rs`) has ever
/// been silent about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavMarker {
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: String,
}

/// H-72 (SPEC-005 §2.5/§2.9): result of parsing markers, including what was dropped.
#[derive(Debug, Clone, Default)]
pub struct WavMarkersResult {
    pub markers: Vec<WavMarker>,
    /// SPEC-005 §2.9: "a `cue ` chunk declaring more than 100 000 points counts as malformed" —
    /// its cue points are dropped entirely (`markers` has none from that chunk).
    pub malformed_cue: bool,
    /// SPEC-005 §4.5: "a [`LIST adtl`] sub-chunk overrunning its parent ends parsing, and the
    /// notice applies" — whatever `labl`/`ltxt` sub-chunks were read before the overrun are kept
    /// (SPEC-005 §2.9's "Malformed `LIST adtl`" row: "the audio and cue positions open;
    /// unreadable names become default names").
    pub adtl_malformed: bool,
    /// SPEC-005 §2.9: cue points whose region (`pos_samples..pos_samples + len_samples`) doesn't
    /// fit within the document, dropped rather than clamped (matching `import.rs`'s existing
    /// filter, unchanged by H-72 — see this ticket's report).
    pub out_of_range_count: u32,
}

impl WavMarkersResult {
    /// SPEC-005 §2.5: both the malformed-`cue ` and the malformed-`LIST adtl` rows fire the same
    /// `notice.open.markers_unreadable`.
    pub fn markers_unreadable(&self) -> bool {
        self.malformed_cue || self.adtl_malformed
    }
}

/// SPEC-005 §2.9/`cue_max_points`: a `cue ` chunk declaring more points than this is malformed.
const CUE_MAX_POINTS: u32 = 100_000;

/// Same as [`write_wav`], but also appends `cue `/`LIST adtl` chunks for `markers` (SPEC-005
/// §2.9) after the audio, before the atomic rename. `markers.is_empty()` writes neither chunk.
///
/// Implemented on top of [`WavStreamWriter`] as a single `write_block` call, so this and a
/// streamed save through `WavStreamWriter` directly produce byte-identical output for the same
/// samples (H-02).
pub fn write_wav_with_markers(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    bits: BitDepth,
    dither: DitherMode,
    samples: &[f32],
    markers: &[WavMarker],
) -> Result<WriteReport> {
    let mut writer = WavStreamWriter::create(path, sample_rate_hz, bits, dither)?;
    if let Err(e) = writer.write_block(samples) {
        writer.abort();
        return Err(e);
    }
    writer.finish(markers)
}

/// A WAV file open for streaming, block-at-a-time writes (H-02): a caller with a large or
/// unknown-length source (a save job reading a document through [`crate`]'s callers) feeds it
/// fixed-size blocks — e.g. `vox_project::CHUNK_SAMPLES` (65 536) — instead of collecting the
/// whole document into one buffer first, so memory stays bounded regardless of length.
///
/// 16/24-bit blocks are dithered through a [`StreamingQuantizer`] that persists across calls, so
/// the output is byte-identical to quantizing the whole document in one call ([`write_wav`]) —
/// see [`StreamingQuantizer`]'s contract on block-size alignment between calls. 32-bit float is
/// written bit-exact, never dithered, with no alignment requirement.
///
/// On any error (from [`Self::write_block`] or [`Self::finish`]), the temp file is left in place
/// for the caller to clean up: since the caller drove the read side too (e.g. reading the
/// document failed), the caller — not this writer — knows when the stream is truly abandoned.
/// Call [`Self::abort`] in that case. `path` is only ever touched by [`Self::finish`]'s final
/// rename, so a failure at any point up to and including a failed `finish` leaves it untouched.
pub struct WavStreamWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    tmp: PathBuf,
    path: PathBuf,
    quantizer: Option<(StreamingQuantizer, u32)>,
    clipped: usize,
    scratch: Vec<i32>,
}

impl WavStreamWriter {
    /// Opens `path`'s temp file (ADR-004 §8) and starts a streaming WAV write at `bits`, dithered
    /// per `dither` (H-20: TPDF or plain rounding; irrelevant for [`BitDepth::Float32`], which is
    /// never dithered).
    pub fn create(
        path: impl AsRef<Path>,
        sample_rate_hz: u32,
        bits: BitDepth,
        dither: DitherMode,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let tmp = temp_path_for(&path);
        let spec = bits.hound_spec(sample_rate_hz);
        let write_result = (|| {
            let file = File::create(&tmp)?;
            hound::WavWriter::new(BufWriter::new(file), spec).map_err(IoError::from)
        })();
        let writer = match write_result {
            Ok(w) => w,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        let quantizer = match bits {
            BitDepth::Float32 => None,
            BitDepth::Int16 => Some((StreamingQuantizer::new(16, dither), 16)),
            BitDepth::Int24 => Some((StreamingQuantizer::new(24, dither), 24)),
        };
        Ok(WavStreamWriter {
            writer,
            tmp,
            path,
            quantizer,
            clipped: 0,
            scratch: Vec::new(),
        })
    }

    /// Writes one block of mono samples, in document order. See the struct docs for the
    /// block-alignment contract on 16/24-bit output.
    pub fn write_block(&mut self, samples: &[f32]) -> Result<()> {
        match &mut self.quantizer {
            None => {
                for &s in samples {
                    self.writer
                        .write_sample(if s.is_finite() { s } else { 0.0 })?;
                }
            }
            Some((quantizer, bits)) => {
                self.scratch.clear();
                self.clipped += quantizer.push(samples, &mut self.scratch);
                if *bits == 16 {
                    for &v in &self.scratch {
                        self.writer.write_sample(v as i16)?;
                    }
                } else {
                    for &v in &self.scratch {
                        self.writer.write_sample(v)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Finalizes the WAV data, appends `cue `/`LIST adtl` chunks for `markers` (SPEC-005 §2.9;
    /// empty writes neither chunk), and atomically renames into place (ADR-004 §8: `fdatasync`,
    /// rename, directory `fsync`). On error, removes the temp file and returns without touching
    /// `path`.
    pub fn finish(self, markers: &[WavMarker]) -> Result<WriteReport> {
        let WavStreamWriter {
            writer,
            tmp,
            path,
            clipped,
            ..
        } = self;
        let result = writer
            .finalize()
            .map_err(IoError::from)
            .and_then(|()| append_markers(&tmp, markers));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        finish(&tmp, &path)?;
        Ok(WriteReport {
            clipped_samples: clipped,
        })
    }

    /// Discards the temp file after the caller gives up on the stream (e.g. reading the source
    /// failed partway through): `path` was never touched, so nothing needs to be restored, but
    /// dropping a `WavStreamWriter` without calling either `finish` or `abort` would leak the
    /// temp file.
    pub fn abort(self) {
        let tmp = self.tmp.clone();
        drop(self);
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Reads the `cue `/`LIST adtl` chunks of a WAV file (SPEC-005 §2.9): position (`dwSampleOffset`
/// when `fccChunk = 'data'` and `dwChunkStart = dwBlockStart = 0`, else `dwPosition`), name (the
/// matching `labl`, decoded as UTF-8; empty/whitespace-only or missing becomes "Marker N", 1-based
/// by position), and region length (a matching `ltxt`'s `dwSampleLength`). Ordered by position,
/// then by cue order (a stable sort). Returns an empty list for a file with no `cue ` chunk, and
/// also — quietly — for one whose chunk layout doesn't parse at all (never fails the audio open on
/// its account). No out-of-range filtering (that needs the document length — see
/// [`read_wav_markers_detailed`]); callers that don't already filter by length themselves
/// (`crates/project/src/import.rs`) get unfiltered positions.
pub fn read_wav_markers(path: impl AsRef<Path>) -> Result<Vec<WavMarker>> {
    let bytes = std::fs::read(path.as_ref())?;
    Ok(parse_wav_markers_internal(&bytes, None)
        .unwrap_or_default()
        .markers)
}

/// H-72 (SPEC-005 §2.5/§2.9): like [`read_wav_markers`], but also reports what was dropped and
/// why — a malformed `cue `/`LIST adtl` chunk ([`WavMarkersResult::markers_unreadable`]) or cue
/// points outside `[0, audio_len_samples]` (`out_of_range_count`) — so a caller can post
/// `notice.open.markers_unreadable`/`notice.open.markers_out_of_range` (SPEC-005 §2.5's exact
/// keys). `markers` is already filtered to fit the document, matching `import.rs`'s own filter.
pub fn read_wav_markers_detailed(
    path: impl AsRef<Path>,
    audio_len_samples: u64,
) -> Result<WavMarkersResult> {
    let bytes = std::fs::read(path.as_ref())?;
    Ok(parse_wav_markers_internal(&bytes, Some(audio_len_samples)).unwrap_or_default())
}

/// H-20 (SPEC-005 §2.10): does `path` carry a `LIST INFO`, `bext`, `iXML` or `smpl` chunk — the
/// metadata PowerVoice reads on open but never writes back? A hand-rolled top-level chunk walk
/// (like [`parse_wav_markers`]'s), because symphonia's WAV reader only surfaces `LIST INFO` as
/// metadata and silently skips every chunk type it doesn't know (`crate::decode`'s module docs).
/// `false` for a file that doesn't parse as RIFF/WAVE at all (never fails the caller's flow on its
/// account, same convention as [`read_wav_markers`]).
pub fn wav_has_foreign_metadata(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = std::fs::read(path.as_ref())?;
    Ok(scan_for_foreign_metadata(&bytes))
}

fn scan_for_foreign_metadata(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return false;
    }
    let mut cur = Cursor::new(&bytes[12..]);
    while cur.remaining() >= 8 {
        let Some(id) = cur.tag() else { break };
        let Some(size) = cur.u32().map(|n| n as usize) else {
            break;
        };
        let Some(body) = cur.take(size) else { break };
        if size % 2 == 1 {
            cur.take(1);
        }
        match &id {
            b"bext" | b"iXML" | b"smpl" | b"id3 " | b"ID3 " => return true,
            b"LIST" if body.len() >= 4 && &body[0..4] == b"INFO" => return true,
            _ => {}
        }
    }
    false
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

pub(crate) fn parse_wav_markers_internal(
    bytes: &[u8],
    audio_len_samples: Option<u64>,
) -> Option<WavMarkersResult> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut cues: Vec<RawCue> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut labels: HashMap<u32, String> = HashMap::new();
    let mut region_lengths: HashMap<u32, u32> = HashMap::new();
    let mut malformed_cue = false;
    let mut adtl_malformed = false;

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
                    malformed_cue = true;
                    continue;
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
                    // `remaining() >= 8` guarantees the 4-byte tag and 4-byte size both read.
                    let Some(sub_id) = c.tag() else { break };
                    let Some(sub_size) = c.u32().map(|n| n as usize) else {
                        break;
                    };
                    let Some(sub_body) = c.take(sub_size) else {
                        // SPEC-005 §4.5: "a sub-chunk overrunning its parent ends parsing, and the
                        // notice applies" — whatever labl/ltxt this LIST adtl already yielded
                        // stays (SPEC-005 §2.9's "unreadable names become default names").
                        adtl_malformed = true;
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

    let mut out_of_range_count = 0u32;
    let mut markers: Vec<WavMarker> = cues
        .iter()
        .filter_map(|cue| {
            let pos = if cue.fcc_chunk == *b"data" && cue.chunk_start == 0 && cue.block_start == 0 {
                cue.sample_offset
            } else {
                cue.position
            };
            let pos_samples = u64::from(pos);
            let len_samples = u64::from(region_lengths.get(&cue.id).copied().unwrap_or(0));
            // H-72 (SPEC-005 §2.9): "cue points with a position > document length are dropped" —
            // matches `crates/project/src/import.rs`'s own pre-existing filter
            // (`pos_samples + len_samples <= len`), which this function now does on its behalf
            // (see `read_wav_markers_detailed`'s doc) so both keep exactly one definition of
            // "out of range".
            if let Some(len) = audio_len_samples
                && !pos_samples
                    .checked_add(len_samples)
                    .is_some_and(|end| end <= len)
            {
                out_of_range_count += 1;
                return None;
            }
            let name = labels.get(&cue.id).cloned().unwrap_or_default();
            Some(WavMarker {
                pos_samples,
                len_samples,
                name,
            })
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
    Some(WavMarkersResult {
        markers,
        malformed_cue,
        adtl_malformed,
        out_of_range_count,
    })
}

/// SPEC-005 §2.9/§4.6: UTF-8 is tried first; any invalid byte sequence decodes the whole string
/// as Windows-1252 instead (H-60 — the S2-03 module doc's "skips the Windows-1252 fallback,
/// deferred" note above no longer applies).
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode_windows_1252(bytes),
    }
}

/// Windows-1252 decode table for bytes `0x80..=0x9F` (the WHATWG Encoding Standard's
/// "windows-1252" index for that range; five of them — 0x81/0x8D/0x8F/0x90/0x9D — are undefined in
/// the codepage and map to their own C1 control code point, matching real Windows behavior).
/// `0x00..=0x7F` is plain ASCII and `0xA0..=0xFF` maps to the identical Unicode code point (Latin-1
/// supplement), so only this 32-entry range needs a table.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => WINDOWS_1252_HIGH[(b - 0x80) as usize],
            // `u8 as char`: every byte value 0x00-0x7F and 0xA0-0xFF is already a valid Unicode
            // scalar value at that same code point (ASCII, then the Latin-1 supplement).
            _ => b as char,
        })
        .collect()
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

        let err =
            write_wav(&path, 48_000, BitDepth::Int16, DitherMode::Tpdf, &[0.0; 10]).unwrap_err();
        assert!(matches!(err, IoError::Io(_)));
        assert_eq!(std::fs::read(&path).unwrap(), b"OLD-CONTENT");

        std::fs::remove_dir(&tmp).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_wav_leaves_no_temp_file_on_success() {
        let dir = tmp_dir("atomic-ok");
        let path = dir.join("out.wav");
        write_wav(
            &path,
            48_000,
            BitDepth::Float32,
            DitherMode::Tpdf,
            &[0.1, -0.2, 0.3],
        )
        .unwrap();
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
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int24,
            DitherMode::Tpdf,
            &[0.0; 100_000],
            &markers,
        )
        .unwrap();

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
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int16,
            DitherMode::Tpdf,
            &[0.0; 10],
            &[],
        )
        .unwrap();
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
        write_wav(
            &path,
            48_000,
            BitDepth::Float32,
            DitherMode::Tpdf,
            &[0.1, 0.2, 0.3],
        )
        .unwrap();
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
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int16,
            DitherMode::Tpdf,
            &[0.0; 100],
            &markers,
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_size, bytes.len() - 8);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- H-60: Windows-1252 marker-name fallback (SPEC-005 §2.9/§4.6, AC-8) ------------------

    #[test]
    fn decode_text_falls_back_to_windows_1252_for_invalid_utf8() {
        // SPEC-005 AC-8's own example: byte 0xE9 (invalid on its own as UTF-8) decodes as 'é' —
        // identical in Windows-1252 and Latin-1 for this byte.
        assert_eq!(decode_text(&[0xE9]), "é");
        // A full name mixing ASCII and a high byte: "caf" + 0xE9 -> "café".
        assert_eq!(decode_text(b"caf\xE9"), "café");
    }

    #[test]
    fn decode_text_keeps_valid_utf8_as_is() {
        assert_eq!(decode_text("Café — take 2".as_bytes()), "Café — take 2");
    }

    #[test]
    fn decode_text_maps_windows_1252_specific_punctuation() {
        // 0x93/0x94 are curly double quotes in Windows-1252 (distinct from Latin-1, which has no
        // printable glyph there) — proves the fallback is really CP1252, not plain Latin-1.
        assert_eq!(decode_text(&[0x93, b'x', 0x94]), "\u{201C}x\u{201D}");
    }

    /// A `labl` chunk carrying a raw Windows-1252 byte (0xE9, invalid standalone UTF-8) round
    /// trips to "é" through the real chunk walker, not just the `decode_text` unit above.
    #[test]
    fn read_wav_markers_decodes_a_windows_1252_label() {
        let dir = tmp_dir("cp1252-label");
        let path = dir.join("in.wav");

        // A minimal valid WAV (44-byte header, no samples) with one cue point and one `labl`
        // carrying a raw Windows-1252 byte for the name "caf\xE9" ("café").
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // patched below
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&96_000u32.to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // no audio

        bytes.extend_from_slice(b"cue ");
        bytes.extend_from_slice(&28u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 cue point
        bytes.extend_from_slice(&1u32.to_le_bytes()); // id
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dwPosition
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dwChunkStart
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dwBlockStart
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dwSampleOffset

        let mut labl_body = Vec::new();
        labl_body.extend_from_slice(&1u32.to_le_bytes()); // cue id
        labl_body.extend_from_slice(b"caf\xE9\0"); // Windows-1252 "café", NUL-terminated
        let mut adtl_body = Vec::new();
        adtl_body.extend_from_slice(b"adtl");
        adtl_body.extend_from_slice(b"labl");
        adtl_body.extend_from_slice(&(labl_body.len() as u32).to_le_bytes());
        adtl_body.extend_from_slice(&labl_body);
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&(adtl_body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&adtl_body);

        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();

        let markers = read_wav_markers(&path).unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "café");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- H-02: WavStreamWriter ---------------------------------------------------------------

    /// A document larger than several `CHUNK_SAMPLES`-style blocks, with off-grid, grid-exact and
    /// silent stretches so both dither paths and the RNG-continuation-across-blocks logic are
    /// exercised, streamed in fixed-size blocks that aren't multiples of the block size to make
    /// sure the writer doesn't secretly require alignment on the *caller's* side beyond "every
    /// call but the last is a multiple of 4096" (here every call is 4096-aligned by construction:
    /// see the comment below).
    fn streaming_signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| match i % 11 {
                0 => 0.0,
                1 => ((i * 97) % 65536) as f32 / 32_768.0 - 1.0, // exact 16-bit grid point
                // `.fract()` keeps the sign of its input in Rust, so `.abs()` first to land in
                // [0, 1) before rescaling to (-0.8, 0.8) — well within range, no clipping.
                _ => ((i as f32 * 12.9898).sin() * 43_758.547).fract().abs() * 1.6 - 0.8,
            })
            .collect()
    }

    #[test]
    fn stream_writer_matches_write_wav_int16() {
        let dir = tmp_dir("stream-16");
        let samples = streaming_signal(200_000);

        let whole_path = dir.join("whole.wav");
        write_wav(
            &whole_path,
            48_000,
            BitDepth::Int16,
            DitherMode::Tpdf,
            &samples,
        )
        .unwrap();

        let streamed_path = dir.join("streamed.wav");
        let mut writer =
            WavStreamWriter::create(&streamed_path, 48_000, BitDepth::Int16, DitherMode::Tpdf)
                .unwrap();
        // Blocks are a multiple of DITHER_BLOCK_SAMPLES (4096): 16 * 4096 = 65 536, matching
        // `vox_project::CHUNK_SAMPLES`.
        for block in samples.chunks(65_536) {
            writer.write_block(block).unwrap();
        }
        let report = writer.finish(&[]).unwrap();

        let whole_bytes = std::fs::read(&whole_path).unwrap();
        let streamed_bytes = std::fs::read(&streamed_path).unwrap();
        assert_eq!(
            whole_bytes, streamed_bytes,
            "streaming in blocks must produce byte-identical output to one write_wav call"
        );
        assert_eq!(report.clipped_samples, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// H-20: `DitherMode::None` writes plain-rounded (never dithered) 16-bit data, and the
    /// streaming writer matches a whole-buffer `write_wav` call at that mode too.
    #[test]
    fn write_wav_with_dither_none_matches_plain_rounding_and_streams_identically() {
        let dir = tmp_dir("dither-none");
        let samples = streaming_signal(200_000);

        let whole_path = dir.join("whole.wav");
        write_wav(
            &whole_path,
            48_000,
            BitDepth::Int16,
            DitherMode::None,
            &samples,
        )
        .unwrap();

        let streamed_path = dir.join("streamed.wav");
        let mut writer =
            WavStreamWriter::create(&streamed_path, 48_000, BitDepth::Int16, DitherMode::None)
                .unwrap();
        for block in samples.chunks(65_536) {
            writer.write_block(block).unwrap();
        }
        writer.finish(&[]).unwrap();
        assert_eq!(
            std::fs::read(&whole_path).unwrap(),
            std::fs::read(&streamed_path).unwrap(),
            "streaming must match a whole-buffer write in None mode too"
        );

        // The written int16 samples equal plain rounding of the f32 input — no TPDF noise.
        let (decoded, _) = read_back_samples(&whole_path);
        for (i, (&x, &got)) in samples.iter().zip(decoded.iter()).enumerate() {
            let plain = ((f64::from(x) * 32_768.0).round() / 32_768.0) as f32;
            assert!(
                (got - plain).abs() < 1e-6,
                "sample {i}: got {got}, expected plain rounding {plain}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stream_writer_matches_write_wav_float32() {
        let dir = tmp_dir("stream-32f");
        let samples = streaming_signal(150_000);

        let whole_path = dir.join("whole.wav");
        write_wav(
            &whole_path,
            48_000,
            BitDepth::Float32,
            DitherMode::Tpdf,
            &samples,
        )
        .unwrap();

        let streamed_path = dir.join("streamed.wav");
        let mut writer =
            WavStreamWriter::create(&streamed_path, 48_000, BitDepth::Float32, DitherMode::Tpdf)
                .unwrap();
        for block in samples.chunks(65_536) {
            writer.write_block(block).unwrap();
        }
        writer.finish(&[]).unwrap();

        assert_eq!(
            std::fs::read(&whole_path).unwrap(),
            std::fs::read(&streamed_path).unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stream_writer_markers_survive_a_streamed_write() {
        let dir = tmp_dir("stream-markers");
        let samples = streaming_signal(140_000);
        let markers = vec![
            WavMarker {
                pos_samples: 10,
                len_samples: 0,
                name: "Before first block boundary".into(),
            },
            WavMarker {
                pos_samples: 65_536,
                len_samples: 4_800,
                name: "Exactly on a block boundary".into(),
            },
            WavMarker {
                pos_samples: 130_000,
                len_samples: 0,
                name: "In the final short block".into(),
            },
        ];

        let path = dir.join("out.wav");
        let mut writer =
            WavStreamWriter::create(&path, 48_000, BitDepth::Int24, DitherMode::Tpdf).unwrap();
        for block in samples.chunks(65_536) {
            writer.write_block(block).unwrap();
        }
        writer.finish(&markers).unwrap();

        let read_back = read_wav_markers(&path).unwrap();
        assert_eq!(read_back.len(), 3);
        assert_eq!(read_back[0].name, "Before first block boundary");
        assert_eq!(read_back[1].pos_samples, 65_536);
        assert_eq!(read_back[1].len_samples, 4_800);
        assert_eq!(read_back[2].name, "In the final short block");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stream_writer_abort_removes_the_temp_file_and_leaves_the_target_untouched() {
        let dir = tmp_dir("stream-abort");
        let path = dir.join("out.wav");
        std::fs::write(&path, b"OLD-CONTENT").unwrap();

        let mut writer =
            WavStreamWriter::create(&path, 48_000, BitDepth::Int16, DitherMode::Tpdf).unwrap();
        writer.write_block(&[0.0; 10]).unwrap();
        writer.abort();

        assert_eq!(std::fs::read(&path).unwrap(), b"OLD-CONTENT");
        let tmp = temp_path_for(&path);
        assert!(!tmp.exists(), "abort must remove the temp file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn h72_marker_parse_detects_out_of_range_cues() {
        let dir = tmp_dir("h72-out-of-range");
        // Create a simple WAV with 1000 samples (indices 0-999)
        let samples = vec![0.0; 1000];
        write_wav(
            dir.join("orig.wav"),
            48_000,
            BitDepth::Int16,
            DitherMode::None,
            &samples,
        )
        .unwrap();

        // Write it with markers at various positions
        // Valid: 500 (0-indexed within 0-999)
        // Invalid: 1000 (>= len, out of range), 2000 (>> len, out of range)
        let markers = vec![
            WavMarker {
                pos_samples: 500,
                len_samples: 0,
                name: "Marker 1".to_string(),
            },
            WavMarker {
                pos_samples: 1000, // >= len, out of range
                len_samples: 0,
                name: "Marker 2".to_string(),
            },
            WavMarker {
                pos_samples: 2000, // >> len, out of range
                len_samples: 0,
                name: "Marker 3".to_string(),
            },
        ];
        let path = dir.join("with_markers.wav");
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int16,
            DitherMode::None,
            &samples,
            &markers,
        )
        .unwrap();

        // Parse with audio_len_samples = 1000
        let bytes = std::fs::read(&path).unwrap();
        let result = parse_wav_markers_internal(&bytes, Some(1000)).unwrap();

        // SPEC-005 §2.9: "Cue points with a position > document length are dropped"
        // So with len=1000, positions > 1000 are dropped
        // Position 500: 500 > 1000? No, keep
        // Position 1000: 1000 > 1000? No, keep
        // Position 2000: 2000 > 1000? Yes, drop
        assert_eq!(
            result.markers.len(),
            2,
            "markers at pos <= len should be kept"
        );
        assert_eq!(result.markers[0].pos_samples, 500);
        assert_eq!(result.markers[1].pos_samples, 1000);
        assert_eq!(result.out_of_range_count, 1, "only position 2000 is > 1000");
        assert!(!result.malformed_cue);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn h72_read_wav_markers_detailed_reports_status() {
        let dir = tmp_dir("h72-detailed");
        let samples = vec![0.5; 500];
        let markers = vec![WavMarker {
            pos_samples: 100,
            len_samples: 50,
            name: "Test Marker".to_string(),
        }];
        let path = dir.join("test.wav");
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int16,
            DitherMode::None,
            &samples,
            &markers,
        )
        .unwrap();

        let result = read_wav_markers_detailed(&path, 500).unwrap();
        assert_eq!(result.markers.len(), 1);
        assert_eq!(result.markers[0].pos_samples, 100);
        assert!(!result.malformed_cue);
        assert!(!result.adtl_malformed);
        assert!(!result.markers_unreadable());
        assert_eq!(result.out_of_range_count, 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// H-72 (SPEC-005 §2.9): a marker whose region (`pos_samples..pos_samples + len_samples`)
    /// runs past the document end is dropped even though its start position alone would be
    /// in-range — the region-aware criterion `read_wav_markers_detailed` shares with
    /// `crates/project/src/import.rs`'s pre-existing filter (this ticket only adds the notice,
    /// not a behavior change — see the ticket report).
    #[test]
    fn h72_marker_parse_drops_a_region_that_overruns_the_document_end() {
        let dir = tmp_dir("h72-region-overrun");
        let samples = vec![0.0; 1000];
        let markers = vec![WavMarker {
            pos_samples: 900,
            len_samples: 200, // 900 + 200 = 1100 > 1000: the position alone is in range, the
            // region isn't.
            name: "Region".to_string(),
        }];
        let path = dir.join("region.wav");
        write_wav_with_markers(
            &path,
            48_000,
            BitDepth::Int16,
            DitherMode::None,
            &samples,
            &markers,
        )
        .unwrap();

        let result = read_wav_markers_detailed(&path, 1000).unwrap();
        assert!(
            result.markers.is_empty(),
            "a region overrunning the document end is dropped, not clamped"
        );
        assert_eq!(result.out_of_range_count, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// SPEC-005 §2.5/§2.9: "a `cue ` chunk declaring more than 100 000 points counts as
    /// malformed" — its cue records are dropped entirely and `markers_unreadable()` is true (the
    /// previous agent's `count > 100_000` heuristic was already correct; H-72 confirms it against
    /// the spec text rather than changing it — see the ticket report).
    #[test]
    fn h72_marker_parse_detects_malformed_cue_chunk() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // RIFF size: unchecked past the header
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"cue ");
        bytes.extend_from_slice(&4u32.to_le_bytes()); // chunk size: just the count field
        bytes.extend_from_slice(&(CUE_MAX_POINTS + 1).to_le_bytes());

        let result =
            parse_wav_markers_internal(&bytes, None).expect("a valid RIFF/WAVE header parses");
        assert!(
            result.malformed_cue,
            "count > CUE_MAX_POINTS is malformed (SPEC-005 §2.9)"
        );
        assert!(!result.adtl_malformed);
        assert!(result.markers.is_empty());
        assert!(result.markers_unreadable());
    }

    /// SPEC-005 §4.5: "a sub-chunk overrunning its parent ends parsing, and the notice applies" —
    /// the "Malformed `LIST adtl`" row of SPEC-005 §2.9's table, which the previous agent's
    /// `WavMarkersResult` didn't track at all (no caller could tell a truncated `LIST adtl` from a
    /// clean one).
    #[test]
    fn h72_marker_parse_detects_adtl_overrun() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"LIST");
        let mut list_body = Vec::new();
        list_body.extend_from_slice(b"adtl");
        list_body.extend_from_slice(b"labl");
        list_body.extend_from_slice(&100u32.to_le_bytes()); // claims 100 bytes; none follow
        bytes.extend_from_slice(&(list_body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&list_body);

        let result =
            parse_wav_markers_internal(&bytes, None).expect("a valid RIFF/WAVE header parses");
        assert!(
            result.adtl_malformed,
            "an oversized labl sub-chunk overruns its LIST adtl parent"
        );
        assert!(!result.malformed_cue);
        assert!(result.markers_unreadable());
    }
}
