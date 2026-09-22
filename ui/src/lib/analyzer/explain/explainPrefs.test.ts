import { afterEach, describe, expect, it } from "vitest";
import { loadShowAnnotationsPref, saveShowAnnotationsPref } from "./explainPrefs";

/** H-115 ticket §2: "Remember the choice" — the Annotations toggle's state survives a reload,
 * the same `localStorage` pattern `theme.svelte.ts` already uses. */
describe("explain annotations preference (H-115)", () => {
  afterEach(() => {
    localStorage.removeItem("powervoice.explain.annotations");
  });

  it("defaults to shown when nothing has been stored yet", () => {
    expect(loadShowAnnotationsPref()).toBe(true);
  });

  it("remembers off across a reload", () => {
    saveShowAnnotationsPref(false);
    expect(loadShowAnnotationsPref()).toBe(false);
  });

  it("remembers on again after being turned back on", () => {
    saveShowAnnotationsPref(false);
    saveShowAnnotationsPref(true);
    expect(loadShowAnnotationsPref()).toBe(true);
  });

  it("falls back to shown when storage throws (private mode)", () => {
    const original = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("blocked");
    };
    try {
      expect(loadShowAnnotationsPref()).toBe(true);
    } finally {
      Storage.prototype.getItem = original;
    }
  });

  it("save never throws when storage throws", () => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("blocked");
    };
    try {
      expect(() => saveShowAnnotationsPref(false)).not.toThrow();
    } finally {
      Storage.prototype.setItem = original;
    }
  });
});
