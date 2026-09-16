#!/usr/bin/env python3
"""Fails if a built Windows bundle (MSI or NSIS `.exe`) lacks the plugin sandbox (H-61).

Sibling to `check_bundle.py` rather than a branch inside it: opening an `.msi` needs `msiexec`
(Windows-only) and a Tauri NSIS installer is most reliably unpacked with the 7-Zip CLI that GitHub's
`windows-latest` runner image ships preinstalled — neither tool exists on the Linux/macOS hosts
`check_bundle.py` also has to run on, so keeping this Windows-only logic in its own file means
`check_bundle.py` stays free of `sys.platform` branches.

The app finds the sandbox beside its own executable
(`vox_plugin_host::SandboxOptions::beside_current_exe`), so every installer this project ships must
put `powervoice-sandbox.exe` in the same directory as `powervoice-app.exe`. Unlike the `.deb`/
AppImage/`.app` layouts (a fixed `usr/bin` or `Contents/MacOS`), neither WiX (MSI) nor NSIS is
guaranteed here to use a specific install-directory name, so this walks the whole extracted tree
looking for any directory that holds both names, rather than assuming one fixed path.

Usage:
    python3 scripts/packaging/check_bundle_windows.py [path-to-bundle ...]

With no arguments, checks every `*.msi` under `target/*/bundle/msi/` and every `*.exe` under
`target/*/bundle/nsis/` (i.e. whatever `just build`/`scripts/packaging/tauri_build.sh` most
recently produced on a Windows host). Exits non-zero, with the offending file(s), on any failure.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Reuse the app/sandbox binary base names and the BundleError convention from check_bundle.py
# (same directory, so this import works whether run as `python3 scripts/packaging/
# check_bundle_windows.py` or via the test file, exactly like test_check_bundle.py does).
import check_bundle

REPO_ROOT = Path(__file__).resolve().parents[2]
APP_BINARY = f"{check_bundle.APP_BINARY}.exe"
SANDBOX_BINARY = f"{check_bundle.SANDBOX_BINARY}.exe"
BundleError = check_bundle.BundleError


# --- the testable core: given an already-extracted tree, is the sandbox beside the app? ----------


def find_all(root: Path, name: str) -> list[Path]:
    """Every file named `name` anywhere under `root` (case-insensitive, like Windows filenames)."""
    return sorted(p for p in root.rglob("*") if p.is_file() and p.name.lower() == name.lower())


def check_windows_tree(root: Path) -> list[str]:
    """Checks that every copy of the app binary has the sandbox binary right beside it.

    Doesn't assume a fixed install-directory layout (WiX's administrative-install tree and an
    unpacked NSIS payload don't share one) — it just requires that wherever `powervoice-app.exe`
    landed, `powervoice-sandbox.exe` landed in the same directory.
    """
    app_paths = find_all(root, APP_BINARY)
    if not app_paths:
        return [f"no {APP_BINARY} found anywhere under {root}"]
    errors = []
    for app_path in app_paths:
        sandbox_path = app_path.parent / SANDBOX_BINARY
        if not sandbox_path.is_file():
            errors.append(f"{app_path} has no {SANDBOX_BINARY} beside it in {app_path.parent}")
    return errors


# --- extraction: real .msi / NSIS .exe installers -> a tree check_windows_tree() can look at ------


def extract_msi(msi_path: Path, dest: Path) -> None:
    """Unpacks an MSI via an administrative install (`msiexec /a ... /qn`) — Windows-only."""
    if shutil.which("msiexec") is None:
        raise BundleError(f"{msi_path}: needs Windows' 'msiexec' to unpack, and it isn't on PATH")
    dest.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        [
            "msiexec",
            "/a",
            str(msi_path.resolve()),
            "/qn",
            f"TARGETDIR={dest.resolve()}",
        ],
        capture_output=True,
    )
    if proc.returncode != 0:
        raise BundleError(
            f"{msi_path}: msiexec /a failed (exit {proc.returncode}): "
            f"{proc.stderr.decode(errors='replace') or proc.stdout.decode(errors='replace')}"
        )


def _find_7z() -> str | None:
    found = shutil.which("7z") or shutil.which("7z.exe")
    if found:
        return found
    import os

    for env_var in ("ProgramFiles", "ProgramFiles(x86)"):
        candidate = Path(os.environ.get(env_var, "")) / "7-Zip" / "7z.exe"
        if candidate.is_file():
            return str(candidate)
    return None


def extract_nsis(exe_path: Path, dest: Path) -> None:
    """Unpacks a Tauri/NSIS installer `.exe` with the 7-Zip CLI (preinstalled on GitHub's
    `windows-latest` runner image; 7-Zip can open NSIS's own archive format directly)."""
    seven_zip = _find_7z()
    if seven_zip is None:
        raise BundleError(f"{exe_path}: needs the '7z' CLI to unpack, and it isn't on PATH")
    dest.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        [seven_zip, "x", str(exe_path.resolve()), f"-o{dest.resolve()}", "-y"],
        capture_output=True,
    )
    if proc.returncode != 0:
        raise BundleError(
            f"{exe_path}: 7z x failed (exit {proc.returncode}): "
            f"{proc.stderr.decode(errors='replace') or proc.stdout.decode(errors='replace')}"
        )


def check_bundle_file(path: Path) -> list[str]:
    """Extracts `path` (a `.msi` or NSIS `.exe`) and checks it, prefixing any error with `path`."""
    with tempfile.TemporaryDirectory() as tmp:
        dest = Path(tmp) / "extracted"
        try:
            if path.suffix.lower() == ".msi":
                extract_msi(path, dest)
            elif path.suffix.lower() == ".exe":
                extract_nsis(path, dest)
            else:
                return [f"{path}: unrecognized bundle type (expected .msi or .exe)"]
        except BundleError as exc:
            return [f"{path}: {exc}"]
        return [f"{path}: {e}" for e in check_windows_tree(dest)]


def find_bundles(target_dir: Path) -> list[Path]:
    msis = sorted(target_dir.glob("*/bundle/msi/*.msi"))
    nsis_exes = sorted(target_dir.glob("*/bundle/nsis/*.exe"))
    return msis + nsis_exes


def main() -> int:
    paths = [Path(p) for p in sys.argv[1:]] or find_bundles(REPO_ROOT / "target")
    if not paths:
        print("no .msi or NSIS .exe bundles found under target/*/bundle/ — run `just build` first.")
        return 1

    all_errors: list[str] = []
    for path in paths:
        errors = check_bundle_file(path)
        all_errors.extend(errors)
        status = "OK" if not errors else "FAIL"
        print(f"[{status}] {path}")

    if all_errors:
        print("\n".join(all_errors), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
