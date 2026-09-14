#!/usr/bin/env python3
"""Validates a `.desktop` file (T-705 packaging test).

Uses `desktop-file-validate` (freedesktop.org's own validator) when it's on PATH; otherwise falls
back to a small format check covering the same things the ticket cares about (present, parseable,
the keys PowerVoice's entry needs). No new dependency either way.

Usage:
    python3 scripts/packaging/check_desktop_entry.py [path-to-file.desktop ...]

With no arguments, validates every `*.desktop` file under `target/*/bundle/` (i.e. whatever
`just build` most recently produced) plus `src-tauri/PowerVoice.desktop` in the AppImage's
freshly-unpacked AppDir. Exits non-zero, with the offending file(s), on any failure.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
REQUIRED_KEYS = ("Type", "Name", "Exec")


def find_desktop_files() -> list[Path]:
    target = REPO_ROOT / "target"
    return sorted(target.glob("*/bundle/**/*.desktop"))


def fallback_check(path: Path) -> list[str]:
    """A minimal format check, used only when `desktop-file-validate` isn't installed."""
    errors = []
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return [f"{path}: not valid UTF-8"]

    lines = text.splitlines()
    if not lines or lines[0].strip() != "[Desktop Entry]":
        errors.append(f"{path}: first line must be '[Desktop Entry]'")

    keys_seen: dict[str, int] = {}
    for line in lines[1:]:
        if not line.strip() or line.startswith("#") or line.startswith("["):
            continue
        if "=" not in line:
            errors.append(f"{path}: malformed line (no '='): {line!r}")
            continue
        key = line.split("=", 1)[0]
        keys_seen[key] = keys_seen.get(key, 0) + 1

    for key, count in keys_seen.items():
        if count > 1:
            errors.append(f"{path}: duplicate key {key!r}")

    for required in REQUIRED_KEYS:
        if required not in keys_seen:
            errors.append(f"{path}: missing required key {required!r}")

    if keys_seen.get("Type") is not None and "Type=Application" not in text:
        errors.append(f"{path}: Type must be 'Application'")

    return errors


def validate(path: Path) -> list[str]:
    validator = shutil.which("desktop-file-validate")
    if validator:
        proc = subprocess.run([validator, str(path)], capture_output=True, text=True)
        if proc.returncode != 0:
            return [f"{path}: {proc.stdout.strip() or proc.stderr.strip()}"]
        return []
    return fallback_check(path)


def main() -> int:
    paths = [Path(p) for p in sys.argv[1:]] or find_desktop_files()
    if not paths:
        print("no .desktop files found under target/*/bundle/ — run `just build` first.")
        return 1

    all_errors: list[str] = []
    for path in paths:
        errors = validate(path)
        all_errors.extend(errors)
        status = "OK" if not errors else "FAIL"
        print(f"[{status}] {path}")

    if all_errors:
        print("\n".join(all_errors), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
