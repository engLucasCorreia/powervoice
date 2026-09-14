import { afterEach, describe, expect, it, vi } from "vitest";
import Button from "./Button.svelte";
import { byTestId, click, render, textSnippet, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Button", () => {
  it("renders its label as a type=button with variant/size hooks", () => {
    r = render(Button, { testid: "b", variant: "primary", size: "lg", children: textSnippet("Apply") });
    const b = byTestId<HTMLButtonElement>(r.target, "b");
    expect(b.tagName).toBe("BUTTON");
    expect(b.type).toBe("button");
    expect(b.textContent?.trim()).toBe("Apply");
    expect(b.dataset.variant).toBe("primary");
    expect(b.dataset.size).toBe("lg");
  });

  it("defaults to a secondary, medium button", () => {
    r = render(Button, { testid: "b", children: textSnippet("Cancel") });
    const b = byTestId(r.target, "b");
    expect(b.dataset.variant).toBe("secondary");
    expect(b.dataset.size).toBe("md");
  });

  it("calls onclick", () => {
    const onclick = vi.fn();
    r = render(Button, { testid: "b", onclick, children: textSnippet("Go") });
    click(byTestId(r.target, "b"));
    expect(onclick).toHaveBeenCalledTimes(1);
  });

  it("disabled: native disabled, no click", () => {
    const onclick = vi.fn();
    r = render(Button, { testid: "b", disabled: true, onclick, children: textSnippet("Go") });
    const b = byTestId<HTMLButtonElement>(r.target, "b");
    expect(b.disabled).toBe(true);
    click(b);
    expect(onclick).not.toHaveBeenCalled();
  });

  it("loading: aria-busy + aria-disabled, stays focusable, swallows clicks, shows a spinner", () => {
    const onclick = vi.fn();
    r = render(Button, { testid: "b", loading: true, onclick, children: textSnippet("Analyze") });
    const b = byTestId<HTMLButtonElement>(r.target, "b");
    expect(b.getAttribute("aria-busy")).toBe("true");
    expect(b.getAttribute("aria-disabled")).toBe("true");
    expect(b.disabled).toBe(false);
    click(b);
    expect(onclick).not.toHaveBeenCalled();
    expect(b.querySelector(".spinner svg")).not.toBeNull();
  });

  it("renders leading/trailing icons as decorative", () => {
    r = render(Button, {
      testid: "b",
      icon: "open",
      iconEnd: "chevronDown",
      children: textSnippet("Open"),
    });
    const icons = byTestId(r.target, "b").querySelectorAll("svg");
    expect(icons.length).toBe(2);
    icons.forEach((svg) => expect(svg.getAttribute("aria-hidden")).toBe("true"));
  });

  it("passes through ARIA/HTML attributes and submit type", () => {
    r = render(Button, {
      testid: "b",
      type: "submit",
      "aria-describedby": "hint",
      title: "Apply the change",
      children: textSnippet("Apply"),
    });
    const b = byTestId<HTMLButtonElement>(r.target, "b");
    expect(b.type).toBe("submit");
    expect(b.getAttribute("aria-describedby")).toBe("hint");
    expect(b.title).toBe("Apply the change");
  });

  it("record variant: filled red dot at rest, data-active on air", () => {
    r = render(Button, { testid: "b", variant: "record", icon: "record", children: textSnippet("Record") });
    const b = byTestId(r.target, "b");
    expect(b.dataset.variant).toBe("record");
    expect(b.dataset.active).toBeUndefined();
    expect(b.querySelector("svg")?.getAttribute("fill")).toBe("currentColor");
    r.cleanup();
    r = render(Button, { testid: "b", variant: "record", icon: "stop", active: true, children: textSnippet("Stop") });
    expect(byTestId(r.target, "b").dataset.active).toBe("true");
    expect(byTestId(r.target, "b").textContent?.trim()).toBe("Stop");
  });

  it("passes aria-pressed through for toggle buttons", () => {
    r = render(Button, { testid: "b", "aria-pressed": true, children: textSnippet("Input") });
    expect(byTestId(r.target, "b").getAttribute("aria-pressed")).toBe("true");
  });
});
