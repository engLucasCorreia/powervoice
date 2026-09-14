import { afterEach, describe, expect, it } from "vitest";
import Separator from "./Separator.svelte";
import { render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Separator", () => {
  it("is a horizontal separator by default", () => {
    r = render(Separator, {});
    const el = r.target.querySelector(".pv-separator");
    expect(el?.getAttribute("role")).toBe("separator");
    expect(el?.getAttribute("aria-orientation")).toBe("horizontal");
  });

  it("vertical separators (between toolbar groups) say so", () => {
    r = render(Separator, { orientation: "vertical" });
    expect(r.target.querySelector(".pv-separator")?.getAttribute("aria-orientation")).toBe("vertical");
  });

  it("decorative separators are hidden from assistive tech", () => {
    r = render(Separator, { decorative: true });
    const el = r.target.querySelector(".pv-separator");
    expect(el?.getAttribute("role")).toBeNull();
    expect(el?.getAttribute("aria-hidden")).toBe("true");
  });
});
