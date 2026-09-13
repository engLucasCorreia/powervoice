//! SPEC-012 AC-7: no lost or duplicated values (coalescing in place, carry-over), plus the
//! live-rack side of parameter routing (mirror = last value sent).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{ParamEvent, ParamId, Transport};
use vox_modules::Gain;
use vox_rack::{Chain, RackNotice, RackOptions};

vox_module_api::install_test_allocator!();

fn ev(offset: u32, id: ParamId, value: f64) -> ParamEvent {
    ParamEvent { offset, id, value }
}

fn logged(log: &Log) -> Vec<(u64, u32, f64)> {
    log.lock()
        .unwrap()
        .iter()
        .map(|e| (e.0, e.1.0, e.2))
        .collect()
}

#[test]
fn same_offset_same_id_coalesces_in_place() {
    let (g, h) = (Probe::VALUE, Probe::OTHER);
    let mut chain = Chain::new();
    let (probe, log) = probe_with_log(0);
    chain.push(probe).unwrap();
    chain.activate(&cfg(1024)).unwrap();
    no_alloc(|| {
        for e in [
            ev(100, g, -6.0),
            ev(100, g, -12.0),
            ev(100, h, 1.0),
            ev(100, g, -3.0),
        ] {
            chain.push_event(0, e).unwrap();
        }
    })
    .unwrap();
    let x = vec![0.0f32; 1024];
    let mut y = vec![0.0f32; 1024];
    no_alloc(|| chain.process(Transport::default(), &x, &mut y)).unwrap();
    chain.deactivate();
    assert_eq!(logged(&log), vec![(100, 0, -3.0), (100, 1, 1.0)]);
}

#[test]
fn ten_thousand_pushes_at_one_offset_never_fill_the_queue() {
    let g = Probe::VALUE;
    let mut chain = Chain::new();
    let (probe, log) = probe_with_log(0);
    chain.push(probe).unwrap();
    chain.activate(&cfg(1024)).unwrap();
    no_alloc(|| {
        for i in 1..=10_000 {
            chain.push_event(0, ev(0, g, f64::from(i))).unwrap();
        }
    })
    .unwrap();
    let x = vec![0.0f32; 64];
    let mut y = vec![0.0f32; 64];
    chain.process(Transport::default(), &x, &mut y);
    chain.deactivate();
    assert_eq!(logged(&log), vec![(0, 0, 10_000.0)]);
}

#[test]
fn two_thousand_events_with_capacity_512_all_arrive_in_order() {
    let g = Probe::VALUE;
    let mut chain = Chain::with_options(RackOptions {
        event_capacity: 512,
        queue_capacity: 2048,
    });
    let (probe, log) = probe_with_log(0);
    chain.push(probe).unwrap();
    chain.activate(&cfg(1024)).unwrap();
    for i in 0..2000u32 {
        chain.push_event(0, ev(i, g, f64::from(i) + 0.5)).unwrap();
    }
    let x = vec![0.0f32; 2000];
    let mut y = vec![0.0f32; 2000];
    // Sub-blocks of 1024: 512 per module call; the rest carries over (offset 0 of the next
    // sub-block / call) and nothing is merged or dropped.
    for _ in 0..3 {
        no_alloc(|| chain.process(Transport::default(), &x, &mut y)).unwrap();
    }
    assert_eq!(chain.module(0).unwrap().param_value(g), Some(1999.5));
    chain.deactivate();
    let values: Vec<f64> = logged(&log).iter().map(|e| e.2).collect();
    let expected: Vec<f64> = (0..2000).map(|i| f64::from(i) + 0.5).collect();
    assert_eq!(values, expected);
    // Positions never go backwards, and no event arrives before its sample.
    let pos: Vec<u64> = logged(&log).iter().map(|e| e.0).collect();
    assert!(pos.windows(2).all(|w| w[0] <= w[1]));
    assert!(pos.iter().enumerate().all(|(i, &p)| p >= i as u64));
}

/// Live rack: many UI changes between two blocks reach the module as the latest value, the
/// mirror equals the last value sent, and every change is echoed.
#[test]
fn live_param_changes_end_at_the_last_value_sent() {
    let x = white(3, 48_000);
    let mut d = Driver::new(&model(vec![gain_slot(0.0)]), 5, x.len());
    d.run_until(&x, 10_000);
    for i in 0..3000 {
        d.host
            .set_param(0, Gain::GAIN_DB, -(f64::from(i % 60)))
            .unwrap();
    }
    let last = d.host.set_param(0, Gain::GAIN_DB, -7.25).unwrap();
    d.run_to_end(&x);
    assert_eq!(d.host.param_value(0, Gain::GAIN_DB), Some(-7.25));
    assert_eq!(last, -7.25);
    let echoes = d
        .notices
        .iter()
        .filter(|(_, n)| matches!(n, RackNotice::ParamChanged { .. }))
        .count();
    assert_eq!(
        echoes, 1,
        "param_changed is coalesced per (slot, id) per tick"
    );
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::ParamChanged { value, .. } if *value == -7.25))
    );
    // The module applies exactly -7.25 dB after the ramp.
    let g = 10f64.powf(-7.25 / 20.0);
    for n in 20_000..x.len() {
        let got = f64::from(d.out[n]);
        assert!((got - f64::from(x[n]) * g).abs() <= 1e-6, "sample {n}");
    }
}
