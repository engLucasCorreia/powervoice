//! Universal audio decode via `symphonia` (ADR-007 §3 Amendment 2; SPEC-005 §2.2-2.4, §4.1-4.2).
//!
//! Symphonia 0.6.1's own WAV reader (`symphonia-format-riff`) already decodes every SPEC-005
//! §2.2 "tolerated" WAV variant this ticket needs — 8-bit unsigned, 32-bit signed, 64-bit float,
//! A-law, µ-law, and any of those inside `WAVE_FORMAT_EXTENSIBLE` with an arbitrary channel mask
//! (verified by reading `symphonia-format-riff`'s chunk parser: it maps every one of those format
//! tags/sub-format GUIDs to a registered `pcm` codec). No custom RIFF sample-format readers are
//! needed; only the hand-rolled `cue `/`LIST adtl` chunk walker in [`crate::wav`] stays, because
//! symphonia's format reader silently skips chunks it doesn't know.
//!
//! RF64/BW64 (`ds64`) is **not** implemented by `symphonia-format-riff` (no trace of it in the
//! source): such files are rejected, per SPEC-005 §2.2's "only if symphonia's WAV reader decodes
//! it" clause.
//!
//! **Encoder delay/padding** (SPEC-005 §2.3): `Track::delay`/`Track::padding` are populated only
//! by the MP3 reader, from a LAME/Xing "Info" tag (`symphonia-bundle-mp3`); WAV, FLAC, Vorbis and
//! M4A never set them (M4A's `elst`/edit-list atoms are parsed structurally by
//! `symphonia-format-isomp4` but never turned into `Track` delay/padding in this version — a
//! known gap, reported rather than worked around). Trimming itself needs no code here:
//! [`AudioDecoderOptions::gapless`] defaults to `true`, and decoders that know their delay/padding
//! (MP3) trim the decoded buffer themselves via the packet's `trim_start`/`trim_end` before this
//! module ever sees it, so a plain `decode()` call already returns exactly the playable samples.
//!
//! **M4A HE-AAC/ALAC**: `symphonia-codec-aac` detects SBR extension data but ignores it (decodes
//! only the AAC-LC core, at half the intended sample rate — a documented limitation, not
//! detectable from `AudioCodecParameters`, so it cannot be turned into a distinct error). ALAC is
//! parsed structurally by `symphonia-format-isomp4` (so the file opens as a container) but no ALAC
//! *decoder* is registered (the `alac` feature isn't enabled), so `make_audio_decoder` cleanly
//! fails with [`IoError::UnsupportedCodec`] — matching SPEC-005's "rejected if the decoder refuses
//! it".
//!
//! **Ogg Opus**: rejected the same way — `symphonia-format-ogg` demuxes the container, but with no
//! `opus` feature there is no decoder for it. **Chained Ogg** (a second logical stream, a new BOS
//! with a different serial) surfaces as extra [`symphonia::core::formats::Track`]s; this module
//! rejects a file whose audio tracks disagree on rate or channel count
//! ([`IoError::ChainedStreamChanged`], SPEC-005 §2.5), and a mid-stream `ResetRequired` (rate/
//! channel change symphonia itself detects while decoding) maps to the same error.
//!
//! **T-202/H-14 history:** `crate::flac::write_flac`'s own output used to be unprobeable by this
//! module (`DecodeSource::open` failing with `IoError::Io(UnexpectedEof)` on
//! `symphonia-bundle-flac`'s sample-count cross-check). H-14 root-caused this to `flacenc` folding
//! the final, shorter frame's block size into STREAMINFO's `min_block_size` instead of leaving it
//! pinned at the configured block size (see `crate::flac`'s module docs for the full analysis);
//! `write_flac` now corrects that field before writing, and this module decodes PowerVoice's own
//! FLAC exports the same as any other FLAC file. `write_flac` also verifies every export through
//! this exact decode path before the atomic rename (SPEC-005 §2.11).

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::{Channels, GenericAudioBufferRef};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, Track};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::downmix::{ChannelInfo, ChannelPeakAccumulator, IdenticalChannelsCheck, channel_infos};
use crate::error::{IoError, Result};

