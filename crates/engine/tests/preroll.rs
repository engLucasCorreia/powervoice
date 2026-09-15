//! H-46: rack pre-roll on Play and seek (SPEC-003 Amendment 2, SPEC-012 §2.5.2, ADR-002
//! Amendment 4). The rack's latency is no longer added to the playback start: the first sample
//! heard is the play position, as soon as an empty rack would play it, whatever the rack latency
//! (0, 480, ≥ 2048 samples); the rack enters the play position warmed up with the audio before
//! it; the playhead mapping stays exact; a seek while playing jumps without waiting for the rack;
//! loops and the document end keep working. Every output callback runs under
//! `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::StreamId;
use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection, RecordedOutput};
use vox_engine::{
    Direction, EngineConfig, HostId, ManualEngine, PlaybackDoc, TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{
    TestDelay, ZIPPER_FREQ_HZ, ZIPPER_LEVEL_DBFS, ZipperWindow, alloc_checks_active,
    analyze_zipper, no_alloc,
};
use vox_module_api::{
    ActivateConfig, LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError,
    ModuleFactory, ModuleRef, ModuleState, ParamId, ParamInfo, ProcessContext, ProcessStatus,
    StateError, Tail, Version,
};
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};

const MS: u64 = 1_000_000;
/// Simulated-clock step while timing a start (the `playback_start` bench's resolution).
const STEP_NS: u64 = 100_000;
/// The 5 ms transport fade at 48 kHz.
const FADE: usize = 240;
/// How much later than with an empty rack a start may be heard, whatever the rack latency: one
/// extra 256-frame callback (5.3 ms) for a long pre-roll, plus a clock step.
const START_SLACK_NS: u64 = 6_500_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-preroll-{tag}-{}-{}",
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

/// The SPEC-012 §4.3 tone (997 Hz, −20 dBFS peak, 48 kHz), `n` samples long.
fn tone(n: usize) -> Vec<f32> {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let w = std::f64::consts::TAU * ZIPPER_FREQ_HZ / 48_000.0;
    (0..n)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect()
}

fn descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        id: id.into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain(id),
        vendor: "PowerVoice".into(),
        description: LocalizedText::plain("H-46 test module"),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    }
}

/// `TestDelay(latency)` under its own id.
struct DelayFactory(ModuleDescriptor, u32);

impl ModuleFactory for DelayFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestDelay::new(self.1)))
    }
}

/// A module whose output depends on the audio before the sample it outputs: `latency` samples of
/// delay, then the mean of the last `window` delayed inputs (`y[n] = mean(x[n − L − W + 1 ..= n −
/// L])`). Cold after `reset()` (the missing history counts as zeros).
struct Boxcar {
    descriptor: ModuleDescriptor,
    latency: usize,
    window: usize,
    /// The last `latency + window` inputs (a ring), and the running sum of the window.
    ring: Vec<f32>,
    head: usize,
    sum: f64,
}

impl Boxcar {
    const ID: &'static str = "org.powervoice.test-h46-boxcar";
    const LATENCY: u32 = 2048;
    const WINDOW: usize = 1024;

    fn new() -> Self {
        Self {
            descriptor: descriptor(Self::ID),
            latency: Self::LATENCY as usize,
            window: Self::WINDOW,
            ring: Vec::new(),
            head: 0,
            sum: 0.0,
        }
    }
}

impl Module for Boxcar {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn activate(&mut self, _config: &ActivateConfig) -> Result<(), ModuleError> {
        self.ring = vec![0.0; self.latency + self.window];
        self.reset();
        Ok(())
    }
    fn deactivate(&mut self) {
        self.ring = Vec::new();
    }
    fn latency_samples(&self) -> u32 {
        self.latency as u32
    }
    fn tail(&self) -> Tail {
        Tail::Samples((self.latency + self.window) as u64)
    }
    fn process(
        &mut self,
        _ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let len = self.ring.len();
        for (x, y) in inputs[0].iter().zip(outputs[0].iter_mut()) {
            // `ring[head]` is x[n − L − W] (leaving the window); x[n − L] enters it.
            let leaving = self.ring[self.head];
            self.ring[self.head] = *x;
            self.head = (self.head + 1) % len;
            let entering = self.ring[(self.head + self.window - 1) % len];
            self.sum += f64::from(entering) - f64::from(leaving);
            *y = (self.sum / self.window as f64) as f32;
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.head = 0;
        self.sum = 0.0;
    }
    fn param_value(&self, _id: ParamId) -> Option<f64> {
        None
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _state: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}

struct BoxcarFactory(ModuleDescriptor);

impl ModuleFactory for BoxcarFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Boxcar::new()))
    }
}

