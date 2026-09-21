import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { closeExplainVoice, openExplainVoice, resetExplainModalForTest } from "./explainModal.svelte";
import { balancedReport, combCurve } from "./voiceFixtures";
import type { VoiceSnapshotInput } from "./snapshot";
import ExplainVoiceModal from "./ExplainVoiceModal.svelte";

/**
 * H-92: the modal shell — title, the real analysed span, the five toggles, the graph, Escape to
 * close, and that closing really does drop the frozen snapshot so live analysis is untouched.
 * `ExplainGraph`'s own drawing is exercised indirectly (jsdom has no canvas, MEMORY.md); what
 * matters here is that the modal wires everything together without throwing and shows the right
 * structural text.
 */

function input(overrides: Partial<VoiceSnapshotInput> = {}): VoiceSnapshotInput {
  const curve = combCurve({ f0Hz: 120, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
  return {
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport(),
    sampleRateHz: 48_000,
    origin: "average",
    ...overrides,
  };
}

function mountModal() {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ExplainVoiceModal, { target });
  flushSync();
  return { target, app };
}

afterEach(() => {
  resetExplainModalForTest();
  document.body.innerHTML = "";
});

describe("ExplainVoiceModal (H-92)", () => {
  it("renders nothing until opened", () => {
    const { target, app } = mountModal();
    expect(target.querySelector('[data-testid="explain-voice-modal"]')).toBeNull();
    unmount(app);
  });

  it("shows the title, the real analysed span, and every toggle on by default", () => {
    openExplainVoice(input({ report: balancedReport({ span_s: 8.4 }) }));
    const { target, app } = mountModal();
    flushSync();

    const dialog = target.querySelector('[data-testid="explain-voice-modal"]');
    expect(dialog).not.toBeNull();
    expect(dialog!.textContent).toContain("Voice Spectrum Analysis");
    expect(target.querySelector('[data-testid="explain-subtitle"]')!.textContent).toContain("8.4");

    for (const id of ["raw", "smoothed", "harmonics", "bands", "eq"]) {
      const toggle = target.querySelector(`[data-testid="explain-toggle-${id}"]`)!;
      expect(toggle.getAttribute("aria-pressed")).toBe("true");
    }
    unmount(app);
  });

  it("Escape closes the modal and drops the frozen snapshot", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const dialog = target.querySelector<HTMLElement>('[data-testid="explain-voice-modal"]')!;
    dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="explain-voice-modal"]')).toBeNull();
    unmount(app);
  });

  it("the Close button does the same", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="explain-close"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="explain-voice-modal"]')).toBeNull();
    unmount(app);
  });

  it("a toggle can be switched off without breaking the graph", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const smoothed = target.querySelector<HTMLButtonElement>('[data-testid="explain-toggle-smoothed"]')!;
    smoothed.click();
    flushSync();
    expect(smoothed.getAttribute("aria-pressed")).toBe("false");
    expect(target.querySelector('[data-testid="explain-graph-root"]')).not.toBeNull();
    unmount(app);
  });

  it("closing externally (not via this component) is reflected on the next render", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    closeExplainVoice();
    flushSync();
    expect(target.querySelector('[data-testid="explain-voice-modal"]')).toBeNull();
    unmount(app);
  });
});
