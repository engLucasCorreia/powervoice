//! Offline render: SPEC-012 AC-9 (realtime equals offline, deterministic), AC-10 (alignment),
//! AC-3/AC-4 offline parts, AC-14 offline part, missing modules, automation carry-over.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use common::*;
use vox_module_api::test_util::TestGain;
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventList, Module, OutputEvents, ParamEvent, ProcessContext,
    ProcessMode, Transport, prepare_state,
};
use vox_modules::Gain;
use vox_rack::offline::{AutomationEvent, RenderError, render, render_with_automation};
use vox_rack::{RackHost, RackOptions};
use vox_testkit::golden::fnv1a_hash;
use vox_testkit::signal;

vox_module_api::install_test_allocator!();

fn ac9_input() -> Vec<f32> {
    let mut x = signal::pink_noise(3, -20.0, 10.0, 48_000).unwrap();
    x.extend(signal::log_sweep(20.0, 20_000.0, -20.0, 2.0, 48_000).unwrap());
    x
}

fn ac9_model() -> vox_rack::RackModel {
    model(vec![gain_slot(-6.0), delay_slot(480), gain_slot(3.0)])
}

fn ac9_automation() -> Vec<(usize, usize, f64)> {
    // (slot, position, gain dB): 20 events at fixed absolute positions.
    (0..20)
        .map(|i| {
            let slot = if i % 2 == 0 { 0 } else { 2 };
            (slot, 7_919 + i * 27_431, -(((i * 7) % 30) as f64) + 2.5)
        })
        .collect()
}

fn realtime(input: &[f32], automation: &[(usize, usize, f64)], seed: u64) -> Vec<f32> {
    let mut d = Driver::new(&ac9_model(), seed, input.len());
    for &(slot, position, value) in automation {
        d.automation.push(RtEvent {
            position,
            slot: d.host.slot_uid(slot).unwrap(),
            id: Gain::GAIN_DB,
            value,
        });
    }
    d.run_to_end(input);
    d.out
}

#[test]
fn ac9_realtime_equals_offline_static_and_automated() {
    let x = ac9_input();
    let reg = registry();
    for automated in [false, true] {
        let auto: Vec<(usize, usize, f64)> = if automated {
            ac9_automation()
        } else {
            Vec::new()
        };
        let events: Vec<AutomationEvent> = auto
            .iter()
            .map(|&(slot, position, value)| AutomationEvent {
                slot,
                position: position as u64,
                id: Gain::GAIN_DB,
                value,
            })
            .collect();
        let off = render_with_automation(&reg, &ac9_model(), SR, &x, &events).unwrap();
        assert_eq!(off.len(), x.len());
        let rt = realtime(&x, &auto, 31 + u64::from(automated));
        let d = max_abs_diff(&rt[480..], &off[..x.len() - 480]);
        println!("AC-9 automated={automated}: max |realtime - offline| = {d:e}");
        assert!(d <= 1e-6, "automated={automated}: {d}");
        let again = render_with_automation(&reg, &ac9_model(), SR, &x, &events).unwrap();
        assert_eq!(fnv1a_hash(&off), fnv1a_hash(&again));
    }
}

#[test]
fn ac10_offline_render_is_time_aligned_and_same_length() {
    let x = signal::impulse(0.0, 1000, 1.0, 48_000).unwrap();
    assert_eq!(x.len(), 48_000);
    let y = render(&registry(), &model(vec![delay_slot(480)]), SR, &x).unwrap();
    assert_eq!(y.len(), 48_000);
    let peak = y
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 1000);
    assert_eq!(y[1000], 1.0);
    assert_eq!(y.iter().filter(|s| **s != 0.0).count(), 1);
}

/// H-03 regression: when the latency flush runs past a block boundary after the input's end, a
/// block starts beyond `input.len()` (4 000 samples + 480 latency: the second 4 096 block starts at
/// 4 096) — the render must not slice the input there.
#[test]
fn ac10_short_input_whose_latency_flush_crosses_a_block_boundary() {
    let x = signal::impulse(0.0, 1000, 4_000.0 / 48_000.0, 48_000).unwrap();
    assert_eq!(x.len(), 4_000);
    let y = render(&registry(), &model(vec![delay_slot(480)]), SR, &x).unwrap();
    assert_eq!(y.len(), 4_000);
    assert_eq!(y[1000], 1.0);
    assert_eq!(y.iter().filter(|s| **s != 0.0).count(), 1);
}

