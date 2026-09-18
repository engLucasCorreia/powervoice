#!/usr/bin/env python3
"""Unit tests for strip_appimage_libs.py's pure logic, against fake AppDir trees (no real
AppImage/appimagetool — repacking is exercised manually, per H-89's final report, the same way
check_bundle.py's real extraction paths are exercised by `just build` itself).

Run directly: `python3 scripts/packaging/test_strip_appimage_libs.py`
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import strip_appimage_libs


class RemoveDenylistedTests(unittest.TestCase):
    def test_removes_denylisted_library(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            appdir = Path(tmp)
            lib_dir = appdir / "usr" / "lib"
            lib_dir.mkdir(parents=True)
            target = lib_dir / "libpipewire-0.3.so.0"
            target.write_bytes(b"")
            removed = strip_appimage_libs.remove_denylisted(appdir)
            self.assertEqual(removed, ["usr/lib/libpipewire-0.3.so.0"])
            self.assertFalse(target.exists())

    def test_leaves_unrelated_libraries_alone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            appdir = Path(tmp)
            lib_dir = appdir / "usr" / "lib"
            lib_dir.mkdir(parents=True)
            kept = lib_dir / "libasound.so.2"
            kept.write_bytes(b"")
            removed = strip_appimage_libs.remove_denylisted(appdir)
            self.assertEqual(removed, [])
            self.assertTrue(kept.exists())

    def test_no_usr_lib_at_all_is_a_noop(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            appdir = Path(tmp)
            (appdir / "usr" / "bin").mkdir(parents=True)
            self.assertEqual(strip_appimage_libs.remove_denylisted(appdir), [])

    def test_versioned_filename_removed_too(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            appdir = Path(tmp)
            lib_dir = appdir / "usr" / "lib"
            lib_dir.mkdir(parents=True)
            target = lib_dir / "libpipewire-0.3.so.0.123.0"
            target.write_bytes(b"")
            removed = strip_appimage_libs.remove_denylisted(appdir)
            self.assertEqual(removed, ["usr/lib/libpipewire-0.3.so.0.123.0"])


class FindAppimagesTests(unittest.TestCase):
    def test_finds_appimage_under_any_profile_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            appimage_dir = target / "release" / "bundle" / "appimage"
            appimage_dir.mkdir(parents=True)
            (appimage_dir / "PowerVoice_0.1.0_amd64.AppImage").write_bytes(b"")
            (target / "release" / "bundle" / "deb").mkdir(parents=True)

            found = strip_appimage_libs.find_appimages(target)
            self.assertEqual([p.name for p in found], ["PowerVoice_0.1.0_amd64.AppImage"])

    def test_empty_when_nothing_built(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(strip_appimage_libs.find_appimages(Path(tmp)), [])


class MainNoAppimagesTests(unittest.TestCase):
    def test_main_is_a_noop_when_nothing_to_strip(self) -> None:
        # No arguments and an empty target/ tree: exits 0 without needing a repack tool at all
        # (the Windows/macOS release jobs never build an AppImage, so this must not fail there).
        with tempfile.TemporaryDirectory() as tmp:
            empty_dir = Path(tmp) / "nonexistent"
            self.assertEqual(strip_appimage_libs.find_appimages(empty_dir), [])


if __name__ == "__main__":
    unittest.main()
