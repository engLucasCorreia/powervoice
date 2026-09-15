//! T-602: windowed offline renders (`vox_rack::offline::render_range`) — the SPEC-012 §2.8
//! amendment for bakes and exports of a range: whole-signal windows equal `render` bit for bit,
//! a range's pre-roll/post-roll context makes it equal the whole-file render's slice, the
//! pre-roll policy, cancellation through the tick callback, and slot failures.

#![allow(clippy::float_cmp)] // exact, bit-identical comparisons
mod common;

use std::sync::Arc;

use common::*;
use vox_modules::{ParametricEq, ParametricEqFactory, TruePeakLimiter, TruePeakLimiterFactory};
use vox_rack::offline::{
    PRE_ROLL_MAX_S, PRE_ROLL_MIN_S, RenderError, RenderWindow, WindowError, build_chain,
    plan_window, render, render_range,
};
use vox_rack::{RackModel, Registry};

vox_module_api::install_test_allocator!();

fn full_registry() -> Arc<Registry> {
    let mut all = test_factories();
    all.push(Arc::new(TruePeakLimiterFactory::new()));
    all.push(Arc::new(ParametricEqFactory::new()));
    Arc::new(Registry::with_factories(all).unwrap())
}

/// `render_range` over an in-memory signal, collecting the output.
fn window_render(
    reg: &Registry,
    m: &RackModel,
    x: &[f32],
    start: usize,
    len: usize,
) -> (Vec<f32>, RenderWindow) {
    let mut out = Vec::with_capacity(len);
    let window = render_range::<()>(
        reg,
        m,
        SR,
        start as u64,
        len as u64,
        x.len() as u64,
        |pos, buf| {
            let p = pos as usize;
            buf.copy_from_slice(&x[p..p + buf.len()]);
            Ok(())
        },
        |s| {
            out.extend_from_slice(s);
            Ok(())
        },
        |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(out.len(), len, "the output has exactly the range's length");
    (out, window)
}

#[test]
fn a_whole_signal_window_is_bit_identical_to_the_spec_012_render() {
    let reg = registry();
    let m = model(vec![gain_slot(-6.0), delay_slot(480), gain_slot(3.0)]);
    let x = white(3, 30_000);
    let (y, window) = window_render(&reg, &m, &x, 0, x.len());
    assert_eq!(window.pre_roll, 0);
    assert_eq!(
        window.post_roll, 0,
        "nothing follows the signal: zeros flush the latency"
    );
    assert_eq!(window.latency, 480);
    let reference = render(&reg, &m, SR, &x).unwrap();
    assert!(bit_identical(&y, &reference));
}

#[test]
fn a_range_equals_the_whole_file_render_slice_with_look_ahead_and_latency() {
    // A quiet tone, then a full-scale burst right after the range: the limiter's look-ahead
    // (its latency) must already pull the gain down *inside* the range, exactly as in the
    // whole-file render — which needs the post-roll (the real audio after the range).
    let reg = full_registry();
    let m = model(vec![slot(TruePeakLimiter::ID, &[]), gain_slot(-1.0)]);
    let mut x = vox_testkit::signal::sine(440.0, -30.0, 1.0, 48_000).unwrap();
    let burst = vox_testkit::signal::sine(440.0, 0.0, 0.2, 48_000).unwrap();
    let (start, end) = (12_000usize, 36_000usize);
    x[end..end + burst.len()].copy_from_slice(&burst);
    let whole = render(&reg, &m, SR, &x).unwrap();

    let (y, window) = window_render(&reg, &m, &x, start, end - start);
    assert!(
        window.latency > 0,
        "the limiter reports its look-ahead as latency"
    );
    assert_eq!(
        window.pre_roll, start as u64,
        "less than 30 s precede the range: all of it"
    );
    assert_eq!(window.post_roll, window.latency);
    assert!(
        bit_identical(&y, &whole[start..end]),
        "a range rendered with its context equals the whole-file render's slice"
    );

    // The context matters: an isolated render of the range differs near its end.
    let isolated = render(&reg, &m, SR, &x[start..end]).unwrap();
    assert!(max_abs_diff(&isolated, &whole[start..end]) > 1e-3);
}

#[test]
fn a_range_far_into_the_signal_pre_rolls_the_policy_minimum() {
    let reg = registry();
    let m = model(vec![gain_slot(-6.0), delay_slot(64)]);
    let x = white(9, (SR * 32.0) as usize);
    let start = (SR * 31.0) as usize;
    let (y, window) = window_render(&reg, &m, &x, start, 4_000);
    assert_eq!(window.pre_roll, (PRE_ROLL_MIN_S * SR) as u64);
    assert_eq!(window.first(), start as u64 - window.pre_roll);
    // Gain and an exact delay keep no signal-dependent state: the slice is bit-exact.
    let whole = render(&reg, &m, SR, &x).unwrap();
    assert!(bit_identical(&y, &whole[start..start + 4_000]));
}

#[test]
fn the_pre_roll_follows_the_tails_up_to_the_cap() {
    let reg = full_registry();
    // Gain only: no tail → the 30 s minimum.
    let chain = build_chain(&reg, &model(vec![gain_slot(0.0)]), SR).unwrap();
    let w = plan_window(&chain, SR, 100 * 48_000, 10, 200 * 48_000);
    assert_eq!(w.pre_roll, (PRE_ROLL_MIN_S * SR) as u64);
    // The EQ reports its ≈ 35 s worst-case ringing (SPEC-015 §4.8): the pre-roll covers it.
    let eq = build_chain(&reg, &model(vec![slot(ParametricEq::ID, &[])]), SR).unwrap();
    let tail = match eq.slot_tail(0).unwrap() {
        vox_module_api::Tail::Samples(n) => n,
        vox_module_api::Tail::Infinite => unreachable!(),
    };
    assert!(tail > (PRE_ROLL_MIN_S * SR) as u64, "EQ tail {tail}");
    let w = plan_window(&eq, SR, 100 * 48_000, 10, 200 * 48_000);
    assert_eq!(w.pre_roll, tail);
    // …but a bypassed slot's tail doesn't count.
    let off = build_chain(
        &reg,
        &model(vec![bypassed(slot(ParametricEq::ID, &[]))]),
        SR,
    )
    .unwrap();
    let w = plan_window(&off, SR, 100 * 48_000, 10, 200 * 48_000);
    assert_eq!(w.pre_roll, (PRE_ROLL_MIN_S * SR) as u64);
    // Two EQs: 70 s of tails, capped at 60 s.
    let two = model(vec![
        slot(ParametricEq::ID, &[]),
        slot(ParametricEq::ID, &[]),
    ]);
    let two = build_chain(&reg, &two, SR).unwrap();
    let w = plan_window(&two, SR, 100 * 48_000, 10, 200 * 48_000);
    assert_eq!(w.pre_roll, (PRE_ROLL_MAX_S * SR) as u64);
    // Never more than the signal before the range.
    let w = plan_window(&two, SR, 1_000, 10, 200 * 48_000);
    assert_eq!(w.pre_roll, 1_000);
}

#[test]
fn the_post_roll_is_the_signal_after_the_range_up_to_the_latency() {
    let reg = registry();
    let chain = build_chain(&reg, &model(vec![delay_slot(480)]), SR).unwrap();
    assert_eq!(plan_window(&chain, SR, 0, 1_000, 10_000).post_roll, 480);
    assert_eq!(plan_window(&chain, SR, 0, 1_000, 1_100).post_roll, 100);
    assert_eq!(plan_window(&chain, SR, 0, 1_000, 1_000).post_roll, 0);
}

#[test]
fn a_tick_error_stops_the_render() {
    let reg = registry();
    let x = white(4, 48_000);
    let mut ticks = 0;
    let mut written = 0usize;
    let err = render_range(
        &reg,
        &model(vec![gain_slot(0.0)]),
        SR,
        0,
        x.len() as u64,
        x.len() as u64,
        |pos, buf| {
            let p = pos as usize;
            buf.copy_from_slice(&x[p..p + buf.len()]);
            Ok(())
        },
        |s| {
            written += s.len();
            Ok(())
        },
        |_, _| {
            ticks += 1;
            if ticks == 2 { Err("cancelled") } else { Ok(()) }
        },
    )
    .unwrap_err();
    assert!(matches!(err, WindowError::Caller("cancelled")));
    assert_eq!(written, 2 * vox_rack::OFFLINE_BLOCK as usize);
}

#[test]
fn a_failing_slot_aborts_the_render_naming_it() {
    let reg = registry();
    let x = white(5, 20_000);
    let err = render_range::<()>(
        &reg,
        &model(vec![
            gain_slot(0.0),
            slot(vox_module_api::test_util::TestNaN::ID, &[]),
        ]),
        SR,
        0,
        x.len() as u64,
        x.len() as u64,
        |pos, buf| {
            let p = pos as usize;
            buf.copy_from_slice(&x[p..p + buf.len()]);
            Ok(())
        },
        |_| Ok(()),
        |_, _| Ok(()),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            WindowError::Render(RenderError::SlotFailed { index: 1, .. })
        ),
        "{err:?}"
    );
}
