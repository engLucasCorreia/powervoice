//! SPEC-017 AC-17: the True-Peak Limiter's CPU cost at 48 kHz in 256-frame blocks, as a
//! percentage of one core (budget ≤ 1.5 %, §4.5). Run by `just bench` (release); fails when over
//! budget.
//!
//! Input: 20 s of `voice_like` plus pink noise, at 0, +12 and +24 dB input gain (transparent,
//! limiting, heavy limiting). Best of 5 passes per setting, each on a fresh instance.

use std::time::Instant;

use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, OutputEvents, ProcessContext, ProcessMode, Transport,
    prepare_state,
};
use vox_modules::TruePeakLimiter;
use vox_testkit::signal;

const RATE: u32 = 48_000;
const BLOCK: usize = 256;
const SECONDS: f64 = 20.0;
const PASSES: usize = 5;
const BUDGET_PERCENT: f64 = 1.5;

fn limiter(input_gain_db: f64) -> Box<dyn Module> {
    let mut m: Box<dyn Module> = Box::new(TruePeakLimiter::new());
    let mut state = m.save_state().expect("state");
    state.params.insert("input_gain_db".into(), input_gain_db);
    let state = prepare_state(&*m, state).expect("prepared state");
    m.load_state(&state).expect("load state");
    m.activate(&ActivateConfig {
        sample_rate: f64::from(RATE),
        max_block: BLOCK as u32,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    })
    .expect("activate");
    m
}

/// Seconds of processing per second of audio, best of [`PASSES`].
fn cost(input: &[f32], input_gain_db: f64) -> f64 {
    let mut out = vec![0.0f32; BLOCK];
    let mut out_events = OutputEvents::with_capacity(8);
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut m = limiter(input_gain_db);
        let started = Instant::now();
        for (i, chunk) in input.chunks(BLOCK).enumerate() {
            let pos = (i * BLOCK) as u64;
            out_events.clear();
            let mut ctx = ProcessContext::new(
                chunk.len() as u32,
                pos,
                Transport {
                    playing: true,
                    position_samples: Some(pos),
                },
                &[],
                &mut out_events,
            );
            m.process(&mut ctx, &[chunk], &mut [&mut out[..chunk.len()]]);
            std::hint::black_box(&out);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    best / (input.len() as f64 / f64::from(RATE))
}

fn main() {
    // `cargo bench` passes `--bench` (and any filter); this harness takes no arguments.
    let voice =
        signal::voice_like(0xB0, 200.0, -3.0, 20.0, 0.2, 0.1, SECONDS, RATE).expect("voice_like");
    let pink = signal::pink_noise(0xB1, -20.0, SECONDS, RATE).expect("pink noise");
    let x: Vec<f32> = voice.iter().zip(&pink).map(|(a, b)| a + b).collect();
    let mut worst = 0.0f64;
    for (label, gain_db) in [
        ("0 dB input gain (transparent)", 0.0),
        ("+12 dB input gain (limiting)", 12.0),
        ("+24 dB input gain (heavy limiting)", 24.0),
    ] {
        let percent = 100.0 * cost(&x, gain_db);
        worst = worst.max(percent);
        println!(
            "true-peak limiter, {RATE} Hz, {BLOCK}-frame blocks, {label}: {percent:.3} % of one \
             core (budget {BUDGET_PERCENT} %)"
        );
    }
    assert!(
        worst <= BUDGET_PERCENT,
        "SPEC-017 AC-17: {worst:.3} % of one core > {BUDGET_PERCENT} %"
    );
}
