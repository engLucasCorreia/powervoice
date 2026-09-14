import { describe, expect, it } from "vitest";
import { formatBinding, shortcutLabelForAction } from "./shortcutLabel";

describe("shortcutLabel (H-19)", () => {
  it("formats a plain letter binding for non-mac and mac", () => {
    expect(shortcutLabelForAction("history.undo", false)).toBe("Ctrl+Z");
    expect(shortcutLabelForAction("history.undo", true)).toBe("⌘Z");
  });

  it("formats a mod+shift binding", () => {
    expect(shortcutLabelForAction("history.redo", false)).toBe("Ctrl+Shift+Z");
    expect(shortcutLabelForAction("history.redo", true)).toBe("⇧⌘Z");
  });

  it("formats a mod+alt binding (marker navigation)", () => {
    expect(shortcutLabelForAction("marker.next", false)).toBe("Ctrl+Alt+→");
    expect(shortcutLabelForAction("marker.next", true)).toBe("⌥⌘→");
  });

  it("formats a shift-only binding with a named key", () => {
    expect(shortcutLabelForAction("transport.play_from_start", false)).toBe("Shift+Space");
  });

  it("formats a plain unmodified binding", () => {
    expect(shortcutLabelForAction("marker.add", false)).toBe("M");
    expect(shortcutLabelForAction("waveform.zoom_in", false)).toBe("=");
    expect(shortcutLabelForAction("waveform.deselect", false)).toBe("Esc");
  });

  // Menu-only actions (Silence, the normalize favorites) have no entry in the real registry at
  // all — this exercises the same "not found" branch against a reduced keymap.
  it("returns undefined for an action with no binding in the given keymap", () => {
    expect(shortcutLabelForAction("spectral.toggle", false, [])).toBeUndefined();
  });

  it("formatBinding matches shortcutLabelForAction for the same binding", () => {
    const binding = { action: "history.undo" as const, code: "KeyZ", mod: true };
    expect(formatBinding(binding, false)).toBe(shortcutLabelForAction("history.undo", false));
  });
});
