import { afterEach, describe, expect, it } from "vitest";
import EmptyState from "./EmptyState.svelte";
import { render, textSnippet, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("EmptyState", () => {
  it("is a section labelled by its heading, with description, actions and shortcuts", () => {
    r = render(EmptyState, {
      icon: "waveform",
      title: "Open a file or start recording",
      description: "Your take appears here.",
      shortcuts: [
        { label: "Open a file", keys: "Ctrl+O" },
        { label: "Record", keys: "Shift+R" },
      ],
      actions: textSnippet("BUTTONS"),
      testid: "empty",
    });
    const section = r.target.querySelector("section");
    const heading = r.target.querySelector("h2");
    expect(section?.getAttribute("aria-labelledby")).toBe(heading?.id);
    expect(heading?.textContent).toBe("Open a file or start recording");
    expect(r.target.querySelector("p")?.textContent).toBe("Your take appears here.");
    expect(r.target.querySelector(".actions")?.textContent).toBe("BUTTONS");
    const terms = [...r.target.querySelectorAll("dt")].map((d) => d.textContent);
    expect(terms).toEqual(["Open a file", "Record"]);
    expect(r.target.querySelectorAll("dd kbd.pv-kbd").length).toBe(2);
    expect(section?.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("renders just a title when that's all it has", () => {
    r = render(EmptyState, { title: "No markers yet", size: "sm", level: 3 });
    expect(r.target.querySelector("h3")?.textContent).toBe("No markers yet");
    expect(r.target.querySelector("dl")).toBeNull();
    expect(r.target.querySelector("p")).toBeNull();
  });
});
