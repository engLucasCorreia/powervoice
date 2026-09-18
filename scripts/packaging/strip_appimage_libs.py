#!/usr/bin/env python3
"""Removes denylisted bundled libraries from a built AppImage and repacks it (H-89 fix).

Tauri's AppImage bundling shells out to `linuxdeploy`, which copies every `NEEDED` shared library
it finds by walking the dependency graph of `powervoice-app` — including ones that must never be
vendored (see `appimage_denylist.py`, e.g. `libpipewire-0.3.so.0`: bundled without its `spa-0.2`
plugin directory, which lives at a different absolute, distro-specific path than the one it was
compiled to look up, so it retries in a tight loop instead of finding its plugins). linuxdeploy has
no config knob for this (only a CLI `--exclude-library` flag Tauri's bundler doesn't expose), so
this script is a post-bundle step: extract the AppImage `tauri build` already produced, delete the
denylisted files, repack.

Run by `scripts/packaging/tauri_build.sh` right after it runs `tauri build` — the same script
`just build` and every OS job of `.github/workflows/release.yml` call (see that script's header
comment), so this is one source of truth for local and CI builds alike. `check_bundle.py`'s
`check_denylisted_libs` is the guard that makes the fix permanent: it fails the build if a
denylisted library ever ships despite this step (e.g. this script silently not running).

Usage:
    python3 scripts/packaging/strip_appimage_libs.py [path-to-appimage ...]

With no arguments, strips every `*.AppImage` under `target/*/bundle/appimage/` (whatever the most
recent `tauri build` produced) — a no-op, printing a message, if none exist (e.g. the
Windows/macOS release jobs, which never build an AppImage in the first place).

Repacking reuses the `linuxdeploy-plugin-appimage` tool Tauri's own bundler already downloaded to
`~/.cache/tauri/` in order to build the AppImage the first time (docs/building.md) — no new
dependency, no extra network access beyond what `tauri build` already needed for the exact same
tool. `APPIMAGE_EXTRACT_AND_RUN=1` is set for that subprocess so it works without FUSE (a
sandboxed CI runner has none — see docs/building.md item 3), matching `check_bundle.py`'s own
`--appimage-extract` (also FUSE-free) for reading the built AppImage back.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from appimage_denylist import DENYLISTED_LIBS  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]


class StripError(Exception):
    """Raised for anything that stops us producing a fixed-up AppImage (as opposed to simply
    finding nothing to strip, which is success)."""


def find_appimages(target_dir: Path) -> list[Path]:
    return sorted(target_dir.glob("*/bundle/appimage/*.AppImage"))


def find_repack_tool() -> Path:
    """Locates the AppImage-packaging tool `tauri build` itself already downloaded to build the
    AppImage in the first place — see the module docstring for why we reuse it rather than adding
    a new dependency."""
    cache_dir = Path(os.environ.get("TAURI_CACHE_DIR", str(Path.home() / ".cache" / "tauri")))
    candidate = cache_dir / "linuxdeploy-plugin-appimage.AppImage"
    if candidate.is_file():
        return candidate
    raise StripError(
        f"can't find {candidate} to repack the AppImage with. It's downloaded by `tauri build` "
        "the first time it builds an AppImage bundle, so run `scripts/packaging/tauri_build.sh "
        "--bundles appimage` (or `just build`) first, then re-run this script."
    )


def remove_denylisted(appdir: Path) -> list[str]:
    """Deletes every denylisted library under `appdir` (any depth — not just `usr/lib`, matching
    `check_bundle.py`'s own guard). Returns the removed paths, relative to `appdir`, for logging."""
    removed = []
    for pattern in DENYLISTED_LIBS:
        for match in sorted(appdir.rglob(pattern)):
            if match.is_file() or match.is_symlink():
                removed.append(str(match.relative_to(appdir)))
                match.unlink()
    return removed


def extract_appimage(appimage_path: Path, dest_parent: Path) -> Path:
    resolved = appimage_path.resolve()
    resolved.chmod(resolved.stat().st_mode | 0o111)
    proc = subprocess.run(
        [str(resolved), "--appimage-extract"],
        cwd=dest_parent,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise StripError(f"{appimage_path}: --appimage-extract failed: {proc.stderr}")
    appdir = dest_parent / "squashfs-root"
    if not appdir.is_dir():
        raise StripError(f"{appimage_path}: --appimage-extract produced no squashfs-root")
    return appdir


def repack(appdir: Path, tool: Path, dest: Path) -> None:
    """Packages `appdir` back into an AppImage at `dest`, overwriting whatever `tauri build` had
    put there."""
    with tempfile.TemporaryDirectory() as work_dir:
        env = dict(os.environ)
        env["APPIMAGE_EXTRACT_AND_RUN"] = "1"
        proc = subprocess.run(
            [str(tool), f"--appdir={appdir}"],
            cwd=work_dir,
            env=env,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise StripError(
                f"repacking {dest} failed (exit {proc.returncode}):\n{proc.stdout}\n{proc.stderr}"
            )
        produced = sorted(Path(work_dir).glob("*.AppImage"))
        if not produced:
            raise StripError(
                f"repacking {dest} produced no .AppImage; tool output:\n{proc.stdout}\n{proc.stderr}"
            )
        # The tool names its output from the AppDir's desktop entry (e.g. PowerVoice-x86_64.AppImage)
        # — move it over the original path so every other consumer (check_bundle.py, the release
        # workflow's glob, the filename the owner downloads) sees the same name it always has.
        shutil.move(str(produced[0]), dest)
        dest.chmod(dest.stat().st_mode | 0o111)


def strip_one(path: Path) -> bool:
    """Returns True if anything was removed (and the file was repacked)."""
    with tempfile.TemporaryDirectory() as work_dir:
        appdir = extract_appimage(path, Path(work_dir))
        removed = remove_denylisted(appdir)
        if not removed:
            print(f"[clean] {path}: no denylisted libraries bundled")
            return False
        for name in removed:
            print(f"[strip] {path}: removing {name}")
        tool = find_repack_tool()
        repack(appdir, tool, path.resolve())
        print(f"[strip] {path}: repacked")
        return True


def main() -> int:
    args = sys.argv[1:]
    if any(a in ("-h", "--help") for a in args):
        print(f"usage: {Path(sys.argv[0]).name} [APPIMAGE ...]")
        print("Strips denylisted libraries (see appimage_denylist.py) from each AppImage and")
        print("repacks it. With no arguments, acts on target/*/bundle/appimage/*.AppImage.")
        return 0
    missing = [a for a in args if not Path(a).exists()]
    if missing:
        print(f"error: no such file: {', '.join(missing)}", file=sys.stderr)
        return 1
    paths = [Path(p) for p in args] or find_appimages(REPO_ROOT / "target")
    if not paths:
        print("no AppImage found under target/*/bundle/appimage/ — nothing to strip.")
        return 0

    for path in paths:
        try:
            strip_one(path)
        except StripError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
