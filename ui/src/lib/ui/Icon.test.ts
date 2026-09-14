import { afterEach, describe, expect, it } from "vitest";
import Icon from "./Icon.svelte";
import { ICON_NAMES } from "./icons";
import { render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Icon (Lucide wrapper)", () => {
  it("is decorative by default: aria-hidden, no role", () => {
    r = render(Icon, { name: "play" });
    const svg = r.target.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute("aria-hidden")).toBe("true");
    expect(svg?.getAttribute("role")).toBeNull();
  });

  it("with a label it is an image with an accessible name", () => {
    r = render(Icon, { name: "warning", label: "Clipped" });
    const svg = r.target.querySelector("svg");
    expect(svg?.getAttribute("role")).toBe("img");
    expect(svg?.getAttribute("aria-label")).toBe("Clipped");
    expect(svg?.getAttribute("aria-hidden")).toBeNull();
  });

  it("maps token sizes to pixels (sm 14, md 16, lg 20)", () => {
    r = render(Icon, { name: "play", size: "sm" });
    expect(r.target.querySelector("svg")?.getAttribute("width")).toBe("14");
    r.cleanup();
    r = render(Icon, { name: "play" });
    expect(r.target.querySelector("svg")?.getAttribute("width")).toBe("16");
    r.cleanup();
    r = render(Icon, { name: "play", size: "lg" });
    expect(r.target.querySelector("svg")?.getAttribute("height")).toBe("20");
  });

  it("can be filled (the record dot)", () => {
    r = render(Icon, { name: "record", filled: true });
    expect(r.target.querySelector("svg")?.getAttribute("fill")).toBe("currentColor");
  });

  it("renders every registered icon", () => {
    expect(ICON_NAMES.length).toBeGreaterThan(40);
    for (const name of ICON_NAMES) {
      const one = render(Icon, { name });
      expect(one.target.querySelector("svg"), name).not.toBeNull();
      one.cleanup();
    }
  });
});
