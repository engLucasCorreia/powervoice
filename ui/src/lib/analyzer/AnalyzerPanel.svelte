<script lang="ts">
  import { untrack } from "svelte";
  import { Button, SegmentedControl, Toggle, ToggleButton, IconButton, formatWithUnit, type SegmentOption } from "../ui";
  import { formatNumber } from "../ui/units";
  import { t } from "../i18n";
  import type { AnalyzerResponseDto, LoudnessSourceDto } from "../ipc/bindings";
  import {
    ANALYZER_CEIL_OPTIONS_DB,
    ANALYZER_FLOOR_OPTIONS_DB,
    DEFAULT_ANALYZER_CEIL_DB,
    DEFAULT_ANALYZER_FLOOR_DB,
  } from "./analyzerMath";
  import { analyzerState, initAnalyzer, setAnalyzerResponse } from "./analyzer.svelte";
  import { initOutputDeviceStatus, outputDeviceStatus } from "./outputDeviceStatus.svelte";
  import {
    acquireLiveVoice,
    averageScope,
    cancelAverage,
    clearSnapshots,
    diagnosticsState,
    freezeSnapshot,
    setAnalyzerMode,
    setAverageSource,
    setDiagnosticsPanelVisible,
    setInspectorOpen,
    setPeakLabels,
    startAverage,
    startSourceVsProcessed,
    type AnalyzerMode,
    type SnapshotSlot,
  } from "./diagnostics.svelte";
  import SpectrumPlot from "./SpectrumPlot.svelte";
  import DiagnosticsPanel from "./DiagnosticsPanel.svelte";
  import type { PlotCurve, PlotOverlay } from "./plotGeometry";

  /**
   * The live output analyzer panel (T-208/H-16, SPEC-007 §2.9) — its look unchanged — with the
   * H-42 diagnostics (SPEC-007 §8), all optional:
   * - **Peaks**: labels on the strongest peaks (frequency, note ± cents, level) and a hover
   *   crosshair with the note;
   * - **Live / Average / Compare**: the live curve; the long-term average of the selection or the
   *   whole file (a Rust job with progress and cancel) with its room-tone curve; A/B snapshots
   *   over the live curve, including Source vs Processed;
   * - **Diagnostics**: the voice statistics panel beside the graph;
   * - the Spectrum Inspector (a larger window, View → Spectrum Inspector).
   * The plot itself is `SpectrumPlot.svelte` (shared with the Inspector; draws on demand).
   */

  const RESPONSES: AnalyzerResponseDto[] = ["fast", "medium", "slow"];
  const MODES: AnalyzerMode[] = ["live", "average", "compare"];
  const SOURCES: LoudnessSourceDto[] = ["processed", "source"];

  let floorDb = $state<number>(DEFAULT_ANALYZER_FLOOR_DB);
  let ceilDb = $state<number>(DEFAULT_ANALYZER_CEIL_DB);
  /** `null` = full range (follows the device's Nyquist rate); set once the user zooms/pans. */
  let zoomRange: [number, number] | null = $state(null);
  let resetKey = $state(0);

  const analyzer = analyzerState();
  const diag = diagnosticsState();
  const device = outputDeviceStatus();
  // H-25: Fast / Medium / Slow as the kit's small segmented control.
  const responseOptions: SegmentOption<AnalyzerResponseDto>[] = RESPONSES.map((r) => ({
    value: r,
    label: t(`analyzer.response.${r}` as `analyzer.response.${AnalyzerResponseDto}`),
  }));
  const modeOptions: SegmentOption<AnalyzerMode>[] = MODES.map((m) => ({
    value: m,
    label: t(`analyzer.mode.${m}` as `analyzer.mode.${AnalyzerMode}`),
  }));
  const sourceOptions: SegmentOption<LoudnessSourceDto>[] = SOURCES.map((s) => ({
    value: s,
    label: t(`analyzer.average.${s}` as `analyzer.average.${LoudnessSourceDto}`),
  }));

  const frame = $derived(analyzer.frame);
  const nyquistHz = $derived(frame ? frame.sampleRateHz / 2 : 24_000);
  const noOutputDevice = $derived(device.current === "not_selected" || device.current === "lost");

  let bandFreqs = new Float64Array(0);
  function bandFrequencies(count: number, f0Hz: number, bandsPerOctave: number): Float64Array {
    if (bandFreqs.length !== count || bandFreqs[0] !== f0Hz) {
      bandFreqs = Float64Array.from({ length: count }, (_, k) => f0Hz * 2 ** (k / bandsPerOctave));
    }
    return bandFreqs;
  }

  const liveCurve = $derived.by((): PlotCurve | null => {
    const f = frame;
    if (!f || f.levelsDb.length === 0) {
      return null;
    }
    return {
      freqsHz: bandFrequencies(f.levelsDb.length, f.f0Hz, f.bandsPerOctave),
      levelsDb: f.levelsDb,
      resolution: "bands",
    };
  });

  // A device reopen/rate change clears the hold and invalidates a zoom picked against the old
  // Nyquist rate (SPEC-007 §4.8.6).
  $effect(() => {
    if (frame?.reset) {
      untrack(() => {
        resetKey += 1;
        zoomRange = null;
      });
    }
  });

  $effect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    void initAnalyzer().then((c) => {
      if (cancelled) {
        c();
      } else {
        cleanup = c;
      }
    });
    return () => {
      cancelled = true;
      cleanup?.();
    };
  });

  $effect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    void initOutputDeviceStatus().then((c) => {
      if (cancelled) {
        c();
      } else {
        cleanup = c;
      }
    });
    return () => {
      cancelled = true;
      cleanup?.();
    };
  });

  // The live voice statistics only run while the diagnostics panel shows them.
  $effect(() => {
    if (diag.prefs.panel_visible) {
      return acquireLiveVoice();
    }
  });

  const scope = $derived(averageScope());
  const jobRunning = $derived(diag.job?.state === "running");
  const jobPct = $derived(Math.round((diag.job?.fraction ?? 0) * 100));
  const average = $derived(
    diag.averages.find((a) => a.source === diag.averageSource) ?? diag.averages[0] ?? null,
  );
  const primary = $derived(diag.mode === "average" ? (average?.curve ?? null) : liveCurve);
  const maxHz = $derived(
    Math.min(diag.mode === "average" && diag.averageReport ? diag.averageReport.sample_rate_hz / 2 : nyquistHz, 24_000),
  );

  const overlays = $derived.by((): PlotOverlay[] => {
    if (diag.mode === "average") {
      return average?.noise ? [{ key: "noise", curve: average.noise, tone: "noise", dashed: true }] : [];
    }
    if (diag.mode === "compare") {
      const out: PlotOverlay[] = [];
      if (diag.snapshots.a) {
        out.push({ key: "a", curve: diag.snapshots.a.curve, tone: "a" });
      }
      if (diag.snapshots.b) {
        out.push({ key: "b", curve: diag.snapshots.b.curve, tone: "b" });
      }
      return out;
    }
    return [];
  });

  const noDataText = $derived.by(() => {
    if (diag.mode === "average") {
      if (average) {
        return null;
      }
      return scope ? t("analyzer.average.empty") : t("analyzer.average.no_document");
    }
    return noOutputDevice ? t("analyzer.no_device") : null;
  });

  const report = $derived(diag.mode === "average" ? (average?.report ?? null) : diag.liveReport);
  const scopeText = $derived(
    t(diag.mode === "average" ? "analyzer.diag.average_scope" : "analyzer.diag.live_scope", {
      seconds: formatNumber(report?.span_s ?? 0, 1),
    }),
  );

  const averageSummary = $derived.by(() => {
    const r = diag.averageReport;
    if (!r || r.sample_rate_hz <= 0) {
      return null;
    }
    const whole = scope ? !scope.selection : false;
    return t("analyzer.average.summary", {
      scope: t(whole ? "analyzer.average.scope_file" : "analyzer.average.scope_selection"),
      duration: formatWithUnit((r.end_sample - r.start_sample) / r.sample_rate_hz, "s", 1),
    });
  });

  function freeze(slot: SnapshotSlot): void {
    const c = liveCurve;
    if (c) {
      freezeSnapshot(slot, "live", c);
    }
  }

  async function chooseResponse(r: AnalyzerResponseDto): Promise<void> {
    await setAnalyzerResponse(r);
  }
