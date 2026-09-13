import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { Notice } from "../ipc/bindings";
import { clearNotices, initNotices, noticesState } from "./notices.svelte";

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
    };
    await emit("notice", notice);

    expect(noticesState().toasts).toHaveLength(1);
    expect(noticesState().toasts[0]?.key).toBe("notice.normalize_silent");

    stop();
  });
});
