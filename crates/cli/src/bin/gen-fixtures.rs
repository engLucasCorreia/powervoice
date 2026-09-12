//! Generates `fixtures/generated/*.wav` for DSP acceptance tests and
//! performance benchmarking (`just fixtures`). Not part of the
//! `powervoice-cli` command surface.

use std::path::{Path, PathBuf};

use anyhow::Result;
use vox_testkit::signal;
use vox_testkit::wav::{self, BitDepth};

const SAMPLE_RATE: u32 = 48_000;

/// Workspace-root-relative, not CWD-relative: `just fixtures` happens to run
/// from the workspace root today, but `cargo run -p powervoice-cli --bin
/// gen-fixtures` from any other directory should still land in the same
/// place rather than creating a stray `fixtures/generated` under wherever
/// the shell happened to be.
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")) // crates/cli
        .join("../../fixtures/generated")
}

fn main() -> Result<()> {
    let out_dir = out_dir();
    std::fs::create_dir_all(&out_dir)?;
    // Cosmetic only: turns the `.../crates/cli/../../fixtures/generated`
    // path used to reach the workspace root into a clean absolute path for
    // the progress messages below.
    let out_dir = out_dir.canonicalize().unwrap_or(out_dir);

    write(
        &out_dir,
        "sine-1khz-minus20dbfs-20s.wav",
        1,
        signal::sine(1000.0, -20.0, 20.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "sweep-20hz-20khz-minus12dbfs-5s.wav",
        1,
        signal::log_sweep(20.0, 20_000.0, -12.0, 5.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "white-noise-minus20dbfs-5s.wav",
        1,
        signal::white_noise(1, -20.0, 5.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "pink-noise-minus20dbfs-5s.wav",
        1,
        signal::pink_noise(2, -20.0, 5.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "tone-bursts-noise-floor-minus70dbfs-20s.wav",
        1,
        signal::tone_bursts(3, 1000.0, -10.0, Some(-70.0), 1.0, 1.0, 20.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "silence-2s.wav",
        1,
        signal::silence(2.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "impulse-0dbfs-1s.wav",
        1,
        signal::impulse(0.0, 0, 1.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "voice-like-snr20db-10s.wav",
        1,
        signal::voice_like(4, 300.0, -18.0, 20.0, 0.3, 0.2, 10.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "tech3341-case1-minus23dbfs-20s.wav",
        2,
        signal::tech3341_case(1, 20.0, SAMPLE_RATE)?,
    )?;
    write(
        &out_dir,
        "tech3341-case2-minus33dbfs-20s.wav",
        2,
        signal::tech3341_case(2, 20.0, SAMPLE_RATE)?,
    )?;

    // Streamed directly to disk (never held whole in memory): 60 min at
    // 48 kHz mono 32-bit float is ~691 MB.
    let long_path = out_dir.join("long-60min-48k-mono.wav");
    println!(
        "writing {} (streamed, ~60 min of audio)...",
        long_path.display()
    );
    signal::write_long_fixture(&long_path, 60.0 * 60.0, SAMPLE_RATE, 42)?;
    println!("wrote {}", long_path.display());

    println!("fixtures written to {}/", out_dir.display());
    Ok(())
}

fn write(out_dir: &Path, name: &str, channels: u16, samples: Vec<f32>) -> Result<()> {
    let path = out_dir.join(name);
    let clipped = wav::write_wav_file(&path, &samples, channels, SAMPLE_RATE, BitDepth::Float32)?;
    if clipped > 0 {
        eprintln!("warning: {clipped} sample(s) in {name} exceeded full scale");
    }
    println!("wrote {}", path.display());
    Ok(())
}
