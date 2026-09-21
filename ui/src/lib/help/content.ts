/**
 * H-107: types for the Help Centre's generated content, plus the (hand-written) helpers that work
 * on it — lookup and full-text search. The data itself is generated (never edit
 * `content.generated.ts` by hand — see `scripts/help/generate.py`'s module docstring for why the
 * Help Centre is generated from `docs/user-guide.md`/`docs/faq.md` rather than a hand-maintained
 * copy). Content strings are plain English straight from the docs — like the About dialog's
 * `thirdPartyNotices.generated.txt`, generated bulk content doesn't go through an i18n key (there's
 * nothing to translate it *from*, since the docs themselves are English-only); every piece of UI
 * chrome around it (the dialog title, the search box, "No results", the nav) does.
 */
import { HELP_DOCS } from "./content.generated";

export interface HelpLink {
  doc: string;
  section: string;
}

export interface HelpSpan {
  text: string;
  bold?: true;
  italic?: true;
  code?: true;
  link?: HelpLink;
}

export interface HelpParagraphBlock {
  type: "p";
  spans: HelpSpan[];
}

export interface HelpSubheadingBlock {
  type: "h3";
  id: string;
  spans: HelpSpan[];
}

export interface HelpListBlock {
  type: "ul" | "ol";
  items: HelpSpan[][];
}

export interface HelpCodeBlock {
  type: "code";
  lang: string;
  text: string;
}

export interface HelpTableBlock {
  type: "table";
  head: HelpSpan[][];
  rows: HelpSpan[][][];
}

export type HelpBlock = HelpParagraphBlock | HelpSubheadingBlock | HelpListBlock | HelpCodeBlock | HelpTableBlock;

export interface HelpSection {
  id: string;
  title: string;
  blocks: HelpBlock[];
}

export interface HelpDoc {
  id: string;
  title: string;
  sections: HelpSection[];
}

export { HELP_DOCS };

export function getDoc(docId: string): HelpDoc | undefined {
  return HELP_DOCS.find((d) => d.id === docId);
}

/** An exact top-level section lookup — what the nav (every id it offers really is a section id)
 * and `selectHelpSection` use. */
export function getSection(docId: string, sectionId: string): { doc: HelpDoc; section: HelpSection } | undefined {
  const doc = getDoc(docId);
  const section = doc?.sections.find((s) => s.id === sectionId);
  return doc && section ? { doc, section } : undefined;
}

function spansText(spans: HelpSpan[]): string {
  return spans.map((s) => s.text).join("");
}

export interface HelpTopic {
  doc: HelpDoc;
  section: HelpSection;
  /** The matched heading's own text — the section's title for a section-level anchor, or the
   * more specific nested `h3`'s text when `anchorId` names a subheading. */
  title: string;
}

/** Resolves a doc + anchor id to the page it lives on — the anchor id is either a section (H2) id
 * itself, or one of that doc's nested `h3` subheading ids (e.g. a panel's "?" pointing at "Remove
 * background noise," which lives inside the "Cleaning up: the effects rack" section). This is how
 * `HelpButton.svelte` and `openHelpCentre`'s target can name the same fine-grained anchors the
 * docs themselves link to, while the Help Centre still only ever navigates to a whole page. */
export function resolveHelpTopic(docId: string, anchorId: string): HelpTopic | undefined {
  const doc = getDoc(docId);
  if (!doc) {
    return undefined;
  }
  for (const section of doc.sections) {
    if (section.id === anchorId) {
      return { doc, section, title: section.title };
    }
  }
  for (const section of doc.sections) {
    for (const block of section.blocks) {
      if (block.type === "h3" && block.id === anchorId) {
        return { doc, section, title: spansText(block.spans) };
      }
    }
  }
  return undefined;
}

function blockText(block: HelpBlock): string {
  switch (block.type) {
    case "p":
    case "h3":
      return spansText(block.spans);
    case "ul":
    case "ol":
      return block.items.map(spansText).join(" ");
    case "code":
      return block.text;
    case "table":
      return [block.head, ...block.rows].map((row) => row.map(spansText).join(" ")).join(" ");
  }
}

const bodyTextCache = new Map<string, string>();

/** The section's body text, flattened for search (title not included — callers check that
 * separately so a title match can be ranked above a body-only match). Cached: the Help Centre
 * re-runs search on every keystroke. */
export function sectionBodyText(docId: string, section: HelpSection): string {
  const key = `${docId}:${section.id}`;
  let cached = bodyTextCache.get(key);
  if (cached === undefined) {
    cached = section.blocks.map(blockText).join(" ");
    bodyTextCache.set(key, cached);
  }
  return cached;
}

export interface HelpSearchHit {
  docId: string;
  docTitle: string;
  sectionId: string;
  sectionTitle: string;
  snippet: string;
  titleMatch: boolean;
}

const SNIPPET_RADIUS = 60;

function snippetAround(body: string, query: string): string {
  const lower = body.toLowerCase();
  const at = lower.indexOf(query.toLowerCase());
  if (at === -1) {
    return body.slice(0, SNIPPET_RADIUS * 2).trim();
  }
  const start = Math.max(0, at - SNIPPET_RADIUS);
  const end = Math.min(body.length, at + query.length + SNIPPET_RADIUS);
  const prefix = start > 0 ? "…" : "";
  const suffix = end < body.length ? "…" : "";
  return `${prefix}${body.slice(start, end).trim()}${suffix}`;
}

/** Full-text search across every Help Centre section: title and body. Title matches sort first;
 * ties keep doc/section order (stable). Empty/whitespace-only queries return no hits — the Help
 * Centre shows its normal nav in that case, not an empty results list. */
export function searchHelp(query: string): HelpSearchHit[] {
  const q = query.trim().toLowerCase();
  if (!q) {
    return [];
  }
  const hits: HelpSearchHit[] = [];
  for (const doc of HELP_DOCS) {
    for (const section of doc.sections) {
      const titleMatch = section.title.toLowerCase().includes(q);
      const body = sectionBodyText(doc.id, section);
      const bodyMatch = body.toLowerCase().includes(q);
      if (!titleMatch && !bodyMatch) {
        continue;
      }
      hits.push({
        docId: doc.id,
        docTitle: doc.title,
        sectionId: section.id,
        sectionTitle: section.title,
        snippet: titleMatch && !bodyMatch ? body.slice(0, SNIPPET_RADIUS * 2).trim() : snippetAround(body, q),
        titleMatch,
      });
    }
  }
  hits.sort((a, b) => Number(b.titleMatch) - Number(a.titleMatch));
  return hits;
}
