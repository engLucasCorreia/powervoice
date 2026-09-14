//! T-802 rack integration with real sandbox processes: a sandboxed slot is bit-exact and
//! allocation-free in the live rack; kill -9 mid-playback → the host keeps playing (bypass),
//! restarts once with the committed state, a second kill leaves the slot "Failed"; removal and
//! teardown reap the processes; state round-trips through the sidecar's JSON and a preset;
//! missing/unstartable plugins load as placeholders.

vox_module_api::install_test_allocator!();

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{ActivateConfig, ChannelLayout, Module, ModuleState, ProcessMode, Transport};
use vox_modules::Gain;
use vox_plugin_host::SandboxOptions;
use vox_plugin_host::state::unwrap;
use vox_presets::ModulePresetStore;
use vox_rack::{LiveRack, MAX_BLOCK, RackHost, RackModel, RackNotice, RackOptions, SlotStatus};

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn transport(pos: usize) -> Transport {
    Transport {
        playing: true,
        position_samples: Some(pos as u64),
    }
}

/// Drives `live` over `x` in `block`-frame calls on this thread (every call under the
/// allocation checker), ticking the host every 3 blocks.
fn drive(host: &mut RackHost, live: &mut LiveRack, x: &[f32], block: usize) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    for (k, (i, o)) in x.chunks(block).zip(y.chunks_mut(block)).enumerate() {
        no_alloc(|| live.process(transport(k * block), i, o)).expect("LiveRack allocated");
        if k % 3 == 0 {
            host.tick();
        }
    }
    y
}

fn reference(x: &[f32], gain_db: f64) -> Vec<f32> {
    let mut g = Gain::new();
    g.load_state(&Gain::state_with_gain_db(gain_db)).unwrap();
    g.activate(&rt()).unwrap();
    let mut y = vec![0.0f32; x.len()];
    let mut oe = vox_module_api::OutputEvents::with_capacity(4);
    for (i, o) in x.chunks(512).zip(y.chunks_mut(512)) {
        let mut ctx = vox_module_api::ProcessContext::new(
            i.len() as u32,
            0,
            Transport::default(),
            &[],
            &mut oe,
        );
        let mut outs = [o];
        g.process(&mut ctx, &[i], &mut outs);
    }
    y
}

fn status(host: &RackHost, index: usize) -> SlotStatus {
    host.slot_info(index).unwrap().status
}

/// Ticks until the rack's background loads are done (T-803: sandboxed slots are created off
/// the control thread).
fn loaded(host: &mut RackHost) {
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "sandboxed slots never loaded"
        );
        host.tick();
        thread::sleep(Duration::from_millis(2));
    }
    host.tick();
}

/// Where a slot that replaced its loading stand-in at the first callback is steady: its
/// latency, the 15 ms crossfade and a block of margin.
fn settled(latency: usize) -> usize {
    latency + (RATE * 0.015).round() as usize + 256
}

#[test]
fn a_sandboxed_slot_is_bit_exact_and_allocation_free_in_the_live_rack() {
    let f = factory("gain", "gain", exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(vec![gain_slot("test:gain@1.0.0", -6.0)]),
    )
    .unwrap();
    assert_eq!(status(&host, 0), SlotStatus::Loading);
    loaded(&mut host);
    let info = host.slot_info(0).unwrap();
    assert!(info.sandboxed);
    assert_eq!(info.status, SlotStatus::Active);
    assert_eq!(info.params[0], Gain::gain_param());
    let latency = host.total_latency_samples() as usize;
    assert_eq!(latency, 256);
    let x = noise(RATE as usize, 11);
    let y = drive(&mut host, &mut live, &x, 256);
    let want = reference(&x, -6.0);
    // The instance replaced its loading stand-in at the first callback (T-803): exact once
    // that crossfade is over.
    let s = settled(latency);
    assert_bits(
        &y[s..],
        &want[s - latency..x.len() - latency],
        "live rack vs in-process Gain",
    );
    let pid = f.live_instances()[0].pid;
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || !pid_exists(pid)));
}