/// Facts about the chosen audio track (SPEC-005 §2.3 "Probe").
#[derive(Debug, Clone)]
pub struct TrackFacts {
    pub sample_rate_hz: u32,
    /// One entry per source channel, in container order (SPEC-005 §2.4).
    pub channels: Vec<ChannelInfo>,
    /// Exact when the container states a sample count (WAV, FLAC); `None` when it doesn't (MP3
    /// without a Xing/Info frame count, Ogg Vorbis, M4A) — SPEC-005 §2.3's "estimated for formats
    /// without a sample count" is left as `None` here rather than guessed from bitrate.
    pub len_samples: Option<u64>,
    /// From the container (MP3's LAME/Xing tag only, see the module docs); informational — the
    /// decoder has already trimmed by these values (`AudioDecoderOptions::gapless`, always on).
    pub delay_samples: Option<u32>,
    pub padding_samples: Option<u32>,
    /// H-20 (SPEC-005 §2.6): the codec's reported bit depth (`AudioCodecParameters
    /// ::bits_per_sample`) — populated for WAV PCM (every tolerated variant, §2.2) and FLAC (from
    /// STREAMINFO); `None` for lossy codecs, which don't need it (SPEC-005 §2.6's save-format
    /// table only distinguishes WAV/FLAC bit depths).
    pub bits_per_sample: Option<u32>,
}

/// What [`probe`]/[`DecodeSource::open`] found (SPEC-005 §2.3 step 1, `document_probe`).
#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub container: &'static str,
    pub codec: &'static str,
    pub track: TrackFacts,
    /// H-20 (SPEC-005 §2.10): the source carries metadata PowerVoice drops on save (`LIST INFO`,
    /// `bext`, `iXML`, `smpl`, ID3/Vorbis comments) — drives `notice.save.metadata_dropped`.
    pub has_foreign_metadata: bool,
}

/// Probes `path` without decoding any audio (SPEC-005 §2.3: "≤ 200 ms for a local file").
pub fn probe(path: &Path) -> Result<ProbeInfo> {
    DecodeSource::open(path).map(|(info, _)| info)
}

/// A file open for streaming, per-channel (not downmixed) decode (SPEC-005 §4.2's decode loop).
pub struct DecodeSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    channels: usize,
    /// Interleaved samples decoded from the current packet, not yet handed to a caller.
    buf: Vec<f32>,
    cursor: usize,
    eof: bool,
    damaged_packets: usize,
    total_packets: usize,
    non_finite_replaced: usize,
}

fn map_open_error(err: SymError) -> IoError {
    match err {
        SymError::Unsupported(msg) => IoError::UnrecognizedFormat(msg.to_string()),
        SymError::IoError(e) => IoError::Io(e),
        other => IoError::UnrecognizedFormat(other.to_string()),
    }
}

fn map_fatal_error(err: SymError) -> IoError {
    match err {
        SymError::ResetRequired => IoError::ChainedStreamChanged,
        SymError::Unsupported(msg) => IoError::UnsupportedCodec(msg.to_string()),
        SymError::IoError(e) => IoError::Io(e),
        other => IoError::UnrecognizedFormat(other.to_string()),
    }
}

fn open_format(path: &Path) -> Result<Box<dyn FormatReader>> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(map_open_error)
}

/// The `(rate, channel-count)` of an audio track's codec parameters, for the chained-stream
/// mismatch check.
fn track_shape(track: &Track) -> Option<(Option<u32>, usize)> {
    let params = track.codec_params.as_ref()?.audio()?;
    Some((
        params.sample_rate,
        params.channels.as_ref().map_or(0, Channels::count),
    ))
}

