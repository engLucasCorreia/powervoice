import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t, type MessageKey } from "../i18n";
import type { Settings } from "../ipc/bindings";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { settingsFixture } from "../test/fixtures";
import {
  armWelcomeOffer,
  dismissWelcomeOffer,
  nextStep,
  prevStep,
  resetTourForTest,
  skipTour,
  startTour,
  tourState,
} from "./tour.svelte";
import { TOUR_IDS, TOURS } from "./tours";

afterEach(() => {
  resetTourForTest();
  resetSettingsStateForTest();
  clearMocks();
});

async function withSettings(progress: Settings["tours"]["progress"] = []): Promise<Settings[]> {
  const saved: Settings[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ tours: { progress } });
    }
    if (cmd === "settings_set") {
      const settings = (args as { settings: Settings }).settings;
      saved.push(settings);
      return settings;
    }
    return null;
  });
  await loadSettings();
  return saved;
}

describe("tour store (T-709)", () => {
  it("starts at a clamped step and walks forward and back within the tour", () => {
    startTour("rack", 99);
    expect(tourState().index).toBe(TOURS.rack.steps.length - 1);
    expect(tourState().isLast).toBe(true);
    prevStep();
    expect(tourState().index).toBe(TOURS.rack.steps.length - 2);
    startTour("rack", -3);
    expect(tourState().index).toBe(0);
    prevStep();
    expect(tourState().index).toBe(0);
    nextStep();
    expect(tourState().step?.id).toBe(TOURS.rack.steps[1]!.id);
  });

  it("saves Done as completed with the tour's version", async () => {
    const saved = await withSettings();
    startTour("punch", TOURS.punch.steps.length - 1);
    nextStep();
    expect(tourState().active).toBe(false);
    await vi.waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]!.tours.progress).toEqual([{ id: "punch", version: TOURS.punch.version, outcome: "completed" }]);
  });

  it("saves Skip as skipped", async () => {
    const saved = await withSettings();
    startTour("noise");
    await skipTour();
    expect(saved[0]!.tours.progress).toEqual([{ id: "noise", version: TOURS.noise.version, outcome: "skipped" }]);
  });

  it("arms the Welcome offer only when the tour is due", () => {
    armWelcomeOffer({ progress: [] });
    expect(tourState().offerArmed).toBe(true);
    armWelcomeOffer({ progress: [{ id: "welcome", version: TOURS.welcome.version, outcome: "skipped" }] });
    expect(tourState().offerArmed).toBe(false);
    armWelcomeOffer({ progress: [{ id: "welcome", version: TOURS.welcome.version - 1, outcome: "completed" }] });
    expect(tourState().offerArmed).toBe(true);
    armWelcomeOffer(undefined);
    expect(tourState().offerArmed).toBe(true);
  });

  it("Don't show again stays dismissed after the tour is taken from Help", async () => {
    const saved = await withSettings();
    armWelcomeOffer({ progress: [] });
    await dismissWelcomeOffer();
    expect(tourState().offerArmed).toBe(false);
    expect(saved.at(-1)!.tours.progress).toEqual([
      { id: "welcome", version: TOURS.welcome.version, outcome: "dismissed" },
    ]);
  });

  it("starting a tour disarms the offer", () => {
    armWelcomeOffer({ progress: [] });
    startTour("welcome");
    expect(tourState().offerArmed).toBe(false);
  });
});

describe("tour content (T-709)", () => {
  it("every tour has steps with unique ids and real strings", () => {
    for (const id of TOUR_IDS) {
      const tour = TOURS[id];
      expect(tour.id).toBe(id);
      expect(tour.steps.length).toBeGreaterThan(0);
      expect(new Set(tour.steps.map((s) => s.id)).size).toBe(tour.steps.length);
      for (const step of tour.steps) {
        const params = step.params?.() ?? {};
        for (const key of [step.titleKey, step.bodyKey, step.waitFor?.hintKey].filter(Boolean) as MessageKey[]) {
          const text = t(key, params);
          expect(text.length).toBeGreaterThan(0);
          // Every placeholder is filled (a shortcut label for this platform).
          expect(text).not.toMatch(/\{\w+\}/);
        }
      }
    }
  });

  it("the Welcome tour follows the first-recording flow in about ten steps", () => {
    expect(TOURS.welcome.steps.map((s) => s.id)).toEqual([
      "intro",
      "devices",
      "record",
      "time",
      "editor",
      "markers",
      "rack",
      "noise",
      "dock",
      "finish",
    ]);
    // Recording a practice take is the action-gated step.
    expect(TOURS.welcome.steps.find((s) => s.id === "record")?.waitFor).toBeDefined();
  });
});
