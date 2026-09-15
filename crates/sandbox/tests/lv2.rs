//! T-807: the LV2 backend through real `powervoice-sandbox` processes, real lilv discovery and
//! the in-repo test plugin (`vox-test-lv2`, its `.ttl` files written into a temporary bundle next
//! to a copy of its `cdylib`): bit-exact with the in-process Gain after the reported latency
//! (realtime and offline), sample-accurate automation by splitting blocks at event offsets, the
//! mono shim for a stereo-only plugin, the state (control values by symbol + `state:interface`
//! properties) wrapped with the plugin identity, the schema from the plugin data (units,
//! enumerations, toggles, logarithmic ports, groups, read-only outputs), scale-point text, values
//! set while inactive reaching the plugin before activation, a re-instantiation for another
//! sample rate, and a latency change reported through the worker → restart request (realtime:
//! the worker thread; offline: synchronously).
//!
//! Every test passes with a note when lilv isn't installed (nothing to host LV2 through).

vox_module_api::install_test_allocator!();

mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, ModuleFactory, ModuleState, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessMode, ProcessStatus, Taper, Transport,
    Unit, param_text,
};
use vox_modules::Gain;
use vox_plugin_host::state::unwrap;
use vox_test_lv2 as tl;

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

/// A temporary directory holding the test bundle.
struct Bundle {
    _dir: TempDir,
    path: std::path::PathBuf,
}

fn bundle() -> Bundle {
    let dir = TempDir::new("lv2");
    let path = make_lv2_bundle(&dir.0, "vox-test");
    Bundle { _dir: dir, path }
}

/// An activated instance of test plugin `uri` at `gain_db`.
fn plugin(
    b: &Bundle,
    uri: &str,
    gain_db: f64,
    mode: ProcessMode,
    max_block: u32,
) -> Box<dyn Module> {
    let mut m = lv2_factory(&b.path, uri, exact_options()).create().unwrap();
    m.load_state(&lv2_gain_state(gain_db)).unwrap();
    m.activate(&config(mode, max_block)).unwrap();
    m
}

#[test]
fn lv2_gain_is_bit_exact_with_the_in_process_gain_after_its_reported_latency() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let x = noise(RATE as usize, 3);
    let want = reference(&x, -6.0, &[]);
    for (mode, max_block, blocks, block) in [
        (
            ProcessMode::Realtime,
            1024,
            vec![256usize, 128, 200],
            256usize,
        ),
        (ProcessMode::Offline, 4096, vec![4096], 4096),
    ] {
        let mut m = plugin(&b, tl::URI_GAIN, -6.0, mode, max_block);
        let latency = m.latency_samples() as usize;
        assert_eq!(latency, block + tl::LATENCY_SAMPLES as usize, "{mode:?}");
        let r = run(&mut *m, &x, &blocks, &[]);
        assert!(!r.restart);
        assert!(r.out[..latency].iter().all(|v| *v == 0.0), "{mode:?}");
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{mode:?} LV2 gain vs in-process Gain"),
        );
        m.deactivate();
    }
}

#[test]
fn automation_reaches_the_plugin_sample_accurately() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
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
        .map(|&(at, db)| (at, tl::PORT_GAIN, db))
        .collect();
    let want = reference(&x, 0.0, &gain_events);
    for uri in [tl::URI_GAIN, tl::URI_GAIN_STEREO] {
        let mut m = plugin(&b, uri, 0.0, ProcessMode::Realtime, 1024);
        let latency = m.latency_samples() as usize;
        let r = run(&mut *m, &x, &[256, 100, 37, 256], &events);
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{uri}: automated LV2 gain"),
        );
        assert_eq!(m.param_value(ParamId(tl::PORT_GAIN)), Some(0.0));
    }
}