struct Audio {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<(LiveRack, Vec<f32>, u32, Duration)>,
}

/// The "audio thread": 256-frame callbacks paced at the real-time rate until `stop`, every
/// call under the allocation checker; returns the output and the longest call.
fn start_audio(mut live: LiveRack, x: Arc<Vec<f32>>) -> Audio {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = thread::spawn(move || {
        let block = 256;
        let period = Duration::from_secs_f64(block as f64 / RATE);
        let mut y = Vec::with_capacity(x.len());
        let mut out = vec![0.0f32; block];
        let (mut violations, mut longest) = (0u32, Duration::ZERO);
        let start = Instant::now();
        for (k, i) in x.chunks(block).enumerate() {
            if flag.load(Ordering::Acquire) {
                break;
            }
            let t = Instant::now();
            if let Err(n) = no_alloc(|| live.process(transport(k * block), i, &mut out[..i.len()]))
            {
                violations += n;
            }
            longest = longest.max(t.elapsed());
            y.extend_from_slice(&out[..i.len()]);
            if let Some(d) =
                (start + period * (k as u32 + 1)).checked_duration_since(Instant::now())
            {
                thread::sleep(d);
            }
        }
        (live, y, violations, longest)
    });
    Audio { stop, thread }
}

/// Ticks the host every 10 ms until `cond` holds (or `timeout`).
fn tick_until(
    host: &mut RackHost,
    notices: &mut Vec<RackNotice>,
    timeout: Duration,
    mut cond: impl FnMut(&RackHost) -> bool,
) -> bool {
    let t0 = Instant::now();
    loop {
        notices.extend(host.tick());
        if cond(host) {
            return true;
        }
        if t0.elapsed() > timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn kill_9_mid_playback_bypasses_restarts_with_state_then_fails() {
    let f = factory("gain", "gain", default_options());
    let (mut host, live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(vec![gain_slot("test:gain@1.0.0", -12.0)]),
    )
    .unwrap();
    loaded(&mut host);
    let latency = host.total_latency_samples() as usize;
    let x = Arc::new(sine(RATE as usize * 30, 220.0, 0.5));
    let audio = start_audio(live, x.clone());
    let mut notices = Vec::new();
    let running = |h: &RackHost| status(h, 0) == SlotStatus::Active;
    tick_until(&mut host, &mut notices, Duration::from_millis(300), |_| {
        false
    });
    assert!(running(&host));
    let first = f.live_instances();
    assert_eq!(first.len(), 1);
    let pid1 = first[0].pid;

    // First kill: bypassed, "Restarting" with the reason, then restarted with the state.
    kill9(pid1);
    let mut restarting_message = None;
    assert!(
        tick_until(&mut host, &mut notices, Duration::from_secs(3), |h| {
            if let SlotStatus::Restarting { message } = status(h, 0) {
                restarting_message = Some(message);
            }
            restarting_message.is_some()
        }),
        "never Restarting: {:?}",
        status(&host, 0)
    );
    assert_eq!(
        restarting_message.as_deref(),
        Some("gain crashed and was bypassed")
    );
    assert!(
        tick_until(&mut host, &mut notices, Duration::from_secs(5), running),
        "not restarted: {:?}",
        status(&host, 0)
    );
    let healthy: Vec<_> = f
        .live_instances()
        .into_iter()
        .filter(|i| i.fault.is_none())
        .collect();
    assert_eq!(healthy.len(), 1);
    let pid2 = healthy[0].pid;
    assert_ne!(pid2, pid1);
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-12.0));
    assert!(
        wait_until(Duration::from_secs(3), || !pid_exists(pid1)),
        "the killed sandbox wasn't reaped"
    );
    tick_until(&mut host, &mut notices, Duration::from_millis(400), |_| {
        false
    });

    // Second kill: "Failed", no further restart.
    kill9(pid2);
    assert!(
        tick_until(
            &mut host,
            &mut notices,
            Duration::from_secs(3),
            |h| matches!(status(h, 0), SlotStatus::Failed { .. })
        ),
        "never Failed: {:?}",
        status(&host, 0)
    );
    tick_until(&mut host, &mut notices, Duration::from_millis(700), |_| {
        false
    });
    assert!(matches!(status(&host, 0), SlotStatus::Failed { .. }));
    assert!(
        f.live_instances().iter().all(|i| i.fault.is_some()),
        "no new sandbox after the second failure"
    );
    let failed = notices
        .iter()
        .filter(|n| matches!(n, RackNotice::SlotFailed { .. }))
        .count();
    let restarted = notices
        .iter()
        .filter(|n| matches!(n, RackNotice::SlotRestarted { .. }))
        .count();
    assert_eq!((failed, restarted), (2, 1));

    audio.stop.store(true, Ordering::Release);
    let (live, y, violations, longest) = audio.thread.join().unwrap();
    assert_eq!(violations, 0, "the audio thread allocated");
    assert!(
        longest < Duration::from_millis(20),
        "a callback blocked for {longest:?}"
    );
    assert!(max_step(&y) < 0.05, "discontinuity {}", max_step(&y));
    // The host kept playing: after the final failure, exactly the latency-matched dry signal.
    let n = y.len();
    let tail = (RATE * 0.4) as usize;
    assert_bits(
        &y[n - tail..],
        &x[n - tail - latency..n - latency],
        "dry tail",
    );
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || pids
        .iter()
        .all(|p| !pid_exists(*p))));
}

