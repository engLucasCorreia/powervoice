//! T-401 rack latency compensation end-to-end through the fake backend (SPEC-012 §2.5, AC-8;
//! SPEC-003 §2.2; ADR-002 §8): the heard position (telemetry anchor, waveform cursor, markers)
//! names the document sample actually leaving the device whatever the rack latency; it follows a
//! mid-play latency change (a module restart) within 100 ms without a silent gap or an
//! allocation in the callback; and the transport reports the document end only once the last
//! sample has been heard (the rack drained its latency). Every output callback runs under
//! `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::StreamId;
use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection, RecordedOutput};
use vox_engine::{
    Direction, EngineConfig, EngineEvent, HostId, ManualEngine, PlaybackDoc, RackCommand,
    TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{TestRestart, alloc_checks_active, no_alloc};
use vox_module_api::{
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, Version,
};
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, RackNotice, Registry, SlotModel};

const MS: u64 = 1_000_000;
const RATE: u64 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-lat-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((x >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
        })
        .collect()
}

/// `TestRestart`: an exact delay whose latency is its `latency_samples` parameter; a change
/// requests a restart, and the replacement runs with the new latency (ADR-005 §8).
struct RestartFactory(ModuleDescriptor);

impl ModuleFactory for RestartFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestRestart::new(0)))
    }
}

/// [TestRestart(`latency`)].
fn delay_rack(latency: u32) -> RackModel {
    RackModel {
        slots: vec![SlotModel::new(
            &ModuleRef {
                id: TestRestart::ID.into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState {
                format_version: 1,
                params: BTreeMap::from([("latency_samples".to_owned(), f64::from(latency))]),
                blob: None,
            },
        )],
    }
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    events: Arc<Mutex<Vec<EngineEvent>>>,
    frames: Arc<Mutex<Vec<TelemetryFrame>>>,
    _dir: TempDir,
}

/// A 48 kHz document `samples` through `rack` on a stereo DAC with 256-frame callbacks.
fn rig(samples: &[f32], rack: RackModel) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(42);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(
            FakeDirection::new(2, &[48_000], 48_000)
                .default_buffer(256)
                .record_output(),
        ),
    );
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let dir = TempDir::new("doc");
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(256 << 20)).unwrap();
    let mut w = ChunkWriter::new(store.clone());
    w.append(samples).unwrap();
    let audio = w.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));

    let restart: Arc<dyn ModuleFactory> =
        Arc::new(RestartFactory(TestRestart::new(0).descriptor().clone()));
    let registry = Arc::new(
        Registry::with_factories(
            vox_modules::builtin_factories()
                .into_iter()
                .chain([restart]),
        )
        .unwrap(),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack;
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    let fr = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f| fr.lock().unwrap().push(*f))));
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    Rig {
        fake,
        eng,
        events,
        frames,
        _dir: dir,
    }
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.fake.advance_by(MS);
            self.eng.tick();
        }
    }

    fn stream(&self) -> StreamId {
        self.fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == Direction::Output)
            .map(|s| s.info.id)
            .expect("an output stream")
    }

    fn output(&self) -> RecordedOutput {
        self.fake.recorded_output(self.stream()).unwrap()
    }

    fn frames(&self) -> Vec<TelemetryFrame> {
        self.frames.lock().unwrap().clone()
    }

    fn latency_notices(&self) -> Vec<u32> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Rack(RackNotice::LatencyChanged { total_samples }) => {
                    Some(*total_samples)
                }
                _ => None,
            })
            .collect()
    }
}

/// The output frame index played at app time `t_ns` (a telemetry anchor's time is the app-clock
/// playback time of a callback's first frame).
fn frame_at(r: &Rig, out: &RecordedOutput, t_ns: u64) -> Option<usize> {
    let id = r.stream();
    let (first, t0) = out
        .blocks
        .iter()
        .map(|b| {
            (
                b.first_frame,
                r.fake.frame_time_ns(id, b.first_frame).unwrap(),
            )
        })
        .rev()
        .find(|&(_, t)| t <= t_ns)?;
    let into = ((t_ns - t0) * RATE + 500_000_000) / 1_000_000_000;
    Some((first + into) as usize)
}

/// The document sample heard at output frame `k`: the unique `q` near `hint` with
/// `rec[k..k + 32] == src[q..q + 32]` (noise makes it unambiguous). `None` inside a crossfade.
fn heard_at(rec: &[f32], src: &[f32], k: usize, hint: usize) -> Option<usize> {
    if k + 32 > rec.len() {
        return None;
    }
    let lo = hint.saturating_sub(12_000);
    let hi = (hint + 12_000).min(src.len() - 32);
    (lo..=hi).find(|&q| (0..32).all(|j| (rec[k + j] - src[q + j]).abs() <= 1e-6))
}

/// Telemetry anchors (p, t) while playing, from `after_ns`, as (reported, heard) pairs.
fn anchors(r: &Rig, src: &[f32], after_ns: u64) -> Vec<(u64, Option<usize>)> {
    let out = r.output();
    r.frames()
        .iter()
        .filter(|f| f.rate > 0.0 && f.playhead_time_ns >= after_ns && f.playhead_sample > 2_000)
        .filter_map(|f| {
            let k = frame_at(r, &out, f.playhead_time_ns)?;
            let p = f.playhead_sample;
            Some((p, heard_at(&out.samples, src, k, p as usize)))
        })
        .collect()
}

