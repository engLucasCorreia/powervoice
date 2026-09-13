//! Live rack (realtime path, block-feeding driver with random block sizes 1…1024 and 0-length
//! flushes; the fake-backend end-to-end versions come with T-105): SPEC-012 AC-1 (insert),
//! AC-2 (remove, reorder), AC-3 (bypass), AC-4 (A/B), AC-8 (latency + restart), AC-14
//! (non-finite guard), AC-15 (module reports), plus a random-edit stress run.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::*;
use vox_module_api::test_util::{
    TestReporter, TestRestart, TestRng, ZIPPER_CHANGE_AT, ZipperWindow, analyze_zipper, no_alloc,
    zipper_signal,
};
use vox_module_api::{ActivateConfig, ModuleFactory, Transport};
use vox_modules::{Gain, GainFactory};
use vox_rack::{LiveRack, RackHost, RackNotice, RackOptions, Registry, SlotStatus, xfade_gain};

vox_module_api::install_test_allocator!();

const S: usize = ZIPPER_CHANGE_AT;
/// Command sample for the level/equality checks (not block-aligned).
const C: usize = 40_013;

fn zipper(out: &[f32], latency: usize, label: &str) {
    let r = analyze_zipper(out, latency, SR, ZipperWindow::step(S, 15.0)).unwrap();
    println!("{label}: {r}");
    assert!(r.pass, "{label}: {r}");
}

/// Every 100 ms window from `from` differs from the reference by `expect_db` ± 0.01 dB.
fn windows_within(a: &[f32], reference: &[f32], from: usize, expect_db: f64) {
    let mut start = from;
    let mut count = 0;
    while start + MS100 <= a.len() {
        let d = rms_db(&a[start..start + MS100]) - rms_db(&reference[start..start + MS100]);
        assert!(
            (d - expect_db).abs() <= 0.01,
            "window at {start}: {d:.4} dB, expected {expect_db}"
        );
        start += MS100;
        count += 1;
    }
    assert!(count >= 5);
}

// ---------------------------------------------------------------------------------------------
// AC-1

#[test]
fn ac1_inserting_a_neutral_gain_keeps_the_delay_warm() {
    let x = white(1, 96_000);
    let m = model(vec![delay_slot(480)]);
    let a = uninterrupted(&m, &x, 7);
    let mut b = Driver::new(&m, 7, x.len());
    b.run_until(&x, C);
    b.host.insert(0, gain_slot(0.0)).unwrap();
    b.run_to_end(&x);
    let d = max_abs_diff(&a, &b.out);
    assert!(d <= 1e-6, "max deviation {d}");
    assert_eq!(b.host.len(), 2);
    assert_eq!(b.host.total_latency_samples(), 480);
    assert_eq!(b.live.latency_samples(), 480);
    assert_eq!(b.host.swaps_in_flight(), 0, "retired chain came back");
}

#[test]
fn ac1_inserting_minus_6_db_reaches_the_level_within_65_ms() {
    let x = white(2, 96_000);
    let m = model(vec![delay_slot(480)]);
    let a = uninterrupted(&m, &x, 8);
    let mut b = Driver::new(&m, 8, x.len());
    b.run_until(&x, C);
    b.host.insert(0, gain_slot(-6.0)).unwrap();
    b.run_to_end(&x);
    windows_within(&b.out, &a, C + MS65, -6.0);
    // Untouched before the command.
    assert!(bit_identical(&a[..C], &b.out[..C]));
}

#[test]
fn ac1_insert_passes_zipper() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 9, x.len());
    b.run_until(&x, S);
    b.host.insert(0, gain_slot(-6.0)).unwrap();
    b.run_to_end(&x);
    zipper(&b.out, 480, "AC-1 insert Gain -6 dB before TestDelay 480");
}

// ---------------------------------------------------------------------------------------------
// AC-2

#[test]
fn ac2_removing_a_gain_restores_the_level() {
    let x = white(3, 96_000);
    let without = uninterrupted(&model(vec![delay_slot(480)]), &x, 10);
    let mut b = Driver::new(&model(vec![gain_slot(-6.0), delay_slot(480)]), 10, x.len());
    b.run_until(&x, C);
    b.host.remove(0).unwrap();
    b.run_to_end(&x);
    windows_within(&b.out, &without, C + MS65, 0.0);
    assert_eq!(b.host.len(), 1);
    assert_eq!(b.live.chain().len(), 1, "compacted after the fade");
}

