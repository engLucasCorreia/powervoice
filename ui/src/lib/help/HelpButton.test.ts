import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { helpCentreState, resetHelpCentreForTest } from "./helpCentre.svelte";
import HelpButton from "./HelpButton.svelte";

afterEach(() => {
  resetHelpCentreForTest();
});

describe("HelpButton (H-107)", () => {
  it("renders an accessible button naming its topic", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(HelpButton, { target, props: { doc: "user-guide", section: "plugins" } });
    flushSync();

    const button = target.querySelector('[data-testid="help-open-user-guide-plugins"]');
    expect(button?.getAttribute("aria-label")).toContain("Plugins");

    unmount(app);
    target.remove();
  });

  it("opens the Help Centre at its topic when clicked", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(HelpButton, { target, props: { doc: "user-guide", section: "plugins" } });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="help-open-user-guide-plugins"]')!.click();
    flushSync();

    expect(helpCentreState().open).toBe(true);
    expect(helpCentreState().docId).toBe("user-guide");
    expect(helpCentreState().sectionId).toBe("plugins");

    unmount(app);
    target.remove();
  });
});
