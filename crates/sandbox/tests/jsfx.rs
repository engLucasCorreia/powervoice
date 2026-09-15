//! T-808: the JSFX backend through real `powervoice-sandbox` processes and the in-repo test
//! scripts (`tests/jsfx/`, run by the vendored ysfx): bit-exact with the in-process Gain (the
//! scripts replicate it) after the reported latency (realtime and offline), sample-accurate slider
//! automation by splitting blocks, the mono shim for a stereo-only script, the state (sliders +
//! `@serialize` bytes) wrapped with the plugin identity, the schema from the header, a
//! `pdc_delay` change asking for a restart, values set while inactive applied before activation,
//! another sample rate, and a script that hangs — while loading or while processing — never
//! taking the editor down.
//!
//! Every test passes with a note where this build doesn't host JSFX.

vox_module_api::install_test_allocator!();

mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, ModuleFactory, ModuleState, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessMode, ProcessStatus, Taper, Transport,
};
use vox_modules::Gain;
use vox_plugin_host::state::unwrap;

/// Slider 1: every test script's gain (dB).
const GAIN: ParamId = ParamId(0);
/// Slider 2: `gain.jsfx`'s mode, `latency.jsfx`'s latency, `serialize.jsfx`'s restore count.
const SLIDER2: ParamId = ParamId(1);
/// `latency.jsfx`'s default `pdc_delay`.
const SCRIPT_LATENCY: usize = 64;

fn config_at(rate: f64, mode: ProcessMode, max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: rate,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    }
}

fn config(mode: ProcessMode, max_block: u32) -> ActivateConfig {
    config_at(RATE, mode, max_block)
}

struct Run {
    out: Vec<f32>,
    restart: bool,
}

/// Runs `m` over `input` in blocks cycling through `blocks`, delivering `(position, param,
/// value)` events at their absolute positions; every `process` under the allocation checker.
fn run(m: &mut dyn Module, input: &[f32], blocks: &[usize], events: &[(usize, u32, f64)]) -> Run {
    let mut out = vec![0.0f32; input.len()];
    let mut out_events = OutputEvents::with_capacity(64);
    let mut evs: Vec<ParamEvent> = Vec::with_capacity(16);
    let (mut pos, mut k, mut steady) = (0usize, 0usize, 0u64);
    let mut restart = false;
    while pos < input.len() {
        let len = blocks[k % blocks.len()].min(input.len() - pos);
        k += 1;
        evs.clear();
        for &(at, id, v) in events {
            if at >= pos && at < pos + len {
                evs.push(ParamEvent {
                    offset: (at - pos) as u32,
                    id: ParamId(id),
                    value: v,
                });
            }
        }
        out_events.clear();
        let (i, o) = (&input[pos..pos + len], &mut out[pos..pos + len]);
        let (status, requested) = no_alloc(|| {
            let mut ctx = ProcessContext::new(
                len as u32,
                steady,
                Transport::default(),
                &evs,
                &mut out_events,
            );
            let mut outs = [o];
            let s = m.process(&mut ctx, &[i], &mut outs);
            (s, ctx.requested(HostRequest::Restart))
        })
        .expect("process allocated");
        assert_eq!(status, ProcessStatus::Continue);
        restart |= requested;
        pos += len;
        steady += len as u64;
    }
    Run { out, restart }
}

/// The in-process Gain at `gain_db` with gain events (dB) at absolute positions.
fn reference(input: &[f32], gain_db: f64, events: &[(usize, f64)]) -> Vec<f32> {
    let mut g = Gain::new();
    g.load_state(&Gain::state_with_gain_db(gain_db)).unwrap();
    g.activate(&config(ProcessMode::Realtime, 512)).unwrap();
    let mut y = vec![0.0f32; input.len()];
    let mut oe = OutputEvents::with_capacity(4);
    for (k, (i, o)) in input.chunks(512).zip(y.chunks_mut(512)).enumerate() {
        let evs: Vec<ParamEvent> = events
            .iter()
            .filter(|(at, _)| at / 512 == k)
            .map(|&(at, v)| ParamEvent {
                offset: (at % 512) as u32,
                id: Gain::GAIN_DB,
                value: v,
            })
            .collect();
        let mut ctx = ProcessContext::new(i.len() as u32, 0, Transport::default(), &evs, &mut oe);
        let mut outs = [o];
        g.process(&mut ctx, &[i], &mut outs);
    }
    y
}

