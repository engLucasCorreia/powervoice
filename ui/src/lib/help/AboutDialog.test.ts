import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import AboutDialog from "./AboutDialog.svelte";
import { openAbout, resetAboutForTest } from "./about.svelte";

afterEach(() => {
  resetAboutForTest();
});

describe("AboutDialog (H-19)", () => {
  it("renders nothing when closed", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AboutDialog, { target, props: { version: "9.9.9" } });
    flushSync();
    expect(target.querySelector('[data-testid="about-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows the app name and version when open", () => {
    openAbout();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AboutDialog, { target, props: { version: "9.9.9" } });
    flushSync();

    const dialog = target.querySelector('[data-testid="about-dialog"]');
    expect(dialog?.getAttribute("role")).toBe("dialog");
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(target.querySelector('[data-testid="about-version"]')?.textContent).toBe(
      "Version 9.9.9",
    );

    unmount(app);
    target.remove();
  });

  it("Close button closes the dialog", () => {
    openAbout();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AboutDialog, { target, props: { version: "9.9.9" } });
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="about-close"]')!.click();
    flushSync();
    expect(target.querySelector('[data-testid="about-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("Third-party notices toggles a scrollable notices panel (T-705)", () => {
    openAbout();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AboutDialog, { target, props: { version: "9.9.9" } });
    flushSync();

    expect(target.querySelector('[data-testid="about-notices"]')).toBeNull();
    const toggle = target.querySelector<HTMLButtonElement>('[data-testid="about-notices-toggle"]')!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");

    toggle.click();
    flushSync();

    const notices = target.querySelector('[data-testid="about-notices"]');
    expect(notices).not.toBeNull();
    expect(notices?.textContent).toContain("PowerVoice — Third-Party Notices");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");

    toggle.click();
    flushSync();
    expect(target.querySelector('[data-testid="about-notices"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Escape closes the dialog", () => {
    openAbout();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(AboutDialog, { target, props: { version: "9.9.9" } });
    flushSync();
    target
      .querySelector('[data-testid="about-dialog"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(target.querySelector('[data-testid="about-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });
});
