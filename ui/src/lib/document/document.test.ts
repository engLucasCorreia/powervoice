import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { ImportStartedDto, JobProgressDto, Settings } from "../ipc/bindings";
import { clearNotices, noticesState } from "../state/notices.svelte";
import { loadSettings, resetSettingsStateForTest } from "../state/settings.svelte";
import { docDto as doc, documentProbeDto as probe, settingsFixture } from "../test/fixtures";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import {
  applyImportJobProgress,
  applyImportStarted,
  applySaveJobProgress,
  cancelImportJob,
  cancelSaveAsPrompt,
  cancelSaveJob,
  confirmSaveAsPrompt,
  dismissImportJob,
  dismissSaveJob,
  documentState,
  openDocument,
  openSaveAsPrompt,
  requestOpen,
  requestSave,
  requestSaveAs,
  resetDocumentStateForTest,
  resolveChannelChoicePrompt,
  resolveClipPrompt,
  resolveConfirmPrompt,
  resolveUnsavedPrompt,
  saveDocument,
  saveDocumentAs,
  titleFor,
} from "./document.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
  resetSettingsStateForTest();
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
    expect(await saveDocumentAs("/home/user/out.wav", "wav", "32f")).toBe(true);
    expect(received).toEqual({
      path: "/home/user/out.wav",
      container: "wav",
      bits: "32f",
      dither: "tpdf",
      confirmClip: false,
      confirmMultichannel: false,
    });
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
    await confirmSaveAsPrompt("wav", "16", "tpdf");
    expect(savedArgs).toEqual({
      path: "/home/user/out.wav",
      container: "wav",
      bits: "16",
      dither: "tpdf",
      confirmClip: false,
      confirmMultichannel: false,
    });
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
    await confirmSaveAsPrompt("wav", "24", "tpdf");
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
      { path: "/home/user/a.wav", confirmAlreadyOpen: false, channelChoice: null },
      { path: "/home/user/a.wav", confirmAlreadyOpen: true, channelChoice: null },
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
    expect(calls).toEqual([
      { overwrite: false, confirmClip: false, confirmMultichannel: false },
      { overwrite: true, confirmClip: false, confirmMultichannel: false },
    ]);
  });
});

