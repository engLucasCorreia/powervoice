import { afterEach, describe, expect, it, vi } from "vitest";
import ToggleButton from "./ToggleButton.svelte";
import { byTestId, click, render, textSnippet, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("ToggleButton", () => {
  it("is a button with aria-pressed that flips on click and reports the new state", () => {
    const onchange = vi.fn();
    r = render(ToggleButton, { testid: "tb", onchange, children: textSnippet("Spectral") });
    const b = byTestId<HTMLButtonElement>(r.target, "tb");
    expect(b.type).toBe("button");
    expect(b.getAttribute("aria-pressed")).toBe("false");
    click(b);
    expect(b.getAttribute("aria-pressed")).toBe("true");
    expect(onchange).toHaveBeenLastCalledWith(true);
    click(b);
    expect(b.getAttribute("aria-pressed")).toBe("false");
    expect(onchange).toHaveBeenLastCalledWith(false);
  });

  it("starts from the pressed prop", () => {
    r = render(ToggleButton, { testid: "tb", pressed: true, children: textSnippet("A/B") });
    expect(byTestId(r.target, "tb").getAttribute("aria-pressed")).toBe("true");
  });

  it("disabled doesn't toggle", () => {
    const onchange = vi.fn();
    r = render(ToggleButton, { testid: "tb", disabled: true, onchange, children: textSnippet("A/B") });
    click(byTestId(r.target, "tb"));
    expect(onchange).not.toHaveBeenCalled();
    expect(byTestId(r.target, "tb").getAttribute("aria-pressed")).toBe("false");
  });

  it("can carry a decorative icon", () => {
    r = render(ToggleButton, { testid: "tb", icon: "spectral", children: textSnippet("Spectral") });
    expect(byTestId(r.target, "tb").querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });
});
