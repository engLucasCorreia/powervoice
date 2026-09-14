import { afterEach, describe, expect, it } from "vitest";
import Badge from "./Badge.svelte";
import StatusDot from "./StatusDot.svelte";
import { render, textSnippet, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Badge", () => {
  it("renders its text with tone and variant hooks (soft by default)", () => {
    r = render(Badge, { tone: "warning", children: textSnippet("3 dropouts") });
    const b = r.target.querySelector(".pv-badge") as HTMLElement;
    expect(b.textContent?.trim()).toBe("3 dropouts");
    expect(b.dataset.tone).toBe("warning");
    expect(b.dataset.variant).toBe("soft");
  });

  it("can carry a decorative icon next to the text", () => {
    r = render(Badge, { tone: "success", variant: "solid", icon: "check", children: textSnippet("Pass") });
    const b = r.target.querySelector(".pv-badge") as HTMLElement;
    expect(b.dataset.variant).toBe("solid");
    expect(b.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });
});

describe("StatusDot", () => {
  it("dot-only: an image named by its label (never colour alone)", () => {
    r = render(StatusDot, { tone: "success", label: "Output connected" });
    const dot = r.target.querySelector(".pv-status") as HTMLElement;
    expect(dot.getAttribute("role")).toBe("img");
    expect(dot.getAttribute("aria-label")).toBe("Output connected");
    expect(dot.dataset.tone).toBe("success");
  });

  it("with showLabel the text is visible and the dot is decorative", () => {
    r = render(StatusDot, { tone: "record", label: "Recording", showLabel: true, pulse: true });
    const status = r.target.querySelector(".pv-status") as HTMLElement;
    expect(status.getAttribute("role")).toBeNull();
    expect(status.textContent?.trim()).toBe("Recording");
    expect(status.querySelector(".dot")?.getAttribute("aria-hidden")).toBe("true");
    expect(status.dataset.pulse).toBe("true");
  });
});
