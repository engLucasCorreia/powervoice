import { describe, expect, it } from "vitest";
import { chooseRenderer, rendererKindAfterLoss } from "./rendererMode";

describe("chooseRenderer (H-13, ADR-009 §4)", () => {
  it("uses WebGL2 when available and preference is auto or webgl2", () => {
    expect(chooseRenderer("auto", true)).toBe("webgl2");
    expect(chooseRenderer("webgl2", true)).toBe("webgl2");
  });

  it("falls back to Canvas2D when WebGL2 is unavailable, regardless of preference", () => {
    expect(chooseRenderer("auto", false)).toBe("canvas2d");
    expect(chooseRenderer("webgl2", false)).toBe("canvas2d");
  });

  it("an explicit canvas2d preference always wins, even when WebGL2 is available", () => {
    expect(chooseRenderer("canvas2d", true)).toBe("canvas2d");
    expect(chooseRenderer("canvas2d", false)).toBe("canvas2d");
  });
});

describe("rendererKindAfterLoss (ADR-009 §4: latch to Canvas2D forever after context loss)", () => {
  it("is unaffected before loss", () => {
    expect(rendererKindAfterLoss("auto", true, false)).toBe("webgl2");
  });

  it("latches to canvas2d once lost, even if webgl2Available still reports true", () => {
    expect(rendererKindAfterLoss("auto", true, true)).toBe("canvas2d");
    expect(rendererKindAfterLoss("webgl2", true, true)).toBe("canvas2d");
  });
});
