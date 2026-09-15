#!/usr/bin/env python3
"""Runs the workspace's benchmarks and writes a Markdown summary (T-110 ticket item 3).

`just bench` runs this script, which runs `cargo bench --workspace`, streaming its combined
stdout/stderr live to the terminal (like a plain `cargo bench` run) while also saving it to
`target/bench/raw.log`. Every bench binary that has a number to check against a PROMPT/SPEC
target prints one machine-parseable report line via `vox_testkit::bench_report::result`:

    BENCH_RESULT crate=<crate> name=<metric> value=<f64> unit=<unit> target=<f64|-> op=<le|ge|-> status=<pass|fail|info>

This script collects those lines (wherever they appear in the combined `cargo bench` output,
divan-based or plain `main`) into `target/bench/summary.md`, grouped by crate. Metrics with no
known target (`target=-`) are `info`; the rest are `pass`/`fail` against their target. Not
committed (`target/` is gitignored).

Exits with `cargo bench`'s own exit code, so a budget assertion failing inside a bench (e.g.
`crates/modules/benches/true_peak_limiter.rs`'s SPEC-017 §4.5 check) still fails `just bench`.

T-704: the summary also merges the `BENCH_RESULT` lines of the other measurement runs, when their
logs exist: `target/bench/big.log` (`just test-big`, the 60-min release checks) and
`target/bench/ui.log` (`just bench-ui`, the headless frame-time sweep). `--no-run` skips
`cargo bench` and rebuilds the summary from the logs already on disk (`raw.log` included);
`just test-big` and `just bench-ui` call it that way. `scripts/bench/matrix.py` then turns the same
logs into the committed targets matrix, `docs/performance.md`.

Usage (same convention as `cargo bench` itself: args before `--` go to `cargo bench`, args
after it go to the bench harness/binaries — divan, or a plain `main` that ignores argv).
Defaults to `--workspace` (T-110: `just bench` is workspace-wide) unless a
`-p`/`--package`/`--exclude`/`--workspace` flag is already given:
    python3 scripts/bench/summary.py                          # cargo bench --workspace

Forwarding divan-specific flags (e.g. `--sample-count`, `--max-time`) after `--` only works when
also pinned to one bench binary with `--bench <name>` — `cargo bench` also runs each crate's own
lib/integration tests as an (unrelated) "bench" target, whose default libtest harness doesn't
understand divan's flags and errors on them:
    python3 scripts/bench/summary.py -p vox-dsp --bench loudness -- --max-time 0.2
    python3 scripts/bench/summary.py --no-run                 # re-summarize the existing logs
"""

from __future__ import annotations

import datetime
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
OUT_DIR = REPO_ROOT / "target" / "bench"
RAW_LOG = OUT_DIR / "raw.log"
SUMMARY = OUT_DIR / "summary.md"
# T-704: other runs' logs merged into the summary (and into docs/performance.md by matrix.py).
EXTRA_LOGS = (OUT_DIR / "big.log", OUT_DIR / "ui.log")

RESULT_LINE = re.compile(r"^BENCH_RESULT (.+)$")
REQUIRED_FIELDS = {"crate", "name", "value", "unit", "target", "op", "status"}


def parse_result_line(rest: str) -> dict[str, str] | None:
    """Parses the `key=value` tokens after the `BENCH_RESULT ` prefix; `None` if malformed."""
    fields: dict[str, str] = {}
    for tok in rest.split():
        if "=" not in tok:
            return None
        key, value = tok.split("=", 1)
        fields[key] = value
    return fields if REQUIRED_FIELDS <= fields.keys() else None


SCOPE_FLAGS = ("-p", "--package", "--workspace", "--exclude")


