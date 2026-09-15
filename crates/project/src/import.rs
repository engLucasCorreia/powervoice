//! Import a file into a session as the undo floor (ADR-004 §8, MEMORY.md T-101: use
//! [`Session::set_floor`]).
//!
//! [`import_wav`] is the original S1-02 path: WAV only, through [`vox_io::WavSource`], average
//! downmix always, no progress/cancel. [`probe_for_import`]/[`import_file`] are T-202's general
//! path (SPEC-005 §2.2-2.4, §4.2): any format [`vox_io::decode`] supports, an explicit downmix
//! choice, streaming progress and cancellation. Both end at [`Session::set_floor`].

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vox_io::{ChannelPeak, DecodeSource, DownmixChoice};

use crate::session::Session;
use crate::snapshot::{DocSnapshot, Marker, MarkerId};
use crate::store::{CancelToken, ChunkWriter};
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// SPEC-005 §3 `doc_rate_range_hz`.
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
pub const MAX_SAMPLE_RATE_HZ: u32 = 384_000;
/// SPEC-005 §3 `max_channels`.
pub const MAX_CHANNELS: u16 = 32;
/// SPEC-005 §3 `probe_window_s`.
pub const CHANNEL_PROBE_WINDOW_S: f64 = 30.0;
/// How often [`import_file`] calls its progress callback (SPEC-005 §2.3: 4-10 Hz;
/// `progress_rate_hz`). Also an upper bound on cancellation latency together with the
/// per-`append` check ([`ChunkWriter`]/[`CancelToken`]).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
/// Frames decoded and downmixed per batch (SPEC-005 §2.3: "cancellation is checked at least every
/// 50 ms" — small enough that even a slow decoder keeps this well under that bound).
const BATCH_FRAMES: usize = 8192;
/// T-704: decoded batches in flight between the import's decoder thread and the chunk writer.
const PIPELINE_BATCHES: usize = 8;

fn validate_rate_and_channels(rate: u32, channels: u16) -> Result<()> {
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&rate) {
        return Err(vox_io::IoError::RateOutOfRange(rate).into());
    }
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(vox_io::IoError::TooManyChannels(channels).into());
    }
    Ok(())
}

/// `> max(10, 1% of packets)` (SPEC-005 §2.5 `damaged_packet_limit`).
fn too_damaged(damaged: usize, total_packets: usize) -> bool {
    damaged > 10.max(total_packets / 100)
}

/// One source channel's role (SPEC-005 §2.4), for `document_probe`'s dialog data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportChannel {
    pub label: String,
    pub is_lfe: bool,
}

/// What probing `path` found (SPEC-005 §2.3 step 1, §2.4; `document_probe`'s IPC payload). The
/// channel probe (peaks, identical-channel check) only runs when there's more than one channel —
/// `channel_peaks_dbfs` is empty and `identical_channels` is `false` for mono.
#[derive(Debug, Clone)]
pub struct ImportProbe {
    pub container: String,
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channels: Vec<ImportChannel>,
    /// `None` when the container doesn't state a sample count (SPEC-005 §2.3).
    pub len_samples: Option<u64>,
    pub channel_peaks_dbfs: Vec<f32>,
    pub identical_channels: bool,
    /// The silent-channel hint's preselected channel index (SPEC-005 §2.4), if any.
    pub suggested_channel: Option<usize>,
    /// H-20 (SPEC-005 §2.6): the codec's reported bit depth, when it has one (WAV PCM variants,
    /// FLAC) — the save-format promotion table's input alongside `codec`/`container`.
    pub bits_per_sample: Option<u32>,
    /// H-20 (SPEC-005 §2.10): the source carries metadata PowerVoice doesn't preserve.
    pub has_foreign_metadata: bool,
}

