//! T-802 `ProxyModule` against real `powervoice-sandbox` processes: parameters mirrored,
//! events forwarded, bit-exact with the in-process Gain after the reported latency (offline and
//! realtime), state wrapped with the plugin identity, block growth restart, crash and hang
//! faults (hang within the heartbeat budget), creation failures, offline renders through the
//! rack. Process-spawning tests live in their own test binaries.

vox_module_api::install_test_allocator!();

mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, HostRequest, Module, ModuleFactory, ModuleState, OutputEvents,
    ParamEvent, ProcessContext, ProcessMode, ProcessStatus, Transport, adapter_health,
};
use vox_modules::Gain;
use vox_plugin_host::state::{StateHeader, unwrap};
use vox_plugin_host::{SandboxFault, SandboxOptions};
use vox_rack::offline::{RenderError, render};

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
    statuses: Vec<ProcessStatus>,
    violations: u32,
}

/// Runs `m` over `input` in blocks cycling through `blocks`, delivering `(position, gain_db)`
/// events at their absolute positions; every `process` under the allocation checker.
fn run(m: &mut dyn Module, input: &[f32], blocks: &[usize], events: &[(usize, f64)]) -> Run {
    let mut out = vec![0.0f32; input.len()];
    let mut out_events = OutputEvents::with_capacity(64);
    let mut evs: Vec<ParamEvent> = Vec::with_capacity(16);
    let (mut pos, mut k, mut steady) = (0usize, 0usize, 0u64);
    let mut statuses = Vec::new();
    let mut violations = 0;
    while pos < input.len() {
        let n = blocks[k % blocks.len()].min(input.len() - pos);
        k += 1;
        evs.clear();
        for &(at, v) in events {
            if at >= pos && at < pos + n {
                evs.push(ParamEvent {
                    offset: (at - pos) as u32,
                    id: Gain::GAIN_DB,
                    value: v,
                });
            }
        }
        out_events.clear();
        let (i, o) = (&input[pos..pos + n], &mut out[pos..pos + n]);
        let r = no_alloc(|| {
            let mut ctx = ProcessContext::new(
                n as u32,
                steady,
                Transport::default(),
                &evs,
                &mut out_events,
            );
            let mut outs = [o];
            m.process(&mut ctx, &[i], &mut outs)
        });
        match r {
            Ok(s) => statuses.push(s),
            Err(v) => violations += v,
        }
        pos += n;
        steady += n as u64;
    }
    Run {
        out,
        statuses,
        violations,
    }
}

/// The in-process Gain at `gain_db` over `input` with the same events.
fn reference(input: &[f32], gain_db: f64, events: &[(usize, f64)]) -> Vec<f32> {
    let mut g = Gain::new();
    g.load_state(&Gain::state_with_gain_db(gain_db)).unwrap();
    g.activate(&config(ProcessMode::Offline, 512)).unwrap();
    run(&mut g, input, &[512], events).out
}

fn bit_exact_through_proxy(mode: ProcessMode, max_block: u32, blocks: &[usize]) {
    let f = factory("gain", "gain", exact_options());
    let mut proxy = f.create().unwrap();
    assert_eq!(proxy.params(), &[Gain::gain_param()][..], "params mirrored");
    proxy.load_state(&Gain::state_with_gain_db(-6.0)).unwrap();
    assert_eq!(proxy.param_value(Gain::GAIN_DB), Some(-6.0));
    proxy.activate(&config(mode, max_block)).unwrap();
    let latency = proxy.latency_samples() as usize;
    let x = noise(30_000, 3);
    let events = [
        (10_000, 3.0),
        (10_001, -20.0),
        (20_000, -60.0),
        (25_123, 6.0),
    ];
    let want = reference(&x, -6.0, &events);
    let mut padded = x.clone();
    padded.resize(x.len() + latency, 0.0);
    let got = run(&mut *proxy, &padded, blocks, &events);
    assert_eq!(got.violations, 0, "the proxy allocated");
    assert!(
        got.statuses.iter().all(|s| *s == ProcessStatus::Continue),
        "a block failed"
    );
    assert!(got.out[..latency].iter().all(|v| *v == 0.0));
    assert_bits(&got.out[latency..], &want, "proxy vs in-process Gain");
    assert_eq!(
        proxy.param_value(Gain::GAIN_DB),
        Some(6.0),
        "mirror follows events"
    );
    proxy.deactivate();
}

#[test]
fn offline_proxy_is_bit_exact_with_in_process_gain() {
    bit_exact_through_proxy(ProcessMode::Offline, 512, &[512, 1, 300, 0, 512, 77]);
}

