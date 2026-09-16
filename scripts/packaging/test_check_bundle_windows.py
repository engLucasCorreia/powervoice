#!/usr/bin/env python3
"""Unit tests for check_bundle_windows.py's tree-checking logic, against fake extracted trees (no
real `.msi`/NSIS `.exe` archives — those need Windows-only tools, `msiexec`/`7z`, and are only
really exercised by the Windows CI job itself).

Run directly: `python3 scripts/packaging/test_check_bundle_windows.py`
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_bundle_windows


def make_tree(root: Path, *, app: bool, sandbox: bool, subdir: str = "PowerVoice") -> Path:
    """Builds a fake extracted-installer tree; returns the directory the binaries would live in."""
    bin_dir = root / subdir
    bin_dir.mkdir(parents=True)
    if app:
        (bin_dir / check_bundle_windows.APP_BINARY).write_bytes(b"MZ")
    if sandbox:
        (bin_dir / check_bundle_windows.SANDBOX_BINARY).write_bytes(b"MZ")
    return bin_dir


class FindAllTests(unittest.TestCase):
    def test_finds_nested_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = make_tree(root, app=True, sandbox=True)
            found = check_bundle_windows.find_all(root, check_bundle_windows.APP_BINARY)
            self.assertEqual(found, [bin_dir / check_bundle_windows.APP_BINARY])

    def test_case_insensitive(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "PowerVoice"
            bin_dir.mkdir()
            (bin_dir / "PowerVoice-App.EXE").write_bytes(b"MZ")
            found = check_bundle_windows.find_all(root, check_bundle_windows.APP_BINARY)
            self.assertEqual(len(found), 1)

    def test_empty_when_absent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            root.mkdir(exist_ok=True)
            self.assertEqual(check_bundle_windows.find_all(root, check_bundle_windows.APP_BINARY), [])


class CheckWindowsTreeTests(unittest.TestCase):
    def test_both_present_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True)
            self.assertEqual(check_bundle_windows.check_windows_tree(root), [])

    def test_missing_sandbox_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=False)
            errors = check_bundle_windows.check_windows_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle_windows.SANDBOX_BINARY, errors[0])

    def test_no_app_binary_anywhere_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=False, sandbox=True)
            errors = check_bundle_windows.check_windows_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle_windows.APP_BINARY, errors[0])

    def test_nested_install_dir_still_found(self) -> None:
        # WiX administrative installs and NSIS payloads both nest arbitrarily deep;
        # check_windows_tree must not assume a fixed depth.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True, subdir="a/b/c/PowerVoice")
            self.assertEqual(check_bundle_windows.check_windows_tree(root), [])

    def test_two_app_copies_each_need_their_own_sandbox(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True, subdir="good")
            make_tree(root, app=True, sandbox=False, subdir="bad")
            errors = check_bundle_windows.check_windows_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("bad", errors[0])


class FindBundlesTests(unittest.TestCase):
    def test_finds_msi_and_nsis_exe_under_any_profile_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            msi_dir = target / "release" / "bundle" / "msi"
            nsis_dir = target / "release" / "bundle" / "nsis"
            msi_dir.mkdir(parents=True)
            nsis_dir.mkdir(parents=True)
            (msi_dir / "PowerVoice_0.1.0_x64_en-US.msi").write_bytes(b"")
            (nsis_dir / "PowerVoice_0.1.0_x64-setup.exe").write_bytes(b"")

            found = check_bundle_windows.find_bundles(target)
            self.assertEqual(
                sorted(p.name for p in found),
                ["PowerVoice_0.1.0_x64-setup.exe", "PowerVoice_0.1.0_x64_en-US.msi"],
            )

    def test_empty_when_nothing_built(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(check_bundle_windows.find_bundles(Path(tmp)), [])


class CheckBundleFileTests(unittest.TestCase):
    def test_unrecognized_extension_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "not-an-installer.txt"
            path.write_bytes(b"")
            errors = check_bundle_windows.check_bundle_file(path)
            self.assertEqual(len(errors), 1)
            self.assertIn("unrecognized bundle type", errors[0])


if __name__ == "__main__":
    unittest.main()
