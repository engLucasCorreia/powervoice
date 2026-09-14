import { invoke } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { IpcError } from "../ipc/bindings";
import { clearNotices, pushNotice } from "../state/notices.svelte";
import { noticeFromIpcError } from "./fromIpcError";
import NoticeHost from "./NoticeHost.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
});

describe("IpcError -> toast", () => {
  it("renders the i18n text with interpolated params (Vitest + mockIPC)", async () => {
    mockIPC((cmd) => {
      if (cmd === "settings_set") {
        const error: IpcError = {
          code: "io",
          key: "error.io",
          params: { message: "disk full" },
        };
        throw error;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    let caught: IpcError | undefined;
    try {
      await invoke("settings_set", { settings: {} });
    } catch (err) {
      caught = err as IpcError;
    }
    expect(caught).toBeDefined();
    expect(caught?.code).toBe("io");

    pushNotice(noticeFromIpcError(caught as IpcError));

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    const toast = target.querySelector('[data-testid="toast"]');
    expect(toast).not.toBeNull();
    expect(toast?.getAttribute("data-level")).toBe("error");
    expect(toast?.textContent).toContain("A file operation failed: disk full");

    unmount(app);
    target.remove();
  });

  it("renders a persistent banner distinctly from a toast, and replaces by id", () => {
    pushNotice({
      level: "error",
      key: "error.device_lost",
      params: {},
      persistent: true,
      id: "device:output",
      cleared: false,
    });
    pushNotice({
      level: "info",
      key: "error.cancelled",
      params: {},
      persistent: true,
      id: "device:output",
      cleared: false,
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    const banners = target.querySelectorAll('[data-testid="banner"]');
    expect(banners.length).toBe(1);
    expect(banners[0]?.textContent).toContain("Cancelled.");

    unmount(app);
    target.remove();
  });
});