#[test]
fn realtime_proxy_is_bit_exact_after_its_latency() {
    // B = min_block (256): callbacks up to 256 frames never trigger a restart.
    bit_exact_through_proxy(ProcessMode::Realtime, 1024, &[256, 128, 1, 256, 0, 64, 200]);
}

#[test]
fn latency_is_the_transport_block_plus_the_plugin_latency() {
    let f = factory("gain", "gain", exact_options());
    let mut p = f.create().unwrap();
    p.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(
        p.latency_samples(),
        256,
        "B = min_block, Gain's own latency 0"
    );
    p.activate(&config(ProcessMode::Offline, 4096)).unwrap();
    assert_eq!(p.latency_samples(), 4096, "offline B = the render block");
    p.activate(&config(ProcessMode::Realtime, 128)).unwrap();
    assert_eq!(p.latency_samples(), 128, "capped by max_block");
}

#[test]
fn callbacks_larger_than_the_block_request_a_restart_with_a_larger_block() {
    let mut o = exact_options();
    o.min_block = 64;
    let f = factory("gain", "gain", o);
    let mut p = f.create().unwrap();
    p.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(p.latency_samples(), 64);
    let x = vec![0.1f32; 300];
    let mut y = vec![0.0f32; 300];
    let mut oe = OutputEvents::with_capacity(8);
    let mut ctx = ProcessContext::new(300, 0, Transport::default(), &[], &mut oe);
    let mut outs = [&mut y[..]];
    p.process(&mut ctx, &[&x], &mut outs);
    assert!(ctx.requested(HostRequest::Restart));
    let mut next = f.create().unwrap();
    next.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(
        next.latency_samples(),
        512,
        "next power of two of the observed 300"
    );
}

#[test]
fn state_is_wrapped_with_the_plugin_identity() {
    let f = factory("gain", "gain", exact_options());
    let mut a = f.create().unwrap();
    a.load_state(&Gain::state_with_gain_db(-9.5)).unwrap();
    let s = a.save_state().unwrap();
    assert_eq!(state_gain_db(&s).to_bits(), (-9.5f64).to_bits());
    let blob = s.blob.clone().expect("the plugin state");
    let (h, data) = unwrap(&blob).unwrap();
    assert_eq!(
        h,
        StateHeader {
            format: "test".into(),
            plugin: "gain".into(),
            id: "test:gain".into(),
            version: "1.0.0".into(),
        }
    );
    let inner: ModuleState = serde_json::from_slice(data).unwrap();
    assert_eq!(state_gain_db(&inner).to_bits(), (-9.5f64).to_bits());

    // Blob only: the value comes back from the plugin and refreshes the mirror.
    let mut b = f.create().unwrap();
    b.load_state(&ModuleState {
        format_version: 1,
        params: Default::default(),
        blob: Some(blob.clone()),
    })
    .unwrap();
    assert_eq!(b.param_value(Gain::GAIN_DB), Some(-9.5));
    // Values win over the blob (the rack's committed state carries both).
    let mut both = Gain::state_with_gain_db(4.0);
    both.blob = Some(blob.clone());
    b.load_state(&both).unwrap();
    assert_eq!(
        state_gain_db(&b.save_state().unwrap()).to_bits(),
        (4.0f64).to_bits()
    );

    // Another plugin's state, and garbage, are refused.
    let other = factory("crash", "crash?after=100000", exact_options());
    let mut c = other.create().unwrap();
    let err = c.load_state(&s).unwrap_err().to_string();
    assert!(err.contains("belongs to test:gain"), "{err}");
    let mut junk = Gain::state_with_gain_db(0.0);
    junk.blob = Some(b"junk".to_vec());
    assert!(c.load_state(&junk).is_err());
}

