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

/** H-115 ticket §1: "the screen is very small — it's OK to make it very big — and a maximise
 * control." */
describe("ExplainVoiceModal maximise (H-115)", () => {
  it("starts at the default (non-maximised) size and toggles to nearly full-viewport on click", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const dialog = target.querySelector<HTMLElement>('[data-testid="explain-voice-modal"]')!;
    expect(dialog.getAttribute("style")).toContain("max(880px, 88vw)");

    target.querySelector<HTMLButtonElement>('[data-testid="explain-maximize"]')!.click();
    flushSync();
    expect(dialog.getAttribute("style")).toContain("98vw");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    unmount(app);
  });
});

/** H-115 ticket §2: "I did not find something to toggle the comments in the picture." */
describe("ExplainVoiceModal Annotations toggle (H-115)", () => {
  afterEach(() => {
    localStorage.removeItem("powervoice.explain.annotations");
  });

  it("is on by default, alongside the other five toggles", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const toggle = target.querySelector('[data-testid="explain-toggle-annotations"]')!;
    expect(toggle.textContent).toContain("Annotations");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    unmount(app);
  });

  it("can be turned off without breaking the graph, and the choice is remembered", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const toggle = target.querySelector<HTMLButtonElement>('[data-testid="explain-toggle-annotations"]')!;
    toggle.click();
    flushSync();
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(target.querySelector('[data-testid="explain-graph-root"]')).not.toBeNull();
    expect(localStorage.getItem("powervoice.explain.annotations")).toBe("0");
    unmount(app);
  });
});

/** H-115 ticket §3: "The EQ advice I toggle on and off but I don't find where it is" — the toggle
 * must never look like a no-op. */
describe("ExplainVoiceModal EQ Advice empty state (H-115)", () => {
  it("says so, with the reason available, when the toggle is on but there is nothing to draw", () => {
    // `balancedReport()` (the default fixture): "every measurement sits in the healthy zone" —
    // no finding crosses a threshold, so `summary.eqBands` is empty.
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const note = target.querySelector('[data-testid="explain-eq-advice-empty"]');
    expect(note).not.toBeNull();
    expect(note!.textContent).toContain("No EQ change suggested for this take");
    expect(note!.getAttribute("title")).toContain("No action suggested");
    unmount(app);
  });

  it("disappears when the EQ Advice toggle itself is off", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="explain-toggle-eq"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="explain-eq-advice-empty"]')).toBeNull();
    unmount(app);
  });
});

/** H-115 ticket §4: "a button to export that screen". The rendered PNG itself needs a real 2D
 * canvas context jsdom doesn't have (MEMORY.md); `explainExport*.test.ts` cover that logic. What
 * is verified here is that the control only offers to export once there is something to export. */
describe("ExplainVoiceModal export menu (H-115)", () => {
  it("is disabled until the graph has produced an export frame (jsdom never sizes the canvas)", () => {
    openExplainVoice(input());
    const { target, app } = mountModal();
    flushSync();
    const trigger = target.querySelector<HTMLButtonElement>('[data-testid="explain-export-menu-trigger"]')!;
    expect(trigger.disabled).toBe(true);
    unmount(app);
  });
});
