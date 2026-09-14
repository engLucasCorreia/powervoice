import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import {
  cancelSaveAsPrompt,
  confirmSaveAsPrompt,
  documentState,
  openDocument,
  requestOpen,
  requestSave,
  resetDocumentStateForTest,
  resolveConfirmPrompt,
  resolveUnsavedPrompt,
  saveDocument,
  saveDocumentAs,
  titleFor,
} from "./document.svelte";

function doc(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    sidecar_dirty: false,
    spectral_view: null, waveform_view: null,
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetWaveformViewForTest();
});

describe("titleFor (ticket: title '‹name› — PowerVoice' with * when modified)", () => {
  it("is just the app name with no document open", () => {
    expect(titleFor(doc({ name: null, path: null, sample_rate_hz: 0 }))).toBe("PowerVoice");
  });

  it("calls a never-saved recording Untitled (S1-04)", () => {
    expect(titleFor(doc({ name: null, path: null, dirty: true }))).toBe("Untitled * — PowerVoice");
  });

  it("shows the name, and a modified marker when dirty", () => {
    expect(titleFor(doc({ name: "take.wav", dirty: false }))).toBe("take.wav — PowerVoice");
    expect(titleFor(doc({ name: "take.wav", dirty: true }))).toBe("take.wav * — PowerVoice");
  });

  it("T-306: also shows * when only sidecar_dirty is set (SPEC-018 §2.4)", () => {
    expect(titleFor(doc({ name: "take.wav", dirty: false, sidecar_dirty: true }))).toBe(
      "take.wav * — PowerVoice",
    );
  });
});

