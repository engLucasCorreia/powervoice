import { afterEach, describe, expect, it } from "vitest";
import { clearActionHandlers, dispatchAction } from "../shortcuts";
import { HELP_DOCS } from "./content";
import {
  closeHelpCentre,
  helpCentreState,
  initHelpCentre,
  openHelpCentre,
  resetHelpCentreForTest,
  selectHelpSection,
  setHelpQuery,
} from "./helpCentre.svelte";

afterEach(() => {
  resetHelpCentreForTest();
  clearActionHandlers();
});

describe("helpCentre.svelte (H-107)", () => {
  it("starts closed, on the first doc's first section", () => {
    const s = helpCentreState();
    expect(s.open).toBe(false);
    expect(s.docId).toBe(HELP_DOCS[0]!.id);
    expect(s.sectionId).toBe(HELP_DOCS[0]!.sections[0]!.id);
  });

  it("openHelpCentre() opens without changing the current topic when no target is given", () => {
    selectHelpSection("faq", "plugins");
    openHelpCentre();
    const s = helpCentreState();
    expect(s.open).toBe(true);
    expect(s.docId).toBe("faq");
    expect(s.sectionId).toBe("plugins");
  });

  it("openHelpCentre({doc, section}) jumps to that topic and clears any leftover search query", () => {
    setHelpQuery("lilv");
    openHelpCentre({ doc: "user-guide", section: "plugins" });
    const s = helpCentreState();
    expect(s.open).toBe(true);
    expect(s.docId).toBe("user-guide");
    expect(s.sectionId).toBe("plugins");
    expect(s.query).toBe("");
  });

  it("openHelpCentre with an unknown target falls back to whatever was already selected", () => {
    selectHelpSection("faq", "plugins");
    openHelpCentre({ doc: "no-such-doc", section: "nope" });
    const s = helpCentreState();
    expect(s.open).toBe(true);
    expect(s.docId).toBe("faq");
    expect(s.sectionId).toBe("plugins");
  });

  it("closeHelpCentre() closes it", () => {
    openHelpCentre();
    closeHelpCentre();
    expect(helpCentreState().open).toBe(false);
  });

  it("selectHelpSection ignores an unknown doc/section", () => {
    selectHelpSection("user-guide", "plugins");
    selectHelpSection("nope", "nope");
    expect(helpCentreState().docId).toBe("user-guide");
    expect(helpCentreState().sectionId).toBe("plugins");
  });

  it("setHelpQuery updates the query", () => {
    setHelpQuery("noise print");
    expect(helpCentreState().query).toBe("noise print");
  });

  it("initHelpCentre wires F1 to open the centre, and its teardown unregisters it", () => {
    const teardown = initHelpCentre();
    dispatchAction("help.open_centre");
    expect(helpCentreState().open).toBe(true);

    resetHelpCentreForTest();
    teardown();
    dispatchAction("help.open_centre");
    expect(helpCentreState().open).toBe(false);
  });
});
