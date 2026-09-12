//! T-006 acceptance: `powervoice-cli gen sine --freq 1000 --level -20 -o - |
//! powervoice-cli analyze -` works.

use std::io::Write;
use std::process::{Command, Stdio};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_powervoice-cli"))
}

#[test]
fn gen_sine_stdout_pipes_into_analyze_stdin() {
    let gen_output = cli()
        .args([
            "gen",
            "sine",
            "--freq",
            "1000",
            "--level",
            "-20",
            "--duration",
            "20",
            "-o",
            "-",
        ])
        .output()
        .expect("run gen");
    assert!(
        gen_output.status.success(),
        "gen failed: {}",
        String::from_utf8_lossy(&gen_output.stderr)
    );
    assert!(!gen_output.stdout.is_empty(), "gen produced no bytes");

    let mut analyze = cli()
        .args(["analyze", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn analyze");
    analyze
        .stdin
        .take()
        .expect("analyze stdin")
        .write_all(&gen_output.stdout)
        .expect("write WAV bytes to analyze stdin");
    let analyze_output = analyze.wait_with_output().expect("run analyze");
    assert!(
        analyze_output.status.success(),
        "analyze failed: {}",
        String::from_utf8_lossy(&analyze_output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&analyze_output.stdout).expect("analyze --json output");
    let peak = report["peak_dbfs"].as_f64().unwrap();
    let rms = report["rms_db"].as_f64().unwrap();
    let integrated = report["integrated_lufs"].as_f64().unwrap();

    assert!((peak - (-20.0)).abs() <= 0.01, "peak {peak}");
    assert!((rms - (-23.01)).abs() <= 0.01, "rms {rms}");
    assert!(
        (integrated - (-23.0)).abs() <= 0.1,
        "integrated {integrated}"
    );
}
