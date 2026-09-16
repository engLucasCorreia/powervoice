//! T-107 monitoring end-to-end through the fake backend (SPEC-002 §2.7, §4.4, AC-9…AC-11;
//! ADR-002 §6): Off / Dry / Through rack levels and click-free switches, takes unaffected by the
//! mode, the latency readout against the measured capture → playback delay, the drift servo over
//! 10 simulated minutes at ±200 ppm and 48 kHz → 44.1 kHz, and clean stops on device loss. Every
//! input and output callback runs under `test_util::no_alloc`.

vox_module_api::install_test_allocator!();

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vox_engine::backend::StreamId;
use vox_engine::backend::fake::{CallbackSizes, FakeBackend, FakeDevice, FakeDirection};
use vox_engine::record::{MonitorMode, RecordState, RecordingResult};
use vox_engine::{
    DevicePrefs, Direction, EngineConfig, HostId, ManualEngine, RackCommand, TelemetryFrame,
};
use vox_module_api::test_util::{
    TestDelay, TestRestart, ZipperWindow, alloc_checks_active, analyze_zipper, no_alloc,
};
use vox_module_api::{Module, ModuleDescriptor, ModuleError, ModuleFactory};
use vox_modules::Gain;
use vox_project::{
    DocSnapshot, FixedFreeSpace, Session, SessionConfig, StoreOptions, TakeMode, TakeWriterOptions,
};
use vox_rack::Registry;

const MS: u64 = 1_000_000;
const RATE: u32 = 48_000;
/// −20 dBFS peak.
const TONE_AMP: f64 = 0.1;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-engine-mon-{tag}-{}-{}",
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

/// `TestDelay(480)`: a module reporting exactly 10 ms of latency at 48 kHz (AC-10).
struct DelayFactory(ModuleDescriptor);

impl ModuleFactory for DelayFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestDelay::new(480)))
    }
}

/// `TestRestart`: a delay whose latency is its `latency_samples` parameter; a change restarts the
/// instance with the new latency (T-401).
struct RestartFactory(ModuleDescriptor);

impl ModuleFactory for RestartFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.0
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestRestart::new(0)))
    }
}

fn registry() -> Registry {
    let delay: Arc<dyn ModuleFactory> =
        Arc::new(DelayFactory(TestDelay::new(480).descriptor().clone()));
    let restart: Arc<dyn ModuleFactory> =
        Arc::new(RestartFactory(TestRestart::new(0).descriptor().clone()));
    Registry::with_factories(
        vox_modules::builtin_factories()
            .into_iter()
            .chain([delay, restart]),
    )
    .unwrap()
}

/// A mono mic at `rate` (true rate skewed by `skew_ppm`), fixed `frames` per callback, 5 ms
/// device latency.
fn mic(
    rate: u32,
    skew_ppm: f64,
    frames: u32,
    source: impl FnMut(u64, usize, u32) -> f32 + Send + 'static,
) -> FakeDevice {
    FakeDevice::new("Mic").with_input(
        FakeDirection::new(1, &[rate], rate)
            .callback_sizes(CallbackSizes::Fixed(frames))
            .latency_ns(5 * MS)
            .skew_ppm(skew_ppm)
            .source(source),
    )
}

/// A stereo DAC at `rate` (skewed by `skew_ppm`), `frames` per callback, 7 ms latency, recorded.
fn dac(rate: u32, skew_ppm: f64, frames: u32) -> FakeDevice {
    FakeDevice::new("DAC").with_output(
        FakeDirection::new(2, &[rate], rate)
            .default_buffer(frames)
            .latency_ns(7 * MS)
            .skew_ppm(skew_ppm)
            .record_output(),
    )
}

/// A 997 Hz tone at −20 dBFS as a function of the input frame index (the device's own clock).
fn tone(k: u64, _ch: usize, rate: u32) -> f32 {
    (TONE_AMP * (std::f64::consts::TAU * 997.0 * k as f64 / f64::from(rate)).sin()) as f32
}

struct Rig {
    fake: FakeBackend,
    eng: ManualEngine,
    /// The recording volume; removed when the rig drops.
    _dir: TempDir,
}