/// [`assert_bits`], except for the sign of zero: EEL2 flushes zeros and denormals when it stores
/// a variable, so a script writes `-0.0` as `+0.0` (the Gain at −∞ dB times a negative sample).
fn assert_script_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    if let Some(i) = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits() && !(*a == 0.0 && *b == 0.0))
    {
        panic!("{what}: sample {i}: got {} want {}", got[i], want[i]);
    }
}

/// An activated instance of test script `script` at `gain_db`.
fn plugin(script: &str, gain_db: f64, mode: ProcessMode, max_block: u32) -> Box<dyn Module> {
    let mut m = jsfx_factory(&jsfx_script(script), exact_options())
        .create()
        .unwrap();
    m.load_state(&jsfx_gain_state(gain_db)).unwrap();
    m.activate(&config(mode, max_block)).unwrap();
    m
}

#[test]
fn jsfx_gain_is_bit_exact_with_the_in_process_gain_after_its_reported_latency() {
    if !jsfx_available() {
        return;
    }
    let x = noise(RATE as usize, 3);
    let want = reference(&x, -6.0, &[]);
    for (script, own) in [("gain.jsfx", 0usize), ("latency.jsfx", SCRIPT_LATENCY)] {
        for (mode, max_block, blocks, block) in [
            (
                ProcessMode::Realtime,
                1024,
                vec![256usize, 128, 200],
                256usize,
            ),
            (ProcessMode::Offline, 4096, vec![4096], 4096),
        ] {
            let mut m = plugin(script, -6.0, mode, max_block);
            let latency = m.latency_samples() as usize;
            assert_eq!(latency, block + own, "{script} {mode:?}");
            let r = run(&mut *m, &x, &blocks, &[]);
            assert!(!r.restart);
            assert!(
                r.out[..latency].iter().all(|v| *v == 0.0),
                "{script} {mode:?}"
            );
            assert_bits(
                &r.out[latency..],
                &want[..x.len() - latency],
                &format!("{script} {mode:?} vs in-process Gain"),
            );
            m.deactivate();
        }
    }
}

#[test]
fn automation_reaches_the_script_sample_accurately() {
    if !jsfx_available() {
        return;
    }
    let x = noise(RATE as usize, 4);
    let gain_events = [
        (1_000usize, -12.0),
        (1_003, -3.0),
        (9_000, 6.0),
        (9_500, -20.0),
        (17_777, -60.0),
        (30_001, 0.0),
    ];
    let events: Vec<(usize, u32, f64)> = gain_events
        .iter()
        .map(|&(at, db)| (at, GAIN.0, db))
        .collect();
    let want = reference(&x, 0.0, &gain_events);
    for script in ["gain.jsfx", "stereo.jsfx", "latency.jsfx"] {
        let mut m = plugin(script, 0.0, ProcessMode::Realtime, 1024);
        let latency = m.latency_samples() as usize;
        let r = run(&mut *m, &x, &[256, 100, 37, 256], &events);
        // The −60 dB event (gain 0, as the Gain's −∞) makes zeros: compared up to their sign.
        assert_script_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{script}: automated gain"),
        );
        assert_eq!(m.param_value(GAIN), Some(0.0));
    }
}

#[test]
fn a_stereo_only_script_runs_through_the_mono_shim() {
    if !jsfx_available() {
        return;
    }
    let x = noise(RATE as usize / 2, 5);
    let mut m = plugin("stereo.jsfx", -4.5, ProcessMode::Realtime, 1024);
    let latency = m.latency_samples() as usize;
    let r = run(&mut *m, &x, &[256], &[]);
    // Upmix (x, x), the script's per-channel gain, downmix (L + R) / 2: exactly the mono gain.
    assert_bits(
        &r.out[latency..],
        &reference(&x, -4.5, &[])[..x.len() - latency],
        "stereo script through the shim",
    );
    assert_eq!(m.params().len(), 1);
    assert_eq!(m.param_value(GAIN), Some(-4.5));
}

/// The PVJS state's sliders and `@serialize` bytes.
fn parse(data: &[u8]) -> (Vec<(u32, f64)>, Vec<u8>) {
    assert_eq!(&data[..4], b"PVJS");
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    assert_eq!(u32_at(4), 1);
    let mut at = 12;
    let mut sliders = Vec::new();
    for _ in 0..u32_at(8) {
        let v = f64::from_le_bytes(data[at + 4..at + 12].try_into().unwrap());
        sliders.push((u32_at(at), v));
        at += 12;
    }
    let n = u32_at(at) as usize;
    assert_eq!(at + 4 + n, data.len());
    (sliders, data[at + 4..].to_vec())
}

