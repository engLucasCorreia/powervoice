import { createRawSnippet, flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Dialog from "./Dialog.svelte";
import { setPlatformForTest } from "./platform";
import type { DialogAction } from "./types";
import { key, render, type Rendered } from "./testing";

const body = createRawSnippet(() => ({
  render: () => `<div><input data-testid="first" /><p>Body text</p></div>`,
}));
const footer = createRawSnippet(() => ({
  render: () => `<div><button type="button" data-testid="cancel">Cancel</button><button type="button" data-testid="ok">OK</button></div>`,
}));

let r: Rendered | null = null;
afterEach(() => {
  setPlatformForTest(null);
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

  describe("footer button order per platform (H-26)", () => {
    const actions = (log: string[]): DialogAction[] => [
      { label: "Don't save", role: "destructive", testid: "discard", onclick: () => log.push("discard") },
      { label: "Cancel", role: "cancel", testid: "cancel", onclick: () => log.push("cancel") },
      { label: "Save", role: "primary", testid: "save", onclick: () => log.push("save") },
    ];
    const order = (root: ParentNode) =>
      [...root.querySelectorAll<HTMLElement>(".footer [data-testid]")].map((b) => b.dataset.testid);

    it("Linux and macOS: primary last, destructive apart on the left", () => {
      for (const platform of ["linux", "mac"] as const) {
        setPlatformForTest(platform);
        r = render(Dialog, { title: "Unsaved", children: body, actions: actions([]) });
        expect(order(r.target), platform).toEqual(["discard", "cancel", "save"]);
        const footer = r.target.querySelector<HTMLElement>(".footer")!;
        expect(footer.dataset.buttonOrder).toBe("primary-last");
        // The flexible gap sits between the destructive button and the answer buttons.
        expect(footer.children[1]?.classList.contains("spacer")).toBe(true);
        r.cleanup();
        r = null;
      }
    });

    it("Windows: primary first, Cancel last", () => {
      setPlatformForTest("windows");
      r = render(Dialog, { title: "Unsaved", children: body, actions: actions([]) });
      expect(order(r.target)).toEqual(["save", "discard", "cancel"]);
      expect(r.target.querySelector<HTMLElement>(".footer")!.dataset.buttonOrder).toBe("primary-first");
    });

    it("styles buttons from their role and runs their action", () => {
      const log: string[] = [];
      r = render(Dialog, { title: "Unsaved", children: body, actions: actions(log) });
      const save = r.target.querySelector<HTMLButtonElement>('[data-testid="save"]')!;
      expect(save.dataset.variant).toBe("primary");
      expect(r.target.querySelector<HTMLElement>('[data-testid="discard"]')!.dataset.variant).toBe("ghost");
      expect(r.target.querySelector<HTMLElement>('[data-testid="cancel"]')!.dataset.variant).toBe("secondary");
      save.click();
      expect(log).toEqual(["save"]);
    });
  });
});
