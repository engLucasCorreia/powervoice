//! H-101 (SPEC-015 §2.6.3 amendment): `EngineHandle::response_curve_preview`/
//! `ManualEngine::response_curve_preview` evaluate a module's response curve from parameters
//! alone, with **no rack slot and no mutation** — the wall H-92 hit trying to draw the Explain
//! modal's dashed EQ-suggestion overlay (`Control::response_curve` needs a live slot). The point
//! of doing it this way is that a preview and the applied result can never disagree, because
//! both run through the exact same `ResponseCurve` extension; these tests prove that by
//! comparing a preview against a real, live Parametric EQ slot set up with the same parameter
//! changes.

#![allow(clippy::float_cmp)] // exact values, not measurements

use std::sync::Arc;

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{EngineConfig, HostId, ManualEngine, RackApiError, RackCommand};
use vox_modules::{Gain, ParametricEq};
use vox_rack::Registry;

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn rig() -> ManualEngine {
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    eng
}

/// The money test: a preview built from parameters alone, with no rack slot at all, reads
/// exactly like a real live Parametric EQ slot set up with the same parameter changes.
#[test]
fn preview_matches_a_real_live_slot_with_the_same_parameter_changes() {
    let mut eng = rig();
    eng.rack_command(RackCommand::Add {
        module_id: ParametricEq::ID.into(),
        index: 0,
    })
    .unwrap();
    // Turn on peak band 1 (band index 2) at 1 kHz, Q 1, +6 dB — the shape
    // `eqSuggest.ts::planEqAction` sends for a boost/cut suggestion.
    let overrides = vec![
        (ParametricEq::param_id(2, ParametricEq::ON), 1.0),
        (ParametricEq::param_id(2, ParametricEq::FREQ), 1_000.0),
        (ParametricEq::param_id(2, ParametricEq::GAIN), 6.0),
        (ParametricEq::param_id(2, ParametricEq::Q), 1.0),
    ];
    for (id, value) in &overrides {
        eng.rack_command(RackCommand::SetParamPlain {
            index: 0,
            id: *id,
            value: *value,
        })
        .unwrap();
    }
    let freqs_hz = vec![100.0, 500.0, 1_000.0, 2_000.0, 8_000.0];

    let applied = eng.response_curve(0, freqs_hz.clone()).unwrap();

    let snapshot_before = eng.rack_snapshot();
    let preview = eng
        .response_curve_preview(ParametricEq::ID.into(), overrides, freqs_hz.clone())
        .unwrap();
    let snapshot_after = eng.rack_snapshot();

    assert_eq!(preview.freqs_hz, applied.freqs_hz);
    assert_eq!(preview.sample_rate_hz, applied.sample_rate_hz);
    assert_eq!(preview.total_db, applied.total_db);
    assert_eq!(preview.components_db, applied.components_db);

    // No rack mutation: the live rack (still holding the one EQ slot added above) is completely
    // unchanged by the preview call.
    assert_eq!(snapshot_before.slots.len(), snapshot_after.slots.len());
    assert_eq!(
        snapshot_before.slots[0].values,
        snapshot_after.slots[0].values
    );
}

/// A preview never needs a live rack at all: called against a freshly polled engine with an
/// empty rack (no `Add` ever sent), it still answers — and the rack stays empty.
#[test]
fn preview_needs_no_rack_slot() {
    let eng = rig();
    assert_eq!(eng.rack_snapshot().slots.len(), 0);

    let preview = eng
        .response_curve_preview(ParametricEq::ID.into(), vec![], vec![1_000.0])
        .unwrap();
    assert_eq!(preview.total_db.len(), 1);
    assert!(preview.total_db[0].abs() < 0.001, "defaults are neutral");

    assert_eq!(eng.rack_snapshot().slots.len(), 0, "still no slot");
}

/// A module with no `ResponseCurve` extension (Gain) previews as `NoExtension`, the same error
/// `transfer_curve` already uses for a missing extension.
#[test]
fn preview_of_a_module_with_no_response_curve_extension_is_no_extension() {
    let eng = rig();
    let err = eng
        .response_curve_preview(Gain::ID.into(), vec![], vec![1_000.0])
        .unwrap_err();
    assert!(matches!(err, RackApiError::NoExtension));
}

/// An unknown module id is rejected, not a panic.
#[test]
fn preview_of_an_unknown_module_id_is_rejected() {
    let eng = rig();
    let err = eng
        .response_curve_preview(
            "org.powervoice.does-not-exist".into(),
            vec![],
            vec![1_000.0],
        )
        .unwrap_err();
    assert!(matches!(err, RackApiError::Rack(_)));
}