/// Paces `p` at the real-time rate in 256-frame blocks until it returns `Error` (or `limit`);
/// returns the time of the first `Error` and the output so far.
fn run_until_error(p: &mut dyn Module, x: &[f32], limit: Duration) -> (Option<Instant>, Vec<f32>) {
    let block = 256;
    let period = Duration::from_secs_f64(block as f64 / RATE);
    let mut y = vec![0.0f32; x.len()];
    let mut oe = OutputEvents::with_capacity(8);
    let start = Instant::now();
    let mut first_error = None;
    let mut errors_after = 0;
    for (k, (i, o)) in x.chunks(block).zip(y.chunks_mut(block)).enumerate() {
        let status = no_alloc(|| {
            let mut ctx =
                ProcessContext::new(i.len() as u32, 0, Transport::default(), &[], &mut oe);
            let mut outs = [o];
            p.process(&mut ctx, &[i], &mut outs)
        })
        .expect("the proxy allocated");
        if status == ProcessStatus::Error {
            first_error.get_or_insert_with(Instant::now);
            errors_after += 1;
            // Keep going a little: the bypassed output must be the dry signal.
            if errors_after > 40 {
                let n = (k + 1) * block;
                y.truncate(n);
                return (first_error, y);
            }
        }
        if start.elapsed() > limit {
            break;
        }
        if let Some(d) = (start + period * (k as u32 + 1)).checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    (first_error, y)
}

#[test]
fn a_crash_is_reported_bypassed_and_reaped() {
    let f = factory("crash", "crash?after=20", default_options());
    let mut p = f.create().unwrap();
    p.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    let health = adapter_health(&*p).expect("the proxy answers AdapterHealth");
    assert_eq!(health.fault(), None);
    let pid = f.live_instances()[0].pid;
    let x = sine(RATE as usize * 3, 220.0, 0.5);
    let (err, y) = run_until_error(&mut *p, &x, Duration::from_secs(3));
    assert!(err.is_some(), "the crash was never reported");
    assert_eq!(health.fault().as_deref(), Some("crashed"));
    assert_eq!(f.live_instances()[0].fault, Some(SandboxFault::Crashed));
    assert!(wait_until(Duration::from_secs(3), || !pid_exists(pid)));
    // Bypassed: exactly the latency-matched dry signal at the end.
    let n = y.len();
    assert_bits(&y[n - 2560..], &x[n - 2560 - 256..n - 256], "dry tail");
    assert!(max_step(&y) < 0.05, "discontinuity {}", max_step(&y));
}

#[test]
fn a_hang_is_detected_within_the_heartbeat_budget_and_killed() {
    let options = default_options();
    let hang_timeout = options.hang_timeout;
    let f = factory("hang", "hang?after=40", options);
    let mut p = f.create().unwrap();
    p.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    let pid = f.live_instances()[0].pid;
    let x = sine(RATE as usize * 3, 220.0, 0.5);
    let start = Instant::now();
    let (err, _) = run_until_error(&mut *p, &x, Duration::from_secs(3));
    let err = err.expect("the hang was never reported");
    // The plugin hangs on its 41st chunk, i.e. ~40 block periods after the start.
    let onset = start + Duration::from_secs_f64(40.0 * 256.0 / RATE);
    let detected = err.saturating_duration_since(onset);
    assert!(
        detected >= hang_timeout - Duration::from_millis(20)
            && detected <= hang_timeout + Duration::from_millis(250),
        "detected {detected:?} after the hang"
    );
    assert_eq!(
        adapter_health(&*p).unwrap().fault().as_deref(),
        Some("stopped responding")
    );
    assert!(
        wait_until(Duration::from_secs(2), || !pid_exists(pid)),
        "the hung sandbox wasn't killed"
    );
}

#[test]
fn creation_fails_cleanly_without_a_binary_or_with_an_unknown_plugin() {
    let bad = factory(
        "gain",
        "gain",
        SandboxOptions::new("/nonexistent/powervoice-sandbox"),
    );
    let e = bad.create().err().unwrap().to_string();
    assert!(e.contains("couldn't start the plugin sandbox"), "{e}");
    let nope = factory("nope", "nope", exact_options());
    let e = nope.create().err().unwrap().to_string();
    assert!(e.contains("no test plugin named `nope`"), "{e}");
    assert!(nope.live_instances().is_empty());
}

#[test]
fn offline_render_through_the_rack_matches_the_in_process_gain() {
    let f = factory("gain", "gain", exact_options());
    let registry = registry_with(&[f]);
    let x = noise(20_000, 5);
    let sandboxed = render(
        &registry,
        &model(vec![gain_slot("test:gain@1.0.0", -6.0)]),
        RATE,
        &x,
    )
    .unwrap();
    let in_process = render(
        &registry,
        &model(vec![gain_slot("org.powervoice.gain@1.0.0", -6.0)]),
        RATE,
        &x,
    )
    .unwrap();
    assert_bits(&sandboxed, &in_process, "offline render");
}

#[test]
fn an_offline_render_aborts_when_the_plugin_crashes() {
    let f = factory("crash", "crash?after=2", exact_options());
    let registry = registry_with(&[f]);
    let x = noise(4096 * 6, 5);
    let t0 = Instant::now();
    let err = render(
        &registry,
        &model(vec![gain_slot("test:crash@1.0.0", 0.0)]),
        RATE,
        &x,
    )
    .unwrap_err();
    assert!(
        matches!(err, RenderError::SlotFailed { index: 0, .. }),
        "{err}"
    );
    assert!(t0.elapsed() < Duration::from_secs(4), "aborted promptly");
}
