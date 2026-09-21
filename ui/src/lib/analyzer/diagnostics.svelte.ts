/**
 * Analyzer diagnostics store (H-42, SPEC-007 §8): the analyzer's mode (Live / Average / Compare),
 * the peak-label and diagnostics-panel toggles, the Spectrum Inspector's settings, the live voice
 * report, the long-term average job and its curves, and the frozen A/B snapshots.
 *
 * - **Live voice report**: `analyzer_voice_subscribe` while at least one consumer holds it
 *   ({@link acquireLiveVoice}: the dock's diagnostics panel, the Inspector). The engine sends a
 *   report only when it changed, so an idle app receives nothing.
 * - **Average**: `spectrum_analyze_start` over the selection (or the whole file), then
 *   `job_progress` + `spectrum_report` events; each result's curve is fetched as a binary `VXLT`
 *   frame. "Source vs Processed" runs one job over both signals and freezes them as A and B.
 * - Prefs persist in `Settings.analyzer_diagnostics` (seeded by `App.svelte`).
 */
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AnalyzerDiagnosticsPrefsDto,
  AnalyzerResponseDto,
  EventName,
  IpcError,
  JobProgressDto,
  LoudnessSourceDto,
  Notice,
  SpectrumReportDto,
  SpectrumScalePref,
  SpectrumSmoothingPref,
  SpectrumWindowPref,
  VoiceReportDto,
} from "../ipc/bindings";
import {
  analyzerUnsubscribe,
  analyzerVoiceSubscribe,
  spectrumAnalyzeCancel,
  spectrumAnalyzeCurve,
  spectrumAnalyzeStart,
} from "../ipc/commands";
import { binFrequencies, decodeVxlt } from "../ipc/inspector";
import type { PlotCurve } from "./plotGeometry";
import { toArrayBuffer } from "../ipc/telemetry";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { hasSelection, selectionState } from "../state/selection.svelte";
import { saveSettings } from "../state/settings.svelte";
import { startJobStatusPoll } from "../state/jobStatusPoll";

export type AnalyzerMode = "live" | "average" | "compare";
export type SnapshotSlot = "a" | "b";
/** What a snapshot was taken from (its legend label). */
export type SnapshotOrigin = "live" | "average" | "source" | "processed" | "inspector";

/** A curve the plot can draw (an owned copy, unlike a live `PlotCurve` view). */
export interface SpectrumCurve extends PlotCurve {
  readonly freqsHz: Float64Array;
  readonly levelsDb: Float32Array;
}

export interface Snapshot {
  origin: SnapshotOrigin;
  curve: SpectrumCurve;
}

export interface AverageCurve {
  source: LoudnessSourceDto;
  curve: SpectrumCurve;
  /** The room-tone spectrum, when the selection had quiet stretches. */
  noise: SpectrumCurve | null;
  report: VoiceReportDto;
}

export interface SpectrumJobState {
  jobId: number;
  fraction: number;
  state: "running" | "done" | "cancelled" | "failed";
  /** `compare` jobs freeze their two results as A (source) and B (processed). */
  purpose: "average" | "compare";
}

export const INSPECTOR_FFT_SIZES = [1024, 2048, 4096, 8192, 16_384, 32_768] as const;

const DEFAULT_PREFS: AnalyzerDiagnosticsPrefsDto = {
  peak_labels: true,
  panel_visible: false,
  inspector_fft_size: 16_384,
  inspector_window: "hann",
  inspector_smoothing: "none",
  inspector_scale: "log",
  inspector_response: "medium",
};

let mode = $state<AnalyzerMode>("live");
let prefs = $state<AnalyzerDiagnosticsPrefsDto>({ ...DEFAULT_PREFS });
let inspectorOpen = $state(false);
let liveReport = $state.raw<VoiceReportDto | null>(null);
let averageSource = $state<LoudnessSourceDto>("processed");
let job = $state<SpectrumJobState | null>(null);
let averageReport = $state.raw<SpectrumReportDto | null>(null);
let averages = $state.raw<AverageCurve[]>([]);
let snapshots = $state.raw<Record<SnapshotSlot, Snapshot | null>>({ a: null, b: null });

let voiceHolders = 0;
let voiceId: number | undefined;
let voiceGeneration = 0;
let unlistenProgress: (() => void) | null = null;
let unlistenReport: (() => void) | null = null;
/** H-96 "belt and braces": stops the recovery poll for the current average/compare job. */
let stopStatusPoll: (() => void) | null = null;