#[test]
fn ac2_remove_passes_zipper() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(-6.0), delay_slot(480)]), 11, x.len());
    b.run_until(&x, S);
    b.host.remove(0).unwrap();
    b.run_to_end(&x);
    zipper(&b.out, 480, "AC-2 remove Gain -6 dB");
}

#[test]
fn ac2_removing_the_last_slot_is_bit_transparent() {
    let x = white(4, 48_000);
    let mut b = Driver::new(&model(vec![gain_slot(-6.0)]), 12, x.len());
    b.run_until(&x, C);
    b.host.remove(0).unwrap();
    b.run_to_end(&x);
    assert!(bit_identical(&b.out[C + XF - 1..], &x[C + XF - 1..]));
    assert!(b.live.chain().is_empty());
}

#[test]
fn ac2_reorder_keeps_the_latency_and_converges() {
    let x = white(5, 96_000);
    let m = model(vec![gain_slot(-6.0), delay_slot(480)]);
    let a = uninterrupted(&m, &x, 13);
    let mut b = Driver::new(&m, 13, x.len());
    b.run_until(&x, C);
    b.host.move_slot(0, 1).unwrap();
    assert_eq!(b.host.total_latency_samples(), 480);
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 480);
    assert_eq!(
        b.host.slot_info(0).unwrap().module,
        "org.powervoice.test-restart@1.0.0"
    );
    let d = max_abs_diff(&a[C + MS65..], &b.out[C + MS65..]);
    assert!(d <= 1e-6, "max deviation {d}");
}

#[test]
fn ac2_reorder_passes_zipper() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(-6.0), delay_slot(480)]), 14, x.len());
    b.run_until(&x, S);
    b.host.move_slot(0, 1).unwrap();
    b.run_to_end(&x);
    zipper(
        &b.out,
        480,
        "AC-2 reorder [Gain -6, Delay 480] -> [Delay 480, Gain -6]",
    );
}

// ---------------------------------------------------------------------------------------------
// AC-3

#[test]
fn ac3_bypassing_a_gain_is_click_free_and_takes_15_ms() {
    for (start_bypassed, label) in [(false, "on"), (true, "off")] {
        let x = zipper_signal();
        let s = if start_bypassed {
            bypassed(gain_slot(-12.0))
        } else {
            gain_slot(-12.0)
        };
        let mut b = Driver::new(&model(vec![s]), 15, x.len());
        b.run_until(&x, S);
        b.host.set_bypass(0, !start_bypassed).unwrap();
        b.run_to_end(&x);
        zipper(&b.out, 0, &format!("AC-3 bypass {label}, Gain -12 dB"));
        let g = 10f32.powf(-12.0 / 20.0);
        let target = |n: usize| if start_bypassed { x[n] * g } else { x[n] };
        // Complete at S + 719 (15.0 ms), not before.
        for n in S + XF - 1..S + XF + 4800 {
            assert!((b.out[n] - target(n)).abs() <= 1e-7, "sample {n}");
        }
        let n = S + XF - 3;
        assert!((b.out[n] - target(n)).abs() > 1e-7 || x[n].abs() < 1e-3);
    }
}

#[test]
fn ac3_bypassing_a_delay_never_shifts_time() {
    let x = white(6, 96_000);
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 16, x.len());
    for (at, on) in [
        (20_011, true),
        (30_000, false),
        (30_100, true),
        (50_500, false),
        (70_000, true),
    ] {
        b.run_until(&x, at);
        b.host.set_bypass(0, on).unwrap();
        assert_eq!(b.host.total_latency_samples(), 480);
    }
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 480);
    assert!(b.out[..480].iter().all(|&s| s == 0.0));
    let d = max_abs_diff(&b.out[480..], &x[..x.len() - 480]);
    assert!(d <= 1e-6, "max deviation {d}");
}

// ---------------------------------------------------------------------------------------------
// AC-4

