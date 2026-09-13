//! `powervoice-cli`: the DSP acceptance tool. `gen` writes synthetic test
//! signals to WAV, `analyze` reports objective measurements on a WAV file,
//! `render --rack` renders a mono WAV through a rack file with the same code as
//! the engine's offline render (SPEC-012 §2.8). `bench` is a stub (T-110).
//!
//! ## `analyze --json` schema and non-finite/missing values
//! Every numeric field in the JSON report is a plain JSON number, *except*
//! that a value can come out as JSON `null` for three different reasons,
//! which the text (non-JSON) report spells out explicitly instead:
//! - **Exact digital silence** (a measurement over all-zero samples, e.g.
//!   `peak_dbfs`/`rms_db` of true silence): the real value is `-inf`, which
//!   JSON has no literal for. Text output prints `-inf`.
//! - **Invalid input** (`NaN`/infinite samples in the source audio): the
//!   measurement is `NaN`, again with no JSON literal. Text output prints
//!   `NaN`.
//! - **Not enough audio** for the measurement's window (noise floor needs
//!   500 ms; max momentary needs 400 ms; max short-term needs 3 s): these
//!   fields are `Option`-typed and `None`. Text output prints `n/a`.
//!
//! JSON consumers that need to tell these apart should treat `null` as
//! "unusable" and, if they must distinguish further, re-derive it from
//! `duration_s` (too-short case) vs. re-checking the source audio for
//! non-finite samples.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use vox_rack::{RackModel, Registry, offline};
use vox_testkit::wav::{BitDepth, SampleFormat};
use vox_testkit::{measure, signal, wav};

#[derive(Parser)]
#[command(name = "powervoice-cli")]
#[command(version)]
#[command(about, long_about = None)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a deterministic test signal to WAV.
    Gen(GenArgs),
    /// Report objective measurements (peak/RMS/loudness/noise floor/...) for a WAV file.
    Analyze(AnalyzeArgs),
    /// Render a mono WAV through an effects rack (32-bit float output, same length and
    /// time-aligned with the input).
    Render(RenderArgs),
    /// Benchmark DSP modules (T-110).
    Bench,
}

#[derive(Clone, Copy, ValueEnum)]
enum SignalArg {
    Sine,
    Sweep,
    White,
    Pink,
    Bursts,
    Silence,
    Impulse,
    Voice,
    Tech3341,
}

#[derive(Clone, Copy, ValueEnum)]
enum BitsArg {
    #[value(name = "16")]
    Bits16,
    #[value(name = "24")]
    Bits24,
    #[value(name = "32f")]
    Bits32f,
}

impl From<BitsArg> for BitDepth {
    fn from(bits: BitsArg) -> Self {
        match bits {
            BitsArg::Bits16 => BitDepth::Int16,
            BitsArg::Bits24 => BitDepth::Int24,
            BitsArg::Bits32f => BitDepth::Float32,
        }
    }
}

#[derive(clap::Args)]
struct GenArgs {
    /// Signal to generate.
    signal: SignalArg,

    /// Output path, or "-" for stdout.
    #[arg(short, long)]
    output: PathBuf,

    /// Sample rate in Hz.
    #[arg(long, default_value_t = 48_000)]
    rate: u32,

    /// Output bit depth.
    #[arg(long, value_enum, default_value_t = BitsArg::Bits32f)]
    bits: BitsArg,

    /// Duration in seconds.
    #[arg(long, default_value_t = 1.0, allow_hyphen_values = true)]
    duration: f64,

    /// Tone frequency in Hz (sine / bursts / voice), or the sweep start
    /// frequency (sweep).
    #[arg(long, default_value_t = 1000.0)]
    freq: f64,

    /// Sweep end frequency in Hz (sweep only).
    #[arg(long)]
    freq_end: Option<f64>,

    /// Phase offset in degrees (sine only).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    phase: f64,

    /// Level in dBFS: peak for sine/sweep/bursts/voice/tech3341, target RMS
    /// for white/pink noise.
    #[arg(long, default_value_t = -20.0, allow_hyphen_values = true)]
    level: f64,

    /// PRNG seed (white / pink / bursts gap noise / voice), for determinism.
    #[arg(long, default_value_t = 0)]
    seed: u64,

    /// Burst "on" duration in seconds (bursts / voice).
    #[arg(long)]
    on: Option<f64>,

    /// Burst "off" duration in seconds (bursts / voice).
    #[arg(long)]
    off: Option<f64>,

    /// RMS dBFS of white noise filling the gaps between bursts (bursts
    /// only); omit for pure silence in the gaps.
    #[arg(long, allow_hyphen_values = true)]
    gap_noise_level: Option<f64>,