/**
 * H-108: the owner hit "Analyzing…" that never finished, with no way to tell slow from stuck —
 * this is the backstop. If **nothing at all** (not a real `job_progress`, not a recovered
 * `job_status` tick) arrives for this long while a job looks "running", it almost certainly is
 * stuck (a lost terminal event past H-96's own recovery, a wedged render thread), so this stops
 * pretending and fails it instead of spinning forever. Generous on purpose: a huge file's
 * "processed" render reports no progress at all until the analysis pass after it starts (only
 * `analyze_buffer`'s frame loop ticks), so a legitimately slow job must not trip this.
 */
const NO_PROGRESS_TIMEOUT_MS = 30_000;
/** How often the watchdog checks; independent of the ~10 Hz real progress rate or H-96's 3 s
 * recovery poll. */
const WATCHDOG_INTERVAL_MS = 2_000;

let lastProgressAt = 0;
let stopWatchdog: (() => void) | null = null;

function localNotice(key: string): Notice {
  return { level: "error", key, params: {}, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null };
}

/** A locally-built error toast for a spectrum-analyze failure that never went through
 * `invoke()` (so there's no `IpcError` to route through `noticeFromIpcError`) — the no-progress
 * timeout above, and `AnalyzerPanel.svelte`'s "done but produced nothing for this source" case. */
export function pushSpectrumFailureNotice(key: string): void {
  pushNotice(localNotice(key));
}

/** (Re)starts the no-progress watchdog for `jobId` (any previous one is stopped first). */
function startWatchdog(jobId: number): void {
  stopWatchdog?.();
  lastProgressAt = Date.now();
  const id = setInterval(() => {
    if (!job || job.jobId !== jobId || job.state !== "running") {
      stopWatchdog?.();
      return;
    }
    if (Date.now() - lastProgressAt >= NO_PROGRESS_TIMEOUT_MS) {
      void spectrumAnalyzeCancel(jobId).catch(() => {});
      job = { ...job, state: "failed" };
      pushSpectrumFailureNotice("error.spectrum.timeout");
      stopWatchdog?.();
    }
  }, WATCHDOG_INTERVAL_MS);
  stopWatchdog = () => clearInterval(id);
}

/** Read-only accessor for components. */
export function diagnosticsState(): {
  readonly mode: AnalyzerMode;
  readonly prefs: AnalyzerDiagnosticsPrefsDto;
  readonly inspectorOpen: boolean;
  readonly liveReport: VoiceReportDto | null;
  readonly averageSource: LoudnessSourceDto;
  readonly job: SpectrumJobState | null;
  readonly averageReport: SpectrumReportDto | null;
  readonly averages: AverageCurve[];
  readonly snapshots: Record<SnapshotSlot, Snapshot | null>;
} {
  return {
    get mode() {
      return mode;
    },
    get prefs() {
      return prefs;
    },
    get inspectorOpen() {
      return inspectorOpen;
    },
    get liveReport() {
      return liveReport;
    },
    get averageSource() {
      return averageSource;
    },
    get job() {
      return job;
    },
    get averageReport() {
      return averageReport;
    },
    get averages() {
      return averages;
    },
    get snapshots() {
      return snapshots;
    },
  };
}

function persist(patch: Partial<AnalyzerDiagnosticsPrefsDto>): void {
  prefs = { ...prefs, ...patch };
  void saveSettings({ analyzer_diagnostics: prefs });
}

/** Seeds the prefs from `Settings.analyzer_diagnostics` (App.svelte, once loaded). */
export function applyDiagnosticsPrefs(next: AnalyzerDiagnosticsPrefsDto | undefined): void {
  prefs = { ...DEFAULT_PREFS, ...(next ?? {}) };
}

export function setAnalyzerMode(next: AnalyzerMode): void {
  mode = next;
}

export function setPeakLabels(on: boolean): void {
  persist({ peak_labels: on });
}

export function setDiagnosticsPanelVisible(on: boolean): void {
  persist({ panel_visible: on });
}

export function setInspectorOpen(open: boolean): void {
  inspectorOpen = open;
}

export function setInspectorFftSize(size: number): void {
  persist({ inspector_fft_size: size });
}

export function setInspectorWindow(window: SpectrumWindowPref): void {
  persist({ inspector_window: window });
}

