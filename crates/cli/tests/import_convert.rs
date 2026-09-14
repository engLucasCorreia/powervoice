//! T-202: `powervoice-cli convert` reading non-WAV formats and `--downmix`, plus
//! `powervoice-cli markers` (SPEC-005 §4.11).
#![allow(clippy::float_cmp)] // deliberate bit-exact channel-copy check

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_powervoice-cli"))
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pv-t202-cli-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn convert_reads_a_flac_input() {
    if !have("flac") {
        eprintln!("skipping: flac CLI not installed");
        return;
    }
    let dir = TempDir::new("flac-in");
    let wav = dir.path("in.wav");
    run_ok(
        cli()
            .args(["gen", "sine", "--freq", "440", "--level", "-6"])
            .args(["--duration", "0.2", "--rate", "48000", "--bits", "24", "-o"])
            .arg(&wav),
    );
    let flac = dir.path("in.flac");
    run_ok(
        Command::new("flac")
            .args(["-f", "--totally-silent", "-o"])
            .arg(&flac)
            .arg(&wav),
    );

    let out_wav = dir.path("out.wav");
    run_ok(
        cli()
            .arg("convert")
            .arg(&flac)
            .arg(&out_wav)
            .args(["--bits", "24"]),
    );
    let analyze = run_ok(cli().arg("analyze").arg(&out_wav).args(["--json"]));
    let text = String::from_utf8_lossy(&analyze.stdout);
    assert!(text.contains("\"sample_rate\":48000") || text.contains("48000"));
}

#[test]
fn convert_downmix_channel_pick_matches_the_chosen_channel() {
    let dir = TempDir::new("downmix");
    // Build a stereo WAV by hand: `gen` only emits mono, so interleave two mono renders.
    let left = dir.path("left.wav");
    let right = dir.path("right.wav");
    run_ok(
        cli()
            .args(["gen", "sine", "--freq", "300", "--level", "-10"])
            .args([
                "--duration",
                "0.05",
                "--rate",
                "48000",
                "--bits",
                "32f",
                "-o",
            ])
            .arg(&left),
    );
    run_ok(
        cli()
            .args(["gen", "white", "--seed", "1", "--level", "-20"])
            .args([
                "--duration",
                "0.05",
                "--rate",
                "48000",
                "--bits",
                "32f",
                "-o",
            ])
            .arg(&right),
    );
    let stereo = dir.path("stereo.wav");
    interleave_wav(&left, &right, &stereo);

    let picked = dir.path("picked.wav");
    run_ok(cli().arg("convert").arg(&stereo).arg(&picked).args([
        "--downmix",
        "ch:1",
        "--bits",
        "32f",
    ]));

    let (right_samples, _) = vox_testkit::wav::read_wav_file(&right).unwrap();
    let (picked_samples, _) = vox_testkit::wav::read_wav_file(&picked).unwrap();
    assert_eq!(picked_samples.len(), right_samples.len());
    for (a, b) in right_samples.iter().zip(picked_samples.iter()) {
        assert_eq!(a, b, "ch:1 must copy the right channel verbatim");
    }
}

/// Interleaves two mono WAV files into one stereo WAV (test helper only).
fn interleave_wav(left: &Path, right: &Path, out: &Path) {
    let (l, info) = vox_testkit::wav::read_wav_file(left).unwrap();
    let (r, _) = vox_testkit::wav::read_wav_file(right).unwrap();
    assert_eq!(l.len(), r.len());
    let mut stereo = Vec::with_capacity(l.len() * 2);
    for i in 0..l.len() {
        stereo.push(l[i]);
        stereo.push(r[i]);
    }
    vox_testkit::wav::write_wav_file(
        out,
        &stereo,
        2,
        info.sample_rate,
        vox_testkit::wav::BitDepth::Float32,
    )
    .unwrap();
}

#[test]
fn markers_lists_cue_points_and_regions() {
    let dir = TempDir::new("markers");
    let wav = dir.path("in.wav");
    run_ok(
        cli()
            .args(["gen", "sine", "--freq", "440", "--level", "-6"])
            .args(["--duration", "0.1", "--rate", "48000", "--bits", "16", "-o"])
            .arg(&wav),
    );
    let (samples, info) = vox_testkit::wav::read_wav_file(&wav).unwrap();
    let markers = vec![
        vox_io::WavMarker {
            pos_samples: 100,
            len_samples: 0,
            name: "Intro".into(),
        },
        vox_io::WavMarker {
            pos_samples: 200,
            len_samples: 50,
            name: "Region".into(),
        },
    ];
    let with_markers = dir.path("with_markers.wav");
    vox_io::write_wav_with_markers(
        &with_markers,
        info.sample_rate,
        vox_io::BitDepth::Int16,
        &samples,
        &markers,
    )
    .unwrap();

    let out = run_ok(cli().arg("markers").arg(&with_markers));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Intro"));
    assert!(text.contains("Region"));

    let json_out = run_ok(cli().arg("markers").arg(&with_markers).arg("--json"));
    let json_text = String::from_utf8_lossy(&json_out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 2);
    assert_eq!(parsed[0]["pos_samples"], 100);
    assert_eq!(parsed[1]["len_samples"], 50);
}

#[test]
fn markers_of_a_plain_wav_reports_none() {
    let dir = TempDir::new("no-markers");
    let wav = dir.path("plain.wav");
    run_ok(
        cli()
            .args(["gen", "silence"])
            .args([
                "--duration",
                "0.05",
                "--rate",
                "48000",
                "--bits",
                "16",
                "-o",
            ])
            .arg(&wav),
    );
    let out = run_ok(cli().arg("markers").arg(&wav));
    assert!(String::from_utf8_lossy(&out.stdout).contains("no markers"));
}
