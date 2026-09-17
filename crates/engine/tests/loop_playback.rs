//! H-37: loop playback (SPEC-003 §2.1, AC-4 and its H-37 amendment; ADR-002 §5, §8 and
//! Amendment 3). Every output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::StreamId;
use vox_engine::backend::fake::{
    CallbackSizes, FakeBackend, FakeDevice, FakeDirection, RecordedOutput,
};
use vox_engine::bake::{DocumentRange, render_document_range_to_vec};
use vox_engine::{
    Direction, EngineConfig, HostId, ManualEngine, PlaybackDoc, TelemetryFrame, TransportCommand,
};
use vox_module_api::test_util::{
    TestDelay, ZIPPER_FREQ_HZ, ZIPPER_LEVEL_DBFS, ZipperWindow, alloc_checks_active,
    analyze_zipper, no_alloc,
};
use vox_module_api::{
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, Version,
};
use vox_project::{CancelToken, ChunkStore, ChunkWriter, DocSnapshot, Range, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};

const MS: u64 = 1_000_000;
const RATE: u64 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-loop-{tag}-{}-{}",
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

fn sine(freq_hz: f64, rate: u32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            (0.5 * (std::f64::consts::TAU * freq_hz * i as f64 / f64::from(rate)).sin()) as f32
        })
        .collect()
}

/// The SPEC-012 §4.3 tone (997 Hz, −20 dBFS peak, 48 kHz): exactly 997 periods per second, so a
/// one-second loop of it is a continuous tone.
fn tone(n: usize) -> Vec<f32> {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let w = std::f64::consts::TAU * ZIPPER_FREQ_HZ / 48_000.0;
    (0..n)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect()
}

/// `TestDelay(latency)` for the registry.
struct DelayFactory(ModuleDescriptor, u32);

impl ModuleFactory for DelayFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestDelay::new(self.1)))
    }
}

/// An empty (unity) rack.
fn empty_rack() -> RackModel {
    RackModel { slots: Vec::new() }
}

/// [TestDelay] (its latency is the rig's `delay`).
fn delay_rack() -> RackModel {
    RackModel {
        slots: vec![SlotModel::new(
            &ModuleRef {
                id: TestDelay::ID.into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState::new(1),
        )],
    }
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    frames: Arc<Mutex<Vec<TelemetryFrame>>>,
    registry: Arc<Registry>,
    store: Arc<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    _dir: TempDir,
}

fn dev_256() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn dev_random() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).callback_sizes(CallbackSizes::FULL_RANDOM)
}

/// A `doc_rate` document `samples` through `rack` on a stereo DAC (`dev`); the delay module in
/// the registry has latency `delay`.
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

    let delay_factory: Arc<dyn ModuleFactory> = Arc::new(DelayFactory(
        TestDelay::new(delay).descriptor().clone(),
        delay,
    ));
    let registry = Arc::new(
        Registry::with_factories(
            vox_modules::builtin_factories()
                .into_iter()
                .chain([delay_factory]),
        )
        .unwrap(),
    );
    let frames = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry.clone());
    cfg.rack = rack;
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    let fr = frames.clone();
    eng.set_telemetry_sink(Some(Box::new(move |f| fr.lock().unwrap().push(*f))));
    eng.set_document(Some(PlaybackDoc {
        store: store.clone(),
        snapshot: snapshot.clone(),
    }));
    eng.poll_devices();
    Rig {
        fake,
        eng,
        frames,
        registry,
        store,
        snapshot,
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

    fn recorded(&self) -> Vec<f32> {
        self.output().samples
    }

    fn frames(&self) -> Vec<TelemetryFrame> {
        self.frames.lock().unwrap().clone()
    }

    /// Selects `[s, e)`, turns loop on and plays from the selection start.
    fn loop_play(&mut self, s: u64, e: u64) {
        self.run_ms(20);
        self.eng.set_selection(Some((s, e)));
        let st = self.eng.set_loop(true);
        assert!(st.loop_enabled);
        assert_eq!(st.loop_range, Some((s, e)));
        assert!(self.eng.transport(TransportCommand::PlayFromStart).playing);
    }
}