export function setInspectorSmoothing(smoothing: SpectrumSmoothingPref): void {
  persist({ inspector_smoothing: smoothing });
}

export function setInspectorScale(scale: SpectrumScalePref): void {
  persist({ inspector_scale: scale });
}

export function setInspectorResponse(response: AnalyzerResponseDto): void {
  persist({ inspector_response: response });
}

export function setAverageSource(next: LoudnessSourceDto): void {
  averageSource = next;
}

// --- Live voice report ---------------------------------------------------------------------------

/**
 * Holds the live voice subscription while the returned release function hasn't been called:
 * the first holder subscribes, the last one to release unsubscribes (the engine then drops the
 * statistics).
 */
export function acquireLiveVoice(): () => void {
  voiceHolders += 1;
  if (voiceHolders === 1) {
    const generation = ++voiceGeneration;
    void analyzerVoiceSubscribe(
      new Channel<VoiceReportDto>((report) => {
        if (generation === voiceGeneration) {
          liveReport = report;
        }
      }),
    )
      .then((id) => {
        if (generation === voiceGeneration) {
          voiceId = id;
        } else {
          void analyzerUnsubscribe(id).catch(() => {});
        }
      })
      .catch(() => {
        // No engine (preview/tests): the panel just waits.
      });
  }
  let released = false;
  return () => {
    if (released) {
      return;
    }
    released = true;
    voiceHolders = Math.max(0, voiceHolders - 1);
    if (voiceHolders === 0) {
      voiceGeneration += 1;
      const id = voiceId;
      voiceId = undefined;
      liveReport = null;
      if (id !== undefined) {
        void analyzerUnsubscribe(id).catch(() => {});
      }
    }
  };
}

/** Test hook: a report as if the engine had sent it. */
export function applyLiveReport(report: VoiceReportDto | null): void {
  liveReport = report;
}

// --- Long-term average job -----------------------------------------------------------------------

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

/** `[start, end)` of the selection, or the whole document; `null` with nothing open. */
export function averageScope(): { range: [number, number]; selection: boolean } | null {
  if (hasSelection()) {
    const current = selectionState().current!;
    return { range: [current.startSample, current.endSample], selection: true };
  }
  const doc = documentState().current;
  return doc.sample_rate_hz > 0 && doc.len_samples > 0 ? { range: [0, doc.len_samples], selection: false } : null;
}

export function canAnalyzeAverage(): boolean {
  return averageScope() !== null && job?.state !== "running";
}

async function ensureListening(): Promise<void> {
  if (!unlistenProgress) {
    try {
      unlistenProgress = await listen<JobProgressDto>("job_progress" satisfies EventName, (event) =>
        applySpectrumJobProgress(event.payload),
      );
    } catch {
      // Not in a Tauri window (tests drive the apply functions directly).
    }
  }
  if (!unlistenReport) {
    try {
      unlistenReport = await listen<SpectrumReportDto>("spectrum_report" satisfies EventName, (event) =>
        void applySpectrumReport(event.payload),
      );
    } catch {
      // See above.
    }
  }
}

async function start(sources: LoudnessSourceDto[], purpose: SpectrumJobState["purpose"]): Promise<void> {
  const scope = averageScope();
  if (!scope || job?.state === "running") {
    return;
  }
  // H-96: subscribe *before* starting the job, so a fast job's terminal `job_progress` can never
  // arrive before a listener is attached (the ordering bug H-96 fixed for export/normalize/bake;
  // H-92's "Explain My Voice" starts this same job and needs the same guarantee).
  await ensureListening();
  try {
    const started = await spectrumAnalyzeStart({
      start_sample: scope.range[0],
      end_sample: scope.range[1],
      sources,
      fft_size: prefs.inspector_fft_size,
      window: prefs.inspector_window,
    });
    job = { jobId: started.job_id, fraction: 0, state: "running", purpose };
    // H-96 "belt and braces": if the real event is ever missed anyway (a dropped IPC message, a
    // webview reload mid-job), poll the shared `job_status` recovery cache rather than leaving
    // the job stuck at "running" forever.
    stopStatusPoll?.();
    stopStatusPoll = startJobStatusPoll(
      started.job_id,
      () => job !== null && job.jobId === started.job_id && job.state === "running",
      applySpectrumJobProgress,
    );
    // H-108: and if *that* recovery path is ever silent too, the watchdog is the last resort.
    startWatchdog(started.job_id);
  } catch (err) {
    if (isIpcError(err)) {
      pushNotice(noticeFromIpcError(err));
    }
  }
}