#[test]
fn ac4_ab_is_the_time_aligned_crossfade_to_the_delayed_input() {
    let x = white(7, 96_000);
    let m = model(vec![delay_slot(480), gain_slot(-6.0)]);
    let a = uninterrupted(&m, &x, 17);
    let mut b = Driver::new(&m, 17, x.len());
    b.run_until(&x, C);
    b.host.set_ab(true);
    b.run_to_end(&x);
    assert!(b.live.ab_enabled());
    for n in 0..x.len() {
        let dry = if n >= 480 { x[n - 480] } else { 0.0 };
        let expected = if n < C {
            a[n]
        } else {
            let g = xfade_gain(n - C, XF as u32);
            (1.0 - g) * a[n] + g * dry
        };
        assert!((b.out[n] - expected).abs() <= 1e-6, "sample {n}");
        if n >= C + XF {
            assert!((b.out[n] - dry).abs() <= 1e-6, "sample {n}");
        }
    }
}

#[test]
fn ac4_ab_passes_zipper() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![delay_slot(480), gain_slot(-6.0)]), 18, x.len());
    b.run_until(&x, S);
    b.host.set_ab(true);
    b.run_to_end(&x);
    zipper(&b.out, 480, "AC-4 A/B on, [Delay 480, Gain -6]");
}

#[test]
fn ac4_ab_follows_a_latency_change_and_is_reset_on_reopen() {
    let x = white(8, 96_000);
    let m = model(vec![delay_slot(480)]);
    let mut b = Driver::new(&m, 19, x.len());
    b.run_until(&x, 10_000);
    b.host.set_ab(true);
    b.run_until(&x, 20_000);
    // Latency grows beyond the initial A/B line (100 ms): a bigger line comes with the chain.
    b.host.insert(1, delay_slot(9_000)).unwrap();
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 9_480);
    for n in 40_000..x.len() {
        assert!((b.out[n] - x[n - 9_480]).abs() <= 1e-6, "sample {n}");
    }
    assert!(b.host.ab());
    b.host.load_model(&m).unwrap();
    assert!(!b.host.ab(), "A/B is off after reopening");
}

// ---------------------------------------------------------------------------------------------
// AC-8

#[test]
fn ac8_latency_sum_and_restart_with_a_new_latency() {
    let m = model(vec![
        delay_slot(480),
        gain_slot(0.0),
        bypassed(delay_slot(128)),
        slot("com.acme.x", &[]),
    ]);
    let x = white(9, 96_000);
    let mut d = Driver::new(&m, 20, x.len());
    assert_eq!(d.host.total_latency_samples(), 608);
    assert_eq!(d.live.latency_samples(), 608);
    let lat: Vec<u32> = (0..4)
        .map(|i| d.host.slot_info(i).unwrap().latency_samples)
        .collect();
    assert_eq!(lat, vec![480, 0, 128, 0]);
    let ms = 608.0 / SR * 1000.0;
    assert_eq!(format!("{ms:.1} ms (608 smp)"), "12.7 ms (608 smp)");
    d.run_until(&x, C);
    d.host.set_param(0, TestRestart::LATENCY, 960.0).unwrap();
    let (mut host_at, mut live_at) = (None, None);
    while d.pos < x.len() {
        let next = (d.pos + 240).min(x.len());
        d.run_until(&x, next);
        if host_at.is_none() && d.host.total_latency_samples() == 1088 {
            host_at = Some(d.pos);
        }
        if live_at.is_none() && d.live.latency_samples() == 1088 {
            live_at = Some(d.pos);
        }
    }
    d.tick();
    let (host_at, live_at) = (host_at.unwrap(), live_at.unwrap());
    assert!(
        host_at <= C + MS100 && live_at <= C + MS100,
        "{host_at} {live_at}"
    );
    assert!(d.notices.iter().any(|(p, n)| *n
        == RackNotice::LatencyChanged {
            total_samples: 1088
        }
        && *p <= C + MS100));
    assert_eq!(d.host.slot_info(0).unwrap().latency_samples, 960);
    // The replacement is in the path: x delayed by 960 + 128 (bypassed, latency-matched).
    for n in live_at + 2 * XF + 1088..x.len() {
        assert!((d.out[n] - x[n - 1088]).abs() <= 1e-6, "sample {n}");
    }
    assert_eq!(d.host.swaps_in_flight(), 0);
}

// ---------------------------------------------------------------------------------------------
// AC-14

