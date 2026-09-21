#!/usr/bin/env python3
"""H-98: a repro harness for bugs that only show up in the *real compiled app* — a frozen
WebView, an unresponsive Cancel/close, a process that ignores SIGTERM — the class H-96 could not
chase because there was no way to drive the real binary and read its CPU/signal behaviour outside
a component test.

This is deliberately small: build the real app, launch it, find its window, sample per-thread CPU
and signal state from `/proc` while you drive it (manually, or via `wtype`/`hyprctl` on a Hyprland/
wlroots session), and check whether it actually dies on SIGTERM. It does not try to synthesize
clicks — see "Driving it" in `docs/contributing.md` for that, and its limits.

Usage:
    python3 scripts/repro/gui_harness.py build [--release]
    python3 scripts/repro/gui_harness.py launch [--release] [--pidfile PATH]
    python3 scripts/repro/gui_harness.py watch PID [--seconds N] [--interval S] [--out PATH]
    python3 scripts/repro/gui_harness.py screenshot PID [--out PATH] [--timeout S]
    python3 scripts/repro/gui_harness.py shutdown-test PID [--timeout S]

Only ever acts on a PID it (or you) just launched — never guess at, or touch, someone else's
running instance.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
BINARY_DEBUG = REPO_ROOT / "target" / "debug" / "powervoice-app"
BINARY_RELEASE = REPO_ROOT / "target" / "release" / "powervoice-app"


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"+ {' '.join(cmd)}", file=sys.stderr)
    # H-101: `cwd` used to be hard-coded here *and* passed by callers, which raised
    # "got multiple values for keyword argument 'cwd'" and broke `build` outright.
    kwargs.setdefault("cwd", REPO_ROOT)
    return subprocess.run(cmd, **kwargs)


def cmd_build(args: argparse.Namespace) -> int:
    """`npm run build` (ui/dist) + `cargo build [-p powervoice-app]` — the compiled binary then
    loads the production frontend bundle directly (no `devUrl`, no mocked IPC), which is the
    "real app" this harness is for. Debug build (~2 min) is enough; pass --release to match what
    ships."""
    # H-101: a plain `cargo build -p powervoice-app` still embeds `devUrl`, so the launched
    # window shows "Could not connect to localhost". Only the Tauri CLI bakes in `frontendDist`.
    r = run(["npm", "run", "build"], cwd=REPO_ROOT / "ui")
    if r.returncode != 0:
        return r.returncode
    cargo_cmd = ["cargo", "build", "-p", "powervoice-app"]
    if args.release:
        cargo_cmd.append("--release")
    return run(cargo_cmd).returncode


def cmd_launch(args: argparse.Namespace) -> int:
    """Launches the compiled binary detached from this shell (`setsid`, so it survives the
    harness process exiting) and resolves its *real* PID — `$!` after `setsid ... &` is the
    `setsid` wrapper's PID, not the app's, whenever `setsid` has to fork (it does whenever the
    calling shell is already a process group leader, which a backgrounded job usually is)."""
    binary = BINARY_RELEASE if args.release else BINARY_DEBUG
    if not binary.exists():
        print(f"error: {binary} does not exist — run `build` first", file=sys.stderr)
        return 1
    log_path = Path(args.log) if args.log else REPO_ROOT / "target" / "gui-harness.log"
    log = open(log_path, "wb")
    subprocess.Popen(
        ["setsid", str(binary)],
        cwd=REPO_ROOT,
        stdout=log,
        stderr=log,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    # The Popen PID is `setsid`'s PID even with start_new_session=True; resolve the real binary's
    # PID by name once it has had a moment to exec.
    pid = None
    for _ in range(50):
        time.sleep(0.1)
        found = subprocess.run(
            ["pgrep", "-f", str(binary)], capture_output=True, text=True
        ).stdout.split()
        if found:
            pid = int(found[0])
            break
    if pid is None:
        print("error: launched process never appeared under its own PID", file=sys.stderr)
        return 1
    print(pid)
    if args.pidfile:
        Path(args.pidfile).write_text(str(pid))
    return 0


def hyprctl_clients() -> list[dict]:
    r = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True)
    if r.returncode != 0:
        return []
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return []


def window_for_pid(pid: int) -> dict | None:
    for client in hyprctl_clients():
        if client.get("pid") == pid:
            return client
    return None


def thread_snapshot(pid: int) -> list[dict]:
    """One sample of every thread's state, %CPU (since-start average — good enough to spot a
    thread pegged at ~100% across consecutive samples) and syscall wait channel, straight from
    `/proc` so it works with or without `ps -T` (which can silently show only the main thread in
    some containerised/sandboxed environments — seen during H-98)."""
    task_dir = Path(f"/proc/{pid}/task")
    if not task_dir.is_dir():
        return []
    threads = []
    for tid_dir in sorted(task_dir.iterdir()):
        tid = tid_dir.name
        try:
            stat = (tid_dir / "stat").read_text()
            status = (tid_dir / "status").read_text()
        except (FileNotFoundError, ProcessLookupError):
            continue
        # comm is the 2nd field but may contain spaces/parens; state is right after it.
        after_comm = stat[stat.rfind(")") + 2 :]
        state = after_comm.split()[0]
        comm = stat[stat.find("(") + 1 : stat.rfind(")")]
        wchan = ""
        wchan_path = tid_dir / "wchan"
        if wchan_path.exists():
            wchan = wchan_path.read_text()
        threads.append({"tid": tid, "comm": comm, "state": state, "wchan": wchan})
        del status
    return threads


def signal_masks(pid: int) -> dict:
    """`SigBlk`/`SigIgn`/`SigCgt` from `/proc/<pid>/status` (main thread) — decode with
    `int(mask, 16) & (1 << (signum - 1))`. If SIGTERM (15) ever shows up in `SigBlk` this is a
    real answer to "why does it ignore SIGTERM", not a guess."""
    text = Path(f"/proc/{pid}/status").read_text()
    out = {}
    for line in text.splitlines():
        for key in ("SigBlk", "SigIgn", "SigCgt", "Threads", "State"):
            if line.startswith(key + ":"):
                out[key] = line.split(":", 1)[1].strip()
    return out


def cmd_watch(args: argparse.Namespace) -> int:
    """Samples thread state + signal masks every `--interval` seconds for `--seconds` total,
    printing one JSON line per sample (and appending to `--out` if given). Run this *while* you
    (or a `wtype`/`hyprctl`-driven script) interact with the app — the freeze this ticket chases
    should show up as threads stuck in `R` state across many consecutive samples with no wall-time
    progress, or in `D` (uninterruptible sleep, which even `SIGKILL` can't interrupt)."""
    out_fh = open(args.out, "a") if args.out else None
    end = time.monotonic() + args.seconds
    while time.monotonic() < end:
        if not Path(f"/proc/{args.pid}").exists():
            print(f"pid {args.pid} is gone", file=sys.stderr)
            break
        sample = {
            "t": round(time.monotonic(), 3),
            "signals": signal_masks(args.pid),
            "threads": thread_snapshot(args.pid),
        }
        line = json.dumps(sample)
        print(line)
        if out_fh:
            out_fh.write(line + "\n")
            out_fh.flush()
        time.sleep(args.interval)
    if out_fh:
        out_fh.close()
    return 0


def cmd_screenshot(args: argparse.Namespace) -> int:
    """`grim -g "<geometry>" out.png` using the window's geometry from `hyprctl clients -j`.
    H-90's lesson stands: a window existing is not evidence it rendered anything — always look at
    the pixels. New H-98 lesson: `grim` can hang with **no** stale process to blame (seen against
    a plain full-screen capture with no app involved) — always wrap it in `timeout`, and still
    check `pgrep -x grim` for a stale one afterwards; if this keeps happening treat "no screenshot"
    as an environment limitation and fall back to the `/proc` evidence from `watch`, not as proof
    of anything about the app."""
    win = window_for_pid(args.pid)
    if win is None:
        print(f"error: no hyprctl window for pid {args.pid}", file=sys.stderr)
        return 1
    (x, y), (w, h) = win["at"], win["size"]
    geometry = f"{x},{y} {w}x{h}"
    out = args.out or f"/tmp/gui-harness-{args.pid}.png"
    r = subprocess.run(["timeout", str(args.timeout), "grim", "-g", geometry, out])
    if r.returncode == 124:
        print(
            "grim timed out — check `pgrep -x grim` for a stale process and kill only that one; "
            "otherwise this environment's screencopy isn't available right now",
            file=sys.stderr,
        )
        return 124
    print(out)
    return r.returncode


def cmd_shutdown_test(args: argparse.Namespace) -> int:
    """The actual repro signal: SIGTERM, then poll for exit, then (only if it truly never exits)
    SIGKILL the same PID we launched — never anything else. Reports elapsed time either way."""
    import os
    import signal

    if not Path(f"/proc/{args.pid}").exists():
        print(f"error: pid {args.pid} is not running", file=sys.stderr)
        return 1
    start = time.monotonic()
    os.kill(args.pid, signal.SIGTERM)
    while time.monotonic() - start < args.timeout:
        if not Path(f"/proc/{args.pid}").exists():
            elapsed = time.monotonic() - start
            print(f"exited {elapsed:.2f}s after SIGTERM")
            return 0
        time.sleep(0.1)
    print(
        f"still alive {args.timeout}s after SIGTERM (ignored it) — sending SIGKILL to pid "
        f"{args.pid} only",
        file=sys.stderr,
    )
    os.kill(args.pid, signal.SIGKILL)
    return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p_build = sub.add_parser("build", help="build ui/dist + the powervoice-app binary")
    p_build.add_argument("--release", action="store_true")
    p_build.set_defaults(func=cmd_build)

    p_launch = sub.add_parser("launch", help="launch the compiled app, print its real PID")
    p_launch.add_argument("--release", action="store_true")
    p_launch.add_argument("--pidfile")
    p_launch.add_argument("--log")
    p_launch.set_defaults(func=cmd_launch)

    p_watch = sub.add_parser("watch", help="sample thread/signal state while you drive the app")
    p_watch.add_argument("pid", type=int)
    p_watch.add_argument("--seconds", type=float, default=30)
    p_watch.add_argument("--interval", type=float, default=1.0)
    p_watch.add_argument("--out")
    p_watch.set_defaults(func=cmd_watch)

    p_shot = sub.add_parser("screenshot", help="grim-capture the app's window")
    p_shot.add_argument("pid", type=int)
    p_shot.add_argument("--out")
    p_shot.add_argument("--timeout", type=float, default=10)
    p_shot.set_defaults(func=cmd_screenshot)

    p_shutdown = sub.add_parser("shutdown-test", help="SIGTERM, then SIGKILL only if needed")
    p_shutdown.add_argument("pid", type=int)
    p_shutdown.add_argument("--timeout", type=float, default=10)
    p_shutdown.set_defaults(func=cmd_shutdown_test)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
