#!/usr/bin/env python3
"""Unit tests for check_bundle.py's tree-checking logic, against fake bundle trees (no real
`.deb`/AppImage archives — the real extraction paths are exercised by `just build` itself).

Run directly: `python3 scripts/packaging/test_check_bundle.py`
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_bundle


def make_tree(root: Path, *, app: bool, sandbox: bool, nested: bool = False) -> None:
    usr_bin = root / ("squashfs-root/usr/bin" if nested else "usr/bin")
    usr_bin.mkdir(parents=True)
    if app:
        (usr_bin / check_bundle.APP_BINARY).write_bytes(b"#!/bin/sh\n")
    if sandbox:
        (usr_bin / check_bundle.SANDBOX_BINARY).write_bytes(b"#!/bin/sh\n")


class FindUsrBinTests(unittest.TestCase):
    def test_top_level(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True)
            self.assertEqual(check_bundle.find_usr_bin(root), root / "usr" / "bin")

    def test_one_level_nested(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True, nested=True)
            found = check_bundle.find_usr_bin(root)
            self.assertEqual(found, root / "squashfs-root" / "usr" / "bin")

    def test_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "usr").mkdir()
            self.assertIsNone(check_bundle.find_usr_bin(root))


class CheckTreeTests(unittest.TestCase):
    def test_both_present_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=True)
            self.assertEqual(check_bundle.check_tree(root), [])

    def test_missing_sandbox_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=False)
            errors = check_bundle.check_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle.SANDBOX_BINARY, errors[0])

    def test_missing_app_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=False, sandbox=True)
            errors = check_bundle.check_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle.APP_BINARY, errors[0])

    def test_both_missing_fails_with_two_errors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=False, sandbox=False)
            self.assertEqual(len(check_bundle.check_tree(root)), 2)

    def test_no_usr_bin_at_all_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            errors = check_bundle.check_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("usr/bin", errors[0])

    def test_sandbox_present_but_not_a_file_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, app=True, sandbox=False)
            (root / "usr" / "bin" / check_bundle.SANDBOX_BINARY).mkdir()
            errors = check_bundle.check_tree(root)
            self.assertEqual(len(errors), 1)


def make_macos_tree(root: Path, *, app: bool, sandbox: bool) -> None:
    bin_dir = root / "Contents" / "MacOS"
    bin_dir.mkdir(parents=True)
    if app:
        (bin_dir / check_bundle.APP_BINARY).write_bytes(b"#!/bin/sh\n")
    if sandbox:
        (bin_dir / check_bundle.SANDBOX_BINARY).write_bytes(b"#!/bin/sh\n")


class FindMacosBinDirTests(unittest.TestCase):
    def test_direct_app_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "PowerVoice.app"
            make_macos_tree(root, app=True, sandbox=True)
            self.assertEqual(check_bundle.find_macos_bin_dir(root), root / "Contents" / "MacOS")

    def test_one_level_wrapped(self) -> None:
        # e.g. a mounted .dmg's root, holding the .app alongside other volume contents.
        with tempfile.TemporaryDirectory() as tmp:
            mount_root = Path(tmp)
            app_root = mount_root / "PowerVoice.app"
            make_macos_tree(app_root, app=True, sandbox=True)
            found = check_bundle.find_macos_bin_dir(mount_root)
            self.assertEqual(found, app_root / "Contents" / "MacOS")

    def test_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertIsNone(check_bundle.find_macos_bin_dir(root))


class CheckMacosTreeTests(unittest.TestCase):
    def test_both_present_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "PowerVoice.app"
            make_macos_tree(root, app=True, sandbox=True)
            self.assertEqual(check_bundle.check_macos_tree(root), [])

    def test_missing_sandbox_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "PowerVoice.app"
            make_macos_tree(root, app=True, sandbox=False)
            errors = check_bundle.check_macos_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle.SANDBOX_BINARY, errors[0])

    def test_no_contents_macos_at_all_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "PowerVoice.app"
            root.mkdir()
            errors = check_bundle.check_macos_tree(root)
            self.assertEqual(len(errors), 1)
            self.assertIn("Contents/MacOS", errors[0])


class CheckBundleFileMacosAppTests(unittest.TestCase):
    def test_app_directory_checked_in_place_no_extraction(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app_root = Path(tmp) / "PowerVoice.app"
            make_macos_tree(app_root, app=True, sandbox=True)
            self.assertEqual(check_bundle.check_bundle_file(app_root), [])

    def test_app_directory_missing_sandbox_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            app_root = Path(tmp) / "PowerVoice.app"
            make_macos_tree(app_root, app=True, sandbox=False)
            errors = check_bundle.check_bundle_file(app_root)
            self.assertEqual(len(errors), 1)
            self.assertIn(check_bundle.SANDBOX_BINARY, errors[0])


class FindBundlesTests(unittest.TestCase):
    def test_finds_deb_and_appimage_under_any_profile_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            deb_dir = target / "release" / "bundle" / "deb"
            appimage_dir = target / "release" / "bundle" / "appimage"
            deb_dir.mkdir(parents=True)
            appimage_dir.mkdir(parents=True)
            (deb_dir / "PowerVoice_0.1.0_amd64.deb").write_bytes(b"")
            (appimage_dir / "PowerVoice_0.1.0_amd64.AppImage").write_bytes(b"")
            (target / "release" / "bundle" / "rpm").mkdir(parents=True)

            found = check_bundle.find_bundles(target)
            self.assertEqual(
                sorted(p.name for p in found),
                ["PowerVoice_0.1.0_amd64.AppImage", "PowerVoice_0.1.0_amd64.deb"],
            )

    def test_finds_macos_app_and_dmg_under_any_profile_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            app_dir = target / "release" / "bundle" / "macos"
            dmg_dir = target / "release" / "bundle" / "dmg"
            app_dir.mkdir(parents=True)
            dmg_dir.mkdir(parents=True)
            (app_dir / "PowerVoice.app").mkdir()
            (dmg_dir / "PowerVoice_0.1.0_aarch64.dmg").write_bytes(b"")

            found = check_bundle.find_bundles(target)
            self.assertEqual(
                sorted(p.name for p in found),
                ["PowerVoice.app", "PowerVoice_0.1.0_aarch64.dmg"],
            )

    def test_empty_when_nothing_built(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(check_bundle.find_bundles(Path(tmp)), [])


class ArReaderTests(unittest.TestCase):
    def test_rejects_bad_magic(self) -> None:
        with self.assertRaises(check_bundle.BundleError):
            check_bundle._read_ar_members(b"not an ar archive at all")

    def test_reads_members_back(self) -> None:
        def ar_member(name: str, data: bytes) -> bytes:
            header = (
                name.ljust(16)[:16]
                + "0".ljust(12)
                + "0".ljust(6)
                + "0".ljust(6)
                + "100644".ljust(8)
                + str(len(data)).ljust(10)
                + "`\n"
            ).encode("ascii")
            body = data
            if len(body) % 2 == 1:
                body += b"\n"
            return header + body

        archive = b"!<arch>\n" + ar_member("debian-binary", b"2.0\n") + ar_member(
            "control.tar.gz", b"fake-control-bytes"
        )
        members = check_bundle._read_ar_members(archive)
        self.assertEqual(members["debian-binary"], b"2.0\n")
        self.assertEqual(members["control.tar.gz"], b"fake-control-bytes")


if __name__ == "__main__":
    unittest.main()
