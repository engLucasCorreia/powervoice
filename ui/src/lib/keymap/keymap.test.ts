import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_KEYMAP } from "./bindings";
import {
  attachKeymap,
  clearActionHandlers,
  dispatchAction,
  isEditableTarget,
  registerAction,
} from "./listener";
import { findDuplicateBindings, isPlatformMac, matchBinding } from "./registry";

function key(
  code: string,
  mods: Partial<{ shift: boolean; ctrl: boolean; meta: boolean; alt: boolean }> = {}
) {
  return {
    code,
    shiftKey: Boolean(mods.shift),
    ctrlKey: Boolean(mods.ctrl),
    metaKey: Boolean(mods.meta),
    altKey: Boolean(mods.alt),
  };
}

describe("default keymap bindings", () => {
  it("resolves every default binding to its action (non-mac: Ctrl as the primary modifier)", () => {
    expect(matchBinding(key("Space"), false)).toBe("transport.play_pause");
    expect(matchBinding(key("Space", { shift: true }), false)).toBe("transport.play_from_start");
    expect(matchBinding(key("Home"), false)).toBe("transport.return_to_start");
    expect(matchBinding(key("KeyR", { shift: true }), false)).toBe("record.toggle");
    expect(matchBinding(key("KeyM"), false)).toBe("marker.add");
    expect(matchBinding(key("KeyZ", { ctrl: true }), false)).toBe("history.undo");
    expect(matchBinding(key("KeyZ", { ctrl: true, shift: true }), false)).toBe("history.redo");
  });

  // S2-01, SPEC-008 §2.11: Ctrl/⌘+X/C/V, Delete, Ctrl/⌘+T; SPEC-006 §2.9: Ctrl/⌘+A, Esc.
  it("resolves the S2-01 edit bindings (non-mac: Ctrl as the primary modifier)", () => {
    expect(matchBinding(key("KeyX", { ctrl: true }), false)).toBe("edit.cut");
    expect(matchBinding(key("KeyC", { ctrl: true }), false)).toBe("edit.copy");
    expect(matchBinding(key("KeyV", { ctrl: true }), false)).toBe("edit.paste");
    expect(matchBinding(key("Delete"), false)).toBe("edit.delete");
    expect(matchBinding(key("KeyT", { ctrl: true }), false)).toBe("edit.trim");
    expect(matchBinding(key("KeyA", { ctrl: true }), false)).toBe("waveform.select_all");
    expect(matchBinding(key("Escape"), false)).toBe("waveform.deselect");
  });

  it("resolves the S2-01 edit bindings on mac (⌘ as the primary modifier)", () => {
    expect(matchBinding(key("KeyX", { meta: true }), true)).toBe("edit.cut");
    expect(matchBinding(key("KeyC", { meta: true }), true)).toBe("edit.copy");
    expect(matchBinding(key("KeyV", { meta: true }), true)).toBe("edit.paste");
    expect(matchBinding(key("KeyT", { meta: true }), true)).toBe("edit.trim");
    expect(matchBinding(key("KeyA", { meta: true }), true)).toBe("waveform.select_all");
    // The "other" platform's modifier must not satisfy a mod binding on mac either.
    expect(matchBinding(key("KeyX", { ctrl: true }), true)).toBeNull();
  });

  it("Silence has no default binding (SPEC-008 §2.11: menu only)", () => {
    expect(DEFAULT_KEYMAP.some((b) => (b.action as string) === "edit.silence")).toBe(false);
  });

  // S3-06, SPEC-014 §2.3: Shift+P captures a noise print (provisional, SPEC-019).
  it("resolves Shift+P to nr.capture_noise_print, distinct from plain P", () => {
    expect(matchBinding(key("KeyP", { shift: true }), false)).toBe("nr.capture_noise_print");
    expect(matchBinding(key("KeyP"), false)).toBeNull();
  });

  it("Shift+Space is not the same action as plain Space", () => {
    const space = matchBinding(key("Space"), false);
    const shiftSpace = matchBinding(key("Space", { shift: true }), false);
    expect(space).not.toBe(shiftSpace);
    expect(space).toBe("transport.play_pause");
    expect(shiftSpace).toBe("transport.play_from_start");
  });

  it("maps Ctrl on non-mac and ⌘ on mac to the same 'mod' bindings", () => {
    expect(matchBinding(key("KeyZ", { ctrl: true }), false)).toBe("history.undo");
    expect(matchBinding(key("KeyZ", { meta: true }), true)).toBe("history.undo");
    // The "other" platform's modifier must not satisfy the binding.
    expect(matchBinding(key("KeyZ", { meta: true }), false)).toBeNull();
    expect(matchBinding(key("KeyZ", { ctrl: true }), true)).toBeNull();
  });

  it("redo requires both mod and shift together on both platforms", () => {
    expect(matchBinding(key("KeyZ", { ctrl: true, shift: true }), false)).toBe("history.redo");
    expect(matchBinding(key("KeyZ", { meta: true, shift: true }), true)).toBe("history.redo");
    expect(matchBinding(key("KeyZ", { ctrl: true }), false)).toBe("history.undo");
  });

  it("an unbound key resolves to null", () => {
    expect(matchBinding(key("KeyQ"), false)).toBeNull();
  });

  it("a held Alt never matches a plain (non-alt) default binding", () => {
    expect(matchBinding(key("Space", { alt: true }), false)).toBeNull();
    expect(matchBinding(key("KeyM", { alt: true }), false)).toBeNull();
  });

  // S2-03, SPEC-009 §2.6/§2.7: Ctrl+0 delete selected marker(s), Ctrl+Alt+arrow navigation.
  it("resolves the S2-03 marker bindings (non-mac: Ctrl as the primary modifier)", () => {
    expect(matchBinding(key("Digit0", { ctrl: true }), false)).toBe("marker.delete_selected");
    expect(matchBinding(key("ArrowRight", { ctrl: true, alt: true }), false)).toBe("marker.next");
    expect(matchBinding(key("ArrowLeft", { ctrl: true, alt: true }), false)).toBe("marker.prev");
    // Alt alone, or mod alone, doesn't satisfy a binding that needs both.
    expect(matchBinding(key("ArrowRight", { alt: true }), false)).toBeNull();
    expect(matchBinding(key("ArrowRight", { ctrl: true }), false)).toBeNull();
  });

  it("resolves the S2-03 marker bindings on mac (⌘ as the primary modifier)", () => {
    expect(matchBinding(key("Digit0", { meta: true }), true)).toBe("marker.delete_selected");
    expect(matchBinding(key("ArrowRight", { meta: true, alt: true }), true)).toBe("marker.next");
    expect(matchBinding(key("ArrowLeft", { meta: true, alt: true }), true)).toBe("marker.prev");
  });

  it("every default binding is unique by (code, shift, mod)", () => {
    expect(findDuplicateBindings(DEFAULT_KEYMAP)).toEqual([]);
  });

  it("detects a duplicate binding when given a deliberately colliding table", () => {
    const withDuplicate = [...DEFAULT_KEYMAP, { action: "marker.add" as const, code: "Space" }];
    expect(findDuplicateBindings(withDuplicate)).toEqual(["Space|shift=false|mod=false|alt=false"]);
  });
});