fn f32s(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn state_is_the_sliders_and_the_serialize_bytes_wrapped_with_its_identity() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("serialize.jsfx"), exact_options());
    let mut a = f.create().unwrap();
    a.load_state(&jsfx_gain_state(-7.5)).unwrap();
    a.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    // Saved while active: `@serialize` runs between two chunks.
    let st = a.save_state().unwrap();
    assert_eq!(st.params["slider1"].to_bits(), (-7.5f64).to_bits());
    let (h, data) = unwrap(st.blob.as_deref().expect("a plugin state")).unwrap();
    assert_eq!(
        (h.format.as_str(), h.id.as_str(), h.version.as_str()),
        ("jsfx", "jsfx:serialize.jsfx", "1.2.3")
    );
    let (sliders, bytes) = parse(data);
    assert_eq!(sliders, vec![(0, -7.5), (1, 0.0)]);
    assert_eq!(bytes, f32s(&[1234.5, 0.0]), "the script's marker and count");
    a.deactivate();

    // The blob alone restores the sliders and goes through the script's own `@serialize` (its
    // restore count shows on slider 2).
    let mut b = f.create().unwrap();
    b.load_state(&ModuleState {
        format_version: 1,
        params: Default::default(),
        blob: st.blob.clone(),
    })
    .unwrap();
    assert_eq!(b.param_value(GAIN), Some(-7.5));
    assert_eq!(b.param_value(SLIDER2), Some(1.0));
    let again = b.save_state().unwrap();
    let (_, bytes) = parse(unwrap(again.blob.as_deref().unwrap()).unwrap().1);
    assert_eq!(bytes, f32s(&[1234.5, 1.0]));

    // A script without `@serialize`: its sliders only.
    let mut gain = jsfx_factory(&jsfx_script("gain.jsfx"), exact_options())
        .create()
        .unwrap();
    gain.load_state(&jsfx_gain_state(-3.0)).unwrap();
    let gst = gain.save_state().unwrap();
    let (sliders, bytes) = parse(unwrap(gst.blob.as_deref().unwrap()).unwrap().1);
    assert_eq!(sliders, vec![(0, -3.0), (1, 0.0), (2, 1000.0), (3, 0.0)]);
    assert!(bytes.is_empty());

    // Another script's state is refused.
    let mut c = jsfx_factory(&jsfx_script("latency.jsfx"), exact_options())
        .create()
        .unwrap();
    assert!(c.load_state(&st).is_err());
}

#[test]
fn the_schema_comes_from_the_script_header() {
    if !jsfx_available() {
        return;
    }
    let m = jsfx_factory(&jsfx_script("gain.jsfx"), exact_options())
        .create()
        .unwrap();
    assert_eq!(m.descriptor().id, "jsfx:gain.jsfx");
    let p = m.params();
    assert_eq!(p.len(), 4);
    let keys: Vec<&str> = p.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["slider1", "slider2", "slider3", "slider4"]);
    assert_eq!(p[0].name.text, "Gain (dB)");
    assert_eq!(
        (p[0].min, p[0].max, p[0].default, p[0].decimals, p[0].step),
        (-60.0, 24.0, 0.0, 1, None)
    );
    assert_eq!(p[0].flags, ParamFlags::AUTOMATABLE);
    let labels: Vec<&str> = p[1].enum_labels.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(labels, ["Clean", "Warm", "Hot"]);
    assert_eq!(
        (p[2].taper, p[2].min, p[2].max, p[2].default),
        (Taper::Log, 20.0, 20_000.0, 1_000.0)
    );
    assert!(p[2].flags.contains(ParamFlags::STEPPED));
    assert!(
        p[3].flags
            .contains(ParamFlags::BOOL | ParamFlags::HIDDEN | ParamFlags::AUTOMATABLE),
        "{:?}",
        p[3].flags
    );
    assert_eq!(p[3].name.text, "Hidden toggle");
}

