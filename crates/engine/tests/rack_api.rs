//! S3-01: rack commands round-trip through the engine on the fake backend (SPEC-012 §2.1, §2.4,
//! §2.6). The generic parameter UI is driven entirely by these commands and the `RackSnapshot`
//! they return, so this test exercises the same surface `src-tauri`'s IPC commands wrap.

vox_module_api::install_test_allocator!();

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{
    EngineConfig, EngineEvent, HostId, ManualEngine, ModuleTelemetryFrame, RackApiError,
    RackCommand,
};
use vox_modules::{Dynamics, Gain, NoiseGate, NoiseReduction, ParametricEq, TruePeakLimiter};
use vox_rack::{NoiseProfileStatus, RackNotice, Registry, SlotStatus};

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
    // H-43: the engine is idle (stopped), so the second tick's identical values aren't resent.
    assert_eq!(frames.len(), 1, "{} frames", frames.len());
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

/// H-63 (SPEC-016 §4.11): `transfer_curve` evaluates the slot's `TransferCurve` extension over
/// the requested level range from the mirror's target values, reports `no support` for a module
/// without it, rejects a bad range and clamps an oversized point count.
#[test]
fn transfer_curve_reflects_the_mirror_and_reports_no_support_for_other_modules() {
    let (mut eng, _events) = rig();

    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    assert!(
        matches!(
            eng.transfer_curve(0, -80.0, 6.0, 8),
            Err(RackApiError::NoExtension)
        ),
        "Gain has no TransferCurve"
    );

    eng.rack_command(RackCommand::Add {
        module_id: Dynamics::ID.into(),
        index: 1,
    })
    .unwrap();
    let curve = eng.transfer_curve(1, -80.0, 6.0, 5).unwrap();
    assert_eq!(curve.in_dbfs, vec![-80.0, -58.5, -37.0, -15.5, 6.0]);
    assert_eq!(curve.rising_db.len(), 5);
    assert_eq!(curve.components_db.len(), 4, "one row per section");
    assert!(
        curve.falling_db.is_none(),
        "the AutoGate is off by default, so there is no hysteresis loop"
    );
    // The default rack slot: only the compressor is on, at −20 dBFS, so the low levels pass.
    assert!((curve.rising_db[0] - (-80.0)).abs() < 1e-9);
    assert!(curve.rising_db[4] < 6.0 - 3.0, "+6 dBFS is compressed");

    // Handles: one per section, with the enable state and the detector offset already applied.
    assert_eq!(curve.handles.len(), 4);
    let compressor = curve.handles[2];
    assert_eq!(compressor.param, Dynamics::COMPRESSOR_THRESHOLD_DB);
    assert!(compressor.enabled, "the compressor is on by default");
    assert!(!curve.handles[0].enabled, "the AutoGate is off by default");
    // Detection defaults to RMS, so the compressor handle sits 3.0103 dB above its threshold.
    assert!((compressor.offset_db - 3.010_299_956_639_812).abs() < 1e-9);
    assert!((compressor.x_dbfs - (-20.0 + compressor.offset_db)).abs() < 1e-9);

    // A `SetParamPlain` echo is visible with no audio processed in between.
    eng.rack_command(RackCommand::SetParamPlain {
        index: 1,
        id: Dynamics::COMPRESSOR_THRESHOLD_DB,
        value: -30.0,
    })
    .unwrap();
    let moved = eng.transfer_curve(1, -80.0, 6.0, 5).unwrap();
    assert!((moved.handles[2].x_dbfs - (-30.0 + compressor.offset_db)).abs() < 1e-9);
    assert!(
        moved.rising_db[4] < curve.rising_db[4],
        "a lower threshold compresses +6 dBFS harder"
    );

    // Enabling the AutoGate adds the Falling branch and mutes below the threshold.
    eng.rack_command(RackCommand::SetParamPlain {
        index: 1,
        id: Dynamics::AUTOGATE_ENABLED,
        value: 1.0,
    })
    .unwrap();
    let gated = eng.transfer_curve(1, -80.0, 6.0, 5).unwrap();
    assert!(gated.falling_db.is_some());
    assert!(
        gated.rising_db[0] == f64::NEG_INFINITY,
        "a closed gate mutes: the level is −inf (H-77, `VXTC` carries it), got {}",
        gated.rising_db[0]
    );
    assert!(gated.handles[0].enabled);

    // A non-increasing or non-finite range is rejected; an oversized point count is clamped.
    for (lo, hi) in [(6.0, -80.0), (0.0, 0.0), (f64::NAN, 6.0)] {
        assert!(matches!(
            eng.transfer_curve(1, lo, hi, 8),
            Err(RackApiError::Rack(_))
        ));
    }
    let many = eng.transfer_curve(1, -80.0, 6.0, 4_000).unwrap();
    assert_eq!(many.in_dbfs.len(), vox_engine::MAX_TRANSFER_CURVE_POINTS);
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

/// Decodes a `VXTC` frame the way the UI's `decodeVxtc` does (H-77, SPEC-016 §4.12), so the
/// Rust-side tests assert the same field layout the TS golden fixture pins.
struct Vxtc {
    seq: u32,
    flags: u32,
    x_min_db: f32,
    x_max_db: f32,
    rising: Vec<f32>,
    falling: Option<Vec<f32>>,
    components: Vec<Vec<f32>>,
    handles: Vec<(u32, f32, f32, u32)>,
}

fn decode_vxtc(bytes: &[u8]) -> Vxtc {
    let u16at = |o: usize| u16::from_le_bytes(bytes[o..o + 2].try_into().unwrap());
    let u32at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let f32at = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    assert_eq!(&bytes[0..4], b"VXTC");
    assert_eq!(u16at(4), 1, "version");
    let header_len = usize::from(u16at(6));
    assert_eq!(header_len, vox_engine::VXTC_HEADER_LEN);
    let flags = u32at(12);
    let points = u32at(24) as usize;
    let components = u32at(28) as usize;
    let handles = u32at(32) as usize;
    assert_eq!(u32at(36), 0, "reserved");
    let mut o = header_len;
    let take = |o: &mut usize| {
        let v: Vec<f32> = (0..points).map(|i| f32at(*o + 4 * i)).collect();
        *o += 4 * points;
        v
    };
    let rising = take(&mut o);
    let falling = (flags & vox_engine::VXTC_HAS_FALLING != 0).then(|| take(&mut o));
    let components: Vec<Vec<f32>> = (0..components).map(|_| take(&mut o)).collect();
    let handles: Vec<(u32, f32, f32, u32)> = (0..handles)
        .map(|k| {
            let base = o + 16 * k;
            (
                u32at(base),
                f32at(base + 4),
                f32at(base + 8),
                u32at(base + 12),
            )
        })
        .collect();
    assert_eq!(o + 16 * handles.len(), bytes.len(), "no trailing bytes");
    Vxtc {
        seq: u32at(8),
        flags,
        x_min_db: f32at(16),
        x_max_db: f32at(20),
        rising,
        falling,
        components,
        handles,
    }
}

/// H-77 (SPEC-016 §4.12, AC-23): the transfer curve travels as a binary `VXTC` frame — every
/// field in the spec's layout, `HAS_FALLING` only with a hysteresis loop, −∞ for a muted level —
/// and 512 points are evaluated and framed well inside the 5 ms budget.
///
/// The exact float comparisons are wire-format checks against Rust's own encoder (the values
/// round-trip bit for bit), not measurements.
#[allow(clippy::float_cmp)]
#[test]
fn transfer_curve_encodes_a_vxtc_frame_within_the_time_budget() {
    let (mut eng, _events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Dynamics::ID.into(),
        index: 0,
    })
    .unwrap();

    let points = eng.transfer_curve(0, -80.0, 6.0, 9).unwrap();
    let frame = decode_vxtc(&points.encode(42));
    assert_eq!(frame.seq, 42, "the request's seq is echoed");
    assert_eq!(frame.flags, 0, "no hysteresis with the AutoGate off");
    assert!(frame.falling.is_none());
    assert_eq!(frame.x_min_db, -80.0);
    assert_eq!(frame.x_max_db, 6.0);
    assert_eq!(frame.rising.len(), 9);
    assert_eq!(frame.components.len(), 4, "one row per section");
    assert_eq!(frame.handles.len(), 4);
    for (i, &x) in frame.rising.iter().enumerate() {
        assert!(
            (f64::from(x) - points.rising_db[i]).abs() < 1e-3,
            "level {i} survives the f32 round trip"
        );
    }
    for (row, expected) in frame.components.iter().zip(&points.components_db) {
        for (i, &x) in row.iter().enumerate() {
            assert!((f64::from(x) - expected[i]).abs() < 1e-3);
        }
    }
    let compressor = frame.handles[2];
    assert_eq!(compressor.0, Dynamics::COMPRESSOR_THRESHOLD_DB.0);
    assert!((f64::from(compressor.1) - points.handles[2].x_dbfs).abs() < 1e-3);
    assert!((f64::from(compressor.2) - points.handles[2].offset_db).abs() < 1e-3);
    assert_eq!(
        compressor.3,
        vox_engine::VXTC_HANDLE_ENABLED,
        "on by default"
    );
    assert_eq!(frame.handles[0].3, 0, "the AutoGate is off by default");

    // The AutoGate's hysteresis adds the Falling branch, and its closed region is −∞.
    eng.rack_command(RackCommand::SetParamPlain {
        index: 0,
        id: Dynamics::AUTOGATE_ENABLED,
        value: 1.0,
    })
    .unwrap();
    // 1 dB apart, so the 3 dB hysteresis band around the −50 dBFS threshold is sampled.
    let gated = eng.transfer_curve(0, -80.0, 6.0, 87).unwrap().encode(43);
    let gated = decode_vxtc(&gated);
    assert_eq!(gated.flags, vox_engine::VXTC_HAS_FALLING);
    let falling = gated.falling.expect("the Falling branch is present");
    assert_eq!(falling.len(), 87);
    assert_eq!(
        gated.rising[0],
        f32::NEG_INFINITY,
        "a closed gate mutes: −∞, never NaN"
    );
    assert!(
        gated.rising.iter().chain(&falling).all(|v| !v.is_nan()),
        "no NaN reaches the UI"
    );
    assert!(
        falling.iter().zip(&gated.rising).any(|(f, r)| f != r),
        "the hysteresis loop differs from the Rising branch"
    );

    // AC-23: `module_transfer_curve` answers within 5 ms for 512 points. Measured over 20 runs
    // so one scheduling hiccup on a loaded CI box doesn't fail the build; a debug build is well
    // inside the budget already.
    let mut worst = std::time::Duration::ZERO;
    for seq in 0..20 {
        let start = std::time::Instant::now();
        let bytes = eng.transfer_curve(0, -80.0, 6.0, 512).unwrap().encode(seq);
        worst = worst.max(start.elapsed());
        assert_eq!(bytes.len(), 40 + 4 * 512 * (2 + 4) + 16 * 4);
    }
    assert!(
        worst < std::time::Duration::from_millis(5),
        "512 points took {worst:?}, over SPEC-016 §4.12's 5 ms budget"
    );
}

