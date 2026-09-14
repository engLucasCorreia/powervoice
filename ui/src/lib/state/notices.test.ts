import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { Notice } from "../ipc/bindings";
import { clearNotices, initNotices, noticesState, pushNotice } from "./notices.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
});

describe("initNotices (S2-02: the shared `notice` event -> the toast/banner store)", () => {
  it("pushes every notice event it receives (e.g. SPEC-010 §2.7's normalize notices)", async () => {
    mockIPC(() => null, { shouldMockEvents: true });
    const stop = await initNotices();

    const notice: Notice = {
      level: "info",
      key: "notice.normalize_silent",
      params: {},
      persistent: false,
      id: null,
      cleared: false,
    };
    await emit("notice", notice);

    expect(noticesState().toasts).toHaveLength(1);
    expect(noticesState().toasts[0]?.key).toBe("notice.normalize_silent");

    stop();
  });
});

describe("pushNotice with cleared: true (H-17: e.g. the disk-almost-full banner)", () => {
  it("removes the banner sharing the id instead of adding anything", () => {
    pushNotice({
      level: "warning",
      key: "notice.disk.almost_full",
      params: {},
      persistent: true,
      id: "disk_almost_full",
      cleared: false,
    });
    expect(noticesState().banners).toHaveLength(1);

    pushNotice({
      level: "info",
      key: "",
      params: {},
      persistent: true,
      id: "disk_almost_full",
      cleared: true,
    });
    expect(noticesState().banners).toHaveLength(0);
    expect(noticesState().toasts).toHaveLength(0);
  });
});
