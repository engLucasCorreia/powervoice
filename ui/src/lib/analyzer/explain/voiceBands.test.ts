import { describe, expect, it } from "vitest";
import { MIDRANGE_BAND_HZ, regionsAt, voiceBands } from "./voiceBands";

describe("voice-region bands (H-92)", () => {
  it("omits the fundamental band when nothing voiced was measured", () => {
    const bands = voiceBands(null);
    expect(bands.map((b) => b.id)).not.toContain("fundamental");
    expect(bands.map((b) => b.id).sort()).toEqual(
      ["air", "low_mids", "midrange", "presence", "rumble", "sibilance"].sort(),
    );
  });

  it("adds the fundamental band from the take's own measured range, not a fixed one", () => {
    const bands = voiceBands({ lowHz: 87, highHz: 129 });
    const f0 = bands.find((b) => b.id === "fundamental");
    expect(f0).toEqual({ id: "fundamental", lowHz: 87, highHz: 129 });
  });

  it("is ascending by low edge, so the caller can draw left to right", () => {
    const bands = voiceBands({ lowHz: 90, highHz: 110 });
    const lows = bands.map((b) => b.lowHz);
    expect(lows).toEqual([...lows].sort((a, b) => a - b));
  });

  it("degenerate pitch (low == high, or zero) never adds a zero-width band", () => {
    expect(voiceBands({ lowHz: 100, highHz: 100 }).some((b) => b.id === "fundamental")).toBe(false);
    expect(voiceBands({ lowHz: 0, highHz: 0 }).some((b) => b.id === "fundamental")).toBe(false);
  });

  it("finds every band a frequency falls inside — bands legitimately overlap", () => {
    const bands = voiceBands(null);
    // 4.5 kHz is inside both presence (2-5k) and sibilance (4-10k).
    const at4500 = regionsAt(4500, bands);
    expect(at4500).toContain("presence");
    expect(at4500).toContain("sibilance");
    // 10 kHz is the shared edge of sibilance and air.
    const at10k = regionsAt(10_000, bands);
    expect(at10k).toContain("sibilance");
    expect(at10k).toContain("air");
  });

  it("a frequency outside every band matches nothing", () => {
    // Between midrange's built-in constant and presence there is no gap; use a very low
    // frequency below rumble instead.
    expect(regionsAt(5, voiceBands(null))).toEqual([]);
  });

  it("the midrange constant bridges low-mids to presence with no invented overlap", () => {
    expect(MIDRANGE_BAND_HZ[0]).toBe(500);
    expect(MIDRANGE_BAND_HZ[1]).toBe(2000);
  });
});