fn one_slot(id: &str) -> RackModel {
    RackModel {
        slots: vec![SlotModel::new(
            &ModuleRef {
                id: id.into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState::new(1),
        )],
    }
}

/// An exact delay of `latency` samples (an empty rack for 0).
fn delay_rack(latency: u32) -> RackModel {
    if latency == 0 {
        RackModel { slots: Vec::new() }
    } else {
        one_slot(TestDelay::ID)
    }
}

fn dev_256() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn dev_64() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(64)
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    frames: Arc<Mutex<Vec<TelemetryFrame>>>,
    /// Simulated time and recorded output length after every step of [`Rig::steps`].
    written: Vec<(u64, usize)>,
    _dir: TempDir,
}

/// A `doc_rate` document `samples` on a stereo DAC (`dev`) through `rack`; the registry holds
/// `TestDelay(delay)` and the [`Boxcar`].
fn rig(dev: FakeDirection, samples: &[f32], doc_rate: u32, rack: RackModel, delay: u32) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(42);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev.record_output()),
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
    let snapshot = Arc::new(DocSnapshot::new(doc_rate, audio.pieces, Vec::new()));

    let factories: Vec<Arc<dyn ModuleFactory>> = vec![
        Arc::new(DelayFactory(
            TestDelay::new(delay).descriptor().clone(),
            delay,
        )),
        Arc::new(BoxcarFactory(Boxcar::new().descriptor().clone())),
    ];
    let registry = Arc::new(
        Registry::with_factories(
            vox_modules::builtin_factories()
                .into_iter()
                .chain(factories),
        )
        .unwrap(),
    );
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack;
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    let fr = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f| fr.lock().unwrap().push(*f))));
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    let mut r = Rig {
        fake,
        eng,
        frames,
        written: Vec::new(),
        _dir: dir,
    };
    r.run_ms(20);
    r
}

impl Rig {
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.fake.advance_by(MS);
            self.eng.tick();
        }
    }

    /// Runs `ms` in [`STEP_NS`] steps, recording when each output frame was written.
    fn steps(&mut self, ms: u64) {
        for _ in 0..ms * MS / STEP_NS {
            self.fake.advance_by(STEP_NS);
            self.eng.tick();
            let len = self.output().samples.len();
            self.written.push((self.fake.now_ns(), len));
        }
    }

    /// The simulated time output frame `k` had been written by (a [`Rig::steps`] step).
    fn written_at(&self, k: usize) -> u64 {
        self.written
            .iter()
            .find(|&&(_, len)| len > k)
            .map(|&(t, _)| t)
            .unwrap_or_else(|| panic!("frame {k} never written"))
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

    fn recorded(&self) -> Vec<f32> {
        self.output().samples
    }

    fn frames(&self) -> Vec<TelemetryFrame> {
        self.frames.lock().unwrap().clone()
    }

    /// Sends `cmd`, then runs 300 ms in fine steps. Returns (output length, time) at the command.
    fn timed(&mut self, cmd: TransportCommand) -> (usize, u64) {
        let at = self.recorded().len();
        let t0 = self.fake.now_ns();
        assert!(self.eng.transport(cmd).playing);
        self.steps(300);
        (at, t0)
    }
}

/// Index of the first occurrence of `needle` in `rec` at or after `from` (exact to 1e-6).
fn find(rec: &[f32], from: usize, needle: &[f32]) -> Option<usize> {
    (from..rec.len().saturating_sub(needle.len())).find(|&k| {
        needle
            .iter()
            .enumerate()
            .all(|(j, &x)| (rec[k + j] - x).abs() <= 1e-6)
    })
}

/// The output frame that carries document sample `p` (located past the fade-in), at or after
/// `from`.
fn frame_of(rec: &[f32], src: &[f32], p: usize, from: usize) -> usize {
    find(rec, from, &src[p + 400..p + 464])
        .unwrap_or_else(|| panic!("document sample {p} is never heard"))
        - 400
}

/// The first non-zero output frame at or after `from`.
fn first_sound(rec: &[f32], from: usize) -> usize {
    (from..rec.len())
        .find(|&i| rec[i] != 0.0)
        .expect("audio after the command")
}

