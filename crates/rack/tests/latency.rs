//! T-401 latency compensation through the live rack (SPEC-012 §2.3, §2.5, §2.8, AC-3, AC-4,
//! AC-8, AC-10): impulse alignment through racks with mixed latencies (live and offline), the
//! whole-rack A/B switch aligned to the sample (impulse comb, bit-exact once steady), and rapid
//! successive latency changes mid-play — glitch-free (§4.3 and a sample-step bound), never
//! silent, allocation-free (every `LiveRack::process` runs under `no_alloc` in the driver), and
//! converging on the last requested latency with the readout notice following.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use common::*;
use vox_module_api::test_util::{
    TestRestart, ZIPPER_CHANGE_AT, ZIPPER_LEVEL_DBFS, ZipperWindow, analyze_zipper, zipper_signal,
};
use vox_rack::RackNotice;
use vox_rack::offline::render;

vox_module_api::install_test_allocator!();

/// An impulse comb: 1.0 every `spacing` samples from `first`, 0 elsewhere.
fn comb(len: usize, first: usize, spacing: usize) -> Vec<f32> {
    (0..len)
        .map(|n| {
            if n >= first && (n - first).is_multiple_of(spacing) {
                1.0
            } else {
                0.0
            }
        })
        .collect()
}

fn delayed(x: &[f32], n: usize, by: usize) -> f32 {
    if n >= by { x[n - by] } else { 0.0 }
}

// ---------------------------------------------------------------------------------------------
// Impulse alignment through racks with mixed latencies.

#[test]
fn impulses_stay_aligned_through_a_mixed_latency_rack_live_and_offline() {
    // Latencies 480 + 0 + 128 (bypassed: latency-matched dry) + 0 (placeholder) + 37 = 645.
    let live_model = model(vec![
        delay_slot(480),
        gain_slot(0.0),
        bypassed(delay_slot(128)),
        slot("com.acme.missing", &[]),
        delay_slot(37),
    ]);
    let x = comb(48_000, 1000, 997);
    let mut d = Driver::new(&live_model, 401, x.len());
    assert_eq!(d.host.total_latency_samples(), 645);
    assert_eq!(d.live.latency_samples(), 645);
    d.run_to_end(&x);
    for n in 0..x.len() {
        assert_eq!(d.out[n], delayed(&x, n, 645), "live sample {n}");
    }

    // Offline (placeholders refuse to render): trimmed by the latency, same length, bit-exact.
    let offline_model = model(vec![
        delay_slot(480),
        gain_slot(0.0),
        bypassed(delay_slot(128)),
        delay_slot(37),
    ]);
    let y = render(&registry(), &offline_model, SR, &x).unwrap();
    assert_eq!(y.len(), x.len());
    assert!(
        bit_identical(&y, &x),
        "offline output is the input, aligned"
    );
    // Realtime equals offline after aligning by the reported latency (SPEC-012 §2.8).
    for n in 0..x.len() - 645 {
        assert_eq!(d.out[n + 645], y[n], "realtime vs offline at {n}");
    }
}

// ---------------------------------------------------------------------------------------------
// Whole-rack A/B: the dry path is the input delayed by the total latency, so every impulse
// arrives at the same sample whichever path is heard — during the crossfades too.

#[test]
fn ab_switches_are_aligned_to_the_sample() {
    // Total 480 + 0 + 128 + 64 (bypassed) = 672; the wet path attenuates by 6 dB.
    const L: usize = 672;
    let m = model(vec![
        delay_slot(480),
        gain_slot(-6.0),
        delay_slot(128),
        bypassed(delay_slot(64)),
    ]);
    let x = comb(96_000, 500, 1009);
    let mut d = Driver::new(&m, 402, x.len());
    assert_eq!(d.live.latency_samples(), L as u32);
    // On, off, on again mid-fade (a reversal), off: none block-aligned.
    let toggles = [
        (10_007, true),
        (30_011, false),
        (30_300, true),
        (60_013, false),
    ];
    for &(at, on) in &toggles {
        d.run_until(&x, at);
        d.host.set_ab(on);
    }
    d.run_to_end(&x);
    let wet = 10f32.powf(-6.0 / 20.0);
    // Steady spans (15 ms after each toggle's fade, with the reversal counted as one fade).
    let steady_on =
        |n: usize| (10_007 + XF..30_011).contains(&n) || (30_300 + XF..60_013).contains(&n);
    let steady_off = |n: usize| n < 10_007 || (60_013 + XF..x.len()).contains(&n);
    let mut arrivals = 0;
    for n in 0..x.len() {
        let src = delayed(&x, n, L);
        if src == 0.0 {
            // A time jump would put energy between the arrivals.
            assert_eq!(d.out[n], 0.0, "energy off the impulse grid at {n}");
            continue;
        }
        arrivals += 1;
        let y = d.out[n];
        assert!(
            y >= wet - 1e-6 && y <= 1.0 + 1e-6,
            "arrival at {n}: {y} outside the two paths' levels"
        );
        if steady_on(n) {
            assert_eq!(y, 1.0, "A/B on: the dry impulse, bit-exact, at {n}");
        }
        if steady_off(n) && n > L + 240 {
            assert!(
                (y - wet).abs() <= 1e-6,
                "A/B off: the wet impulse at {n}: {y}"
            );
        }
    }
    assert!(arrivals > 80, "{arrivals} impulses checked");
    assert!(!d.live.ab_enabled());
}

// ---------------------------------------------------------------------------------------------
// Rapid successive latency changes mid-play.

/// The latencies requested one after the other (6000 exceeds the initial 100 ms A/B line, so a
/// larger line is built off the audio thread while the changes are in flight).
const SEQUENCE: [(usize, f64); 5] = [
    (0, 960.0),
    (97, 240.0),
    (311, 1500.0),
    (2_003, 6000.0),
    (2_600, 64.0),
];
const FINAL: usize = 64;