def run_cargo_bench(cargo_args: list[str], harness_args: list[str]) -> tuple[int, list[str]]:
    """Runs `cargo bench` (workspace-wide unless `cargo_args` already scopes it), tee'd to
    stdout and `RAW_LOG`; returns (exit code, lines)."""
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    scoped = any(a in SCOPE_FLAGS or a.startswith("-p=") or a.startswith("--package=") for a in cargo_args)
    cmd = ["cargo", "bench", *([] if scoped else ["--workspace"]), *cargo_args]
    if harness_args:
        cmd += ["--", *harness_args]
    proc = subprocess.Popen(
        cmd,
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    assert proc.stdout is not None
    lines: list[str] = []
    with open(RAW_LOG, "w", encoding="utf-8") as raw:
        for line in proc.stdout:
            sys.stdout.write(line)
            raw.write(line)
            lines.append(line.rstrip("\n"))
    return proc.wait(), lines


def collect_results(lines: list[str]) -> list[dict[str, str]]:
    results = []
    for line in lines:
        m = RESULT_LINE.match(line)
        if not m:
            continue
        fields = parse_result_line(m.group(1))
        if fields is not None:
            results.append(fields)
    return results


def fmt_value(raw: str) -> str:
    try:
        return f"{float(raw):.4g}"
    except ValueError:
        return raw


def log_sources() -> list[Path]:
    """The logs the summary reads: `raw.log` (`cargo bench`) plus whichever extra logs exist."""
    return [p for p in (RAW_LOG, *EXTRA_LOGS) if p.exists()]


def read_log_lines() -> list[str]:
    lines: list[str] = []
    for path in log_sources():
        lines.extend(path.read_text(encoding="utf-8", errors="replace").splitlines())
    return lines


def fmt_mtime(path: Path) -> str:
    stamp = datetime.datetime.fromtimestamp(path.stat().st_mtime, datetime.timezone.utc)
    return stamp.strftime("%Y-%m-%d %H:%M UTC")


def render_markdown(results: list[dict[str, str]], exit_code: int) -> str:
    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    out: list[str] = []
    out.append("# PowerVoice bench summary\n\n")
    out.append(
        f"Generated {now} by `scripts/bench/summary.py` (`just bench`). Full `cargo bench` "
        "console output (divan's own statistical tables included): `target/bench/raw.log` "
        "(not committed). This is T-704's performance baseline (T-110).\n\n"
    )
    sources = log_sources()
    if sources:
        out.append("Sources: " + ", ".join(
            f"`{p.relative_to(REPO_ROOT)}` ({fmt_mtime(p)})" for p in sources) + ".\n\n")
    if exit_code != 0:
        out.append(f"**`cargo bench` exited with status {exit_code}.**\n\n")
    if not results:
        out.append("No `BENCH_RESULT` lines were captured — see `target/bench/raw.log`.\n")
        return "".join(out)

    by_crate: dict[str, list[dict[str, str]]] = {}
    for r in results:
        by_crate.setdefault(r["crate"], []).append(r)

    passed = [r for r in results if r["status"] == "pass"]
    failed = [r for r in results if r["status"] == "fail"]
    info = [r for r in results if r["status"] == "info"]
    out.append(
        f"{len(results)} metrics reported: **{len(passed)} pass**, **{len(failed)} fail**, "
        f"{len(info)} informational (no PROMPT/SPEC target).\n\n"
    )

    if failed:
        out.append("## Failed targets\n\n")
        out.append("| crate | metric | value | unit | target |\n")
        out.append("|---|---|---|---|---|\n")
        for r in sorted(failed, key=lambda r: (r["crate"], r["name"])):
            out.append(
                f"| {r['crate']} | {r['name']} | {fmt_value(r['value'])} | {r['unit']} | "
                f"{r['op']} {fmt_value(r['target'])} |\n"
            )
        out.append("\n")

    for crate in sorted(by_crate):
        out.append(f"## {crate}\n\n")
        out.append("| metric | value | unit | target | status |\n")
        out.append("|---|---|---|---|---|\n")
        for r in sorted(by_crate[crate], key=lambda r: r["name"]):
            target = "-" if r["target"] == "-" else f"{r['op']} {fmt_value(r['target'])}"
            out.append(
                f"| {r['name']} | {fmt_value(r['value'])} | {r['unit']} | {target} | "
                f"{r['status']} |\n"
            )
        out.append("\n")
    return "".join(out)


def main() -> int:
    argv = sys.argv[1:]
    no_run = "--no-run" in argv
    argv = [a for a in argv if a != "--no-run"]
    if "--" in argv:
        idx = argv.index("--")
        cargo_args, harness_args = argv[:idx], argv[idx + 1 :]
    else:
        cargo_args, harness_args = argv, []
    exit_code = 0
    if not no_run:
        exit_code, _ = run_cargo_bench(cargo_args, harness_args)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    results = collect_results(read_log_lines())
    SUMMARY.write_text(render_markdown(results, exit_code), encoding="utf-8")
    print(f"\nWrote {SUMMARY.relative_to(REPO_ROOT)} ({len(results)} BENCH_RESULT lines).")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
