import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { SpectrumReportDto } from "../ipc/bindings";
import { openDocument, resetDocumentStateForTest } from "../document/document.svelte";
import { docDto } from "../test/fixtures";
import { applySpectrumJobProgress, applySpectrumReport, resetDiagnosticsForTest, setInspectorOpen } from "./diagnostics.svelte";
import { explainModalState, resetExplainModalForTest } from "./explain/explainModal.svelte";
import { explainVoiceState } from "./explain/explainVoice.svelte";
import { balancedReport } from "./explain/voiceFixtures";
import { resetInspectorStreamForTest } from "./inspectorStream.svelte";
import SpectrumInspector from "./SpectrumInspector.svelte";

/** H-42 (SPEC-007 §8.3): the Spectrum Inspector window's lifecycle and controls. */

afterEach(() => {
  clearMocks();
  resetInspectorStreamForTest();
  resetDiagnosticsForTest();
  document.body.innerHTML = "";
});

const settle = async () => {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
};

function recordCalls(): Array<[string, Record<string, unknown>]> {
  const calls: Array<[string, Record<string, unknown>]> = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
    if (cmd === "analyzer_inspector_subscribe") {
      return 42;
    }
    if (cmd === "analyzer_voice_subscribe") {
      return 43;
    }
    return null;
  });
  return calls;
}

describe("SpectrumInspector (H-42)", () => {
  it("opens from the store, streams at its settings, reconfigures, and closes with Escape", async () => {
    const calls = recordCalls();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectrumInspector, { target });
    flushSync();
    expect(target.querySelector('[data-testid="spectrum-inspector"]')).toBeNull();

    setInspectorOpen(true);
    await settle();
    const win = target.querySelector<HTMLElement>('[data-testid="spectrum-inspector"]')!;
    expect(win).not.toBeNull();
    expect(win.getAttribute("role")).toBe("dialog");
    expect(win.getAttribute("aria-modal")).toBe("false");
    const sub = calls.find(([c]) => c === "analyzer_inspector_subscribe")!;
    expect(sub[1].config).toEqual({ fft_size: 16_384, window: "hann", response: "medium" });
    expect(calls.some(([c]) => c === "analyzer_voice_subscribe")).toBe(true);
    expect(target.querySelector('[data-testid="inspector-resolution"]')!.textContent).toContain("Hz per bin");

    // FFT size → the same stream is reconfigured, not re-subscribed.
    const fft = target.querySelector<HTMLSelectElement>('select[data-testid="inspector-fft"], [data-testid="inspector-fft"] select')!;
    fft.value = [...fft.options].find((o) => o.textContent?.replace(/\D/g, "") === "4096")!.value;
    fft.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    const configure = calls.filter(([c]) => c === "analyzer_inspector_configure");
    expect(configure.at(-1)?.[1]).toEqual({ id: 42, config: { fft_size: 4096, window: "hann", response: "medium" } });
    expect(calls.filter(([c]) => c === "analyzer_inspector_subscribe").length).toBe(1);

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(target.querySelector('[data-testid="spectrum-inspector"]')).toBeNull();
    const unsubscribed = calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id);
    expect(unsubscribed).toContain(42);
    expect(unsubscribed).toContain(43);
    unmount(app);
  });

  it("leaving the Live source closes the engine stream", async () => {
    const calls = recordCalls();
    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();
    const average = [...target.querySelectorAll<HTMLElement>('[data-testid="inspector-source"] [role="radio"], [data-testid="inspector-source"] button')].find(
      (el) => el.textContent?.trim() === "Average",
    )!;
    average.click();
    await settle();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id)).toContain(42);
    expect(target.querySelector('[data-testid="inspector-analyze"]')).not.toBeNull();
    unmount(app);
  });
});

// --- H-117: a legend for every curve, and Explain My Voice reusing an Average already here ------

/** A `VXLT` frame (see `ui/src/lib/ipc/inspector.ts`) carrying `levels` and, optionally, a
 * room-tone `noise` curve, for one result of a `spectrum_report`. */
function vxlt(jobId: number, index: number, levels: number[], noise?: number[]): ArrayBuffer {
  const bins = levels.length;
  const hasNoise = noise !== undefined;
  const buf = new ArrayBuffer(36 + 4 * bins * (hasNoise ? 2 : 1));
  const dv = new DataView(buf);
  "VXLT".split("").forEach((c, i) => dv.setUint8(i, c.charCodeAt(0)));
  dv.setUint16(4, 1, true);
  dv.setUint16(6, 36, true);
  dv.setUint32(8, jobId, true);
  dv.setUint32(12, index, true);
  dv.setUint32(16, 48_000, true);
  dv.setUint32(20, 4, true);
  dv.setUint32(24, 0, true);
  dv.setUint32(28, bins, true);
  dv.setUint32(32, hasNoise ? 1 : 0, true);
  levels.forEach((v, k) => dv.setFloat32(36 + 4 * k, v, true));
  if (noise) {
    noise.forEach((v, k) => dv.setFloat32(36 + 4 * bins + 4 * k, v, true));
  }
  return buf;
}

function averageReport(jobId: number, hasNoise = false): SpectrumReportDto {
  return {
    job_id: jobId,
    sample_rate_hz: 48_000,
    fft_size: 4,
    window: "hann",
    start_sample: 0,
    end_sample: 48_000,
    results: [{ source: "processed", frames: 10, has_noise: hasNoise, report: balancedReport() }],
  };
}