/// SPEC-003 §2.2 / ADR-002 §8: the telemetry anchor names the document sample leaving the device
/// at the anchor's time — with no rack latency, 10 ms and 100 ms of it.
#[test]
fn the_heard_position_names_the_sample_leaving_the_device() {
    let src = noise(1, 3 * 48_000);
    for latency in [0u32, 480, 4800] {
        let mut r = rig(&src, delay_rack(latency));
        r.run_ms(20);
        assert!(r.eng.transport(TransportCommand::Play).playing);
        r.run_ms(1_200);
        let a = anchors(&r, &src, 0);
        assert!(a.len() > 20, "latency {latency}: {} anchors", a.len());
        for (p, heard) in a {
            let q = heard.unwrap_or_else(|| panic!("latency {latency}: nothing heard at {p}"));
            assert!(
                (p as i64 - q as i64).abs() <= 1,
                "latency {latency}: the playhead reads {p} while {q} is heard"
            );
        }
        assert_eq!(r.fake.rt_violations(), 0);
    }
}

/// SPEC-012 AC-8: a module restart that changes the rack latency mid-play (480 → 960) moves the
/// heard-position offset with it within 100 ms of the command, with no silent gap and no
/// allocation in the callback; the readout notice follows.
#[test]
fn a_latency_change_mid_play_moves_the_heard_position_within_100_ms() {
    let src = noise(2, 4 * 48_000);
    let mut r = rig(&src, delay_rack(480));
    r.run_ms(20);
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(800);
    let t_cmd = r.fake.now_ns();
    let at_cmd = r.output().samples.len();
    r.eng
        .rack_command(RackCommand::SetParamPlain {
            index: 0,
            id: TestRestart::LATENCY,
            value: 960.0,
        })
        .unwrap();
    let mut notice_at = None;
    for _ in 0..600 {
        r.run_ms(1);
        if notice_at.is_none() && r.latency_notices().contains(&960) {
            notice_at = Some(r.fake.now_ns() - t_cmd);
        }
    }
    let notice_at = notice_at.expect("a LatencyChanged(960) notice");
    assert!(notice_at <= 100 * MS, "readout after {} ms", notice_at / MS);
    assert_eq!(r.eng.rack_snapshot().latency_samples, 960);

    // Before the command and from 100 ms after it, the playhead names what is heard.
    let checked = anchors(&r, &src, t_cmd + 100 * MS);
    assert!(
        checked.len() > 20,
        "{} anchors after the change",
        checked.len()
    );
    for (p, heard) in checked {
        let q = heard.unwrap_or_else(|| panic!("nothing heard at {p}"));
        assert!(
            (p as i64 - q as i64).abs() <= 1,
            "after the change the playhead reads {p} while {q} is heard"
        );
    }
    let rec = r.output().samples;
    let silent = rec[at_cmd..]
        .windows(3)
        .any(|w| w.iter().all(|v| v.abs() < 1e-7));
    assert!(!silent, "silent gap after the latency change");
    assert!(r.eng.transport_state().playing);
    assert_eq!(r.fake.rt_violations(), 0, "the output callback allocated");
}

/// SPEC-003 §2.2 / SPEC-012 §2.5: the document end is reported (transport stops, playhead at the
/// end) only once its last sample has left the rack, 100 ms after it entered — the playhead
/// never stops ahead of the audio.
#[test]
fn the_document_end_is_reported_once_the_last_sample_is_heard() {
    let src = noise(3, 24_000);
    for latency in [0u32, 4800] {
        let mut r = rig(&src, delay_rack(latency));
        r.run_ms(20);
        assert!(r.eng.transport(TransportCommand::Play).playing);
        let mut stopped_len = None;
        for _ in 0..2_000 {
            r.run_ms(1);
            if !r.eng.transport_state().playing {
                stopped_len = Some(r.output().samples.len());
                break;
            }
        }
        let stopped_len = stopped_len.expect("the transport ends");
        r.run_ms(300);
        let rec = r.output().samples;
        let last = rec.iter().rposition(|&x| x != 0.0).expect("audio");
        assert!(
            stopped_len > last,
            "latency {latency}: stopped at output frame {stopped_len}, audio until {last}"
        );
        assert!(
            stopped_len <= last + 2 * 256 + 96,
            "latency {latency}: stopped at {stopped_len}, long after the audio ({last})"
        );
        // The whole document was heard, in order.
        let k = heard_at(&rec, &src, last + 1 - 32, src.len() - 32).expect("the last samples");
        assert_eq!(k, src.len() - 32, "latency {latency}");
        let st = r.eng.transport_state();
        assert_eq!(
            st.playhead_samples,
            src.len() as u64,
            "end = Pause at the end"
        );
        assert_eq!(r.fake.rt_violations(), 0);
        // Play again from the start: the rack starts clean and plays the document.
        assert!(r.eng.transport(TransportCommand::Play).playing);
        r.run_ms(700);
        assert!(!r.eng.transport_state().playing, "second pass ended");
        assert_eq!(r.fake.rt_violations(), 0);
    }
}
