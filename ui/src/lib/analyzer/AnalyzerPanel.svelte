<script lang="ts">
  import { SegmentedControl, Toggle, formatWithUnit, type SegmentOption } from "../ui";
  import { t } from "../i18n";
  import type { AnalyzerResponseDto } from "../ipc/bindings";
  import {
    fullFreqRange,
    frequencyTicks,
    formatHoverFreqHz,
    uForFreq,
    freqForU,
    zoomFreqRange,
    panFreqRange,
  } from "../spectrum/freqAxis";
  import {
    ANALYZER_CEIL_OPTIONS_DB,
    ANALYZER_FLOOR_OPTIONS_DB,
    edgeAlignedLabel,
    dbAxisTicks,
    DEFAULT_ANALYZER_CEIL_DB,
    DEFAULT_ANALYZER_FLOOR_DB,
    nearestAnalyzerBand,
    yForAnalyzerDb,
  } from "./analyzerMath";
  import { analyzerState, initAnalyzer, setAnalyzerResponse } from "./analyzer.svelte";
  import { initOutputDeviceStatus, outputDeviceStatus } from "./outputDeviceStatus.svelte";
  import { createPeakHold, resetPeakHold, updatePeakHold, type PeakHoldBand } from "./peakHold";

  /**
   * The live output analyzer panel (T-208/H-16, SPEC-007 §2.9): a filled spectrum curve on a log
   * frequency axis (20 Hz .. min(Nyquist, 24 kHz), zoomable/pannable — wheel to zoom around the
   * pointer, drag to pan, double-click to reset), a floor/ceiling picker, a Fast/Medium/Slow
   * response selector and a peak-hold toggle. Canvas2D (SPEC-007 §4.1: the panel is small, ≤ 246
   * points at 60 Hz). Renders in the bottom dock, to the right of the meter bridge.
   */

  const RESPONSES: AnalyzerResponseDto[] = ["fast", "medium", "slow"];

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);
  let peaks: PeakHoldBand[] = [];
  let hover: { x: number; y: number } | null = $state(null);
  let floorDb = $state<number>(DEFAULT_ANALYZER_FLOOR_DB);
  let ceilDb = $state<number>(DEFAULT_ANALYZER_CEIL_DB);
  /** `null` = full range (follows the device's Nyquist rate); set once the user zooms/pans. */
  let zoomRange: [number, number] | null = $state(null);
  let dragStartX: number | null = null;
  let dragStartRange: [number, number] | null = null;
  let dragMoved = false;

  const analyzer = analyzerState();
  // H-25: Fast / Medium / Slow as the kit's small segmented control.
  const responseOptions: SegmentOption<AnalyzerResponseDto>[] = RESPONSES.map((r) => ({
    value: r,
    label: t(`analyzer.response.${r}` as `analyzer.response.${AnalyzerResponseDto}`),
  }));
  const device = outputDeviceStatus();
  const frame = $derived(analyzer.frame);
  const nyquistHz = $derived(frame ? frame.sampleRateHz / 2 : 24_000);
  const fullRange = $derived(fullFreqRange("log", Math.min(nyquistHz, 24_000)));
  const displayRange = $derived(zoomRange ?? fullRange);
  const noOutputDevice = $derived(
    device.current === "not_selected" || device.current === "lost",
  );

  // H-24 item 5: persistent frequency (bottom) and dB (left gutter) axis labels — the analyzer
  // used to draw grid lines with no labels at all. `frequencyTicks` walks a *vertical* axis
  // (SPEC-007 §2.4/§4.7); this pane is horizontal, so only its `freqHz`/`label` are used and the
  // x position is recomputed with `xForFreq` (`draw()` already does this for the grid lines).
  const freqAxisTicks = $derived.by(() => {
    if (width <= 0) {
      return [];
    }
    const [fLo, fHi] = displayRange;
    return frequencyTicks(fLo, fHi, "log", width, 40).map((tick) => {
      const x = xForFreq(tick.freqHz);
      return { freqHz: tick.freqHz, x, label: tick.label, align: edgeAlignedLabel(x, width) };
    });
  });

  const dbTicks = $derived.by(() => (height > 0 ? dbAxisTicks(floorDb, ceilDb, height, 22) : []));

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

  $effect(() => {
    const el = canvasEl;
    if (!el) {
      width = 0;
      height = 0;
      return;
    }
    width = el.clientWidth;
    height = el.clientHeight;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        width = Math.max(0, Math.round(entry.contentRect.width));
        height = Math.max(0, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Peak-hold ballistics run per animation frame (SPEC-007 §4.8 step 7), decoupled from the
  // ~60 Hz frame arrival rate.
  $effect(() => {
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dtS = Math.max(0, Math.min(0.25, (now - last) / 1000));
      last = now;
      const f = frame;
      if (f) {
        if (f.reset) {
          resetPeakHold(peaks);
          // A device reopen/rate change invalidates a manual zoom picked against the old
          // Nyquist rate (SPEC-007 §4.8.6 treats this exactly like the analyzer's own reset).
          zoomRange = null;
        }
        if (peaks.length !== f.levelsDb.length) {
          peaks = createPeakHold(f.levelsDb.length);
        }
        if (analyzer.peakHold) {
          peaks = updatePeakHold(peaks, f.levelsDb, dtS);
        }
      }
      draw();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  function colorToken(name: string, fallback: string): string {
    if (!canvasEl) {
      return fallback;
    }
    const value = getComputedStyle(canvasEl).getPropertyValue(name).trim();
    return value || fallback;
  }

  function yForDb(db: number): number {
    return yForAnalyzerDb(db, floorDb, ceilDb, height);
  }

  function xForFreq(freqHz: number): number {
    const [fLo, fHi] = displayRange;
    return uForFreq(freqHz, fLo, fHi, "log") * width;
  }

  function draw(): void {
    if (!canvasEl || width <= 0 || height <= 0) {
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // jsdom in tests, or a browser with no 2D canvas support
    }
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(width * dpr));
    const backingH = Math.max(1, Math.round(height * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = colorToken("--surface-inset", "#16171a");
    ctx.fillRect(0, 0, width, height);

    const gridColor = colorToken("--analyzer-grid", "#34373d");
    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 1;
    ctx.globalAlpha = 0.6;
    // H-24 item 5: the grid lines sit exactly at the labeled dB/Hz ticks (10 dB/20 dB and the
    // log-frequency ladder), not an independent 12 dB spacing — so a line always has a label.
    for (const tick of dbTicks) {
      const y = Math.round(tick.y) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    for (const tick of freqAxisTicks) {
      const x = Math.round(tick.x) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    const f = frame;
    if (f && f.levelsDb.length > 0) {
      const bottomY = yForDb(floorDb);
      ctx.beginPath();
      f.levelsDb.forEach((db, k) => {
        const x = xForFreq(bandCenterHzLocal(k, f.f0Hz, f.bandsPerOctave));
        const y = yForDb(Number.isFinite(db) ? db : floorDb);
        if (k === 0) {
          ctx.moveTo(x, y);
        } else {
          ctx.lineTo(x, y);
        }
      });
      const lastX = xForFreq(
        bandCenterHzLocal(f.levelsDb.length - 1, f.f0Hz, f.bandsPerOctave),
      );
      ctx.lineTo(lastX, bottomY);
      ctx.lineTo(xForFreq(bandCenterHzLocal(0, f.f0Hz, f.bandsPerOctave)), bottomY);
      ctx.closePath();
      ctx.fillStyle = colorToken("--analyzer-fill", "rgba(127, 200, 255, 0.28)");
      ctx.fill();

      if (analyzer.peakHold && peaks.length === f.levelsDb.length) {
        ctx.strokeStyle = colorToken("--analyzer-peak", "#ffb454");
        ctx.lineWidth = 1;
        ctx.beginPath();
        peaks.forEach((band, k) => {
          if (!Number.isFinite(band.value)) {
            return;
          }
          const x = xForFreq(bandCenterHzLocal(k, f.f0Hz, f.bandsPerOctave));
          const y = yForDb(band.value);
          ctx.moveTo(x - 2, y);
          ctx.lineTo(x + 2, y);
        });
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  function bandCenterHzLocal(k: number, f0Hz: number, bandsPerOctave: number): number {
    return f0Hz * 2 ** (k / bandsPerOctave);
  }

  function handleClick(): void {
    if (dragMoved) {
      // The mouseup that ends a pan also fires a click; don't reset the hold on top of it.
      dragMoved = false;
      return;
    }
    resetPeakHold(peaks);
  }

  function handleDoubleClick(): void {
    zoomRange = null;
  }

  function handleWheel(e: WheelEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect || width <= 0) {
      return;
    }
    e.preventDefault();
    const [lo, hi] = displayRange;
    const u = (e.clientX - rect.left) / width;
    const anchorHz = freqForU(u, lo, hi, "log");
    // SPEC-007 §2.4's wheel-zoom convention: scrolling down (deltaY > 0) zooms out.
    const factor = e.deltaY > 0 ? Math.SQRT2 : Math.SQRT1_2;
    zoomRange = zoomFreqRange(lo, hi, "log", anchorHz, factor, nyquistHz);
  }

  function handleMouseDown(e: MouseEvent): void {
    dragStartX = e.clientX;
    dragStartRange = displayRange;
    dragMoved = false;
  }

  function handleMouseMove(e: MouseEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect) {
      return;
    }
    hover = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    if (dragStartX !== null && dragStartRange && width > 0) {
      if (Math.abs(e.clientX - dragStartX) > 2) {
        dragMoved = true;
      }
      const deltaFrac = -(e.clientX - dragStartX) / width;
      zoomRange = panFreqRange(dragStartRange[0], dragStartRange[1], "log", deltaFrac, nyquistHz);
    }
  }

  function endDrag(): void {
    dragStartX = null;
    dragStartRange = null;
  }

  function handleMouseLeave(): void {
    hover = null;
    endDrag();
  }

  const hoverText = $derived.by(() => {
    if (!hover || width <= 0) {
      return null;
    }
    const [fLo, fHi] = displayRange;
    const freqHz = freqForU(hover.x / width, fLo, fHi, "log");
    const f = frame;
    let dbText = t("meter.silence");
    if (f && f.levelsDb.length > 0) {
      const band = nearestAnalyzerBand(freqHz, f.f0Hz, f.bandsPerOctave, f.levelsDb.length);
      const db = f.levelsDb[band];
      if (db !== undefined && Number.isFinite(db)) {
        dbText = db.toFixed(1);
      }
    }
    return t("analyzer.hover", { freq: formatHoverFreqHz(freqHz), db: dbText });
  });

  async function chooseResponse(r: AnalyzerResponseDto): Promise<void> {
    await setAnalyzerResponse(r);
  }
</script>

<section class="analyzer-panel" data-testid="analyzer-panel">
  <div class="header">
    <span class="title">{t("panel.analyzer.title")}</span>
    <SegmentedControl
      options={responseOptions}
      value={analyzer.response}
      label={t("panel.analyzer.title")}
      size="sm"
      onchange={chooseResponse}
    />
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
    <Toggle bind:checked={analyzer.peakHold} label={t("analyzer.peak_hold")} size="sm" />
  </div>
  <div class="body">
    <div class="db-axis" data-testid="analyzer-db-axis">
      <span class="unit">{t("analyzer.unit_dbfs")}</span>
      <span class="corner-unit" data-testid="analyzer-freq-unit">{t("spectral.freq_unit")}</span>
      {#each dbTicks as tick (tick.db)}
        <span class="tick" data-align={edgeAlignedLabel(tick.y, height, 6)} style={`top: ${tick.y}px`}>{tick.label}</span>
      {/each}
    </div>
    <div class="plot">
      <div class="canvas-wrap">
        <canvas
          bind:this={canvasEl}
          onclick={handleClick}
          ondblclick={handleDoubleClick}
          onwheel={handleWheel}
          onmousedown={handleMouseDown}
          onmousemove={handleMouseMove}
          onmouseup={endDrag}
          onmouseleave={handleMouseLeave}
        ></canvas>
        {#if noOutputDevice}
          <div class="overlay">{t("analyzer.no_device")}</div>
        {:else if hoverText}
          <div class="hover" style:left="{hover?.x ?? 0}px">{hoverText}</div>
        {/if}
      </div>
      <div class="freq-axis" data-testid="analyzer-freq-axis">
        {#each freqAxisTicks as tick (tick.freqHz)}
          <span class="tick" data-align={tick.align} style={`left: ${tick.x}px`}>{tick.label}</span>
        {/each}
      </div>
    </div>
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
  .header {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2) var(--pv-space-3);
    min-height: var(--pv-panel-header-h);
    padding: var(--pv-space-1) var(--pv-space-3);
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

  /* H-24 item 5: a left dB gutter (SPEC-007 §2.9's floor/ceiling axis) and a bottom frequency
   * axis (SPEC-007 §2.4) — persistent labels, unlike the old grid-lines-with-no-text. Both are
   * plain DOM overlays (not canvas-drawn text) positioned from the same pure-math tick lists the
   * grid lines already use, so they never fight the canvas's own draw loop (item 4: no container
   * is ever sized from its own content — these are siblings with their own fixed CSS size). */
  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .db-axis {
    position: relative;
    width: 34px;
    flex: none;
    border-right: 1px solid var(--analyzer-grid);
    overflow: hidden;
  }

  .db-axis .unit,
  .db-axis .corner-unit {
    position: absolute;
    left: 2px;
    font-size: 9px;
    line-height: 12px;
    color: var(--pv-text-tertiary);
  }

  .db-axis .unit {
    top: 2px;
  }

  /* H-25: top and bottom dB labels align inside the gutter instead of straddling its edges
     (where they collided with the units). */
  .db-axis .tick[data-align="start"] {
    transform: translateY(0);
  }

  .db-axis .tick[data-align="end"] {
    transform: translateY(-100%);
  }

  .db-axis .unit {
    left: 2px;
    top: 12px;
  }

  /* H-25: the frequency unit, once, in the corner under the dB gutter (next to "20"). */
  .db-axis .corner-unit {
    bottom: 1px;
  }

  .db-axis .tick {
    position: absolute;
    right: 3px;
    transform: translateY(-50%);
    line-height: 12px;
    font-size: 10px;
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .plot {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .canvas-wrap {
    position: relative;
    flex: 1;
    min-height: 2.5rem;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .freq-axis {
    position: relative;
    flex: none;
    height: 14px;
    border-top: 1px solid var(--analyzer-grid);
    overflow: hidden;
  }

  .freq-axis .tick {
    position: absolute;
    top: 1px;
    transform: translateX(-50%);
    font-size: 10px;
    line-height: 12px;
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .freq-axis .tick[data-align="start"] {
    transform: translateX(1px);
  }

  .freq-axis .tick[data-align="end"] {
    transform: translateX(calc(-100% - 1px));
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--pv-text-tertiary);
    pointer-events: none;
  }

  .hover {
    position: absolute;
    top: 2px;
    transform: translateX(-50%);
    padding: var(--pv-space-half) var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-1);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
    white-space: nowrap;
  }
</style>
