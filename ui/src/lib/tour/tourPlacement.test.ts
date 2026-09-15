import { describe, expect, it } from "vitest";
import { placeTourCard, spotlightRect } from "./tourPlacement";

const VIEWPORT = { width: 1280, height: 720 };
const CARD = { width: 352, height: 200 };

describe("spotlightRect (T-709)", () => {
  it("grows the target by the padding", () => {
    expect(spotlightRect({ left: 100, top: 50, width: 40, height: 28 }, VIEWPORT, 6)).toEqual({
      left: 94,
      top: 44,
      width: 52,
      height: 40,
    });
  });

  it("clips to the window and is null when nothing is on screen", () => {
    expect(spotlightRect({ left: -20, top: 700, width: 100, height: 100 }, VIEWPORT, 6)).toEqual({
      left: 0,
      top: 694,
      width: 86,
      height: 26,
    });
    expect(spotlightRect({ left: 2000, top: 10, width: 40, height: 40 }, VIEWPORT, 6)).toBeNull();
  });
});

describe("placeTourCard (T-709)", () => {
  it("centres the card under the target by default, with the pointer on the target's centre", () => {
    const spot = { left: 500, top: 40, width: 80, height: 32 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "bottom", { gapPx: 14, edgePx: 12 });
    expect(placed.placement).toBe("bottom");
    expect(placed.top).toBe(40 + 32 + 14);
    expect(placed.left).toBe(540 - 176);
    expect(placed.arrowPx).toBe(176);
    expect(placed.maxHeight).toBeNull();
  });

  it("flips above a target near the bottom of the window", () => {
    const spot = { left: 500, top: 600, width: 80, height: 60 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "bottom", { gapPx: 14, edgePx: 12 });
    expect(placed.placement).toBe("top");
    expect(placed.top).toBe(600 - 14 - 200);
  });

  it("shifts back inside the window near an edge, keeping the pointer on the target", () => {
    const spot = { left: 8, top: 40, width: 30, height: 30 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "bottom", { gapPx: 14, edgePx: 12 });
    expect(placed.left).toBe(12);
    // The target's centre (23 px) is closer to the corner than the inset allows.
    expect(placed.arrowPx).toBe(20);
  });

  it("flips a side placement and centres it vertically on the target", () => {
    const spot = { left: 1100, top: 300, width: 160, height: 100 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "right", { gapPx: 14, edgePx: 12 });
    expect(placed.placement).toBe("left");
    expect(placed.left).toBe(1100 - 14 - 352);
    expect(placed.top).toBe(300 + 50 - 100);
    expect(placed.arrowPx).toBe(100);
  });

  it("falls back to a perpendicular side when neither preferred side fits", () => {
    // A tall column on the left edge: nothing to its left, a card fits to its right.
    const spot = { left: 0, top: 60, width: 240, height: 640 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "bottom", { gapPx: 14, edgePx: 12 });
    expect(placed.placement).toBe("right");
  });

  it("puts the card inside a target that fills the window", () => {
    const spot = { left: 0, top: 0, width: 1280, height: 720 };
    const placed = placeTourCard(spot, CARD, VIEWPORT, "bottom", { gapPx: 14, edgePx: 12 });
    expect(placed.placement).toBe("inside");
    expect(placed.left).toBe(1280 - 12 - 352);
    expect(placed.top).toBe(720 - 12 - 200);
    expect(placed.arrowPx).toBeNull();
  });

  it("centres the card when there is no target, and scrolls a card taller than the window", () => {
    const centred = placeTourCard(null, CARD, VIEWPORT);
    expect(centred).toEqual({ left: 464, top: 260, placement: "center", maxHeight: null, arrowPx: null });
    const tall = placeTourCard(null, { width: 352, height: 900 }, VIEWPORT, "bottom", { edgePx: 12 });
    expect(tall.maxHeight).toBe(696);
    expect(tall.top).toBe(12);
  });
});