#[test]
fn ac3_offline_bypassed_slot_equals_the_rack_without_it() {
    let x = white(1, 48_000);
    let reg = registry();
    let with = render(
        &reg,
        &model(vec![gain_slot(-6.0), bypassed(delay_slot(480))]),
        SR,
        &x,
    )
    .unwrap();
    let without = render(&reg, &model(vec![gain_slot(-6.0)]), SR, &x).unwrap();
    assert!(max_abs_diff(&with, &without) <= 1e-6);
    let with = render(
        &reg,
        &model(vec![bypassed(gain_slot(-12.0)), delay_slot(480)]),
        SR,
        &x,
    )
    .unwrap();
    let without = render(&reg, &model(vec![delay_slot(480)]), SR, &x).unwrap();
    assert!(max_abs_diff(&with, &without) <= 1e-6);
}

#[test]
fn ac4_offline_render_ignores_ab() {
    let x = white(2, 48_000);
    let m = model(vec![delay_slot(480), gain_slot(-6.0)]);
    let (mut host, _live) =
        RackHost::new(registry(), rt_config(), RackOptions::default(), &m).unwrap();
    let off = render(&registry(), &host.model(), SR, &x).unwrap();
    host.set_ab(true);
    let on = render(&registry(), &host.model(), SR, &x).unwrap();
    assert_eq!(fnv1a_hash(&off), fnv1a_hash(&on));
}

#[test]
fn ac14_offline_render_aborts_naming_the_slot() {
    let x = white(3, 48_000);
    let e = render(
        &registry(),
        &model(vec![gain_slot(0.0), slot("org.powervoice.test-nan", &[])]),
        SR,
        &x,
    )
    .unwrap_err();
    assert!(matches!(e, RenderError::SlotFailed { index: 1, .. }), "{e}");
    assert_eq!(e.to_string(), "slot 2 (Test NaN) produced invalid audio");
}

#[test]
fn missing_modules_are_an_error_listing_the_ids() {
    let e = render(
        &registry(),
        &model(vec![
            gain_slot(0.0),
            slot("com.acme.x", &[]),
            slot("com.acme.y", &[]),
        ]),
        SR,
        &[0.0; 16],
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "unknown module id(s): com.acme.x@1.0.0, com.acme.y@1.0.0"
    );
}

/// 1500 events inside one 4096 block overflow the per-slot queue; the render cuts the block so
/// every event keeps its exact sample position.
#[test]
fn dense_automation_keeps_exact_positions() {
    let mut reg = vox_rack::Registry::with_factories(test_factories()).unwrap();
    reg.register(std::sync::Arc::new(
        vox_module_api::test_util::TestGainFactory::new(),
    ))
    .unwrap();
    let x = white(4, 6000);
    let events: Vec<AutomationEvent> = (0..1500u64)
        .map(|i| AutomationEvent {
            slot: 0,
            position: 100 + 2 * i,
            id: TestGain::GAIN_DB,
            value: -((i % 40) as f64),
        })
        .collect();
    let m = model(vec![slot(TestGain::ID, &[("gain_db", 0.0)])]);
    let got = render_with_automation(&reg, &m, SR, &x, &events).unwrap();
    // Reference: TestGain directly, one call with every event.
    let mut g = TestGain::new();
    g.load_state(&prepare_state(&g, TestGain::state_with_gain_db(0.0)).unwrap())
        .unwrap();
    g.activate(&ActivateConfig {
        sample_rate: SR,
        max_block: 8192,
        mode: ProcessMode::Offline,
        layout: ChannelLayout::MONO,
    })
    .unwrap();
    let mut list = EventList::with_capacity(2000);
    for e in &events {
        list.push(ParamEvent {
            offset: e.position as u32,
            id: e.id,
            value: e.value,
        })
        .unwrap();
    }
    let mut expected = vec![0.0f32; x.len()];
    let mut oe = OutputEvents::with_capacity(4);
    let mut ctx = ProcessContext::new(
        x.len() as u32,
        0,
        Transport::default(),
        list.as_slice(),
        &mut oe,
    );
    g.process(&mut ctx, &[&x], &mut [&mut expected]);
    assert!(bit_identical(&got, &expected));
}
