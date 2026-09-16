#!/usr/bin/env python3
"""T-704: writes the performance targets matrix into `docs/performance.md`.

Reads every `BENCH_RESULT` line (the T-110 convention, `vox_testkit::bench_report`) from the
measurement logs — `target/bench/raw.log` (`just bench`), `target/bench/big.log`
(`just test-big`, release, 60-min documents) and `target/bench/ui.log` (`just bench-ui`, the
headless frame-time sweep) — and rewrites the block between the `<!-- BEGIN TARGETS MATRIX -->`
and `<!-- END TARGETS MATRIX -->` markers of `docs/performance.md`: one table per PROMPT §2 /
spec performance target, each metric with its measured value, target, margin and status. The rest
of the document (method, fixes, findings) is hand-written.

Margin: `(target − value) / target` for a budget (`le`), `(value − target) / target` for a floor
(`ge`). Status: `pass`, `tight` (passes with < 30 % margin — T-704's bar for "fix it"), `FAIL`,
or `not run` (no line in any log).

Usage: `just perf-matrix` (python3 scripts/bench/matrix.py).
"""

from __future__ import annotations

import datetime
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from summary import OUT_DIR, collect_results, fmt_value  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
DOC = REPO_ROOT / "docs" / "performance.md"
BEGIN = "<!-- BEGIN TARGETS MATRIX -->"
END = "<!-- END TARGETS MATRIX -->"
LOGS = {
    "bench": OUT_DIR / "raw.log",
    "test-big": OUT_DIR / "big.log",
    "bench-ui": OUT_DIR / "ui.log",
}
TIGHT_MARGIN = 0.30

FRAME_SIZES = ("1280x720", "2126x850")
FRAME_RENDERERS = ("canvas2d", "auto")
FRAME_STATS = ("p50_ms", "p95_ms", "p99_ms", "frames_over_50ms")
IDLE_SCENES = ("empty", "document", "document_rack", "spectral")


def frame_metrics(scene: str) -> list[str]:
    return [
        f"frame_{scene}_{r}_{size}_{stat}"
        for r in FRAME_RENDERERS
        for size in FRAME_SIZES
        for stat in FRAME_STATS
    ]


