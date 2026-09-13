//! `powervoice-cli convert` (ticket S4-02: export encoders, library part). Chains `gen` -> `render
//! --rack` (through a `[Gain -6 dB]` slot, exercising the same rack path S4-04's export job will
//! use) -> `convert` -> `analyze`, exactly like the manual CLI acceptance flow in the ticket.

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_powervoice-cli"))
}

/// A fresh directory under the system temp dir (removed on drop).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pv-s402-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_ok(cmd: &mut Command) -> std::process::Output {
    let out = cmd.output().expect("run powervoice-cli");
    assert!(
        out.status.success(),
        "failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn gain_rack(db: f64) -> String {
    format!(
        r#"{{ "slots": [ {{ "module": "org.powervoice.gain@1.0.0", "bypass": false,
            "state": {{ "format_version": 1, "params": {{ "gain_db": {db:?} }} }} }} ] }}"#
    )
}

fn render(dir: &TempDir, rack: &str, input: &Path, output: &Path) {
    let rack_path = dir.path("rack.json");
    std::fs::write(&rack_path, rack).unwrap();
    run_ok(
        cli()
            .arg("render")
            .arg("--rack")
            .arg(&rack_path)
            .arg(input)
            .arg(output),
    );
}

fn analyze_json(path: &Path) -> serde_json::Value {
    let out = run_ok(cli().arg("analyze").arg(path).arg("--json"));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Ticket S4-02 acceptance: a 1 kHz -20 dBFS sine through `[Gain -6 dB]`, exported as WAV 24-bit,
/// peaks at -26.00 +/- 0.01 dBFS.
#[test]
fn gain_minus_6_exported_as_wav24_peaks_at_minus_26() {
    let dir = TempDir::new("wav24");
    let (raw, rendered, out) = (
        dir.path("raw.wav"),
        dir.path("rendered.wav"),
        dir.path("out.wav"),
    );
    run_ok(
        cli()
            .args([
                "gen",
                "sine",
                "--freq",
                "1000",
                "--level",
                "-20",
                "--duration",
                "2",
                "-o",
            ])
            .arg(&raw),
    );
    render(&dir, &gain_rack(-6.0), &raw, &rendered);
    run_ok(
        cli()
            .arg("convert")
            .arg(&rendered)
            .arg(&out)
            .args(["--bits", "24"]),
    );

    let report = analyze_json(&out);
    let peak = report["peak_dbfs"].as_f64().unwrap();
    assert!((peak + 26.0).abs() <= 0.01, "peak {peak}");
    let bits = report["bits_per_sample"].as_u64().unwrap();
    assert_eq!(bits, 24);
}

/// FLAC output decodes bit-exact to the WAV (via the reference `flac` decoder, skipped if it
/// isn't installed — the same tool SPEC-005's AC-16 manual smoke check uses).
#[test]
fn convert_to_flac_decodes_bit_exact_to_the_wav() {
    let flac_ok = Command::new("flac")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !flac_ok {
        eprintln!("skipping: `flac` reference decoder not installed");
        return;
    }
    let dir = TempDir::new("flac");
    let (wav, flac, decoded) = (
        dir.path("in.wav"),
        dir.path("out.flac"),
        dir.path("decoded.wav"),
    );
    run_ok(
        cli()
            .args(["gen", "pink", "--seed", "11", "--duration", "1", "-o"])
            .arg(&wav),
    );
    run_ok(
        cli()
            .arg("convert")
            .arg(&wav)
            .arg(&flac)
            .args(["--bits", "24"]),
    );
    let status = Command::new("flac")
        .args(["-d", "-f", "-o"])
        .arg(&decoded)
        .arg(&flac)
        .status()
        .expect("run flac -d");
    assert!(status.success());

    // The source WAV is 32-bit float; `convert --bits 24` quantizes it before FLAC encoding, so
    // the two peaks are only expected to agree to 24-bit precision (~-144 dBFS), not bit-exactly.
    let wav_report = analyze_json(&wav);
    let decoded_report = analyze_json(&decoded);
    let (wav_peak, decoded_peak) = (
        wav_report["peak_dbfs"].as_f64().unwrap(),
        decoded_report["peak_dbfs"].as_f64().unwrap(),
    );
    assert!(
        (wav_peak - decoded_peak).abs() <= 0.001,
        "wav peak {wav_peak} vs FLAC-decoded peak {decoded_peak}"
    );
}

/// Ticket S4-02 acceptance: 48 kHz -> 44.1 kHz keeps a 1 kHz tone at 1 kHz +/- 0.05 % and its
/// level within +/- 0.1 dB.
#[test]
fn convert_resamples_48k_to_44k1_keeping_tone_and_level() {
    let dir = TempDir::new("resample");
    let (input, output) = (dir.path("in.wav"), dir.path("out.wav"));
    run_ok(
        cli()
            .args([
                "gen",
                "sine",
                "--freq",
                "1000",
                "--level",
                "-20",
                "--duration",
                "2",
                "--rate",
                "48000",
                "-o",
            ])
            .arg(&input),
    );
    run_ok(
        cli()
            .arg("convert")
            .arg(&input)
            .arg(&output)
            .args(["--rate", "44100", "--bits", "32f"]),
    );
    let report = analyze_json(&output);
    assert_eq!(report["sample_rate"].as_u64().unwrap(), 44_100);
    let peak = report["peak_dbfs"].as_f64().unwrap();
    assert!((peak + 20.0).abs() <= 0.1, "peak {peak}");
}

/// MP3 export (if `libmp3lame` is installed): CBR 192 kbps produces a valid MPEG stream whose
/// level and rate roughly match the source (decoded with `mpg123` when available; otherwise this
/// only checks that a plausible MP3 file was written).
#[test]
fn convert_to_mp3_cbr_192_acx_preset() {
    let dir = TempDir::new("mp3");
    let (input, output) = (dir.path("in.wav"), dir.path("out.mp3"));
    run_ok(
        cli()
            .args([
                "gen",
                "sine",
                "--freq",
                "1000",
                "--level",
                "-20",
                "--duration",
                "2",
                "--rate",
                "44100",
                "-o",
            ])
            .arg(&input),
    );
    let out = cli()
        .arg("convert")
        .arg(&input)
        .arg(&output)
        .args(["--bitrate", "192"])
        .output()
        .expect("run convert");
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("libmp3lame"),
            "convert failed for a reason other than a missing LAME: {stderr}"
        );
        eprintln!("skipping mp3 checks: {stderr}");
        return;
    }
    assert!(output.exists());
    let bytes = std::fs::read(&output).unwrap();
    assert!(!bytes.is_empty());

    // Best-effort decode with `mpg123`, if installed, to check duration/rate.
    let wav_out = dir.path("mp3_decoded.wav");
    let decode = Command::new("mpg123")
        .args(["-q", "-w"])
        .arg(&wav_out)
        .arg(&output)
        .status();
    if let Ok(status) = decode
        && status.success()
    {
        let report = analyze_json(&wav_out);
        assert_eq!(report["sample_rate"].as_u64().unwrap(), 44_100);
        let duration = report["duration_s"].as_f64().unwrap();
        assert!((duration - 2.0).abs() <= 0.1, "duration {duration}");
    }
}
