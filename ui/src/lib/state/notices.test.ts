import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { noticeFixture } from "../test/fixtures";
import { clearNotices, initNotices, noticesState, pushNotice } from "./notices.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
});

describe("initNotices (S2-02: the shared `notice` event -> the toast/banner store)", () => {
  it("pushes every notice event it receives (e.g. SPEC-010 §2.7's normalize notices)", async () => {
    mockIPC(() => null, { shouldMockEvents: true });
    const stop = await initNotices();

    const notice = noticeFixture({ key: "notice.normalize_silent" });
    await emit("notice", notice);

    expect(noticesState().toasts).toHaveLength(1);
    expect(noticesState().toasts[0]?.key).toBe("notice.normalize_silent");

    stop();
  });
});

describe("pushNotice with cleared: true (H-17: e.g. the disk-almost-full banner)", () => {
  it("removes the banner sharing the id instead of adding anything", () => {
    pushNotice(
      noticeFixture({
        level: "warning",
        key: "notice.disk.almost_full",
        persistent: true,
        id: "disk_almost_full",
      }),
    );
    expect(noticesState().banners).toHaveLength(1);

    pushNotice(
      noticeFixture({
        key: "",
        persistent: true,
        id: "disk_almost_full",
        cleared: true,
      }),
    );
    expect(noticesState().banners).toHaveLength(0);
    expect(noticesState().toasts).toHaveLength(0);
  });
});

describe("pushNotice with auto_dismiss_ms (H-59, SPEC-001 §2.3)", () => {
  const banner = (key: string, autoDismissMs: number | null) =>
    noticeFixture({
      key,
      persistent: true,
      id: "device:output",
      auto_dismiss_ms: autoDismissMs,
    });

  it("dismisses the reconnected banner on its own, while the lost banner stays", () => {
    vi.useFakeTimers();
    try {
      pushNotice(banner("notice.device.lost.output.playback_stopped", null));
      vi.advanceTimersByTime(60_000);
      expect(noticesState().banners).toHaveLength(1);

      // The reconnect replaces the lost banner in place, then goes away by itself.
      pushNotice(banner("notice.device.reconnected.output", 4000));
      expect(noticesState().banners).toHaveLength(1);
      expect(noticesState().banners[0]?.key).toBe("notice.device.reconnected.output");
      vi.advanceTimersByTime(3999);
      expect(noticesState().banners).toHaveLength(1);
      vi.advanceTimersByTime(1);
      expect(noticesState().banners).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("never lets an expiring banner's timer sweep away a newer banner reusing its id", () => {
    vi.useFakeTimers();
    try {
      pushNotice(banner("notice.device.reconnected.output", 4000));
      // The device drops again before the reconnect banner expired: that banner must stay.
      vi.advanceTimersByTime(3000);
      pushNotice(banner("notice.device.lost.output.playback_stopped", null));
      vi.advanceTimersByTime(60_000);
      expect(noticesState().banners).toHaveLength(1);
      expect(noticesState().banners[0]?.key).toBe("notice.device.lost.output.playback_stopped");
    } finally {
      vi.useRealTimers();
    }
  });
});
