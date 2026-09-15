import { describe, expect, it } from "vitest";
import { placePopover } from "./placement";

const viewport = { width: 1280, height: 720 };
const menu = { width: 200, height: 300 };

describe("placePopover (viewport clamping)", () => {
  it("opens below the anchor, start-aligned, when there is room", () => {
    const anchor = { left: 100, top: 40, width: 60, height: 28 };
    expect(placePopover(anchor, menu, viewport, "bottom-start")).toEqual({
      left: 100,
      top: 72,
      placement: "bottom-start",
      maxHeight: null,
    });
  });

  it("end-aligns to the anchor's right edge for bottom-end", () => {
    const anchor = { left: 1000, top: 40, width: 100, height: 28 };
    const placed = placePopover(anchor, menu, viewport, "bottom-end");
    expect(placed.left).toBe(900);
  });

  it("flips above when it would leave the bottom of the window", () => {
    const anchor = { left: 100, top: 600, width: 60, height: 28 };
    const placed = placePopover(anchor, menu, viewport, "bottom-start");
    expect(placed.placement).toBe("top-start");
    expect(placed.top).toBe(600 - 4 - 300);
    expect(placed.maxHeight).toBeNull();
  });

  it("shifts left to stay inside the right edge", () => {
    const anchor = { left: 1200, top: 40, width: 60, height: 28 };
    const placed = placePopover(anchor, menu, viewport, "bottom-start");
    expect(placed.left).toBe(1280 - 8 - 200);
  });

  it("never goes past the left edge", () => {
    const anchor = { left: 2, top: 40, width: 20, height: 20 };
    expect(placePopover(anchor, menu, viewport, "bottom-end").left).toBe(8);
  });

  it("takes the roomier side and scrolls when it fits on neither", () => {
    const tall = { width: 200, height: 900 };
    const anchor = { left: 100, top: 500, width: 60, height: 28 };
    const placed = placePopover(anchor, tall, viewport, "bottom-start");
    expect(placed.placement).toBe("top-start");
    expect(placed.maxHeight).toBe(500 - 4 - 8);
    expect(placed.top).toBe(8);
  });

  it("puts a submenu to the right of its row, flipping left near the right edge", () => {
    const row = { left: 300, top: 100, width: 180, height: 24 };
    const right = placePopover(row, menu, viewport, "right-start", { alignOffsetPx: -4 });
    expect(right).toMatchObject({ left: 484, top: 96, placement: "right-start" });

    const nearEdge = { left: 1000, top: 100, width: 180, height: 24 };
    const flipped = placePopover(nearEdge, menu, viewport, "right-start");
    expect(flipped.placement).toBe("left-start");
    expect(flipped.left).toBe(1000 - 4 - 200);
  });

  it("slides a submenu up so its bottom stays inside the window", () => {
    const row = { left: 300, top: 600, width: 180, height: 24 };
    const placed = placePopover(row, menu, viewport, "right-start");
    expect(placed.top).toBe(720 - 8 - 300);
  });
});