/// The output frame played at app time `t_ns`.
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
    let into = ((t_ns - t0) * 48_000 + 500_000_000) / 1_000_000_000;
    Some((first + into) as usize)
}

/// The document sample heard at output frame `k`: the unique `q` in `lo..hi` with
/// `rec[k..k + 32] == src[q..q + 32]`.
fn heard_at(rec: &[f32], src: &[f32], k: usize, lo: usize, hi: usize) -> Option<usize> {
    if k + 32 > rec.len() {
        return None;
    }
    (lo..hi.min(src.len() - 32)).find(|&q| (0..32).all(|j| (rec[k + j] - src[q + j]).abs() <= 1e-6))
}

/// Every telemetry anchor of the pass that starts at output frame `k0` with document sample `p`
/// names what is heard: `p + (k − k0)` inside the fade-in, the sample found in the output past
/// it. Returns how many anchors were checked.
fn check_anchors(r: &Rig, src: &[f32], k0: usize, p: u64, end: usize) -> usize {
    let out = r.output();
    let t_k0 = r.fake.frame_time_ns(r.stream(), k0 as u64).unwrap();
    let mut checked = 0;
    for f in r.frames() {
        if f.rate <= 0.0 || f.playhead_time_ns + 1_000 < t_k0 {
            continue;
        }
        let Some(k) = frame_at(r, &out, f.playhead_time_ns) else {
            continue;
        };
        if k >= end {
            continue;
        }
        let ph = f.playhead_sample;
        assert!(
            ph >= p,
            "the playhead reads {ph} before the play position {p}"
        );
        if k < k0 + FADE {
            let want = p + (k - k0) as u64;
            assert!(
                ph.abs_diff(want) <= 1,
                "in the fade-in the playhead reads {ph} at frame {k}, {want} is heard"
            );
        } else {
            let q = heard_at(&out.samples, src, k, p as usize, src.len())
                .unwrap_or_else(|| panic!("nothing of the document heard at frame {k}"));
            assert!(
                ph.abs_diff(q as u64) <= 1,
                "the playhead reads {ph} while {q} is heard"
            );
        }
        checked += 1;
    }
    checked
}

/// Asserts that the pass starting at output frame `k0` is document sample `p` onwards: faded in
/// over [`FADE`] frames (same sign, never louder), then exact for `n` samples.
fn assert_heard_from(rec: &[f32], src: &[f32], k0: usize, p: usize, n: usize) {
    for j in 0..FADE {
        let (y, x) = (rec[k0 + j], src[p + j]);
        assert!(
            y.abs() <= x.abs() + 1e-6 && y * x >= 0.0,
            "fade-in frame {j}: {y} for {x}"
        );
    }
    for j in FADE..n {
        assert!(
            (rec[k0 + j] - src[p + j]).abs() <= 1e-6,
            "frame {} carries {} instead of document sample {} ({})",
            k0 + j,
            rec[k0 + j],
            p + j,
            src[p + j]
        );
    }
}

/// SPEC-003 AC-1 + Amendment 2: with a rack of 0, 480, 2048 and 4800 samples of latency (4800:
/// the pre-roll and the look-ahead exceed the reader's read-ahead, so the pre-roll spans reader
/// refills), Play is heard as fast as with an empty rack, the first sample heard is the play
/// position, nothing before it leaks out, and the playhead names what is heard from the first
/// frame on.
#[test]
fn the_first_heard_sample_is_the_play_position() {
    let src = noise(1, 4 * 48_000);
    let p = 30_000usize;
    let mut base = None;
    for latency in [0u32, 480, 2048, 4800] {
        let mut r = rig(dev_256(), &src, 48_000, delay_rack(latency), latency);
        assert_eq!(r.eng.rack_snapshot().latency_samples, latency);
        r.eng.transport(TransportCommand::Seek(p as u64));
        let (at, t0) = r.timed(TransportCommand::Play);
        let rec = r.recorded();
        let k0 = frame_of(&rec, &src, p, at);
        assert_eq!(
            first_sound(&rec, at),
            k0 + 1,
            "latency {latency}: something before the play position was heard"
        );
        assert_heard_from(&rec, &src, k0, p, 8_000);
        let t = r.written_at(k0 + 1) - t0;
        let base_ns = *base.get_or_insert(t);
        assert!(
            t <= base_ns + START_SLACK_NS,
            "latency {latency}: heard {:.1} ms after Play (empty rack: {:.1} ms)",
            t as f64 / 1e6,
            base_ns as f64 / 1e6
        );
        let checked = check_anchors(&r, &src, k0, p as u64, rec.len());
        assert!(checked > 10, "latency {latency}: {checked} anchors");
        assert_eq!(r.fake.rt_violations(), 0, "latency {latency}: allocated");
    }
}

