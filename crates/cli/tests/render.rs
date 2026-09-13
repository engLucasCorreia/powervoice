//! `powervoice-cli render --rack` (SPEC-012 AC-10, SPEC-000 AC-3 CLI side).

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
use std::path::{Path, PathBuf};
use std::process::Command;

use vox_rack::{RackModel, Registry, offline};
use vox_testkit::golden::fnv1a_hash;
use vox_testkit::wav::{self, BitDepth, SampleFormat};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_powervoice-cli"))
}

/// A fresh directory under the system temp dir (removed on drop).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pv-t103-{}-{name}", std::process::id()));
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

fn gain_rack(db: f64) -> String {
    format!(
        r#"{{ "slots": [ {{ "module": "org.powervoice.gain@1.0.0", "bypass": false,
            "state": {{ "format_version": 1, "params": {{ "gain_db": {db:?} }} }} }} ] }}"#
    )
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

fn render(dir: &TempDir, rack: &str, input: &Path, output: &Path) -> std::process::Output {
    let rack_path = dir.path("rack.json");
    std::fs::write(&rack_path, rack).unwrap();
    cli()
        .arg("render")
        .arg("--rack")
        .arg(&rack_path)
        .arg(input)
        .arg(output)
        .output()
        .expect("run render")
}

#[test]
fn gain_minus_6_on_a_minus_20_dbfs_sine_analyzes_to_minus_26() {
    let dir = TempDir::new("gain");
    let (input, output) = (dir.path("in.wav"), dir.path("out.wav"));
    run_ok(
        cli()
            .args([
                "gen",
                "sine",
                "--freq",
                "997",
                "--level",
                "-20",
                "--duration",
                "3",
                "-o",
            ])
            .arg(&input),
    );
    let out = render(&dir, &gain_rack(-6.0), &input, &output);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (samples, info) = wav::read_wav_file(&output).unwrap();
    assert_eq!(
        (info.channels, info.sample_rate, info.bits_per_sample),
        (1, 48_000, 32)
    );
    assert_eq!(info.sample_format, SampleFormat::Float);
    assert_eq!(samples.len(), 3 * 48_000);
    let report = run_ok(cli().arg("analyze").arg(&output).arg("--json"));
    let v: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    let peak = v["peak_dbfs"].as_f64().unwrap();
    assert!((peak + 26.0).abs() <= 0.01, "peak {peak}");
}

#[test]
fn output_is_same_length_and_time_aligned() {
    let dir = TempDir::new("impulse");
    let (input, output) = (dir.path("in.wav"), dir.path("out.wav"));
    let x = vox_testkit::signal::impulse(0.0, 1000, 1.0, 48_000).unwrap();
    wav::write_wav_file(&input, &x, 1, 48_000, BitDepth::Float32).unwrap();
    let out = render(&dir, &gain_rack(0.0), &input, &output);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (y, _) = wav::read_wav_file(&output).unwrap();
    assert_eq!(y.len(), 48_000);
    assert_eq!(y[1000], 1.0);
    assert_eq!(y.iter().filter(|s| **s != 0.0).count(), 1);
}

#[test]
fn unknown_module_is_an_error_listing_the_id() {
    let dir = TempDir::new("unknown");
    let (input, output) = (dir.path("in.wav"), dir.path("out.wav"));
    run_ok(
        cli()
            .args(["gen", "sine", "--duration", "0.1", "-o"])
            .arg(&input),
    );
    let rack = r#"[ { "module": "com.acme.x@2.0.0", "bypass": false, "state": {} },
                    { "module": "org.powervoice.gain@1.0.0", "bypass": false } ]"#;
    let out = render(&dir, rack, &input, &output);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("com.acme.x@2.0.0"), "{err}");
    assert!(!output.exists(), "no silently dry output");
}

/// SPEC-000 AC-3 (CLI side): the CLI output is bit-identical to `rack::offline::render` for the
/// same input and rack.
#[test]
fn cli_render_is_bit_identical_to_the_library_render() {
    let dir = TempDir::new("ac3");
    let (input, output) = (dir.path("pink.wav"), dir.path("out.wav"));
    run_ok(
        cli()
            .args(["gen", "pink", "--seed", "7", "--duration", "5", "-o"])
            .arg(&input),
    );
    let rack = r#"{ "slots": [
        { "module": "org.powervoice.gain@1.0.0", "bypass": false,
          "state": { "format_version": 1, "params": { "gain_db": -6.0 } } },
        { "module": "org.powervoice.gain@1.0.0", "bypass": true,
          "state": { "format_version": 1, "params": { "gain_db": 12.0 } } },
        { "module": "org.powervoice.gain@1.0.0", "bypass": false,
          "state": { "format_version": 1, "params": { "gain_db": 3.5 } } } ] }"#;
    let out = render(&dir, rack, &input, &output);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (cli_out, _) = wav::read_wav_file(&output).unwrap();
    let (x, info) = wav::read_wav_file(&input).unwrap();
    let registry = Registry::with_factories(vox_modules::builtin_factories()).unwrap();
    let lib_out = offline::render(
        &registry,
        &RackModel::from_json(rack).unwrap(),
        f64::from(info.sample_rate),
        &x,
    )
    .unwrap();
    assert_eq!(fnv1a_hash(&cli_out), fnv1a_hash(&lib_out));
}