#[test]
fn ac14_non_finite_output_is_caught_and_the_slot_bypassed() {
    let m = model(vec![
        slot("org.powervoice.test-nan", &[]),
        slot(Sentinel::ID, &[]),
        gain_slot(0.0),
    ]);
    let x = white(10, 48_000);
    let mut d = Driver::new(&m, 21, x.len());
    d.run_to_end(&x);
    assert!(d.out.iter().all(|s| s.is_finite()));
    assert!(!SENTINEL_SAW_NON_FINITE.load(Ordering::Relaxed));
    assert!(SENTINEL_SAMPLES.load(Ordering::Relaxed) >= x.len());
    let from = NAN_FROM as usize + XF;
    assert!(
        bit_identical(&d.out[from..], &x[from..]),
        "bypassed within 15 ms"
    );
    let failed: Vec<&String> = d
        .notices
        .iter()
        .filter_map(|(_, n)| match n {
            RackNotice::SlotFailed {
                index: 0, message, ..
            } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        failed,
        vec!["Test NaN produced invalid audio and was bypassed"]
    );
    assert!(matches!(
        d.host.slot_info(0).unwrap().status,
        SlotStatus::Failed { .. }
    ));
}

// ---------------------------------------------------------------------------------------------
// AC-15

#[test]
fn ac15_module_reports_reach_the_mirror_within_100_ms() {
    let x = white(11, 5 * 48_000);
    let mut d = Driver::new(&model(vec![slot(TestReporter::ID, &[])]), 22, x.len());
    d.run_to_end(&x);
    let reports: Vec<(usize, f64)> = d
        .notices
        .iter()
        .filter_map(|(p, n)| match n {
            RackNotice::ParamChanged { id, value, .. } if *id == TestReporter::COUNT => {
                Some((*p, *value))
            }
            _ => None,
        })
        .collect();
    let expected = (x.len() - 1) / REPORT_PERIOD as usize;
    assert_eq!(reports.len(), expected, "no report lost");
    for (k, (pos, v)) in reports.iter().enumerate() {
        let k = k + 1;
        assert_eq!(*v, k as f64);
        assert!(
            *pos <= k * REPORT_PERIOD as usize + MS100,
            "report {k} seen at {pos}"
        );
    }
    assert_eq!(
        d.host.param_value(0, TestReporter::COUNT),
        Some(expected as f64)
    );
    assert_eq!(d.host.dropped_rt_events(), 0);
}

// ---------------------------------------------------------------------------------------------
// Stress: random live edits under the allocation checker; the rack stays consistent.

#[test]
fn random_live_edits_are_rt_safe_and_consistent() {
    let x = white(12, 10 * 48_000);
    let mut d = Driver::new(&model(vec![gain_slot(-3.0), delay_slot(64)]), 23, x.len());
    let mut rng = TestRng::new(0xED17);
    let mut at = 2000;
    while at < x.len() - 20_000 {
        d.run_until(&x, at);
        let len = d.host.len();
        match rng.below(8) {
            0 if len < 10 => {
                let s = if rng.chance(0.5) {
                    gain_slot(-rng.unit_f64() * 20.0)
                } else {
                    delay_slot(rng.range_u32(0, 3000))
                };
                d.host
                    .insert(rng.below(len as u64 + 1) as usize, s)
                    .unwrap();
            }
            1 if len > 0 => d.host.remove(rng.below(len as u64) as usize).unwrap(),
            2 if len > 1 => d
                .host
                .move_slot(
                    rng.below(len as u64) as usize,
                    rng.below(len as u64) as usize,
                )
                .unwrap(),
            3 if len > 0 => d
                .host
                .set_bypass(rng.below(len as u64) as usize, rng.chance(0.5))
                .unwrap(),
            4 => d.host.set_ab(rng.chance(0.3)),
            5 if len > 0 => {
                let i = rng.below(len as u64) as usize;
                if d.host.slot_info(i).unwrap().module.starts_with(Gain::ID) {
                    d.host
                        .set_param(i, Gain::GAIN_DB, -rng.unit_f64() * 40.0)
                        .unwrap();
                } else {
                    d.host
                        .set_param(i, TestRestart::LATENCY, f64::from(rng.range_u32(0, 2000)))
                        .unwrap();
                }
            }
            6 if len > 0 => d.host.restart(rng.below(len as u64) as usize).unwrap(),
            _ => {}
        }
        at += rng.range_u32(10, 9000) as usize;
    }
    d.run_to_end(&x);
    for _ in 0..4 {
        d.run_until(&x, x.len());
        d.tick();
    }
    assert!(d.out.iter().all(|s| s.is_finite()));
    assert_eq!(d.host.swaps_in_flight(), 0);
    assert_eq!(d.live.chain().len(), d.host.len(), "dead slots compacted");
    assert_eq!(d.live.latency_samples(), d.host.total_latency_samples());
}

// ---------------------------------------------------------------------------------------------
// New instances with latency: the crossfade waits until their output is valid (review item 1;
// SPEC-012 §2.2 "audible within 50 ms plus the new module's reported latency").

fn max_silent_run(x: &[f32]) -> usize {
    let (mut best, mut run) = (0, 0);
    for &s in x {
        if s.abs() < 1e-6 {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

#[test]
fn inserting_a_latency_module_waits_for_its_output() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(0.0)]), 40, x.len());
    b.run_until(&x, S);
    b.host.insert(1, delay_slot(480)).unwrap();
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 480);
    zipper(&b.out, 480, "insert TestDelay 480 after Gain 0 dB");
    assert!(
        bit_identical(&b.out[S..S + 480], &x[S..S + 480]),
        "the undelayed input plays until the new output is valid"
    );
    assert!(max_silent_run(&b.out[S..]) < 3);
}

