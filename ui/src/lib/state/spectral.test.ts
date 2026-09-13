import { afterEach, describe, expect, it } from "vitest";
import { clearActionHandlers, dispatchAction } from "../keymap";
import { initSpectral, resetSpectralForTest, spectralState } from "./spectral.svelte";

afterEach(() => {
  clearActionHandlers();
  resetSpectralForTest();
});

describe("spectralState (T-207, SPEC-007 §2.1)", () => {
  it("defaults to hidden, 50% split, log scale, inferno, -120/0 dB, Auto FFT", () => {
    const s = spectralState();
    expect(s.visible).toBe(false);
    expect(s.splitRatio).toBe(50);
    expect(s.freqScale).toBe("log");
    expect(s.colormap).toBe("inferno");
    expect(s.floorDb).toBe(-120);
    expect(s.ceilDb).toBe(0);
    expect(s.fftSize).toBeNull();
  });

  it("toggle() flips visibility", () => {
    const s = spectralState();
    s.toggle();
    expect(spectralState().visible).toBe(true);
    s.toggle();
    expect(spectralState().visible).toBe(false);
  });

  it("Shift+D (spectral.toggle action) toggles visibility once wired by initSpectral", () => {
    const teardown = initSpectral();
    expect(spectralState().visible).toBe(false);
    dispatchAction("spectral.toggle");
    expect(spectralState().visible).toBe(true);
    dispatchAction("spectral.toggle");
    expect(spectralState().visible).toBe(false);
    teardown();
  });

  it("setSplitRatio clamps to [0, 100]; a pane dragged to either edge stays reachable", () => {
    const s = spectralState();
    s.setSplitRatio(-10);
    expect(spectralState().splitRatio).toBe(0);
    s.setSplitRatio(150);
    expect(spectralState().splitRatio).toBe(100);
    s.setSplitRatio(37.5);
    expect(spectralState().splitRatio).toBe(37.5);
  });

  it("setFreqScale / setColormap / setFftSize set the value directly", () => {
    const s = spectralState();
    s.setFreqScale("linear");
    expect(spectralState().freqScale).toBe("linear");
    s.setColormap("viridis");
    expect(spectralState().colormap).toBe("viridis");
    s.setFftSize(4096);
    expect(spectralState().fftSize).toBe(4096);
    s.setFftSize(null);
    expect(spectralState().fftSize).toBeNull();
  });

  it("setFloorDb/setCeilDb clamp to their ranges and keep at least a 20 dB span", () => {
    const s = spectralState();
    s.setFloorDb(-1000);
    expect(spectralState().floorDb).toBe(-150); // clamped to the range floor
    s.setFloorDb(-40);
    s.setCeilDb(-45); // would leave a 5 dB span against -40 -> pushed apart to 20
    const after = spectralState();
    expect(after.floorDb).toBe(-40);
    expect(after.ceilDb).toBe(-20);
    expect(after.ceilDb - after.floorDb).toBe(20);
    s.setCeilDb(1000);
    expect(spectralState().ceilDb).toBe(6); // clamped to the range ceiling
  });
});
