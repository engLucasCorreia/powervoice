//! T-110: export encode throughput — WAV (16/24-bit dithered, 32-bit float), FLAC (16/24-bit)
//! and MP3 (runtime-loaded LAME, ADR-007 §4; skipped when `mp3_available()` is false, e.g. no
//! system `libmp3lame`). No PROMPT/SPEC number names encode throughput directly — reported
//! informationally (`target: None`), but it bears on export/bake responsiveness.

use std::sync::OnceLock;
use std::time::Instant;

use divan::Bencher;
use vox_io::{
    BitDepth, DitherMode, FlacBitDepth, Mp3Settings, encode_mp3, mp3_available, write_flac,
    write_wav,
};
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
const CORPUS_SECONDS: f64 = 20.0;

fn corpus() -> &'static [f32] {
    static C: OnceLock<Vec<f32>> = OnceLock::new();
    C.get_or_init(|| {
        let voice = signal::voice_like(0xC051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, RATE)
            .expect("voice_like");
        let pink = signal::pink_noise(0xC052, -30.0, CORPUS_SECONDS, RATE).expect("pink noise");
        voice
            .iter()
            .zip(&pink)
            .map(|(a, b)| a + b)
            .collect::<Vec<f32>>()
    })
    .as_slice()
}

/// A scratch file path removed on drop, even on panic (best-effort).
struct ScratchFile(std::path::PathBuf);

impl ScratchFile {
    fn new(tag: &str, ext: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "vox-bench-encode-{tag}-{}.{ext}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[divan::bench]
fn wav_int16(bencher: Bencher) {
    let f = ScratchFile::new("divan-wav16", "wav");
    bencher.bench_local(|| {
        write_wav(&f.0, RATE, BitDepth::Int16, DitherMode::Tpdf, corpus()).expect("write_wav");
    });
}

#[divan::bench]
fn wav_float32(bencher: Bencher) {
    let f = ScratchFile::new("divan-wav32", "wav");
    bencher.bench_local(|| {
        write_wav(&f.0, RATE, BitDepth::Float32, DitherMode::Tpdf, corpus()).expect("write_wav");
    });
}

#[divan::bench]
fn flac_int16(bencher: Bencher) {
    let f = ScratchFile::new("divan-flac16", "flac");
    bencher.bench_local(|| {
        write_flac(&f.0, RATE, FlacBitDepth::Int16, DitherMode::Tpdf, corpus())
            .expect("write_flac");
    });
}

/// Seconds of audio encoded per second of wall time, best of a few passes; `None` if the format
/// can't run here (MP3 with no `libmp3lame`).
fn realtime_factor(
    encode: impl Fn(&std::path::Path) -> vox_io::Result<()>,
    tag: &str,
) -> Option<f64> {
    const PASSES: usize = 3;
    let input = corpus();
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let f = ScratchFile::new(tag, "bin");
        let started = Instant::now();
        encode(&f.0).ok()?;
        best = best.min(started.elapsed().as_secs_f64());
    }
    Some((input.len() as f64 / f64::from(RATE)) / best)
}

type Encode = fn(&std::path::Path) -> vox_io::Result<()>;

fn report() {
    println!(
        "export encode throughput, {CORPUS_SECONDS} s of voice-like + pink noise, realtime \
         factor (audio seconds encoded per wall second)"
    );
    let cases: [(&str, Encode); 3] = [
        ("wav_int16", |p| {
            write_wav(p, RATE, BitDepth::Int16, DitherMode::Tpdf, corpus()).map(|_| ())
        }),
        ("wav_float32", |p| {
            write_wav(p, RATE, BitDepth::Float32, DitherMode::Tpdf, corpus()).map(|_| ())
        }),
        ("flac_int16", |p| {
            write_flac(p, RATE, FlacBitDepth::Int16, DitherMode::Tpdf, corpus()).map(|_| ())
        }),
    ];
    for (name, encode) in cases {
        match realtime_factor(encode, name) {
            Some(factor) => {
                println!("  {name}: {factor:.1}x realtime");
                bench_report::result(
                    "vox-io",
                    &format!("encode_{name}_realtime_factor"),
                    factor,
                    "realtime_x",
                    None,
                );
            }
            None => println!("  {name}: failed (unexpected)"),
        }
    }
    if mp3_available() {
        let factor = realtime_factor(
            |p| encode_mp3(p, RATE, corpus(), Mp3Settings::ACX).map(|_| ()),
            "mp3",
        )
        .expect("libmp3lame reported available but encode failed");
        println!("  mp3_acx_192kbps: {factor:.1}x realtime");
        bench_report::result(
            "vox-io",
            "encode_mp3_acx_192kbps_realtime_factor",
            factor,
            "realtime_x",
            None,
        );
    } else {
        println!("  mp3_acx_192kbps: skipped (no libmp3lame on this machine, ADR-007 §4)");
    }
}

fn main() {
    report();
    divan::main();
}