#[test]
fn restarting_a_latency_module_is_click_free() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 41, x.len());
    b.run_until(&x, S);
    b.host.restart(0).unwrap();
    b.run_to_end(&x);
    zipper(&b.out, 480, "restart TestDelay 480 (same latency)");
    let d = max_abs_diff(&b.out[480..], &x[..x.len() - 480]);
    assert!(d <= 1e-6, "{d}");
}

#[test]
fn restart_to_a_new_latency_is_click_free_and_never_silent() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 42, x.len());
    b.run_until(&x, S);
    b.host.set_param(0, TestRestart::LATENCY, 960.0).unwrap();
    b.host.restart(0).unwrap();
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 960);
    zipper(&b.out, 960, "restart TestDelay 480 -> 960");
    assert!(max_silent_run(&b.out[S..]) < 3);
    for n in S..S + 960 {
        assert!(
            (b.out[n] - x[n - 480]).abs() <= 1e-7,
            "old instance plays at {n}"
        );
    }
    assert!(max_abs_diff(&b.out[S + 960 + XF..], &x[S + XF..x.len() - 960]) <= 1e-6);
}

#[test]
fn reorder_by_moving_the_latency_slot_is_click_free() {
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(-6.0), delay_slot(480)]), 43, x.len());
    b.run_until(&x, S);
    b.host.move_slot(1, 0).unwrap();
    b.run_to_end(&x);
    assert_eq!(b.live.latency_samples(), 480);
    zipper(&b.out, 480, "reorder: move TestDelay 480 above Gain -6");
    assert!(max_silent_run(&b.out[S..]) < 3);
}

#[test]
fn two_restarts_within_15_ms_stay_seamless() {
    let x = white(30, 96_000);
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 44, x.len());
    b.run_until(&x, C);
    b.host.restart(0).unwrap();
    b.run_until(&x, C + 100);
    b.host.restart(0).unwrap();
    b.run_to_end(&x);
    for _ in 0..3 {
        b.tick();
    }
    let d = max_abs_diff(&b.out[480..], &x[..x.len() - 480]);
    assert!(d <= 1e-6, "{d}");
    assert_eq!(b.host.swaps_in_flight(), 0);
    assert_eq!(b.live.chain().len(), 1);
}

#[test]
fn ac8_restart_path_never_goes_silent() {
    let x = white(33, 96_000);
    let mut d = Driver::new(&model(vec![delay_slot(480), gain_slot(0.0)]), 47, x.len());
    d.run_until(&x, C);
    d.host.set_param(0, TestRestart::LATENCY, 960.0).unwrap();
    d.run_to_end(&x);
    assert_eq!(d.live.latency_samples(), 960);
    assert!(max_silent_run(&d.out[C..]) < 3);
}

// ---------------------------------------------------------------------------------------------
// H-01: a bypass toggle or a failure inside a new instance's warm-up hold waits until its output
// (and its fresh dry line) is valid, instead of cancelling the hold (SPEC-012 §2.2, §2.3, §2.9).