# (target, how it is measured, recipe, metrics). Metric names are the `name=` of BENCH_RESULT lines.
GROUPS: list[tuple[str, str, str, list[str]]] = [
    (
        "PROMPT §2 — open a 60-min 48 kHz mono WAV in < 3 s, waveform shown (SPEC-006 AC-19: first "
        "draw ≤ 3 s after open)",
        "`powervoice-app` `perf_big`: `DocumentService::open` (the `document_open` path: probe, "
        "import into the chunk store with peak pyramids, sidecar, engine hand-over), worst of 3, "
        "headless; then the UI's first zoom-to-fit `peaks_get`. `vox-project` `big.rs`: "
        "`import_file` alone, cold (`posix_fadvise(DONTNEED)`) and warm page cache.",
        "just test-big",
        [
            "open_60min_wav_ms",
            "open_60min_to_first_overview_ms",
            "open_60min_first_overview_1280px_ms",
            "open_60min_first_overview_2126px_ms",
            "import_60min_wav_cold_cache_ms",
            "import_60min_wav_warm_cache_ms",
        ],
    ),
    (
        "PROMPT §2 — 60 fps scroll/zoom; SPEC-006 AC-18: p50 ≤ 16.7 ms, p99 ≤ 50 ms, ≤ 1 frame "
        "> 50 ms (T-704 adds p95 ≤ 16.7 ms)",
        "`scripts/bench/ui_frames.mjs`: the preview App (`?preview&scene=document&doc=60min`) in "
        "headless Chromium with uncapped rAF, 10 s zoom-then-scroll sweep through the real wheel "
        "handler after a 3 s warm-up; renderer Setting `canvas2d` (fallback) and `auto` (WebGL2 "
        "first, the default).",
        "just bench-ui",
        frame_metrics("document"),
    ),
    (
        "SPEC-007 AC-10 — split view (waveform + spectral) frame time, tiles cached: same "
        "tolerance as SPEC-006 AC-18",
        "Same sweep with `scene=spectral` (split view).",
        "just bench-ui",
        frame_metrics("spectral"),
    ),
    (
        "H-43 — idle main-thread CPU ≤ 2 % of one core in a release build (≤ 10 % debug); "
        "playback still draws at the display rate (60 fps)",
        "`scripts/bench/ui_frames.mjs` idle pass: the preview App (renderer `auto`) in "
        "vsync-paced (60 Hz) headless Chromium — not the uncapped sweep above — 10 s idle per "
        "scene after a 4 s settle; busy = CDP `TaskDuration` / wall time. `dev` is the Vite dev "
        "server (debug JS), `release` a `vite build` bundle with the bench preview "
        "(`VITE_PV_BENCH_PREVIEW=1`). Then Play for 3 s: animation frames the app ran per second.",
        "just bench-ui",
        [
            *[
                f"idle_{build}_{scene}_main_thread_pct"
                for build in ("dev", "release")
                for scene in IDLE_SCENES
            ],
            *[
                f"playback_{build}_{scene}_fps"
                for build in ("dev", "release")
                for scene in IDLE_SCENES
                if scene != "empty"
            ],
        ],
    ),
    (
        "PROMPT §2 / SPEC-003 AC-1 — playback start < 50 ms (Play → first non-silent frame written "
        "to the device buffer)",
        "`vox-engine` `playback_start`: `ManualEngine` on the `FakeBackend` clock, stream open and "
        "idle, worst of 8 Play phases, 0.1 ms resolution; empty rack and the default voice rack. "
        "H-46: the rack is pre-rolled, so the voice rack's overhead over the empty rack is "
        "asserted too (≤ 5 ms).",
        "just bench",
        [
            f"playback_start_{rack}_{b}f_written_max_ms"
            for rack in ("empty_rack", "voice_rack")
            for b in (64, 256, 1024)
        ]
        + [f"playback_start_voice_rack_{b}f_overhead_max_ms" for b in (64, 256, 1024)],
    ),
    (
        "PROMPT §2 — full rack at 48 kHz < 20 % of one core",
        "`vox-rack` `full_rack` (T-110): the typical voice rack's `process()` at each realtime "
        "block size; `vox-engine` `callback_histogram`: the whole output callback.",
        "just bench",
        [f"full_rack_process_{b}f_pct_core" for b in (64, 128, 256, 512, 1024)],
    ),
    (
        "SPEC-007 AC-18 / H-42 §8.11 — live analyzer diagnostics ≤ 2 % of one core at 60 Hz "
        "(≤ 333 µs per analyzer frame)",
        "`vox-dsp` `diagnostics`: the live voice tracker fed one 60 Hz control tick (800 samples at "
        "48 kHz) per frame, a report every 6th frame, 20 s of speech-like signal, best of 3; one "
        "Spectrum Inspector frame per FFT size (30 Hz); the long-term average job (FFT 16 384 + "
        "diagnostics) as a realtime factor.",
        "just bench",
        [
            "diagnostics_live_per_frame",
            *[f"inspector_frame_fft{n}" for n in (1024, 4096, 16384, 32768)],
            "ltas_offline_realtime_factor",
        ],
    ),
    (
        "PROMPT §2 — noise-reduction latency ≤ 50 ms",
        "`vox-modules` `spec_budgets`: the default instance's `latency_samples()` (N = 2048).",
        "just bench",
        ["noise_reduction_latency_48000hz_ms", "noise_reduction_latency_44100hz_ms"],
    ),
    (
        "SPEC-013 AC-17 — Noise Gate: 60 s pink noise, HPF on, look-ahead 5 ms ≤ 0.3 s",
        "`vox-modules` `spec_budgets`, offline 4096-frame blocks, best of 3.",
        "just bench",
        ["noise_gate_60s_hpf_lookahead5ms_render_s"],
    ),
    (
        "SPEC-014 AC-18 — Noise Reduction: 60 s ≤ 1.2 s at defaults, ≤ 1.8 s at N = 8192; "
        "60 s capture ≤ 0.3 s",
        "`vox-modules` `spec_budgets` with a captured print (the STFT path — T-110's bench "
        "measured the no-print delay line), the AC-6 signal.",
        "just bench",
        [
            "noise_reduction_60s_defaults_render_s",
            "noise_reduction_60s_n8192_render_s",
            "noise_reduction_capture_60s_s",
        ],
    ),
    (
        "SPEC-015 AC-22 — EQ response curve: 2048 points, 9 components ≤ 2 ms (median of 100)",
        "`vox-modules` `spec_budgets`.",
        "just bench",
        ["eq_response_curve_2048pts_9components_median_ms"],
    ),
    (
        "SPEC-016 AC-19 — Dynamics: 60 s all sections + RMS + look-ahead 10 ms ≤ 0.6 s; "
        "defaults ≤ 0.3 s",
        "`vox-modules` `spec_budgets`.",
        "just bench",
        [
            "dynamics_60s_all_sections_rms_lookahead10ms_render_s",
            "dynamics_60s_defaults_render_s",
        ],
    ),
    (
        "SPEC-017 AC-17 — True-Peak Limiter ≤ 1.5 % of one core (256-frame blocks)",
        "`vox-modules` `true_peak_limiter`, worst of 0/+12/+24 dB input gain.",
        "just bench",
        ["true_peak_limiter_256f_worst_pct_core"],
    ),
    (
        "SPEC-004 AC-3 — undo/redo on 20 000 pieces ≤ 50 ms",
        "`vox-project` `history_exact` (H-17), real disk.",
        "just test-big",
        ["spec004_ac3_worst_undo_redo_20000_pieces_ms"],
    ),
    (
        "SPEC-004 AC-5 / T-301 — memory budget: resident ≤ budget + 128 MiB",
        "`vox-project` `big.rs`: 60-min playback + 200 seeks at a 512 MiB budget. `perf_big`: the "
        "store's peak resident memory and the process RSS growth with a 60-min document open "
        "(default budget).",
        "just test-big",
        [
            "spec004_ac5_60min_playback_peak_resident_mib",
            "open_60min_store_peak_resident_mib",
            "open_60min_process_rss_growth_mib",
        ],
    ),
    (
        "SPEC-004 AC-11 — recovering a 60-min session with 1000 records < 5 s",
        "`vox-project` `recovery` (H-17).",
        "just test-big",
        ["spec004_ac11_recover_60min_1000_records_ms"],
    ),
    (
        "SPEC-007 AC-9 — spectrogram tiles on a 60-min document: visible ≤ 200 ms, refined "
        "≤ 2 s, warm ≤ 50 ms",
        "`vox-engine` `tests/spectro.rs` (engine side: compute + channel send), 1920 px, FFT 2048.",
        "just test-big",
        [
            f"spec007_ac9_{kind}_{view}s_view_ms"
            for view in (3600, 600, 60, 10, 1)
            for kind in ("visible", "refined", "warm")
        ],
    ),
    (
        "SPEC-008 AC-14 — edit ops on 20 000 pieces + 1000 markers: splice ≤ 5 ms p95, command "
        "(incl. journal fdatasync) ≤ 50 ms p95",
        "`vox-project` `history_exact`, 100 seeded runs per op, real disk.",
        "just test-big",
        [
            f"spec008_ac14_{op}_{kind}_p95_ms"
            for op in ("copy", "cut", "paste", "delete", "trim", "silence", "insert_silence")
            for kind in ("splice", "command")
            if not (op == "copy" and kind == "command")
        ],
    ),
    (
        "SPEC-018 AC-18 — sidecar write/read ≤ 150 ms p95",
        "`vox-project` `sidecar_perf`.",
        "just test-big",
        ["spec018_ac18_sidecar_write_p95_ms", "spec018_ac18_sidecar_read_p95_ms"],
    ),
]