fn rig(mic: FakeDevice, dac: FakeDevice) -> Rig {
    assert!(
        alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let fake = FakeBackend::new(7);
    fake.plug(HostId::Alsa, dac);
    fake.plug(HostId::Alsa, mic);
    fake.set_rt_guard(|f| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), Arc::new(registry()));
    cfg.prefs = DevicePrefs {
        input_device: Some("Mic".to_owned()),
        input_channel: 1,
        ..DevicePrefs::default()
    };
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let dir = TempDir::new("rig");
    cfg.disk_space = Arc::new(FixedFreeSpace::new(100 << 30));
    cfg.record_volume = dir.0.clone();
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    Rig {
        fake,
        eng,
        _dir: dir,
    }
}

impl Rig {
    /// Advances `ms` of simulated time, ticking the control thread every `step_ms`.
    fn run_ms_step(&mut self, ms: u64, step_ms: u64) {
        let mut left = ms;
        while left > 0 {
            let s = left.min(step_ms);
            self.fake.advance_by(s * MS);
            self.eng.tick();
            left -= s;
        }
    }

    fn run_ms(&mut self, ms: u64) {
        self.run_ms_step(ms, 4);
    }

    fn stream(&self, dir: Direction) -> StreamId {
        self.fake
            .streams()
            .iter()
            .rev()
            .find(|s| s.info.direction == dir && s.alive)
            .map(|s| s.info.id)
            .expect("a live stream")
    }

    fn output(&self) -> Vec<f32> {
        let id = self.stream(Direction::Output);
        self.fake.recorded_output(id).unwrap().samples
    }

    fn state(&self) -> RecordState {
        self.eng.record_state()
    }

    fn set_mode(&mut self, mode: MonitorMode) -> RecordState {
        self.eng.set_monitor_mode(mode)
    }

    fn arm(&mut self) {
        let st = self.eng.set_armed(true);
        assert!(st.armed && st.input_open, "{st:?}");
    }

    fn add_module(&mut self, id: &str) {
        let n = self.eng.rack_snapshot().slots.len();
        self.eng
            .rack_command(RackCommand::Add {
                module_id: id.into(),
                index: n,
            })
            .unwrap();
    }

    /// Installs the 60 Hz telemetry sink (H-51), collecting every frame.
    fn telemetry_sink(&mut self) -> Arc<Mutex<Vec<TelemetryFrame>>> {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        self.eng
            .set_telemetry_sink(Some(Box::new(move |f: &TelemetryFrame| {
                sink.lock().unwrap().push(*f)
            })));
        frames
    }
}

/// RMS in dB of `x`.
fn rms_db(x: &[f32]) -> f64 {
    let e = x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64;
    10.0 * e.log10()
}

