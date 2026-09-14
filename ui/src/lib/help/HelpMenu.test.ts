import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetMenuBarForTest } from "../menu/menubar.svelte";
import { aboutState, resetAboutForTest } from "./about.svelte";
import HelpMenu from "./HelpMenu.svelte";

afterEach(() => {
  resetMenuBarForTest();
  resetAboutForTest();
});

function mountMenu(): { target: HTMLElement; app: ReturnType<typeof mount> } {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(HelpMenu, { target });
  flushSync();
  return { target, app };
}

describe("HelpMenu (H-19)", () => {
  it("is a closed dropdown with a menubar-item trigger by default", () => {
    const { target, app } = mountMenu();
    const trigger = target.querySelector('[data-testid="menu-trigger-help"]');
    expect(trigger?.getAttribute("role")).toBe("menuitem");
    expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
    expect(target.querySelector('[data-testid="help-menu"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("opens on click, exposing About PowerVoice… as a role=menuitem", () => {
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-help"]')!.click();
    flushSync();

    const popup = target.querySelector('[data-testid="help-menu"]');
    expect(popup?.getAttribute("role")).toBe("menu");
    const about = target.querySelector('[data-testid="menu-about"]');
    expect(about?.getAttribute("role")).toBe("menuitem");
    expect(about?.textContent).toContain("About PowerVoice");

    unmount(app);
    target.remove();
  });

  it("About PowerVoice… opens the About dialog and closes the menu", () => {
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-help"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-about"]')!.click();
    flushSync();

    expect(aboutState().open).toBe(true);
    expect(target.querySelector('[data-testid="help-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