/** Average mode's Analyze: the chosen signal over the selection or the whole file. */
export function startAverage(): Promise<void> {
  return start([averageSource], "average");
}

/** Compare mode's Source vs Processed: one job, frozen as A (source) and B (processed). */
export function startSourceVsProcessed(): Promise<void> {
  return start(["source", "processed"], "compare");
}

export function cancelAverage(): void {
  if (job?.state === "running") {
    void spectrumAnalyzeCancel(job.jobId).catch(() => {});
  }
}

/** Applies one `job_progress` event (ignores other kinds/jobs). */
export function applySpectrumJobProgress(payload: JobProgressDto): void {
  if (payload.kind !== "spectrum_analyze" || !job || payload.job_id !== job.jobId) {
    return;
  }
  lastProgressAt = Date.now(); // H-108: any tick for our job, real or recovered, feeds the watchdog.
  job = { ...job, fraction: payload.fraction, state: payload.state };
}

function binsCurve(levels: Float32Array, sampleRateHz: number, fftSize: number): SpectrumCurve {
  return { freqsHz: binFrequencies(levels.length, sampleRateHz, fftSize), levelsDb: levels, resolution: "bins" };
}

/** Applies a finished `spectrum_report`: fetches each curve, then shows them (Average) or
 * freezes them as A/B (Compare). */
export async function applySpectrumReport(payload: SpectrumReportDto): Promise<void> {
  if (!job || payload.job_id !== job.jobId) {
    return;
  }
  const purpose = job.purpose;
  const curves: AverageCurve[] = [];
  for (let i = 0; i < payload.results.length; i++) {
    const result = payload.results[i]!;
    try {
      const raw = toArrayBuffer(await spectrumAnalyzeCurve(payload.job_id, i));
      const frame = raw ? decodeVxlt(raw) : null;
      if (!frame) {
        continue;
      }
      curves.push({
        source: result.source,
        curve: binsCurve(frame.levelsDb, frame.sampleRateHz, frame.fftSize),
        noise: frame.noiseDb ? binsCurve(frame.noiseDb, frame.sampleRateHz, frame.fftSize) : null,
        report: result.report,
      });
    } catch (err) {
      if (isIpcError(err)) {
        pushNotice(noticeFromIpcError(err));
      }
    }
  }
  if (purpose === "compare") {
    const source = curves.find((c) => c.source === "source");
    const processed = curves.find((c) => c.source === "processed");
    snapshots = {
      a: source ? { origin: "source", curve: source.curve } : snapshots.a,
      b: processed ? { origin: "processed", curve: processed.curve } : snapshots.b,
    };
  }
  averageReport = payload;
  averages = curves;
}

// --- Snapshots -------------------------------------------------------------------------------------

/** Freezes a copy of `curve` into `slot`. */
export function freezeSnapshot(slot: SnapshotSlot, origin: SnapshotOrigin, curve: PlotCurve): void {
  const copy: SpectrumCurve = {
    freqsHz: Float64Array.from(curve.freqsHz),
    levelsDb: Float32Array.from(curve.levelsDb),
    resolution: curve.resolution,
  };
  snapshots = { ...snapshots, [slot]: { origin, curve: copy } };
}

export function clearSnapshots(): void {
  snapshots = { a: null, b: null };
}

/** Test/teardown helper. */
export function resetDiagnosticsForTest(): void {
  mode = "live";
  prefs = { ...DEFAULT_PREFS };
  inspectorOpen = false;
  liveReport = null;
  averageSource = "processed";
  job = null;
  averageReport = null;
  averages = [];
  snapshots = { a: null, b: null };
  voiceHolders = 0;
  voiceId = undefined;
  voiceGeneration += 1;
  for (const unlisten of [unlistenProgress, unlistenReport]) {
    // Tauri's unlisten is async: listeners registered without the mocked event plugin (tests)
    // reject when removed, synchronously or not.
    try {
      void Promise.resolve(unlisten?.() as unknown).catch(() => {});
    } catch {
      // See above.
    }
  }
  unlistenProgress = null;
  unlistenReport = null;
  stopStatusPoll?.();
  stopStatusPoll = null;
  stopWatchdog?.();
  stopWatchdog = null;
}
