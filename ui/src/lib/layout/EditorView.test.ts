import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearActionHandlers } from "../keymap";
import { resetSpectralForTest, spectralState } from "../state/spectral.svelte";
import EditorView from "./EditorView.svelte";

beforeEach(() => {
  mockIPC(() => null);
});

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetSpectralForTest();
});

describe("EditorView split layout (T-207, SPEC-007 §2.1)", () => {
  it("shows only the waveform pane by default (spectral pane hidden)", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    flushSync();

    expect(target.querySelector('[data-testid="editor-waveform-pane"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="editor-divider"]')).toBeNull();
    expect(target.querySelector('[data-testid="editor-spectral-pane"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("shows the divider and spectral pane once toggled visible", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(EditorView, { target });
    flushSync();

    spectralState().toggle();
    flushSync();

    expect(target.querySelector('[data-testid="editor-divider"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="editor-spectral-pane"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("defaults the split to 50/50", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    const app = mount(EditorView, { target });
    flushSync();

    const wave = target.querySelector<HTMLElement>('[data-testid="editor-waveform-pane"]')!;
    const spec = target.querySelector<HTMLElement>('[data-testid="editor-spectral-pane"]')!;
    expect(wave.style.flex).toBe("50 1 0%");
    expect(spec.style.flex).toBe("50 1 0%");

    unmount(app);
    target.remove();
  });

  it("dragging the divider changes the split ratio, clamped to [0, 100]", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    const app = mount(EditorView, { target });
    flushSync();

    const container = target.querySelector<HTMLElement>('[data-testid="editor"]')!;
    // jsdom's getBoundingClientRect is all zeros by default; stub a 200px-tall editor.
    container.getBoundingClientRect = () =>
      ({ top: 0, height: 200, left: 0, width: 100, right: 100, bottom: 200, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;

    const divider = target.querySelector<HTMLElement>('[data-testid="editor-divider"]')!;
    divider.setPointerCapture = () => {};
    divider.releasePointerCapture = () => {};
    divider.dispatchEvent(new PointerEvent("pointerdown", { clientY: 100, bubbles: true }));
    divider.dispatchEvent(new PointerEvent("pointermove", { clientY: 150, bubbles: true }));
    flushSync();

    expect(spectralState().splitRatio).toBeCloseTo(75, 6);

    // Dragging past the bottom clamps to 100 (waveform-only collapses the spectral strip).
    divider.dispatchEvent(new PointerEvent("pointermove", { clientY: 1000, bubbles: true }));
    flushSync();
    expect(spectralState().splitRatio).toBe(100);

    divider.dispatchEvent(new PointerEvent("pointerup", { clientY: 1000, bubbles: true }));

    unmount(app);
    target.remove();
  });

  it("double-clicking the divider resets the split to 50%", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    spectralState().setVisible(true);
    spectralState().setSplitRatio(20);
    const app = mount(EditorView, { target });
    flushSync();

    const divider = target.querySelector<HTMLElement>('[data-testid="editor-divider"]')!;
    divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    flushSync();

    expect(spectralState().splitRatio).toBe(50);

    unmount(app);
    target.remove();
  });
});
