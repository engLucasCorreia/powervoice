/**
 * The tour engine's state (T-709): which tour is running and on which step, and whether the
 * Welcome tour is due to be offered. `TourOverlay.svelte` draws it; `WelcomeOffer.svelte` asks.
 * Progress (completed/skipped/dismissed + the tour's version) is saved in `Settings.tours`.
 */
import type { TourOutcome, ToursSettingsDto } from "../ipc/bindings";
import { closeAllMenus } from "../menu/menubar.svelte";
import { saveSettings, settingsState } from "../state/settings.svelte";
import { recordOutcome, shouldOfferTour } from "./progress";
import { TOURS, type TourDef, type TourId, type TourStep } from "./tours";

interface Running {
  tour: TourDef;
  index: number;
}

let running = $state<Running | null>(null);
let offerArmed = $state(false);
/** Where focus was when the tour started; restored when it ends. */
let returnFocus: HTMLElement | null = null;

/** Read-only accessor for components. */
export function tourState(): {
  readonly active: boolean;
  readonly tour: TourDef | null;
  readonly step: TourStep | null;
  readonly index: number;
  readonly count: number;
  readonly isLast: boolean;
  readonly offerArmed: boolean;
} {
  return {
    get active() {
      return running !== null;
    },
    get tour() {
      return running?.tour ?? null;
    },
    get step() {
      return running ? (running.tour.steps[running.index] ?? null) : null;
    },
    get index() {
      return running?.index ?? 0;
    },
    get count() {
      return running?.tour.steps.length ?? 0;
    },
    get isLast() {
      return running !== null && running.index === running.tour.steps.length - 1;
    },
    get offerArmed() {
      return offerArmed;
    },
  };
}

function enter(next: Running): void {
  running = next;
  next.tour.steps[next.index]?.enter?.();
}

/** Starts (or restarts) a tour, optionally at a step (0-based, clamped). */
export function startTour(tour: TourId | TourDef, index = 0): void {
  const def = typeof tour === "string" ? TOURS[tour] : tour;
  if (def.steps.length === 0) {
    return;
  }
  offerArmed = false;
  closeAllMenus();
  if (!running) {
    returnFocus = typeof document !== "undefined" && document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }
  enter({ tour: def, index: Math.min(Math.max(0, Math.trunc(index)), def.steps.length - 1) });
}

/** Next step, or Done on the last one. */
export function nextStep(): void {
  if (!running) {
    return;
  }
  if (running.index >= running.tour.steps.length - 1) {
    void endTour("completed");
    return;
  }
  enter({ tour: running.tour, index: running.index + 1 });
}

export function prevStep(): void {
  if (running && running.index > 0) {
    enter({ tour: running.tour, index: running.index - 1 });
  }
}

/** Skip tour / Esc: ends it and remembers that it was skipped. */
export function skipTour(): Promise<void> {
  return endTour("skipped");
}

async function saveOutcome(id: string, version: number, outcome: TourOutcome): Promise<void> {
  const current = settingsState().current;
  if (!current) {
    return;
  }
  const progress = current.tours?.progress ?? [];
  await saveSettings({ tours: { progress: recordOutcome(progress, id, version, outcome) } });
}

async function endTour(outcome: TourOutcome): Promise<void> {
  const ended = running;
  if (!ended) {
    return;
  }
  running = null;
  const focus = returnFocus;
  returnFocus = null;
  if (focus?.isConnected) {
    focus.focus();
  }
  await saveOutcome(ended.tour.id, ended.tour.version, outcome);
}

// --- First-run offer -----------------------------------------------------------------------------

/** After settings load: arm the Welcome offer when the tour is due (`progress.ts`). */
export function armWelcomeOffer(tours: ToursSettingsDto | null | undefined): void {
  offerArmed = shouldOfferTour(tours?.progress ?? [], TOURS.welcome.id, TOURS.welcome.version);
}

/** Start: runs the Welcome tour. */
export function acceptWelcomeOffer(): void {
  offerArmed = false;
  startTour("welcome");
}

/** Later: nothing is saved; the next start offers it again. */
export function postponeWelcomeOffer(): void {
  offerArmed = false;
}

/** Don't show again: saved, never offered again (Help → Take the Tour still runs it). */
export function dismissWelcomeOffer(): Promise<void> {
  offerArmed = false;
  return saveOutcome(TOURS.welcome.id, TOURS.welcome.version, "dismissed");
}

/** Test/teardown helper. */
export function resetTourForTest(): void {
  running = null;
  offerArmed = false;
  returnFocus = null;
}