/// A long latency (100 ms at 48 kHz), in the range of Slice 3's noise reduction.
const LONG: usize = 4800;

/// §4.3 over the whole transition, unshifted: T_s = the latency hold + the 15 ms crossfade.
fn zipper_hold(out: &[f32], latency: usize, label: &str) {
    let t_s_ms = 15.0 + latency as f64 * 1000.0 / SR;
    let r = analyze_zipper(out, 0, SR, ZipperWindow::step(S, t_s_ms)).unwrap();
    println!("{label}: {r}");
    assert!(r.pass, "{label}: {r}");
}

/// From S the output plays `before` for `hold` samples, then crossfades (15 ms) to `after`.
fn assert_hold_then_fade(
    out: &[f32],
    hold: usize,
    before: impl Fn(usize) -> f32,
    after: impl Fn(usize) -> f32,
    label: &str,
) {
    for n in S..out.len() {
        let expected = if n < S + hold {
            before(n)
        } else {
            let g = xfade_gain(n - S - hold, XF as u32);
            (1.0 - g) * before(n) + g * after(n)
        };
        assert!(
            (out[n] - expected).abs() <= 1e-6,
            "{label}: sample {n}: {} vs {expected}",
            out[n]
        );
    }
}

#[test]
fn bypass_inside_an_insert_hold_waits_for_the_new_output() {
    let label = "insert TestDelay 4800, bypass on 1000 samples later";
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(0.0)]), 50, x.len());
    b.run_until(&x, S);
    b.host.insert(1, delay_slot(LONG as u32)).unwrap();
    b.run_until(&x, S + 1000);
    b.host.set_bypass(1, true).unwrap();
    b.run_to_end(&x);
    zipper_hold(&b.out, LONG, label);
    assert!(max_silent_run(&b.out[S..]) < 3, "{label}: silent gap");
    assert_hold_then_fade(&b.out, LONG, |n| x[n], |n| x[n - LONG], label);
    assert_eq!(b.live.latency_samples(), LONG as u32);
}

#[test]
fn unbypass_inside_a_bypassed_insert_hold_waits_for_the_new_output() {
    let label = "insert TestDelay 4800 bypassed, un-bypass 1000 samples later";
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![gain_slot(0.0)]), 51, x.len());
    b.run_until(&x, S);
    b.host.insert(1, bypassed(delay_slot(LONG as u32))).unwrap();
    b.run_until(&x, S + 1000);
    b.host.set_bypass(1, false).unwrap();
    b.run_to_end(&x);
    zipper_hold(&b.out, LONG, label);
    assert!(max_silent_run(&b.out[S..]) < 3, "{label}: silent gap");
    assert_hold_then_fade(&b.out, LONG, |n| x[n], |n| x[n - LONG], label);
    assert_eq!(b.live.latency_samples(), LONG as u32);
}

#[test]
fn bypass_inside_a_restart_hold_keeps_the_old_instance_until_the_new_output_is_valid() {
    let label = "restart TestDelay 480 -> 4800, bypass on 1000 samples later";
    let x = zipper_signal();
    let mut b = Driver::new(&model(vec![delay_slot(480)]), 52, x.len());
    b.run_until(&x, S);
    b.host
        .set_param(0, TestRestart::LATENCY, LONG as f64)
        .unwrap();
    b.host.restart(0).unwrap();
    b.run_until(&x, S + 1000);
    b.host.set_bypass(0, true).unwrap();
    b.run_to_end(&x);
    for _ in 0..3 {
        b.tick();
    }
    zipper_hold(&b.out, LONG, label);
    assert!(max_silent_run(&b.out[S..]) < 3, "{label}: silent gap");
    assert_hold_then_fade(&b.out, LONG, |n| x[n - 480], |n| x[n - LONG], label);
    assert_eq!(b.live.latency_samples(), LONG as u32);
    assert_eq!(b.host.swaps_in_flight(), 0);
    assert_eq!(b.live.chain().len(), 1, "outgoing instance retired");
}