/// Index in `rec` of the first occurrence of `needle` (64 samples, exact to 1e-6).
fn find(rec: &[f32], needle: &[f32]) -> Option<usize> {
    (0..rec.len().saturating_sub(needle.len())).find(|&k| {
        needle
            .iter()
            .enumerate()
            .all(|(j, &x)| (rec[k + j] - x).abs() <= 1e-6)
    })
}

/// The output index of the loop start `s` in the first pass (located past the fade-in).
fn pass_base(rec: &[f32], src: &[f32], s: usize) -> usize {
    let probe = s + 400;
    find(rec, &src[probe..probe + 64]).expect("the loop in the output") - 400
}

/// Max abs difference between `rec[at..]` and `want`.
fn max_diff(rec: &[f32], at: usize, want: &[f32]) -> f32 {
    want.iter()
        .enumerate()
        .map(|(i, &x)| (rec[at + i] - x).abs())
        .fold(0.0, f32::max)
}

/// SPEC-003 AC-4: with an empty (unity) rack and random callback sizes, three passes of a loop
/// are sample-exact copies of `source[S..E)` — no dropped, duplicated or gap samples at a seam,
/// and the sample after E − 1 is S. Looping repeats until Stop. No allocation in the callback.
#[test]
fn ac4_three_passes_are_sample_exact() {
    let src = noise(1, 2 * 48_000);
    let (s, e) = (12_000usize, 21_600usize);
    let l = e - s;
    let mut r = rig(dev_random(), &src, 48_000, empty_rack(), 0);
    r.loop_play(s as u64, e as u64);
    r.run_ms(900);
    assert!(
        r.eng.transport_state().playing,
        "looping repeats until Stop"
    );
    let rec = r.recorded();
    let base = pass_base(&rec, &src, s);
    assert!(rec.len() > base + 3 * l, "three passes recorded");
    // Pass 1 after the 5 ms fade-in, then passes 2 and 3 in full.
    assert!(
        max_diff(&rec, base + 240, &src[s + 240..e]) <= 1e-6,
        "pass 1"
    );
    for pass in 1..3 {
        let d = max_diff(&rec, base + pass * l, &src[s..e]);
        assert!(d <= 1e-6, "pass {} differs by {d}", pass + 1);
    }
    assert!((rec[base + l - 1] - src[e - 1]).abs() <= 1e-6);
    assert!(
        (rec[base + l] - src[s]).abs() <= 1e-6,
        "the sample after E − 1 is S"
    );
    let st = r.eng.transport(TransportCommand::Stop);
    assert_eq!(
        st.playhead_samples, s as u64,
        "Stop returns to the play start"
    );
    r.run_ms(50);
    assert!(!r.eng.transport_state().playing);
    assert_eq!(r.fake.rt_violations(), 0, "the output callback allocated");
}

