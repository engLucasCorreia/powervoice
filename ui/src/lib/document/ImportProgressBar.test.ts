import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearNotices } from "../state/notices.svelte";
import {
  applyImportJobProgress,
  applyImportStarted,
  dismissImportJob,
  documentState,
  resetDocumentStateForTest,
} from "./document.svelte";
import ImportProgressBar from "./ImportProgressBar.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
});

describe("ImportProgressBar (H-20, SPEC-005 §2.3 — the simpler document-shell design)", () => {
  it("is hidden with no import running", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ImportProgressBar, { target });
    flushSync();
    expect(target.querySelector('[data-testid="import-progress-bar"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("shows the document shell (name, known length, percent) once import_started arrives", () => {
    applyImportStarted({
      job_id: 1,
      name: "podcast.wav",
      sample_rate_hz: 48_000,
      len_samples: 48_000,
    });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ImportProgressBar, { target });
    flushSync();

    const bar = target.querySelector('[data-testid="import-progress-bar"]');
    expect(bar).not.toBeNull();
    expect(bar!.textContent).toContain("podcast.wav");
    expect(bar!.textContent).toContain("0 %");
    expect(target.querySelector('[data-testid="import-progress-length"]')!.textContent).toBe(
      "00:00:01.000",
    );

    applyImportJobProgress({ job_id: 1, kind: "import", state: "running", fraction: 0.5 });
    flushSync();
    expect(target.querySelector('[data-testid="import-progress-bar"]')!.textContent).toContain(
      "50 %",
    );

    unmount(app);
    target.remove();
  });

  it("omits the length span when the container states no sample count", () => {
    applyImportStarted({ job_id: 2, name: "stream.mp3", sample_rate_hz: 44_100, len_samples: null });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ImportProgressBar, { target });
    flushSync();

    expect(target.querySelector('[data-testid="import-progress-length"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Cancel calls document_open_cancel with the running job's id", async () => {
    applyImportStarted({ job_id: 7, name: "take.wav", sample_rate_hz: 48_000, len_samples: 96_000 });
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "document_open_cancel") {
        calls.push(args);
        return undefined;
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ImportProgressBar, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="import-progress-cancel"]')!.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toEqual([{ jobId: 7 }]);

    unmount(app);
    target.remove();
  });

  it("disappears once the job is no longer running (dismissed automatically)", () => {
    applyImportStarted({ job_id: 3, name: "take.wav", sample_rate_hz: 48_000, len_samples: 48_000 });
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ImportProgressBar, { target });
    flushSync();
    expect(target.querySelector('[data-testid="import-progress-bar"]')).not.toBeNull();

    applyImportJobProgress({ job_id: 3, kind: "import", state: "done", fraction: 1 });
    flushSync();
    expect(target.querySelector('[data-testid="import-progress-bar"]')).toBeNull();
    expect(documentState().importJob).toBeNull();

    unmount(app);
    target.remove();
  });

  it("dismissImportJob is idempotent with nothing showing", () => {
    expect(() => dismissImportJob()).not.toThrow();
  });
});