/// Probes `path` (container/codec/rate/channels, SPEC-005 §2.3 step 1) and, for multichannel
/// input, the first `min(`[`CHANNEL_PROBE_WINDOW_S`]`, length)` for the open dialog's data
/// (SPEC-005 §2.4). Validates the rate/channel-count range (§2.5) — this is the same check
/// [`import_file`] repeats defensively, so a caller that only wants to know whether a file *can*
/// open doesn't need to call both.
pub fn probe_for_import(path: &Path) -> Result<ImportProbe> {
    let info = vox_io::probe(path)?;
    let channel_count = u16::try_from(info.track.channels.len()).unwrap_or(u16::MAX);
    validate_rate_and_channels(info.track.sample_rate_hz, channel_count)?;

    let (channel_peaks_dbfs, identical_channels, suggested_channel) = if channel_count > 1 {
        let probed = vox_io::probe_channels(path, CHANNEL_PROBE_WINDOW_S)?;
        let peaks: Vec<ChannelPeak> = probed
            .peaks_dbfs
            .iter()
            .map(|&peak_dbfs| ChannelPeak { peak_dbfs })
            .collect();
        let suggested = vox_io::suggest_silent_channel(&peaks);
        (probed.peaks_dbfs, probed.identical, suggested)
    } else {
        (Vec::new(), false, None)
    };

    Ok(ImportProbe {
        container: info.container.to_string(),
        codec: info.codec.to_string(),
        sample_rate_hz: info.track.sample_rate_hz,
        channels: info
            .track
            .channels
            .into_iter()
            .map(|c| ImportChannel {
                label: c.label,
                is_lfe: c.is_lfe,
            })
            .collect(),
        len_samples: info.track.len_samples,
        channel_peaks_dbfs,
        identical_channels,
        suggested_channel,
        bits_per_sample: info.track.bits_per_sample,
        has_foreign_metadata: info.has_foreign_metadata,
    })
}

/// What [`import_file`] did, for the journal `open` record (SPEC-005 §4.12) and notices.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub snapshot: Arc<DocSnapshot>,
    pub container: String,
    pub codec: String,
    pub source_channels: u16,
    pub damaged_packets: usize,
    pub non_finite_replaced: usize,
    /// H-20 (SPEC-005 §2.10): the source carries metadata PowerVoice doesn't preserve on save
    /// (`LIST INFO`, `bext`, `iXML`, `smpl`, ID3/Vorbis comments) — drives
    /// `notice.save.metadata_dropped` at the document's first Save.
    pub has_foreign_metadata: bool,
}

