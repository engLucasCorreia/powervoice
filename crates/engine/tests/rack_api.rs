//! S3-01: rack commands round-trip through the engine on the fake backend (SPEC-012 §2.1, §2.4,
//! §2.6). The generic parameter UI is driven entirely by these commands and the `RackSnapshot`
//! they return, so this test exercises the same surface `src-tauri`'s IPC commands wrap.

vox_module_api::install_test_allocator!();

use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{
    EngineConfig, EngineEvent, HostId, ManualEngine, ModuleTelemetryFrame, RackApiError,
    RackCommand,
};
use vox_modules::{Gain, NoiseGate, ParametricEq, TruePeakLimiter};
use vox_rack::{RackNotice, Registry, SlotStatus};

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn rig() -> (ManualEngine, Arc<Mutex<Vec<EngineEvent>>>) {
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    (eng, events)
}

/// H-03 (SPEC-016 §4.12 `VXMT`): with a module-telemetry sink installed, every control tick
/// publishes the values of the slots that have a `Telemetry` extension, keyed by slot uid in
/// the snapshot's channel order — and nothing while no slot has one.
#[test]
fn module_telemetry_frames_carry_the_slots_with_telemetry() {
    let (mut eng, _events) = rig();
    let frames: Arc<Mutex<Vec<ModuleTelemetryFrame>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    eng.set_module_telemetry_sink(Some(Box::new(move |f: &ModuleTelemetryFrame| {
        sink.lock().unwrap().push(f.clone());
    })));
    eng.tick();
    assert!(
        frames.lock().unwrap().is_empty(),
        "no slot has telemetry yet"
    );

    let snap = eng
        .rack_command(RackCommand::Add {
            module_id: TruePeakLimiter::ID.into(),
            index: 0,
        })
        .unwrap();
    let info = &snap.slots[0].info;
    assert_eq!(info.telemetry.len(), 1);
    assert_eq!(info.telemetry[0].key, "gain_reduction_db");
    let uid = info.uid.0;
    eng.tick();
    eng.tick();
    let frames = frames.lock().unwrap();
    assert!(frames.len() >= 2, "{} frames", frames.len());
    let last = frames.last().unwrap();
    assert_eq!(last.seq, frames.len() as u32 - 1);
    let record = last
        .records
        .iter()
        .find(|r| r.slot_uid == uid)
        .expect("the limiter's record");
    assert_eq!(record.values.len(), 1);
    assert!(record.values[0] <= 0.0 && record.values[0] >= -24.0);
    let bytes = last.encode();
    assert_eq!(&bytes[0..4], b"VXMT");
}

/// Before any output device is open there is no live rack: an empty snapshot and every command
/// reports `Unavailable` rather than panicking (the module docs' documented edge case).
#[test]
fn rack_is_unavailable_before_an_output_device_opens() {
    let fake = FakeBackend::new(1); // nothing plugged: no output ever opens
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();

    let snap = eng.rack_snapshot();
    assert!(snap.slots.is_empty());
    assert_eq!(snap.latency_samples, 0);

    let err = eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    });
    assert!(matches!(err, Err(RackApiError::Unavailable)));
}

/// `rack_registry` lists the built-ins regardless of whether a live rack exists (SPEC-012 §2.1
/// Add-module menu).
#[test]
fn registry_lists_the_builtin_modules() {
    let (eng, _events) = rig();
    let ids: Vec<String> = eng.rack_registry().into_iter().map(|d| d.id).collect();
    assert!(ids.contains(&Gain::ID.to_owned()));
    assert!(ids.contains(&NoiseGate::ID.to_owned()));
}

