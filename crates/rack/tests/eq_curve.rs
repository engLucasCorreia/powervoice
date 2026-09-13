//! S3-07 (SPEC-015 §2.6.6): `RackHost`'s `ResponseCurve` extension access — the same capture-at-
//! `Loaded`-construction pattern S3-06 used for `NoiseProfile`, so the graph can evaluate the
//! module's curve from the control thread without touching the audio-thread instance.

mod common;

use common::*;
use vox_modules::{ParametricEq, ParametricEqFactory};
use vox_rack::{RackHost, RackOptions, Registry};

use std::sync::Arc;

/// A registry with the shared test factories plus the real Parametric EQ module.
fn eq_registry() -> Arc<Registry> {
    let mut factories = test_factories();
    factories.push(Arc::new(ParametricEqFactory::new()));
    Arc::new(Registry::with_factories(factories).unwrap())
}

fn eq_slot() -> vox_rack::SlotModel {
    slot(ParametricEq::ID, &[])
}

fn host_with(registry: Arc<Registry>, m: &vox_rack::RackModel) -> RackHost {
    let (host, _live) = RackHost::new(registry, rt_config(), RackOptions::default(), m).unwrap();
    host
}

#[test]
fn response_curve_extension_exists_only_for_modules_that_implement_it() {
    let registry = eq_registry();
    let m = model(vec![gain_slot(0.0), eq_slot()]);
    let host = host_with(registry, &m);

    // Gain has no `ResponseCurve` extension at all.
    assert!(host.slot_info(0).unwrap().curve_handles.is_none());
    assert!(host.response_curve_extension(0).is_none());

    // The EQ does, with 9 handles (SPEC-015 §3: HP, L, 1..5, H, LP) in band order.
    let handles = host.slot_info(1).unwrap().curve_handles.unwrap();
    assert_eq!(handles.len(), 9);
    assert!(host.response_curve_extension(1).is_some());

    // HP/LP (bands 0 and 8) have no gain/Q handles; the rest do.
    assert_eq!(handles[0].gain, None);
    assert_eq!(handles[0].q, None);
    assert_eq!(handles[8].gain, None);
    assert_eq!(handles[8].q, None);
    for h in &handles[1..8] {
        assert!(h.gain.is_some());
        assert!(h.q.is_some());
    }
    // Every handle has an enable id (SPEC-015 §3 "enable").
    assert!(handles.iter().all(|h| h.enable.is_some()));

    // A placeholder (unknown module) has no extension either.
    let placeholder_model = model(vec![slot("com.acme.unknown", &[])]);
    let host2 = host_with(eq_registry(), &placeholder_model);
    assert!(host2.slot_info(0).unwrap().curve_handles.is_none());
    assert!(host2.response_curve_extension(0).is_none());
}

#[test]
fn response_curve_extension_evaluates_the_mirrors_target_values_from_any_thread() {
    let registry = eq_registry();
    let mut host = host_with(registry, &model(vec![eq_slot()]));

    // Default EQ: every band flat/neutral, so the total response is 0 dB everywhere (SPEC-015
    // §2.2 "defaults are neutral" / AC-2 bit-exact).
    let ext = host.response_curve_extension(0).unwrap();
    let info = host.slot_info(0).unwrap();
    let values: Vec<f64> = info
        .params
        .iter()
        .map(|p| host.param_value(0, p.id).unwrap_or(p.default))
        .collect();
    let freqs = [20.0, 100.0, 1_000.0, 10_000.0, 20_000.0];
    let mut out = [0.0; 5];
    ext.magnitude_db(&values, SR, &freqs, &mut out);
    for db in out {
        assert!(db.abs() < 1e-9, "default EQ should be flat, got {db}");
    }

    // Setting a peak's gain (band 2 = index 2, id 32 per ParametricEq::param_id(2, GAIN)) moves
    // the total curve at its center frequency, and `response_curve_extension` sees the change
    // immediately through the mirror (the module docs' "target values" contract) without any
    // audio being processed.
    let freq_id = ParametricEq::param_id(2, ParametricEq::FREQ);
    let gain_id = ParametricEq::param_id(2, ParametricEq::GAIN);
    host.set_param(0, freq_id, 1_000.0).unwrap();
    host.set_param(0, gain_id, 12.0).unwrap();
    let info = host.slot_info(0).unwrap();
    let values: Vec<f64> = info
        .params
        .iter()
        .map(|p| host.param_value(0, p.id).unwrap_or(p.default))
        .collect();
    let mut out = [0.0; 5];
    ext.magnitude_db(&values, SR, &freqs, &mut out);
    assert!(
        (out[2] - 12.0).abs() < 0.01,
        "expected ~+12 dB at 1 kHz, got {}",
        out[2]
    );
    assert!(
        out[0].abs() < out[2].abs(),
        "20 Hz, far from the peak's 1 kHz centre, should move far less than the centre itself"
    );

    // `component_magnitude_db` gives one band's own response, on or off (SPEC-015 §4.9).
    let mut comp = [0.0; 5];
    ext.component_magnitude_db(2, &values, SR, &freqs, &mut comp);
    assert!((comp[2] - 12.0).abs() < 0.01);
    assert_eq!(ext.component_count(&values), 9);
}