impl DecodeSource {
    /// Opens `path`, probing it and preparing (but not yet running) the decode of its first audio
    /// track (SPEC-005 §4.2 steps 1-2).
    pub fn open(path: &Path) -> Result<(ProbeInfo, DecodeSource)> {
        let mut format = open_format(path)?;
        let audio_tracks: Vec<&Track> = format
            .tracks()
            .iter()
            .filter(|t| {
                t.codec_params
                    .as_ref()
                    .and_then(CodecParameters::audio)
                    .is_some()
            })
            .collect();
        let Some(&track) = audio_tracks.first() else {
            return Err(IoError::NoAudioTrack);
        };
        if let Some(first_shape) = track_shape(track) {
            for other in &audio_tracks[1..] {
                if track_shape(other) != Some(first_shape) {
                    return Err(IoError::ChainedStreamChanged);
                }
            }
        }
        let track_id = track.id;
        let delay_samples = track.delay;
        let padding_samples = track.padding;
        let len_samples = track.num_frames;
        let params = track
            .codec_params
            .as_ref()
            .and_then(CodecParameters::audio)
            .cloned()
            .ok_or(IoError::NoAudioTrack)?;
        let sample_rate_hz = params
            .sample_rate
            .ok_or_else(|| IoError::UnrecognizedFormat("no sample rate".into()))?;
        let (channel_count, mask): (u16, Option<u32>) = match &params.channels {
            Some(Channels::Positioned(pos)) => {
                let bits = pos.bits();
                (bits.count_ones() as u16, Some(bits as u32))
            }
            Some(other) => (other.count() as u16, None),
            None => return Err(IoError::UnrecognizedFormat("no channel layout".into())),
        };
        if channel_count == 0 {
            return Err(IoError::UnrecognizedFormat("0 channels".into()));
        }
        let channels = channel_infos(channel_count, mask);

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(map_fatal_error)?;
        let codec = decoder.codec_info().short_name;
        let container = format.format_info().short_name;
        let bits_per_sample = params.bits_per_sample;
        // H-20 (SPEC-005 §2.10): does the container carry metadata PowerVoice doesn't preserve?
        // `format.metadata()` catches Vorbis comments (FLAC/Ogg) and ID3 (MP3/M4A) uniformly;
        // WAV's `LIST INFO`, `bext`, `iXML` and `smpl` chunks need the raw scan below, because
        // symphonia's WAV reader only surfaces `LIST INFO` here and silently skips the rest (this
        // module's own docs, "hound has no cue/adtl support" applies equally to those chunks).
        let has_foreign_metadata = format
            .metadata()
            .current()
            .is_some_and(|rev| !rev.media.tags.is_empty() || !rev.media.visuals.is_empty())
            || (container == "wave" && crate::wav::wav_has_foreign_metadata(path).unwrap_or(false));

        let source = DecodeSource {
            format,
            decoder,
            track_id,
            channels: channel_count as usize,
            buf: Vec::new(),
            cursor: 0,
            eof: false,
            damaged_packets: 0,
            total_packets: 0,
            non_finite_replaced: 0,
        };
        let info = ProbeInfo {
            container,
            codec,
            has_foreign_metadata,
            track: TrackFacts {
                sample_rate_hz,
                channels,
                len_samples,
                delay_samples,
                padding_samples,
                bits_per_sample,
            },
        };
        Ok((info, source))
    }

    /// The source's channel count (matches the probe's `track.channels.len()`).
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Damaged packets skipped so far (SPEC-005 §2.5).
    pub fn damaged_packets(&self) -> usize {
        self.damaged_packets
    }

    /// Packets seen so far (damaged + good), for the `damaged_packet_limit` ratio check.
    pub fn total_packets(&self) -> usize {
        self.total_packets
    }

    /// Non-finite samples replaced by `0.0` so far (SPEC-005 §2.3).
    pub fn non_finite_replaced(&self) -> usize {
        self.non_finite_replaced
    }

    /// Decodes the next packet for our track into `self.buf`, skipping packets that belong to
    /// another track and damaged packets (SPEC-005 §2.5: "each damaged packet is replaced by
    /// silence of its duration when known, else skipped"). `Ok(true)` means `self.buf` holds a
    /// (possibly empty) freshly decoded frame set; `Ok(false)` means the stream ended.
    fn decode_next_packet(&mut self) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(e) => return Err(map_fatal_error(e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            self.total_packets += 1;
            match self.decoder.decode(&packet) {
                Ok(audio_buf) => {
                    self.buf.clear();
                    copy_interleaved(&audio_buf, &mut self.buf);
                    for s in self.buf.iter_mut() {
                        if !s.is_finite() {
                            *s = 0.0;
                            self.non_finite_replaced += 1;
                        }
                    }
                    return Ok(true);
                }
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    // "damaged packet with a known duration is replaced by silence of that
                    // duration": symphonia doesn't hand us a duration for a packet it failed to
                    // decode, so we skip it (timing drifts by at most one packet, which the
                    // caller-level truncation/damage notices already account for).
                    self.damaged_packets += 1;
                    continue;
                }
                Err(e) => return Err(map_fatal_error(e)),
            }
        }
    }

    /// Reads up to `out.len() / channels()` interleaved frames into `out` (`out.len()` must be a
    /// multiple of `channels()`). Returns the number of frames written; `0` means end of stream.
    pub fn read_frames(&mut self, out: &mut [f32]) -> Result<usize> {
        let channels = self.channels.max(1);
        debug_assert_eq!(out.len() % channels, 0);
        let want = out.len();
        let mut got = 0usize;
        while got < want {
            if self.cursor >= self.buf.len() {
                if self.eof {
                    break;
                }
                if self.decode_next_packet()? {
                    // Only a *successful* decode replaced `self.buf` with fresh data — until
                    // then `self.buf` may still hold the previous (fully-consumed) packet, and
                    // resetting the cursor early would let it be served a second time.
                    self.cursor = 0;
                    continue;
                }
                self.eof = true;
                self.buf.clear();
                self.cursor = 0;
                break;
            }
            let avail = self.buf.len() - self.cursor;
            let take = avail.min(want - got);
            out[got..got + take].copy_from_slice(&self.buf[self.cursor..self.cursor + take]);
            self.cursor += take;
            got += take;
        }
        Ok(got / channels)
    }
}

