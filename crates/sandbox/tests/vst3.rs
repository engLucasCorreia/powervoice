//! T-806: the VST3 backend through real `powervoice-sandbox` processes and the in-repo test
//! plugin (`vox-test-vst3`, dlopened from its `cdylib`): bit-exact with the in-process Gain
//! after the reported latency (realtime and offline), sample-accurate automation through the
//! `IParameterChanges` queues, the mono shim for a stereo-only plugin (with a combined
//! controller), component + controller state wrapped with the plugin identity, the plugin's own
//! parameter text, the schema mapping (normalized / step-index domains, units as groups),
//! values set while inactive reaching the processor before activation, and a latency change →
//! restart request (controller → `restartComponent(kLatencyChanged)`).

vox_module_api::install_test_allocator!();

mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, ModuleFactory, ModuleState, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessMode, ProcessStatus, Transport,
    param_text,
};
use vox_modules::Gain;
use vox_plugin_host::state::unwrap;
use vox_test_vst3 as tv;

fn config(mode: ProcessMode, max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    }
}

fn n(db: f64) -> f64 {
    tv::gain_db_to_normalized(db)
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

/// An activated instance of test class `cid` at `gain_db`.
fn plugin(cid: &str, gain_db: f64, mode: ProcessMode, max_block: u32) -> Box<dyn Module> {
    let mut m = vst3_factory(cid, exact_options()).create().unwrap();
    m.load_state(&vst3_gain_state(gain_db)).unwrap();
    m.activate(&config(mode, max_block)).unwrap();
    m
}

#[test]
fn vst3_gain_is_bit_exact_with_the_in_process_gain_after_its_reported_latency() {
    let x = noise(RATE as usize, 3);
    let want = reference(&x, -6.0, &[]);
    for (mode, max_block, blocks, b) in [
        (
            ProcessMode::Realtime,
            1024,
            vec![256usize, 128, 200],
            256usize,
        ),
        (ProcessMode::Offline, 4096, vec![4096], 4096),
    ] {
        let mut m = plugin(tv::ID_GAIN, -6.0, mode, max_block);
        let latency = m.latency_samples() as usize;
        assert_eq!(latency, b + tv::LATENCY_SAMPLES as usize, "{mode:?}");
        let r = run(&mut *m, &x, &blocks, &[]);
        assert!(!r.restart);
        assert!(r.out[..latency].iter().all(|v| *v == 0.0), "{mode:?}");
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{mode:?} VST3 gain vs in-process Gain"),
        );
        m.deactivate();
    }
}

#[test]
fn automation_reaches_the_plugin_sample_accurately() {
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
        .map(|&(at, db)| (at, tv::PARAM_GAIN, n(db)))
        .collect();
    let want = reference(&x, 0.0, &gain_events);
    for cid in [tv::ID_GAIN, tv::ID_GAIN_STEREO] {
        let mut m = plugin(cid, 0.0, ProcessMode::Realtime, 1024);
        let latency = m.latency_samples() as usize;
        let r = run(&mut *m, &x, &[256, 100, 37, 256], &events);
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{cid}: automated VST3 gain"),
        );
        assert_eq!(m.param_value(ParamId(tv::PARAM_GAIN)), Some(n(0.0)));
    }
}

#[test]
fn a_stereo_only_plugin_with_a_combined_controller_runs_through_the_mono_shim() {
    let x = noise(RATE as usize / 2, 5);
    let mut m = plugin(tv::ID_GAIN_STEREO, -4.5, ProcessMode::Realtime, 1024);
    let latency = m.latency_samples() as usize;
    let r = run(&mut *m, &x, &[256], &[]);
    // Upmix (x, x), the plugin's per-channel gain, downmix (L + R) / 2: exactly the mono gain.
    assert_bits(
        &r.out[latency..],
        &reference(&x, -4.5, &[])[..x.len() - latency],
        "stereo plugin through the shim",
    );
    // The component is its own controller: schema, values and text come from it.
    assert_eq!(m.params().len(), 1);
    assert_eq!(m.param_value(ParamId(tv::PARAM_GAIN)), Some(n(-4.5)));
    let text = param_text(&*m).expect("the plugin formats its own values");
    assert_eq!(
        text.values_to_text(&[(ParamId(tv::PARAM_GAIN), n(-4.5))]),
        vec![Some("-4.50 dB".to_owned())]
    );
}

/// The PVV3 frame's two parts.
fn parts(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&data[..4], b"PVV3");
    let len_a = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
    let a = data[16..16 + len_a].to_vec();
    let rest = &data[16 + len_a..];
    let len_b = u64::from_le_bytes(rest[..8].try_into().unwrap()) as usize;
    (a, rest[8..8 + len_b].to_vec())
}