describe("isPlatformMac", () => {
  it("recognizes common macOS platform strings", () => {
    expect(isPlatformMac("MacIntel")).toBe(true);
    expect(isPlatformMac("Mac68K")).toBe(true);
  });

  it("does not flag Windows/Linux platform strings", () => {
    expect(isPlatformMac("Win32")).toBe(false);
    expect(isPlatformMac("Linux x86_64")).toBe(false);
  });
});

describe("isEditableTarget", () => {
  it("is true for a text input, a textarea, and a contenteditable element", () => {
    const input = document.createElement("input");
    input.type = "text";
    expect(isEditableTarget(input)).toBe(true);

    const textarea = document.createElement("textarea");
    expect(isEditableTarget(textarea)).toBe(true);

    const div = document.createElement("div");
    // jsdom's `contentEditable` IDL setter doesn't reflect to the attribute (a known jsdom
    // limitation), so set the attribute directly, which is what `isEditableTarget` checks.
    div.setAttribute("contenteditable", "true");
    expect(isEditableTarget(div)).toBe(true);
  });

  it("is false for a non-text input, a button, and null", () => {
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    expect(isEditableTarget(checkbox)).toBe(false);

    const button = document.createElement("button");
    expect(isEditableTarget(button)).toBe(false);

    expect(isEditableTarget(null)).toBe(false);
  });
});

