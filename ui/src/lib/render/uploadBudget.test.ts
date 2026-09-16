import { describe, expect, it } from "vitest";
import {
  DEFAULT_UPLOAD_BUDGET,
  planUploads,
  prioritizeUploads,
  type UploadCandidate,
} from "./uploadBudget";

const tile = (over: Partial<UploadCandidate> & { key: string }): UploadCandidate => ({
  bytes: 256 * 1025,
  x0: 0,
  x1: 100,
  seq: 0,
  ...over,
});

describe("prioritizeUploads", () => {
  it("puts tiles overlapping the viewport before off-screen margin tiles", () => {
    const order = prioritizeUploads(
      [
        tile({ key: "left-margin", x0: -300, x1: -100, seq: 9 }),
        tile({ key: "visible", x0: 10, x1: 90, seq: 1 }),
        tile({ key: "right-margin", x0: 200, x1: 300, seq: 8 }),
      ],
      100,
    ).map((c) => c.key);
    expect(order[0]).toBe("visible");
    expect(order.slice(1).sort()).toEqual(["left-margin", "right-margin"]);
  });

  it("orders visible tiles newest first", () => {
    const order = prioritizeUploads(
      [tile({ key: "old", x0: 0, x1: 50, seq: 1 }), tile({ key: "new", x0: 50, x1: 100, seq: 7 })],
      100,
    ).map((c) => c.key);
    expect(order).toEqual(["new", "old"]);
  });

  it("breaks a tie between equally new tiles by distance to the viewport centre", () => {
    const order = prioritizeUploads(
      [
        tile({ key: "edge", x0: 0, x1: 20, seq: 3 }),
        tile({ key: "centre", x0: 40, x1: 60, seq: 3 }),
      ],
      100,
    ).map((c) => c.key);
    expect(order).toEqual(["centre", "edge"]);
  });

  it("orders off-screen tiles by how far outside the viewport they are, not by age", () => {
    const order = prioritizeUploads(
      [
        tile({ key: "far", x0: 500, x1: 600, seq: 9 }),
        tile({ key: "near", x0: 105, x1: 205, seq: 1 }),
      ],
      100,
    ).map((c) => c.key);
    expect(order).toEqual(["near", "far"]);
  });

  it("is a pure function of its input (no mutation, stable for equal input)", () => {
    const input = [tile({ key: "a", seq: 2 }), tile({ key: "b", seq: 1 })];
    const snapshot = JSON.stringify(input);
    prioritizeUploads(input, 100);
    expect(JSON.stringify(input)).toBe(snapshot);
  });

  it("returns an empty plan for no candidates", () => {
    expect(prioritizeUploads([], 100)).toEqual([]);
  });
});

describe("planUploads", () => {
  const budget = { maxTiles: 3, maxBytes: 1_000_000 };

  it("uploads at most maxTiles per frame and defers the rest in priority order", () => {
    const candidates = [
      tile({ key: "a", bytes: 10, x0: 0, x1: 10, seq: 1 }),
      tile({ key: "b", bytes: 10, x0: 10, x1: 20, seq: 2 }),
      tile({ key: "c", bytes: 10, x0: 20, x1: 30, seq: 3 }),
      tile({ key: "d", bytes: 10, x0: 30, x1: 40, seq: 4 }),
      tile({ key: "e", bytes: 10, x0: 40, x1: 50, seq: 5 }),
    ];
    const plan = planUploads(candidates, budget, 100);
    expect(plan.upload.map((c) => c.key)).toEqual(["e", "d", "c"]);
    expect(plan.deferred.map((c) => c.key)).toEqual(["b", "a"]);
    expect(plan.deferredBytes).toBe(20);
  });

  it("stops at maxBytes even below maxTiles", () => {
    const candidates = [
      tile({ key: "a", bytes: 400_000, seq: 1 }),
      tile({ key: "b", bytes: 400_000, seq: 2 }),
      tile({ key: "c", bytes: 400_000, seq: 3 }),
    ];
    const plan = planUploads(candidates, budget, 100);
    expect(plan.upload.map((c) => c.key)).toEqual(["c", "b"]);
    expect(plan.deferred.map((c) => c.key)).toEqual(["a"]);
  });

  it("always uploads the first candidate, so a tile larger than the whole budget still lands", () => {
    const plan = planUploads([tile({ key: "huge", bytes: 50_000_000 })], budget, 100);
    expect(plan.upload.map((c) => c.key)).toEqual(["huge"]);
    expect(plan.deferred).toEqual([]);
  });

  it("reports nothing pending when everything fits", () => {
    const plan = planUploads([tile({ key: "a", bytes: 10 })], budget, 100);
    expect(plan.deferred).toEqual([]);
    expect(plan.deferredBytes).toBe(0);
  });

  it("handles no candidates", () => {
    const plan = planUploads([], budget, 100);
    expect(plan).toEqual({ upload: [], deferred: [], deferredBytes: 0 });
  });

  it("ships a default budget that is positive and finite", () => {
    expect(DEFAULT_UPLOAD_BUDGET.maxTiles).toBeGreaterThan(0);
    expect(DEFAULT_UPLOAD_BUDGET.maxBytes).toBeGreaterThan(0);
    expect(Number.isFinite(DEFAULT_UPLOAD_BUDGET.maxBytes)).toBe(true);
  });

  it("spreads a backlog over successive frames until it is empty", () => {
    let pending = [
      tile({ key: "a", bytes: 10, seq: 1 }),
      tile({ key: "b", bytes: 10, seq: 2 }),
      tile({ key: "c", bytes: 10, seq: 3 }),
      tile({ key: "d", bytes: 10, seq: 4 }),
      tile({ key: "e", bytes: 10, seq: 5 }),
      tile({ key: "f", bytes: 10, seq: 6 }),
      tile({ key: "g", bytes: 10, seq: 7 }),
    ];
    const uploaded: string[] = [];
    let frames = 0;
    while (pending.length > 0) {
      const plan = planUploads(pending, budget, 100);
      uploaded.push(...plan.upload.map((c) => c.key));
      pending = plan.deferred;
      frames += 1;
      expect(frames).toBeLessThan(10);
    }
    expect(uploaded.sort()).toEqual(["a", "b", "c", "d", "e", "f", "g"]);
    expect(frames).toBe(3);
  });
});