#[test]
fn state_is_the_component_and_controller_state_wrapped_with_the_plugin_identity() {
    let f = vst3_factory(tv::ID_GAIN, exact_options());
    let mut a = f.create().unwrap();
    a.load_state(&vst3_gain_state(-7.5)).unwrap();
    // Values set while inactive reach the processor (and its state) at activation.
    a.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    let st = a.save_state().unwrap();
    assert_eq!(st.params["p0"].to_bits(), n(-7.5).to_bits());
    let (h, data) = unwrap(st.blob.as_deref().expect("a plugin state")).unwrap();
    assert_eq!(
        (h.format.as_str(), h.id.as_str(), h.version.as_str()),
        ("vst3", format!("vst3:{}", tv::ID_GAIN).as_str(), "1.2.3")
    );
    let (component, controller) = parts(data);
    assert_eq!(&component[..4], &tv::COMPONENT_STATE_MAGIC);
    assert_eq!(
        f64::from_le_bytes(component[8..16].try_into().unwrap()).to_bits(),
        (-7.5f64).to_bits()
    );
    assert_eq!(&controller[..4], &tv::CONTROLLER_STATE_MAGIC);
    a.deactivate();

    // The blob alone restores the value (component state → the controller through
    // `setComponentState`).
    let mut b = f.create().unwrap();
    b.load_state(&ModuleState {
        format_version: 1,
        params: Default::default(),
        blob: st.blob.clone(),
    })
    .unwrap();
    assert_eq!(b.param_value(ParamId(tv::PARAM_GAIN)), Some(n(-7.5)));

    // Another plugin's state is refused.
    let mut c = vst3_factory(tv::ID_LATENCY, exact_options())
        .create()
        .unwrap();
    assert!(c.load_state(&st).is_err());
}

#[test]
fn the_schema_and_parameter_text_come_from_the_plugin() {
    let m = vst3_factory(tv::ID_LATENCY, exact_options())
        .create()
        .unwrap();
    assert_eq!(m.descriptor().id, format!("vst3:{}", tv::ID_LATENCY));
    let params = m.params();
    assert_eq!(params.len(), 2);
    let (g, l) = (&params[0], &params[1]);
    assert_eq!((g.key.as_str(), g.name.text.as_str()), ("p0", "Gain"));
    assert_eq!((g.min, g.max, g.default), (0.0, 1.0, n(0.0)));
    assert!(g.flags.contains(ParamFlags::AUTOMATABLE));
    assert_eq!(g.group, None);
    assert_eq!((l.key.as_str(), l.name.text.as_str()), ("p1", "Latency"));
    assert!(l.flags.contains(ParamFlags::STEPPED));
    assert_eq!(
        (l.min, l.max, l.step, l.default),
        (0.0, f64::from(tv::MAX_LATENCY), Some(1.0), 64.0)
    );
    assert_eq!(m.groups().len(), 1, "the Advanced unit");
    assert_eq!(m.groups()[0].name.text, "Advanced");
    assert_eq!(l.group, Some(m.groups()[0].id));
    assert_eq!(
        m.param_value(ParamId(tv::PARAM_LATENCY)),
        Some(f64::from(tv::LATENCY_SAMPLES))
    );

    let text = param_text(&*m).expect("the plugin formats its own values");
    assert_eq!(
        text.values_to_text(&[
            (ParamId(tv::PARAM_GAIN), n(-6.0)),
            (ParamId(tv::PARAM_LATENCY), 128.0),
            (ParamId(99), 1.0),
        ]),
        vec![
            Some("-6.00 dB".to_owned()),
            Some("128 smp".to_owned()),
            None
        ]
    );
    // The plugin parses plain numbers; the host strips the units.
    assert_eq!(
        text.text_to_value(ParamId(tv::PARAM_GAIN), " -3.5 dB")
            .map(f64::to_bits),
        Some(n(-3.5).to_bits())
    );
    assert_eq!(
        text.text_to_value(ParamId(tv::PARAM_LATENCY), "300 smp"),
        Some(300.0)
    );
    assert_eq!(text.text_to_value(ParamId(tv::PARAM_GAIN), "loud"), None);
}

/// Runs blocks (a little slower than real time is unnecessary: just with pauses, so the
/// sandbox's main thread gets its idle ticks) until a restart is requested or `timeout`.
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
fn a_latency_change_asks_for_a_restart_and_the_replacement_reports_it() {
    let f = vst3_factory(tv::ID_LATENCY, exact_options());
    let mut m = f.create().unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + tv::LATENCY_SAMPLES);
    let x = noise(9_600, 6);
    // The processor gets the change; the host syncs its controller, which asks for a restart.
    let r = run(&mut *m, &x, &[256], &[(1_000, tv::PARAM_LATENCY, 300.0)]);
    assert!(
        r.restart || run_until_restart(&mut *m, Duration::from_secs(5)),
        "no restart request after the latency change"
    );
    assert_eq!(m.param_value(ParamId(tv::PARAM_LATENCY)), Some(300.0));
    // What the rack's replacement does: a new instance from the current state.
    let st = m.save_state().unwrap();
    m.deactivate();
    let mut next = f.create().unwrap();
    next.load_state(&st).unwrap();
    next.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(next.latency_samples(), 256 + 300);
    // No second request from the new instance.
    assert!(!run_until_restart(&mut *next, Duration::from_millis(200)));
}

/// The rack's committed state is the mirror values plus the blob captured at instantiation, and
/// the values win: a value that differs from the blob reaches the processor before activation,
/// so the reported latency is already the new one (ADR-008 Amendment 6 §2).
#[test]
fn values_set_while_inactive_apply_before_activation() {
    let f = vst3_factory(tv::ID_LATENCY, exact_options());
    let stale = {
        let a = f.create().unwrap();
        a.save_state().unwrap()
    };
    let mut m = f.create().unwrap();
    let mut state = stale;
    state.params.insert("p1".into(), 300.0);
    m.load_state(&state).unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + 300);
    assert_eq!(m.param_value(ParamId(tv::PARAM_LATENCY)), Some(300.0));
    assert!(!run_until_restart(&mut *m, Duration::from_millis(200)));
}
