import { describe, expect, it } from "vitest";
import { voiceBandLabel } from "./explainLabels";

describe("voiceBandLabel (H-92): structural, never interpretive", () => {
  it("names every band the graph can shade", () => {
    const ids = ["rumble", "fundamental", "low_mids", "midrange", "presence", "sibilance", "air"] as const;
    for (const id of ids) {
      expect(voiceBandLabel(id)).toBeTruthy();
    }
  });
});
