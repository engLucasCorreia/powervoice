import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { t } from "../i18n";
import type { Settings } from "../ipc/bindings";
import { applyRecordStateForTest, recordState, resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { settingsFixture } from "../test/fixtures";
import { resetTourForTest, startTour, tourState } from "./tour.svelte";
import type { TourDef } from "./tours";
import TourOverlay from "./TourOverlay.svelte";

type Rect = { left: number; top: number; width: number; height: number };
type Anchor = HTMLElement & { rect: Rect };

const anchors: HTMLElement[] = [];

/** A `data-tour` anchor with a controllable layout box (jsdom has no layout). */
function anchor(name: string, rect: Rect, parent: HTMLElement = document.body): Anchor {
  const el = document.createElement("button") as unknown as Anchor;
  el.dataset.tour = name;
  el.textContent = name;
  el.rect = rect;
  el.getBoundingClientRect = () =>
    ({
      ...el.rect,
      x: el.rect.left,
      y: el.rect.top,
      right: el.rect.left + el.rect.width,
      bottom: el.rect.top + el.rect.height,
      toJSON: () => ({}),
    }) as DOMRect;
  parent.appendChild(el);
  anchors.push(el);
  return el;
}

const TEST_TOUR: TourDef = {
  id: "test",
  version: 3,
  nameKey: "tour.name.rack",
  steps: [
    { id: "one", target: ["alpha"], titleKey: "tour.rack.panel.title", bodyKey: "tour.rack.panel.body" },
    { id: "two", target: ["missing-anchor"], titleKey: "tour.rack.add.title", bodyKey: "tour.rack.add.body" },
    { id: "three", target: ["beta"], titleKey: "tour.rack.slots.title", bodyKey: "tour.rack.slots.body" },
  ],
};

let app: ReturnType<typeof mount> | null = null;
let host: HTMLElement | null = null;
let saved: Settings[] = [];

function mountOverlay(): void {
  host = document.createElement("div");
  document.body.appendChild(host);
  app = mount(TourOverlay, { target: host });
  flushSync();
}

const q = <T extends HTMLElement = HTMLElement>(id: string): T | null =>
  document.querySelector<T>(`[data-testid="${id}"]`);

function card(): HTMLElement {
  const el = q("tour-card");
  if (!el) {
    throw new Error("no tour card");
  }
  return el;
}

function press(el: Element, key: string): void {
  el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  flushSync();
}

beforeEach(async () => {
  saved = [];
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ tours: { progress: [{ id: "rack", version: 1, outcome: "completed" }] } });
    }
    if (cmd === "settings_set") {
      const settings = (args as { settings: Settings }).settings;
      saved.push(settings);
      return settings;
    }
    return null;
  });
  await loadSettings();
  anchor("alpha", { left: 400, top: 100, width: 80, height: 28 });
  anchor("beta", { left: 400, top: 300, width: 80, height: 28 });
});

