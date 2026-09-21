import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import HelpCentre from "./HelpCentre.svelte";
import { openHelpCentre, resetHelpCentreForTest } from "./helpCentre.svelte";

afterEach(() => {
  resetHelpCentreForTest();
});

function mountCentre(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(HelpCentre, { target });
  flushSync();
  return { target, app };
}

describe("HelpCentre (H-107)", () => {
  it("renders nothing when closed", () => {
    const { target, app } = mountCentre();
    expect(target.querySelector('[data-testid="help-centre"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("opens on a real dialog showing the current topic's content by default", () => {
    openHelpCentre({ doc: "user-guide", section: "plugins" });
    const { target, app } = mountCentre();

    const dialog = target.querySelector('[data-testid="help-centre"]');
    expect(dialog?.getAttribute("role")).toBe("dialog");
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(target.querySelector('[data-testid="help-content"]')?.textContent).toContain("CLAP");
    expect(target.querySelector('[data-testid="help-nav-user-guide-plugins"]')?.getAttribute("aria-current")).toBe(
      "page",
    );

    unmount(app);
    target.remove();
  });

  it("clicking a different nav item switches the displayed content", () => {
    openHelpCentre({ doc: "user-guide", section: "plugins" });
    const { target, app } = mountCentre();

    target.querySelector<HTMLButtonElement>('[data-testid="help-nav-faq-plugins"]')!.click();
    flushSync();

    expect(target.querySelector('[data-testid="help-content"]')?.textContent).toContain("Blocklisted");

    unmount(app);
    target.remove();
  });

  it("finds a section by a term that only appears in its body text, and opens it", () => {
    openHelpCentre({ doc: "user-guide", section: "overview" });
    const { target, app } = mountCentre();

    const input = target.querySelector<HTMLInputElement>('[data-testid="help-search"]')!;
    input.value = "lilv";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    // The nav is replaced by results while searching.
    expect(target.querySelector('[data-testid="help-nav"]')).toBeNull();
    const resultButtons = target.querySelectorAll('[data-testid^="help-result-"]');
    expect(resultButtons.length).toBeGreaterThan(0);

    (resultButtons[0] as HTMLButtonElement).click();
    flushSync();

    // Opening a result clears the query and shows that section's content again.
    expect(target.querySelector<HTMLInputElement>('[data-testid="help-search"]')!.value).toBe("");
    expect(target.querySelector('[data-testid="help-nav"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("shows an empty state for a query with no matches", () => {
    openHelpCentre();
    const { target, app } = mountCentre();

    const input = target.querySelector<HTMLInputElement>('[data-testid="help-search"]')!;
    input.value = "qxzzptlkjw-not-a-real-word";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(target.querySelector('[data-testid="help-results"]')?.textContent).toContain("No results");

    unmount(app);
    target.remove();
  });

  it("Escape clears a query first, then closes the dialog", () => {
    openHelpCentre();
    const { target, app } = mountCentre();
    const dialog = target.querySelector('[data-testid="help-centre"]')!;

    const input = target.querySelector<HTMLInputElement>('[data-testid="help-search"]')!;
    input.value = "plugins";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="help-results"]')).not.toBeNull();

    dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="help-centre"]')).not.toBeNull();
    expect(target.querySelector<HTMLInputElement>('[data-testid="help-search"]')!.value).toBe("");

    target
      .querySelector('[data-testid="help-centre"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="help-centre"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Close button closes the dialog", () => {
    openHelpCentre();
    const { target, app } = mountCentre();
    target.querySelector<HTMLButtonElement>('[data-testid="help-centre-close"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="help-centre"]')).toBeNull();
    unmount(app);
    target.remove();
  });
});
