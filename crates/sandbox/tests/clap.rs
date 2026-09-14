//! T-803: the CLAP backend through real `powervoice-sandbox` processes and the in-repo test
//! plugin (`vox-test-clap`, dlopened from its `cdylib`): bit-exact with the in-process Gain
//! after the reported latency (realtime and offline), sample-accurate automation, the mono
//! shim for a stereo-only plugin, state wrapped with the plugin identity, the plugin's own
//! parameter text, the schema mapping, and a latency change → restart request.
//! Process-spawning tests live in their own test binaries.

vox_module_api::install_test_allocator!();

mod common;

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, ModuleFactory, ModuleState, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessMode, ProcessStatus, Transport,
    param_text,
};
use vox_modules::Gain;
use vox_plugin_host::state::unwrap;
use vox_test_clap as tc;

fn config(mode: ProcessMode, max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    }
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
        let n = blocks[k % blocks.len()].min(input.len() - pos);
        k += 1;
        evs.clear();
        for &(at, id, v) in events {
            if at >= pos && at < pos + n {
                evs.push(ParamEvent {
                    offset: (at - pos) as u32,
                    id: ParamId(id),
                    value: v,
                });
            }
        }
        out_events.clear();
        let (i, o) = (&input[pos..pos + n], &mut out[pos..pos + n]);
        let (status, requested) = no_alloc(|| {
            let mut ctx = ProcessContext::new(
                n as u32,
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
        pos += n;
        steady += n as u64;
    }
    Run { out, restart }
}

/// The in-process Gain at `gain_db` with gain events at absolute positions.
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

/// An activated instance of test plugin `id` at `gain_db`.
fn plugin(id: &str, gain_db: f64, mode: ProcessMode, max_block: u32) -> Box<dyn Module> {
    let mut m = clap_factory(id, exact_options()).create().unwrap();
    m.load_state(&clap_gain_state(gain_db)).unwrap();
    m.activate(&config(mode, max_block)).unwrap();
    m
}

#[test]
fn clap_gain_is_bit_exact_with_the_in_process_gain_after_its_reported_latency() {
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
        let mut m = plugin(tc::ID_GAIN, -6.0, mode, max_block);
        let latency = m.latency_samples() as usize;
        assert_eq!(latency, b + tc::LATENCY_SAMPLES as usize, "{mode:?}");
        let r = run(&mut *m, &x, &blocks, &[]);
        assert!(!r.restart);
        assert!(r.out[..latency].iter().all(|v| *v == 0.0), "{mode:?}");
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{mode:?} CLAP gain vs in-process Gain"),
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
        .map(|&(at, v)| (at, tc::PARAM_GAIN, v))
        .collect();
    let want = reference(&x, 0.0, &gain_events);
    for id in [tc::ID_GAIN, tc::ID_GAIN_STEREO] {
        let mut m = plugin(id, 0.0, ProcessMode::Realtime, 1024);
        let latency = m.latency_samples() as usize;
        let r = run(&mut *m, &x, &[256, 100, 37, 256], &events);
        assert_bits(
            &r.out[latency..],
            &want[..x.len() - latency],
            &format!("{id}: automated CLAP gain"),
        );
        assert_eq!(m.param_value(ParamId(tc::PARAM_GAIN)), Some(0.0));
    }
}

#[test]
fn a_stereo_only_plugin_runs_through_the_mono_shim() {
    let x = noise(RATE as usize / 2, 5);
    let mut m = plugin(tc::ID_GAIN_STEREO, -4.5, ProcessMode::Realtime, 1024);
    let latency = m.latency_samples() as usize;
    let r = run(&mut *m, &x, &[256], &[]);
    // Upmix (x, x), the plugin's per-channel gain, downmix (L + R) / 2: exactly the mono gain.
    assert_bits(
        &r.out[latency..],
        &reference(&x, -4.5, &[])[..x.len() - latency],
        "stereo plugin through the shim",
    );
}

#[test]
fn state_is_the_plugins_own_wrapped_with_its_identity() {
    let f = clap_factory(tc::ID_GAIN, exact_options());
    let mut a = f.create().unwrap();
    a.load_state(&clap_gain_state(-7.5)).unwrap();
    let st = a.save_state().unwrap();
    assert_eq!(st.params["p0"].to_bits(), (-7.5f64).to_bits());
    let (h, data) = unwrap(st.blob.as_deref().expect("a plugin state")).unwrap();
    assert_eq!(
        (h.format.as_str(), h.id.as_str(), h.version.as_str()),
        ("clap", "clap:org.powervoice.test.gain", "1.2.3")
    );
    assert_eq!(&data[..4], &tc::STATE_MAGIC);

    // The blob alone restores the value (the plugin's own state stream).
    let mut b = f.create().unwrap();
    b.load_state(&ModuleState {
        format_version: 1,
        params: Default::default(),
        blob: st.blob.clone(),
    })
    .unwrap();
    assert_eq!(b.param_value(ParamId(tc::PARAM_GAIN)), Some(-7.5));

    // Another plugin's state is refused.
    let mut c = clap_factory(tc::ID_LATENCY, exact_options())
        .create()
        .unwrap();
    assert!(c.load_state(&st).is_err());
}

#[test]
fn the_schema_and_parameter_text_come_from_the_plugin() {
    let m = clap_factory(tc::ID_LATENCY, exact_options())
        .create()
        .unwrap();
    assert_eq!(m.descriptor().id, "clap:org.powervoice.test.latency");
    let params = m.params();
    assert_eq!(params.len(), 2);
    let (g, l) = (&params[0], &params[1]);
    assert_eq!((g.key.as_str(), g.name.text.as_str()), ("p0", "Gain"));
    assert_eq!((g.min, g.max, g.default), (-60.0, 24.0, 0.0));
    assert!(g.flags.contains(ParamFlags::AUTOMATABLE));
    assert_eq!(g.group, None);
    assert_eq!(l.key, "p1");
    assert!(l.flags.contains(ParamFlags::STEPPED));
    assert_eq!(l.step, Some(1.0));
    assert_eq!(m.groups().len(), 1);
    assert_eq!(m.groups()[0].name.text, "Advanced");
    assert_eq!(l.group, Some(m.groups()[0].id));
    assert_eq!(
        m.param_value(ParamId(tc::PARAM_LATENCY)),
        Some(f64::from(tc::LATENCY_SAMPLES))
    );

    let text = param_text(&*m).expect("the plugin formats its own values");
    assert_eq!(
        text.values_to_text(&[
            (ParamId(tc::PARAM_GAIN), -6.0),
            (ParamId(tc::PARAM_LATENCY), 128.0),
            (ParamId(99), 1.0),
        ]),
        vec![
            Some("-6.00 dB".to_owned()),
            Some("128 smp".to_owned()),
            None
        ]
    );
    assert_eq!(
        text.text_to_value(ParamId(tc::PARAM_GAIN), " -3.5 dB"),
        Some(-3.5)
    );
    assert_eq!(text.text_to_value(ParamId(tc::PARAM_GAIN), "loud"), None);
}

#[test]
fn a_latency_change_asks_for_a_restart_and_the_replacement_reports_it() {
    let f = clap_factory(tc::ID_LATENCY, exact_options());
    let mut m = f.create().unwrap();
    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(m.latency_samples(), 256 + tc::LATENCY_SAMPLES);
    let x = noise(9_600, 6);
    let r = run(&mut *m, &x, &[256], &[(1_000, tc::PARAM_LATENCY, 300.0)]);
    assert!(r.restart, "no restart request after the latency change");
    assert_eq!(m.param_value(ParamId(tc::PARAM_LATENCY)), Some(300.0));
    // What the rack's replacement does: a new instance from the current values.
    let st = m.save_state().unwrap();
    m.deactivate();
    let mut n = f.create().unwrap();
    n.load_state(&st).unwrap();
    n.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(n.latency_samples(), 256 + 300);
    // A second request doesn't repeat within one instance.
    let r = run(&mut *n, &x, &[256], &[]);
    assert!(!r.restart);
}