def load() -> dict[str, tuple[dict[str, str], str]]:
    """metric name → (its latest BENCH_RESULT fields, the recipe whose log it came from)."""
    found: dict[str, tuple[dict[str, str], str]] = {}
    for recipe, path in LOGS.items():
        if not path.exists():
            continue
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for r in collect_results(lines):
            found[r["name"]] = (r, recipe)
    return found


def margin(value: float, target: float, op: str) -> float | None:
    if target == 0:
        return None
    return (target - value) / target if op == "le" else (value - target) / target


def row(name: str, found: dict[str, tuple[dict[str, str], str]]) -> str:
    if name not in found:
        return f"| `{name}` | — | — | — | not run |"
    r, _ = found[name]
    value = fmt_value(r["value"])
    if r["target"] == "-":
        return f"| `{name}` | {value} {r['unit']} | — | — | info |"
    v, t = float(r["value"]), float(r["target"])
    m = margin(v, t, r["op"])
    sign = "≤" if r["op"] == "le" else "≥"
    m_text = "—" if m is None else f"{m * 100:+.0f} %"
    if r["status"] == "fail":
        status = "**FAIL**"
    elif m is not None and m < TIGHT_MARGIN:
        status = "tight"
    else:
        status = "pass"
    return f"| `{name}` | {value} {r['unit']} | {sign} {fmt_value(r['target'])} | {m_text} | {status} |"