fn fnv1a(x: &[f32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in x {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

// --- AC-9 --------------------------------------------------------------------------------------

/// AC-9: same-clock 48 kHz devices, a −6 dB Gain in the rack, a 997 Hz −20 dBFS input, armed with
/// playback stopped: Off is digital silence, Dry is −23.01 ± 0.05 dB RMS (the rack is not
/// applied), Through rack −29.01 ± 0.05 dB; every switch passes SPEC-012 §4.3 with T_s = 10 ms;
/// no allocation in either callback.
#[test]
fn ac9_modes_set_the_level_and_switch_without_clicks() {
    let mut r = rig(mic(RATE, 0.0, 256, tone), dac(RATE, 0.0, 256));
    r.run_ms(20);
    r.add_module(Gain::ID);
    r.eng
        .rack_command(RackCommand::SetParamText {
            index: 0,
            id: Gain::GAIN_DB,
            text: "-6.0".into(),
        })
        .unwrap();
    r.arm();
    r.run_ms(1_000);
    let off = r.output();
    assert!(
        off[off.len() - 24_000..].iter().all(|&x| x == 0.0),
        "Off: digital silence"
    );

    let mut switches = Vec::new();
    for (mode, want_db) in [
        (MonitorMode::Dry, -23.01),
        (MonitorMode::ThroughRack, -29.01),
        (MonitorMode::Dry, -23.01),
        (MonitorMode::Off, f64::NEG_INFINITY),
    ] {
        switches.push(r.output().len());
        let st = r.set_mode(mode);
        assert_eq!(st.monitoring, mode != MonitorMode::Off, "{st:?}");
        r.run_ms(1_000);
        let out = r.output();
        let level = rms_db(&out[out.len() - 24_000..]);
        if want_db.is_finite() {
            assert!((level - want_db).abs() < 0.05, "{mode:?}: {level} dB");
        } else {
            assert!(
                out[out.len() - 24_000..].iter().all(|&x| x == 0.0),
                "Off again: digital silence"
            );
        }
    }
    let out = r.output();
    for &at in &switches {
        // The command lands at the next callback (≤ one 256-frame period after the call).
        let report = analyze_zipper(
            &out,
            0,
            f64::from(RATE),
            ZipperWindow {
                start: at,
                end: at + 256,
                t_s_ms: 10.0,
            },
        )
        .unwrap();
        assert!(report.pass, "switch at {at}: {report}");
    }
    let st = r.state();
    assert_eq!(
        (st.monitor_underruns, st.monitor_overruns),
        (0, 0),
        "{st:?}"
    );
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

// --- H-51 item 6: the output meter reads what the user hears, including Dry monitoring --------

/// H-51 (SPEC-007 §2.9, ADR-002 §4): Dry-monitoring the same −20 dBFS tone as AC-9, with no rack
/// and playback stopped, must show up in the *output telemetry meter*
/// (`out_peak_dbfs`/`out_rms_dbfs`), at the same −23.01 dB RMS AC-9 measures on the device output
/// itself. Before the fix the meter was computed pre-Dry (post-rack only), so it kept reading
/// silence while Dry monitoring was clearly audible on the device — a divergence from the
/// analyzer tap, which (per SPEC-007 §2.9) already includes Dry monitoring.
#[test]
fn h51_output_meter_matches_the_device_during_dry_monitoring() {
    let mut r = rig(mic(RATE, 0.0, 256, tone), dac(RATE, 0.0, 256));
    let frames = r.telemetry_sink();
    r.run_ms(20);
    r.arm();
    let st = r.set_mode(MonitorMode::Dry);
    assert!(st.monitoring, "{st:?}");
    r.run_ms(1_000);

    let out = r.output();
    let device_db = rms_db(&out[out.len() - 24_000..]);
    assert!(
        (device_db - (-23.01)).abs() < 0.05,
        "sanity check against AC-9: {device_db} dB"
    );

    let last = frames
        .lock()
        .unwrap()
        .last()
        .copied()
        .expect("at least one telemetry frame while monitoring");
    assert!(
        (f64::from(last.out_rms_dbfs) - device_db).abs() < 0.5,
        "the meter should read what the device hears: meter {} dB vs device {device_db} dB",
        last.out_rms_dbfs
    );
    assert!(
        last.out_peak_dbfs > -30.0 && last.out_peak_dbfs.is_finite(),
        "the meter peak should reflect the dry-monitored tone, got {}",
        last.out_peak_dbfs
    );

    // Off: the meter must fall back to silence along with the device.
    r.set_mode(MonitorMode::Off);
    r.run_ms(1_000);
    let last_off = frames.lock().unwrap().last().copied().unwrap();
    assert!(
        last_off.out_peak_dbfs <= -120.0 && last_off.out_rms_dbfs <= -120.0,
        "{last_off:?}"
    );
}

/// Records one 1 s take with `mode` monitoring (same deterministic rig every time) and returns
/// its FNV-1a hash.
fn take_hash(mode: MonitorMode) -> u64 {
    let mut r = rig(mic(RATE, 0.0, 256, tone), dac(RATE, 0.0, 256));
    r.run_ms(20);
    r.add_module(Gain::ID);
    r.set_mode(mode);
    r.arm();
    r.run_ms(300);
    let dir = TempDir::new("take");
    let mut session = Session::create(
        &dir.0,
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: StoreOptions::with_memory_budget(64 << 20),
        },
    )
    .unwrap();
    let capture = session
        .begin_take(TakeMode::New, TakeWriterOptions::default())
        .unwrap();
    let done: Arc<Mutex<Option<RecordingResult>>> = Arc::new(Mutex::new(None));
    let d = done.clone();
    r.eng
        .record_start(
            RATE,
            capture,
            Box::new(move |res| *d.lock().unwrap() = Some(res)),
        )
        .expect("record_start");
    r.run_ms(1_000);
    r.eng.record_stop();
    let res = (0..2_000)
        .find_map(|_| {
            r.run_ms(1);
            done.lock().unwrap().take()
        })
        .expect("the take finished");
    let step = session
        .commit_take(&res.finished, &[])
        .unwrap()
        .expect("one undoable edit");
    let snap: &DocSnapshot = &step.snapshot;
    let mut take = vec![0.0f32; snap.len_samples as usize];
    session.store().read(snap, 0, &mut take).unwrap();
    assert!(take.len() > 40_000);
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
    drop(session);
    fnv1a(&take)
}

/// AC-9: monitoring is never recorded — takes from the same source in the three modes are
/// bit-identical.
#[test]
fn ac9_takes_are_identical_in_every_mode() {
    let off = take_hash(MonitorMode::Off);
    assert_eq!(take_hash(MonitorMode::Dry), off);
    assert_eq!(take_hash(MonitorMode::ThroughRack), off);
}

// --- AC-10 -------------------------------------------------------------------------------------

/// Input frames carrying a single 0.5 impulse (AC-10), 3 s apart from 10.5 s on.
const IMPULSES: [u64; 4] = [504_000, 648_000, 792_000, 936_000];

/// The measured delay (ms) from the capture of input frame `k` to the playback of its impulse:
/// the output frame of the largest |sample| after it, refined by parabolic interpolation.
fn measured_delay_ms(r: &Rig, k: u64) -> f64 {
    let (in_id, out_id) = (r.stream(Direction::Input), r.stream(Direction::Output));
    let t_cap = r.fake.frame_time_ns(in_id, k).unwrap();
    let out = r.output();
    let t0 = r.fake.frame_time_ns(out_id, 0).unwrap();
    // Search 0…100 ms after the capture.
    let from = ((t_cap - t0) as f64 * f64::from(RATE) / 1e9) as usize;
    let to = (from + 4_800).min(out.len() - 1);
    let j = (from..to)
        .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
        .unwrap();
    assert!(out[j].abs() > 0.2, "impulse heard (peak {} at {j})", out[j]);
    let (a, b, c) = (
        f64::from(out[j - 1].abs()),
        f64::from(out[j].abs()),
        f64::from(out[j + 1].abs()),
    );
    let frac = 0.5 * (a - c) / (a - 2.0 * b + c);
    let t_play =
        r.fake.frame_time_ns(out_id, j as u64).unwrap() as f64 + frac * 1e9 / f64::from(RATE);
    (t_play - t_cap as f64) / 1e6
}

/// AC-10: 256-frame periods at 48 kHz, 5 ms input and 7 ms output latency: after 10 s the
/// measured capture → playback delay of an impulse equals the readout within ±1 ms (≈ 23.7 ms,
/// SPEC-002 §4.4's example); a module reporting 480 samples raises both by 10.0 ± 0.5 ms through
/// the rack, and neither in Dry mode.
#[test]
fn ac10_latency_readout_matches_the_measured_delay() {
    let source = |k: u64, _ch: usize, _rate: u32| if IMPULSES.contains(&k) { 0.5 } else { 0.0 };
    let mut r = rig(mic(RATE, 0.0, 256, source), dac(RATE, 0.0, 256));
    r.run_ms(20);
    r.set_mode(MonitorMode::Dry);
    r.arm();
    let t_arm = r.fake.now_ns();
    let readout = |r: &Rig| f64::from(r.state().monitor_latency_us.expect("a readout")) / 1000.0;
    let until = |r: &mut Rig, s: f64| {
        let t = t_arm + (s * 1e9) as u64;
        let ms = (t - r.fake.now_ns()) / MS;
        r.run_ms(ms);
    };

    // Dry, empty rack.
    until(&mut r, 12.0);
    let (r0, d0) = (readout(&r), measured_delay_ms(&r, IMPULSES[0]));
    assert!(
        (r0 - 23.7).abs() < 0.051,
        "readout {r0} ms (SPEC-002 §4.4 example)"
    );
    assert!(
        (d0 - r0).abs() <= 1.0,
        "measured {d0} ms vs readout {r0} ms"
    );

    // Through an empty rack: the same.
    r.set_mode(MonitorMode::ThroughRack);
    until(&mut r, 15.0);
    let (r1, d1) = (readout(&r), measured_delay_ms(&r, IMPULSES[1]));
    assert!((r1 - r0).abs() < 0.051, "{r1} vs {r0}");
    assert!(
        (d1 - r1).abs() <= 1.0,
        "measured {d1} ms vs readout {r1} ms"
    );

    // A 480-sample module through the rack: +10 ms on both.
    r.add_module(TestDelay::ID);
    until(&mut r, 18.0);
    let (r2, d2) = (readout(&r), measured_delay_ms(&r, IMPULSES[2]));
    assert!((r2 - r0 - 10.0).abs() <= 0.5, "readout {r2} vs {r0}");
    assert!((d2 - d0 - 10.0).abs() <= 0.5, "measured {d2} vs {d0}");
    assert!(
        (d2 - r2).abs() <= 1.0,
        "measured {d2} ms vs readout {r2} ms"
    );

    // Dry with the module in the rack: neither changes.
    r.set_mode(MonitorMode::Dry);
    until(&mut r, 21.0);
    let (r3, d3) = (readout(&r), measured_delay_ms(&r, IMPULSES[3]));
    assert!((r3 - r0).abs() < 0.051, "{r3} vs {r0}");
    assert!((d3 - d0).abs() <= 0.5, "{d3} vs {d0}");

    let st = r.state();
    assert_eq!(
        (st.monitor_underruns, st.monitor_overruns),
        (0, 0),
        "{st:?}"
    );
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
}

// --- AC-11 -------------------------------------------------------------------------------------

/// Triangle period (input frames) and amplitude: the output sample value tells which input frame
/// it came from (polynomial interpolation is exact on the linear ramps).
const TRI_P: u64 = 96_000;
const TRI_A: f64 = 0.5;

/// T-401 (SPEC-012 AC-8, SPEC-002 §2.7): a rack latency change — a module restart with a new
/// latency, monitoring through the rack — reaches the monitoring readout within 100 ms of the
/// command, not at the next periodic recompute.
#[test]
fn t401_monitoring_readout_follows_a_rack_latency_change_within_100_ms() {
    let mut r = rig(mic(RATE, 0.0, 256, |_, _, _| 0.0), dac(RATE, 0.0, 256));
    r.run_ms(20);
    r.set_mode(MonitorMode::ThroughRack);
    r.arm();
    r.add_module(TestRestart::ID);
    let set = |r: &mut Rig, value: f64| {
        r.eng
            .rack_command(RackCommand::SetParamPlain {
                index: 0,
                id: TestRestart::LATENCY,
                value,
            })
            .unwrap();
    };
    set(&mut r, 480.0);
    r.run_ms(1_500);
    let readout = |r: &Rig| i64::from(r.state().monitor_latency_us.expect("a readout"));
    // 480 → 960 samples: +10 ms; 960 → 240: −15 ms.
    for (value, delta_us) in [(960.0, 10_000i64), (240.0, -15_000)] {
        let before = readout(&r);
        set(&mut r, value);
        let t0 = r.fake.now_ns();
        let mut at = None;
        for _ in 0..400 {
            r.run_ms_step(1, 1);
            if (readout(&r) - before - delta_us).abs() <= 100 {
                at = Some(r.fake.now_ns() - t0);
                break;
            }
        }
        let at = at.unwrap_or_else(|| {
            panic!(
                "{value}: readout {} µs never reached {before} {delta_us:+} µs",
                readout(&r)
            )
        });
        assert!(
            at <= 100 * MS,
            "{value}: the readout followed after {} ms",
            at / MS
        );
    }
    assert_eq!(r.fake.rt_violations(), 0);
}

fn triangle(k: u64) -> f64 {
    let phi = (k % TRI_P) as f64 / TRI_P as f64;
    if phi < 0.5 {
        TRI_A * (4.0 * phi - 1.0)
    } else {
        TRI_A * (3.0 - 4.0 * phi)
    }
}

struct DriftReport {
    /// Worst |delay − readout| after 10 s (ms).
    worst_delay_error_ms: f64,
    /// Worst sample step after 10 s relative to the triangle's slope (1.0 = the slope).
    worst_step: f64,
    /// Input frames per output frame over the last 60 s, relative to the true clock ratio − 1.
    ratio_error: f64,
    underruns_after_10s: u32,
    overruns_after_10s: u32,
}

/// Dry-monitors the triangle for `secs` from a `rate_in` mic skewed `skew_in` ppm to a
/// `rate_out` DAC skewed `skew_out` ppm (256-frame periods), analyzing the output in 1 s pieces.
fn drift_run(rate_in: u32, skew_in: f64, rate_out: u32, skew_out: f64, secs: u64) -> DriftReport {
    let source = |k: u64, _ch: usize, _rate: u32| triangle(k) as f32;
    let mut r = rig(
        mic(rate_in, skew_in, 256, source),
        dac(rate_out, skew_out, 256),
    );
    r.run_ms(20);
    r.set_mode(MonitorMode::Dry);
    r.arm();
    let (in_id, out_id) = (r.stream(Direction::Input), r.stream(Direction::Output));
    let t_in0 = r.fake.frame_time_ns(in_id, 0).unwrap() as f64;
    let t_out0 = r.fake.frame_time_ns(out_id, 0).unwrap() as f64;
    let true_in = f64::from(rate_in) * (1.0 + skew_in * 1e-6);
    let true_out = f64::from(rate_out) * (1.0 + skew_out * 1e-6);
    let slope = 4.0 * TRI_A / TRI_P as f64 * f64::from(rate_in) / f64::from(rate_out);

    let mut report = DriftReport {
        worst_delay_error_ms: 0.0,
        worst_step: 0.0,
        ratio_error: 0.0,
        underruns_after_10s: 0,
        overruns_after_10s: 0,
    };
    let mut counters_at_10s = (0, 0);
    let mut readout_ms = 0.0;
    let mut prev: Option<f32> = None;
    let mut first_pair: Option<(f64, f64)> = None;
    let mut last_pair = (0.0, 0.0);
    for s in 1..=secs {
        r.run_ms_step(1_000, 16);
        let rec = r.fake.take_recorded_output(out_id).unwrap();
        let Some(first) = rec.blocks.first() else {
            continue;
        };
        let j0 = first.first_frame;
        if s == 10 {
            let st = r.state();
            counters_at_10s = (st.monitor_underruns, st.monitor_overruns);
            readout_ms = f64::from(st.monitor_latency_us.expect("a readout")) / 1000.0;
        }
        if s <= 10 {
            prev = rec.samples.last().copied();
            continue;
        }
        for (i, &y) in rec.samples.iter().enumerate() {
            // Away from the triangle's corners (the 8-point interpolator rings a little around a
            // kink), every step equals the ramp's slope: a skipped or repeated input frame would
            // show as 2×, an underrun fade far more.
            let corner = |v: f32| f64::from(v.abs()) > TRI_A * 0.999;
            if let Some(p) = prev
                && !corner(p)
                && !corner(y)
            {
                report.worst_step = report.worst_step.max(f64::from((y - p).abs()) / slope);
            }
            prev = Some(y);
            if i % 97 != 0 {
                continue;
            }
            let j = (j0 + i as u64) as f64;
            let t_play = t_out0 + j * 1e9 / true_out;
            // Decode the input frame, choosing the candidate nearest the expected one.
            let u = (f64::from(y) / TRI_A + 1.0) / 4.0;
            if !(0.02..=0.48).contains(&u) {
                continue;
            }
            let guess = (t_play - readout_ms * 1e6 - t_in0) * true_in / 1e9;
            let base = (guess / TRI_P as f64).floor();
            let k = [-1.0, 0.0, 1.0]
                .iter()
                .flat_map(|d| {
                    [
                        (base + d + u) * TRI_P as f64,
                        (base + d + 1.0 - u) * TRI_P as f64,
                    ]
                })
                .min_by(|a, b| (a - guess).abs().total_cmp(&(b - guess).abs()))
                .unwrap();
            let t_cap = t_in0 + k * 1e9 / true_in;
            let delay_ms = (t_play - t_cap) / 1e6;
            report.worst_delay_error_ms = report
                .worst_delay_error_ms
                .max((delay_ms - readout_ms).abs());
            if s + 60 > secs {
                first_pair.get_or_insert((j, k));
                last_pair = (j, k);
            }
        }
    }
    let st = r.state();
    report.underruns_after_10s = st.monitor_underruns - counters_at_10s.0;
    report.overruns_after_10s = st.monitor_overruns - counters_at_10s.1;
    let (j1, k1) = first_pair.expect("samples in the last 60 s");
    let (j2, k2) = last_pair;
    let expected = true_in / true_out;
    report.ratio_error = ((k2 - k1) / (j2 - j1)) / expected - 1.0;
    assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
    report
}

fn assert_drift(name: &str, rep: &DriftReport) {
    assert_eq!(
        (rep.underruns_after_10s, rep.overruns_after_10s),
        (0, 0),
        "{name}: monitor underruns/overruns after 10 s"
    );
    assert!(
        rep.worst_delay_error_ms <= 1.0,
        "{name}: latency strayed {} ms from the readout",
        rep.worst_delay_error_ms
    );
    assert!(
        rep.worst_step <= 1.05,
        "{name}: a discontinuity of {}× the signal slope",
        rep.worst_step
    );
    assert!(
        rep.ratio_error.abs() <= 20e-6,
        "{name}: playback speed off by {} ppm",
        rep.ratio_error * 1e6
    );
}

/// AC-11: input +200 ppm against the output, 10 simulated minutes.
#[test]
fn ac11_drift_plus_200_ppm_for_10_minutes() {
    let rep = drift_run(RATE, 200.0, RATE, 0.0, 600);
    assert_drift("+200 ppm", &rep);
}

/// AC-11: input −200 ppm against the output, 10 simulated minutes (both clocks off true time).
#[test]
fn ac11_drift_minus_200_ppm_for_10_minutes() {
    let rep = drift_run(RATE, -100.0, RATE, 100.0, 600);
    assert_drift("−200 ppm", &rep);
}

/// AC-11: 48 kHz input → 44.1 kHz output, 10 simulated minutes.
#[test]
fn ac11_rate_mismatch_48k_to_44k1_for_10_minutes() {
    let rep = drift_run(RATE, 0.0, 44_100, 0.0, 600);
    assert_drift("48 k → 44.1 k", &rep);
}

/// AC-11: a physical 997 Hz tone captured by a +200 ppm mic (or a 48 kHz mic into a 44.1 kHz
/// DAC) measures 997 Hz ± 20 ppm against the output device's clock over the last 60 s.
#[test]
fn ac11_tone_keeps_997_hz_on_the_output_clock() {
    for (rate_in, skew_in, rate_out) in [(RATE, 200.0, RATE), (RATE, 0.0, 44_100)] {
        let true_in = f64::from(rate_in) * (1.0 + skew_in * 1e-6);
        // The tone exists in true time; the skewed device samples it at its true rate.
        let source = move |k: u64, _ch: usize, _rate: u32| {
            (TONE_AMP * (std::f64::consts::TAU * 997.0 * k as f64 / true_in).sin()) as f32
        };
        let mut r = rig(mic(rate_in, skew_in, 256, source), dac(rate_out, 0.0, 256));
        r.run_ms(20);
        r.set_mode(MonitorMode::Dry);
        r.arm();
        let out_id = r.stream(Direction::Output);
        r.run_ms_step(10_000, 16);
        let _ = r.fake.take_recorded_output(out_id);
        r.run_ms_step(60_000, 16);
        let y = r.fake.take_recorded_output(out_id).unwrap().samples;
        let mut first = None;
        let mut last = 0.0;
        let mut n = 0u64;
        for i in 1..y.len() {
            let (a, b) = (f64::from(y[i - 1]), f64::from(y[i]));
            if a < 0.0 && b >= 0.0 {
                let t = (i - 1) as f64 + a / (a - b);
                first.get_or_insert(t);
                last = t;
                n += 1;
            }
        }
        let f = (n - 1) as f64 * f64::from(rate_out) / (last - first.unwrap());
        assert!(
            (f / 997.0 - 1.0).abs() <= 20e-6,
            "{rate_in} Hz {skew_in:+} ppm → {rate_out} Hz: {f} Hz"
        );
        assert_eq!(r.state().monitor_underruns, 0);
        assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
    }
}

// --- Device loss -------------------------------------------------------------------------------

/// Monitoring stops cleanly when the input or the output is lost: no panic, no allocation,
/// silence afterwards, `monitoring` off.
#[test]
fn monitoring_stops_cleanly_on_device_loss() {
    for lose in [Direction::Input, Direction::Output] {
        let mut r = rig(mic(RATE, 0.0, 256, tone), dac(RATE, 0.0, 256));
        r.run_ms(20);
        r.set_mode(MonitorMode::ThroughRack);
        r.arm();
        r.run_ms(500);
        assert!(r.state().monitoring);
        let out_id = r.stream(Direction::Output);
        let id = r.stream(lose);
        r.fake.lose_stream(id);
        r.run_ms(1_000);
        let st = r.state();
        assert!(!st.monitoring, "{lose:?} lost: {st:?}");
        if lose == Direction::Input {
            let out = r.fake.recorded_output(out_id).unwrap().samples;
            assert!(
                out[out.len() - 24_000..].iter().all(|&x| x == 0.0),
                "silent after the input was lost"
            );
        }
        assert_eq!(r.fake.rt_violations(), 0, "a callback allocated");
    }
}