    /// Target SNR in dB, tone RMS vs. background noise RMS (voice only).
    #[arg(long, default_value_t = 20.0, allow_hyphen_values = true)]
    snr: f64,

    /// EBU Tech 3341 case: 1 (-23 dBFS/ch) or 2 (-33 dBFS/ch) (tech3341 only).
    #[arg(long, default_value_t = 1)]
    case: u8,
}

#[derive(clap::Args)]
struct AnalyzeArgs {
    /// Input WAV path, or "-" for stdin.
    input: PathBuf,

    /// Print measurements as JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Gen(args)) => cmd_gen(args),
        Some(Commands::Analyze(args)) => cmd_analyze(args),
        Some(Commands::Render(args)) => cmd_render(args),
        Some(Commands::Bench) => {
            println!("bench: not implemented yet (T-110)");
            Ok(())
        }
        None => unreachable!("arg_required_else_help prints help and exits before this point"),
    }
}

#[derive(clap::Args)]
struct RenderArgs {
    /// Rack file: `{ "slots": [...] }` or a bare slot array, in the sidecar's slot schema.
    #[arg(long)]
    rack: PathBuf,

    /// Input WAV (mono).
    input: PathBuf,

    /// Output WAV (32-bit float, input rate).
    output: PathBuf,
}

/// The composition root's registry: every built-in module (SPEC-012 §2.10).
fn builtin_registry() -> Result<Registry> {
    Ok(Registry::with_factories(vox_modules::builtin_factories())?)
}

fn cmd_render(args: RenderArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.rack)
        .with_context(|| format!("reading rack file {:?}", args.rack))?;
    let model = RackModel::from_json(&text)
        .with_context(|| format!("parsing rack file {:?}", args.rack))?;
    let registry = builtin_registry()?;
    let missing = registry.missing_ids(&model);
    if !missing.is_empty() {
        bail!(
            "unknown module id(s): {} (installed: {})",
            missing.join(", "),
            registry.ids().join(", ")
        );
    }
    let (samples, info) = wav::read_wav_file(&args.input)
        .with_context(|| format!("reading WAV input {:?}", args.input))?;
    if info.channels != 1 {
        bail!(
            "render --rack needs a mono WAV, {:?} has {} channels",
            args.input,
            info.channels
        );
    }
    let out = offline::render(&registry, &model, f64::from(info.sample_rate), &samples)
        .context("rendering")?;
    wav::write_wav_file(&args.output, &out, 1, info.sample_rate, BitDepth::Float32)
        .with_context(|| format!("writing {:?}", args.output))?;
    Ok(())
}

fn cmd_gen(args: GenArgs) -> Result<()> {
    let (samples, channels): (Vec<f32>, u16) = match args.signal {
        SignalArg::Sine => (
            signal::sine_with_phase(args.freq, args.level, args.phase, args.duration, args.rate)?,
            1,
        ),
        SignalArg::Sweep => {
            let freq_end = args
                .freq_end
                .context("gen sweep requires --freq-end <hz>")?;
            (
                signal::log_sweep(args.freq, freq_end, args.level, args.duration, args.rate)?,
                1,
            )
        }
        SignalArg::White => (
            signal::white_noise(args.seed, args.level, args.duration, args.rate)?,
            1,
        ),
        SignalArg::Pink => (
            signal::pink_noise(args.seed, args.level, args.duration, args.rate)?,
            1,
        ),
        SignalArg::Bursts => {
            let on_s = args.on.unwrap_or(1.0);
            let off_s = args.off.unwrap_or(1.0);
            (
                signal::tone_bursts(
                    args.seed,
                    args.freq,
                    args.level,
                    args.gap_noise_level,
                    on_s,
                    off_s,
                    args.duration,
                    args.rate,
                )?,
                1,
            )
        }
        SignalArg::Silence => (signal::silence(args.duration, args.rate)?, 1),
        SignalArg::Impulse => (signal::impulse(args.level, 0, args.duration, args.rate)?, 1),
        SignalArg::Voice => {
            let on_s = args.on.unwrap_or(0.3);
            let off_s = args.off.unwrap_or(0.2);
            (
                signal::voice_like(
                    args.seed,
                    args.freq,
                    args.level,
                    args.snr,
                    on_s,
                    off_s,
                    args.duration,
                    args.rate,
                )?,
                1,
            )
        }
        SignalArg::Tech3341 => (
            signal::tech3341_case(args.case, args.duration, args.rate)?,
            2,
        ),
    };

    let bits: BitDepth = args.bits.into();
    let clipped = if args.output.to_str() == Some("-") {
        let (bytes, clipped) = wav::encode_wav_bytes(&samples, channels, args.rate, bits)?;
        io::stdout().write_all(&bytes)?;
        clipped
    } else {
        wav::write_wav_file(&args.output, &samples, channels, args.rate, bits)?
    };
    if clipped > 0 {
        eprintln!(
            "warning: {clipped} of {} sample(s) exceeded full scale (|x| > 1.0) \
             and were clamped for {:?} output",
            samples.len(),
            args.bits_debug_name()
        );
    }
    Ok(())
}