/// H-37 amendment of AC-4: the rack is not reset at a seam — it keeps processing the looped
/// stream, so its tails continue. Through an exact 480-sample delay the output is the looped
/// stream delayed by 480 across every seam (a reset would clear the delay line: 480 zeros).
#[test]
fn the_rack_keeps_running_across_the_seam() {
    let src = noise(2, 2 * 48_000);
    let (s, e) = (12_000usize, 21_600usize);
    let l = e - s;
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(), 480);
    r.loop_play(s as u64, e as u64);
    r.run_ms(900);
    let rec = r.recorded();
    let base = pass_base(&rec, &src, s);
    for pass in 1..3 {
        let d = max_diff(&rec, base + pass * l, &src[s..e]);
        assert!(d <= 1e-6, "pass {} differs by {d}", pass + 1);
    }
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 AC-4 / SPEC-012 §4.3: PowerVoice adds no artifact at the seam. A one-second loop
/// of the 997 Hz tone is a continuous tone, so every seam passes the §4.3 click analysis and no
/// sample-to-sample step exceeds the tone's own slope.
#[test]
fn the_seam_adds_no_click() {
    let src = tone(3 * 48_000);
    let (s, e) = (24_000usize, 72_000usize);
    let l = e - s;
    let mut r = rig(dev_random(), &src, 48_000, empty_rack(), 0);
    r.loop_play(s as u64, e as u64);
    // Past the second seam by more than the §4.3 analysis window.
    r.run_ms(3_000);
    let rec = r.recorded();
    let base = pass_base(&rec, &src, s);
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let slope = (amp * std::f64::consts::TAU * ZIPPER_FREQ_HZ / 48_000.0 + 1e-4) as f32;
    for seam in [base + l, base + 2 * l] {
        let report = analyze_zipper(&rec, 0, 48_000.0, ZipperWindow::step(seam, 5.0))
            .expect("steady tone around the seam");
        assert!(report.pass, "§4.3 at the seam {seam}: {report}");
        let step = rec[seam - 64..seam + 64]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            step <= slope,
            "step {step} at the seam {seam} above {slope}"
        );
    }
    assert_eq!(r.fake.rt_violations(), 0);
}

/// The output frame played at app time `t_ns` (an anchor's time is the playback time of a
/// callback's first frame).
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

/// The document sample heard at output frame `k`: the `q` in `[lo, hi)` with
/// `rec[k..k + 32] == src[q..q + 32]`.
fn heard_at(rec: &[f32], src: &[f32], k: usize, lo: usize, hi: usize) -> Option<usize> {
    if k + 32 > rec.len() {
        return None;
    }
    (lo..hi.min(src.len() - 32)).find(|&q| (0..32).all(|j| (rec[k + j] - src[q + j]).abs() <= 1e-6))
}