#[test]
fn removal_and_teardown_reap_the_sandboxes() {
    let shm_before = pvs_shm_objects();
    let f = factory("gain", "gain", exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(vec![
            gain_slot("test:gain@1.0.0", 0.0),
            gain_slot("test:gain@1.0.0", -3.0),
        ]),
    )
    .unwrap();
    loaded(&mut host);
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    assert_eq!(pids.len(), 2);
    let x = noise(4800, 1);
    drive(&mut host, &mut live, &x, 256);
    host.remove(0).unwrap();
    // The fade-out runs on the audio side; the retired instance comes back through the return
    // ring and is dropped on a tick, which retires its process.
    let gone = wait_until(Duration::from_secs(5), || {
        drive(&mut host, &mut live, &x[..1024], 256);
        host.tick();
        f.live_instances().len() == 1
    });
    assert!(gone, "the removed slot's proxy is still alive");
    let removed: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|p| !f.live_instances().iter().any(|i| i.pid == *p))
        .collect();
    assert_eq!(removed.len(), 1);
    assert!(wait_until(Duration::from_secs(3), || !pid_exists(
        removed[0]
    )));
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || pids
        .iter()
        .all(|p| !pid_exists(*p))));
    assert!(f.live_instances().is_empty());
    assert_eq!(
        pvs_shm_objects(),
        shm_before,
        "no shared-memory object left"
    );
}

