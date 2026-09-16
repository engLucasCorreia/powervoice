#!/usr/bin/env python3
"""Unit tests for scripts/docs/check.py (T-706), against small in-memory documents and a temporary
repository tree. The real run over the repository happens in `just check`.

Run directly: `python3 scripts/docs/test_check.py`
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check  # noqa: E402


class SlugTests(unittest.TestCase):
    def test_github_rules(self) -> None:
        self.assertEqual(check.github_slug("Threading model"), "threading-model")
        self.assertEqual(check.github_slug("ADR-002 — Threading & real-time rules"),
                         "adr-002--threading--real-time-rules")
        self.assertEqual(check.github_slug("The `vox-engine` crate"), "the-vox-engine-crate")
        self.assertEqual(check.github_slug("[Link](x.md) and **bold**"), "link-and-bold")
        self.assertEqual(check.github_slug("snake_case stays"), "snake_case-stays")

    def test_duplicates_get_suffixes(self) -> None:
        anchors = check.anchors_of("# A\n## A\n### A\n")
        self.assertEqual(anchors, {"a", "a-1", "a-2"})

    def test_headings_in_code_are_ignored(self) -> None:
        anchors = check.anchors_of("# Real\n```\n# not a heading\n```\n")
        self.assertEqual(anchors, {"real"})

    def test_explicit_anchor(self) -> None:
        self.assertIn("here", check.anchors_of('Text <a id="here"></a>\n'))


class LinkTests(unittest.TestCase):
    def test_finds_inline_reference_and_html_links(self) -> None:
        text = ("See [a](a.md) and ![img](i.png \"t\").\n"
                "[ref]: b.md#x\n"
                '<img src="c.png"> `[not](code.md)`\n'
                "```\n[also not](fence.md)\n```\n")
        self.assertEqual([t for _, t in check.links_of(text)], ["a.md", "i.png", "b.md#x", "c.png"])

    def test_broken_links_and_anchors(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs").mkdir()
            (root / "docs/b.md").write_text("# Title\n## Sub part\n", encoding="utf-8")
            (root / "docs/a.md").write_text(
                "[ok](b.md#sub-part) [bad anchor](b.md#nope) [missing](c.md) "
                "[self](#top) [web](https://example.com) [dir](.)\n# Top\n",
                encoding="utf-8",
            )
            problems = check.check_links(root, [root / "docs/a.md"])
            messages = [p.message for p in problems]
            self.assertEqual(messages, ["missing anchor: b.md#nope", "broken link: c.md"])

    def test_hub_must_link_every_doc(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/sub").mkdir(parents=True)
            (root / "docs/a.md").write_text("# A\n", encoding="utf-8")
            (root / "docs/sub/b.md").write_text("# B\n", encoding="utf-8")
            (root / "docs/README.md").write_text("[A](a.md)\n", encoding="utf-8")
            problems = check.check_hub(root)
            self.assertEqual([p.message for p in problems], ["docs/sub/b.md is not linked from the hub"])


class MermaidTests(unittest.TestCase):
    def lint(self, text: str) -> list[str]:
        return [msg for _, msg in check.lint_mermaid(text.strip("\n").split("\n"))]

    def test_valid_flowchart(self) -> None:
        self.assertEqual(self.lint("""
flowchart LR
  subgraph core["Rust core (crates)"]
    a["engine (RT)"] -->|"ring (SPSC)"| b[rack]
  end
  b --> c((disk))
"""), [])

    def test_valid_sequence(self) -> None:
        self.assertEqual(self.lint("""