def machine() -> str:
    cpu = "unknown CPU"
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    fs = "?"
    try:
        fs = subprocess.run(
            ["stat", "-f", "-c", "%T", str(REPO_ROOT / "target")],
            capture_output=True, text=True, check=False,
        ).stdout.strip() or "?"
    except OSError:
        pass
    return f"{cpu}, {os.cpu_count()} threads, {platform.system()} {platform.release()}, `target/` on {fs}"


def render(found: dict[str, tuple[dict[str, str], str]]) -> str:
    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    out = [BEGIN, "", f"_Generated {now} by `just perf-matrix` on {machine()}. Logs:"]
    stamps = []
    for recipe, path in LOGS.items():
        if path.exists():
            t = datetime.datetime.fromtimestamp(path.stat().st_mtime, datetime.timezone.utc)
            stamps.append(f"`just {recipe}` {t.strftime('%Y-%m-%d %H:%M UTC')}")
        else:
            stamps.append(f"`just {recipe}` not run")
    out.append(", ".join(stamps) + "._")
    out.append("")
    counts = {"pass": 0, "tight": 0, "FAIL": 0, "not run": 0, "info": 0}
    for title, how, recipe, metrics in GROUPS:
        out.append(f"### {title}")
        out.append("")
        out.append(f"How: {how} Reproduce: `{recipe}`.")
        out.append("")
        out.append("| metric | measured | target | margin | status |")
        out.append("|---|---|---|---|---|")
        for name in metrics:
            line = row(name, found)
            out.append(line)
            status = line.rsplit("|", 2)[1].strip().strip("*")
            counts[status] = counts.get(status, 0) + 1
        out.append("")
    summary = (
        f"**{counts['pass']} pass, {counts['tight']} tight (< 30 % margin), {counts['FAIL']} fail, "
        f"{counts['not run']} not run.**"
    )
    out.insert(4, summary)
    out.insert(5, "")
    out.append(END)
    return "\n".join(out)


def main() -> int:
    found = load()
    block = render(found)
    text = DOC.read_text(encoding="utf-8") if DOC.exists() else f"# Performance\n\n{BEGIN}\n{END}\n"
    if BEGIN not in text or END not in text:
        print(f"{DOC} has no {BEGIN} / {END} markers", file=sys.stderr)
        return 1
    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)
    DOC.write_text(head + block + tail, encoding="utf-8")
    print(f"Wrote the targets matrix into {DOC.relative_to(REPO_ROOT)} ({len(found)} metrics in the logs).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
