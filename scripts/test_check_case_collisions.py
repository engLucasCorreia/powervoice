#!/usr/bin/env python3
"""Unit tests for scripts/check_case_collisions.py (H-69), against small in-memory path lists.
The real run over the repository happens in `just check`.

Run directly: `python3 scripts/test_check_case_collisions.py`
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_case_collisions as check  # noqa: E402


class ResolutionCandidatesTests(unittest.TestCase):
    def test_non_ts_file_only_resolves_as_itself(self) -> None:
        self.assertEqual(check.resolution_candidates("ui/src/lib/menu/MenuBar.svelte"),
                         {"ui/src/lib/menu/MenuBar.svelte"})

    def test_svelte_ts_module_also_resolves_without_the_trailing_ts(self) -> None:
        self.assertEqual(
            check.resolution_candidates("ui/src/lib/menu/menubar.svelte.ts"),
            {"ui/src/lib/menu/menubar.svelte.ts", "ui/src/lib/menu/menubar.svelte"},
        )

    def test_declaration_file_is_not_stripped(self) -> None:
        # A `.d.ts` file is never itself the target of an import specifier that drops `.ts`.
        self.assertEqual(check.resolution_candidates("ui/src/lib/ipc/bindings.d.ts"),
                         {"ui/src/lib/ipc/bindings.d.ts"})

    def test_plain_ts_module_also_loses_its_extension(self) -> None:
        self.assertEqual(
            check.resolution_candidates("ui/src/lib/state/edit.ts"),
            {"ui/src/lib/state/edit.ts", "ui/src/lib/state/edit"},
        )


class FindCollisionsTests(unittest.TestCase):
    def test_no_collision_among_unrelated_files(self) -> None:
        paths = [
            "ui/src/lib/menu/MenuBar.svelte",
            "ui/src/lib/menu/MenuBarMenu.svelte",
            "ui/src/lib/menu/menu.svelte.ts",
            "ui/src/lib/help/ShortcutsDialog.svelte",
            "ui/src/lib/help/shortcuts.svelte.ts",
        ]
        self.assertEqual(check.find_collisions(paths), [])

    def test_reproduces_the_h61_menubar_collision(self) -> None:
        paths = [
            "ui/src/lib/menu/MenuBar.svelte",
            "ui/src/lib/menu/menubar.svelte.ts",
        ]
        collisions = check.find_collisions(paths)
        self.assertEqual(len(collisions), 1)
        lowered, a, b = collisions[0]
        self.assertEqual(lowered, "ui/src/lib/menu/menubar.svelte")
        self.assertEqual((a, b), ("ui/src/lib/menu/MenuBar.svelte", "ui/src/lib/menu/menubar.svelte.ts"))

    def test_reproduces_the_h61_shortcuts_dialog_collision(self) -> None:
        paths = [
            "ui/src/lib/help/ShortcutsDialog.svelte",
            "ui/src/lib/help/shortcutsDialog.svelte.ts",
        ]
        collisions = check.find_collisions(paths)
        self.assertEqual(len(collisions), 1)
        lowered, _a, _b = collisions[0]
        self.assertEqual(lowered, "ui/src/lib/help/shortcutsdialog.svelte")

    def test_two_plain_files_differing_only_by_case_collide(self) -> None:
        paths = ["docs/Readme.md", "docs/README.md"]
        collisions = check.find_collisions(paths)
        self.assertEqual(len(collisions), 1)

    def test_identical_path_list_twice_over_is_not_a_false_collision(self) -> None:
        # A file's own `.ts`-stripped candidate matching *itself* is not a second source.
        paths = ["ui/src/lib/state/edit.ts"]
        self.assertEqual(check.find_collisions(paths), [])

    def test_three_way_collision_reports_every_pair_once(self) -> None:
        paths = ["a/Foo.ts", "a/foo.ts", "a/FOO.TS"]
        # foo.ts and FOO.TS are literally distinct tracked paths (differ only by case) even
        # before considering the .ts-stripped candidate; every unordered pair should appear once.
        collisions = check.find_collisions(paths)
        pairs = {(a, b) for _lowered, a, b in collisions}
        self.assertEqual(
            pairs,
            {
                ("a/FOO.TS", "a/Foo.ts"),
                ("a/FOO.TS", "a/foo.ts"),
                ("a/Foo.ts", "a/foo.ts"),
            },
        )


class MainTests(unittest.TestCase):
    def test_real_repository_tree_has_no_collisions(self) -> None:
        # This is the actual guard `just check` relies on: run it over the real tracked tree.
        paths = check.tracked_paths(check.REPO_ROOT)
        self.assertGreater(len(paths), 100, "expected git ls-files to find the real repo tree")
        collisions = check.find_collisions(paths)
        self.assertEqual(collisions, [], f"unexpected case collisions: {collisions}")


if __name__ == "__main__":
    unittest.main()
