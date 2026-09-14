import { describe, expect, it } from "vitest";
import { splitMnemonic } from "./mnemonic";

describe("splitMnemonic (H-19)", () => {
  it("splits at the first case-insensitive occurrence of the letter", () => {
    expect(splitMnemonic("File", "f")).toEqual({ before: "", letter: "F", after: "ile" });
    expect(splitMnemonic("Effects", "c")).toEqual({ before: "Effe", letter: "c", after: "ts" });
  });

  it("matches case-insensitively but preserves the label's own casing", () => {
    expect(splitMnemonic("Help", "H")).toEqual({ before: "", letter: "H", after: "elp" });
  });

  it("falls back to the whole label with no underlined letter when it doesn't occur", () => {
    expect(splitMnemonic("View", "z")).toEqual({ before: "View", letter: "", after: "" });
  });
});
