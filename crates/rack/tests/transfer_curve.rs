//! H-63 (SPEC-016 §4.11, SPEC-013 §4.3): `RackHost`'s `TransferCurve` extension access — the
//! same capture-at-`Loaded`-construction pattern S3-06/S3-07 used for `NoiseProfile` and
//! `ResponseCurve`, so the transfer graph can evaluate the module's curve from the control thread
//! without touching the audio-thread instance.

#![allow(clippy::float_cmp)] // −inf and the exact branch equality are the point here

mod common;

use std::sync::Arc;

use common::*;
use vox_modules::{Dynamics, DynamicsFactory, NoiseGate, NoiseGateFactory};
use vox_rack::{CurveBranch, RackHost, RackOptions, Registry};

/// A registry with the shared test factories plus the two modules that answer the extension.
fn dyn_registry() -> Arc<Registry> {
    let mut factories = test_factories();
    factories.push(Arc::new(DynamicsFactory::new()));
    factories.push(Arc::new(NoiseGateFactory::new()));
    Arc::new(Registry::with_factories(factories).unwrap())
}

fn host_with(m: &vox_rack::RackModel) -> RackHost {
    let (host, _live) =
        RackHost::new(dyn_registry(), rt_config(), RackOptions::default(), m).unwrap();
    host
}

#[test]
fn transfer_curve_extension_exists_only_for_modules_that_implement_it() {
    let m = model(vec![
        gain_slot(0.0),
        slot(Dynamics::ID, &[]),
        slot(NoiseGate::ID, &[]),
    ]);
    let host = host_with(&m);

    // Gain has no `TransferCurve` extension at all.
    assert!(host.slot_info(0).unwrap().transfer_handles.is_none());
    assert!(host.transfer_curve_extension(0).is_none());

    // Dynamics: one handle per section, in processing order, each with its enable (§4.11).
    let handles = host.slot_info(1).unwrap().transfer_handles.unwrap();
    assert_eq!(handles.len(), 4);
    assert_eq!(handles[0].threshold, Dynamics::AUTOGATE_THRESHOLD_DB);
    assert_eq!(handles[3].threshold, Dynamics::LIMITER_THRESHOLD_DB);
    assert!(handles.iter().all(|h| h.enable.is_some()));
    assert!(host.transfer_curve_extension(1).is_some());

    // Noise Gate: one handle, no enable (the gate is always on, SPEC-013 §4.3).
    let handles = host.slot_info(2).unwrap().transfer_handles.unwrap();
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].threshold, NoiseGate::THRESHOLD_DB);
    assert_eq!(handles[0].enable, None);

    // A placeholder (unknown module) has no extension either.
    let host2 = host_with(&model(vec![slot("com.acme.unknown", &[])]));
    assert!(host2.slot_info(0).unwrap().transfer_handles.is_none());
    assert!(host2.transfer_curve_extension(0).is_none());
}

#[test]
fn transfer_curve_extension_evaluates_the_mirrors_target_values_from_any_thread() {
    let mut host = host_with(&model(vec![slot(Dynamics::ID, &[])]));
    let ext = host.transfer_curve_extension(0).unwrap();
    let values = |host: &RackHost| -> Vec<f64> {
        let info = host.slot_info(0).unwrap();
        info.params
            .iter()
            .map(|p| host.param_value(0, p.id).unwrap_or(p.default))
            .collect()
    };

    // The default Dynamics has only the compressor on, at −20 dBFS / 3:1 with a 6 dB knee and no
    // makeup, so −40 dBFS passes untouched and −10 dBFS is pulled down.
    let levels = [-40.0, -20.0, -10.0];
    let mut out = [0.0; 3];
    ext.output_dbfs(&values(&host), CurveBranch::Rising, &levels, &mut out);
    assert!((out[0] - levels[0]).abs() < 1e-9, "below the knee: {out:?}");
    assert!(out[2] < levels[2] - 3.0, "above the threshold: {out:?}");
    // No hysteresis while the AutoGate is off; both branches agree.
    assert!(!ext.has_hysteresis(&values(&host)));

    // A `set_param` on the control thread is visible immediately, with no audio processed.
    host.set_param(0, Dynamics::COMPRESSOR_MAKEUP_DB, 6.0)
        .unwrap();
    let mut with_makeup = [0.0; 3];
    ext.output_dbfs(
        &values(&host),
        CurveBranch::Rising,
        &levels,
        &mut with_makeup,
    );
    assert!(
        (with_makeup[0] - (out[0] + 6.0)).abs() < 1e-9,
        "makeup lifts the whole curve: {with_makeup:?} vs {out:?}"
    );

    // Turning the AutoGate on introduces the hysteresis loop: between T − 3 and T the branches
    // differ (SPEC-016 §4.11).
    host.set_param(0, Dynamics::AUTOGATE_ENABLED, 1.0).unwrap();
    host.set_param(0, Dynamics::AUTOGATE_THRESHOLD_DB, -30.0)
        .unwrap();
    let v = values(&host);
    assert!(ext.has_hysteresis(&v));
    let probes = [-40.0, -31.0, -20.0];
    let (mut rising, mut falling) = ([0.0; 3], [0.0; 3]);
    ext.output_dbfs(&v, CurveBranch::Rising, &probes, &mut rising);
    ext.output_dbfs(&v, CurveBranch::Falling, &probes, &mut falling);
    assert_eq!(rising[0], f64::NEG_INFINITY, "closed below T on both");
    assert_eq!(falling[0], f64::NEG_INFINITY);
    assert_eq!(rising[1], f64::NEG_INFINITY, "−31 dBFS is below T");
    assert!(falling[1].is_finite(), "…but above T − 3, so still open");
    assert_eq!(rising[2], falling[2], "above T both branches are open");
}