/// Runs blocks until a restart is requested or `timeout`.
fn run_until_restart(m: &mut dyn Module, timeout: Duration) -> bool {
    let x = noise(256, 9);
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if run(m, &x, &[256], &[]).restart {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn a_pdc_delay_change_asks_for_a_restart() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("latency.jsfx"), exact_options());
    let mut m = f.create().unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples() as usize, 256 + SCRIPT_LATENCY);
    let x = noise(9_600, 6);
    let r = run(&mut *m, &x, &[256], &[(1_000, SLIDER2.0, 300.0)]);
    assert!(
        r.restart || run_until_restart(&mut *m, Duration::from_secs(5)),
        "no restart request after the latency change"
    );
    assert_eq!(m.param_value(SLIDER2), Some(300.0));
    // What the rack's replacement does: a new instance from the current state.
    let st = m.save_state().unwrap();
    m.deactivate();
    let mut next = f.create().unwrap();
    next.load_state(&st).unwrap();
    next.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(next.latency_samples(), 256 + 300);
    assert!(!run_until_restart(&mut *next, Duration::from_millis(200)));

    // Offline: the request travels with the chunk whose `@slider` changed `pdc_delay` (H-36), so
    // three blocks are enough, every time.
    let mut off = f.create().unwrap();
    off.activate(&config(ProcessMode::Offline, 4096)).unwrap();
    let three = noise(4096 * 3, 13);
    let r = run(&mut *off, &three, &[4096], &[(100, SLIDER2.0, 128.0)]);
    assert!(r.restart, "offline latency change");
}

/// The rack's committed state wins over the blob captured at load: a value set while inactive is
/// on the slider before `@init`, so the reported latency is already the new one.
#[test]
fn values_set_while_inactive_apply_before_activation() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("latency.jsfx"), exact_options());
    let mut state = f.create().unwrap().save_state().unwrap();
    let mut m = f.create().unwrap();
    state.params.insert("slider2".into(), 300.0);
    m.load_state(&state).unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + 300);
    assert!(!run_until_restart(&mut *m, Duration::from_millis(200)));
    // Deactivating and activating again runs `@init` again, with the same result.
    m.deactivate();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + 300);
}

#[test]
fn another_sample_rate_reinitialises_the_script() {
    if !jsfx_available() {
        return;
    }
    let mut m = jsfx_factory(&jsfx_script("gain.jsfx"), exact_options())
        .create()
        .unwrap();
    m.load_state(&jsfx_gain_state(-2.5)).unwrap();
    m.activate(&config_at(44_100.0, ProcessMode::Realtime, 1024))
        .unwrap();
    let latency = m.latency_samples() as usize;
    let x = noise(44_100, 12);
    let r = run(&mut *m, &x, &[256], &[]);
    assert_bits(
        &r.out[latency..],
        &reference(&x, -2.5, &[])[..x.len() - latency],
        "44.1 kHz",
    );
}

/// A script that stops answering in `@sample` is detected like any hung plugin: the slot is
/// bypassed (`Error`) and its sandbox killed; the editor carries on.
#[test]
fn a_script_that_hangs_while_processing_is_detected_and_killed() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("hang_processing.jsfx"), default_options());
    let mut p = f.create().unwrap();
    p.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    let pid = f.live_instances()[0].pid;
    let x = sine(RATE as usize * 3, 220.0, 0.5);
    let period = Duration::from_secs_f64(256.0 / RATE);
    let mut oe = OutputEvents::with_capacity(8);
    let mut y = vec![0.0f32; 256];
    let start = Instant::now();
    let mut failed = false;
    for (k, i) in x.chunks(256).enumerate() {
        let status = no_alloc(|| {
            let mut ctx = ProcessContext::new(256, 0, Transport::default(), &[], &mut oe);
            let mut outs = [&mut y[..]];
            p.process(&mut ctx, &[i], &mut outs)
        })
        .expect("the proxy allocated");
        if status == ProcessStatus::Error {
            failed = true;
            break;
        }
        if let Some(d) = (start + period * (k as u32 + 1)).checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    assert!(failed, "the hang was never reported");
    assert!(f.live_instances()[0].fault.is_some());
    assert!(
        wait_until(Duration::from_secs(3), || !pid_exists(pid)),
        "the hung sandbox wasn't killed"
    );
}

/// A script whose `@init` never returns fails to load within the request timeout.
#[test]
fn a_script_that_hangs_while_loading_fails_cleanly() {
    if !jsfx_available() {
        return;
    }
    let mut options = exact_options();
    options.request_timeout = Duration::from_millis(1500);
    let f = jsfx_factory(&jsfx_script("hang.jsfx"), options);
    let t0 = Instant::now();
    assert!(f.create().is_err());
    assert!(t0.elapsed() < Duration::from_secs(6), "{:?}", t0.elapsed());
    assert!(f.live_instances().is_empty());
}
