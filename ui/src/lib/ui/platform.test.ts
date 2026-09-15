import { afterEach, describe, expect, it } from "vitest";
import { orderDialogActions, type DialogActionRole } from "./dialogActions";
import { currentPlatform, detectPlatform, setPlatformForTest } from "./platform";

afterEach(() => setPlatformForTest(null));

describe("detectPlatform", () => {
  it("reads the webview's navigator.platform", () => {
    expect(detectPlatform("Win32", "")).toBe("windows");
    expect(detectPlatform("MacIntel", "")).toBe("mac");
    expect(detectPlatform("Linux x86_64", "")).toBe("linux");
  });

  it("falls back to the user agent when the platform is empty", () => {
    expect(detectPlatform("", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
    expect(detectPlatform("", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)")).toBe("mac");
    expect(detectPlatform("", "Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux");
  });

  it("can be overridden by tests", () => {
    setPlatformForTest("windows");
    expect(currentPlatform()).toBe("windows");
    setPlatformForTest(null);
    expect(currentPlatform()).toBe(detectPlatform());
  });
});

describe("orderDialogActions (platform button order)", () => {
  const actions: { id: string; role: DialogActionRole }[] = [
    { id: "acx", role: "utility" },
    { id: "discard", role: "destructive" },
    { id: "cancel", role: "cancel" },
    { id: "float", role: "alternate" },
    { id: "save", role: "primary" },
  ];
  const ids = (list: { id: string }[]) => list.map((a) => a.id);

  it("macOS and Linux: primary last, Cancel just left of it, destructive apart on the left", () => {
    for (const platform of ["mac", "linux"] as const) {
      const { leading, trailing } = orderDialogActions(actions, platform);
      expect(ids(leading), platform).toEqual(["acx", "discard"]);
      expect(ids(trailing), platform).toEqual(["float", "cancel", "save"]);
    }
  });

  it("Windows: primary first, Cancel last, all together on the right", () => {
    const { leading, trailing } = orderDialogActions(actions, "windows");
    expect(ids(leading)).toEqual(["acx"]);
    expect(ids(trailing)).toEqual(["save", "float", "discard", "cancel"]);
  });

  it("a two-button dialog flips between platforms", () => {
    const pair: { id: string; role: DialogActionRole }[] = [
      { id: "cancel", role: "cancel" },
      { id: "ok", role: "primary" },
    ];
    expect(ids(orderDialogActions(pair, "linux").trailing)).toEqual(["cancel", "ok"]);
    expect(ids(orderDialogActions(pair, "windows").trailing)).toEqual(["ok", "cancel"]);
  });

  it("keeps the given order among buttons that share a role", () => {
    const list: { id: string; role: DialogActionRole }[] = [
      { id: "close", role: "cancel" },
      { id: "retry", role: "alternate" },
      { id: "again", role: "alternate" },
    ];
    expect(ids(orderDialogActions(list, "mac").trailing)).toEqual(["retry", "again", "close"]);
  });
});
