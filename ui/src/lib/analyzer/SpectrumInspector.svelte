<script lang="ts">
  import { tick } from "svelte";
  import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
  import { Button, IconButton, SegmentedControl, Select, ToggleButton, type SegmentOption, type SelectOption } from "../ui";
  import { formatNumber } from "../ui/units";
  import { t } from "../i18n";
  import type {
    AnalyzerResponseDto,
    IpcError,
    Notice,
    SpectrumScalePref,
    SpectrumSmoothingPref,
    SpectrumWindowPref,
  } from "../ipc/bindings";
  import { spectrumExportCsv } from "../ipc/commands";
  import { binFrequencies } from "../ipc/inspector";
  import { noticeFromIpcError } from "../notices/fromIpcError";
  import { pushNotice } from "../state/notices.svelte";
  import { documentState } from "../document/document.svelte";
  import {
    acquireLiveVoice,
    averageScope,
    canAnalyzeAverage,
    cancelAverage,
    diagnosticsState,
    freezeSnapshot,
    INSPECTOR_FFT_SIZES,
    setInspectorFftSize,
    setInspectorOpen,
    setInspectorResponse,
    setInspectorScale,
    setInspectorSmoothing,
    setInspectorWindow,
    setPeakLabels,
    startAverage,
    type SnapshotSlot,
  } from "./diagnostics.svelte";
  import { openExplainVoice } from "./explain/explainModal.svelte";
  import type { VoiceSnapshotInput } from "./explain/snapshot";
  import { closeInspectorStream, inspectorStream, openInspectorStream } from "./inspectorStream.svelte";
  import { formatNote } from "./notes";
  import type { SpectralPeak } from "./peaks";
  import type { PlotCurve, PlotOverlay } from "./plotGeometry";
  import { smoothFractionalOctave, smoothingOctaves } from "./smoothing";
  import { spectrumCsv } from "./spectrumCsv";
  import SpectrumPlot from "./SpectrumPlot.svelte";
  import DiagnosticsPanel from "./DiagnosticsPanel.svelte";

  /**
   * The Spectrum Inspector (H-42, SPEC-007 §8.3): a larger, non-modal window (View → Spectrum
   * Inspector, or the analyzer's button) with FFT size 1 024 … 32 768, the window function,
   * fractional-octave smoothing, a log or linear axis with range zoom (Shift-drag), the same peak
   * labels at bin resolution, a peak table, the diagnostics, and CSV export of what's shown.
   * Sources: the live output (its own engine stream at these settings, only while shown), the
   * long-term average, or a frozen snapshot. Draggable by its title bar, resizable from the
   * corner; Escape closes it. Not modal: the app's shortcuts keep working.
   */

  type Source = "live" | "average" | "a" | "b";
  const SOURCES: Source[] = ["live", "average", "a", "b"];
  const WINDOWS: SpectrumWindowPref[] = ["hann", "blackman_harris", "flat_top", "rectangular"];
  const SMOOTHINGS: SpectrumSmoothingPref[] = ["none", "third", "sixth", "twelfth"];
  const SCALES: SpectrumScalePref[] = ["log", "linear"];
  const RESPONSES: AnalyzerResponseDto[] = ["fast", "medium", "slow"];
  const FLOOR_DB = -140;
  const CEIL_DB = 0;
  const MIN_W = 560;
  const MIN_H = 360;

  const diag = diagnosticsState();
  const stream = inspectorStream();
  let source = $state<Source>("live");
  let zoom = $state<[number, number] | null>(null);
  let tablePeaks = $state.raw<SpectralPeak[]>([]);
  let rect = $state({ x: 0, y: 0, w: 960, h: 580 });
  let placed = false;
  let windowEl: HTMLDivElement | undefined = $state();

  const sourceOptions: SegmentOption<Source>[] = SOURCES.map((s) => ({
    value: s,
    label: t(`inspector.source.${s}` as `inspector.source.${Source}`),
  }));
  const fftOptions: SelectOption<number>[] = INSPECTOR_FFT_SIZES.map((n) => ({ value: n, label: formatNumber(n, 0) }));
  const windowOptions: SelectOption<SpectrumWindowPref>[] = WINDOWS.map((w) => ({
    value: w,
    label: t(`inspector.window.${w}` as `inspector.window.${SpectrumWindowPref}`),
  }));
  const smoothingOptions: SelectOption<SpectrumSmoothingPref>[] = SMOOTHINGS.map((s) => ({
    value: s,
    label: t(`inspector.smoothing.${s}` as `inspector.smoothing.${SpectrumSmoothingPref}`),
  }));
  const scaleOptions: SegmentOption<SpectrumScalePref>[] = SCALES.map((s) => ({
    value: s,
    label: t(`inspector.scale.${s}` as `inspector.scale.${SpectrumScalePref}`),
  }));
  const responseOptions: SegmentOption<AnalyzerResponseDto>[] = RESPONSES.map((r) => ({
    value: r,
    label: t(`analyzer.response.${r}` as `analyzer.response.${AnalyzerResponseDto}`),
  }));

  const open = $derived(diag.inspectorOpen);
  const prefs = $derived(diag.prefs);

  // The live stream exists only while the Inspector shows it.
  $effect(() => {
    if (!open || source !== "live") {
      closeInspectorStream();
      return;
    }
    openInspectorStream({
      fft_size: prefs.inspector_fft_size,
      window: prefs.inspector_window,
      response: prefs.inspector_response,
    });
  });
  $effect(() => () => closeInspectorStream());

  $effect(() => {
    if (open) {
      return acquireLiveVoice();
    }
  });

  // First open: centre in the window; later opens keep the last place.
  $effect(() => {
    if (!open || placed || typeof window === "undefined") {
      return;
    }
    placed = true;
    const w = Math.max(MIN_W, Math.min(980, window.innerWidth - 48));
    const h = Math.max(MIN_H, Math.min(600, window.innerHeight - 96));
    rect = { x: Math.max(16, (window.innerWidth - w) / 2), y: Math.max(40, (window.innerHeight - h) / 2), w, h };
  });

  $effect(() => {
    if (open) {
      void tick().then(() => windowEl?.focus());
    }
  });

  let freqCache: { key: string; freqs: Float64Array } | null = null;
  function freqsFor(bins: number, fs: number, fft: number): Float64Array {
    const key = `${bins}:${fs}:${fft}`;
    if (freqCache?.key !== key) {
      freqCache = { key, freqs: binFrequencies(bins, fs, fft) };
    }
    return freqCache.freqs;
  }

  const average = $derived(
    diag.averages.find((a) => a.source === diag.averageSource) ?? diag.averages[0] ?? null,
  );

  const rawCurve = $derived.by((): PlotCurve | null => {
    switch (source) {
      case "live": {
        const f = stream.frame;
        return f ? { freqsHz: freqsFor(f.levelsDb.length, f.sampleRateHz, f.fftSize), levelsDb: f.levelsDb, resolution: "bins" } : null;
      }
      case "average":
        return average?.curve ?? null;
      case "a":
        return diag.snapshots.a?.curve ?? null;
      default:
        return diag.snapshots.b?.curve ?? null;
    }
  });

  const curve = $derived.by((): PlotCurve | null => {
    const c = rawCurve;
    const width = smoothingOctaves(prefs.inspector_smoothing);
    if (!c || width === null || c.resolution !== "bins") {
      return c;
    }
    return { freqsHz: c.freqsHz, levelsDb: smoothFractionalOctave(c.freqsHz, c.levelsDb, width), resolution: "bins" };
  });

  const sampleRateHz = $derived(
    stream.frame?.sampleRateHz ?? diag.averageReport?.sample_rate_hz ?? (documentState().current.sample_rate_hz || 48_000),
  );
  const maxHz = $derived(sampleRateHz / 2);

  const overlays = $derived.by((): PlotOverlay[] => {
    const out: PlotOverlay[] = [];
    if (source === "average" && average?.noise) {
      out.push({ key: "noise", curve: average.noise, tone: "noise", dashed: true });
    }
    if (source !== "a" && diag.snapshots.a) {
      out.push({ key: "a", curve: diag.snapshots.a.curve, tone: "a" });
    }
    if (source !== "b" && diag.snapshots.b) {
      out.push({ key: "b", curve: diag.snapshots.b.curve, tone: "b" });
    }
    return out;
  });

  const resolution = $derived(
    t("inspector.resolution", {
      hz: formatNumber(sampleRateHz / prefs.inspector_fft_size, sampleRateHz / prefs.inspector_fft_size < 10 ? 2 : 1),
      ms: formatNumber((prefs.inspector_fft_size / sampleRateHz) * 1000, 0),
    }),
  );

  const scope = $derived(averageScope());
  const jobRunning = $derived(diag.job?.state === "running");
  const report = $derived(source === "average" ? (average?.report ?? null) : diag.liveReport);
  const noDataText = $derived(curve ? null : source === "average" && !jobRunning ? t("analyzer.average.empty") : t("inspector.no_data"));

  function close(): void {
    setInspectorOpen(false);
  }

  // H-117: Explain My Voice belongs here too — the Inspector is "the analysis" as far as the
  // owner is concerned — and must call the same `openExplainVoice` path the dock button (H-92)
  // uses, so the two are indistinguishable. The one behaviour this adds: if an Average result for
  // the current scope is already sitting in `average` (e.g. the owner just ran one in this same
  // window), open on it immediately instead of running a second job — that avoidable wait was
  // exactly what H-108 was filed over.
  let pendingExplain = $state(false);

  function explainInputFromAverage(): VoiceSnapshotInput | null {
    const a = average;
    const r = diag.averageReport;
    if (!a || !r) {
      return null;
    }
    return {
      freqsHz: a.curve.freqsHz,
      levelsDb: a.curve.levelsDb,
      resolution: "bins",
      report: a.report,
      sampleRateHz: r.sample_rate_hz,
      origin: "average",
    };
  }

  function requestExplain(): void {
    const existing = explainInputFromAverage();
    if (existing) {
      openExplainVoice(existing);
      return;
    }
    if (!canAnalyzeAverage()) {
      return;
    }
    pendingExplain = true;
    void startAverage();
  }

  $effect(() => {
    if (!pendingExplain) {
      return;
    }
    const j = diag.job;
    if (!j || j.purpose !== "average") {
      return;
    }
    if (j.state === "done") {
      // Same guard as H-92's: only trust `averageReport` once it is unmistakably this job's own
      // (matched by `job_id`), since the "done" progress event can arrive before the report does.
      if (diag.averageReport?.job_id !== j.jobId) {
        return;
      }
      const input = explainInputFromAverage();
      if (input) {
        pendingExplain = false;
        openExplainVoice(input);
      }
    } else if (j.state === "failed" || j.state === "cancelled") {
      pendingExplain = false;
    }
  });

  function freeze(slot: SnapshotSlot): void {
    const c = curve;
    if (c) {
      freezeSnapshot(slot, source === "average" ? "average" : "inspector", c);
    }
  }

  function isIpcError(value: unknown): value is IpcError {
    return typeof value === "object" && value !== null && "code" in value && "key" in value;
  }

  async function exportCsv(): Promise<void> {
    const c = curve;
    if (!c) {
      return;
    }
    let path: string | null = null;
    try {
      path = await saveFileDialog({
        title: t("inspector.export_title"),
        defaultPath: "spectrum.csv",
        filters: [{ name: t("inspector.csv_filter"), extensions: ["csv"] }],
      });
    } catch {
      return;
    }
    if (!path) {
      return;
    }
    const [lo, hi] = zoom ?? [0, Infinity];
    try {
      const written = await spectrumExportCsv(path, spectrumCsv(c, lo, hi));
      const n: Notice = {
        level: "info",
        key: "notice.inspector.csv_saved",
        params: { path: written },
        persistent: false,
        id: null,
        cleared: false,
    auto_dismiss_ms: null,
        action: null,
      };
      pushNotice(n);
    } catch (err) {
      if (isIpcError(err)) {
        pushNotice(noticeFromIpcError(err));
      }
    }
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  function drag(e: PointerEvent, kind: "move" | "resize"): void {
    if (e.button !== 0 || (kind === "move" && (e.target as HTMLElement).closest("button"))) {
      return;
    }
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY, rect: { ...rect } };
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - start.x;
      const dy = ev.clientY - start.y;
      if (kind === "move") {
        rect = {
          ...rect,
          x: Math.max(0, Math.min(window.innerWidth - 160, start.rect.x + dx)),
          y: Math.max(0, Math.min(window.innerHeight - 48, start.rect.y + dy)),
        };
      } else {
        rect = {
          ...rect,
          w: Math.max(MIN_W, Math.min(window.innerWidth - start.rect.x, start.rect.w + dx)),
          h: Math.max(MIN_H, Math.min(window.innerHeight - start.rect.y, start.rect.h + dy)),
        };
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function formatPeakFreq(freqHz: number): string {
    return freqHz < 1000 ? `${formatNumber(freqHz, 1)} Hz` : `${formatNumber(freqHz / 1000, 3)} kHz`;
  }
</script>

<!-- H-117: every curve the plot can draw, named — the dock's chip style (H-92), plus the main
     curve itself (unlabelled here before, unlike the dock's mode control) so this larger view
     never leaves a line unexplained. -->
{#snippet legend()}
  <span class="legend-chip" data-tone="voice">
    <span class="swatch" data-tone="voice"></span>
    {t("inspector.legend.voice", { mode: t(`inspector.source.${source}` as `inspector.source.${Source}`) })}
  </span>
  {#each overlays as o (o.key)}
    <span class="legend-chip" data-tone={o.tone}>
      <span class="swatch" data-tone={o.tone}></span>
      {#if o.tone === "noise"}
        {t("inspector.legend.room_tone")}
      {:else}
        {@const snap = o.tone === "a" ? diag.snapshots.a : diag.snapshots.b}
        {t("analyzer.snapshot.legend", {
          slot: o.tone.toUpperCase(),
          name: snap ? t(`analyzer.snapshot.${snap.origin}` as `analyzer.snapshot.${typeof snap.origin}`) : "",
        })}
      {/if}
    </span>
  {/each}
{/snippet}

{#if open}
  <div
    bind:this={windowEl}
    class="inspector"
    role="dialog"
    aria-modal="false"
    aria-labelledby="spectrum-inspector-title"
    tabindex="-1"
    data-testid="spectrum-inspector"
    style:left="{rect.x}px"
    style:top="{rect.y}px"
    style:width="{rect.w}px"
    style:height="{rect.h}px"
    onkeydown={handleKeydown}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="titlebar" title={t("inspector.move")} onpointerdown={(e) => drag(e, "move")}>
      <h2 id="spectrum-inspector-title">{t("inspector.title")}</h2>
      <span class="resolution" data-testid="inspector-resolution">{resolution}</span>
      <span class="spacer"></span>
      <IconButton icon="close" size="sm" label={t("inspector.close")} testid="inspector-close" onclick={close} />
    </header>

    <div class="toolbar">
      <SegmentedControl options={sourceOptions} bind:value={source} label={t("inspector.source")} size="sm" testid="inspector-source" />
      <Select
        options={fftOptions}
        value={prefs.inspector_fft_size}
        label={t("inspector.fft")}
        size="sm"
        testid="inspector-fft"
        onchange={setInspectorFftSize}
      />
      <Select
        options={windowOptions}
        value={prefs.inspector_window}
        label={t("inspector.window")}
        size="sm"
        testid="inspector-window"
        onchange={setInspectorWindow}
      />
      <Select
        options={smoothingOptions}
        value={prefs.inspector_smoothing}
        label={t("inspector.smoothing")}
        size="sm"
        testid="inspector-smoothing"
        onchange={setInspectorSmoothing}
      />
      <SegmentedControl
        options={scaleOptions}
        value={prefs.inspector_scale}
        label={t("inspector.scale")}
        size="sm"
        testid="inspector-scale"
        onchange={(s) => {
          zoom = null;
          setInspectorScale(s);
        }}
      />
      {#if source === "live"}
        <SegmentedControl
          options={responseOptions}
          value={prefs.inspector_response}
          label={t("inspector.response")}
          size="sm"
          onchange={setInspectorResponse}
        />
      {/if}
    </div>
    <div class="toolbar secondary">
      <ToggleButton size="sm" icon="marker" pressed={prefs.peak_labels} testid="inspector-peaks-toggle" onchange={setPeakLabels}>
        {t("analyzer.peaks")}
      </ToggleButton>
      {#if source === "average"}
        {#if jobRunning}
          <Button size="sm" variant="ghost" onclick={cancelAverage}>{t("analyzer.average.cancel")}</Button>
          <span class="muted">{t("analyzer.average.progress", { pct: Math.round((diag.job?.fraction ?? 0) * 100) })}</span>
        {:else}
          <Button size="sm" icon="analyzer" disabled={!scope} testid="inspector-analyze" onclick={() => void startAverage()}>
            {average ? t("analyzer.average.reanalyze") : t("analyzer.average.analyze")}
          </Button>
        {/if}
      {/if}
      <Button size="sm" disabled={!curve} testid="inspector-freeze-a" onclick={() => freeze("a")}>{t("analyzer.compare.freeze_a")}</Button>
      <Button size="sm" disabled={!curve} testid="inspector-freeze-b" onclick={() => freeze("b")}>{t("analyzer.compare.freeze_b")}</Button>
      <Button
        size="sm"
        icon="explain"
        disabled={!scope || jobRunning}
        loading={pendingExplain}
        title={scope ? t("analyzer.explain_tooltip") : t("analyzer.explain_needs_document")}
        testid="inspector-explain-open"
        onclick={requestExplain}
      >
        {pendingExplain ? t("analyzer.explain_analyzing") : t("analyzer.explain_open")}
      </Button>
      <span class="spacer"></span>
      <span class="muted hint">{t("inspector.zoom_hint")}</span>
      <Button size="sm" variant="ghost" disabled={zoom === null} testid="inspector-zoom-reset" onclick={() => (zoom = null)}>
        {t("inspector.zoom_reset")}
      </Button>
      <Button size="sm" icon="save" disabled={!curve} testid="inspector-export" onclick={() => void exportCsv()}>
        {t("inspector.export_csv")}
      </Button>
    </div>

    <div class="content">
      <SpectrumPlot
        {curve}
        {overlays}
        {legend}
        scale={prefs.inspector_scale}
        {maxHz}
        floorDb={FLOOR_DB}
        ceilDb={CEIL_DB}
        peakLabels={prefs.peak_labels}
        diffAB={overlays.some((o) => o.tone === "a") && overlays.some((o) => o.tone === "b")}
        rangeZoom
        {noDataText}
        testid="inspector"
        bind:zoom
        onpeaks={(p) => (tablePeaks = p)}
      />
      <div class="side">
        {#if prefs.peak_labels && tablePeaks.length > 0}
          <table class="peaks" data-testid="inspector-peaks">
            <caption>{t("inspector.peaks")}</caption>
            <thead>
              <tr>
                <th scope="col">{t("inspector.peak_col_freq")}</th>
                <th scope="col">{t("inspector.peak_col_note")}</th>
                <th scope="col">{t("inspector.peak_col_level")}</th>
              </tr>
            </thead>
            <tbody>
              {#each tablePeaks as p (p.index)}
                <tr>
                  <td>{formatPeakFreq(p.freqHz)}</td>
                  <td>{formatNote(p.freqHz)}</td>
                  <td>{formatNumber(p.levelDb, 1)} dB</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
        <DiagnosticsPanel
          {report}
          scopeText={t(source === "average" ? "analyzer.diag.average_scope" : "analyzer.diag.live_scope", {
            seconds: formatNumber(report?.span_s ?? 0, 1),
          })}
          emptyText={source === "average" ? t("analyzer.diag.no_report") : t("analyzer.diag.waiting")}
          testid="inspector-diagnostics"
        />
      </div>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize" title={t("inspector.resize")} onpointerdown={(e) => drag(e, "resize")}></div>
  </div>
{/if}

<style>
  .inspector {
    position: fixed;
    z-index: 900;
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-panel);
    box-shadow: var(--pv-shadow-3);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    overflow: hidden;
  }

  .inspector:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
  }

  .titlebar {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
    flex: none;
    height: var(--pv-panel-header-h);
    padding: 0 var(--pv-space-2) 0 var(--pv-space-3);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-elevated, var(--pv-bg-panel));
    cursor: move;
    user-select: none;
  }

  h2 {
    margin: 0;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-semibold);
  }

  .resolution,
  .muted {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .hint {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .spacer {
    flex: 1;
  }

  .toolbar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2) var(--pv-space-3);
    padding: var(--pv-space-2) var(--pv-space-3) 0;
  }

  .toolbar.secondary {
    padding-bottom: var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .content {
    display: flex;
    flex: 1;
    min-height: 0;
    background: var(--analyzer-bg);
  }

  .side {
    display: flex;
    flex-direction: column;
    flex: 0 0 300px;
    min-height: 0;
    border-left: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-panel);
  }

  .side > :global(aside) {
    flex: 1;
  }

  .peaks {
    flex: none;
    width: 100%;
    border-collapse: collapse;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
  }

  .peaks caption {
    padding: var(--pv-space-2) var(--pv-space-3) var(--pv-space-1);
    color: var(--pv-text-primary);
    font-weight: var(--pv-weight-semibold);
    text-align: left;
  }

  .peaks th {
    padding: 0 var(--pv-space-3) 2px;
    color: var(--pv-text-tertiary);
    font-weight: var(--pv-weight-normal, 400);
    text-align: left;
  }

  .peaks td {
    padding: 2px var(--pv-space-3);
    color: var(--pv-text-primary);
    white-space: nowrap;
  }

  .peaks tbody tr:nth-child(odd) td {
    background: var(--pv-bg-inset);
  }

  .resize {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
    background: linear-gradient(135deg, transparent 50%, var(--pv-border) 50%);
  }

  /* H-117: the same chip look as the dock's legend (AnalyzerPanel.svelte), so the two views read
     the same, plus a "voice" swatch for the main curve the dock never needed to name. */
  .legend-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--pv-text-tertiary);
    font-size: 10px;
    line-height: 12px;
    white-space: nowrap;
  }

  .swatch {
    width: 12px;
    height: 2px;
    border-radius: 1px;
    background: var(--analyzer-fill);
  }

  .swatch[data-tone="a"] {
    background: var(--analyzer-compare-a);
  }

  .swatch[data-tone="b"] {
    background: var(--analyzer-compare-b);
  }

  .swatch[data-tone="noise"] {
    background: var(--analyzer-noise);
  }
</style>