/// ADR-002 §8 (H-37 item 2): with 100 ms of rack latency, the playhead names the sample leaving
/// the device across every wrap — just after the rack input wraps to S, the heard sample is
/// still in the last 100 ms before E, and the playhead says so (it is not clamped to the play
/// start).
#[test]
fn the_heard_position_maps_back_into_the_loop_end_after_a_wrap() {
    let src = noise(3, 2 * 48_000);
    let (s, e) = (24_000usize, 48_000usize);
    let latency = 4_800usize;
    let mut r = rig(dev_256(), &src, 48_000, delay_rack(), latency as u32);
    r.loop_play(s as u64, e as u64);
    let t_play = r.fake.now_ns();
    r.run_ms(1_800);
    let out = r.output();
    let mut checked = 0;
    let mut after_wrap = 0;
    for f in r.frames() {
        if f.rate <= 0.0 || f.playhead_time_ns < t_play + 250 * MS {
            continue;
        }
        let k = frame_at(&r, &out, f.playhead_time_ns).expect("a frame");
        let p = f.playhead_sample as usize;
        assert!((s..e).contains(&p), "the playhead {p} left the loop");
        let q = heard_at(&out.samples, &src, k, s, e)
            .unwrap_or_else(|| panic!("nothing heard at the anchor {p}"));
        assert!(
            (p as i64 - q as i64).abs() <= 1,
            "the playhead reads {p} while {q} is heard"
        );
        checked += 1;
        if p >= e - latency {
            after_wrap += 1;
        }
    }
    assert!(checked > 60, "{checked} anchors checked");
    assert!(
        after_wrap >= 6,
        "{after_wrap} anchors in the post-wrap window"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 §2.1 (H-37 amendment): turning loop off during looped playback lets the pass being
/// heard finish, then playback stops at the old loop end (Pause semantics: the playhead stays at
/// E). Covers a toggle well before the seam (the reader has not wrapped yet) and one 30 ms before
/// it (the reader, ~200 ms ahead, already queued the next pass).
#[test]
fn turning_loop_off_mid_pass_finishes_the_pass_and_stops_at_the_loop_end() {
    let src = noise(4, 2 * 48_000);
    let (s, e) = (12_000usize, 36_000usize);
    let l = e - s;
    for before_seam_ms in [250usize, 30] {
        let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
        r.loop_play(s as u64, e as u64);
        r.run_ms(100);
        let base = pass_base(&r.recorded(), &src, s);
        // Toggle during the second pass, `before_seam_ms` before its end.
        let toggle_at = base + 2 * l - before_seam_ms * 48;
        for _ in 0..2_000 {
            if r.recorded().len() >= toggle_at {
                break;
            }
            r.run_ms(1);
        }
        let st = r.eng.set_loop(false);
        assert!(!st.loop_enabled);
        assert!(st.playing, "the pass is not cut off");
        r.run_ms(1_000);
        let st = r.eng.transport_state();
        assert!(!st.playing, "{before_seam_ms} ms: stopped after the pass");
        assert_eq!(
            st.playhead_samples, e as u64,
            "{before_seam_ms} ms: at the loop end"
        );
        let rec = r.recorded();
        let d = max_diff(&rec, base + l, &src[s..e]);
        assert!(
            d <= 1e-6,
            "{before_seam_ms} ms: the second pass differs by {d}"
        );
        assert!(
            rec[base + 2 * l..].iter().all(|&x| x == 0.0),
            "{before_seam_ms} ms: nothing after the old loop end"
        );
        assert_eq!(r.fake.rt_violations(), 0);
    }
}

/// SPEC-003 §2.1: turning loop on during playback (before the selection end) wraps at the
/// selection end.
#[test]
fn turning_loop_on_mid_play_wraps_at_the_selection_end() {
    let src = noise(5, 2 * 48_000);
    let (s, e) = (24_000usize, 48_000usize);
    let l = e - s;
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.run_ms(20);
    let st = r.eng.set_selection(Some((s as u64, e as u64)));
    assert_eq!(st.loop_range, None, "loop is off");
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(200);
    assert!(r.eng.set_loop(true).loop_range.is_some());
    r.run_ms(1_300);
    assert!(r.eng.transport_state().playing);
    let rec = r.recorded();
    let base = pass_base(&rec, &src, s);
    for pass in 0..2 {
        let d = max_diff(&rec, base + pass * l, &src[s..e]);
        assert!(d <= 1e-6, "pass {} differs by {d}", pass + 1);
    }
    assert!(
        (rec[base - 1] - src[s - 1]).abs() <= 1e-6,
        "played into the loop from before it"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-80 (SPEC-003 §2.1/§3, Amendment 3): loop with no time selection loops the whole document —
/// the owner-requested feature (the previous "inert with no selection" reading is superseded).
/// Three passes over the whole document are sample-exact, the seam is seamless, and playback
/// never stops at the document end while looping.
#[test]
fn loop_loops_the_whole_document_without_a_selection() {
    let src = noise(6, 24_000);
    let mut r = rig(dev_random(), &src, 48_000, empty_rack(), 0);
    r.run_ms(20);
    let st = r.eng.set_loop(true);
    assert!(st.loop_enabled);
    assert_eq!(
        st.loop_range,
        Some((0, src.len() as u64)),
        "H-80: no selection loops the whole document"
    );
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(2_500);
    assert!(
        r.eng.transport_state().playing,
        "looping repeats until Stop — no stop at the document end"
    );
    let rec = r.recorded();
    let base = pass_base(&rec, &src, 0);
    let l = src.len();
    assert!(rec.len() > base + 3 * l, "three passes recorded");
    for pass in 1..3 {
        let d = max_diff(&rec, base + pass * l, &src);
        assert!(d <= 1e-6, "pass {} differs by {d}", pass + 1);
    }
    assert!(
        (rec[base + l] - src[0]).abs() <= 1e-6,
        "the sample after the document end is the document start"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-80: a selection shorter than the 10 ms minimum falls back to the whole document, exactly
/// like no selection — the same choice as "no selection", so the Loop button's on-state always
/// means "this is actually looping" (never a silently-inert loop with the toggle lit).
#[test]
fn a_selection_under_the_minimum_falls_back_to_the_whole_document() {
    let src = noise(7, 24_000);
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.run_ms(20);
    let st = r.eng.set_loop(true);
    assert!(st.loop_enabled);
    assert_eq!(st.loop_range, Some((0, src.len() as u64)));
    assert_eq!(
        r.eng.set_selection(Some((100, 500))).loop_range,
        Some((0, src.len() as u64)),
        "shorter than 10 ms: falls back to the whole document, not inert"
    );
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(700);
    assert!(
        r.eng.transport_state().playing,
        "still looping — a too-short selection never silently stops looping"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-80 item 3: making a selection while looping the whole document switches the loop to that
/// selection; clearing the selection switches back to the whole document. No stop, no glitch at
/// the switch (playback never pauses, `playing` stays true throughout).
#[test]
fn switching_between_a_selection_and_the_whole_document_is_seamless() {
    let src = noise(8, 48_000);
    let (s, e) = (20_000u64, 30_000u64);
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.run_ms(20);
    let st = r.eng.set_loop(true);
    assert_eq!(st.loop_range, Some((0, src.len() as u64)));
    assert!(r.eng.transport(TransportCommand::PlayFromStart).playing);
    r.run_ms(20);
    assert!(r.eng.transport_state().playing);

    // A selection appears: the loop switches to it without stopping.
    let st = r.eng.set_selection(Some((s, e)));
    assert_eq!(st.loop_range, Some((s, e)));
    assert!(st.playing, "no stop at the switch");
    r.run_ms(300);
    assert!(r.eng.transport_state().playing, "looping the selection");
    let rec_mid = r.recorded();

    // Clearing the selection: back to the whole document, still without stopping.
    let st = r.eng.set_selection(None);
    assert_eq!(st.loop_range, Some((0, src.len() as u64)));
    assert!(st.playing, "no stop at the switch back");
    r.run_ms(300);
    assert!(
        r.eng.transport_state().playing,
        "looping the whole document again"
    );
    assert!(
        r.recorded().len() > rec_mid.len(),
        "playback kept advancing across both switches"
    );
    assert_eq!(r.fake.rt_violations(), 0);
}

/// The required tests' last row: loop toggled on during the final second of playback (no
/// selection, so the whole-document region's end is the document end) wraps back to the start
/// rather than stopping there.
#[test]
fn turning_loop_on_during_the_final_second_of_playback_wraps() {
    // 3 s, well over the ~200-330 ms read-ahead window, so enabling loop "in the final second"
    // finds the reader still short of the document end (not already finished / `a.done`).
    let src = noise(9, 3 * 48_000);
    let l = src.len();
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.run_ms(20);
    assert!(
        !r.eng
            .transport(TransportCommand::Seek((l - 48_000) as u64))
            .playing
    );
    assert!(r.eng.transport(TransportCommand::Play).playing);
    r.run_ms(500);
    let st = r.eng.set_loop(true);
    assert_eq!(st.loop_range, Some((0, l as u64)));
    r.run_ms(1_000);
    assert!(
        r.eng.transport_state().playing,
        "wrapped instead of stopping at the document end"
    );
    let rec = r.recorded();
    // Playback started at `l - 48_000` and never naturally revisits the document start; finding
    // it in the output is proof the reader wrapped back to 0 instead of stopping at `l`.
    find(&rec, &src[0..64]).expect("the document start recurs in the output: it wrapped");
    assert_eq!(r.fake.rt_violations(), 0);
}

/// SPEC-003 §2.4 + H-37: a 44.1 kHz document looped on a 48 kHz device. The reader resamples
/// the looped stream (one second = exactly 1000 periods of 1 kHz, so the looped signal is a
/// continuous tone): the output stays 1 kHz ± 0.05 % over three passes, no seam makes a jump,
/// and the playhead wraps inside the loop without drifting.
#[test]
fn a_resampled_loop_is_seamless_and_its_playhead_does_not_drift() {
    let src = sine(1_000.0, 44_100, 3 * 44_100);
    let (s, e) = (22_050u64, 66_150u64);
    let dev = FakeDirection::new(2, &[48_000], 48_000).default_buffer(256);
    let mut r = rig(dev, &src, 44_100, empty_rack(), 0);
    r.loop_play(s, e);
    let t_play = r.fake.now_ns();
    r.run_ms(3_300);
    assert!(r.eng.transport_state().playing);
    let rec = r.recorded();
    let first = rec.iter().position(|&x| x != 0.0).expect("audio") + 480;
    let body = &rec[first..rec.len() - 480];
    assert!(body.len() > 3 * 48_000, "three passes");
    // Frequency over all passes (rising zero crossings).
    let mut crossings = Vec::new();
    for i in 1..body.len() {
        let (a, b) = (f64::from(body[i - 1]), f64::from(body[i]));
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f64 + a / (a - b));
        }
    }
    let n = crossings.len() - 1;
    let f = n as f64 * 48_000.0 / (crossings[n] - crossings[0]);
    assert!((f - 1_000.0).abs() <= 0.5, "{f} Hz");
    // No jump anywhere (a dropped or doubled frame at a seam would double a step).
    let slope = (0.5 * std::f64::consts::TAU * 1_000.0 / 48_000.0) as f32;
    let step = body
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(step <= slope * 1.1, "step {step} (slope {slope})");
    // The playhead stays in the loop and advances at the document rate, modulo the loop.
    let l = (e - s) as f64;
    let anchors: Vec<(f64, f64)> = r
        .frames()
        .iter()
        .filter(|f| f.rate > 0.0 && f.playhead_time_ns > t_play + 200 * MS)
        .map(|f| (f.playhead_sample as f64, f.playhead_time_ns as f64))
        .collect();
    assert!(anchors.len() > 150);
    let (p0, t0) = anchors[0];
    for &(p, t) in &anchors {
        assert!((s as f64..e as f64).contains(&p), "{p} outside the loop");
        let want = (p0 - s as f64 + (t - t0) * 44_100.0 / 1e9).rem_euclid(l);
        let got = p - s as f64;
        let err = (got - want).abs().min(l - (got - want).abs());
        assert!(
            err <= 8.0,
            "playhead {p} is {err} samples off its loop-wrapped prediction"
        );
    }
    assert_eq!(r.fake.rt_violations(), 0);
}

/// H-37 item 4: export and bake render the document range, never the loop.
#[test]
fn export_and_bake_are_unaffected_by_loop() {
    let src = noise(7, 24_000);
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.loop_play(4_800, 9_600);
    r.run_ms(300);
    assert!(r.eng.transport_state().loop_range.is_some());
    let rendered = render_document_range_to_vec(
        &r.registry,
        &RackModel::default(),
        DocumentRange {
            store: &r.store,
            snapshot: &r.snapshot,
            sample_rate_hz: 48_000,
            range: Range {
                start: 0,
                end: src.len() as u64,
            },
        },
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(rendered, src, "the whole document, once");
}

/// H-81 (owner-reported "loop off doesn't stop the loop"): a very short loop (well under the
/// ~200-330 ms read-ahead) means the reader has already generated many passes ahead of what is
/// heard. Turning loop off must still stop promptly at the *currently heard* pass's end, and
/// `loop_range` must clear at once (it drives the overlay strip in every pane), not after however
/// many passes were pre-generated.
#[test]
fn h81_turning_loop_off_with_many_passes_already_queued() {
    let src = noise(8, 48_000);
    let (s, e) = (12_000usize, 13_000usize);
    let mut r = rig(dev_256(), &src, 48_000, empty_rack(), 0);
    r.loop_play(s as u64, e as u64);
    r.run_ms(500);
    let st = r.eng.set_loop(false);
    assert!(!st.loop_enabled, "loop_enabled reflects off immediately");
    assert_eq!(
        st.loop_range, None,
        "loop_range clears immediately (drives the overlay)"
    );
    r.run_ms(2_000);
    let st = r.eng.transport_state();
    assert!(!st.playing, "must have stopped by now");
    assert_eq!(
        st.playhead_samples, e as u64,
        "stopped exactly at the old loop end"
    );
}

/// H-81 regression: the REAL threaded engine (control thread + reader thread + a real output
/// callback thread via `spawn_driver`), not `ManualEngine`'s inline/deterministic path. Turning
/// loop off while genuinely playing across real threads must still stop the pass promptly at the
/// old loop end.
#[test]
fn h81_threaded_loop_off_stops_promptly() {
    use std::time::Duration;
    use vox_engine::{Engine, EngineHandle};

    assert!(alloc_checks_active());
    let fake = FakeBackend::new(11);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev_256().record_output()),
    );
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let dir = TempDir::new("threaded-loop");
    let src = noise(9, 96_000);
    let (s, e) = (12_000u64, 24_000u64);
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(256 << 20)).unwrap();
    let mut w = ChunkWriter::new(store.clone());
    w.append(&src).unwrap();
    let audio = w.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let engine = Engine::start(cfg).unwrap();
    let h: EngineHandle = engine.handle();
    let _driver = fake.spawn_driver(Duration::from_millis(1));

    h.set_document(Some(PlaybackDoc {
        store: store.clone(),
        snapshot: snapshot.clone(),
    }));

    let mut opened = false;
    for _ in 0..500 {
        if h.transport_state().can_play {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(opened, "the output stream opened");

    h.set_selection(Some((s, e)));
    let st = h.set_loop(true);
    assert!(st.loop_enabled);
    assert_eq!(st.loop_range, Some((s, e)));
    assert!(h.transport(TransportCommand::PlayFromStart).playing);

    // Let it loop several real passes (250 us/sample at 48 kHz means the 12_000-sample loop is
    // 250 ms; give it a full second: ~4 passes).
    std::thread::sleep(Duration::from_millis(1_000));
    assert!(
        h.transport_state().playing,
        "still looping before the toggle"
    );

    let st = h.set_loop(false);
    assert!(!st.loop_enabled);
    assert_eq!(st.loop_range, None, "loop_range clears immediately");

    // Give it generous real time to finish the pass and stop.
    let mut stopped = false;
    let mut last = h.transport_state();
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(10));
        last = h.transport_state();
        if !last.playing {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "still playing 2s after loop off: {last:?}");
    assert_eq!(last.playhead_samples, e, "stopped at the old loop end");
}

/// H-81 (stress regression): unaligned loop boundaries, a non-empty rack (latency in the picture,
/// so `flush_end`/pending-end interacts with `end_at_wrap`), random callback sizes, and a sweep of
/// toggle-off timings relative to the seam.
#[test]
fn h81_stress_unaligned_loop_off_with_rack_latency() {
    let src = noise(10, 3 * 48_000);
    let (s, e) = (13_037usize, 29_999usize);
    let l = e - s;
    for before_seam_ms in [1usize, 3, 7, 13, 29, 47, 90, 150, 210] {
        let mut r = rig(dev_random(), &src, 48_000, delay_rack(), 337);
        r.loop_play(s as u64, e as u64);
        r.run_ms(50);
        let base = pass_base(&r.recorded(), &src, s);
        let toggle_at = base + 2 * l - before_seam_ms.min(2 * l / 48) * 48;
        let mut spins = 0;
        while r.recorded().len() < toggle_at && spins < 5_000 {
            r.run_ms(1);
            spins += 1;
        }
        let st = r.eng.set_loop(false);
        assert!(!st.loop_enabled);
        assert_eq!(
            st.loop_range, None,
            "{before_seam_ms}ms: loop_range clears immediately"
        );
        r.run_ms(3_000);
        let st = r.eng.transport_state();
        assert!(
            !st.playing,
            "{before_seam_ms}ms: must have stopped, got playing={}",
            st.playing
        );
        assert_eq!(
            st.playhead_samples, e as u64,
            "{before_seam_ms}ms: stopped at the old loop end"
        );
        assert_eq!(
            r.fake.rt_violations(),
            0,
            "{before_seam_ms}ms: rt violation"
        );
    }
}
