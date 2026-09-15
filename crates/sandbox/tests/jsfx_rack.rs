//! T-808: JSFX effects in the live rack — loaded in the background with a "Loading" status, enum
//! slider labels as parameter text, a `pdc_delay` change restarting the script with the new
//! latency (compensated), and state round trips (sliders + `@serialize`) through the sidecar JSON
//! and a module preset. Every test passes with a note where this build doesn't host JSFX.

vox_module_api::install_test_allocator!();

mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleRef, ModuleState, OutputEvents, ParamId,
    ProcessContext, ProcessMode, Transport,
};
use vox_modules::Gain;
use vox_plugin_host::state::unwrap;
use vox_presets::ModulePresetStore;
use vox_rack::{
    LiveRack, MAX_BLOCK, RackHost, RackModel, RackNotice, RackOptions, SlotModel, SlotStatus,
};

const GAIN: ParamId = ParamId(0);
const SLIDER2: ParamId = ParamId(1);

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn loaded(host: &mut RackHost) -> Vec<RackNotice> {
    let mut notices = Vec::new();
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(t0.elapsed() < Duration::from_secs(10), "never loaded");
        notices.extend(host.tick());
        thread::sleep(Duration::from_millis(2));
    }
    notices.extend(host.tick());
    notices
}

fn settled(latency: usize) -> usize {
    latency + (RATE * 0.015).round() as usize + 256
}

/// Drives `live` over `x` in 256-frame calls (allocation-checked), ticking every 3 blocks.
fn drive(
    host: &mut RackHost,
    live: &mut LiveRack,
    x: &[f32],
    notices: &mut Vec<RackNotice>,
) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    for (k, (i, o)) in x.chunks(256).zip(y.chunks_mut(256)).enumerate() {
        let t = Transport {
            playing: true,
            position_samples: Some((k * 256) as u64),
        };
        no_alloc(|| live.process(t, i, o)).expect("LiveRack allocated");
        if k % 3 == 0 {
            notices.extend(host.tick());
        }
    }
    y
}

fn reference(x: &[f32], gain_db: f64) -> Vec<f32> {
    let mut g = Gain::new();
    g.load_state(&Gain::state_with_gain_db(gain_db)).unwrap();
    g.activate(&rt()).unwrap();
    let mut y = vec![0.0f32; x.len()];
    let mut oe = OutputEvents::with_capacity(4);
    for (i, o) in x.chunks(512).zip(y.chunks_mut(512)) {
        let mut ctx = ProcessContext::new(i.len() as u32, 0, Transport::default(), &[], &mut oe);
        let mut outs = [o];
        g.process(&mut ctx, &[i], &mut outs);
    }
    y
}

fn jsfx_slot(id: &str, gain_db: f64) -> SlotModel {
    let r: ModuleRef = format!("{id}@1.2.3").parse().unwrap();
    SlotModel::new(&r, false, &jsfx_gain_state(gain_db))
}

fn status(host: &RackHost) -> SlotStatus {
    host.slot_info(0).unwrap().status
}

#[test]
fn a_jsfx_effect_loads_in_the_background_and_shows_its_enum_labels() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("gain.jsfx"), exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(Vec::new()),
    )
    .unwrap();
    host.insert_module(0, "jsfx:gain.jsfx").unwrap();
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Loading);
    let notices = loaded(&mut host);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotLoaded { .. }))
    );
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Active);
    assert!(info.sandboxed);
    assert!(info.params.iter().any(|p| p.key == "slider1"));
    let latency = host.total_latency_samples() as usize;
    assert_eq!(latency, 256);

    host.take_notices();
    host.set_param(0, SLIDER2, 2.0).unwrap();
    let texts: Vec<String> = host
        .take_notices()
        .into_iter()
        .filter_map(|n| match n {
            RackNotice::ParamChanged { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Hot".to_owned()]);
    host.set_param(0, GAIN, -4.5).unwrap();

    // Heard at -4.5 dB once the fade-in and the gain ramp are over, latency-compensated.
    let x = noise(RATE as usize, 7);
    let mut notices = Vec::new();
    let y = drive(&mut host, &mut live, &x, &mut notices);
    let want = reference(&x, -4.5);
    let s = settled(latency) + Gain::ramp_samples(RATE) as usize;
    assert_bits(
        &y[s..],
        &want[s - latency..x.len() - latency],
        "rack JSFX gain",
    );
    let pid = f.live_instances()[0].pid;
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || !pid_exists(pid)));
}