/// Streams `path` through [`vox_io::decode`], downmixing by `downmix` (SPEC-005 §2.4, §4.3), into
/// `session`'s store, then makes the result the undo floor — the general-format counterpart of
/// [`import_wav`] (SPEC-005 §4.2's decode loop). WAV `cue `/`LIST adtl` markers are read the same
/// way `import_wav`'s callers already do ([`vox_io::read_wav_markers`], harmless/empty for a
/// non-WAV `path`).
///
/// `progress(frames_done, len_samples_hint)` is called at ~[`PROGRESS_INTERVAL`] and once more
/// after the loop ends; `len_samples_hint` mirrors [`ImportProbe::len_samples`] (`None` when the
/// container doesn't state a count). Cancelling `cancel` stops the import within one batch of
/// [`BATCH_FRAMES`] frames and leaves the session's undo floor unset — the caller discards the
/// session directory, as with any other import error (SPEC-005 §2.3 "Cancel").
pub fn import_file(
    session: &mut Session,
    path: &Path,
    downmix: DownmixChoice,
    cancel: &CancelToken,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<ImportResult> {
    let (info, mut source) = DecodeSource::open(path)?;
    let channel_count = u16::try_from(info.track.channels.len()).unwrap_or(u16::MAX);
    validate_rate_and_channels(info.track.sample_rate_hz, channel_count)?;
    if info.track.sample_rate_hz != session.sample_rate_hz() {
        return Err(ProjectError::InvalidArgument(
            "import_file: the source's sample rate must match the session's",
        ));
    }
    let lfe: Vec<bool> = info.track.channels.iter().map(|c| c.is_lfe).collect();
    let channels = source.channels().max(1);
    let len_hint = info.track.len_samples;

    let mut writer = ChunkWriter::with_cancel(Arc::clone(session.store()), cancel.clone());
    let mut frames_done: u64 = 0;
    let mut last_progress = Instant::now();
    // T-704: decoding + downmixing (a decoder thread) overlaps committing chunks (this thread:
    // peaks, CRC, copy into the store) — they were ~0.45 s and ~0.5 s of a 60-min import, run one
    // after the other per batch. Batches flow through a bounded channel and their buffers are
    // recycled, so memory stays at `PIPELINE_BATCHES` batches. The decoder stops at the first
    // error (sent through the channel), on cancel, or when this side hangs up (a write error).
    let (source, pipeline) = std::thread::scope(|scope| {
        let (batches_tx, batches_rx) =
            std::sync::mpsc::sync_channel::<vox_io::Result<Vec<f32>>>(PIPELINE_BATCHES);
        let (spare_tx, spare_rx) = std::sync::mpsc::channel::<Vec<f32>>();
        let decoder_cancel = cancel.clone();
        let lfe = &lfe;
        let decoder = scope.spawn(move || {
            let mut raw = vec![0f32; channels * BATCH_FRAMES];
            while !decoder_cancel.is_cancelled() {
                let n = match source.read_frames(&mut raw) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        let _ = batches_tx.send(Err(e));
                        break;
                    }
                };
                let mut mono = spare_rx
                    .try_recv()
                    .unwrap_or_else(|_| Vec::with_capacity(BATCH_FRAMES));
                mono.clear();
                mono.extend(
                    raw[..n * channels]
                        .chunks_exact(channels)
                        .map(|frame| vox_io::downmix_frame(frame, lfe, downmix)),
                );
                if batches_tx.send(Ok(mono)).is_err() {
                    break;
                }
            }
            source
        });
        let written = (|| -> Result<()> {
            for batch in batches_rx {
                if cancel.is_cancelled() {
                    return Err(ProjectError::Cancelled);
                }
                let mono = batch?;
                writer.append(&mono)?;
                frames_done += mono.len() as u64;
                if last_progress.elapsed() >= PROGRESS_INTERVAL {
                    progress(frames_done, len_hint);
                    last_progress = Instant::now();
                }
                let _ = spare_tx.send(mono);
            }
            Ok(())
        })();
        // `batches_rx` is gone (consumed or dropped), so a decoder blocked on a full channel wakes
        // with a send error and returns.
        (decoder.join(), written)
    });
    let source = source.map_err(|_| ProjectError::InvalidArgument("import decoder panicked"))?;
    pipeline?;
    if cancel.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    progress(frames_done, len_hint);

    let damaged_packets = source.damaged_packets();
    let total_packets = source.total_packets();
    if too_damaged(damaged_packets, total_packets) {
        writer.cancel();
        return Err(vox_io::IoError::TooDamaged(damaged_packets, total_packets).into());
    }

    let audio = writer.finish()?;
    let len = audio.len_samples;
    let wav_markers = vox_io::read_wav_markers(path).unwrap_or_default();
    let markers: Vec<Marker> = wav_markers
        .into_iter()
        .filter(|m| {
            m.pos_samples
                .checked_add(m.len_samples)
                .is_some_and(|end| end <= len)
        })
        .enumerate()
        .map(|(i, m)| Marker::new(MarkerId(i as u64 + 1), m.pos_samples, m.len_samples, m.name))
        .collect();
    let snapshot = session.set_floor(&audio, markers)?;

    Ok(ImportResult {
        snapshot,
        container: info.container.to_string(),
        codec: info.codec.to_string(),
        source_channels: channel_count,
        damaged_packets,
        non_finite_replaced: source.non_finite_replaced(),
        has_foreign_metadata: info.has_foreign_metadata,
    })
}