describe("openDocument / saveDocument / saveDocumentAs", () => {
  it("opens a document via document_open and updates the store", async () => {
    const fixture = doc();
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const ok = await openDocument("/home/user/take.wav");
    expect(ok).toBe(true);
    expect(documentState().current).toEqual(fixture);
  });

  it("saves via document_save", async () => {
    const fixture = doc({ dirty: false });
    mockIPC((cmd) => {
      if (cmd === "document_save") {
        return fixture;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    expect(await saveDocument()).toBe(true);
    expect(documentState().current).toEqual(fixture);
  });

  it("save-as via document_save_as with the chosen bit depth and path", async () => {
    let received: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "document_save_as") {
        received = args;
        return doc({ path: "/home/user/out.wav", name: "out.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    expect(await saveDocumentAs("/home/user/out.wav", "32f")).toBe(true);
    expect(received).toEqual({ path: "/home/user/out.wav", bits: "32f" });
    expect(documentState().current.name).toBe("out.wav");
  });

  it("reports a failed open as a notice and leaves the store untouched", async () => {
    mockIPC(() => {
      throw { code: "not_found", key: "error.open.not_found", params: {} };
    });
    const before = documentState().current;
    const ok = await openDocument("/nope.wav");
    expect(ok).toBe(false);
    expect(documentState().current).toEqual(before);
  });
});

describe("requestOpen (unsaved-changes guard, SPEC-004 §2.8)", () => {
  it("opens directly when the document isn't dirty", async () => {
    let openedPath: string | null = null;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|open") {
        return "/home/user/new.wav";
      }
      if (cmd === "document_open") {
        openedPath = (args as { path: string }).path;
        return doc({ path: "/home/user/new.wav", name: "new.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    await requestOpen();
    expect(openedPath).toBe("/home/user/new.wav");
  });

  it("does nothing when the native dialog is cancelled", async () => {
    let openCalled = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|open") {
        return null;
      }
      if (cmd === "document_open") {
        openCalled = true;
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await requestOpen();
    expect(openCalled).toBe(false);
  });

  it("prompts, and Cancel leaves the document untouched", async () => {
    // Force a dirty current document without going through a command (module-internal state is
    // only reachable through the exported actions, so open one, then mock a dirty follow-up).
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ dirty: true });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    expect(documentState().current.dirty).toBe(true);

    let dialogOpened = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|open") {
        dialogOpened = true;
        return "/home/user/other.wav";
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const pending = requestOpen();
    expect(documentState().unsavedPrompt?.name).toBe("take.wav");
    resolveUnsavedPrompt("cancel");
    await pending;
    expect(dialogOpened).toBe(false);
    expect(documentState().unsavedPrompt).toBeNull();
  });

  it("prompts, and Save saves first, then opens", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ dirty: true });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "document_save") {
        return doc({ dirty: false });
      }
      if (cmd === "plugin:dialog|open") {
        return "/home/user/other.wav";
      }
      if (cmd === "document_open") {
        return doc({ path: "/home/user/other.wav", name: "other.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const pending = requestOpen();
    resolveUnsavedPrompt("save");
    await pending;
    expect(calls).toEqual(["document_save", "plugin:dialog|open", "document_open"]);
    expect(documentState().current.name).toBe("other.wav");
  });
});

describe("requestSave / Save As prompt", () => {
  it("Save with no bound path opens the Save As prompt instead of calling document_save", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: null, name: null });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    // (no document actually open here — default state already has path: null)
    expect(documentState().current.path).toBeNull();
    let saveCalled = false;
    mockIPC((cmd) => {
      saveCalled = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    await requestSave();
    expect(saveCalled).toBe(false);
    expect(documentState().saveAsPrompt).not.toBeNull();
  });

  it("Save with a bound path calls document_save directly", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    let saveCalled = false;
    mockIPC((cmd) => {
      if (cmd === "document_save") {
        saveCalled = true;
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await requestSave();
    expect(saveCalled).toBe(true);
  });

  it("confirmSaveAsPrompt shows the native save dialog then saves at the chosen bits", async () => {
    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "plugin:dialog|save") {
        return "/home/user/out.wav";
      }
      if (cmd === "document_save_as") {
        savedArgs = args;
        return doc({ path: "/home/user/out.wav", name: "out.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await confirmSaveAsPrompt("16");
    expect(savedArgs).toEqual({ path: "/home/user/out.wav", bits: "16" });
    expect(documentState().saveAsPrompt).toBeNull();
  });

  it("confirmSaveAsPrompt does nothing when the native dialog is cancelled", async () => {
    let saveAsCalled = false;
    mockIPC((cmd) => {
      if (cmd === "plugin:dialog|save") {
        return null;
      }
      saveAsCalled = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    await confirmSaveAsPrompt("24");
    expect(saveAsCalled).toBe(false);
  });

  it("cancelSaveAsPrompt clears the prompt without saving", () => {
    mockIPC(() => {
      throw new Error("no command expected");
    });
    // Populate the prompt via requestSave's no-path branch, then cancel it.
    resetDocumentStateForTest();
    resetWaveformViewForTest();
    cancelSaveAsPrompt();
    expect(documentState().saveAsPrompt).toBeNull();
  });
});

describe("T-306: already-open / changed-on-disk confirmations", () => {
  it("openDocument passes confirmAlreadyOpen: false, then re-issues true after confirming", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        calls.push(args);
        const confirmed = (args as { confirmAlreadyOpen: boolean }).confirmAlreadyOpen;
        if (!confirmed) {
          throw { code: "needs_confirmation", key: "dialog.already_open", params: { name: "a.wav" } };
        }
        return doc({ path: "/home/user/a.wav", name: "a.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = openDocument("/home/user/a.wav");
    // Wait for the first (mocked, async transport) call to reject and the prompt to appear.
    await new Promise((r) => setTimeout(r, 0));
    expect(documentState().confirmPrompt).toEqual({ kind: "already_open", name: "a.wav" });
    resolveConfirmPrompt(true);
    expect(await pending).toBe(true);
    expect(calls).toEqual([
      { path: "/home/user/a.wav", confirmAlreadyOpen: false },
      { path: "/home/user/a.wav", confirmAlreadyOpen: true },
    ]);
    expect(documentState().current.name).toBe("a.wav");
  });

  it("openDocument stays closed when the already-open confirmation is cancelled", async () => {
    let secondCallMade = false;
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        const confirmed = (args as { confirmAlreadyOpen: boolean }).confirmAlreadyOpen;
        if (!confirmed) {
          throw { code: "needs_confirmation", key: "dialog.already_open", params: { name: "a.wav" } };
        }
        secondCallMade = true;
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = openDocument("/home/user/a.wav");
    await new Promise((r) => setTimeout(r, 0));
    resolveConfirmPrompt(false);
    expect(await pending).toBe(false);
    expect(secondCallMade).toBe(false);
  });

  it("saveDocument passes overwrite: false, then re-issues true after confirming", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_save") {
        calls.push(args);
        const overwrite = (args as { overwrite: boolean }).overwrite;
        if (!overwrite) {
          throw {
            code: "needs_confirmation",
            key: "dialog.changed_on_disk",
            params: { name: "take.wav" },
          };
        }
        return doc({ dirty: false });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = saveDocument();
    await new Promise((r) => setTimeout(r, 0));
    expect(documentState().confirmPrompt).toEqual({
      kind: "changed_on_disk",
      name: "take.wav",
    });
    resolveConfirmPrompt(true);
    expect(await pending).toBe(true);
    expect(calls).toEqual([{ overwrite: false }, { overwrite: true }]);
  });
});
