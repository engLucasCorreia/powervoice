import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App.svelte";

afterEach(() => {
  clearMocks();
});

describe("App shell", () => {
  it("renders all five layout regions and the mocked app_info version", async () => {
    mockIPC((cmd) => {
      if (cmd === "app_info") {
        return { name: "VoxEdit", version: "9.9.9" };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(App, { target });

    // `onMount` awaits the mocked `invoke()` call before setting `version`; let that microtask
    // chain settle, then force Svelte to flush the resulting state update into the DOM.
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();

    for (const testId of ["toolbar", "editor", "rack", "markers-properties", "meter-bridge"]) {
      expect(target.querySelector(`[data-testid="${testId}"]`), `missing region: ${testId}`).not.toBeNull();
    }
    expect(target.querySelector('[data-testid="app-version"]')?.textContent).toBe("9.9.9");

    unmount(app);
    target.remove();
  });
});