#[test]
fn a_stereo_only_plugin_runs_through_the_mono_shim() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let x = noise(RATE as usize / 2, 5);
    let mut m = plugin(&b, tl::URI_GAIN_STEREO, -4.5, ProcessMode::Realtime, 1024);
    let latency = m.latency_samples() as usize;
    let r = run(&mut *m, &x, &[256], &[]);
    // Upmix (x, x), the plugin's per-channel gain, downmix (L + R) / 2: exactly the mono gain.
    assert_bits(
        &r.out[latency..],
        &reference(&x, -4.5, &[])[..x.len() - latency],
        "stereo plugin through the shim",
    );
    assert_eq!(m.params().len(), 1);
    assert_eq!(m.param_value(ParamId(tl::PORT_GAIN)), Some(-4.5));
}

/// The PVL2 state's control values and properties (key, type, bytes).
type Parsed = (Vec<(String, f32)>, Vec<(String, String, Vec<u8>)>);

fn parse(data: &[u8]) -> Parsed {
    assert_eq!(&data[..4], b"PVL2");
    let mut at = 8;
    let u32_at = |at: &mut usize| {
        let v = u32::from_le_bytes(data[*at..*at + 4].try_into().unwrap());
        *at += 4;
        v as usize
    };
    let text = |at: &mut usize| {
        let n = u32::from_le_bytes(data[*at..*at + 4].try_into().unwrap()) as usize;
        let s = data[*at + 4..*at + 4 + n].to_vec();
        *at += 4 + n;
        s
    };
    let mut controls = Vec::new();
    for _ in 0..u32_at(&mut at) {
        let symbol = String::from_utf8(text(&mut at)).unwrap();
        let v = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        at += 4;
        controls.push((symbol, v));
    }
    let mut props = Vec::new();
    for _ in 0..u32_at(&mut at) {
        let key = String::from_utf8(text(&mut at)).unwrap();
        let type_ = String::from_utf8(text(&mut at)).unwrap();
        at += 4;
        props.push((key, type_, text(&mut at)));
    }
    assert_eq!(at, data.len());
    (controls, props)
}

fn restores(parsed: &Parsed) -> i32 {
    let (_, t, v) = parsed
        .1
        .iter()
        .find(|(k, _, _)| k == tl::STATE_KEY_RESTORES)
        .expect("the plugin's restore counter");
    assert_eq!(t, "http://lv2plug.in/ns/ext/atom#Int");
    i32::from_le_bytes(v[..4].try_into().unwrap())
}

#[test]
fn state_is_the_control_values_and_the_plugins_properties_wrapped_with_its_identity() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let f = lv2_factory(&b.path, tl::URI_GAIN, exact_options());
    let mut a = f.create().unwrap();
    a.load_state(&lv2_gain_state(-7.5)).unwrap();
    a.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    let st = a.save_state().unwrap();
    assert_eq!(st.params["gain"].to_bits(), (-7.5f64).to_bits());
    let (h, data) = unwrap(st.blob.as_deref().expect("a plugin state")).unwrap();
    assert_eq!(
        (h.format.as_str(), h.id.as_str(), h.version.as_str()),
        ("lv2", format!("lv2:{}", tl::URI_GAIN).as_str(), "2.3")
    );
    let parsed = parse(data);
    assert!(
        parsed.0.contains(&("gain".to_owned(), -7.5)),
        "{:?}",
        parsed.0
    );
    assert!(
        parsed.0.contains(&("Toggle".to_owned(), 1.0)),
        "by symbol, verbatim"
    );
    assert_eq!(restores(&parsed), 0, "a fresh instance");
    let note = &parsed
        .1
        .iter()
        .find(|p| p.0 == tl::STATE_KEY_NOTE)
        .unwrap()
        .2;
    assert_eq!(note, format!("{}\0", tl::STATE_NOTE).as_bytes());
    a.deactivate();

    // The blob alone restores the value and goes through the plugin's own restore.
    let mut b2 = f.create().unwrap();
    b2.load_state(&ModuleState {
        format_version: 1,
        params: Default::default(),
        blob: st.blob.clone(),
    })
    .unwrap();
    assert_eq!(b2.param_value(ParamId(tl::PORT_GAIN)), Some(-7.5));
    let again = b2.save_state().unwrap();
    assert_eq!(
        restores(&parse(unwrap(again.blob.as_deref().unwrap()).unwrap().1)),
        1
    );

    // A plugin without `state:interface`: the control values only.
    let s = lv2_factory(&b.path, tl::URI_GAIN_STEREO, exact_options());
    let mut stereo = s.create().unwrap();
    stereo.load_state(&lv2_gain_state(-3.0)).unwrap();
    let sst = stereo.save_state().unwrap();
    let parsed = parse(unwrap(sst.blob.as_deref().unwrap()).unwrap().1);
    assert_eq!(parsed, (vec![("gain".to_owned(), -3.0)], Vec::new()));

    // Another plugin's state is refused.
    let mut c = lv2_factory(&b.path, tl::URI_LATENCY, exact_options())
        .create()
        .unwrap();
    assert!(c.load_state(&st).is_err());
}