/// SPEC-003 Amendment 2 (warm-up): the rack enters the play position with the audio before it in
/// its state. Through the [`Boxcar`] (2048 samples of latency, a 1024-sample moving mean), the
/// first samples heard are the means over real audio, not over the silence of a cold start.
#[test]
fn the_preroll_warms_the_rack_up_with_the_audio_before_the_play_position() {
    // Noise on a DC offset, so a cold window (zeros before the play position) is far off.
    let src: Vec<f32> = noise(2, 3 * 48_000).iter().map(|x| x + 0.25).collect();
    let p = 60_000usize;
    let mut r = rig(dev_256(), &src, 48_000, one_slot(Boxcar::ID), 0);
    assert_eq!(r.eng.rack_snapshot().latency_samples, Boxcar::LATENCY);
    r.eng.transport(TransportCommand::Seek(p as u64));
    let (at, _) = r.timed(TransportCommand::Play);
    let rec = r.recorded();
    // The fade-in's first frame has gain 0: the play position is the frame before the first sound.
    let k0 = first_sound(&rec, at) - 1;
    let w = Boxcar::WINDOW;
    for j in FADE..3 * w {
        let q = p + j;
        let want = src[q + 1 - w..=q]
            .iter()
            .map(|&x| f64::from(x))
            .sum::<f64>()
            / w as f64;
        assert!(
            (f64::from(rec[k0 + j]) - want).abs() <= 1e-4,
            "frame {j} after the start: {} instead of the warm mean {want}",
            rec[k0 + j]
        );
    }
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 Amendment 2 (seek while playing): the heard output fades out at once (not after the
/// rack latency), the rack is reset and pre-rolled at the target, and the target is heard as fast
/// as with an empty rack; nothing of the pre-roll leaks out; the playhead stays exact.
#[test]
fn a_seek_while_playing_jumps_without_waiting_for_the_rack() {
    let src = noise(3, 5 * 48_000);
    let p2 = 150_003usize;
    let mut base = None;
    for latency in [0u32, 480, 2048, 4800] {
        let mut r = rig(dev_256(), &src, 48_000, delay_rack(latency), latency);
        assert!(r.eng.transport(TransportCommand::Play).playing);
        r.run_ms(400);
        let (at, t0) = r.timed(TransportCommand::Seek(p2 as u64));
        let rec = r.recorded();
        let k2 = frame_of(&rec, &src, p2, at);
        // The old audio fades out within one fade of the command, then silence until the target.
        let z0 = (at..k2)
            .find(|&i| rec[i..k2].iter().all(|&x| x == 0.0))
            .unwrap_or(k2);
        assert!(
            z0 <= at + FADE + 1,
            "latency {latency}: the old audio went on until frame {z0} (command at {at})"
        );
        assert_heard_from(&rec, &src, k2, p2, 8_000);
        let t = r.written_at(k2 + 1) - t0;
        let base_ns = *base.get_or_insert(t);
        assert!(
            t <= base_ns + START_SLACK_NS,
            "latency {latency}: the target heard {:.1} ms after the seek (empty rack: {:.1} ms)",
            t as f64 / 1e6,
            base_ns as f64 / 1e6
        );
        let checked = check_anchors(&r, &src, k2, p2 as u64, rec.len());
        assert!(checked > 10, "latency {latency}: {checked} anchors");
        assert_eq!(r.fake.rt_violations(), 0, "latency {latency}: allocated");
    }
}

/// The seek's fade-out (after the rack) and the pre-rolled fade-in pass SPEC-012 §4.3 at the
/// transition, with 2048 samples of rack latency.
#[test]
fn a_seek_through_a_rack_with_latency_does_not_click() {
    let src = tone(5 * 48_000);
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(2048), 2048);
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(600);
    let at = r.recorded().len();
    r.eng.transport(TransportCommand::Seek(100_003));
    r.run_ms(600);
    assert!(r.eng.transport_state().playing);
    let rec = r.recorded();
    let z0 = (at..rec.len() - 1)
        .find(|&i| rec[i] == 0.0 && rec[i + 1] == 0.0)
        .expect("a silent gap");
    let z1 = (z0..rec.len()).find(|&i| rec[i] != 0.0).expect("audio");
    let report = analyze_zipper(
        &rec,
        0,
        48_000.0,
        ZipperWindow {
            start: z0 - FADE,
            end: z1,
            t_s_ms: 5.0,
        },
    )
    .expect("steady tone around the transition");
    assert!(report.pass, "§4.3 at the transition {z0}..{z1}: {report}");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// Pause, then Play again at the pause position (no rack reset, SPEC-003 §4): the resume is
/// heard at once, from exactly the position the pause stopped at.
#[test]
fn a_resume_after_pause_is_heard_at_once_from_the_pause_position() {
    let src = noise(4, 4 * 48_000);
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(2048), 2048);
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(500);
    assert!(!r.eng.transport(TransportCommand::Pause).playing);
    r.run_ms(300);
    let q = r.eng.transport_state().playhead_samples as usize;
    let (at, t0) = r.timed(TransportCommand::Play);
    let rec = r.recorded();
    let k = frame_of(&rec, &src, q, at);
    assert_eq!(first_sound(&rec, at), k + 1, "the resume starts at {q}");
    assert_heard_from(&rec, &src, k, q, 8_000);
    let t = r.written_at(k + 1) - t0;
    assert!(t <= 12 * MS, "resumed {:.1} ms after Play", t as f64 / 1e6);
    assert!(check_anchors(&r, &src, k, q as u64, rec.len()) > 10);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-37 regression: a loop played from its start through 2048 samples of rack latency starts at
/// S at once and repeats sample-exactly; the playhead names what is heard on every pass.
#[test]
fn a_loop_through_a_rack_with_latency_starts_at_once_and_stays_exact() {
    let src = noise(5, 3 * 48_000);
    let (s, e) = (30_000usize, 42_000usize);
    let l = e - s;
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(2048), 2048);
    r.eng.set_selection(Some((s as u64, e as u64)));
    assert_eq!(r.eng.set_loop(true).loop_range, Some((s as u64, e as u64)));
    let (at, t0) = r.timed(TransportCommand::PlayFromStart);
    r.run_ms(600);
    let rec = r.recorded();
    let k0 = frame_of(&rec, &src, s, at);
    assert_eq!(first_sound(&rec, at), k0 + 1);
    assert_heard_from(&rec, &src, k0, s, l);
    for pass in 1..3 {
        let base = k0 + pass * l;
        assert!(
            (0..l).all(|j| (rec[base + j] - src[s + j]).abs() <= 1e-6),
            "pass {} differs",
            pass + 1
        );
    }
    assert!(r.written_at(k0 + 1) - t0 <= 12 * MS);
    // Anchors past the first fade-in, away from the seams (a 32-sample window can't straddle one).
    let out = r.output();
    let t_start = r
        .fake
        .frame_time_ns(r.stream(), (k0 + FADE) as u64)
        .unwrap();
    let mut checked = 0;
    for f in r.frames() {
        if f.rate <= 0.0 || f.playhead_time_ns < t_start {
            continue;
        }
        let Some(k) = frame_at(&r, &out, f.playhead_time_ns) else {
            continue;
        };
        if let Some(q) = heard_at(&out.samples, &src, k, s, e) {
            assert!(
                f.playhead_sample.abs_diff(q as u64) <= 1,
                "the playhead reads {} while {q} is heard",
                f.playhead_sample
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked} anchors");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 Amendment 1 ("a Play at or after E plays on without looping") with a pre-roll that
/// reaches back across the loop end: the pre-roll does not wrap, playback goes on past E.
#[test]
fn a_preroll_across_the_loop_end_does_not_wrap() {
    let src = noise(6, 3 * 48_000);
    let (s, e) = (12_000u64, 21_600u64);
    let p = e as usize + 1_000;
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(4800), 4800);
    r.eng.set_selection(Some((s, e)));
    assert_eq!(r.eng.set_loop(true).loop_range, Some((s, e)));
    r.eng.transport(TransportCommand::Seek(p as u64));
    let (at, _) = r.timed(TransportCommand::Play);
    r.run_ms(300);
    let rec = r.recorded();
    let k0 = frame_of(&rec, &src, p, at);
    assert_heard_from(&rec, &src, k0, p, 20_000);
    assert!(r.eng.transport_state().playing);
    assert!(rec.len() > k0 + 20_000, "ran long enough");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// T-401 regression: a Play 1000 samples before the document end with 4800 samples of rack
/// latency (the end is inside the look-ahead): the 1000 samples are heard at once, and the
/// transport ends once the last one has been heard, at the end.
#[test]
fn an_end_inside_the_lookahead_is_heard_at_once_and_ends_the_transport() {
    let n = 48_000usize;
    let src = noise(7, n);
    let p = n - 1_000;
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(4800), 4800);
    r.eng.transport(TransportCommand::Seek(p as u64));
    let at = r.recorded().len();
    let t0 = r.fake.now_ns();
    assert!(r.eng.transport(TransportCommand::Play).playing);
    let mut stopped_len = None;
    for _ in 0..3_000 {
        r.steps(1);
        if stopped_len.is_none() && !r.eng.transport_state().playing {
            stopped_len = Some(r.recorded().len());
        }
    }
    let rec = r.recorded();
    let k0 = first_sound(&rec, at) - 1;
    assert_heard_from(&rec, &src, k0, p, 1_000);
    assert!(
        rec[k0 + 1_000..].iter().all(|&x| x == 0.0),
        "silence after the end"
    );
    assert!(r.written_at(k0 + 1) - t0 <= 12 * MS);
    let stopped_len = stopped_len.expect("the transport ends");
    assert!(
        stopped_len >= k0 + 1_000 && stopped_len <= k0 + 1_000 + 2 * 256 + 96,
        "stopped at output frame {stopped_len}, the last sample is frame {}",
        k0 + 999
    );
    assert_eq!(r.eng.transport_state().playhead_samples, n as u64);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// A Stop while the rack is still being pre-rolled (64-frame callbacks, 8192 samples of latency:
/// the pre-roll spans several callbacks) leaves silence — nothing of the pre-roll is heard — and
/// stops at the play position; the next Play starts normally.
#[test]
fn a_stop_during_the_preroll_leaves_silence() {
    let src = noise(8, 3 * 48_000);
    let p = 40_000u64;
    let mut r = rig(dev_64(), &src, 48_000, delay_rack(8192), 8192);
    r.eng.transport(TransportCommand::Seek(p));
    let at = r.recorded().len();
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.steps(2);
    assert!(
        r.recorded()[at..].iter().all(|&x| x == 0.0),
        "the pre-roll should still be running"
    );
    let st = r.eng.transport(TransportCommand::Stop);
    assert!(!st.playing);
    r.run_ms(400);
    assert!(
        r.recorded()[at..].iter().all(|&x| x == 0.0),
        "something was heard after the Stop"
    );
    assert_eq!(r.eng.transport_state().playhead_samples, p);
    let (at, t0) = r.timed(TransportCommand::Play);
    let rec = r.recorded();
    let k0 = frame_of(&rec, &src, p as usize, at);
    assert_eq!(first_sound(&rec, at), k0 + 1);
    assert_heard_from(&rec, &src, k0, p as usize, 10_000);
    assert!(r.written_at(k0 + 1) - t0 <= 25 * MS);
    assert_eq!(r.fake.rt_violations(), 0);
}

/// ADR-002 §5 (device ≠ document rate): a 44.1 kHz document on a 48 kHz-only device through
/// 2048 samples of rack latency starts as fast as through an empty rack, and the playhead starts
/// at the play position.
#[test]
fn a_resampled_start_does_not_wait_for_the_rack() {
    let src = noise(9, 3 * 44_100);
    let p = 30_000u64;
    let mut base = None;
    for latency in [0u32, 2048] {
        let mut r = rig(dev_256(), &src, 44_100, delay_rack(latency), latency);
        r.eng.transport(TransportCommand::Seek(p));
        let (at, t0) = r.timed(TransportCommand::Play);
        let rec = r.recorded();
        let k = first_sound(&rec, at);
        let t = r.written_at(k) - t0;
        let base_ns = *base.get_or_insert(t);
        assert!(
            t <= base_ns + START_SLACK_NS,
            "latency {latency}: heard {:.1} ms after Play (empty rack: {:.1} ms)",
            t as f64 / 1e6,
            base_ns as f64 / 1e6
        );
        let first = r
            .frames()
            .into_iter()
            .find(|f| f.rate > 0.0 && f.playhead_time_ns >= t0)
            .expect("an anchor");
        assert!(
            first.playhead_sample.abs_diff(p) <= 2,
            "latency {latency}: the first anchor reads {}",
            first.playhead_sample
        );
        assert_eq!(r.fake.rt_violations(), 0);
    }
}
