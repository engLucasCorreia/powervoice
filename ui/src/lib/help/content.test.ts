import { describe, expect, it } from "vitest";
import { getSection, HELP_DOCS, resolveHelpTopic, searchHelp, sectionBodyText } from "./content";

describe("Help Centre content (H-107)", () => {
  it("loads the generated docs with the expected ids", () => {
    expect(HELP_DOCS.map((d) => d.id)).toEqual(["user-guide", "faq"]);
    for (const doc of HELP_DOCS) {
      expect(doc.sections.length).toBeGreaterThan(0);
      expect(doc.title.length).toBeGreaterThan(0);
    }
  });

  it("getSection finds a real section and returns undefined for a missing one", () => {
    const found = getSection("user-guide", "plugins");
    expect(found?.section.title).toBe("Plugins");
    expect(getSection("user-guide", "does-not-exist")).toBeUndefined();
    expect(getSection("no-such-doc", "plugins")).toBeUndefined();
  });

  it("every internal link resolves to a real section (no dangling cross-reference)", () => {
    for (const doc of HELP_DOCS) {
      for (const section of doc.sections) {
        const walk = (spans: { link?: { doc: string; section: string } }[]) => {
          for (const span of spans) {
            if (span.link) {
              expect(getSection(span.link.doc, span.link.section), `broken link from ${doc.id}/${section.id}`).toBeDefined();
            }
          }
        };
        for (const block of section.blocks) {
          if (block.type === "p" || block.type === "h3") {
            walk(block.spans);
          } else if (block.type === "ul" || block.type === "ol") {
            block.items.forEach(walk);
          } else if (block.type === "table") {
            [block.head, ...block.rows].forEach((row) => row.forEach(walk));
          }
        }
      }
    }
  });

  it("finds a section by a term that only appears in its body text, not its title", () => {
    // "lilv" never appears in a heading (checked against docs/user-guide.md) — only in the LV2
    // troubleshooting section's body — so this is the acceptance check's own "term only in body".
    const hits = searchHelp("lilv");
    expect(hits.length).toBeGreaterThan(0);
    expect(hits.some((h) => h.docId === "user-guide" && h.sectionTitle.toLowerCase().includes("lilv"))).toBe(false);
    expect(hits.every((h) => !h.titleMatch)).toBe(true);
    expect(hits[0]!.snippet.toLowerCase()).toContain("lilv");
  });

  it("ranks a title match above a body-only match", () => {
    const hits = searchHelp("plugins");
    const firstBodyOnlyIndex = hits.findIndex((h) => !h.titleMatch);
    const lastTitleIndex = hits.map((h) => h.titleMatch).lastIndexOf(true);
    if (firstBodyOnlyIndex !== -1 && lastTitleIndex !== -1) {
      expect(lastTitleIndex).toBeLessThan(firstBodyOnlyIndex);
    }
  });

  it("is case-insensitive and ignores a blank query", () => {
    expect(searchHelp("LILV").length).toBe(searchHelp("lilv").length);
    expect(searchHelp("   ")).toEqual([]);
    expect(searchHelp("")).toEqual([]);
  });

  it("finds no results for nonsense", () => {
    expect(searchHelp("qxzzptlkjw-not-a-real-word")).toEqual([]);
  });

  it("resolveHelpTopic finds a top-level section by its own id", () => {
    const topic = resolveHelpTopic("user-guide", "plugins");
    expect(topic?.section.id).toBe("plugins");
    expect(topic?.title).toBe("Plugins");
  });

  it("resolveHelpTopic finds a nested subheading and returns its containing section", () => {
    const topic = resolveHelpTopic("user-guide", "remove-background-noise");
    expect(topic?.section.id).toBe("cleaning-up-the-effects-rack");
    expect(topic?.title).toBe("Remove background noise");
  });

  it("resolveHelpTopic returns undefined for an unknown doc or anchor", () => {
    expect(resolveHelpTopic("no-such-doc", "plugins")).toBeUndefined();
    expect(resolveHelpTopic("user-guide", "no-such-anchor")).toBeUndefined();
  });

  it("caches section body text (repeated calls return the same string)", () => {
    const { doc, section } = getSection("user-guide", "plugins")!;
    expect(sectionBodyText(doc.id, section)).toBe(sectionBodyText(doc.id, section));
    expect(sectionBodyText(doc.id, section).length).toBeGreaterThan(0);
  });
});