/// Add / move / bypass / A-B / remove round-trip through the engine and are reflected
/// immediately in each command's returned snapshot (SPEC-012 §2.1–§2.3).
#[test]
fn add_move_bypass_ab_and_remove_round_trip() {
    let (mut eng, _events) = rig();

    let snap = eng
        .rack_command(RackCommand::Add {
            module_id: Gain::ID.into(),
            index: 0,
        })
        .unwrap();
    assert_eq!(snap.slots.len(), 1);
    assert_eq!(
        snap.slots[0].info.module,
        format!("{}@{}", Gain::ID, Gain::VERSION)
    );
    assert!(matches!(snap.slots[0].info.status, SlotStatus::Active));

    let snap = eng
        .rack_command(RackCommand::Add {
            module_id: NoiseGate::ID.into(),
            index: 1,
        })
        .unwrap();
    assert_eq!(snap.slots.len(), 2);
    assert!(snap.slots[1].info.module.starts_with(NoiseGate::ID));

    let snap = eng
        .rack_command(RackCommand::Move { from: 0, to: 1 })
        .unwrap();
    assert!(snap.slots[0].info.module.starts_with(NoiseGate::ID));
    assert!(snap.slots[1].info.module.starts_with(Gain::ID));

    let snap = eng
        .rack_command(RackCommand::SetBypass { index: 1, on: true })
        .unwrap();
    assert!(snap.slots[1].info.bypass);

    assert!(!snap.ab);
    let snap = eng.rack_command(RackCommand::SetAb { on: true }).unwrap();
    assert!(snap.ab);

    let snap = eng.rack_command(RackCommand::Remove { index: 0 }).unwrap();
    assert_eq!(snap.slots.len(), 1);
    assert!(snap.slots[0].info.module.starts_with(Gain::ID));

    // An unknown module id is rejected without touching the rack.
    let err = eng.rack_command(RackCommand::Add {
        module_id: "org.powervoice.nope".into(),
        index: 0,
    });
    assert!(matches!(err, Err(RackApiError::Rack(_))));
    assert_eq!(eng.rack_snapshot().slots.len(), 1);

    // Out-of-range indices are rejected the same way.
    let err = eng.rack_command(RackCommand::Remove { index: 9 });
    assert!(matches!(err, Err(RackApiError::Rack(_))));
}

/// A normalized-position set and a typed-text set both round-trip through Rust's own text
/// formatting (SPEC-012 §2.6: the UI never formats or parses values itself), and the value the
/// mirror reports afterward always equals the last value sent (§2.4 "no lost values").
#[test]
fn param_set_normalized_and_text_round_trip_through_rust_formatting() {
    let (mut eng, events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();

    let snap = eng
        .rack_command(RackCommand::SetParamNormalized {
            index: 0,
            id: Gain::GAIN_DB,
            value: 1.0, // top of the taper: +24 dB
        })
        .unwrap();
    let pi = snap.slots[0]
        .info
        .params
        .iter()
        .position(|p| p.id == Gain::GAIN_DB)
        .unwrap();
    assert!((snap.slots[0].values[pi] - 24.0).abs() < 1e-9);

    let snap = eng
        .rack_command(RackCommand::SetParamText {
            index: 0,
            id: Gain::GAIN_DB,
            text: "-6.0".into(),
        })
        .unwrap();
    assert!((snap.slots[0].values[pi] - (-6.0)).abs() < 1e-9);

    // Unparseable text sends nothing and reports the rack's `InvalidText` error.
    let err = eng.rack_command(RackCommand::SetParamText {
        index: 0,
        id: Gain::GAIN_DB,
        text: "banana".into(),
    });
    assert!(matches!(err, Err(RackApiError::Rack(_))));
    assert!((eng.rack_snapshot().slots[0].values[pi] - (-6.0)).abs() < 1e-9);

    // The engine forwards a `ParamChanged` notice carrying Rust's own display text.
    let saw_text = events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Rack(RackNotice::ParamChanged { id, text, .. })
                if *id == Gain::GAIN_DB && text == "-6.0 dB"
        )
    });
    assert!(saw_text, "expected a ParamChanged notice with \"-6.0 dB\"");
}

/// `Restart` of a healthy slot replaces its instance from the committed state and reports
/// `SlotRestarted` (ADR-005 §12) — the H-01 handoff the UI relies on to re-read `slot_info`.
#[test]
fn restart_reports_slot_restarted() {
    let (mut eng, events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    events.lock().unwrap().clear();

    eng.rack_command(RackCommand::Restart { index: 0 }).unwrap();
    let saw_restart = events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Rack(RackNotice::SlotRestarted { index: 0, .. })
        )
    });
    assert!(saw_restart, "expected a SlotRestarted notice");
}

