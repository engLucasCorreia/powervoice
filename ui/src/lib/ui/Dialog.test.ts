import { createRawSnippet, flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Dialog from "./Dialog.svelte";
import { key, render, type Rendered } from "./testing";

const body = createRawSnippet(() => ({
  render: () => `<div><input data-testid="first" /><p>Body text</p></div>`,
}));
const footer = createRawSnippet(() => ({
  render: () => `<div><button type="button" data-testid="cancel">Cancel</button><button type="button" data-testid="ok">OK</button></div>`,
}));

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
  document.body.innerHTML = "";
});

function dialog(root: ParentNode): HTMLElement {
  const el = root.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]');
  if (!el) throw new Error("no dialog");
  return el;
}

describe("Dialog shell", () => {
  it("is a modal dialog labelled by its title, with body, footer and passthrough attributes", () => {
    r = render(Dialog, {
      title: "Normalize",
      titleId: "normalize-title",
      testid: "normalize-dialog",
      size: "sm",
      "data-kind": "close",
      children: body,
      footer,
    });
    const d = dialog(r.target);
    expect(d.getAttribute("aria-modal")).toBe("true");
    expect(d.getAttribute("aria-labelledby")).toBe("normalize-title");
    expect(document.getElementById("normalize-title")?.textContent).toBe("Normalize");
    expect(d.dataset.testid).toBe("normalize-dialog");
    expect(d.dataset.kind).toBe("close");
    expect(d.dataset.size).toBe("sm");
    expect(d.querySelector(".body p")?.textContent).toBe("Body text");
    expect(d.querySelector(".footer [data-testid='ok']")).not.toBeNull();
  });

  it("alertdialog role and an auto-generated title id", () => {
    r = render(Dialog, { title: "Discard changes?", role: "alertdialog", children: body });
    const d = dialog(r.target);
    expect(d.getAttribute("role")).toBe("alertdialog");
    const id = d.getAttribute("aria-labelledby");
    expect(id).toBeTruthy();
    expect(document.getElementById(id!)?.textContent).toBe("Discard changes?");
  });

  it("takes focus on open and gives it back to the opener on close", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    r = render(Dialog, { title: "About", children: body, footer });
    await Promise.resolve();
    flushSync();
    expect(dialog(r.target).contains(document.activeElement)).toBe(true);
    r.cleanup();
    r = null;
    expect(document.activeElement).toBe(opener);
  });

  it("traps Tab inside the dialog", () => {
    r = render(Dialog, { title: "Export", children: body, footer });
    const first = r.target.querySelector<HTMLElement>('[data-testid="first"]')!;
    const last = r.target.querySelector<HTMLElement>('[data-testid="ok"]')!;
    last.focus();
    key(last, "Tab");
    expect(document.activeElement).toBe(first);
    key(first, "Tab", { shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("forwards keydown to the owner (Escape/Enter semantics stay with each dialog)", () => {
    const onkeydown = vi.fn();
    r = render(Dialog, { title: "Normalize", children: body, onkeydown });
    key(dialog(r.target), "Escape");
    expect(onkeydown).toHaveBeenCalledTimes(1);
    expect((onkeydown.mock.calls[0]?.[0] as KeyboardEvent).key).toBe("Escape");
  });
});