/// `GenericAudioBufferRef::copy_to_vec_interleaved` for every sample format symphonia gives us
/// (SPEC-005 §2.3's conversion table — verified against `symphonia_core::audio::conv`: 16/24-bit
/// int -> `v/2^(bits-1)`, 8-bit unsigned -> `(u-128)/128`, 32-bit int -> `v/2^31` via an `f64`
/// intermediate then rounded to `f32`, 32-bit float copied bit-exact, 64-bit float rounded to the
/// nearest `f32`, A-law/µ-law expanded by `symphonia-codec-pcm`'s G.711 tables to `i16` then
/// `/2^15` — every one of these is a plain widening/narrowing numeric conversion with no dither
/// applied on the way to `f32`).
fn copy_interleaved(buf: &GenericAudioBufferRef<'_>, out: &mut Vec<f32>) {
    buf.copy_to_vec_interleaved(out);
}

/// What a channel probe (SPEC-005 §2.4) found over `min(probe_window_s, file length)`.
#[derive(Debug, Clone)]
pub struct ChannelProbeResult {
    /// One peak per source channel, in container order (dBFS; `f32::NEG_INFINITY` for silence).
    pub peaks_dbfs: Vec<f32>,
    /// `true` when every channel was bit-identical over the window (SPEC-005 §2.4).
    pub identical: bool,
}

