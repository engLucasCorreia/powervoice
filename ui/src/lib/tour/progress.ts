/**
 * Tour progress and the first-run offer rules (T-709), pure so they are tested without a store.
 *
 * `Settings.tours.progress` keeps one entry per tour: the tour's content version when it ended and
 * how it ended. A tour that gains steps bumps its version, so someone who completed or skipped the
 * old one is offered it again — except "Don't show again", which is sticky: finishing the tour
 * later from Help keeps it dismissed (the user asked never to be prompted).
 */
import type { TourOutcome, TourProgressDto } from "../ipc/bindings";

export function progressFor(progress: readonly TourProgressDto[], id: string): TourProgressDto | undefined {
  return progress.find((p) => p.id === id);
}

/** `progress` with `id`'s entry replaced (or added). */
export function recordOutcome(
  progress: readonly TourProgressDto[],
  id: string,
  version: number,
  outcome: TourOutcome,
): TourProgressDto[] {
  const previous = progressFor(progress, id);
  const sticky = previous?.outcome === "dismissed" ? "dismissed" : outcome;
  return [...progress.filter((p) => p.id !== id), { id, version, outcome: sticky }];
}

/** Whether the Welcome tour (at `version`) should be offered at start-up. */
export function shouldOfferTour(progress: readonly TourProgressDto[], id: string, version: number): boolean {
  const previous = progressFor(progress, id);
  if (!previous) {
    return true;
  }
  if (previous.outcome === "dismissed") {
    return false;
  }
  return previous.version < version;
}

export interface OfferGate {
  /** Settings said the tour is due (`shouldOfferTour`) and the user hasn't answered yet. */
  armed: boolean;
  /** The start-up crash-recovery check has finished. */
  recoveryChecked: boolean;
  /** The crash-recovery dialog (or Recovery & Storage) is open. */
  recoveryOpen: boolean;
  /** A take is being recorded or finished. */
  recording: boolean;
  /** A tour is already running. */
  tourActive: boolean;
}

/** The offer never appears over the crash-recovery dialog, while recording, or over a tour. */
export function canShowOffer(gate: OfferGate): boolean {
  return gate.armed && gate.recoveryChecked && !gate.recoveryOpen && !gate.recording && !gate.tourActive;
}
