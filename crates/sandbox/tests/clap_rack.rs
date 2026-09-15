//! T-803: CLAP effects in the live rack — loaded in the background with a "Loading" status,
//! the plugin's own parameter text in notices and snapshots, typed text parsed by the plugin,
//! a latency change restarting the plugin with the new latency (compensated), and state round
//! trips through the sidecar JSON and a module preset.

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
use vox_plugin_host::state::{StateHeader, unwrap, wrap};
use vox_presets::ModulePresetStore;
use vox_rack::{
    LiveRack, MAX_BLOCK, RackHost, RackModel, RackNotice, RackOptions, SlotModel, SlotStatus,
};
use vox_test_clap as tc;

const GAIN_ID: &str = "clap:org.powervoice.test.gain";
const LATENCY_ID: &str = "clap:org.powervoice.test.latency";

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

fn clap_slot(id: &str, gain_db: f64) -> SlotModel {
    let r: ModuleRef = format!("{id}@1.2.3").parse().unwrap();
    SlotModel::new(&r, false, &clap_gain_state(gain_db))
}

fn status(host: &RackHost) -> SlotStatus {
    host.slot_info(0).unwrap().status
}

#[test]
fn a_clap_effect_loads_in_the_background_and_shows_its_own_parameter_text() {
    let f = clap_factory(tc::ID_GAIN, exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(Vec::new()),
    )
    .unwrap();
    host.insert_module(0, GAIN_ID).unwrap();
    // Not created on this thread: the slot is loading, dry, with no schema yet.
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Loading);
    assert_eq!(info.name, "Test org.powervoice.test.gain");
    assert!(info.params.is_empty());
    let notices = loaded(&mut host);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotLoaded { .. }))
    );
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Active);
    assert!(info.sandboxed);
    assert_eq!(info.params[0].key, "p0");
    let latency = host.total_latency_samples() as usize;
    assert_eq!(latency, 256 + tc::LATENCY_SAMPLES as usize);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::LatencyChanged { total_samples } if *total_samples as usize == latency))
    );

    // The plugin's own text: in notices, in snapshots, and when parsing typed text.
    host.take_notices();
    host.set_param(0, ParamId(tc::PARAM_GAIN), -6.0).unwrap();
    let texts: Vec<String> = host
        .take_notices()
        .into_iter()
        .filter_map(|n| match n {
            RackNotice::ParamChanged { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["-6.00 dB".to_owned()]);
    assert_eq!(
        host.set_param_text(0, ParamId(tc::PARAM_GAIN), "-4.5 dB")
            .unwrap()
            .to_bits(),
        (-4.5f64).to_bits()
    );
    assert_eq!(host.param_texts(0), vec!["-4.50 dB".to_owned()]);

    // Heard at -4.5 dB once the fade-in and the gain ramp are over, latency-compensated.
    let x = noise(RATE as usize, 7);
    let mut n = Vec::new();
    let y = drive(&mut host, &mut live, &x, &mut n);
    let want = reference(&x, -4.5);
    let s = settled(latency) + Gain::ramp_samples(RATE) as usize;
    assert_bits(
        &y[s..],
        &want[s - latency..x.len() - latency],
        "rack CLAP gain",
    );
    let pid = f.live_instances()[0].pid;
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || !pid_exists(pid)));
}

#[test]
fn a_latency_change_restarts_the_plugin_and_the_rack_compensates_it() {
    let f = clap_factory(tc::ID_LATENCY, exact_options());
    let (mut host, mut live) = RackHost::new(
        registry_with(std::slice::from_ref(&f)),
        rt(),
        RackOptions::default(),
        &model(vec![clap_slot(LATENCY_ID, 0.0)]),
    )
    .unwrap();
    loaded(&mut host);
    assert_eq!(host.total_latency_samples(), 256 + tc::LATENCY_SAMPLES);
    let first = f.live_instances()[0].pid;
    let x = noise(4_800, 8);
    let mut notices = Vec::new();
    drive(&mut host, &mut live, &x, &mut notices);
    host.set_param(0, ParamId(tc::PARAM_LATENCY), 300.0)
        .unwrap();
    let t0 = Instant::now();
    while host.total_latency_samples() != 256 + 300 || host.is_loading() {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "latency never changed: {}",
            host.total_latency_samples()
        );
        drive(&mut host, &mut live, &x, &mut notices);
        notices.extend(host.tick());
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
    assert_eq!(host.param_value(0, ParamId(tc::PARAM_LATENCY)), Some(300.0));
    // The old process is retired once its crossfade is over.
    assert!(
        wait_until(Duration::from_secs(5), || {
            drive(&mut host, &mut live, &x[..1024], &mut notices);
            !pid_exists(first)
        }),
        "the replaced sandbox wasn't retired"
    );
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    host.teardown(live);
    assert!(wait_until(Duration::from_secs(3), || pids
        .iter()
        .all(|p| !pid_exists(*p))));
}

