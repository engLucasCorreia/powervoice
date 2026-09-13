//! Chain basics (T-005 skeleton tests, carried over to `Chain`): processing, event routing
//! across sub-blocks, carry-over, latency sum, chain edits, RT safety.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use std::sync::atomic::Ordering;

use common::*;
use vox_module_api::test_util::{TestGain, TestRng, no_alloc};
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventListError, Module, OutputEvents, ParamEvent, ParamFlags,
    ProcessContext, ProcessStatus, Tail, Transport, prepare_state,
};
use vox_rack::{Chain, LiveRack, PushEventError, RackError, RackHost, RackOptions};

vox_module_api::install_test_allocator!();

fn gain(db: f64) -> Box<dyn Module> {
    let mut m = TestGain::new();
    m.load_state(&prepare_state(&m, TestGain::state_with_gain_db(db)).unwrap())
        .unwrap();
    Box::new(m)
}

fn gain_event(offset: u32, db: f64) -> ParamEvent {
    ParamEvent {
        offset,
        id: TestGain::GAIN_DB,
        value: db,
    }
}

fn sine_1k(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / SR).sin()) as f32)
        .collect()
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = TestRng::new(seed);
    (0..n).map(|_| 0.8 * rng.bipolar_f32()).collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Runs `input` through `chain` in host blocks of `block` frames, under the allocation checker.
fn run(chain: &mut Chain, input: &[f32], block: usize) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    for (i, o) in input.chunks(block).zip(out.chunks_mut(block)) {
        let status = no_alloc(|| chain.process(Transport::default(), i, o))
            .expect("chain.process allocated");
        assert_eq!(status, ProcessStatus::Continue);
    }
    out
}

#[test]
fn three_test_gains_at_minus_6_db_give_minus_18_db() {
    let x = sine_1k(48_000);
    let settle = TestGain::ramp_samples(SR) as usize;
    let mut from_state = Chain::new();
    let mut from_events = Chain::new();
    for _ in 0..3 {
        from_state.push(gain(-6.0)).unwrap();
        from_events.push(gain(0.0)).unwrap();
    }
    from_state.activate(&cfg(256)).unwrap();
    from_events.activate(&cfg(256)).unwrap();
    for slot in 0..3 {
        from_events.push_event(slot, gain_event(0, -6.0)).unwrap();
    }
    for chain in [&mut from_state, &mut from_events] {
        let y = run(chain, &x, 480);
        let db = 20.0 * (rms(&y[settle..]) / rms(&x[settle..])).log10();
        assert!((db + 18.0).abs() <= 0.01, "{db} dB");
    }
}