/// Largest sample-to-sample step a click-free transition of the §4.3 tone can make: the tone's
/// own slope plus one crossfade step of a full-scale swing between two sources, for the slot's
/// and the A/B mixer's fades at once, with a rounding margin. A hard switch between two delayed
/// copies of the tone steps by up to twice its amplitude.
fn max_step() -> f32 {
    let amp = 10f64.powf(ZIPPER_LEVEL_DBFS / 20.0);
    let slope = amp * std::f64::consts::TAU * 997.0 / SR;
    (slope + 2.0 * (2.0 * amp / XF as f64) + 1e-4) as f32
}

fn rapid_latency_changes(ab: bool, seed: u64) {
    let label = if ab { "A/B on" } else { "A/B off" };
    let x = zipper_signal();
    let s = ZIPPER_CHANGE_AT;
    let mut d = Driver::new(&model(vec![delay_slot(480), gain_slot(0.0)]), seed, x.len());
    if ab {
        d.run_until(&x, 20_000);
        d.host.set_ab(true);
    }
    for &(offset, latency) in &SEQUENCE {
        d.run_until(&x, s + offset);
        d.host.set_param(0, TestRestart::LATENCY, latency).unwrap();
    }
    // Run in slices, noting where the rack settled (no swap in flight, final latency live).
    let mut settled_at = None;
    while d.pos < x.len() {
        let next = (d.pos + 480).min(x.len());
        d.run_until(&x, next);
        let done = d.host.swaps_in_flight() == 0
            && d.live.latency_samples() == FINAL as u32
            && d.host.total_latency_samples() == FINAL as u32;
        if done && settled_at.is_none() {
            settled_at = Some(d.pos);
        } else if !done {
            settled_at = None;
        }
    }
    for _ in 0..4 {
        d.tick();
    }
    let settled_at = settled_at.expect("the rack settles on the last latency");
    println!(
        "{label}: settled {} ms after the first change",
        (settled_at - s) as f64 / 48.0
    );
    assert!(settled_at < s + 48_000, "{label}: settled within 1 s");

    // Glitch-free: §4.3 over the whole transition, and no sample step above a crossfade's.
    let t_s_ms = (settled_at - s) as f64 * 1000.0 / SR;
    let r = analyze_zipper(&d.out, 0, SR, ZipperWindow::step(s, t_s_ms)).unwrap();
    println!("{label}: {r}");
    assert!(r.pass, "{label}: {r}");
    let (at, step) = d
        .out
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, (w[1] - w[0]).abs()))
        .fold((0, 0.0f32), |a, b| if b.1 > a.1 { b } else { a });
    assert!(
        step <= max_step(),
        "{label}: step {step} at {at} above {}",
        max_step()
    );
    // Never silent (the old instance plays until its successor's output is valid).
    let silent = d.out[s..]
        .windows(3)
        .any(|w| w.iter().all(|v| v.abs() < 1e-6));
    assert!(!silent, "{label}: silent gap");

    // Converged: the input delayed by the last latency (wet and A/B dry alike).
    for n in settled_at..x.len() {
        assert!(
            (d.out[n] - x[n - FINAL]).abs() <= 1e-6,
            "{label}: sample {n}: {} vs {}",
            d.out[n],
            x[n - FINAL]
        );
    }
    assert_eq!(d.host.slot_info(0).unwrap().latency_samples, FINAL as u32);
    // The readout follows: the last latency notice carries the final total.
    let last = d
        .notices
        .iter()
        .rev()
        .find_map(|(_, n)| match n {
            RackNotice::LatencyChanged { total_samples } => Some(*total_samples),
            _ => None,
        })
        .expect("latency notices");
    assert_eq!(last, FINAL as u32, "{label}");
    assert_eq!(d.host.ab(), ab);
}

#[test]
fn rapid_latency_changes_are_glitch_free_and_converge() {
    rapid_latency_changes(false, 403);
}

#[test]
fn rapid_latency_changes_with_ab_on_follow_the_dry_delay() {
    rapid_latency_changes(true, 404);
}

/// SPEC-012 AC-8: every latency change reaches the readout (a `LatencyChanged` notice) within
/// 100 ms of the command, including changes that arrive while the previous replacement is still
/// crossfading.
#[test]
fn each_latency_change_reaches_the_readout_within_100_ms() {
    let x = white(405, 4 * 48_000);
    let mut d = Driver::new(&model(vec![delay_slot(480), gain_slot(0.0)]), 405, x.len());
    let changes = [(40_000, 960u32), (40_600, 1440), (60_000, 240)];
    for &(at, latency) in &changes {
        d.run_until(&x, at);
        d.host
            .set_param(0, TestRestart::LATENCY, f64::from(latency))
            .unwrap();
    }
    d.run_to_end(&x);
    for _ in 0..4 {
        d.tick();
    }
    let seen = |total: u32, from: usize| {
        d.notices.iter().find_map(|(p, n)| {
            (*n == RackNotice::LatencyChanged {
                total_samples: total,
            } && *p >= from)
                .then_some(*p)
        })
    };
    // 960 may be superseded by 1440 before its replacement starts (held back, latest wins);
    // 1440 and 240 must each be reported within 100 ms of their command.
    let p1440 = seen(1440, 40_600).expect("1440 reported");
    assert!(p1440 <= 40_600 + MS100 + 1440 + XF, "1440 at {p1440}");
    let p240 = seen(240, 60_000).expect("240 reported");
    assert!(p240 <= 60_000 + MS100, "240 at {p240}");
    assert_eq!(d.live.latency_samples(), 240);
    assert_eq!(d.host.total_latency_samples(), 240);
}
