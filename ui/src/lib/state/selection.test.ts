import { afterEach, describe, expect, it } from "vitest";
import {
  beginDrag,
  beginHandleDrag,
  dragTo,
  endDrag,
  endHandleDrag,
  handleDragTo,
  isHandleDragging,
  isSelectionLocked,
  resetSelectionForTest,
  selectionState,
  setSelectionFromResult,
  setSelectionLocked,
} from "./selection.svelte";

afterEach(() => {
  resetSelectionForTest();
});

describe("handle drag (SPEC-006 §2.9, grab handles)", () => {
  it("live-updates the moving edge while the other stays fixed", () => {
    setSelectionFromResult([100, 200]);
    beginHandleDrag(200); // dragging the start handle; the end (200) is fixed
    expect(isHandleDragging()).toBe(true);
    handleDragTo(150);
    expect(selectionState().current).toEqual({ startSample: 150, endSample: 200 });
    handleDragTo(120);
    expect(selectionState().current).toEqual({ startSample: 120, endSample: 200 });
  });

  it("dragging a handle past the opposite handle swaps which edge is start, never start > end", () => {
    setSelectionFromResult([100, 200]);
    beginHandleDrag(100); // dragging the end handle; the start (100) is fixed
    handleDragTo(50); // dragged past the fixed start
    const sel = selectionState().current;
    expect(sel).toEqual({ startSample: 50, endSample: 100 });
    expect(sel!.startSample).toBeLessThan(sel!.endSample);
  });

  it("endHandleDrag returns the fixed edge it was anchored to, and clears the drag", () => {
    setSelectionFromResult([100, 200]);
    beginHandleDrag(200);
    handleDragTo(150);
    const fixed = endHandleDrag();
    expect(fixed).toBe(200);
    expect(isHandleDragging()).toBe(false);
    expect(endHandleDrag()).toBeNull();
  });

  it("beginHandleDrag clears any in-progress plain drag's anchor, so a stray dragTo is a no-op", () => {
    beginDrag(50);
    beginHandleDrag(200);
    expect(isHandleDragging()).toBe(true);
    dragTo(60); // dragTo requires dragAnchorSample, cleared by beginHandleDrag -> no-op
    handleDragTo(150);
    expect(selectionState().current).toEqual({ startSample: 150, endSample: 200 });
    endHandleDrag();
  });

  it("T-304: selection gestures (including handle drags) are ignored while locked", () => {
    setSelectionFromResult([100, 200]);
    setSelectionLocked(true);
    expect(isSelectionLocked()).toBe(true);
    beginHandleDrag(200);
    expect(isHandleDragging()).toBe(false);
    handleDragTo(150);
    expect(selectionState().current).toEqual({ startSample: 100, endSample: 200 });
    setSelectionLocked(false);
  });
});