impl GenArgs {
    /// A short name for the chosen bit depth, for the clipping warning.
    fn bits_debug_name(&self) -> &'static str {
        match self.bits {
            BitsArg::Bits16 => "16-bit int",
            BitsArg::Bits24 => "24-bit int",
            BitsArg::Bits32f => "32-bit float",
        }
    }
}

#[derive(Serialize)]
struct AnalyzeReport {
    duration_s: f64,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    sample_format: String,
    peak_dbfs: f64,
    rms_db: f64,
    crest_factor_db: f64,
    dc_offset: f64,
    /// `None` if the input is shorter than the 500 ms noise-floor window.
    noise_floor_db: Option<f64>,
    integrated_lufs: f64,
    /// `None` if the input is shorter than the 400 ms momentary window.
    max_momentary_lufs: Option<f64>,
    /// `None` if the input is shorter than the 3 s short-term window.
    max_short_term_lufs: Option<f64>,
    lra_lu: f64,
    true_peak_dbtp: f64,
}

/// Formats an `f64` measurement for the human-readable report: finite
/// values as fixed-point, `-inf`/`NaN` via their normal `Display` (which
/// already prints exactly `-inf` / `NaN`, ignoring the precision spec).
fn fmt_db(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.2}")
    } else {
        format!("{value}")
    }
}

/// As [`fmt_db`], but `None` (measurement window longer than the input)
/// prints as `n/a`.
fn fmt_db_opt(value: Option<f64>) -> String {
    match value {
        Some(v) => fmt_db(v),
        None => "n/a".to_string(),
    }
}

fn cmd_analyze(args: AnalyzeArgs) -> Result<()> {
    let (samples, info) = if args.input.to_str() == Some("-") {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        wav::read_wav(io::Cursor::new(bytes))
    } else {
        wav::read_wav_file(&args.input)
    }
    .with_context(|| format!("reading WAV input {:?}", args.input))?;

    let loudness = measure::loudness(&samples, u32::from(info.channels), info.sample_rate)
        .context("measuring loudness")?;

    let report = AnalyzeReport {
        duration_s: info.duration_s(samples.len()),
        sample_rate: info.sample_rate,
        channels: info.channels,
        bits_per_sample: info.bits_per_sample,
        sample_format: match info.sample_format {
            SampleFormat::Int => "int".to_string(),
            SampleFormat::Float => "float".to_string(),
        },
        peak_dbfs: measure::peak_dbfs(&samples),
        rms_db: measure::rms_db(&samples),
        crest_factor_db: measure::crest_factor_db(&samples),
        dc_offset: measure::dc_offset(&samples),
        noise_floor_db: measure::noise_floor_db(
            &samples,
            info.channels,
            info.sample_rate,
            measure::DEFAULT_NOISE_FLOOR_WINDOW_MS,
        ),
        integrated_lufs: loudness.integrated_lufs,
        max_momentary_lufs: loudness.max_momentary_lufs,
        max_short_term_lufs: loudness.max_short_term_lufs,
        lra_lu: loudness.lra_lu,
        true_peak_dbtp: loudness.true_peak_dbtp,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("duration:        {:.3} s", report.duration_s);
        println!(
            "format:          {} Hz, {} ch, {}-bit {}",
            report.sample_rate, report.channels, report.bits_per_sample, report.sample_format
        );
        println!("peak:            {} dBFS", fmt_db(report.peak_dbfs));
        println!("rms:             {} dB", fmt_db(report.rms_db));
        println!("crest factor:    {} dB", fmt_db(report.crest_factor_db));
        println!("dc offset:       {:.6}", report.dc_offset);
        println!("noise floor:     {} dB", fmt_db_opt(report.noise_floor_db));
        println!("integrated:      {} LUFS", fmt_db(report.integrated_lufs));
        println!(
            "max momentary:   {} LUFS",
            fmt_db_opt(report.max_momentary_lufs)
        );
        println!(
            "max short-term:  {} LUFS",
            fmt_db_opt(report.max_short_term_lufs)
        );
        println!("LRA:             {} LU", fmt_db(report.lra_lu));
        println!("true peak:       {} dBTP", fmt_db(report.true_peak_dbtp));
    }
    Ok(())
}