#[test]
fn state_round_trips_through_the_sidecar_json_and_a_module_preset() {
    let f = factory("gain", "gain", exact_options());
    let registry = registry_with(std::slice::from_ref(&f));
    let (mut host, live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![gain_slot("test:gain@1.0.0", 0.0)]),
    )
    .unwrap();
    loaded(&mut host);
    host.set_param(0, Gain::GAIN_DB, -9.5).unwrap();
    let saved = host.model();

    // Sidecar: the document service stores `serde_json::to_value(&RackModel)`.
    let text = serde_json::to_string_pretty(&serde_json::to_value(&saved).unwrap()).unwrap();
    let back: RackModel =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(&text).unwrap()).unwrap();
    assert_eq!(back, saved);
    let st: ModuleState = serde_json::from_value(back.slots[0].state.clone()).unwrap();
    assert_eq!(state_gain_db(&st).to_bits(), (-9.5f64).to_bits());
    let (header, _) = unwrap(st.blob.as_deref().expect("wrapped plugin state")).unwrap();
    assert_eq!(
        (header.format.as_str(), header.id.as_str()),
        ("test", "test:gain")
    );

    let (mut reopened, mut live2) =
        RackHost::new(registry.clone(), rt(), RackOptions::default(), &back).unwrap();
    loaded(&mut reopened);
    assert_eq!(reopened.param_value(0, Gain::GAIN_DB), Some(-9.5));
    assert_eq!(status(&reopened, 0), SlotStatus::Active);
    let x = noise(12_000, 2);
    let y = drive(&mut reopened, &mut live2, &x, 256);
    let want = reference(&x, -9.5);
    let s = settled(256);
    assert_bits(&y[s..], &want[s - 256..x.len() - 256], "reopened rack");

    // Module preset: saved from the live slot, applied to a fresh slot at 0 dB.
    let dir = std::env::temp_dir().join(format!("powervoice-t802-presets-{}", std::process::id()));
    let store = ModulePresetStore::new(&dir);
    let slot_state = host.slot_state(0).unwrap();
    store.save("test:gain", "Quiet", &slot_state).unwrap();
    let preset = store.load("test:gain", "Quiet").unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(preset, slot_state);
    let (mut third, live3) = RackHost::new(
        registry,
        rt(),
        RackOptions::default(),
        &model(vec![gain_slot("test:gain@1.0.0", 0.0)]),
    )
    .unwrap();
    loaded(&mut third);
    third.apply_module_preset(0, &preset).unwrap();
    // A preset with a plugin state replaces the instance (in the background, T-803).
    loaded(&mut third);
    assert_eq!(third.param_value(0, Gain::GAIN_DB), Some(-9.5));
    assert_eq!(
        state_gain_db(&third.slot_state(0).unwrap()).to_bits(),
        (-9.5f64).to_bits()
    );

    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    host.teardown(live);
    reopened.teardown(live2);
    third.teardown(live3);
    assert!(wait_until(Duration::from_secs(3), || pids
        .iter()
        .all(|p| !pid_exists(*p))));
}

#[test]
fn missing_and_unstartable_plugins_load_as_placeholders() {
    // Not registered (dev flag off, or the plugin uninstalled): "Not installed", kept verbatim.
    let m = model(vec![gain_slot("test:gain@1.0.0", -3.0)]);
    let (mut host, live) =
        RackHost::new(registry_with(&[]), rt(), RackOptions::default(), &m).unwrap();
    match status(&host, 0) {
        SlotStatus::Missing { message, too_new } => {
            assert_eq!(message, "Missing module test:gain@1.0.0");
            assert!(!too_new);
        }
        other => panic!("expected Missing, got {other:?}"),
    }
    assert_eq!(host.model(), m);
    assert!(host.restart(0).is_err(), "Retry of a still-missing plugin");
    host.teardown(live);

    // Registered but the sandbox can't start: "Failed" with the reason, Retry fails cleanly.
    let bad = factory(
        "gain",
        "gain",
        SandboxOptions::new("/nonexistent/powervoice-sandbox"),
    );
    let (mut host, live) =
        RackHost::new(registry_with(&[bad]), rt(), RackOptions::default(), &m).unwrap();
    loaded(&mut host);
    match status(&host, 0) {
        SlotStatus::Failed { message } => {
            assert!(message.starts_with("Couldn't start test:gain"), "{message}");
            assert!(
                message.contains("couldn't start the plugin sandbox"),
                "{message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // Retry loads again in the background (T-803) and fails the same way.
    host.restart(0).unwrap();
    assert_eq!(status(&host, 0), SlotStatus::Loading);
    loaded(&mut host);
    assert!(matches!(status(&host, 0), SlotStatus::Failed { .. }));
    assert_eq!(host.model(), m);
    host.teardown(live);
}
