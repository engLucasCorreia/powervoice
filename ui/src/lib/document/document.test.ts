import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import {
  cancelSaveAsPrompt,
  confirmSaveAsPrompt,
  documentState,
  openDocument,
  requestOpen,
  requestSave,
  resetDocumentStateForTest,
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
    ...overrides,
  };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
});

describe("titleFor (ticket: title '‹name› — PowerVoice' with * when modified)", () => {
  it("is just the app name with no document open", () => {
    expect(titleFor(doc({ name: null }))).toBe("PowerVoice");
  });

  it("shows the name, and a modified marker when dirty", () => {
    expect(titleFor(doc({ name: "take.wav", dirty: false }))).toBe("take.wav — PowerVoice");
    expect(titleFor(doc({ name: "take.wav", dirty: true }))).toBe("take.wav * — PowerVoice");
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
    cancelSaveAsPrompt();
    expect(documentState().saveAsPrompt).toBeNull();
  });
});