</script>

{#snippet legend()}
  {#each overlays as o (o.key)}
    <span class="legend-chip" data-tone={o.tone}>
      <span class="swatch" data-tone={o.tone}></span>
      {#if o.tone === "noise"}
        {t("analyzer.average.room_tone")}
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

<section class="analyzer-panel" data-testid="analyzer-panel">
  <div class="header">
    <span class="title">{t("panel.analyzer.title")}</span>
    <SegmentedControl
      options={modeOptions}
      value={diag.mode}
      label={t("analyzer.mode.label")}
      size="sm"
      testid="analyzer-mode"
      onchange={setAnalyzerMode}
    />
    {#if diag.mode !== "average"}
      <SegmentedControl
        options={responseOptions}
        value={analyzer.response}
        label={t("panel.analyzer.title")}
        size="sm"
        onchange={chooseResponse}
      />
    {/if}
    <label class="axis-picker">
      <span>{t("analyzer.floor")}</span>
      <select data-testid="analyzer-floor" bind:value={floorDb}>
        {#each ANALYZER_FLOOR_OPTIONS_DB as v (v)}
          <option value={v}>{formatWithUnit(v, "dB", 0)}</option>
        {/each}
      </select>
    </label>
    <label class="axis-picker">
      <span>{t("analyzer.ceiling")}</span>
      <select data-testid="analyzer-ceiling" bind:value={ceilDb}>
        {#each ANALYZER_CEIL_OPTIONS_DB as v (v)}
          <option value={v}>{formatWithUnit(v, "dB", 0)}</option>
        {/each}
      </select>
    </label>
    <span class="spacer"></span>
    <ToggleButton
      size="sm"
      icon="marker"
      pressed={diag.prefs.peak_labels}
      testid="analyzer-peaks-toggle"
      onchange={setPeakLabels}
    >
      {t("analyzer.peaks")}
    </ToggleButton>
    {#if diag.mode !== "average"}
      <Toggle bind:checked={analyzer.peakHold} label={t("analyzer.peak_hold")} size="sm" />
    {/if}
    <ToggleButton
      size="sm"
      icon="info"
      pressed={diag.prefs.panel_visible}
      testid="analyzer-diagnostics-toggle"
      onchange={setDiagnosticsPanelVisible}
    >
      {t("analyzer.diagnostics")}
    </ToggleButton>
    <IconButton
      icon="analyzer"
      size="sm"
      label={t("analyzer.inspector_open")}
      pressed={diag.inspectorOpen}
      testid="analyzer-open-inspector"
      onclick={() => setInspectorOpen(!diag.inspectorOpen)}
    />
  </div>

  {#if diag.mode === "average"}
    <div class="mode-bar" data-testid="analyzer-average-bar">
      <SegmentedControl
        options={sourceOptions}
        value={diag.averageSource}
        label={t("analyzer.average.signal")}
        size="sm"
        disabled={jobRunning}
        onchange={setAverageSource}
      />
      {#if jobRunning}
        <div
          class="progress"
          role="progressbar"
          aria-label={t("analyzer.average.progress", { pct: jobPct })}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={jobPct}
          data-testid="analyzer-average-progress"
        >
          <span class="progress-fill" style:width="{jobPct}%"></span>
        </div>
        <span class="muted">{t("analyzer.average.progress", { pct: jobPct })}</span>
        <Button size="sm" variant="ghost" testid="analyzer-average-cancel" onclick={cancelAverage}>
          {t("analyzer.average.cancel")}
        </Button>
      {:else}
        <Button size="sm" icon="analyzer" disabled={!scope} testid="analyzer-average-analyze" onclick={() => void startAverage()}>
          {average ? t("analyzer.average.reanalyze") : t("analyzer.average.analyze")}
        </Button>
        {#if averageSummary}
          <span class="muted">{averageSummary}</span>
        {/if}
      {/if}
    </div>
  {:else if diag.mode === "compare"}
    <div class="mode-bar" data-testid="analyzer-compare-bar">
      <Button size="sm" disabled={!liveCurve} testid="analyzer-freeze-a" title={t("analyzer.compare.freeze_a_tooltip")} onclick={() => freeze("a")}>
        {t("analyzer.compare.freeze_a")}
      </Button>
      <Button size="sm" disabled={!liveCurve} testid="analyzer-freeze-b" title={t("analyzer.compare.freeze_b_tooltip")} onclick={() => freeze("b")}>
        {t("analyzer.compare.freeze_b")}
      </Button>
      <Button
        size="sm"
        disabled={!scope || jobRunning}
        testid="analyzer-source-vs-processed"
        title={t("analyzer.compare.source_vs_processed_tooltip")}
        onclick={() => void startSourceVsProcessed()}
      >
        {t("analyzer.compare.source_vs_processed")}
      </Button>
      {#if jobRunning}
        <div class="progress" role="progressbar" aria-label={t("analyzer.average.progress", { pct: jobPct })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={jobPct}>
          <span class="progress-fill" style:width="{jobPct}%"></span>
        </div>
        <Button size="sm" variant="ghost" onclick={cancelAverage}>{t("analyzer.average.cancel")}</Button>
      {/if}
      <span class="spacer"></span>
      <Button size="sm" variant="ghost" disabled={!diag.snapshots.a && !diag.snapshots.b} testid="analyzer-compare-clear" onclick={clearSnapshots}>
        {t("analyzer.compare.clear")}
      </Button>
    </div>
  {/if}

  <div class="body">
    <SpectrumPlot
      curve={primary}
      {overlays}
      {maxHz}
      {floorDb}
      {ceilDb}
      peakHold={diag.mode !== "average" && analyzer.peakHold}
      peakLabels={diag.prefs.peak_labels}
      diffAB={diag.mode === "compare"}
      {resetKey}
      {noDataText}
      testid="analyzer"
      bind:zoom={zoomRange}
      {legend}
    />
    {#if diag.prefs.panel_visible}
      <div class="side">
        <DiagnosticsPanel
          {report}
          {scopeText}
          emptyText={diag.mode === "average" ? t("analyzer.diag.no_report") : t("analyzer.diag.waiting")}
          testid="analyzer-diagnostics"
          onclose={() => setDiagnosticsPanelVisible(false)}
        />
      </div>
    {/if}
  </div>
</section>

<style>
  .analyzer-panel {
    display: flex;
    flex-direction: column;
    min-width: 240px;
    flex: 1;
    background: var(--pv-bg-panel);
    border-left: var(--pv-border-width) solid var(--pv-border-subtle);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
  }

  /* H-25: the analyzer's header follows the panel-header anatomy (32 px, sm kit controls). */
  .header,
  .mode-bar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2) var(--pv-space-3);
    min-height: var(--pv-panel-header-h);
    padding: var(--pv-space-1) var(--pv-space-3);
  }

  .mode-bar {
    gap: var(--pv-space-2);
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
    font-size: var(--pv-text-xs);
  }

  .title {
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-secondary);
  }

  .axis-picker {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .axis-picker select {
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
  }

  .axis-picker select:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .spacer {
    flex: 1;
  }

  .muted {
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .progress {
    position: relative;
    width: 120px;
    height: 4px;
    border-radius: 2px;
    background: var(--pv-bg-inset);
    overflow: hidden;
  }

  .progress-fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--pv-accent);
  }

  /* H-24 item 5 / H-42: the plot (with its own axes) and, optionally, the diagnostics beside
     it — both definite-size flex children, never sized from their content. */
  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .side {
    display: flex;
    flex: 0 1 288px;
    min-width: 220px;
    min-height: 0;
    border-left: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .side > :global(*) {
    flex: 1;
  }

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
    background: var(--analyzer-compare-a);
  }

  .swatch[data-tone="b"] {
    background: var(--analyzer-compare-b);
  }

  .swatch[data-tone="noise"] {
    background: var(--analyzer-noise);
  }
</style>
