/**
 * H-107: the Help Centre dialog's visibility/navigation state — same trivial module-scoped-state
 * pattern as `about.svelte.ts`/`shortcuts.svelte.ts`, plus the bit of navigation state (current
 * doc/section, search query) a browsable dialog needs that a static one doesn't.
 */
import { registerAction } from "../shortcuts";
import { getSection, HELP_DOCS, resolveHelpTopic } from "./content";

let open = $state(false);
let docId = $state(HELP_DOCS[0]?.id ?? "");
let sectionId = $state(HELP_DOCS[0]?.sections[0]?.id ?? "");
let query = $state("");

export interface HelpCentreState {
  readonly open: boolean;
  readonly docId: string;
  readonly sectionId: string;
  readonly query: string;
}

export function helpCentreState(): HelpCentreState {
  return {
    get open() {
      return open;
    },
    get docId() {
      return docId;
    },
    get sectionId() {
      return sectionId;
    },
    get query() {
      return query;
    },
  };
}

/** Opens the Help Centre, optionally jumping straight to a topic (a panel's "?" —
 * `HelpButton.svelte`, which may name either a section or one of its nested subheadings —
 * `resolveHelpTopic`). Falls back to whatever was already selected if the target doesn't exist (a
 * renamed heading should never make the button do nothing). */
export function openHelpCentre(target?: { doc: string; section: string }): void {
  query = "";
  const topic = target && resolveHelpTopic(target.doc, target.section);
  if (topic) {
    docId = topic.doc.id;
    sectionId = topic.section.id;
  }
  open = true;
}

export function closeHelpCentre(): void {
  open = false;
}

export function selectHelpSection(doc: string, section: string): void {
  if (!getSection(doc, section)) {
    return;
  }
  docId = doc;
  sectionId = section;
}

export function setHelpQuery(next: string): void {
  query = next;
}

/** Wires the F1 keymap action (App.svelte: `onMount(() => initHelpCentre())`). Returns the
 * teardown, same shape as every other feature's `initX`/`registerAction` pair. */
export function initHelpCentre(): () => void {
  return registerAction("help.open_centre", () => openHelpCentre());
}

/** Test/teardown helper. */
export function resetHelpCentreForTest(): void {
  open = false;
  docId = HELP_DOCS[0]?.id ?? "";
  sectionId = HELP_DOCS[0]?.sections[0]?.id ?? "";
  query = "";
}
