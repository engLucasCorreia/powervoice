import { invoke } from "@tauri-apps/api/core";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { IpcError } from "../ipc/bindings";
import { initMarkers, resetMarkersForTest } from "../markers/markers.svelte";
import { clearNotices, pushNotice } from "../state/notices.svelte";
import { noticeFixture } from "../test/fixtures";
import { noticeFromIpcError } from "./fromIpcError";
import NoticeHost from "./NoticeHost.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetMarkersForTest();
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
    pushNotice(
      noticeFixture({
        level: "error",
        key: "error.device_lost",
        persistent: true,
        id: "device:output",
      }),
    );
    pushNotice(
      noticeFixture({
        key: "error.cancelled",
        persistent: true,
        id: "device:output",
      }),
    );

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

describe("Notice.action (H-67, SPEC-002 AC-7)", () => {
  it("a notice with no action renders no button — looks exactly as it does today", () => {
    pushNotice(noticeFixture());

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    expect(target.querySelector('[data-testid="notice-action"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("a toast with an action renders a keyboard-reachable button whose click dispatches it (Go to first dropout)", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") {
        return [
          { id: 1, pos_samples: 144_000, len_samples: 480, name: "Dropout 10 ms", kind: "dropout" },
        ];
      }
      if (cmd === "transport_seek") {
        seeks.push(args);
        return null;
      }
      return null;
    });
    const stopMarkers = await initMarkers();

    pushNotice(
      noticeFixture({
        level: "warning",
        key: "notice.record.dropouts",
        params: { count: "1" },
        action: { label_key: "notice.action.go_to_first", id: "go_to_first_dropout" },
      }),
    );

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="notice-action"]');
    expect(button).not.toBeNull();
    expect(button?.textContent).toContain("Go to first");
    // Keyboard reachable: a plain <button>, never removed from the tab order.
    expect(button?.tabIndex).toBe(0);
    expect(button?.disabled).toBe(false);

    button?.click();
    flushSync();
    expect(seeks).toEqual([{ positionSamples: 144_000 }]);

    unmount(app);
    target.remove();
    stopMarkers();
  });

  it("a toast with the Undo-marker-delete action dispatches history_undo (H-64, SPEC-009 §2.6)", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      if (cmd === "history_undo") {
        calls.push(cmd);
        return { can_undo: false, can_redo: true, undo_label: null, redo_label: "Delete Markers" };
      }
      return null;
    });

    pushNotice(
      noticeFixture({
        key: "notice.markers_deleted",
        params: { count: "37" },
        action: { label_key: "notice.action.undo", id: "undo_marker_delete" },
      }),
    );

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="notice-action"]');
    expect(button?.textContent).toContain("Undo");
    button?.click();
    await Promise.resolve();
    flushSync();

    expect(calls).toEqual(["history_undo"]);

    unmount(app);
    target.remove();
  });

  it("a persistent banner with an action renders the same button", () => {
    pushNotice(
      noticeFixture({
        level: "warning",
        key: "notice.record.dropouts",
        params: { count: "1" },
        persistent: true,
        id: "dropouts",
        action: { label_key: "notice.action.go_to_first", id: "go_to_first_dropout" },
      }),
    );

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(NoticeHost, { target });
    flushSync();

    const banner = target.querySelector('[data-testid="banner"]');
    expect(banner?.querySelector('[data-testid="notice-action"]')?.textContent).toContain(
      "Go to first",
    );

    unmount(app);
    target.remove();
  });
});