describe("attachKeymap + dispatch", () => {
  afterEach(() => {
    clearActionHandlers();
  });

  it("dispatches the matched action to its registered handler", () => {
    const detach = attachKeymap(window, { isMac: false });
    const handler = vi.fn();
    const unregister = registerAction("marker.add", handler);

    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyM", bubbles: true }));
    expect(handler).toHaveBeenCalledTimes(1);

    unregister();
    detach();
  });

  it("is a no-op when no handler is registered for the matched action", () => {
    const detach = attachKeymap(window, { isMac: false });
    // No handler registered for "history.undo" — dispatching directly must not throw.
    expect(() => dispatchAction("history.undo")).not.toThrow();
    window.dispatchEvent(
      new KeyboardEvent("keydown", { code: "KeyZ", ctrlKey: true, bubbles: true })
    );
    detach();
  });

  it("ignores key events while a text input has focus", () => {
    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    const detach = attachKeymap(window, { isMac: false });
    const handler = vi.fn();
    const unregister = registerAction("marker.add", handler);

    input.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyM", bubbles: true }));
    expect(handler).not.toHaveBeenCalled();

    unregister();
    detach();
    input.remove();
  });

  it("does not ignore key events while a non-editable element has focus", () => {
    const button = document.createElement("button");
    document.body.appendChild(button);
    button.focus();

    const detach = attachKeymap(window, { isMac: false });
    const handler = vi.fn();
    const unregister = registerAction("marker.add", handler);

    button.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyM", bubbles: true }));
    expect(handler).toHaveBeenCalledTimes(1);

    unregister();
    detach();
    button.remove();
  });

  // S2-01, AC-19-style: Ctrl+X/C/V/T and Delete dispatch, and none of them fire with a text
  // input focused (native cut/copy/paste/delete keep their text meaning there).
  it("dispatches Ctrl+X/C/V, Delete and Ctrl+T to their registered handlers", () => {
    const detach = attachKeymap(window, { isMac: false });
    const handlers = {
      cut: vi.fn(),
      copy: vi.fn(),
      paste: vi.fn(),
      del: vi.fn(),
      trim: vi.fn(),
    };
    const unregisters = [
      registerAction("edit.cut", handlers.cut),
      registerAction("edit.copy", handlers.copy),
      registerAction("edit.paste", handlers.paste),
      registerAction("edit.delete", handlers.del),
      registerAction("edit.trim", handlers.trim),
    ];

    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyX", ctrlKey: true, bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyC", ctrlKey: true, bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyV", ctrlKey: true, bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Delete", bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyT", ctrlKey: true, bubbles: true }));

    expect(handlers.cut).toHaveBeenCalledTimes(1);
    expect(handlers.copy).toHaveBeenCalledTimes(1);
    expect(handlers.paste).toHaveBeenCalledTimes(1);
    expect(handlers.del).toHaveBeenCalledTimes(1);
    expect(handlers.trim).toHaveBeenCalledTimes(1);

    for (const unregister of unregisters) {
      unregister();
    }
    detach();
  });

  it("ignores Ctrl+X/C/V/Delete while a text input has focus (native text editing)", () => {
    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    const detach = attachKeymap(window, { isMac: false });
    const handler = vi.fn();
    const unregister = registerAction("edit.cut", handler);

    input.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyX", ctrlKey: true, bubbles: true }));
    expect(handler).not.toHaveBeenCalled();

    unregister();
    detach();
    input.remove();
  });
});