#[test]
fn a_latency_change_restarts_the_script_and_the_rack_compensates_it() {
    if !jsfx_available() {
        return;
    }
    let f = jsfx_factory(&jsfx_script("latency.jsfx"), exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(vec![jsfx_slot("jsfx:latency.jsfx", 0.0)]),
    )
    .unwrap();
    loaded(&mut host);
    assert_eq!(host.total_latency_samples(), 256 + 64);
    let first = f.live_instances()[0].pid;
    let x = noise(4_800, 8);
    let mut notices = Vec::new();
    drive(&mut host, &mut live, &x, &mut notices);
    host.set_param(0, SLIDER2, 300.0).unwrap();
    let t0 = Instant::now();
    while host.total_latency_samples() != 256 + 300 || host.is_loading() {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "latency never changed: {}",
            host.total_latency_samples()
        );
        drive(&mut host, &mut live, &x, &mut notices);
        notices.extend(host.tick());
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotRestarted { .. })),
        "{notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::LatencyChanged { total_samples: 556 }))
    );
    assert_eq!(status(&host), SlotStatus::Active);
    assert_eq!(host.param_value(0, SLIDER2), Some(300.0));
    let restarts = |ns: &[RackNotice]| {
        ns.iter()
            .filter(|n| matches!(n, RackNotice::SlotRestarted { .. }))
            .count()
    };
    let before = restarts(&notices);
    assert!(
        wait_until(Duration::from_secs(5), || {
            drive(&mut host, &mut live, &x[..1024], &mut notices);
            !pid_exists(first)
        }),
        "the replaced sandbox wasn't retired"
    );
    for _ in 0..20 {
        drive(&mut host, &mut live, &x[..1024], &mut notices);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(restarts(&notices), before, "{notices:?}");
    assert_eq!(host.total_latency_samples(), 256 + 300);
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || pids
        .iter()
        .all(|p| !pid_exists(*p))));
}

#[test]
fn jsfx_state_round_trips_through_the_sidecar_json_and_a_module_preset() {
    if !jsfx_available() {
        return;
    }
    let id = "jsfx:serialize.jsfx";
    let f = jsfx_factory(&jsfx_script("serialize.jsfx"), exact_options());
    let registry = registry_with(std::slice::from_ref(&f));
    let (mut host, live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![jsfx_slot(id, 0.0)]),
    )
    .unwrap();
    loaded(&mut host);
    host.set_param(0, GAIN, -9.5).unwrap();
    let saved = host.model();

    // Sidecar JSON.
    let text = serde_json::to_string_pretty(&serde_json::to_value(&saved).unwrap()).unwrap();
    let back: RackModel =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(&text).unwrap()).unwrap();
    assert_eq!(back, saved);
    assert!(back.slots[0].module.starts_with(&format!("{id}@")));
    let st: ModuleState = serde_json::from_value(back.slots[0].state.clone()).unwrap();
    assert_eq!(st.params["slider1"].to_bits(), (-9.5f64).to_bits());
    let (header, data) = unwrap(st.blob.as_deref().expect("wrapped plugin state")).unwrap();
    assert_eq!((header.format.as_str(), header.id.as_str()), ("jsfx", id));
    assert_eq!(&data[..4], b"PVJS");

    let (mut reopened, live2) =
        RackHost::new(registry.clone(), rt(), RackOptions::default(), &back).unwrap();
    loaded(&mut reopened);
    assert_eq!(reopened.param_value(0, GAIN), Some(-9.5));

    // Module preset: saved from the live slot, applied to a fresh slot at 0 dB.
    let presets = TempDir::new("t808-presets");
    let store = ModulePresetStore::new(&presets.0);
    let slot_state = host.slot_state(0).unwrap();
    store.save(id, "Quiet", &slot_state).unwrap();
    let preset = store.load(id, "Quiet").unwrap();
    assert_eq!(preset, slot_state);
    let (mut third, live3) = RackHost::new(
        registry,
        rt(),
        RackOptions::default(),
        &model(vec![jsfx_slot(id, 0.0)]),
    )
    .unwrap();
    loaded(&mut third);
    third.apply_module_preset(0, &preset).unwrap();
    loaded(&mut third);
    assert_eq!(third.param_value(0, GAIN), Some(-9.5));
    assert_eq!(
        third.slot_state(0).unwrap().params["slider1"].to_bits(),
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