#[test]
fn a_failure_inside_an_insert_hold_waits_for_the_dry_line() {
    let label = "insert Probe 4800 that outputs NaN after 1000 samples";
    let reg = Arc::new(
        Registry::with_factories([
            Arc::new(GainFactory::new()) as Arc<dyn ModuleFactory>,
            factory(|| {
                let mut p = Probe::new(LONG as u32);
                p.nan_from = Some(1000);
                Box::new(p)
            }),
        ])
        .unwrap(),
    );
    let x = zipper_signal();
    let mut b = Driver::with_registry(reg, &model(vec![gain_slot(0.0)]), 53, x.len());
    b.run_until(&x, S);
    b.host
        .insert(1, slot("org.powervoice.test-probe", &[]))
        .unwrap();
    b.run_to_end(&x);
    assert!(b.out.iter().all(|s| s.is_finite()));
    zipper_hold(&b.out, LONG, label);
    assert!(max_silent_run(&b.out[S..]) < 3, "{label}: silent gap");
    assert_hold_then_fade(&b.out, LONG, |n| x[n], |n| x[n - LONG], label);
    assert!(
        b.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotFailed { index: 1, .. }))
    );
}

fn run_blocks(host: &mut RackHost, live: &mut LiveRack, blocks: usize) {
    let x = [0.25f32; 64];
    let mut y = [0.0f32; 64];
    let t = Transport {
        playing: true,
        position_samples: None,
    };
    for _ in 0..blocks {
        no_alloc(|| live.process(t, &x, &mut y)).expect("LiveRack::process allocated");
        host.tick();
    }
}

#[test]
fn a_replacement_without_a_crossfade_does_not_hold_back_the_next_one() {
    // At 50 Hz the crossfade is 1 sample: a latency-0 replacement completes at the swap.
    let config = ActivateConfig {
        sample_rate: 50.0,
        ..rt_config()
    };
    let (mut host, mut live) = RackHost::new(
        registry(),
        config,
        RackOptions::default(),
        &model(vec![delay_slot(0)]),
    )
    .unwrap();
    host.restart(0).unwrap();
    run_blocks(&mut host, &mut live, 4);
    host.set_param(0, TestRestart::LATENCY, 5.0).unwrap();
    host.restart(0).unwrap();
    run_blocks(&mut host, &mut live, 4);
    assert_eq!(live.latency_samples(), 5, "second replacement went through");
    assert_eq!(host.swaps_in_flight(), 0);
}

// ---------------------------------------------------------------------------------------------
// Lifecycle events are never crowded out by module reports (review item 3); teardown.

#[test]
fn lifecycle_events_survive_a_flood_of_module_reports() {
    use std::sync::Arc;
    let reg = Arc::new(
        vox_rack::Registry::with_factories([
            Arc::new(vox_modules::GainFactory::new()) as Arc<dyn vox_module_api::ModuleFactory>,
            factory(|| Box::new(TestRestart::new(0))),
            factory(|| Box::new(TestReporter::new(1))),
        ])
        .unwrap(),
    );
    let x = white(31, 48_000);
    let m = model(vec![
        slot(TestReporter::ID, &[]),
        gain_slot(-6.0),
        delay_slot(64),
    ]);
    let mut d = Driver::with_registry(reg, &m, 45, x.len());
    d.run_until(&x, 10_000);
    d.host.remove(1).unwrap();
    d.host.restart(1).unwrap();
    d.run_to_end(&x);
    for _ in 0..3 {
        d.tick();
    }
    assert!(d.host.dropped_rt_events() > 0, "the flood overflowed");
    assert_eq!(d.live.chain().len(), 2, "SlotRemoved arrived: compacted");
    assert_eq!(d.host.swaps_in_flight(), 0);
}

#[test]
fn into_chains_and_teardown_hand_every_chain_back() {
    let m = model(vec![gain_slot(-6.0), delay_slot(480)]);
    let (mut host, live) = vox_rack::RackHost::new(
        registry(),
        rt_config(),
        vox_rack::RackOptions::default(),
        &m,
    )
    .unwrap();
    host.insert(0, gain_slot(0.0)).unwrap();
    let mut chains = live.into_chains();
    assert_eq!(chains.len(), 2, "installed + the swap still queued");
    for c in &mut chains {
        c.deactivate();
    }
    let x = white(34, 20_000);
    let mut d = Driver::new(&m, 46, x.len());
    d.run_until(&x, 5_000);
    d.host.insert(0, gain_slot(0.0)).unwrap();
    let Driver { host, live, .. } = d;
    host.teardown(live);
}
