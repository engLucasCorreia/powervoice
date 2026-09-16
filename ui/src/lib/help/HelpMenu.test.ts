import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetMenuBarForTest } from "../menu/menu.svelte";
import { resetTourForTest, tourState } from "../tour/tour.svelte";
import { TOUR_IDS } from "../tour/tours";
import { aboutState, resetAboutForTest } from "./about.svelte";
import HelpMenu from "./HelpMenu.svelte";
import { resetShortcutsDialogForTest, shortcutsDialogState } from "./shortcuts.svelte";

afterEach(() => {
  resetMenuBarForTest();
  resetAboutForTest();
  resetTourForTest();
  resetShortcutsDialogForTest();
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

  it("Keyboard Shortcuts opens the shortcuts dialog and closes the menu (T-701)", () => {
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-help"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-shortcuts"]')!.click();
    flushSync();

    expect(shortcutsDialogState().open).toBe(true);
    expect(target.querySelector('[data-testid="help-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Take the Tour starts the Welcome tour from its first step (T-709)", () => {
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-help"]')!.click();
    flushSync();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-tour"]')!.click();
    flushSync();

    expect(tourState().tour?.id).toBe("welcome");
    expect(tourState().index).toBe(0);
    expect(target.querySelector('[data-testid="help-menu"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Tours ▸ lists every tour and replays the one chosen (T-709)", () => {
    const { target, app } = mountMenu();
    target.querySelector<HTMLButtonElement>('[data-testid="menu-trigger-help"]')!.click();
    flushSync();
    target.querySelector<HTMLElement>('[data-testid="menu-tours"]')!.click();
    flushSync();

    const list = target.querySelector('[data-testid="menu-tours-list"]');
    expect(list?.getAttribute("role")).toBe("menu");
    for (const id of TOUR_IDS) {
      expect(target.querySelector(`[data-testid="menu-tour-${id}"]`)).not.toBeNull();
    }
    target.querySelector<HTMLElement>('[data-testid="menu-tour-loudness"]')!.click();
    flushSync();
    expect(tourState().tour?.id).toBe("loudness");

    unmount(app);
    target.remove();
  });
});
