//! T-110: `StreamResampler` throughput (ADR-002 §5: the playback reader's document → device
//! rate conversion, `rubato::Fft`, allocation-free after construction). No PROMPT/SPEC number
//! names resampler throughput directly — reported informationally (`target: None`); it only
//! matters in the (uncommon, ADR-002 §5) fallback where the output device can't run at the
//! document rate.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use divan::Bencher;
use vox_dsp::resample::StreamResampler;
use vox_testkit::bench_report;
use vox_testkit::signal;

const CORPUS_SECONDS: f64 = 5.0;
/// Typical device callback size.
const OUT_BLOCKS: [usize; 3] = [128, 512, 1024];
/// `(in_hz, out_hz)`: the common SPEC-000 §2.4 fallback (44.1 kHz document, 48 kHz device) and
/// its reverse.
const RATE_PAIRS: [(u32, u32); 2] = [(44_100, 48_000), (48_000, 44_100)];

fn corpus(rate_hz: u32) -> Vec<f32> {
    signal::voice_like(0x9051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, rate_hz)
        .expect("voice_like")
}

fn corpus_44100() -> &'static [f32] {
    static C: OnceLock<Vec<f32>> = OnceLock::new();
    C.get_or_init(|| corpus(44_100)).as_slice()
}

fn corpus_48000() -> &'static [f32] {
    static C: OnceLock<Vec<f32>> = OnceLock::new();
    C.get_or_init(|| corpus(48_000)).as_slice()
}

fn corpus_for(rate_hz: u32) -> &'static [f32] {
    match rate_hz {
        44_100 => corpus_44100(),
        48_000 => corpus_48000(),
        _ => unreachable!("only the two RATE_PAIRS rates are used"),
    }
}

struct BenchState {
    resampler: StreamResampler,
    input: &'static [f32],
    pos: Cell<usize>,
    out: Vec<f32>,
}

fn pull_one(s: &mut BenchState) {
    let input = s.input;
    let pos = &s.pos;
    s.resampler
        .pull(&mut s.out, |chunk| {
            for x in chunk.iter_mut() {
                *x = input[pos.get() % input.len()];
                pos.set(pos.get() + 1);
            }
        })
        .expect("pull");
}

#[divan::bench(args = RATE_PAIRS)]
fn rate_pair(bencher: Bencher, (in_hz, out_hz): (u32, u32)) {
    bencher
        .with_inputs(|| BenchState {
            resampler: StreamResampler::new(in_hz, out_hz).expect("construct"),
            input: corpus_for(in_hz),
            pos: Cell::new(0),
            out: vec![0.0f32; 512],
        })
        .bench_local_refs(pull_one);
}

/// Seconds of output produced per second of wall time, best of a few passes.
fn realtime_factor(in_hz: u32, out_hz: u32, out_block: usize) -> f64 {
    const PASSES: usize = 3;
    let input = corpus_for(in_hz);
    let total_out = (input.len() as f64 * f64::from(out_hz) / f64::from(in_hz)).round() as usize;
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut resampler = StreamResampler::new(in_hz, out_hz).expect("construct");
        let mut out = vec![0.0f32; out_block];
        let pos = Cell::new(0usize);
        let started = Instant::now();
        let mut produced = 0usize;
        while produced < total_out {
            let n = out_block.min(total_out - produced);
            resampler
                .pull(&mut out[..n], |chunk| {
                    for x in chunk.iter_mut() {
                        *x = input[pos.get() % input.len()];
                        pos.set(pos.get() + 1);
                    }
                })
                .expect("pull");
            std::hint::black_box(&out);
            produced += n;
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    (total_out as f64 / f64::from(out_hz)) / best
}

fn report() {
    println!(
        "StreamResampler throughput, {CORPUS_SECONDS} s of voice-like input, realtime factor \
         (output seconds produced per wall second)"
    );
    for (in_hz, out_hz) in RATE_PAIRS {
        for block in OUT_BLOCKS {
            let factor = realtime_factor(in_hz, out_hz, block);
            println!(
                "  {in_hz} Hz -> {out_hz} Hz, {block}-frame output pulls: {factor:.1}x realtime"
            );
            bench_report::result(
                "vox-dsp",
                &format!("resample_{in_hz}_to_{out_hz}_{block}f_realtime_factor"),
                factor,
                "realtime_x",
                None,
            );
        }
    }
}

fn main() {
    report();
    divan::main();
}
