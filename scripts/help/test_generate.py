#!/usr/bin/env python3
"""Unit tests for scripts/help/generate.py (H-107), against small in-memory documents.

Run directly: `python3 scripts/help/test_generate.py`
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import generate  # noqa: E402


class BlockParsingTests(unittest.TestCase):
    def test_heading_levels_and_ids(self) -> None:
        blocks = generate.parse_blocks("# Title\n\n## Section One\n\n### Sub heading\n")
        self.assertEqual([type(b).__name__ for b in blocks], ["RawHeading", "RawHeading", "RawHeading"])
        self.assertEqual(blocks[0].level, 1)
        self.assertEqual(blocks[1].id, "section-one")
        self.assertEqual(blocks[2].id, "sub-heading")

    def test_wrapped_paragraph_joins_lines(self) -> None:
        blocks = generate.parse_blocks("This is a long\nparagraph that wraps\nacross lines.\n")
        self.assertEqual(len(blocks), 1)
        self.assertIsInstance(blocks[0], generate.RawPara)
        self.assertEqual(blocks[0].text, "This is a long paragraph that wraps across lines.")

    def test_list_with_wrapped_continuation(self) -> None:
        text = "- **Plugins**: does a thing and\n  keeps going on the next line.\n- **Folders**: a second item.\n"
        blocks = generate.parse_blocks(text)
        self.assertEqual(len(blocks), 1)
        lst = blocks[0]
        self.assertIsInstance(lst, generate.RawList)
        self.assertEqual(lst.kind, "ul")
        self.assertEqual(lst.items[0], "**Plugins**: does a thing and keeps going on the next line.")
        self.assertEqual(lst.items[1], "**Folders**: a second item.")

    def test_ordered_list(self) -> None:
        blocks = generate.parse_blocks("1. First\n2. Second\n")
        self.assertIsInstance(blocks[0], generate.RawList)
        self.assertEqual(blocks[0].kind, "ol")
        self.assertEqual(blocks[0].items, ["First", "Second"])

    def test_table(self) -> None:
        text = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n"
        blocks = generate.parse_blocks(text)
        self.assertEqual(len(blocks), 1)
        table = blocks[0]
        self.assertIsInstance(table, generate.RawTable)
        self.assertEqual(table.header, ["A", "B"])
        self.assertEqual(table.rows, [["1", "2"], ["3", "4"]])

    def test_code_fence_preserves_body_verbatim(self) -> None:
        text = "```sh\nPOWERVOICE_WEBKIT_DMABUF=1 powervoice-app\n```\n"
        blocks = generate.parse_blocks(text)
        self.assertIsInstance(blocks[0], generate.RawCode)
        self.assertEqual(blocks[0].lang, "sh")
        self.assertEqual(blocks[0].text, "POWERVOICE_WEBKIT_DMABUF=1 powervoice-app")

    def test_duplicate_heading_ids_get_suffixed(self) -> None:
        blocks = generate.parse_blocks("## Export\n\n## Export\n")
        self.assertEqual([b.id for b in blocks], ["export", "export-1"])


class SectionGroupingTests(unittest.TestCase):
    def test_h1_and_intro_become_overview(self) -> None:
        doc = generate.group_sections("user-guide", generate.parse_blocks("# The Guide\n\nIntro text.\n\n## First\n\nBody.\n"))
        self.assertEqual(doc.title, "The Guide")
        self.assertEqual([s.id for s in doc.sections], ["overview", "first"])

    def test_h3_nests_inside_its_h2_section(self) -> None:
        doc = generate.group_sections(
            "user-guide",
            generate.parse_blocks("## Parent\n\nIntro.\n\n### Child\n\nMore.\n\n## Sibling\n\nOther.\n"),
        )
        self.assertEqual([s.id for s in doc.sections], ["parent", "sibling"])
        parent_blocks = doc.sections[0].blocks
        self.assertTrue(any(isinstance(b, generate.RawHeading) and b.id == "child" for b in parent_blocks))


class InlineAndLinkTests(unittest.TestCase):
    def _index(self):
        docs = {
            "user-guide": generate.group_sections(
                "user-guide",
                generate.parse_blocks("# Guide\n\n## Plugins\n\nBody.\n\n### LV2 plugins are unavailable\n\nMore.\n"),
            ),
        }
        return generate.build_anchor_index(docs)

    def test_bold_italic_code_spans(self) -> None:
        index = self._index()
        spans = generate.parse_inline("A **bold** and *italic* and `code` word.", "user-guide", index)
        texts = [(s.get("text"), s.get("bold"), s.get("italic"), s.get("code")) for s in spans]
        self.assertIn(("bold", True, None, None), texts)
        self.assertIn(("italic", None, True, None), texts)
        self.assertIn(("code", None, None, True), texts)

    def test_internal_link_to_h2_resolves(self) -> None:
        index = self._index()
        spans = generate.parse_inline("See [Plugins](user-guide.md#plugins) for more.", "user-guide", index)
        linked = next(s for s in spans if s.get("link"))
        self.assertEqual(linked["link"], {"doc": "user-guide", "section": "plugins"})

    def test_internal_link_to_h3_resolves_to_containing_h2(self) -> None:
        index = self._index()
        spans = generate.parse_inline(
            "See [LV2 plugins are unavailable](#lv2-plugins-are-unavailable) below.", "user-guide", index
        )
        linked = next(s for s in spans if s.get("link"))
        self.assertEqual(linked["link"], {"doc": "user-guide", "section": "plugins"})

    def test_external_and_unresolvable_links_drop_the_href(self) -> None:
        index = self._index()
        spans = generate.parse_inline("[README](../README.md#license) and [gone](user-guide.md#nope).", "user-guide", index)
        self.assertTrue(all("link" not in s for s in spans))
        self.assertIn("README", "".join(s["text"] for s in spans))

    def test_bare_doc_link_with_no_anchor_targets_first_section(self) -> None:
        index = self._index()
        spans = generate.parse_inline("[the guide](user-guide.md)", "faq", index)
        linked = next(s for s in spans if s.get("link"))
        self.assertEqual(linked["link"], {"doc": "user-guide", "section": "plugins"})


class RealDocsIntegrationTests(unittest.TestCase):
    """Runs the real generator against the actual docs/ files — catches the docs drifting into a
    shape this generator can't parse, independent of `--check`'s byte-for-byte diff."""

    def test_build_docs_covers_required_topics(self) -> None:
        docs = generate.build_docs()
        by_id = {d["id"]: d for d in docs}
        self.assertIn("user-guide", by_id)
        self.assertIn("faq", by_id)

        def all_text(doc: dict) -> str:
            chunks: list[str] = []

            def walk_spans(spans):
                for s in spans:
                    chunks.append(s.get("text", ""))

            def walk_block(b: dict) -> None:
                t = b["type"]
                if t in ("p", "h3"):
                    walk_spans(b["spans"])
                elif t in ("ul", "ol"):
                    for item in b["items"]:
                        walk_spans(item)
                elif t == "table":
                    for row in [b["head"], *b["rows"]]:
                        for cell in row:
                            walk_spans(cell)
                elif t == "code":
                    chunks.append(b["text"])

            for s in doc["sections"]:
                chunks.append(s["title"])
                for b in s["blocks"]:
                    walk_block(b)
            return " ".join(chunks)

        text = all_text(by_id["user-guide"]) + " " + all_text(by_id["faq"])
        for must_have in ("CLAP", "VST3", "LV2", "JSFX", "lilv", "Blocklisted", "ACX", "Noise Reduction"):
            self.assertIn(must_have, text, f"generated Help Centre content is missing {must_have!r}")

    def test_no_section_has_a_blank_id(self) -> None:
        for doc in generate.build_docs():
            for section in doc["sections"]:
                self.assertTrue(section["id"], f"{doc['id']} has a section with a blank id")


if __name__ == "__main__":
    unittest.main()
