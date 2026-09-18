#!/usr/bin/env python3
"""Fails if a built Linux/macOS bundle lacks the plugin sandbox (H-45 packaging check; H-61 added
the macOS half — Windows has its own sibling script, `check_bundle_windows.py`, since MSI/NSIS
need Windows-only tools to open) or ships a denylisted library (H-89; see `appimage_denylist.py`).

The app finds the sandbox beside its own executable
(`vox_plugin_host::SandboxOptions::beside_current_exe`), so every bundle this project ships must
contain `powervoice-sandbox` next to `powervoice-app` (`usr/bin/` in a `.deb`/AppImage,
`Contents/MacOS/` in a macOS `.app`/`.dmg`). Nothing enforced that before H-45 — a local
`just build` silently produced a bundle plugins couldn't load into.

Usage:
    python3 scripts/packaging/check_bundle.py [path-to-bundle ...]

With no arguments, checks every `*.deb`, `*.AppImage`, `*.app` and `*.dmg` under `target/*/bundle/`
(i.e. whatever `just build` / `scripts/packaging/tauri_build.sh` most recently produced). Exits
non-zero, with the offending file(s), on any failure.

No extra dependencies: `.deb` archives (an `ar` archive of `debian-binary`, `control.tar.*` and
`data.tar.*`) are unpacked with a small pure-Python `ar` reader plus the stdlib `tarfile` (falling
back to the `zstd` CLI for `data.tar.zst`, which newer `dpkg`/bundlers default to and stdlib
`tarfile` can't read); AppImages extract themselves via their own `--appimage-extract` (no FUSE
needed, unlike actually running one); a macOS `.app` is already a plain directory tree, no
extraction needed; a `.dmg` is mounted read-only with the macOS-only `hdiutil` (so the `.dmg`
branch only actually runs on a macOS host, same as the AppImage branch only really exercises
`--appimage-extract` on Linux — `just check`'s unit tests only cover the pure tree-checking logic
against fake trees, on any OS).
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from appimage_denylist import DENYLISTED_LIBS  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
APP_BINARY = "powervoice-app"
SANDBOX_BINARY = "powervoice-sandbox"


class BundleError(Exception):
    """Raised for a malformed archive (as opposed to a missing binary, which is a plain error string)."""


# --- the testable core: given an already-extracted tree, is the sandbox there? -------------------


def find_usr_bin(root: Path) -> Path | None:
    """Locates the `usr/bin` directory in an extracted `.deb`/AppImage tree.

    Both layouts put binaries at the top-level `usr/bin/`; this also tolerates it being nested
    one level down (e.g. `squashfs-root/usr/bin`, a stray wrapping directory) since callers may
    hand this a slightly different root than expected.
    """
    direct = root / "usr" / "bin"
    if direct.is_dir():
        return direct
    for candidate in sorted(root.glob("*/usr/bin")):
        if candidate.is_dir():
            return candidate
    return None


def check_usr_bin(usr_bin: Path) -> list[str]:
    """Checks that both binaries are present (and are files, not e.g. dangling symlinks)."""
    errors = []
    for name in (APP_BINARY, SANDBOX_BINARY):
        path = usr_bin / name
        if not path.is_file():
            errors.append(f"missing {usr_bin}/{name}")
    return errors


def check_denylisted_libs(root: Path) -> list[str]:
    """H-89 guard: fails if any library on `appimage_denylist.DENYLISTED_LIBS` shipped inside the
    bundle. These are libraries that, like a graphics driver, must match something running on the
    end user's system (a daemon, a plugin ecosystem with a distro-specific path) rather than be
    vendored — bundling one broke every non-Ubuntu AppImage user (`libpipewire-0.3.so.0`; see that
    module's docstring). Scans the whole tree, not just `usr/lib`, so it also catches an
    unexpected location and keeps working if a future bundle layout moves libraries around.
    """
    errors = []
    for pattern in DENYLISTED_LIBS:
        for match in sorted(root.rglob(pattern)):
            if match.is_file() or match.is_symlink():
                errors.append(
                    f"denylisted library bundled: {match.relative_to(root)} "
                    f"(matches {pattern!r} in appimage_denylist.DENYLISTED_LIBS — H-89)"
                )
    return errors


def check_tree(root: Path) -> list[str]:
    """Checks an already-extracted bundle tree (a `.deb`'s data.tar, an AppImage's AppDir, ...)."""
    usr_bin = find_usr_bin(root)
    if usr_bin is None:
        return [f"no usr/bin directory found under {root}"]
    return check_usr_bin(usr_bin) + check_denylisted_libs(root)


def find_macos_bin_dir(root: Path) -> Path | None:
    """Locates `Contents/MacOS` in a macOS `.app` bundle (H-61).

    `root` is normally the `.app` directory itself; also tolerates one level of wrapping (e.g. a
    `.dmg`'s mount point, which holds the `.app` alongside a `.background`/`Applications` symlink)
    the same way `find_usr_bin` tolerates a stray AppImage wrapping directory.
    """
    direct = root / "Contents" / "MacOS"
    if direct.is_dir():
        return direct
    for candidate in sorted(root.glob("*.app/Contents/MacOS")):
        if candidate.is_dir():
            return candidate
    return None


def check_macos_bin_dir(bin_dir: Path) -> list[str]:
    """Checks that both binaries are present in `Contents/MacOS` (no `.exe`-style suffix on macOS)."""
    errors = []
    for name in (APP_BINARY, SANDBOX_BINARY):
        path = bin_dir / name
        if not path.is_file():
            errors.append(f"missing {bin_dir}/{name}")
    return errors


def check_macos_tree(root: Path) -> list[str]:
    """Checks an already-extracted macOS bundle tree (a `.app` directory, or a mounted `.dmg`)."""
    bin_dir = find_macos_bin_dir(root)
    if bin_dir is None:
        return [f"no Contents/MacOS directory found under {root}"]
    return check_macos_bin_dir(bin_dir) + check_denylisted_libs(root)


# --- extraction: real .deb / AppImage / .dmg archives -> a tree check_tree() can look at ---------


def _read_ar_members(data: bytes) -> dict[str, bytes]:
    """A minimal reader for the common `ar` format `.deb` files use (short member names, no GNU
    extended-name table — `.deb` only ever has three: `debian-binary`, `control.tar.*`,
    `data.tar.*`)."""
    if data[:8] != b"!<arch>\n":
        raise BundleError("not an ar archive (bad magic)")
    members: dict[str, bytes] = {}
    offset = 8
    while offset + 60 <= len(data):
        header = data[offset : offset + 60]
        name = header[0:16].decode("ascii", errors="replace").strip().rstrip("/")
        size_field = header[48:58].decode("ascii", errors="replace").strip()
        try:
            size = int(size_field)
        except ValueError as exc:
            raise BundleError(f"malformed ar header size {size_field!r}") from exc
        offset += 60
        members[name] = data[offset : offset + size]
        offset += size
        if offset % 2 == 1:  # ar pads each member to an even offset
            offset += 1
    return members


def _extract_tar_member(member_data: bytes, member_name: str, dest: Path) -> None:
    """Extracts a `data.tar(.gz|.xz|.bz2|.zst)` blob (by member name) into `dest`."""
    if member_name.endswith(".zst"):
        if shutil.which("zstd") is None:
            raise BundleError(
                f"{member_name} needs the 'zstd' CLI to decompress, and it isn't on PATH"
            )
        with tempfile.NamedTemporaryFile(suffix=".tar") as tar_file:
            proc = subprocess.run(
                ["zstd", "-d", "-f", "-o", tar_file.name],
                input=member_data,
                capture_output=True,
            )
            if proc.returncode != 0:
                raise BundleError(f"zstd -d failed: {proc.stderr.decode(errors='replace')}")
            with tarfile.open(tar_file.name, mode="r:") as tar:
                tar.extractall(dest, filter="data")
        return

    import io

    with tarfile.open(fileobj=io.BytesIO(member_data), mode="r:*") as tar:
        tar.extractall(dest, filter="data")


def extract_deb(deb_path: Path, dest: Path) -> None:
    members = _read_ar_members(deb_path.read_bytes())
    data_member = next((name for name in members if name.startswith("data.tar")), None)
    if data_member is None:
        raise BundleError(f"{deb_path}: no data.tar* member in the ar archive")
    _extract_tar_member(members[data_member], data_member, dest)


def extract_appimage(appimage_path: Path, dest: Path) -> None:
    resolved = appimage_path.resolve()
    resolved.chmod(resolved.stat().st_mode | 0o111)  # ensure it's executable
    with tempfile.TemporaryDirectory() as work_dir:
        proc = subprocess.run(
            [str(resolved), "--appimage-extract"],
            cwd=work_dir,
            capture_output=True,
        )
        if proc.returncode != 0:
            raise BundleError(
                f"{appimage_path}: --appimage-extract failed: {proc.stderr.decode(errors='replace')}"
            )
        squashfs_root = Path(work_dir) / "squashfs-root"
        if not squashfs_root.is_dir():
            raise BundleError(f"{appimage_path}: --appimage-extract produced no squashfs-root")
        shutil.copytree(squashfs_root, dest)


def extract_dmg(dmg_path: Path, dest: Path) -> None:
    """Mounts a `.dmg` read-only via macOS's `hdiutil` and copies the `.app` it contains.

    macOS-only (there's no pure-Python or cross-platform way to open a `.dmg`) — like
    `extract_appimage` really only runs for real on the OS that produced the bundle.
    """
    if shutil.which("hdiutil") is None:
        raise BundleError(f"{dmg_path}: needs macOS's 'hdiutil' to mount, and it isn't on PATH")
    with tempfile.TemporaryDirectory() as mount_dir:
        proc = subprocess.run(
            ["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mount_dir, str(dmg_path)],
            capture_output=True,
        )
        if proc.returncode != 0:
            raise BundleError(f"{dmg_path}: hdiutil attach failed: {proc.stderr.decode(errors='replace')}")
        try:
            apps = sorted(Path(mount_dir).glob("*.app"))
            if not apps:
                raise BundleError(f"{dmg_path}: no .app found inside the mounted image")
            shutil.copytree(apps[0], dest)
        finally:
            subprocess.run(["hdiutil", "detach", mount_dir, "-quiet"], capture_output=True)


def check_bundle_file(path: Path) -> list[str]:
    """Extracts `path` (a `.deb`, `.AppImage`, `.app` or `.dmg`) and checks it, prefixing any
    error with `path`."""
    if path.name.endswith(".app") and path.is_dir():
        # Already a plain directory tree - nothing to extract.
        return [f"{path}: {e}" for e in check_macos_tree(path)]

    with tempfile.TemporaryDirectory() as tmp:
        dest = Path(tmp) / "extracted"
        try:
            if path.suffix == ".deb":
                extract_deb(path, dest)
            elif path.name.endswith(".AppImage"):
                extract_appimage(path, dest)
            elif path.name.endswith(".dmg"):
                extract_dmg(path, dest)
                return [f"{path}: {e}" for e in check_macos_tree(dest)]
            else:
                return [f"{path}: unrecognized bundle type (expected .deb, .AppImage, .app or .dmg)"]
        except BundleError as exc:
            return [f"{path}: {exc}"]
        return [f"{path}: {e}" for e in check_tree(dest)]


def find_bundles(target_dir: Path) -> list[Path]:
    debs = sorted(target_dir.glob("*/bundle/deb/*.deb"))
    appimages = sorted(target_dir.glob("*/bundle/appimage/*.AppImage"))
    apps = sorted(target_dir.glob("*/bundle/macos/*.app"))
    dmgs = sorted(target_dir.glob("*/bundle/dmg/*.dmg"))
    return debs + appimages + apps + dmgs


def main() -> int:
    paths = [Path(p) for p in sys.argv[1:]] or find_bundles(REPO_ROOT / "target")
    if not paths:
        print(
            "no .deb, .AppImage, .app or .dmg bundles found under target/*/bundle/ — "
            "run `just build` first."
        )
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