/// S3-07 (SPEC-015 §2.6.6): `SetParamPlain` sets the mirror straight from a plain Hz/dB/Q value
/// (no normalization, no text parsing) and echoes the same `ParamChanged` notice the other two
/// setters do.
#[test]
fn set_param_plain_round_trips_through_the_mirror() {
    let (mut eng, events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: ParametricEq::ID.into(),
        index: 0,
    })
    .unwrap();

    let freq_id = ParametricEq::param_id(2, ParametricEq::FREQ);
    let snap = eng
        .rack_command(RackCommand::SetParamPlain {
            index: 0,
            id: freq_id,
            value: 1_234.0,
        })
        .unwrap();
    let pi = snap.slots[0]
        .info
        .params
        .iter()
        .position(|p| p.id == freq_id)
        .unwrap();
    assert!((snap.slots[0].values[pi] - 1_234.0).abs() < 1e-9);

    let saw_text = events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Rack(RackNotice::ParamChanged { id, .. }) if *id == freq_id
        )
    });
    assert!(saw_text, "expected a ParamChanged notice for the plain set");

    // Out of range: rejected without touching the rack (same contract as the other setters).
    let err = eng.rack_command(RackCommand::SetParamPlain {
        index: 9,
        id: freq_id,
        value: 1.0,
    });
    assert!(matches!(err, Err(RackApiError::Rack(_))));
}

/// S3-07 (SPEC-015 §2.6.6): `response_curve` evaluates the EQ's `ResponseCurve` extension at the
/// requested frequencies from the mirror's target values — reflecting a `SetParamPlain` echo
/// immediately, with no audio processed in between (AC-16's "no audio processed in between",
/// exercised here through the JSON lean-slice path rather than the binary `VXRC` frame).
#[test]
fn response_curve_reflects_the_mirror_and_reports_no_support_for_other_modules() {
    let (mut eng, _events) = rig();

    // No slot at all: `Unavailable`? No — out of range is a `Rack` error once the rack exists;
    // before any output device opens it's `Unavailable` (covered by the dedicated test above).
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    let err = eng.response_curve(0, vec![1_000.0]);
    assert!(
        matches!(err, Err(RackApiError::Rack(_))),
        "Gain has no ResponseCurve"
    );

    eng.rack_command(RackCommand::Add {
        module_id: ParametricEq::ID.into(),
        index: 1,
    })
    .unwrap();
    let freqs = vec![20.0, 1_000.0, 20_000.0];
    let curve = eng.response_curve(1, freqs.clone()).unwrap();
    assert_eq!(curve.freqs_hz, freqs);
    assert_eq!(curve.total_db, vec![0.0, 0.0, 0.0], "default EQ is flat");
    assert_eq!(curve.components_db.len(), 9);

    let freq_id = ParametricEq::param_id(2, ParametricEq::FREQ);
    let gain_id = ParametricEq::param_id(2, ParametricEq::GAIN);
    eng.rack_command(RackCommand::SetParamPlain {
        index: 1,
        id: freq_id,
        value: 1_000.0,
    })
    .unwrap();
    eng.rack_command(RackCommand::SetParamPlain {
        index: 1,
        id: gain_id,
        value: 6.0,
    })
    .unwrap();
    let curve = eng.response_curve(1, freqs).unwrap();
    assert!(
        (curve.total_db[1] - 6.0).abs() < 0.01,
        "1 kHz should read ~+6 dB now"
    );
    assert!(
        curve.total_db[0].abs() < curve.total_db[1].abs(),
        "20 Hz, three decades from the peak's centre, should move far less than the centre itself"
    );

    // An oversized request is truncated, not rejected.
    let many: Vec<f64> = (0..600).map(|i| 20.0 + f64::from(i)).collect();
    let curve = eng.response_curve(1, many).unwrap();
    assert_eq!(curve.freqs_hz.len(), vox_engine::MAX_RESPONSE_CURVE_POINTS);
}

/// `rack_model` (H-08 handoff: export builds its offline chain from this instead of
/// `RackModel::default()`) reflects the live rack immediately after a mutating command, with no
/// live-rack case falling back to an empty model.
#[test]
fn rack_model_reflects_the_live_rack() {
    let (mut eng, _events) = rig();
    assert!(eng.rack_model().slots.is_empty());

    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    let model = eng.rack_model();
    assert_eq!(model.slots.len(), 1);
    assert_eq!(
        model.slots[0].module,
        format!("{}@{}", Gain::ID, Gain::VERSION)
    );

    eng.rack_command(RackCommand::Remove { index: 0 }).unwrap();
    assert!(eng.rack_model().slots.is_empty());
}