sequenceDiagram
  participant UI as UI (Svelte)
  UI->>App: transport_play()
  alt playing
    App-->>UI: state (ok)
  else stopped
    Note over UI,App: nothing (yet
  end
"""), [])

    def test_unknown_type_and_empty(self) -> None:
        self.assertEqual(self.lint("flowchartx TD\n a --> b"), ["unknown diagram type 'flowchartx'"])
        self.assertEqual(self.lint("\n%% only a comment\n"), ["empty mermaid block"])

    def test_unbalanced(self) -> None:
        self.assertEqual(self.lint('flowchart TD\n  a["x"] --> b[y\n'), ["unclosed '['"])
        self.assertIn("1 subgraph(s) but 0 'end' line(s)", self.lint("flowchart TD\n subgraph s\n a --> b\n"))

    def test_unquoted_label_with_parentheses(self) -> None:
        problems = self.lint("flowchart TD\n  a[engine (RT)] --> b\n")
        self.assertEqual(len(problems), 1)
        self.assertIn("unquoted label", problems[0])

    def test_sequence_blocks_and_semicolons(self) -> None:
        self.assertEqual(self.lint("sequenceDiagram\n  loop tick\n  A->>B: x\n"),
                         ["1 block(s) not closed with 'end'"])
        self.assertEqual(self.lint("sequenceDiagram\n  A->>B: one; two\n"),
                         ["';' in sequence text ends the statement (use ',' or '#59;')"])

    def test_node_named_end(self) -> None:
        self.assertIn("a node named 'end' breaks flowcharts", self.lint("flowchart TD\n  a --> end\n"))


class GeneratedSectionTests(unittest.TestCase):
    def test_replace_section(self) -> None:
        text = "a\n<!-- BEGIN GENERATED: x -->\nold\n<!-- END GENERATED: x -->\nb\n"
        self.assertEqual(check.replace_section(text, "x", "new"),
                         "a\n<!-- BEGIN GENERATED: x -->\nnew\n<!-- END GENERATED: x -->\nb\n")

    def test_markers_inside_code_are_ignored(self) -> None:
        text = ("A doc may mention `<!-- BEGIN GENERATED: x -->` inline,\n"
                "```\n<!-- BEGIN GENERATED: y -->\n```\n"
                "<!-- BEGIN GENERATED: real -->\nold\n<!-- END GENERATED: real -->\n")
        self.assertEqual([name for name, _, _ in check.find_sections(text)], ["real"])
        self.assertIn("new", check.replace_section(text, "real", "new"))

    def test_missing_end_marker(self) -> None:
        with self.assertRaises(ValueError):
            check.find_sections("<!-- BEGIN GENERATED: x -->\n")

    def test_crate_graph_from_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ("app", "core", "util"):
                (root / "crates" / name).mkdir(parents=True)
            metadata = {"packages": [
                {"name": "app", "manifest_path": str(root / "crates/app/Cargo.toml"),
                 "targets": [{"kind": ["bin"]}],
                 "dependencies": [{"name": "core", "kind": None}, {"name": "util", "kind": "dev"},
                                  {"name": "serde", "kind": None}]},
                {"name": "core", "manifest_path": str(root / "crates/core/Cargo.toml"),
                 "targets": [{"kind": ["lib"]}], "dependencies": []},
                {"name": "util", "manifest_path": str(root / "crates/util/Cargo.toml"),
                 "targets": [{"kind": ["lib"]}], "dependencies": [{"name": "core", "kind": None}]},
            ]}
            out = check.gen_crate_graph(root, root / "docs/x.md", metadata)
            self.assertIn("  app --> core", out)
            self.assertIn("  util --> core", out)
            self.assertNotIn("app --> util", out)  # dev-only edges stay out of the graph
            self.assertNotIn("serde", out)  # external crates stay out
            self.assertIn("| `app` | [`crates/app`](../crates/app) | bin | `core` | `util` |", out)
            self.assertEqual(check.lint_mermaid(out.split("```")[1].split("\n")[1:]), [])

    def test_ipc_tables_from_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ipc = root / "src-tauri/src/ipc"
            ipc.mkdir(parents=True)
            (ipc / "mod.rs").write_text(
                '#[cfg(not(feature = "spike"))]\ncrate::ipc_commands!(\n    alpha,\n    beta,\n);\n'
                '#[cfg(feature = "spike")]\ncrate::ipc_commands!(alpha, beta, spike_only);\n',
                encoding="utf-8",
            )
            (ipc / "events.rs").write_text(
                "crate::ipc_events!(\n    ping,\n    pong\n);\n"
                "pub fn emit_pong(app: &A) -> R {\n    app.emit(EventName::pong.as_str(), 1)\n}\n",
                encoding="utf-8",
            )
            (ipc / "a_commands.rs").write_text(
                "/// Streams frames. More detail | here.\n#[tauri::command]\n"
                "pub async fn alpha(channel: Channel) -> Result<(), E> {\n}\n\n"
                "/// Binary peaks, e.g. for the view. Second sentence.\n#[tauri::command]\n"
                "pub async fn beta(\n    x: u32,\n) -> Result<Response, E> {\n"
                "    app.emit(EventName::ping.as_str(), x);\n}\n"
                "#[cfg(test)]\nmod tests {\n    fn t() { emit_pong(&a); }\n}\n",
                encoding="utf-8",
            )
            (root / "src-tauri/src/other.rs").write_text("fn f() { emit_pong(&app); }\n", encoding="utf-8")
            doc = root / "docs/architecture/ipc.md"
            commands = check.gen_ipc_commands(root, doc)
            self.assertIn("| 1 | `alpha` | [`a_commands.rs`](../../src-tauri/src/ipc/a_commands.rs) "
                          "| stream (`Channel`) | Streams frames. |", commands)
            self.assertIn("| 2 | `beta` |", commands)
            self.assertIn("| binary (`Response`) | Binary peaks, e.g. for the view. |", commands)
            self.assertNotIn("spike_only", commands)
            events = check.gen_ipc_events(root, doc)
            self.assertIn("| 1 | `ping` | [`ipc/a_commands.rs`](../../src-tauri/src/ipc/a_commands.rs) |", events)
            # `pong` is emitted through the `emit_pong` helper; test modules don't count.
            self.assertIn("| 2 | `pong` | [`other.rs`](../../src-tauri/src/other.rs) |", events)


if __name__ == "__main__":
    unittest.main()