/// Seeded white noise in `[-amp, amp)` (xorshift64*, no extra test dependency — mirrors
/// `nr_capture.rs`'s helper, kept local since integration test binaries don't share code).
fn white_noise(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            let u = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            amp * (2.0 * u - 1.0) as f32
        })
        .collect()
}

/// H-85 (SPEC-014 §2.8 item 2): `noise_profile_curve` reports no support for a module without
/// the `NoiseProfile` extension, empty points before any print exists, the real `describe()`
/// points once one is captured and applied, and empty again after `ClearNoisePrint` — matching
/// the panel's "empty with no print" rule (§2.8).
#[test]
fn noise_profile_curve_tracks_the_committed_print() {
    let (mut eng, _events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    let err = eng.noise_profile_curve(0);
    assert!(
        matches!(err, Err(RackApiError::NoExtension)),
        "Gain has no NoiseProfile extension"
    );

    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 1,
    })
    .unwrap();
    let empty = eng.noise_profile_curve(1).unwrap();
    assert!(empty.freqs_hz.is_empty(), "no print yet");
    assert!(empty.levels_dbfs.is_empty());

    let prep = eng.nr_capture_prepare(Some(1)).unwrap();
    assert_eq!(prep.index, 1);
    assert!(!prep.inserted, "the slot already existed");
    let noise = white_noise(3, 0.01, 48_000);
    let blob = prep
        .extension
        .capture(&noise, 48_000.0, &prep.values, &AtomicBool::new(false))
        .unwrap();
    let snap = eng.nr_capture_apply(1, blob).unwrap();
    assert_eq!(
        snap.slots[1].info.noise_profile,
        Some(NoiseProfileStatus::Loaded)
    );

    let curve = eng.noise_profile_curve(1).unwrap();
    assert_eq!(curve.freqs_hz.len(), curve.levels_dbfs.len());
    assert!(!curve.freqs_hz.is_empty(), "the print now describes points");
    assert!(
        curve.freqs_hz.windows(2).all(|w| w[0] < w[1]),
        "band centres rise monotonically (SPEC-007 §4.10's ladder)"
    );
    assert!(
        curve
            .levels_dbfs
            .iter()
            .all(|d| d.is_finite() || *d == f64::NEG_INFINITY),
        "no NaN reaches the UI"
    );

    let cleared = eng
        .rack_command(RackCommand::ClearNoisePrint { index: 1 })
        .unwrap();
    assert_eq!(
        cleared.slots[1].info.noise_profile,
        Some(NoiseProfileStatus::None)
    );
    let after_clear = eng.noise_profile_curve(1).unwrap();
    assert!(
        after_clear.freqs_hz.is_empty(),
        "Clear Noise Print empties the graph"
    );
}