#[test]
fn clap_state_round_trips_through_the_sidecar_json_and_a_module_preset() {
    let f = clap_factory(tc::ID_GAIN, exact_options());
    let registry = registry_with(std::slice::from_ref(&f));
    let (mut host, live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![clap_slot(GAIN_ID, 0.0)]),
    )
    .unwrap();
    loaded(&mut host);
    host.set_param(0, ParamId(tc::PARAM_GAIN), -9.5).unwrap();
    let saved = host.model();

    // Sidecar JSON.
    let text = serde_json::to_string_pretty(&serde_json::to_value(&saved).unwrap()).unwrap();
    let back: RackModel =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(&text).unwrap()).unwrap();
    assert_eq!(back, saved);
    assert!(
        back.slots[0]
            .module
            .starts_with("clap:org.powervoice.test.gain@")
    );
    let st: ModuleState = serde_json::from_value(back.slots[0].state.clone()).unwrap();
    assert_eq!(st.params["p0"].to_bits(), (-9.5f64).to_bits());
    let (header, data) = unwrap(st.blob.as_deref().expect("wrapped plugin state")).unwrap();
    assert_eq!(
        (header.format.as_str(), header.id.as_str()),
        ("clap", GAIN_ID)
    );
    assert_eq!(&data[..4], &tc::STATE_MAGIC);

    let (mut reopened, live2) =
        RackHost::new(registry.clone(), rt(), RackOptions::default(), &back).unwrap();
    loaded(&mut reopened);
    assert_eq!(reopened.param_value(0, ParamId(tc::PARAM_GAIN)), Some(-9.5));

    // Module preset: saved from the live slot, applied to a fresh slot at 0 dB.
    let dir = TempDir::new("t803-presets");
    let store = ModulePresetStore::new(&dir.0);
    let slot_state = host.slot_state(0).unwrap();
    store.save(GAIN_ID, "Quiet", &slot_state).unwrap();
    let preset = store.load(GAIN_ID, "Quiet").unwrap();
    assert_eq!(preset, slot_state);
    let (mut third, live3) = RackHost::new(
        registry,
        rt(),
        RackOptions::default(),
        &model(vec![clap_slot(GAIN_ID, 0.0)]),
    )
    .unwrap();
    loaded(&mut third);
    third.apply_module_preset(0, &preset).unwrap();
    loaded(&mut third);
    assert_eq!(third.param_value(0, ParamId(tc::PARAM_GAIN)), Some(-9.5));
    assert_eq!(
        third.slot_state(0).unwrap().params["p0"].to_bits(),
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

/// T-810 — version and identity (ADR-008 Amendment 2 §7 "a blob whose id or format differs is
/// refused"; loading applies the blob first, then the values):
/// - a state blob wrapped for a **different plugin id** is refused before its bytes ever reach
///   the plugin — the slot fails gracefully (never a crash), and the foreign blob round-trips
///   verbatim for the sidecar to keep (it is never dropped or reconstructed);
/// - a blob whose header names a **different version** of the *same* id is not refused by that
///   identity check (only `format`/`id` are compared) — it reaches the plugin's own
///   `LoadState`, which here accepts it (real bytes, real id, only the label differs);
/// - bytes the plugin's own loader actually rejects fail the same graceful way, with the
///   original (real, well-formed) blob kept for write-back.
#[test]
fn a_state_blob_for_a_different_plugin_id_is_refused_a_different_version_is_not() {
    let f = clap_factory(tc::ID_GAIN, exact_options());
    let registry = registry_with(std::slice::from_ref(&f));
    let gain_ref: ModuleRef = format!("{GAIN_ID}@1.2.3").parse().unwrap();

    // Capture a real, valid wrapped blob from a live instance.
    let (mut host, live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![clap_slot(GAIN_ID, -4.0)]),
    )
    .unwrap();
    loaded(&mut host);
    let good = host.slot_state(0).unwrap();
    let good_blob = good.blob.clone().expect("wrapped plugin state");
    let (header, data) = unwrap(&good_blob).unwrap();
    host.teardown(live);

    // --- Different id: refused, foreign blob kept verbatim. ---------------------------------
    let foreign = ModuleState {
        format_version: good.format_version,
        params: good.params.clone(),
        blob: Some(wrap(
            &StateHeader {
                format: header.format.clone(),
                plugin: header.plugin.clone(),
                id: "clap:org.powervoice.bogus".into(),
                version: header.version.clone(),
            },
            data,
        )),
    };
    let foreign_slot = SlotModel::new(&gain_ref, false, &foreign);
    let original = serde_json::to_value(&foreign_slot).unwrap();
    let (mut bad_host, bad_live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![foreign_slot]),
    )
    .unwrap();
    let notices = loaded(&mut bad_host);
    let SlotStatus::Failed { message } = status(&bad_host) else {
        panic!("expected Failed, got {:?}", status(&bad_host));
    };
    assert!(
        message.contains("clap:org.powervoice.bogus"),
        "message names the blob's own id: {message}"
    );
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotFailed { .. })),
        "a notice reaches the UI"
    );
    assert_eq!(
        serde_json::to_value(&bad_host.model().slots[0]).unwrap(),
        original,
        "the foreign blob round-trips verbatim, never dropped"
    );
    bad_host.teardown(bad_live);

    // --- Different version, same id: not refused by the identity check. --------------------
    let different_version = ModuleState {
        format_version: good.format_version,
        params: good.params.clone(),
        blob: Some(wrap(
            &StateHeader {
                format: header.format.clone(),
                plugin: header.plugin.clone(),
                id: header.id.clone(),
                version: "9.9.9".into(),
            },
            data,
        )),
    };
    let (mut ok_host, ok_live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![SlotModel::new(&gain_ref, false, &different_version)]),
    )
    .unwrap();
    loaded(&mut ok_host);
    assert_eq!(status(&ok_host), SlotStatus::Active);
    assert_eq!(
        ok_host.param_value(0, ParamId(tc::PARAM_GAIN)),
        Some(-4.0),
        "a differently-versioned-but-real blob still loads"
    );
    ok_host.teardown(ok_live);

    // --- Right id/format, bytes the plugin's own loader rejects. ---------------------------
    let corrupt = ModuleState {
        format_version: good.format_version,
        params: good.params.clone(),
        blob: Some(wrap(&header, b"not this plugin's real state bytes")),
    };
    let corrupt_slot = SlotModel::new(&gain_ref, false, &corrupt);
    let corrupt_original = serde_json::to_value(&corrupt_slot).unwrap();
    let (mut corrupt_host, corrupt_live) = RackHost::new(
        registry,
        rt(),
        RackOptions::default(),
        &model(vec![corrupt_slot]),
    )
    .unwrap();
    loaded(&mut corrupt_host);
    // Whether the test plugin's own loader is strict enough to reject arbitrary bytes is
    // backend-specific; either way it must never crash the sandbox or the rack, and a rejection
    // must keep the original blob verbatim, exactly like the foreign-id case above.
    match status(&corrupt_host) {
        SlotStatus::Failed { .. } => assert_eq!(
            serde_json::to_value(&corrupt_host.model().slots[0]).unwrap(),
            corrupt_original,
            "a rejected blob round-trips verbatim too"
        ),
        SlotStatus::Active => {}
        other => panic!("unexpected status: {other:?}"),
    }
    corrupt_host.teardown(corrupt_live);
}