#[test]
fn the_schema_and_parameter_text_come_from_the_plugin_data() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let m = lv2_factory(&b.path, tl::URI_GAIN, exact_options())
        .create()
        .unwrap();
    assert_eq!(m.descriptor().id, format!("lv2:{}", tl::URI_GAIN));
    let p = |id: u32| m.params().iter().find(|p| p.id == ParamId(id)).unwrap();
    let gain = p(tl::PORT_GAIN);
    let g = Gain::gain_param();
    assert_eq!(
        (gain.key.as_str(), gain.name.text.as_str()),
        ("gain", "Gain")
    );
    assert_eq!((gain.min, gain.max, gain.default), (g.min, g.max, 0.0));
    assert_eq!(gain.unit, Unit::Db);
    assert!(gain.flags.contains(ParamFlags::AUTOMATABLE));
    let mode = p(tl::PORT_MODE);
    let labels: Vec<&str> = mode.enum_labels.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(labels, ["Clean", "Warm", "Hot"]);
    let toggle = p(tl::PORT_TOGGLE);
    assert_eq!(toggle.key, "toggle", "symbols are lower-cased into keys");
    assert!(toggle.flags.contains(ParamFlags::BOOL));
    let freq = p(tl::PORT_FREQ);
    assert_eq!((freq.taper, freq.unit.clone()), (Taper::Log, Unit::Hz));
    let level = p(tl::PORT_LEVEL);
    assert!(level.flags.contains(ParamFlags::READ_ONLY));
    assert!(!level.flags.contains(ParamFlags::AUTOMATABLE));
    assert_eq!(
        m.params().len(),
        5,
        "audio, atom and latency ports aren't parameters"
    );

    let text = param_text(&*m).expect("scale points are the plugin's own text");
    assert_eq!(
        text.values_to_text(&[
            (ParamId(tl::PORT_MODE), 1.0),
            (ParamId(tl::PORT_GAIN), -6.0),
        ]),
        vec![Some("Warm".to_owned()), None]
    );
    assert_eq!(
        text.text_to_value(ParamId(tl::PORT_MODE), " hot "),
        Some(2.0)
    );

    let l = lv2_factory(&b.path, tl::URI_LATENCY, exact_options())
        .create()
        .unwrap();
    let lat = l
        .params()
        .iter()
        .find(|p| p.id == ParamId(tl::PORT_LATENCY))
        .unwrap();
    assert!(lat.flags.contains(ParamFlags::STEPPED));
    assert_eq!(
        (lat.min, lat.max, lat.default, lat.unit.clone()),
        (0.0, f64::from(tl::MAX_LATENCY), 64.0, Unit::Samples)
    );
    assert_eq!(l.groups().len(), 1);
    assert_eq!(l.groups()[0].name.text, "Advanced");
    assert_eq!(lat.group, Some(l.groups()[0].id));
}