/// Streams `source` through a [`crate::store::ChunkWriter`] and makes the result the session's
/// undo floor (SPEC-004 §2.1, ADR-004 §8): the document opens clean, at the imported audio, with
/// `wav_markers` (SPEC-005 §2.9's `cue `/`LIST adtl` reader, [`vox_io::read_wav_markers`]) as its
/// markers. Markers get ids `1..=n` in canonical (position) order (SPEC-009 §2.13 "ids after
/// open", case 5 — no sidecar); a marker whose range would fall outside the imported audio is
/// dropped rather than failing the whole import (defensive: `set_floor` would otherwise refuse the
/// floor over one bad cue point from a hand-edited or malformed file).
///
/// `source`'s sample rate must match `session`'s (the document rate is the source rate — the
/// caller creates the session with [`crate::SessionConfig::new`]`(rate)` from the same file,
/// ADR-004 §8). `source` already downmixes multichannel input to mono by averaging
/// ([`vox_io::WavSource::read_mono`], SPEC-005 §2.4 default); picking a channel instead and
/// progress events are deferred (S1-02 ticket scope).
pub fn import_wav(
    session: &mut Session,
    source: &mut vox_io::WavSource,
    wav_markers: Vec<vox_io::WavMarker>,
) -> Result<Arc<DocSnapshot>> {
    if source.sample_rate_hz() != session.sample_rate_hz() {
        return Err(ProjectError::InvalidArgument(
            "import_wav: the source's sample rate must match the session's",
        ));
    }
    let mut writer = session.chunk_writer();
    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    loop {
        let n = source.read_mono(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.append(&buf[..n])?;
    }
    let audio = writer.finish()?;
    let len = audio.len_samples;
    let markers: Vec<Marker> = wav_markers
        .into_iter()
        .filter(|m| {
            m.pos_samples
                .checked_add(m.len_samples)
                .is_some_and(|end| end <= len)
        })
        .enumerate()
        .map(|(i, m)| Marker::new(MarkerId(i as u64 + 1), m.pos_samples, m.len_samples, m.name))
        .collect();
    session.set_floor(&audio, markers)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // deliberate bit-exact downmix/copy checks (SPEC-005 AC-9)
mod tests {
    use super::*;
    use crate::{SessionConfig, StoreOptions};

    fn write_ref_wav(path: &std::path::Path, samples: &[f32], channels: u16, rate: u32) {
        vox_testkit::wav::write_wav_file(
            path,
            samples,
            channels,
            rate,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
    }

    #[test]
    fn import_mono_wav_becomes_the_undo_floor() {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-import-{}-{}",
            std::process::id(),
            "mono"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        write_ref_wav(&wav_path, &samples, 1, 48_000);

        let (rate, channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        assert_eq!(channels, 1);

        let mut config = SessionConfig::new(rate);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();

        let snapshot = import_wav(&mut session, &mut source, Vec::new()).unwrap();
        assert_eq!(snapshot.len_samples, samples.len() as u64);
        assert!(!session.is_dirty(), "import is not an undoable edit");
        assert_eq!(
            session.history().undo_depth(),
            0,
            "no undo entry for import"
        );

        let mut out = vec![0.0f32; samples.len()];
        session.store().read(&snapshot, 0, &mut out).unwrap();
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_a_sample_rate_mismatch() {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-import-{}-{}",
            std::process::id(),
            "ratemismatch"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::silence(0.05, 44_100).unwrap();
        write_ref_wav(&wav_path, &samples, 1, 44_100);
        let (_, _, mut source) = vox_io::read_wav(&wav_path).unwrap();

        let mut config = SessionConfig::new(48_000);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        assert!(matches!(
            import_wav(&mut session, &mut source, Vec::new()),
            Err(ProjectError::InvalidArgument(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S2-03: WAV cue markers become the floor's markers, with fresh sequential ids in position
    /// order (SPEC-009 §2.13 case 5); an out-of-range one is dropped rather than failing the
    /// import.
    #[test]
    fn import_wav_carries_cue_markers_as_the_floor_markers() {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-import-{}-{}",
            std::process::id(),
            "markers"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        write_ref_wav(&wav_path, &samples, 1, 48_000);

        let (rate, _channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        let wav_markers = vec![
            vox_io::WavMarker {
                pos_samples: 1_000,
                len_samples: 0,
                name: "Intro".into(),
            },
            vox_io::WavMarker {
                pos_samples: 2_000,
                len_samples: 500,
                name: "Region".into(),
            },
            vox_io::WavMarker {
                pos_samples: samples.len() as u64 + 10, // out of range: dropped
                len_samples: 0,
                name: "Bad".into(),
            },
        ];

        let mut config = SessionConfig::new(rate);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        let snapshot = import_wav(&mut session, &mut source, wav_markers).unwrap();

        assert_eq!(
            snapshot.markers.len(),
            2,
            "the out-of-range marker is dropped"
        );
        assert_eq!(snapshot.markers[0].id.0, 1);
        assert_eq!(snapshot.markers[0].pos_samples, 1_000);
        assert_eq!(&*snapshot.markers[0].name, "Intro");
        assert_eq!(snapshot.markers[1].id.0, 2);
        assert_eq!(snapshot.markers[1].pos_samples, 2_000);
        assert_eq!(snapshot.markers[1].len_samples, 500);
        assert!(!session.is_dirty(), "import is not an undoable edit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- T-202: import_file / probe_for_import ------------------------------------------------

    fn test_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-importfile-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new_session(dir: &std::path::Path, rate: u32) -> Session {
        let mut config = SessionConfig::new(rate);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        Session::create(dir, config).unwrap()
    }

    fn write_interleaved_wav(
        path: &std::path::Path,
        interleaved: &[f32],
        channels: u16,
        rate: u32,
    ) {
        vox_testkit::wav::write_wav_file(
            path,
            interleaved,
            channels,
            rate,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
    }

    #[test]
    fn import_file_of_a_mono_wav_matches_import_wav() {
        let dir = test_dir("mono");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        let path = dir.join("in.wav");
        write_interleaved_wav(&path, &samples, 1, 48_000);

        let mut session = new_session(&dir, 48_000);
        let cancel = CancelToken::new();
        let mut ticks = 0;
        let result = import_file(
            &mut session,
            &path,
            DownmixChoice::Average,
            &cancel,
            |_, _| {
                ticks += 1;
            },
        )
        .unwrap();
        assert_eq!(result.snapshot.len_samples, samples.len() as u64);
        assert_eq!(result.source_channels, 1);
        assert_eq!(result.damaged_packets, 0);
        assert!(!session.is_dirty());

        let mut out = vec![0.0f32; samples.len()];
        session.store().read(&result.snapshot, 0, &mut out).unwrap();
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-005 AC-9a: `L = sine, R = -L`, average -> digital silence.
    #[test]
    fn import_file_average_downmix_of_opposite_channels_is_silent() {
        let dir = test_dir("silent");
        let mono = vox_testkit::signal::sine(997.0, -6.0, 0.05, 48_000).unwrap();
        let mut stereo = Vec::with_capacity(mono.len() * 2);
        for &s in &mono {
            stereo.push(s);
            stereo.push(-s);
        }
        let path = dir.join("stereo.wav");
        write_interleaved_wav(&path, &stereo, 2, 48_000);

        let mut session = new_session(&dir, 48_000);
        let cancel = CancelToken::new();
        let result = import_file(
            &mut session,
            &path,
            DownmixChoice::Average,
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result.source_channels, 2);

        let mut out = vec![1.0f32; mono.len()];
        session.store().read(&result.snapshot, 0, &mut out).unwrap();
        assert!(
            out.iter().all(|&s| s == 0.0),
            "L = -R must average to exact digital silence"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-005 AC-9d: picking a channel copies it verbatim.
    #[test]
    fn import_file_pick_channel_copies_the_chosen_channel_verbatim() {
        let dir = test_dir("pick");
        let left = vox_testkit::signal::sine(300.0, -10.0, 0.05, 48_000).unwrap();
        let right = vox_testkit::signal::white_noise(1, -20.0, 0.05, 48_000).unwrap();
        let mut stereo = Vec::with_capacity(left.len() * 2);
        for i in 0..left.len() {
            stereo.push(left[i]);
            stereo.push(right[i]);
        }
        let path = dir.join("stereo.wav");
        write_interleaved_wav(&path, &stereo, 2, 48_000);

        let mut session = new_session(&dir, 48_000);
        let cancel = CancelToken::new();
        let result = import_file(
            &mut session,
            &path,
            DownmixChoice::Channel(1),
            &cancel,
            |_, _| {},
        )
        .unwrap();

        let mut out = vec![0.0f32; right.len()];
        session.store().read(&result.snapshot, 0, &mut out).unwrap();
        for (a, b) in right.iter().zip(out.iter()) {
            assert_eq!(a, b, "picking channel 1 must copy R verbatim");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_for_import_flags_identical_channels_without_a_choice_needed() {
        let dir = test_dir("identical");
        let mono = vox_testkit::signal::sine(500.0, -10.0, 0.1, 48_000).unwrap();
        let mut stereo = Vec::with_capacity(mono.len() * 2);
        for &s in &mono {
            stereo.push(s);
            stereo.push(s);
        }
        let path = dir.join("dualmono.wav");
        write_interleaved_wav(&path, &stereo, 2, 48_000);

        let probe = probe_for_import(&path).unwrap();
        assert!(probe.identical_channels);
        assert_eq!(probe.channels.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_for_import_suggests_the_active_channel_when_one_side_is_silent() {
        let dir = test_dir("suggest");
        let active = vox_testkit::signal::sine(500.0, -20.0, 1.0, 48_000).unwrap();
        let mut stereo = Vec::with_capacity(active.len() * 2);
        for &s in &active {
            stereo.push(s); // Left active
            stereo.push(0.0); // Right silent
        }
        let path = dir.join("onesilent.wav");
        write_interleaved_wav(&path, &stereo, 2, 48_000);

        let probe = probe_for_import(&path).unwrap();
        assert!(!probe.identical_channels);
        assert_eq!(probe.suggested_channel, Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_file_cancel_mid_stream_leaves_no_undo_floor() {
        let dir = test_dir("cancel");
        // Long enough to guarantee more than one progress-check window at BATCH_FRAMES chunks.
        let samples = vox_testkit::signal::sine(220.0, -6.0, 2.0, 48_000).unwrap();
        let path = dir.join("long.wav");
        write_interleaved_wav(&path, &samples, 1, 48_000);

        let mut session = new_session(&dir, 48_000);
        let cancel = CancelToken::new();
        let cancel_after_first_tick = cancel.clone();
        let mut ticks = 0u32;
        let result = import_file(
            &mut session,
            &path,
            DownmixChoice::Average,
            &cancel,
            |_frames, _total| {
                ticks += 1;
                cancel_after_first_tick.cancel();
            },
        );
        assert!(matches!(result, Err(ProjectError::Cancelled)));
        assert_eq!(session.current().len_samples, 0, "no partial document");
        assert!(!session.is_dirty());
        assert_eq!(session.history().undo_depth(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_file_rejects_a_rate_outside_the_accepted_range() {
        let dir = test_dir("badrate");
        let samples = vox_testkit::signal::silence(0.01, 4_000).unwrap();
        let path = dir.join("lowrate.wav");
        write_interleaved_wav(&path, &samples, 1, 4_000);

        let probe_err = probe_for_import(&path).unwrap_err();
        assert!(matches!(
            probe_err,
            ProjectError::Wav(vox_io::IoError::RateOutOfRange(4_000))
        ));

        let mut session = new_session(&dir, 4_000);
        let cancel = CancelToken::new();
        let err = import_file(
            &mut session,
            &path,
            DownmixChoice::Average,
            &cancel,
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ProjectError::Wav(vox_io::IoError::RateOutOfRange(4_000))
        ));
        assert_eq!(err.i18n_key(), "error.open.rate_out_of_range");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_file_of_a_flac_file_carries_no_markers_but_imports_the_audio() {
        // Uses the reference `flac` CLI, like `crates/io/tests/codec_decode.rs` (see its
        // module docs for why `vox_io::write_flac`'s own output isn't used here).
        if std::process::Command::new("flac")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping: flac CLI not installed");
            return;
        }
        let dir = test_dir("flac");
        let samples = vox_testkit::signal::sine(997.0, -20.0, 0.2, 48_000).unwrap();
        let wav_path = dir.join("in.wav");
        // `flac`'s encoder doesn't accept a 32-bit float WAV; use Int24, like the analogous
        // `crates/io/tests/codec_decode.rs` test.
        vox_testkit::wav::write_wav_file(
            &wav_path,
            &samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Int24,
        )
        .unwrap();
        let flac_path = dir.join("in.flac");
        let status = std::process::Command::new("flac")
            .args(["-f", "--totally-silent", "-o"])
            .arg(&flac_path)
            .arg(&wav_path)
            .status()
            .unwrap();
        assert!(status.success());

        let mut session = new_session(&dir, 48_000);
        let cancel = CancelToken::new();
        let result = import_file(
            &mut session,
            &flac_path,
            DownmixChoice::Average,
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result.container, "flac");
        assert_eq!(result.snapshot.markers.len(), 0);
        assert_eq!(result.snapshot.len_samples, samples.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
