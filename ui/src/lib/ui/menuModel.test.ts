import { describe, expect, it } from "vitest";
import { isTypeaheadKey, moveMenuFocus, typeaheadIndex } from "./menuModel";

describe("moveMenuFocus", () => {
  const disabled = [false, true, false, false];

  it("moves down and up, skipping disabled items and wrapping", () => {
    expect(moveMenuFocus(0, "ArrowDown", disabled)).toBe(2);
    expect(moveMenuFocus(3, "ArrowDown", disabled)).toBe(0);
    expect(moveMenuFocus(2, "ArrowUp", disabled)).toBe(0);
    expect(moveMenuFocus(0, "ArrowUp", disabled)).toBe(3);
  });

  it("starts at the first (down) or last (up) item when nothing is focused", () => {
    expect(moveMenuFocus(-1, "ArrowDown", disabled)).toBe(0);
    expect(moveMenuFocus(-1, "ArrowUp", disabled)).toBe(3);
  });

  it("Home/End jump to the first/last enabled item", () => {
    expect(moveMenuFocus(2, "Home", [true, false, false])).toBe(1);
    expect(moveMenuFocus(0, "End", [false, false, true])).toBe(1);
  });

  it("returns null for other keys and for an all-disabled menu", () => {
    expect(moveMenuFocus(0, "a", disabled)).toBeNull();
    expect(moveMenuFocus(0, "ArrowDown", [true, true])).toBeNull();
    expect(moveMenuFocus(0, "ArrowDown", [])).toBeNull();
  });
});

describe("typeaheadIndex", () => {
  const labels = ["Save", "Save As…", "Export…", "Settings", "Close"];
  const none = labels.map(() => false);

  it("finds the next item starting with the typed letter, after the current one", () => {
    expect(typeaheadIndex(labels, none, "e", 0)).toBe(2);
    expect(typeaheadIndex(labels, none, "c", 4)).toBe(4);
  });

  it("repeating a letter cycles through matches", () => {
    expect(typeaheadIndex(labels, none, "s", 0)).toBe(1);
    expect(typeaheadIndex(labels, none, "ss", 1)).toBe(3);
    expect(typeaheadIndex(labels, none, "sss", 3)).toBe(0);
  });

  it("a longer query narrows the match and may stay on the current item", () => {
    expect(typeaheadIndex(labels, none, "save a", 0)).toBe(1);
    expect(typeaheadIndex(labels, none, "se", 3)).toBe(3);
  });

  it("skips disabled items and returns null without a match", () => {
    expect(typeaheadIndex(labels, [false, true, false, false, false], "s", 0)).toBe(3);
    expect(typeaheadIndex(labels, none, "z", 0)).toBeNull();
  });

  it("only printable keys without modifiers feed typeahead", () => {
    expect(isTypeaheadKey({ key: "a", ctrlKey: false, metaKey: false, altKey: false })).toBe(true);
    expect(isTypeaheadKey({ key: "a", ctrlKey: true, metaKey: false, altKey: false })).toBe(false);
    expect(isTypeaheadKey({ key: "ArrowDown", ctrlKey: false, metaKey: false, altKey: false })).toBe(false);
    expect(isTypeaheadKey({ key: " ", ctrlKey: false, metaKey: false, altKey: false })).toBe(false);
  });
});