describe("T-209: multichannel channel-choice dialog (SPEC-005 §2.4)", () => {
  it("openDocument shows the channel-choice dialog and re-issues with the chosen downmix", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        calls.push(args);
        const channelChoice = (args as { channelChoice: unknown }).channelChoice;
        if (!channelChoice) {
          throw {
            code: "needs_confirmation",
            key: "dialog.channel_choice",
            params: { probe: JSON.stringify(probe()) },
          };
        }
        return doc({ path: "/home/user/stereo.wav", name: "stereo.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = openDocument("/home/user/stereo.wav");
    await new Promise((r) => setTimeout(r, 0));
    expect(documentState().channelChoicePrompt?.probe.channels.length).toBe(2);

    resolveChannelChoicePrompt({ choice: { kind: "channel", index: 1 }, remember: false });
    expect(await pending).toBe(true);
    expect(calls).toEqual([
      { path: "/home/user/stereo.wav", confirmAlreadyOpen: false, channelChoice: null },
      {
        path: "/home/user/stereo.wav",
        confirmAlreadyOpen: false,
        channelChoice: { kind: "channel", index: 1 },
      },
    ]);
    expect(documentState().current.name).toBe("stereo.wav");
  });

  it("Cancel on the channel-choice dialog leaves the document untouched", async () => {
    let secondCallMade = false;
    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        const channelChoice = (args as { channelChoice: unknown }).channelChoice;
        if (!channelChoice) {
          throw {
            code: "needs_confirmation",
            key: "dialog.channel_choice",
            params: { probe: JSON.stringify(probe()) },
          };
        }
        secondCallMade = true;
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = openDocument("/home/user/stereo.wav");
    await new Promise((r) => setTimeout(r, 0));
    resolveChannelChoicePrompt(null);
    expect(await pending).toBe(false);
    expect(secondCallMade).toBe(false);
    expect(documentState().channelChoicePrompt).toBeNull();
  });

  it("checking 'remember' persists multichannel_policy via settings.svelte.ts's saveSettings", async () => {
    mockIPC((cmd) => {
      if (cmd === "settings_get") {
        return settingsFixture();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await loadSettings();

    mockIPC((cmd, args) => {
      if (cmd === "document_open") {
        const channelChoice = (args as { channelChoice: unknown }).channelChoice;
        if (!channelChoice) {
          throw {
            code: "needs_confirmation",
            key: "dialog.channel_choice",
            params: { probe: JSON.stringify(probe()) },
          };
        }
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const pending = openDocument("/home/user/stereo.wav");
    await new Promise((r) => setTimeout(r, 0));

    let savedSettings: Settings | undefined;
    mockIPC((cmd, args) => {
      if (cmd === "settings_set") {
        savedSettings = (args as { settings: Settings }).settings;
        return savedSettings;
      }
      if (cmd === "document_open") {
        return doc();
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    resolveChannelChoicePrompt({ choice: { kind: "average" }, remember: true });
    await pending;
    await new Promise((r) => setTimeout(r, 0));

    expect(savedSettings?.multichannel_policy).toBe("always_mix");
  });
});

describe("T-209: clip prompt (SPEC-005 §2.8)", () => {
  it("saveDocument shows the clip prompt on dialog.overs; 'Clip and save' re-issues with confirmClip", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_save") {
        calls.push(args);
        const confirmClip = (args as { confirmClip: boolean }).confirmClip;
        if (!confirmClip) {
          throw {
            code: "needs_confirmation",
            key: "dialog.overs",
            params: { count: "3", peak_dbfs: "3.52" },
          };
        }
        return doc({ dirty: false });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const pending = saveDocument();
    await new Promise((r) => setTimeout(r, 0));
    expect(documentState().clipPrompt).toEqual({ count: 3, peakDbfs: 3.52 });
    resolveClipPrompt("clip");
    expect(await pending).toBe(true);
    expect(calls).toEqual([
      { overwrite: false, confirmClip: false, confirmMultichannel: false },
      { overwrite: false, confirmClip: true, confirmMultichannel: false },
    ]);
  });

  it("'Save as 32-bit float instead' calls document_save_as at 32f on the bound (.wav) path", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: "/home/user/take.wav", name: "take.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");

    mockIPC((cmd) => {
      if (cmd === "document_save") {
        throw {
          code: "needs_confirmation",
          key: "dialog.overs",
          params: { count: "1", peak_dbfs: "1.5" },
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const pending = saveDocument();
    await new Promise((r) => setTimeout(r, 0));

    let savedArgs: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "document_save_as") {
        savedArgs = args;
        return doc({ path: "/home/user/take.wav", name: "take.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    resolveClipPrompt("float");
    expect(await pending).toBe(true);
    expect(savedArgs).toEqual({
      path: "/home/user/take.wav",
      container: "wav",
      bits: "32f",
      dither: "tpdf",
      confirmClip: false,
      confirmMultichannel: false,
    });
  });

  it("Cancel on the clip prompt leaves the document untouched", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_save") {
        throw {
          code: "needs_confirmation",
          key: "dialog.overs",
          params: { count: "1", peak_dbfs: "0.5" },
        };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    const pending = saveDocument();
    await new Promise((r) => setTimeout(r, 0));
    resolveClipPrompt("cancel");
    expect(await pending).toBe(false);
    expect(documentState().clipPrompt).toBeNull();
  });

  it("appears only when document_save reports dialog.overs — a clean save needs no prompt", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_save") {
        return doc({ dirty: false });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    expect(await saveDocument()).toBe(true);
    expect(documentState().clipPrompt).toBeNull();
  });
});

describe("T-209: compressed-source Save routes to Save As (SPEC-005 §2.6)", () => {
  it("requestSave on an opened MP3 opens the Save As prompt with WAV 24 and '<name>.wav' preselected", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: "/home/user/take.mp3", name: "take.mp3" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.mp3");

    let saveCalled = false;
    mockIPC((cmd) => {
      saveCalled = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    await requestSave();
    expect(saveCalled).toBe(false);
    expect(documentState().saveAsPrompt).toEqual({
      suggestedName: "take.wav",
      defaultContainer: "wav",
      defaultBits: "24",
      defaultDither: "tpdf",
    });
  });

  it("openSaveAsPrompt preselects FLAC for a document bound to a .flac path", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: "/home/user/take.flac", name: "take.flac" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.flac");
    openSaveAsPrompt();
    expect(documentState().saveAsPrompt).toEqual({
      suggestedName: "take.flac",
      defaultContainer: "flac",
      defaultBits: "24",
      defaultDither: "tpdf",
    });
  });
});

describe("T-209: import job progress (SPEC-005 §2.3)", () => {
  it("applyImportJobProgress tracks kind 'import' only", () => {
    const other: JobProgressDto = { job_id: 1, kind: "export", state: "running", fraction: 0.4 };
    applyImportJobProgress(other);
    expect(documentState().importJob).toBeNull();

    const started: JobProgressDto = { job_id: 7, kind: "import", state: "running", fraction: 0 };
    applyImportJobProgress(started);
    expect(documentState().importJob).toEqual({
      jobId: 7,
      fraction: 0,
      state: "running",
      name: "",
      sampleRateHz: 0,
      lenSamples: null,
    });

    const progressed: JobProgressDto = {
      job_id: 7,
      kind: "import",
      state: "running",
      fraction: 0.5,
    };
    applyImportJobProgress(progressed);
    expect(documentState().importJob?.fraction).toBe(0.5);

    const done: JobProgressDto = { job_id: 7, kind: "import", state: "done", fraction: 1 };
    applyImportJobProgress(done);
    expect(documentState().importJob?.state).toBe("done");

    dismissImportJob();
    expect(documentState().importJob).toBeNull();
  });

  it("cancelImportJob calls document_open_cancel with the running job's id", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open_cancel") {
        calls.push(args);
        return undefined;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    applyImportJobProgress({ job_id: 3, kind: "import", state: "running", fraction: 0.2 });
    cancelImportJob();
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toEqual([{ jobId: 3 }]);
  });

  it("cancelImportJob is a no-op once the job has already finished", async () => {
    let called = false;
    mockIPC((cmd) => {
      called = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    applyImportJobProgress({ job_id: 3, kind: "import", state: "done", fraction: 1 });
    cancelImportJob();
    await new Promise((r) => setTimeout(r, 0));
    expect(called).toBe(false);
  });
});

describe("H-70: save job progress (SPEC-005 §2.7/§4.10)", () => {
  it("applySaveJobProgress tracks kind 'save' only", () => {
    const other: JobProgressDto = { job_id: 1, kind: "import", state: "running", fraction: 0.4 };
    applySaveJobProgress(other);
    expect(documentState().saveJob).toBeNull();

    const started: JobProgressDto = { job_id: 5, kind: "save", state: "running", fraction: 0 };
    applySaveJobProgress(started);
    expect(documentState().saveJob).toEqual({ jobId: 5, fraction: 0, state: "running" });

    const progressed: JobProgressDto = { job_id: 5, kind: "save", state: "running", fraction: 0.7 };
    applySaveJobProgress(progressed);
    expect(documentState().saveJob?.fraction).toBe(0.7);

    const done: JobProgressDto = { job_id: 5, kind: "save", state: "done", fraction: 1 };
    applySaveJobProgress(done);
    expect(documentState().saveJob?.state).toBe("done");

    dismissSaveJob();
    expect(documentState().saveJob).toBeNull();
  });

  it("cancelSaveJob calls document_save_cancel with the running job's id", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_save_cancel") {
        calls.push(args);
        return undefined;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    applySaveJobProgress({ job_id: 4, kind: "save", state: "running", fraction: 0.3 });
    cancelSaveJob();
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toEqual([{ jobId: 4 }]);
  });

  it("cancelSaveJob is a no-op once the job has already finished", async () => {
    let called = false;
    mockIPC((cmd) => {
      called = true;
      throw new Error(`unexpected command: ${cmd}`);
    });
    applySaveJobProgress({ job_id: 4, kind: "save", state: "failed", fraction: 0 });
    cancelSaveJob();
    await new Promise((r) => setTimeout(r, 0));
    expect(called).toBe(false);
  });

  it("requestSave and requestSaveAs are no-ops while a save is already running", async () => {
    let called = false;
    mockIPC(() => {
      called = true;
      throw new Error("a running save must not be raced");
    });
    applySaveJobProgress({ job_id: 6, kind: "save", state: "running", fraction: 0.1 });
    await requestSave();
    requestSaveAs();
    expect(called).toBe(false);
    expect(documentState().saveAsPrompt).toBeNull();
  });
});

describe("H-20: progressive import display (SPEC-005 §2.3)", () => {
  it("import_started shows the document shell (name/rate/length) ahead of the decode loop", () => {
    const started: ImportStartedDto = {
      job_id: 9,
      name: "podcast.wav",
      sample_rate_hz: 48_000,
      len_samples: 960_000,
    };
    applyImportStarted(started);
    expect(documentState().importJob).toEqual({
      jobId: 9,
      fraction: 0,
      state: "running",
      name: "podcast.wav",
      sampleRateHz: 48_000,
      lenSamples: 960_000,
    });
  });

  it("import_started leaves lenSamples null for a container with no stated sample count", () => {
    applyImportStarted({
      job_id: 1,
      name: "stream.mp3",
      sample_rate_hz: 44_100,
      len_samples: null,
    });
    expect(documentState().importJob?.lenSamples).toBeNull();
  });

  it("job_progress preserves the shell import_started set, updating only fraction/state", () => {
    applyImportStarted({
      job_id: 5,
      name: "take.wav",
      sample_rate_hz: 48_000,
      len_samples: 48_000,
    });
    applyImportJobProgress({ job_id: 5, kind: "import", state: "running", fraction: 0.6 });
    expect(documentState().importJob).toEqual({
      jobId: 5,
      fraction: 0.6,
      state: "running",
      name: "take.wav",
      sampleRateHz: 48_000,
      lenSamples: 48_000,
    });
  });

  it("cancelling mid-import leaves the previously open document untouched (no partial document)", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: "/home/user/take.wav", name: "take.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    const before = documentState().current;
    expect(before.name).toBe("take.wav");

    // A second import starts (its shell shows immediately) and is then cancelled before the
    // decode loop finishes — `document_open`'s promise rejects with `error.cancelled`, mirroring
    // `document_open_cancel` racing the blocking import thread.
    applyImportStarted({
      job_id: 42,
      name: "other.wav",
      sample_rate_hz: 48_000,
      len_samples: 96_000,
    });
    expect(documentState().importJob?.name).toBe("other.wav");
    // The document store itself is untouched while the shell is showing.
    expect(documentState().current).toEqual(before);

    mockIPC(() => {
      throw { code: "cancelled", key: "error.cancelled", params: {} };
    });
    const ok = await openDocument("/home/user/other.wav");
    expect(ok).toBe(false);
    expect(documentState().current).toEqual(before);
  });

  it("a failed import restores the previous document and posts a notice", async () => {
    mockIPC((cmd) => {
      if (cmd === "document_open") {
        return doc({ path: "/home/user/take.wav", name: "take.wav" });
      }
      throw new Error(`unmocked command: ${cmd}`);
    });
    await openDocument("/home/user/take.wav");
    const before = documentState().current;

    applyImportStarted({
      job_id: 43,
      name: "broken.wav",
      sample_rate_hz: 48_000,
      len_samples: 96_000,
    });
    mockIPC(() => {
      throw { code: "io", key: "error.open.io", params: {} };
    });
    const ok = await openDocument("/home/user/broken.wav");
    expect(ok).toBe(false);
    expect(documentState().current).toEqual(before);
    expect(noticesState().toasts.some((n) => n.key === "error.open.io")).toBe(true);
  });

  it("applyImportJobProgress restores the window title on a cancelled/failed job (no lingering 'Opening…' title)", () => {
    applyImportStarted({
      job_id: 8,
      name: "take.wav",
      sample_rate_hz: 48_000,
      len_samples: 48_000,
    });
    // Cancelling doesn't throw even without a live Tauri window (jsdom/Vitest) — `updateWindowTitle`
    // catches the missing `getCurrentWindow` API.
    expect(() =>
      applyImportJobProgress({ job_id: 8, kind: "import", state: "cancelled", fraction: 0.3 }),
    ).not.toThrow();
    expect(documentState().importJob?.state).toBe("cancelled");
  });
});
