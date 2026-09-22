import { describe, expect, it } from "vitest";
import { explainDialogSize, explainDialogStyle } from "./explainDialogSize";

/**
 * H-115 ticket §1: the modal must size itself from the viewport, not a pixel cap — the owner's
 * complaint was a fixed-880px-wide box on a 3756×2121 display. `max(floor, vw/vh)` never gets
 * capped back down by a pixel ceiling; only the floor keeps a laptop usable.
 */
describe("explainDialogSize (H-115)", () => {
  it("the default size is a CSS max() of a pixel floor and a viewport share, never a bare pixel value", () => {
    const size = explainDialogSize(false);
    expect(size.width).toBe("max(880px, 88vw)");
    expect(size.height).toBe("max(600px, 86vh)");
  });

  it("maximised goes to nearly the full viewport, with no pixel cap at all", () => {
    const size = explainDialogSize(true);
    expect(size.width).toBe("98vw");
    expect(size.height).toBe("96vh");
    expect(size.width).not.toMatch(/px/);
    expect(size.height).not.toMatch(/px/);
  });
});

describe("explainDialogStyle (H-115)", () => {
  it("also overrides max-width/max-height, so Dialog's own 90vw/85vh base cap can't clip it back down", () => {
    const style = explainDialogStyle(true);
    expect(style).toContain("width: 98vw;");
    expect(style).toContain("max-width: 98vw;");
    expect(style).toContain("height: 96vh;");
    expect(style).toContain("max-height: 96vh;");
  });

  it("the default (non-maximised) style carries the same max() through both width and max-width", () => {
    const style = explainDialogStyle(false);
    expect(style).toContain("width: max(880px, 88vw); max-width: max(880px, 88vw);");
  });
});