#[test]
fn events_split_at_sub_block_boundaries_keep_their_sample_position() {
    let x = noise(1, 1000);
    let events = [
        gain_event(0, -20.0),
        gain_event(10, -3.0),
        gain_event(63, -40.0),
        gain_event(64, 6.0),
        gain_event(500, -60.0),
        gain_event(999, 0.0),
    ];
    let mut direct = TestGain::new();
    direct.activate(&cfg(1024)).unwrap();
    let mut expected = vec![0.0; 1000];
    let mut out_events = OutputEvents::with_capacity(8);
    let mut ctx = ProcessContext::new(1000, 0, Transport::default(), &events, &mut out_events);
    direct.process(&mut ctx, &[&x], &mut [&mut expected]);

    let mut chain = Chain::new();
    chain.push(gain(0.0)).unwrap();
    chain.activate(&cfg(64)).unwrap();
    for e in events {
        chain.push_event(0, e).unwrap();
    }
    let got = run(&mut chain, &x, 1000);
    assert!(bit_identical(&got, &expected));
    assert_eq!(
        chain
            .module(0)
            .unwrap()
            .param_value(TestGain::GAIN_DB)
            .map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
}

#[test]
fn chain_matches_modules_processed_in_sequence() {
    let x = noise(2, 3000);
    let (g1, g2, g3) = (-3.0, 4.5, -10.0);
    let mut expected = x.clone();
    for db in [g1, g2, g3] {
        let mut m = gain(db);
        m.activate(&cfg(4096)).unwrap();
        let input = expected.clone();
        let mut out_events = OutputEvents::with_capacity(8);
        let mut ctx = ProcessContext::new(3000, 0, Transport::default(), &[], &mut out_events);
        m.process(&mut ctx, &[&input], &mut [&mut expected]);
    }
    for (host_block, max_block) in [(1usize, 1u32), (37, 16), (3000, 1024), (1024, 1024)] {
        let mut chain = Chain::new();
        for db in [g1, g2, g3] {
            chain.push(gain(db)).unwrap();
        }
        chain.activate(&cfg(max_block)).unwrap();
        let got = run(&mut chain, &x, host_block);
        assert!(
            bit_identical(&got, &expected),
            "host block {host_block}, max_block {max_block}"
        );
    }
}

#[test]
fn full_event_lists_carry_over_without_losing_values() {
    let options = RackOptions {
        event_capacity: 4,
        queue_capacity: 16,
    };
    let mut chain = Chain::with_options(options);
    let (probe, log) = probe_with_log(0);
    chain.push(probe).unwrap();
    chain.activate(&cfg(10)).unwrap();
    // 30-frame host block → sub-blocks [0,10) [10,20) [20,30).
    for (offset, v) in [
        (0, 1.0),
        (0, 2.0),
        (0, 3.0),
        (0, 4.0),
        (0, 5.0),
        (0, 6.0),
        (15, 7.0),
        (29, 8.0),
        (29, 9.0),
        (29, 10.0),
        (29, 11.0),
        (29, 12.0),
    ] {
        chain.push_event(0, value_event(offset, v)).unwrap();
    }
    let x = vec![0.0; 30];
    run(&mut chain, &x, 30);
    run(&mut chain, &x, 30);
    // Same-(offset, id) pushes coalesce in place: the module sees (0, 6.0), (15, 7.0), (29, 12.0).
    assert_eq!(
        chain.module(0).unwrap().param_value(Probe::VALUE),
        Some(12.0)
    );
    chain.deactivate();
    let got: Vec<(u64, f64)> = log.lock().unwrap().iter().map(|e| (e.0, e.2)).collect();
    assert_eq!(got, vec![(0, 6.0), (15, 7.0), (29, 12.0)]);
}

#[test]
fn push_event_errors() {
    let mut chain = Chain::with_options(RackOptions {
        event_capacity: 2,
        queue_capacity: 2,
    });
    chain.push(gain(0.0)).unwrap();
    assert_eq!(
        chain.push_event(1, gain_event(0, 0.0)),
        Err(PushEventError::NoSuchSlot(1))
    );
    chain.push_event(0, gain_event(5, 0.0)).unwrap();
    assert_eq!(
        chain.push_event(0, gain_event(4, 0.0)),
        Err(PushEventError::List(EventListError::OutOfOrder))
    );
    // Same (offset, id): coalesced, never Full.
    chain.push_event(0, gain_event(5, -1.0)).unwrap();
    chain.push_event(0, gain_event(6, 0.0)).unwrap();
    assert_eq!(
        chain.push_event(0, gain_event(7, 0.0)),
        Err(PushEventError::List(EventListError::Full))
    );
}

#[test]
fn zero_length_block_flushes_events() {
    let mut chain = Chain::new();
    chain.push(gain(0.0)).unwrap();
    chain.activate(&cfg(64)).unwrap();
    chain.push_event(0, gain_event(0, -9.0)).unwrap();
    no_alloc(|| chain.process(Transport::default(), &[], &mut [])).unwrap();
    assert_eq!(
        chain.module(0).unwrap().param_value(TestGain::GAIN_DB),
        Some(-9.0)
    );
}

#[test]
fn latency_is_the_sum_of_slot_latencies() {
    let mut chain = Chain::new();
    chain.push(Box::new(Probe::new(3))).unwrap();
    chain.push(gain(0.0)).unwrap();
    chain.push(Box::new(Probe::new(5))).unwrap();
    assert_eq!(chain.latency_samples(), 0, "unknown until activated");
    chain.activate(&cfg(32)).unwrap();
    assert_eq!(chain.latency_samples(), 8);
    assert_eq!(chain.slot_latency_samples(2), Some(5));
    assert_eq!(chain.slot_tail(0), Some(Tail::Samples(3)));
    let x = noise(3, 200);
    let y = run(&mut chain, &x, 50);
    assert!(y[..8].iter().all(|&s| s == 0.0));
    assert!(bit_identical(&y[8..], &x[..192]));
    chain.insert(1, Box::new(Probe::new(10))).unwrap();
    assert_eq!(chain.latency_samples(), 18);
    chain.remove(1).unwrap();
    assert_eq!(chain.latency_samples(), 8);
    // A bypassed slot still counts (its dry path is latency-matched).
    chain.set_bypass_now(0, true).unwrap();
    assert_eq!(chain.latency_samples(), 8);
}

#[test]
fn insert_remove_reorder() {
    let mut chain = Chain::new();
    chain.push(gain(-1.0)).unwrap();
    chain.push(gain(-2.0)).unwrap();
    chain.insert(0, gain(-3.0)).unwrap();
    let order = |c: &Chain| -> Vec<f64> {
        (0..c.len())
            .map(|i| c.module(i).unwrap().param_value(TestGain::GAIN_DB).unwrap())
            .collect()
    };
    assert_eq!(order(&chain), vec![-3.0, -1.0, -2.0]);
    chain.move_slot(0, 2).unwrap();
    assert_eq!(order(&chain), vec![-1.0, -2.0, -3.0]);
    chain.move_slot(2, 1).unwrap();
    assert_eq!(order(&chain), vec![-1.0, -3.0, -2.0]);
    let removed = chain.remove(1).unwrap();
    assert_eq!(removed.param_value(TestGain::GAIN_DB), Some(-3.0));
    assert!(matches!(
        chain.remove(5),
        Err(RackError::IndexOutOfRange { index: 5, len: 2 })
    ));
    assert!(matches!(
        chain.move_slot(0, 2),
        Err(RackError::IndexOutOfRange { index: 2, .. })
    ));
    assert!(matches!(
        chain.insert(3, gain(0.0)),
        Err(RackError::IndexOutOfRange { .. })
    ));
    let p = Probe::new(0);
    let deactivations = p.deactivations.clone();
    chain.push(Box::new(p)).unwrap();
    chain.activate(&cfg(16)).unwrap();
    chain.remove(2).unwrap();
    assert_eq!(deactivations.load(Ordering::Relaxed), 1);
    for _ in chain.len()..vox_rack::MAX_SLOTS {
        chain.push(gain(0.0)).unwrap();
    }
    assert!(matches!(
        chain.push(gain(0.0)),
        Err(RackError::TooManySlots)
    ));
}

#[test]
fn rejects_unsupported_layouts_and_invalid_schemas() {
    let mut chain = Chain::new();
    let mut odd = Probe::new(0);
    odd.layouts = vec![ChannelLayout {
        inputs: 2,
        outputs: 1,
    }];
    let e = chain.push(Box::new(odd)).unwrap_err();
    assert!(matches!(e, RackError::UnsupportedLayout { .. }));
    assert_eq!(e.to_string(), "Probe has an unsupported channel layout");
    let mut bad = Probe::new(0);
    bad.params[0].flags = ParamFlags::READ_ONLY | ParamFlags::AUTOMATABLE;
    assert!(matches!(
        chain.push(Box::new(bad)),
        Err(RackError::Schema { .. })
    ));
    assert!(chain.is_empty());
    // Stereo-only modules run through the dual-mono shim.
    let mut stereo = Probe::new(4);
    stereo.layouts = vec![ChannelLayout::STEREO];
    chain.push(Box::new(stereo)).unwrap();
    chain.activate(&cfg(16)).unwrap();
    assert_eq!(chain.latency_samples(), 4);
}

#[test]
fn activation_failure_rolls_back() {
    let mut chain = Chain::new();
    let ok = Probe::new(0);
    let deactivations = ok.deactivations.clone();
    chain.push(Box::new(ok)).unwrap();
    let mut failing = Probe::new(0);
    failing.fail_activate = true;
    chain.push(Box::new(failing)).unwrap();
    let e = chain.activate(&cfg(16)).unwrap_err();
    assert!(matches!(e, RackError::Activate { index: 1, .. }));
    assert_eq!(e.to_string(), "Couldn't start Probe: resource error: nope");
    assert!(!chain.is_active());
    assert_eq!(deactivations.load(Ordering::Relaxed), 1);
    let mut chain = Chain::new();
    assert!(matches!(
        chain.activate(&ActivateConfig {
            max_block: 0,
            ..cfg(1)
        }),
        Err(RackError::InvalidConfig(_))
    ));
    assert!(matches!(
        chain.activate(&ActivateConfig {
            layout: ChannelLayout::STEREO,
            ..cfg(1)
        }),
        Err(RackError::InvalidConfig(_))
    ));
    chain.activate(&cfg(8)).unwrap();
    assert!(matches!(
        chain.activate(&cfg(8)),
        Err(RackError::AlreadyActive)
    ));
}

#[test]
fn inactive_or_empty_chain_passes_through() {
    let x = noise(4, 100);
    let mut chain = Chain::new();
    let mut y = vec![0.0; 100];
    chain.process(Transport::default(), &x, &mut y);
    assert!(bit_identical(&x, &y));
    chain.activate(&cfg(16)).unwrap();
    let y = run(&mut chain, &x, 33);
    assert!(bit_identical(&x, &y));
}

#[test]
fn process_reset_push_and_bypass_are_allocation_free() {
    let mut chain = Chain::new();
    for db in [-1.0, -2.0, -3.0, -4.0] {
        chain.push(gain(db)).unwrap();
    }
    chain.push(Box::new(Probe::new(100))).unwrap();
    chain.activate(&cfg(128)).unwrap();
    let x = noise(5, 20_000);
    let mut y = vec![0.0; 20_000];
    let mut rng = TestRng::new(9);
    no_alloc(|| {
        let mut pos = 0;
        while pos < x.len() {
            let n = (rng.range_u32(0, 700) as usize).min(x.len() - pos);
            let _ = chain.push_event(rng.below(4) as usize, gain_event(0, -rng.unit_f64() * 30.0));
            if rng.chance(0.05) {
                let _ = chain.set_bypass(rng.below(5) as usize, rng.chance(0.5));
            }
            chain.process(Transport::default(), &x[pos..pos + n], &mut y[pos..pos + n]);
            if rng.chance(0.05) {
                chain.reset();
            }
            pos += n;
        }
    })
    .expect("chain RT path allocated");
    assert!(y.iter().all(|s| s.is_finite()));
}

#[test]
fn rack_types_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Chain>();
    assert_send::<Box<Chain>>();
    assert_send::<LiveRack>();
    assert_send::<RackHost>();
}