function clickSource(target: HTMLElement, label: "Live" | "Average"): void {
  const el = [
    ...target.querySelectorAll<HTMLElement>('[data-testid="inspector-source"] [role="radio"], [data-testid="inspector-source"] button'),
  ].find((e) => e.textContent?.trim() === label)!;
  el.click();
}

describe("Spectrum Inspector legend (H-117)", () => {
  afterEach(() => {
    resetDocumentStateForTest();
  });

  it("names the voice curve, the room tone, and any frozen snapshot", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    // Keep a (no-op) Tauri IPC mock active rather than `clearMocks()`: the Inspector's own Live
    // stream subscribes as soon as it mounts (default source), and its `Channel` construction
    // isn't guarded the way the listener setup elsewhere is.
    mockIPC(() => null);

    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();

    const chip = (tone: string) => target.querySelector(`.legend-chip[data-tone="${tone}"]`)?.textContent?.trim() ?? null;
    // Live, nothing else drawn yet: only the voice chip, no unexplained dashed curve.
    expect(chip("voice")).toContain("Live");
    expect(chip("noise")).toBeNull();

    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        return { job_id: 9 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        return vxlt(a.jobId, a.index, [-10, -20, -30], [-60, -65, -70]);
      }
      return null;
    });
    clickSource(target, "Average");
    await settle();
    target.querySelector<HTMLButtonElement>('[data-testid="inspector-analyze"]')!.click();
    await settle();
    applySpectrumJobProgress({ job_id: 9, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport(averageReport(9, true));
    await settle();

    expect(chip("voice")).toContain("Average");
    // The dashed gray curve the owner couldn't identify: named, and its purpose spelled out.
    expect(chip("noise")).toContain("Room tone");
    expect(chip("noise")).toContain("quiet between phrases");
    expect(chip("noise")).toContain("compare");

    target.querySelector<HTMLButtonElement>('[data-testid="inspector-freeze-a"]')!.click();
    await settle();
    expect(chip("a")).toContain("A:");

    unmount(app);
  });
});

describe("Explain My Voice inside the Inspector (H-117)", () => {
  afterEach(() => {
    resetDocumentStateForTest();
    resetExplainModalForTest();
  });

  it("opens immediately from an Average result already held here — it never starts a second job", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    // Keep a (no-op) Tauri IPC mock active rather than `clearMocks()`: the Inspector's own Live
    // stream subscribes as soon as it mounts (default source), and its `Channel` construction
    // isn't guarded the way the listener setup elsewhere is.
    mockIPC(() => null);

    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();

    // Seed an existing Average result the way the toolbar's own Analyze button would.
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        return { job_id: 5 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        return vxlt(a.jobId, a.index, [-12, -24, -36]);
      }
      return null;
    });
    clickSource(target, "Average");
    await settle();
    target.querySelector<HTMLButtonElement>('[data-testid="inspector-analyze"]')!.click();
    await settle();
    applySpectrumJobProgress({ job_id: 5, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport(averageReport(5));
    await settle();

    const started: unknown[] = [];
    mockIPC((cmd) => {
      if (cmd === "spectrum_analyze_start") {
        started.push(cmd);
      }
      return null;
    });

    const startedAtMs = performance.now();
    target.querySelector<HTMLButtonElement>('[data-testid="inspector-explain-open"]')!.click();
    await settle();
    const elapsedMs = performance.now() - startedAtMs;

    expect(started).toHaveLength(0); // no second Average job — reused the one already here
    expect(explainModalState().open).toBe(true);
    const snapshot = explainVoiceState().snapshot!;
    expect(snapshot.origin).toBe("average");
    expect(snapshot.resolution).toBe("bins");
    expect(Array.from(snapshot.rawDb)).toEqual([-12, -24, -36]);
    // H-117 verification: reusing the held Average must not go anywhere near a second job's
    // round trip — this stays under a test-machine-noise-tolerant budget, not a UI timing spec.
    expect(elapsedMs).toBeLessThan(200);

    unmount(app);
  });

  it("starts its own Average job when there is nothing to reuse yet, and opens once it completes", async () => {
    mockIPC((cmd) => (cmd === "document_open" ? docDto({ len_samples: 96_000 }) : null));
    await openDocument("/home/user/take.wav");
    // Keep a (no-op) Tauri IPC mock active rather than `clearMocks()`: the Inspector's own Live
    // stream subscribes as soon as it mounts (default source), and its `Channel` construction
    // isn't guarded the way the listener setup elsewhere is.
    mockIPC(() => null);

    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();

    const button = target.querySelector<HTMLButtonElement>('[data-testid="inspector-explain-open"]')!;
    expect(button.disabled).toBe(false);

    const started: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "spectrum_analyze_start") {
        started.push((args as { request: unknown }).request);
        return { job_id: 42 };
      }
      if (cmd === "spectrum_analyze_curve") {
        const a = args as { jobId: number; index: number };
        return vxlt(a.jobId, a.index, [-10, -20, -30]);
      }
      return null;
    });
    button.click();
    await settle();

    expect(started).toHaveLength(1);
    expect(explainModalState().open).toBe(false);
    expect(button.textContent).toContain("Analyzing");

    applySpectrumJobProgress({ job_id: 42, kind: "spectrum_analyze", state: "done", fraction: 1 });
    await applySpectrumReport(averageReport(42));
    await settle();

    expect(explainModalState().open).toBe(true);
    unmount(app);
  });

  it("stays disabled with no document open", async () => {
    mockIPC(() => null);
    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();
    expect(target.querySelector<HTMLButtonElement>('[data-testid="inspector-explain-open"]')!.disabled).toBe(true);
    unmount(app);
  });
});
