import { describe, expect, it } from "vitest";
import { findDuplicateBindings, matchBinding, SHORTCUTS } from "./registry";

/** H-37 (SPEC-003 §2.5 amendment): Ctrl/⌘+L toggles loop playback (Audition's default). */
describe("the loop playback shortcut", () => {
  const ev = (mods: { ctrl?: boolean; meta?: boolean; shift?: boolean }) => ({
    code: "KeyL",
    ctrlKey: Boolean(mods.ctrl),
    metaKey: Boolean(mods.meta),
    shiftKey: Boolean(mods.shift),
    altKey: false,
  });

  it("is Ctrl+L off macOS and ⌘+L on macOS, global scope", () => {
    expect(matchBinding(ev({ ctrl: true }), false)).toBe("transport.toggle_loop");
    expect(matchBinding(ev({ meta: true }), true)).toBe("transport.toggle_loop");
    expect(matchBinding(ev({}), false)).toBeNull();
    const def = SHORTCUTS.find((s) => s.action === "transport.toggle_loop");
    expect(def).toMatchObject({ code: "KeyL", mod: true, scope: "global", labelKey: "shortcut.transport.toggle_loop" });
    expect(findDuplicateBindings(SHORTCUTS)).toEqual([]);
  });
});
