//! H-03: the True-Peak Limiter inside the rack.
//! - SPEC-017 AC-2 (SPEC-012 AC-10): an offline render keeps an impulse at its input position
//!   (the rack compensates the limiter's L + D + 1 latency).
//! - The generic per-slot telemetry path the rack slot's gain-reduction meter reads
//!   (`SlotInfo::telemetry`, `RackHost::read_telemetry`): descriptions with the slot info, values
//!   from the live instance, and a replaced instance's own cells after a restart.

#![allow(clippy::float_cmp)] // exact telemetry values are the property under test

mod common;

use std::sync::Arc;

use common::*;
use vox_modules::{TruePeakLimiter, TruePeakLimiterFactory};
use vox_rack::offline::render;
use vox_rack::{Registry, SlotModel, TelemetryKind};

vox_module_api::install_test_allocator!();

fn tpl_registry() -> Arc<Registry> {
    let mut factories = test_factories();
    factories.push(Arc::new(TruePeakLimiterFactory::new()));
    Arc::new(Registry::with_factories(factories).unwrap())
}

fn tpl_slot(params: &[(&str, f64)]) -> SlotModel {
    slot(TruePeakLimiter::ID, params)
}

#[test]
fn ac2_offline_render_through_the_rack_keeps_the_impulse_aligned() {
    for lookahead in [1.0, 5.0, 10.0] {
        let mut x = vec![0.0f32; 4_000];
        x[1_000] = 0.25;
        let m = model(vec![tpl_slot(&[("lookahead_ms", lookahead)])]);
        let y = render(&tpl_registry(), &m, SR, &x).unwrap();
        assert_eq!(y.len(), x.len());
        for (i, v) in y.iter().enumerate() {
            let want: f32 = if i == 1_000 { 0.25 } else { 0.0 };
            assert_eq!(
                v.to_bits(),
                want.to_bits(),
                "look-ahead {lookahead} ms, sample {i}"
            );
        }
    }
}

#[test]
fn slot_telemetry_reads_the_gain_reduction_and_follows_a_restart() {
    let m = model(vec![
        gain_slot(0.0),
        tpl_slot(&[("input_gain_db", 12.0)]),
        slot("com.acme.unknown", &[]),
    ]);
    let len = 96_000;
    let mut d = Driver::with_registry(tpl_registry(), &m, 0x7E1E, len);

    // Channel descriptions travel with the slot info; a placeholder has none.
    let info = d.host.slot_info(1).unwrap();
    assert_eq!(info.telemetry.len(), 1);
    assert_eq!(info.telemetry[0].key, "gain_reduction_db");
    assert_eq!(info.telemetry[0].kind, TelemetryKind::GainReduction);
    assert_eq!((info.telemetry[0].min, info.telemetry[0].max), (-24.0, 0.0));
    assert!(d.host.slot_info(2).unwrap().telemetry.is_empty());
    let limiter_value = |d: &Driver| {
        let all = d.host.read_telemetry();
        assert!(
            all.iter().all(|s| s.uid != d.host.slot_uid(2).unwrap()),
            "placeholders have no telemetry"
        );
        all.iter()
            .find(|s| s.uid == info.uid)
            .expect("the limiter's values")
            .values[0]
    };

    // A 0 dBFS tone pushed +12 dB into −1 dBTP: ≈ 13 dB of gain reduction.
    let w = std::f64::consts::TAU * 997.0 / SR;
    let x: Vec<f32> = (0..len).map(|n| (w * n as f64).sin() as f32).collect();
    d.run_until(&x, 24_000);
    let gr = limiter_value(&d);
    assert!((-14.0..=-12.0).contains(&gr), "gain reduction {gr} dB");

    // A restart replaces the instance: the host now reads the new instance's cells (0 dB until
    // it processes audio), not the old instance's held value.
    d.host.restart(1).unwrap();
    assert_eq!(limiter_value(&d), 0.0);
    d.run_until(&x, len);
    let gr = limiter_value(&d);
    assert!(
        (-14.0..=-12.0).contains(&gr),
        "after the restart: gain reduction {gr} dB"
    );
}
