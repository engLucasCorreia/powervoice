import { describe, expect, it } from "vitest";
import { ViewportWriter } from "./viewportFollow";

describe("ViewportWriter", () => {
  it("a fresh writer never reports a user change (nothing applied yet)", () => {
    const writer = new ViewportWriter();
    expect(writer.isUserChange(0)).toBe(false);
    expect(writer.isUserChange(12_345)).toBe(false);
  });

  it("no change is reported when startSample matches the last value set", () => {
    const writer = new ViewportWriter();
    writer.set(1_000);
    expect(writer.isUserChange(1_000)).toBe(false);
  });

  it("a change is reported when startSample differs from the last value set", () => {
    const writer = new ViewportWriter();
    writer.set(1_000);
    expect(writer.isUserChange(2_000)).toBe(true);
  });

  it("reset() forgets the last value, so the next check never reports a change", () => {
    const writer = new ViewportWriter();
    writer.set(1_000);
    writer.reset();
    expect(writer.isUserChange(2_000)).toBe(false);
  });

  it("tracks a sequence of its own writes without ever flagging them as user changes", () => {
    const writer = new ViewportWriter();
    let current = 0;
    writer.set(current);
    // Each tick re-checks the *previously applied* value before writing the next one — exactly
    // how `WaveformView`'s effect uses this across animation frames.
    for (const next of [500, 1_000, 1_000, 900]) {
      expect(writer.isUserChange(current)).toBe(false);
      current = next;
      writer.set(current);
    }
  });
});
