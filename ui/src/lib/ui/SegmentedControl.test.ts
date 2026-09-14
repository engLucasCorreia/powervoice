import { afterEach, describe, expect, it, vi } from "vitest";
import SegmentedControl from "./SegmentedControl.svelte";
import { key, render, click, type Rendered } from "./testing";

const OPTIONS = [
  { value: "fast", label: "Fast" },
  { value: "medium", label: "Medium" },
  { value: "slow", label: "Slow" },
];

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

function radios(root: HTMLElement): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
}

describe("SegmentedControl (radio group)", () => {
  it("is a labelled radiogroup; the selected segment is checked and the only tab stop", () => {
    r = render(SegmentedControl, { options: OPTIONS, value: "medium", label: "Response" });
    const group = r.target.querySelector('[role="radiogroup"]');
    expect(group?.getAttribute("aria-label")).toBe("Response");
    const [fast, medium, slow] = radios(r.target);
    expect(medium?.getAttribute("aria-checked")).toBe("true");
    expect(fast?.getAttribute("aria-checked")).toBe("false");
    expect(medium?.tabIndex).toBe(0);
    expect(fast?.tabIndex).toBe(-1);
    expect(slow?.tabIndex).toBe(-1);
  });

  it("click selects and reports the value", () => {
    const onchange = vi.fn();
    r = render(SegmentedControl, { options: OPTIONS, value: "fast", label: "Response", onchange });
    click(radios(r.target)[2]!);
    expect(onchange).toHaveBeenCalledWith("slow");
    expect(radios(r.target)[2]?.getAttribute("aria-checked")).toBe("true");
  });

  it("arrow keys move selection and focus (wrapping), Home/End jump", () => {
    const onchange = vi.fn();
    r = render(SegmentedControl, { options: OPTIONS, value: "fast", label: "Response", onchange });
    const items = radios(r.target);
    items[0]!.focus();
    key(items[0]!, "ArrowRight");
    expect(onchange).toHaveBeenLastCalledWith("medium");
    expect(document.activeElement).toBe(radios(r.target)[1]);
    key(radios(r.target)[1]!, "End");
    expect(onchange).toHaveBeenLastCalledWith("slow");
    key(radios(r.target)[2]!, "ArrowRight");
    expect(onchange).toHaveBeenLastCalledWith("fast");
    key(radios(r.target)[0]!, "ArrowLeft");
    expect(onchange).toHaveBeenLastCalledWith("slow");
  });

  it("skips disabled segments", () => {
    const onchange = vi.fn();
    const options = [OPTIONS[0]!, { ...OPTIONS[1]!, disabled: true }, OPTIONS[2]!];
    r = render(SegmentedControl, { options, value: "fast", label: "Response", onchange });
    key(radios(r.target)[0]!, "ArrowRight");
    expect(onchange).toHaveBeenLastCalledWith("slow");
    expect(radios(r.target)[1]?.disabled).toBe(true);
  });

  it("with no matching value the first enabled segment is the tab stop", () => {
    r = render(SegmentedControl, { options: OPTIONS, value: "none", label: "Response" });
    expect(radios(r.target)[0]?.tabIndex).toBe(0);
  });

  it("icon-only segments are named by their label", () => {
    const options = [
      { value: "db", label: "Decibels", icon: "loudness" as const, iconOnly: true },
      { value: "pct", label: "Percent", icon: "meters" as const, iconOnly: true },
    ];
    r = render(SegmentedControl, { options, value: "db", label: "Unit" });
    const [db] = radios(r.target);
    expect(db?.getAttribute("aria-label")).toBe("Decibels");
    expect(db?.textContent?.trim()).toBe("");
  });

  it("disabled group disables every segment", () => {
    r = render(SegmentedControl, { options: OPTIONS, value: "fast", label: "Response", disabled: true });
    expect(radios(r.target).every((b) => b.disabled)).toBe(true);
  });
});