afterEach(() => {
  if (app) {
    unmount(app);
    app = null;
  }
  host?.remove();
  host = null;
  for (const el of anchors.splice(0)) {
    el.remove();
  }
  document.querySelectorAll("[aria-modal]").forEach((el) => el.remove());
  resetTourForTest();
  resetRecordForTest();
  resetSettingsStateForTest();
  clearMocks();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("TourOverlay (T-709)", () => {
  it("shows the step card with its count, title and a live announcement, and focuses it", () => {
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    expect(q("tour-overlay")).not.toBeNull();
    expect(q("tour-step-count")?.textContent).toBe("Step 1 of 3");
    expect(q("tour-title")?.textContent).toBe(t("tour.rack.panel.title"));
    expect(q("tour-body")?.textContent).toBe(t("tour.rack.panel.body"));
    expect(card().getAttribute("role")).toBe("dialog");
    expect(card().getAttribute("aria-labelledby")).toBe("pv-tour-title");
    expect(document.activeElement).toBe(card());
    expect(q("tour-live")?.textContent).toBe(
      t("tour.announce", { tour: t("tour.name.rack"), n: 1, count: 3, title: t("tour.rack.panel.title") }),
    );
    // Spotlight around the target: its rect grown by 6 px.
    const spot = q("tour-spotlight");
    expect(spot?.style.left).toBe("394px");
    expect(spot?.style.top).toBe("94px");
    expect(spot?.style.width).toBe("92px");
    expect(card().dataset.placement).toBe("bottom");
    // No Back on the first step.
    expect(q("tour-back")).toBeNull();
  });

  it("Next, Back and Skip move through the tour; Skip saves it as skipped", async () => {
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    q("tour-next")!.click();
    flushSync();
    expect(q("tour-step-count")?.textContent).toBe("Step 2 of 3");
    expect(q("tour-live")?.textContent).toContain("step 2 of 3");
    expect(document.activeElement).toBe(card());

    q("tour-back")!.click();
    flushSync();
    expect(q("tour-step-count")?.textContent).toBe("Step 1 of 3");

    q("tour-skip")!.click();
    flushSync();
    expect(q("tour-overlay")).toBeNull();
    expect(tourState().active).toBe(false);
    await vi.waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]!.tours.progress).toEqual([
      { id: "rack", version: 1, outcome: "completed" },
      { id: "test", version: 3, outcome: "skipped" },
    ]);
  });

  it("Done on the last step completes the tour and returns focus to where it was", async () => {
    const opener = anchor("opener", { left: 10, top: 10, width: 40, height: 20 });
    opener.focus();
    mountOverlay();
    startTour(TEST_TOUR, 2);
    flushSync();

    expect(q("tour-skip")).toBeNull();
    expect(q("tour-next")?.textContent?.trim()).toBe(t("tour.done"));
    q("tour-next")!.click();
    flushSync();

    expect(tourState().active).toBe(false);
    expect(document.activeElement).toBe(opener);
    await vi.waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]!.tours.progress).toContainEqual({ id: "test", version: 3, outcome: "completed" });
  });

  it("drives by keyboard (→ / Enter next, ← back, Esc skips) and keeps those keys from the app's shortcuts", () => {
    const appShortcuts = vi.fn();
    window.addEventListener("keydown", appShortcuts);
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    press(card(), "ArrowRight");
    expect(tourState().index).toBe(1);
    press(card(), "ArrowLeft");
    expect(tourState().index).toBe(0);
    press(card(), "Enter");
    expect(tourState().index).toBe(1);
    // Space on a card button is that button's — never the transport's play/pause.
    press(q("tour-next")!, " ");
    press(card(), "Escape");
    expect(tourState().active).toBe(false);
    expect(appShortcuts).not.toHaveBeenCalled();
    window.removeEventListener("keydown", appShortcuts);
  });

  it("falls back to a centred card over a full scrim when the target is missing", () => {
    mountOverlay();
    startTour(TEST_TOUR, 1);
    flushSync();

    expect(card().dataset.placement).toBe("center");
    expect(q("tour-scrim")).not.toBeNull();
    expect(q("tour-spotlight")).toBeNull();
    expect(q("tour-title")?.textContent).toBe(t("tour.rack.add.title"));
  });

  it("treats a hidden anchor as missing", () => {
    const hidden = document.createElement("div");
    hidden.hidden = true;
    document.body.appendChild(hidden);
    anchors.push(hidden);
    anchor("gamma", { left: 10, top: 10, width: 10, height: 10 }, hidden);
    mountOverlay();
    startTour({ ...TEST_TOUR, steps: [{ ...TEST_TOUR.steps[0]!, target: ["gamma"] }] });
    flushSync();
    expect(card().dataset.placement).toBe("center");
  });

  it("flips the card above a target near the bottom of the window", () => {
    const beta = anchors[1] as Anchor;
    beta.rect = { left: 400, top: window.innerHeight - 40, width: 80, height: 28 };
    mountOverlay();
    startTour(TEST_TOUR, 2);
    flushSync();
    expect(card().dataset.placement).toBe("top");
  });

  it("follows the target when the layout changes (resize)", () => {
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();
    expect(q("tour-spotlight")?.style.left).toBe("394px");

    (anchors[0] as Anchor).rect = { left: 600, top: 180, width: 120, height: 40 };
    window.dispatchEvent(new Event("resize"));
    flushSync();

    const spot = q("tour-spotlight");
    expect(spot?.style.left).toBe("594px");
    expect(spot?.style.top).toBe("174px");
    expect(spot?.style.width).toBe("132px");
  });

  it("advances an action-gated step once the app state changes (Next stays available to skip)", () => {
    const gated: TourDef = {
      ...TEST_TOUR,
      steps: [
        {
          ...TEST_TOUR.steps[0]!,
          interactive: true,
          waitFor: { done: () => recordState().state.recording, hintKey: "tour.welcome.record.wait" },
        },
        TEST_TOUR.steps[2]!,
      ],
    };
    mountOverlay();
    startTour(gated);
    flushSync();

    expect(q("tour-wait")?.textContent).toContain(t("tour.welcome.record.wait"));
    expect(q("tour-next")?.dataset.variant).toBe("secondary");
    // An interactive step leaves the cut-out open for the pointer: four shields, not five.
    expect(document.querySelectorAll('[data-testid="tour-shield"]')).toHaveLength(4);

    applyRecordStateForTest({ recording: true });
    flushSync();
    expect(tourState().index).toBe(1);
  });

  it("doesn't bounce forward from a gated step whose action had already happened", () => {
    applyRecordStateForTest({ recording: true });
    const gated: TourDef = {
      ...TEST_TOUR,
      steps: [
        {
          ...TEST_TOUR.steps[0]!,
          waitFor: { done: () => recordState().state.recording, hintKey: "tour.welcome.record.wait" },
        },
        TEST_TOUR.steps[2]!,
      ],
    };
    mountOverlay();
    startTour(gated);
    flushSync();
    expect(tourState().index).toBe(0);
    expect(q("tour-next")?.dataset.variant).toBe("primary");
    // Stop, then record again: that's the action.
    applyRecordStateForTest({ recording: false });
    flushSync();
    applyRecordStateForTest({ recording: true });
    flushSync();
    expect(tourState().index).toBe(1);
  });

  it("blocks the pointer over the target on a normal step", () => {
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();
    expect(document.querySelectorAll('[data-testid="tour-shield"]')).toHaveLength(5);
  });

  it("runs a step's enter hook when the step becomes current", () => {
    const enter = vi.fn();
    mountOverlay();
    startTour({ ...TEST_TOUR, steps: [TEST_TOUR.steps[0]!, { ...TEST_TOUR.steps[2]!, enter }] });
    flushSync();
    expect(enter).not.toHaveBeenCalled();
    q("tour-next")!.click();
    flushSync();
    expect(enter).toHaveBeenCalledTimes(1);
  });

  it("steps aside while a modal dialog it doesn't point into is open, then comes back", () => {
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    const modal = document.createElement("div");
    modal.setAttribute("aria-modal", "true");
    document.body.appendChild(modal);
    window.dispatchEvent(new Event("resize"));
    flushSync();
    expect(q("tour-overlay")).toBeNull();
    expect(tourState().active).toBe(true);
    // Esc belongs to the dialog while the tour is paused.
    press(document.body, "Escape");
    expect(tourState().active).toBe(true);

    modal.remove();
    window.dispatchEvent(new Event("resize"));
    flushSync();
    expect(q("tour-overlay")).not.toBeNull();
    expect(document.activeElement).toBe(card());
  });

  it("stays on top of a dialog that holds the step's target", () => {
    const modal = document.createElement("div");
    modal.setAttribute("aria-modal", "true");
    document.body.appendChild(modal);
    anchor("inside", { left: 200, top: 200, width: 60, height: 24 }, modal);
    mountOverlay();
    startTour({ ...TEST_TOUR, steps: [{ ...TEST_TOUR.steps[0]!, target: ["inside"] }] });
    flushSync();
    expect(q("tour-overlay")).not.toBeNull();
    expect(q("tour-spotlight")?.style.left).toBe("194px");
  });

  it("respects prefers-reduced-motion: no transitions and no smooth scrolling", () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.stubGlobal(
      "matchMedia",
      vi.fn((query: string) => ({
        matches: query.includes("reduce"),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    expect(q("tour-overlay")?.dataset.reducedMotion).toBe("true");
    expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ behavior: "auto" }));
  });

  it("scrolls smoothly when motion is allowed", () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.stubGlobal(
      "matchMedia",
      vi.fn((query: string) => ({ matches: false, media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    mountOverlay();
    startTour(TEST_TOUR);
    flushSync();

    expect(q("tour-overlay")?.dataset.reducedMotion).toBeUndefined();
    expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ behavior: "smooth", block: "nearest" }));
  });
});
