import { flushSync, mount, unmount } from "svelte";
import { describe, expect, it, vi } from "vitest";
import type { ThemePref } from "../ipc/bindings";
import ThemePicker from "./ThemePicker.svelte";

function render(value: ThemePref, onchange = vi.fn()) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ThemePicker, { target, props: { value, label: "Theme", onchange } });
  flushSync();
  return {
    target,
    onchange,
    radios: () => [...target.querySelectorAll<HTMLButtonElement>('[role="radio"]')],
    done: () => {
      unmount(app);
      target.remove();
    },
  };
}

describe("ThemePicker (T-708)", () => {
  it("is a labelled radio group with one card per theme, the current one checked", () => {
    const r = render("light");
    const group = r.target.querySelector('[role="radiogroup"]');
    expect(group?.getAttribute("aria-label")).toBe("Theme");
    expect(r.radios().map((b) => b.dataset.testid)).toEqual([
      "preferences-theme-dark",
      "preferences-theme-light",
      "preferences-theme-system",
      "preferences-theme-high_contrast",
    ]);
    expect(r.radios().map((b) => b.getAttribute("aria-checked"))).toEqual(["false", "true", "false", "false"]);
    // Roving tab stop on the checked card.
    expect(r.radios().map((b) => b.tabIndex)).toEqual([-1, 0, -1, -1]);
    expect(r.radios()[3]?.textContent).toContain("High contrast");
    r.done();
  });

  it("each swatch renders a live miniature in that theme's own tokens", () => {
    const r = render("dark");
    const themed = (pref: string) =>
      [...r.target.querySelectorAll(`[data-testid="theme-swatch-${pref}"] [data-theme]`)].map((el) =>
        el.getAttribute("data-theme"),
      );
    expect(themed("dark")).toEqual(["dark"]);
    expect(themed("light")).toEqual(["light"]);
    expect(themed("high_contrast")).toEqual(["high-contrast"]);
    expect(themed("system")).toEqual(["dark", "light"]); // split diagonally
    expect(r.target.querySelector('[data-testid="theme-swatch-dark"]')?.getAttribute("aria-hidden")).toBe("true");
    r.done();
  });

  it("a click picks a theme; the current one is not re-picked", () => {
    const r = render("dark");
    r.radios()[3]!.click();
    expect(r.onchange).toHaveBeenCalledWith("high_contrast");
    r.radios()[0]!.click();
    expect(r.onchange).toHaveBeenCalledTimes(1);
    r.done();
  });

  it("arrow keys move and select (radio semantics), Home/End jump", () => {
    const r = render("dark");
    r.radios()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(r.onchange).toHaveBeenLastCalledWith("light");
    expect(document.activeElement).toBe(r.radios()[1]);
    r.radios()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(r.onchange).toHaveBeenLastCalledWith("high_contrast");
    r.radios()[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    expect(r.onchange).toHaveBeenLastCalledWith("high_contrast"); // wraps to the last
    r.done();
  });
});
