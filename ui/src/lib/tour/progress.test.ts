import { describe, expect, it } from "vitest";
import type { TourProgressDto } from "../ipc/bindings";
import { canShowOffer, recordOutcome, shouldOfferTour, type OfferGate } from "./progress";

const READY: OfferGate = {
  armed: true,
  recoveryChecked: true,
  recoveryOpen: false,
  recording: false,
  tourActive: false,
};

describe("tour progress (T-709)", () => {
  it("offers a tour nobody has seen", () => {
    expect(shouldOfferTour([], "welcome", 1)).toBe(true);
  });

  it("doesn't offer a completed or skipped tour again until it gains a version", () => {
    const completed: TourProgressDto[] = [{ id: "welcome", version: 1, outcome: "completed" }];
    const skipped: TourProgressDto[] = [{ id: "welcome", version: 1, outcome: "skipped" }];
    expect(shouldOfferTour(completed, "welcome", 1)).toBe(false);
    expect(shouldOfferTour(skipped, "welcome", 1)).toBe(false);
    expect(shouldOfferTour(completed, "welcome", 2)).toBe(true);
    expect(shouldOfferTour(skipped, "welcome", 2)).toBe(true);
  });

  it("never offers a dismissed tour, even a newer version", () => {
    const dismissed: TourProgressDto[] = [{ id: "welcome", version: 1, outcome: "dismissed" }];
    expect(shouldOfferTour(dismissed, "welcome", 1)).toBe(false);
    expect(shouldOfferTour(dismissed, "welcome", 7)).toBe(false);
  });

  it("records one entry per tour, replacing the previous one", () => {
    let progress = recordOutcome([], "rack", 1, "skipped");
    progress = recordOutcome(progress, "welcome", 1, "completed");
    progress = recordOutcome(progress, "rack", 2, "completed");
    expect(progress).toEqual([
      { id: "welcome", version: 1, outcome: "completed" },
      { id: "rack", version: 2, outcome: "completed" },
    ]);
  });

  it("keeps 'Don't show again' sticky when the tour is later taken from Help", () => {
    const dismissed: TourProgressDto[] = [{ id: "welcome", version: 1, outcome: "dismissed" }];
    expect(recordOutcome(dismissed, "welcome", 2, "completed")).toEqual([
      { id: "welcome", version: 2, outcome: "dismissed" },
    ]);
  });
});

describe("first-run offer gate (T-709)", () => {
  it("shows once armed with recovery checked and nothing in the way", () => {
    expect(canShowOffer(READY)).toBe(true);
  });

  it("never shows over the crash-recovery dialog, or before recovery was checked", () => {
    expect(canShowOffer({ ...READY, recoveryOpen: true })).toBe(false);
    expect(canShowOffer({ ...READY, recoveryChecked: false })).toBe(false);
  });

  it("never shows while recording or during a tour, or when not armed", () => {
    expect(canShowOffer({ ...READY, recording: true })).toBe(false);
    expect(canShowOffer({ ...READY, tourActive: true })).toBe(false);
    expect(canShowOffer({ ...READY, armed: false })).toBe(false);
  });
});
