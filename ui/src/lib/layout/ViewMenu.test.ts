import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { applyAnalyzerPrefs, resetAnalyzerForTest } from "../analyzer/analyzer.svelte";
import { resetSettingsStateForTest } from "../state/settings.svelte";
import ViewMenu from "./ViewMenu.svelte";

afterEach(() => {
  clearMocks();
  resetAnalyzerForTest();
  resetSettingsStateForTest();
});

/** H-16 (SPEC-007 §2.9): "shown by default and can be hidden with View → Analyzer". */
describe("ViewMenu", () => {
  it("reflects the analyzer's current visibility and toggles it on click", () => {
    // `setAnalyzerVisible` persists via `settings_set` (a no-op here: no `loadSettings()` call
    // means the settings store has nothing loaded yet to merge the patch into) — the click
    // handler's job is the visible/aria-pressed toggle this test actually checks.
    mockIPC(() => null);
    applyAnalyzerPrefs({ visible: true, response: "medium", peakHold: true });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ViewMenu, { target });
    flushSync();

    const button = target.querySelector('[data-testid="menu-view-analyzer"]');
    expect(button).not.toBeNull();
    expect(button?.getAttribute("aria-pressed")).toBe("true");

    (button as HTMLButtonElement).click();
    flushSync();
    expect(button?.getAttribute("aria-pressed")).toBe("false");

    (button as HTMLButtonElement).click();
    flushSync();
    expect(button?.getAttribute("aria-pressed")).toBe("true");

    unmount(app);
    target.remove();
  });
});