/// Decodes up to `min(window_s, file length)` of `path` and returns each channel's peak plus
/// whether every channel was bit-identical (SPEC-005 §2.4, §4.3). Mono files trivially report one
/// peak and `identical: false` (there's only one channel — no downmix choice applies).
pub fn probe_channels(path: &Path, window_s: f64) -> Result<ChannelProbeResult> {
    let (info, mut source) = DecodeSource::open(path)?;
    let channels = info.track.channels.len().max(1);
    let window_frames = (window_s * f64::from(info.track.sample_rate_hz)).round() as u64;
    let cap = info
        .track
        .len_samples
        .unwrap_or(u64::MAX)
        .min(window_frames);

    let mut peaks = ChannelPeakAccumulator::new(channels);
    let mut identical = IdenticalChannelsCheck::new(channels);
    const BATCH_FRAMES: usize = 4096;
    let mut buf = vec![0f32; BATCH_FRAMES * channels];
    let mut done = 0u64;
    while done < cap {
        let want_frames = (cap - done).min(BATCH_FRAMES as u64) as usize;
        let n = source.read_frames(&mut buf[..want_frames * channels])?;
        if n == 0 {
            break;
        }
        for frame in buf[..n * channels].chunks_exact(channels) {
            peaks.push_frame(frame);
            identical.push_frame(frame);
        }
        done += n as u64;
    }
    Ok(ChannelProbeResult {
        peaks_dbfs: peaks.finish().into_iter().map(|p| p.peak_dbfs).collect(),
        identical: identical.all_identical(),
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // deliberate bit-exact conversion checks throughout
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-io-decode-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A minimal hand-built RIFF/WAVE writer, for WAV variants `hound` can't write (SPEC-005 §2.2
    /// AC-1: "hand-built WAV files"). `fmt_extra` is appended after the base 16-byte `fmt ` body
    /// (empty for plain PCM/float, or a `cbSize` + extension for `WAVE_FORMAT_EXTENSIBLE`).
    struct RawWavBuilder {
        format_tag: u16,
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
        fmt_extra: Vec<u8>,
        data: Vec<u8>,
        /// H-20: raw chunks appended after `data` (e.g. `bext`, `LIST INFO`), each already
        /// including its own 8-byte tag+size header and pad byte.
        trailing_chunks: Vec<u8>,
    }

    impl RawWavBuilder {
        fn new(format_tag: u16, channels: u16, sample_rate: u32, bits_per_sample: u16) -> Self {
            RawWavBuilder {
                format_tag,
                channels,
                sample_rate,
                bits_per_sample,
                fmt_extra: Vec::new(),
                data: Vec::new(),
                trailing_chunks: Vec::new(),
            }
        }

        /// H-20: appends a raw chunk after `data` (SPEC-005 §2.10's `bext`/`LIST INFO`/etc test
        /// fixtures — `id` must be exactly 4 bytes, e.g. `b"bext"`).
        fn chunk(mut self, id: &[u8; 4], body: &[u8]) -> Self {
            self.trailing_chunks.extend_from_slice(id);
            self.trailing_chunks
                .extend_from_slice(&(body.len() as u32).to_le_bytes());
            self.trailing_chunks.extend_from_slice(body);
            if body.len() % 2 == 1 {
                self.trailing_chunks.push(0);
            }
            self
        }

        /// WAV requires an (extended) `fmt ` chunk with an explicit `cbSize` field for any
        /// non-PCM/non-float format tag (A-law, µ-law): a plain 16-byte `fmt ` is "malformed" for
        /// those (`symphonia-format-riff`'s `read_alaw_pcm_fmt`/`read_mulaw_pcm_fmt` require
        /// exactly an 18-byte chunk).
        fn cb_size_zero(mut self) -> Self {
            self.fmt_extra.extend_from_slice(&0u16.to_le_bytes());
            self
        }

        fn extensible(mut self, valid_bits: u16, channel_mask: u32, sub_format: [u8; 16]) -> Self {
            self.fmt_extra.extend_from_slice(&22u16.to_le_bytes()); // cbSize
            self.fmt_extra.extend_from_slice(&valid_bits.to_le_bytes());
            self.fmt_extra
                .extend_from_slice(&channel_mask.to_le_bytes());
            self.fmt_extra.extend_from_slice(&sub_format);
            self
        }

        fn write_bytes(mut self, bytes: &[u8]) -> Self {
            self.data.extend_from_slice(bytes);
            self
        }

        fn build(self) -> Vec<u8> {
            let block_align = self.channels * (self.bits_per_sample / 8);
            let byte_rate = self.sample_rate * u32::from(block_align);
            let fmt_body_len = 16 + self.fmt_extra.len();
            let mut fmt_chunk = Vec::new();
            fmt_chunk.extend_from_slice(&self.format_tag.to_le_bytes());
            fmt_chunk.extend_from_slice(&self.channels.to_le_bytes());
            fmt_chunk.extend_from_slice(&self.sample_rate.to_le_bytes());
            fmt_chunk.extend_from_slice(&byte_rate.to_le_bytes());
            fmt_chunk.extend_from_slice(&block_align.to_le_bytes());
            fmt_chunk.extend_from_slice(&self.bits_per_sample.to_le_bytes());
            fmt_chunk.extend_from_slice(&self.fmt_extra);
            assert_eq!(fmt_chunk.len(), fmt_body_len);

            let mut data_padded = self.data.clone();
            if data_padded.len() % 2 == 1 {
                data_padded.push(0);
            }
            let riff_size =
                4 + (8 + fmt_chunk.len()) + (8 + data_padded.len()) + self.trailing_chunks.len();
            let mut out = Vec::new();
            out.extend_from_slice(b"RIFF");
            out.extend_from_slice(&(riff_size as u32).to_le_bytes());
            out.extend_from_slice(b"WAVE");
            out.extend_from_slice(b"fmt ");
            out.extend_from_slice(&(fmt_chunk.len() as u32).to_le_bytes());
            out.extend_from_slice(&fmt_chunk);
            out.extend_from_slice(b"data");
            out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&data_padded);
            out.extend_from_slice(&self.trailing_chunks);
            out
        }
    }

    const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ];
    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: [u8; 16] = [
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ];

    fn write_file(bytes: &[u8], name: &str) -> std::path::PathBuf {
        let dir = tmp_dir("wav-variant");
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    fn decode_all(path: &Path) -> (ProbeInfo, Vec<f32>) {
        let (info, mut source) = DecodeSource::open(path).unwrap();
        let channels = info.track.channels.len().max(1);
        let mut out = Vec::new();
        let mut buf = vec![0f32; channels * 4096];
        loop {
            let n = source.read_frames(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n * channels]);
        }
        (info, out)
    }

    #[test]
    fn eight_bit_unsigned_pcm_matches_the_spec_conversion() {
        // SPEC-005 §2.3: (u - 128) / 128.
        let samples: Vec<u8> = vec![0, 64, 128, 192, 255];
        let bytes = RawWavBuilder::new(1, 1, 48_000, 8)
            .write_bytes(&samples)
            .build();
        let path = write_file(&bytes, "u8.wav");
        let (info, decoded) = decode_all(&path);
        assert_eq!(info.track.sample_rate_hz, 48_000);
        assert_eq!(decoded.len(), samples.len());
        for (u, got) in samples.iter().zip(decoded.iter()) {
            let expected = (f32::from(*u) - 128.0) / 128.0;
            assert!(
                (got - expected).abs() < 1e-6,
                "u8 {u} -> got {got}, expected {expected}"
            );
        }
    }

    #[test]
    fn thirty_two_bit_int_pcm_matches_the_spec_conversion() {
        // SPEC-005 §2.3: v / 2^31, rounded to nearest f32.
        let samples: Vec<i32> = vec![0, i32::MIN, i32::MAX, 1 << 20, -(1 << 20)];
        let mut bytes_data = Vec::new();
        for s in &samples {
            bytes_data.extend_from_slice(&s.to_le_bytes());
        }
        let bytes = RawWavBuilder::new(1, 1, 48_000, 32)
            .write_bytes(&bytes_data)
            .build();
        let path = write_file(&bytes, "s32.wav");
        let (_, decoded) = decode_all(&path);
        assert_eq!(decoded.len(), samples.len());
        for (s, got) in samples.iter().zip(decoded.iter()) {
            let expected = (f64::from(*s) / 2147483648.0) as f32;
            assert_eq!(*got, expected, "s32 {s} -> got {got}, expected {expected}");
        }
    }

    #[test]
    fn sixty_four_bit_float_pcm_matches_the_spec_conversion() {
        // SPEC-005 §2.3: rounded to nearest f32.
        let samples: Vec<f64> = vec![0.0, 0.5, -0.5, 1.5, -1.5, 0.1234567890123];
        let mut bytes_data = Vec::new();
        for s in &samples {
            bytes_data.extend_from_slice(&s.to_le_bytes());
        }
        let bytes = RawWavBuilder::new(3, 1, 48_000, 64)
            .write_bytes(&bytes_data)
            .build();
        let path = write_file(&bytes, "f64.wav");
        let (_, decoded) = decode_all(&path);
        assert_eq!(decoded.len(), samples.len());
        for (s, got) in samples.iter().zip(decoded.iter()) {
            assert_eq!(*got, *s as f32);
        }
    }

    // --- A-law / µ-law: exhaustive reference-table cross-check (ITU-T G.711) -------------------

    fn alaw_decode_reference(a: u8) -> i16 {
        let a = a ^ 0x55;
        let sign = a & 0x80;
        let exponent = (a & 0x70) >> 4;
        let mantissa = i16::from(a & 0x0F);
        let magnitude = if exponent == 0 {
            (mantissa << 4) + 8
        } else {
            ((mantissa << 4) + 0x108) << (exponent - 1)
        };
        if sign == 0 { -magnitude } else { magnitude }
    }

    fn ulaw_decode_reference(u: u8) -> i16 {
        let u = !u;
        let sign = u & 0x80;
        let exponent = (u >> 4) & 0x07;
        let mantissa = i16::from(u & 0x0F);
        let magnitude = (((mantissa << 3) + 0x84) << exponent) - 0x84;
        if sign != 0 { -magnitude } else { magnitude }
    }

    #[test]
    fn alaw_matches_g711_reference_expansion_for_every_byte_value() {
        let samples: Vec<u8> = (0u8..=255).collect();
        let bytes = RawWavBuilder::new(6, 1, 8_000, 8)
            .cb_size_zero()
            .write_bytes(&samples)
            .build();
        let path = write_file(&bytes, "alaw.wav");
        let (_, decoded) = decode_all(&path);
        assert_eq!(decoded.len(), 256);
        for (byte, got) in samples.iter().zip(decoded.iter()) {
            let expected = f32::from(alaw_decode_reference(*byte)) / 32768.0;
            assert!(
                (got - expected).abs() < 1e-6,
                "A-law byte {byte:#04x}: got {got}, expected {expected} (G.711 reference)"
            );
        }
    }

    #[test]
    fn mulaw_matches_g711_reference_expansion_for_every_byte_value() {
        let samples: Vec<u8> = (0u8..=255).collect();
        let bytes = RawWavBuilder::new(7, 1, 8_000, 8)
            .cb_size_zero()
            .write_bytes(&samples)
            .build();
        let path = write_file(&bytes, "mulaw.wav");
        let (_, decoded) = decode_all(&path);
        assert_eq!(decoded.len(), 256);
        for (byte, got) in samples.iter().zip(decoded.iter()) {
            let expected = f32::from(ulaw_decode_reference(*byte)) / 32768.0;
            assert!(
                (got - expected).abs() < 1e-6,
                "u-law byte {byte:#04x}: got {got}, expected {expected} (G.711 reference)"
            );
        }
    }

    #[test]
    fn five_point_one_extensible_reports_the_spec_channel_order_and_lfe() {
        // 5.1, mask 0x3F = FL, FR, FC, LFE, BL, BR (SPEC-005 AC-9e), 24-bit PCM.
        let frame_values: [i32; 6] = [100, 200, 300, 400, 500, 600];
        let mut data = Vec::new();
        for v in frame_values {
            let bytes = v.to_le_bytes(); // little-endian i32; take the low 3 bytes
            data.extend_from_slice(&bytes[0..3]);
        }
        let bytes = RawWavBuilder::new(0xFFFE, 6, 48_000, 24)
            .extensible(24, 0x3F, KSDATAFORMAT_SUBTYPE_PCM)
            .write_bytes(&data)
            .build();
        let path = write_file(&bytes, "surround51.wav");
        let (info, decoded) = decode_all(&path);
        assert_eq!(info.track.channels.len(), 6);
        let labels: Vec<&str> = info
            .track
            .channels
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "Left",
                "Right",
                "Center",
                "LFE",
                "Surround Left",
                "Surround Right"
            ]
        );
        assert!(info.track.channels[3].is_lfe);
        assert_eq!(decoded.len(), 6);
        for (v, got) in frame_values.iter().zip(decoded.iter()) {
            let expected = (f64::from(*v) / 8_388_608.0) as f32;
            assert!(
                (got - expected).abs() < 1e-5,
                "got {got}, expected {expected}"
            );
        }
    }

    #[test]
    fn extensible_float_is_read_as_float() {
        let samples: Vec<f32> = vec![0.25, -0.75, 1.5];
        let mut data = Vec::new();
        for s in &samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        let bytes = RawWavBuilder::new(0xFFFE, 1, 48_000, 32)
            .extensible(32, 0x1, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT)
            .write_bytes(&data)
            .build();
        let path = write_file(&bytes, "ext_float.wav");
        let (_, decoded) = decode_all(&path);
        assert_eq!(decoded, samples);
    }

    /// H-20 (SPEC-005 §2.6): `TrackFacts::bits_per_sample` reports the source's real bit depth for
    /// PCM WAV variants (`symphonia-format-riff`'s `append_format_params` calls
    /// `with_bits_per_sample` for `FormatData::Pcm`/`Extensible`, but *not* for plain
    /// `FormatData::IeeeFloat` — a symphonia quirk this test documents rather than works around,
    /// since `save_format_for_import`'s WAV branch keys on `codec` (`pcm_f32le`/`pcm_f64le`), not
    /// `bits_per_sample`, precisely because of this gap; only the FLAC branch relies on the field).
    #[test]
    fn bits_per_sample_reports_the_source_depth_for_pcm_wav_variants() {
        let cases: &[(u16, u16, u32)] = &[
            (1, 8, 8),   // PCM 8-bit unsigned
            (1, 16, 16), // PCM 16-bit
            (1, 24, 24), // PCM 24-bit
        ];
        for &(format_tag, bits, expected) in cases {
            let bytes = RawWavBuilder::new(format_tag, 1, 48_000, bits)
                .write_bytes(&vec![0u8; (bits / 8) as usize])
                .build();
            let path = write_file(&bytes, &format!("bits-{format_tag}-{bits}.wav"));
            let info = probe(&path).unwrap();
            assert_eq!(
                info.track.bits_per_sample,
                Some(expected),
                "format_tag {format_tag} bits {bits}"
            );
        }
    }

    /// H-20 (SPEC-005 §2.10): a plain WAV with no extra metadata reports `has_foreign_metadata:
    /// false`; one with a `bext` or `LIST INFO` chunk (chunk types symphonia's WAV reader either
    /// skips entirely or only partially surfaces) reports `true`.
    #[test]
    fn has_foreign_metadata_detects_bext_and_list_info_but_not_a_plain_wav() {
        let plain = RawWavBuilder::new(1, 1, 48_000, 16)
            .write_bytes(&[0, 0])
            .build();
        let plain_path = write_file(&plain, "plain.wav");
        assert!(!probe(&plain_path).unwrap().has_foreign_metadata);

        let with_bext = RawWavBuilder::new(1, 1, 48_000, 16)
            .write_bytes(&[0, 0])
            .chunk(b"bext", &[0u8; 8])
            .build();
        let bext_path = write_file(&with_bext, "bext.wav");
        assert!(probe(&bext_path).unwrap().has_foreign_metadata);

        let mut info_body = Vec::new();
        info_body.extend_from_slice(b"INFO");
        info_body.extend_from_slice(b"INAM");
        info_body.extend_from_slice(&4u32.to_le_bytes());
        info_body.extend_from_slice(b"Test");
        let with_info = RawWavBuilder::new(1, 1, 48_000, 16)
            .write_bytes(&[0, 0])
            .chunk(b"LIST", &info_body)
            .build();
        let info_path = write_file(&with_info, "list_info.wav");
        assert!(probe(&info_path).unwrap().has_foreign_metadata);
    }

    #[test]
    fn empty_wav_decodes_to_zero_samples() {
        let bytes = RawWavBuilder::new(1, 1, 48_000, 16).build();
        let path = write_file(&bytes, "empty.wav");
        let (info, decoded) = decode_all(&path);
        assert_eq!(info.track.sample_rate_hz, 48_000);
        assert_eq!(decoded.len(), 0);
    }

    #[test]
    fn zero_byte_file_is_unrecognized() {
        let path = write_file(&[], "zero.wav");
        let err = probe(&path).unwrap_err();
        assert!(matches!(
            err,
            IoError::UnrecognizedFormat(_) | IoError::Io(_)
        ));
    }

    #[test]
    fn random_bytes_are_unrecognized_not_a_panic() {
        // Seeded xorshift, not a WAV/FLAC/MP3/Ogg/M4A magic by construction.
        let mut state = 0x1234_5678_u32;
        let mut bytes = Vec::with_capacity(65_536);
        for _ in 0..65_536 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            bytes.push((state & 0xFF) as u8);
        }
        let path = write_file(&bytes, "random.bin");
        let err = probe(&path).unwrap_err();
        assert!(matches!(
            err,
            IoError::UnrecognizedFormat(_) | IoError::Io(_)
        ));
    }

    #[test]
    fn missing_path_is_an_io_error() {
        let err = probe(Path::new("/nonexistent/path/does-not-exist.wav")).unwrap_err();
        assert!(matches!(err, IoError::Io(_)));
    }

    /// A hand-rolled MS-ADPCM `fmt ` tag (0x0002): symphonia's `pcm` codec doesn't cover it and
    /// `adpcm` isn't an enabled feature (SPEC-005 §2.2: rejected).
    #[test]
    fn ms_adpcm_wav_is_rejected_as_an_unsupported_codec() {
        let bytes = RawWavBuilder::new(0x0002, 1, 8_000, 4)
            .write_bytes(&[0u8; 32])
            .build();
        let path = write_file(&bytes, "adpcm.wav");
        let err = probe(&path).unwrap_err();
        assert!(
            matches!(
                err,
                IoError::UnsupportedCodec(_) | IoError::UnrecognizedFormat(_)
            ),
            "got {err:?}"
        );
    }

    /// 10 000 header mutations never panic (SPEC-005 AC-14's fuzz-style unit test).
    #[test]
    fn header_mutation_fuzz_never_panics() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0) - 0.5).collect();
        let mut base_data = Vec::new();
        for s in &samples {
            base_data.extend_from_slice(&s.to_le_bytes());
        }
        let base = RawWavBuilder::new(3, 1, 48_000, 32)
            .write_bytes(&base_data)
            .build();

        let dir = tmp_dir("fuzz");
        let path = dir.join("mutant.wav");
        let mut state = 0x9E37_79B9_u32;
        for _ in 0..10_000 {
            let mut mutated = base.clone();
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let pos = (state as usize) % mutated.len();
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            mutated[pos] = (state & 0xFF) as u8;
            std::fs::write(&path, &mutated).unwrap();
            // Only requirement: never panics, regardless of the result.
            let _ = probe(&path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