#[test]
fn control_outputs_reach_the_mirror_as_read_only_values() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let mut m = plugin(&b, tl::URI_GAIN, -3.0, ProcessMode::Realtime, 1024);
    let x = noise(4_800, 11);
    run(&mut *m, &x, &[256], &[]);
    let t0 = Instant::now();
    while m.param_value(ParamId(tl::PORT_LEVEL)) != Some(-3.0) {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the level output never arrived"
        );
        run(&mut *m, &x[..512], &[256], &[]);
    }
}

/// Runs blocks until a restart is requested or `timeout` (the worker thread answers
/// asynchronously).
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
fn a_latency_change_goes_through_the_worker_and_asks_for_a_restart() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let f = lv2_factory(&b.path, tl::URI_LATENCY, exact_options());
    let mut m = f.create().unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + tl::LATENCY_SAMPLES);
    let x = noise(9_600, 6);
    // The plugin schedules work; the worker thread's response changes the reported latency.
    let r = run(&mut *m, &x, &[256], &[(1_000, tl::PORT_LATENCY, 300.0)]);
    assert!(
        r.restart || run_until_restart(&mut *m, Duration::from_secs(5)),
        "no restart request after the latency change"
    );
    assert_eq!(m.param_value(ParamId(tl::PORT_LATENCY)), Some(300.0));
    // What the rack's replacement does: a new instance from the current state.
    let st = m.save_state().unwrap();
    m.deactivate();
    let mut next = f.create().unwrap();
    next.load_state(&st).unwrap();
    next.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(next.latency_samples(), 256 + 300);
    assert!(!run_until_restart(&mut *next, Duration::from_millis(200)));

    // Offline, the worker runs synchronously (right after the `run` that scheduled it), so the
    // plugin reports the new latency at its next `run` — the chunk after the event's — and the
    // sandbox sends the restart request with that chunk. The proxy is pipelined one block deep:
    // a request emitted by the chunk still in flight when `process` returns is only popped by a
    // later `process` call, so the stream goes on for several blocks after the event (one
    // continuous stream, as in a render).
    let mut off = f.create().unwrap();
    off.activate(&config(ProcessMode::Offline, 4096)).unwrap();
    let long = noise(4096 * 8, 13);
    let r = run(&mut *off, &long, &[4096], &[(100, tl::PORT_LATENCY, 128.0)]);
    assert!(r.restart, "offline worker round trip");
}

/// The rack's committed state wins over the blob captured at instantiation: a value set while
/// inactive is on the port before activation, so the reported latency is already the new one.
#[test]
fn values_set_while_inactive_apply_before_activation() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let f = lv2_factory(&b.path, tl::URI_LATENCY, exact_options());
    let stale = f.create().unwrap().save_state().unwrap();
    let mut m = f.create().unwrap();
    let mut state = stale;
    state.params.insert("latency".into(), 300.0);
    m.load_state(&state).unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + 300);
    assert!(!run_until_restart(&mut *m, Duration::from_millis(200)));
}

#[test]
fn another_sample_rate_reinstantiates_with_the_state() {
    if !lv2_available() {
        return;
    }
    let b = bundle();
    let mut m = lv2_factory(&b.path, tl::URI_GAIN, exact_options())
        .create()
        .unwrap();
    m.load_state(&lv2_gain_state(-2.5)).unwrap();
    m.activate(&config_at(44_100.0, ProcessMode::Realtime, 1024))
        .unwrap();
    let latency = m.latency_samples() as usize;
    let x = noise(44_100, 12);
    let r = run(&mut *m, &x, &[256], &[]);
    assert_bits(
        &r.out[latency..],
        &reference(&x, -2.5, &[])[..x.len() - latency],
        "44.1 kHz instance",
    );
    // The plugin's properties were carried over (its restore ran once).
    let st = m.save_state().unwrap();
    assert_eq!(
        restores(&parse(unwrap(st.blob.as_deref().unwrap()).unwrap().1)),
        1
    );
}
