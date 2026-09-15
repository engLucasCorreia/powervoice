<script lang="ts">
  import type { Snippet } from "svelte";
  import { estimateLabelWidthPx, fitAxisLabels, type Rect } from "../ui/axisLabels";
  import { formatNumber } from "../ui/units";
  import { t } from "../i18n";
  import {
    freqForU,
    frequencyTicks,
    formatHoverFreqHz,
    fullFreqRange,
    panFreqRange,
    uForFreq,
    zoomFreqRange,
    type FreqScale,
  } from "../spectrum/freqAxis";
  import { dbAxisTicks, yForAnalyzerDb } from "./analyzerMath";
  import { createPeakHold, resetPeakHold, updatePeakHold, type PeakHoldBand } from "./peakHold";
  import { findPeaks, type SpectralPeak } from "./peaks";
  import { placePeakLabels } from "./peakLabels";
  import { formatNote } from "./notes";
  import {
    allBelow,
    columnize,
    emaAlpha,
    levelAt,
    smoothLevels,
    type PlotCurve,
    type PlotOverlay,
  } from "./plotGeometry";
  import { themeColors, type ThemeColors } from "../theme/themeColors";
  import { themeState } from "../theme/theme.svelte";

  /**
   * The spectrum plot shared by the dock analyzer and the Spectrum Inspector (H-42, SPEC-007
   * §8.1): the analyzer's filled curve on a log (or linear) frequency axis with its dB gutter and
   * frequency strip (unchanged look, H-24/H-26), plus the H-42 overlays — frozen A/B and
   * room-tone curves, labels on the strongest peaks (frequency, note ± cents, level) with slowly
   * decaying peak markers, and a hover crosshair (frequency, level, note; B − A in Compare).
   *
   * **Drawing is on demand** (the owner's idle-CPU request, H-43 will schedule it): one animation
   * frame per input change — a new curve, hover, zoom, toggle, resize or theme switch — and more
   * only while the peak hold or the peak markers are still falling. A silent curve that is
   * already off the bottom of the axis doesn't redraw at all. `requestDraw()` is the hook.
   *
   * Pointer: wheel zooms around the pointer, drag pans, Shift-drag zooms to a range (when
   * `rangeZoom`), double-click resets, click resets the peak hold. Keyboard (the plot is
   * focusable): ←/→ pan, +/− zoom, 0 resets.
   */

  const LABEL_TAU_S = 0.4;
  const LABEL_PICK_MS = 100;
  const MARKER_HOLD_S = 1.5;
  const MARKER_FALL_DB_PER_S = 6;
  const LABEL_FONT_PX = 10;
  const LABEL_H_PX = 28;

  let {
    curve,
    overlays = [],
    scale = "log",
    maxHz,
    floorDb,
    ceilDb,
    peakHold = false,
    peakLabels = false,
    peakCount = 5,
    diffAB = false,
    rangeZoom = false,
    resetKey = 0,
    noDataText = null,
    testid,
    zoom = $bindable(null),
    onpeaks,
    legend,
  }: {
    curve: PlotCurve | null;
    overlays?: PlotOverlay[];
    scale?: FreqScale;
    /** Upper end of the full range (Nyquist, capped by the caller). */
    maxHz: number;
    floorDb: number;
    ceilDb: number;
    peakHold?: boolean;
    peakLabels?: boolean;
    peakCount?: number;
    /** Hover shows B − A when overlays `a` and `b` are both present. */
    diffAB?: boolean;
    rangeZoom?: boolean;
    /** Bump to clear the peak hold and the markers (a device reset). */
    resetKey?: number;
    noDataText?: string | null;
    /** Prefix for every `data-testid` (`analyzer`, `inspector`). */
    testid: string;
    /** `null` = the full range. */
    zoom?: [number, number] | null;
    onpeaks?: (peaks: SpectralPeak[]) => void;
    /** Drawn in the band above the plot (legends). */
    legend?: Snippet;
  } = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);
  let hover: { x: number; y: number } | null = $state(null);
  let rangeSel: { from: number; to: number } | null = $state(null);
  let labelPeaks = $state.raw<SpectralPeak[]>([]);

  let holds: PeakHoldBand[] = [];
  let markers: Array<{ freqHz: number; levelDb: number; holdS: number }> = [];
  let labelLevels: Float32Array | null = null;
  let labelCurve: PlotCurve | null = null;
  let lastCurve: PlotCurve | null = null;
  let lastCurveAt = 0;
  let lastFrameAt = 0;
  let lastPickAt = -Infinity;
  let quietDrawn = false;
  let raf = 0;

  let dragStartX: number | null = null;
  let dragStartRange: [number, number] | null = null;
  let dragMoved = false;

  const fullRange = $derived(fullFreqRange(scale, maxHz));
  const displayRange = $derived(zoom ?? fullRange);

  function xForFreq(freqHz: number): number {
    const [fLo, fHi] = displayRange;
    return uForFreq(freqHz, fLo, fHi, scale) * width;
  }

  function yForDb(db: number): number {
    return yForAnalyzerDb(db, floorDb, ceilDb, height);
  }

  // H-24/H-26 axes: persistent DOM labels fitted along each axis (unchanged from T-208).
  const freqAxisTicks = $derived.by(() => {
    if (width <= 0) {
      return [];
    }
    const [fLo, fHi] = displayRange;
    return frequencyTicks(fLo, fHi, scale, width, 40).map((tick) => ({
      freqHz: tick.freqHz,
      x: uForFreq(tick.freqHz, fLo, fHi, scale) * width,
      label: tick.label,
    }));
  });
  const dbTicks = $derived.by(() => (height > 0 ? dbAxisTicks(floorDb, ceilDb, height, 22) : []));
  const dbLabels = $derived(
    fitAxisLabels(
      dbTicks.map((tick) => ({ ...tick, pos: tick.y, size: 12 })),
      { length: height },
    ),
  );
  const freqLabels = $derived(
    fitAxisLabels(
      freqAxisTicks.map((tick) => ({ ...tick, pos: tick.x, size: estimateLabelWidthPx(tick.label, 10) })),
      { length: width, gapPx: 4 },
    ),
  );

  // --- Drawing on demand ------------------------------------------------------------------------

  export function requestDraw(): void {
    if (raf !== 0 || typeof requestAnimationFrame !== "function") {
      return;
    }
    raf = requestAnimationFrame(onFrame);
  }

  function onFrame(now: number): void {
    raf = 0;
    let again = false;
    try {
      again = step(now);
      draw();
    } finally {
      // H-32: a throw in one frame must not stop the next change from drawing — `raf` is
      // already clear, and an animation still in progress asks for its next frame.
      if (again) {
        requestDraw();
      }
    }
  }

  /** Advances the ballistics; true while something is still moving. */
  function step(now: number): boolean {
    const dtS = lastFrameAt > 0 ? Math.max(0, Math.min(0.25, (now - lastFrameAt) / 1000)) : 0;
    lastFrameAt = now;
    let animating = false;
    const c = curve;
    if (c && peakHold) {
      if (holds.length !== c.levelsDb.length) {
        holds = createPeakHold(c.levelsDb.length);
      }
      holds = updatePeakHold(holds, c.levelsDb, dtS);
      for (let i = 0; i < holds.length; i++) {
        const b = holds[i]!;
        if (b.value >= floorDb && b.value > (c.levelsDb[i] ?? -Infinity) + 0.05) {
          animating = true;
          break;
        }
      }
    }
    if (peakLabels) {
      if (labelLevels && labelCurve && now - lastPickAt >= LABEL_PICK_MS) {
        lastPickAt = now;
        pickPeaks(labelCurve, labelLevels);
      } else if (labelLevels && now - lastPickAt < LABEL_PICK_MS && lastCurveAt > lastPickAt) {
        animating = true; // a newer curve is waiting for its pick
      }
      for (const m of markers) {
        if (m.holdS > 0) {
          m.holdS = Math.max(0, m.holdS - dtS);
        } else {
          m.levelDb -= MARKER_FALL_DB_PER_S * dtS;
        }
      }
      markers = markers.filter((m) => {
        const live = c ? levelAt(c, m.freqHz) : -Infinity;
        return m.levelDb >= floorDb && m.levelDb > live + 0.05;
      });
      if (markers.length > 0) {
        animating = true;
      }
    }
    return animating;
  }

  function pickPeaks(c: PlotCurve, levels: Float32Array): void {
    const [fLo, fHi] = displayRange;
    const peaks = findPeaks(
      { freqsHz: c.freqsHz, levelsDb: levels },
      { count: peakCount, floorDb: floorDb + 6, fMinHz: Math.max(fLo, 20), fMaxHz: fHi },
    );
    for (const p of peaks) {
      const m = markers.find((k) => Math.abs(Math.log2(k.freqHz / p.freqHz)) < 1 / 12);
      if (!m) {
        markers.push({ freqHz: p.freqHz, levelDb: p.levelDb, holdS: MARKER_HOLD_S });
      } else if (p.levelDb >= m.levelDb) {
        m.freqHz = p.freqHz;
        m.levelDb = p.levelDb;
        m.holdS = MARKER_HOLD_S;
      }
    }
    labelPeaks = peaks;
    onpeaks?.(peaks);
  }

  function strokeCurve(
    ctx: CanvasRenderingContext2D,
    freqs: ArrayLike<number>,
    levels: ArrayLike<number>,
  ): void {
    const [fLo, fHi] = displayRange;
    const pts = columnize(freqs, levels, xForFreq, fLo, fHi);
    ctx.beginPath();
    let started = false;
    for (const p of pts) {
      const y = yForDb(Number.isFinite(p.db) ? p.db : floorDb);
      if (!started) {
        ctx.moveTo(p.x, y);
        started = true;
      } else {
        ctx.lineTo(p.x, y);
      }
    }
    ctx.stroke();
  }

  function overlayColor(colors: ThemeColors, tone: PlotOverlay["tone"]): string {
    switch (tone) {
      case "a":
        return colors.analyzer.compareA.css;
      case "b":
        return colors.analyzer.compareB.css;
      default:
        return colors.analyzer.noise.css;
    }
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
    const colors = themeColors();
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = colors.analyzer.bg.css;
    ctx.fillRect(0, 0, width, height);

    // Grid at the labelled ticks (H-24 item 5).
    ctx.strokeStyle = colors.analyzer.grid.css;
    ctx.lineWidth = 1;
    ctx.globalAlpha = 0.6;
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

    const [fLo, fHi] = displayRange;
    const c = curve;
    let visible = false;
    let holdVisible = false;
    if (c && c.levelsDb.length > 0) {
      // The analyzer's filled curve (T-208 look).
      const pts = columnize(c.freqsHz, c.levelsDb, xForFreq, fLo, fHi);
      if (pts.length > 0) {
        const bottomY = yForDb(floorDb);
        ctx.beginPath();
        pts.forEach((p, i) => {
          const y = yForDb(Number.isFinite(p.db) ? p.db : floorDb);
          if (i === 0) {
            ctx.moveTo(p.x, y);
          } else {
            ctx.lineTo(p.x, y);
          }
          if (p.db >= floorDb) {
            visible = true;
          }
        });
        ctx.lineTo(pts[pts.length - 1]!.x, bottomY);
        ctx.lineTo(pts[0]!.x, bottomY);
        ctx.closePath();
        ctx.fillStyle = colors.analyzer.fill.css;
        ctx.fill();
      }

      if (peakHold && holds.length === c.levelsDb.length) {
        ctx.strokeStyle = colors.analyzer.peak.css;
        ctx.lineWidth = colors.strokePx;
        if (c.resolution === "bands") {
          ctx.beginPath();
          holds.forEach((band, k) => {
            if (!Number.isFinite(band.value)) {
              return;
            }
            const f = c.freqsHz[k] ?? 0;
            if (f < fLo || f > fHi) {
              return;
            }
            const x = xForFreq(f);
            const y = yForDb(band.value);
            ctx.moveTo(x - 2, y);
            ctx.lineTo(x + 2, y);
            if (band.value >= floorDb) {
              holdVisible = true;
            }
          });
          ctx.stroke();
        } else {
          const values = new Float32Array(holds.length);
          holds.forEach((b, k) => {
            values[k] = b.value;
            if (b.value >= floorDb) {
              holdVisible = true;
            }
          });
          ctx.globalAlpha = 0.8;
          strokeCurve(ctx, c.freqsHz, values);
          ctx.globalAlpha = 1;
        }
      }
    }

    // Overlays: frozen A/B, room tone (dashed).
    for (const o of overlays) {
      ctx.strokeStyle = overlayColor(colors, o.tone);
      ctx.lineWidth = Math.max(1.5, colors.strokePx * 1.5);
      ctx.setLineDash(o.dashed ? [5, 4] : []);
      strokeCurve(ctx, o.curve.freqsHz, o.curve.levelsDb);
    }
    ctx.setLineDash([]);

    // Peak markers: a dot on each labelled peak, a slowly falling tick where it last peaked.
    if (peakLabels) {
      ctx.fillStyle = colors.analyzer.marker.css;
      ctx.strokeStyle = colors.analyzer.marker.css;
      ctx.lineWidth = colors.strokePx;
      for (const m of markers) {
        const x = xForFreq(m.freqHz);
        const y = yForDb(m.levelDb);
        ctx.globalAlpha = 0.75;
        ctx.beginPath();
        ctx.moveTo(x - 4, y);
        ctx.lineTo(x + 4, y);
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
      for (const p of labelPeaks) {
        const x = xForFreq(p.freqHz);
        const y = yForDb(p.levelDb);
        ctx.beginPath();
        ctx.arc(x, y, 3, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = colors.analyzer.bg.css;
        ctx.lineWidth = 1;
        ctx.stroke();
      }
    }

    // Crosshair.
    if (hover) {
      ctx.strokeStyle = colors.analyzer.crosshair.css;
      ctx.lineWidth = 1;
      const x = Math.round(hover.x) + 0.5;
      const y = Math.round(hover.y) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    ctx.restore();
    quietDrawn = !visible && !holdVisible && labelPeaks.length === 0 && markers.length === 0;
  }

  // A new curve: fold it into the label curve, then redraw — unless it is silent and the last
  // frame already showed nothing (the idle case: no frame at all).
  $effect(() => {
    const c = curve;
    if (c === lastCurve) {
      return;
    }
    const now = performance.now();
    const dtS = lastCurveAt > 0 ? (now - lastCurveAt) / 1000 : 0;
    lastCurveAt = now;
    lastCurve = c;
    if (!c) {
      labelLevels = null;
      labelCurve = null;
      labelPeaks = [];
      requestDraw();
      return;
    }
    const sameGrid = labelCurve !== null && labelCurve.levelsDb.length === c.levelsDb.length;
    labelLevels = smoothLevels(sameGrid ? labelLevels : null, c.levelsDb, emaAlpha(dtS, LABEL_TAU_S));
    labelCurve = c;
    if (quietDrawn && allBelow(c.levelsDb, floorDb) && labelLevels && allBelow(labelLevels, floorDb + 6)) {
      return;
    }
    requestDraw();
  });

  // Everything else that changes the picture.
  $effect(() => {
    void [overlays, scale, floorDb, ceilDb, displayRange, hover, width, height, peakHold, peakLabels];
    void [noDataText, maxHz, themeState().revision];
    quietDrawn = false;
    requestDraw();
  });

  $effect(() => {
    void resetKey;
    resetPeakHold(holds);
    markers = [];
    quietDrawn = false;
    requestDraw();
  });

  $effect(() => {
    if (!peakLabels) {
      labelPeaks = [];
      markers = [];
    }
  });

  $effect(() => () => {
    if (raf !== 0 && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(raf);
    }
    raf = 0;
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

  // --- Labels, hover ---------------------------------------------------------------------------

  function formatPeakFreq(freqHz: number): string {
    if (freqHz < 1000) {
      return `${formatNumber(freqHz, freqHz < 100 ? 1 : 0)} Hz`;
    }
    return `${formatNumber(freqHz / 1000, freqHz < 10_000 ? 2 : 1)} kHz`;
  }

  const hoverReadout = $derived.by(() => {
    if (!hover || width <= 0) {
      return null;
    }
    const [fLo, fHi] = displayRange;
    const freqHz = freqForU(hover.x / width, fLo, fHi, scale);
    const c = curve;
    let dbText = t("meter.silence");
    if (c && c.levelsDb.length > 0) {
      const db = levelAt(c, freqHz);
      if (Number.isFinite(db)) {
        dbText = formatNumber(db, 1);
      }
    }
    let text = peakLabels
      ? t("analyzer.hover_note", { freq: formatHoverFreqHz(freqHz), db: dbText, note: formatNote(freqHz) })
      : t("analyzer.hover", { freq: formatHoverFreqHz(freqHz), db: dbText });
    const a = overlays.find((o) => o.tone === "a");
    const b = overlays.find((o) => o.tone === "b");
    if (diffAB && a && b) {
      const d = levelAt(b.curve, freqHz) - levelAt(a.curve, freqHz);
      if (Number.isFinite(d)) {
        const signed = d > 0 ? `+${formatNumber(d, 1)}` : formatNumber(d, 1);
        text = `${text} · ${t("analyzer.compare.diff", { db: signed })}`;
      }
    }
    return { text, x: hover.x };
  });

  const placed = $derived.by(() => {
    if (!peakLabels || width <= 0 || height <= 0) {
      return [];
    }
    const items = labelPeaks.map((p) => {
      const top = formatPeakFreq(p.freqHz);
      const bottom = `${formatNote(p.freqHz)} · ${t("analyzer.peak_level", { db: formatNumber(p.levelDb, 1) })}`;
      const w = Math.max(estimateLabelWidthPx(top, LABEL_FONT_PX + 1), estimateLabelWidthPx(bottom, LABEL_FONT_PX)) + 12;
      return {
        key: Math.round(p.freqHz * 10),
        x: xForFreq(p.freqHz),
        y: yForDb(p.levelDb),
        width: w,
        height: LABEL_H_PX,
        top,
        bottom,
      };
    });
    const reserved: Rect[] = [];
    if (hoverReadout) {
      reserved.push({ x: hoverReadout.x - 130, y: 0, width: 260, height: 24 });
    }
    return placePeakLabels(items, { width, height, reserved });
  });

  const ariaLabel = $derived(
    t("analyzer.plot_aria", {
      peaks: labelPeaks
        .map((p) =>
          t("analyzer.plot_aria_peak", {
            freq: formatPeakFreq(p.freqHz),
            note: formatNote(p.freqHz),
            db: formatNumber(p.levelDb, 0),
          }),
        )
        .join(", "),
    }),
  );

  // --- Pointer & keyboard ----------------------------------------------------------------------

  function handleClick(): void {
    if (dragMoved) {
      // The mouseup that ends a pan also fires a click; don't reset the hold on top of it.
      dragMoved = false;
      return;
    }
    resetPeakHold(holds);
    markers = [];
    requestDraw();
  }

  function handleDoubleClick(): void {
    zoom = null;
  }

  function zoomAround(u: number, factor: number): void {
    const [lo, hi] = displayRange;
    const anchorHz = freqForU(u, lo, hi, scale);
    zoom = zoomFreqRange(lo, hi, scale, anchorHz, factor, maxHz);
  }

  function handleWheel(e: WheelEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect || width <= 0) {
      return;
    }
    e.preventDefault();
    // SPEC-007 §2.4's wheel-zoom convention: scrolling down (deltaY > 0) zooms out.
    zoomAround((e.clientX - rect.left) / width, e.deltaY > 0 ? Math.SQRT2 : Math.SQRT1_2);
  }

  function handleMouseDown(e: MouseEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (rangeZoom && e.shiftKey && rect) {
      const x = e.clientX - rect.left;
      rangeSel = { from: x, to: x };
      return;
    }
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
    if (rangeSel) {
      rangeSel = { ...rangeSel, to: hover.x };
      return;
    }
    if (dragStartX !== null && dragStartRange && width > 0) {
      if (Math.abs(e.clientX - dragStartX) > 2) {
        dragMoved = true;
      }
      const deltaFrac = -(e.clientX - dragStartX) / width;
      zoom = panFreqRange(dragStartRange[0], dragStartRange[1], scale, deltaFrac, maxHz);
    }
  }

  function endDrag(): void {
    if (rangeSel && width > 0) {
      const a = Math.max(0, Math.min(rangeSel.from, rangeSel.to));
      const b = Math.min(width, Math.max(rangeSel.from, rangeSel.to));
      if (b - a > 8) {
        const [lo, hi] = displayRange;
        zoom = [freqForU(a / width, lo, hi, scale), freqForU(b / width, lo, hi, scale)];
      }
      rangeSel = null;
      dragMoved = true;
    }
    dragStartX = null;
    dragStartRange = null;
  }

  function handleMouseLeave(): void {
    hover = null;
    rangeSel = null;
    endDrag();
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey || e.metaKey || e.altKey) {
      return;
    }
    const [lo, hi] = displayRange;
    switch (e.key) {
      case "ArrowLeft":
      case "ArrowRight":
        zoom = panFreqRange(lo, hi, scale, e.key === "ArrowLeft" ? -0.1 : 0.1, maxHz);
        break;
      case "+":
      case "=":
        zoomAround(0.5, Math.SQRT1_2);
        break;
      case "-":
      case "_":
        zoomAround(0.5, Math.SQRT2);
        break;
      case "0":
      case "Home":
        zoom = null;
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<div class="plot-root" data-testid={`${testid}-plot`}>
  <!-- H-26: the dB gutter is a column of three cells — the axis title ("dBFS") in a band above
       the plot, the tick labels beside it, and the frequency unit in the corner under it — so
       no unit ever sits on a tick label. -->
  <div class="db-axis" data-testid={`${testid}-db-axis`}>
    <span class="axis-title" data-testid={`${testid}-db-unit`}>{t("analyzer.unit_dbfs")}</span>
    <div class="db-ticks">
      {#each dbLabels as tick (tick.db)}
        <span class="tick" data-align={tick.align} style={`top: ${tick.y}px`}>{tick.label}</span>
      {/each}
    </div>
    <span class="corner-unit" data-testid={`${testid}-freq-unit`}>{t("spectral.freq_unit")}</span>
  </div>
  <div class="plot">
    <div class="title-band">
      {#if legend}{@render legend()}{/if}
    </div>
    <!-- ARIA's "application" role is an interactive widget role (the plot takes arrow/+/−/0
         keys), but Svelte's a11y list doesn't count it as one. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="canvas-wrap"
      role="application"
      tabindex="0"
      aria-label={ariaLabel}
      data-testid={`${testid}-canvas-wrap`}
      onkeydown={handleKeydown}
    >
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
      {#if rangeSel}
        <div
          class="range-sel"
          style:left="{Math.min(rangeSel.from, rangeSel.to)}px"
          style:width="{Math.abs(rangeSel.to - rangeSel.from)}px"
        ></div>
      {/if}
      {#each placed as p (p.item.key)}
        <div
          class="peak-label"
          data-testid={`${testid}-peak-label`}
          data-spot={p.spot}
          style:left="{p.rect.x}px"
          style:top="{p.rect.y}px"
          style:width="{p.rect.width}px"
        >
          <span class="peak-freq">{p.item.top}</span>
          <span class="peak-note">{p.item.bottom}</span>
        </div>
      {/each}
      {#if noDataText}
        <div class="overlay">{noDataText}</div>
      {:else if hoverReadout}
        <div class="hover" data-testid={`${testid}-hover`} style:left="{hoverReadout.x}px">{hoverReadout.text}</div>
      {/if}
    </div>
    <div class="freq-axis" data-testid={`${testid}-freq-axis`}>
      {#each freqLabels as tick (tick.freqHz)}
        <span class="tick" data-align={tick.align} style={`left: ${tick.x}px`}>{tick.label}</span>
      {/each}
    </div>
  </div>
</div>

<style>
  .plot-root {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .db-axis {
    display: flex;
    flex-direction: column;
    width: 34px;
    flex: none;
  }

  .axis-title,
  .title-band {
    flex: none;
    height: 14px;
  }

  .title-band {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: var(--pv-space-2);
    padding-right: var(--pv-space-1);
    overflow: hidden;
  }

  .axis-title,
  .corner-unit {
    padding-right: 3px;
    color: var(--pv-text-tertiary);
    font-size: 10px;
    line-height: 12px;
    text-align: right;
    white-space: nowrap;
  }

  .axis-title {
    padding-top: 1px;
  }

  .corner-unit {
    flex: none;
    height: 14px;
    border-top: 1px solid transparent;
    padding-top: 1px;
  }

  .db-ticks {
    position: relative;
    flex: 1;
    min-height: 0;
    border-right: 1px solid var(--analyzer-grid);
    overflow: hidden;
  }

  .db-ticks .tick {
    position: absolute;
    right: 3px;
    transform: translateY(-50%);
    line-height: 12px;
    font-size: 10px;
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .db-ticks .tick[data-align="start"] {
    transform: translateY(0);
  }

  .db-ticks .tick[data-align="end"] {
    transform: translateY(-100%);
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
    overflow: hidden;
  }

  .canvas-wrap:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
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
    padding: 0 var(--pv-space-4);
    color: var(--pv-text-tertiary);
    text-align: center;
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

  /* H-42 peak labels: a quiet two-line tag — frequency on top, note and level under it. */
  .peak-label {
    position: absolute;
    display: flex;
    flex-direction: column;
    justify-content: center;
    height: 28px;
    padding: 0 6px;
    border: var(--pv-border-width) solid var(--pv-border-subtle);
    border-left: 2px solid var(--analyzer-marker);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-overlay);
    pointer-events: none;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    line-height: 12px;
    overflow: hidden;
  }

  .peak-freq {
    color: var(--pv-text-primary);
    font-size: 11px;
    font-weight: var(--pv-weight-semibold);
  }

  .peak-note {
    color: var(--pv-text-secondary);
    font-size: 10px;
  }

  .range-sel {
    position: absolute;
    top: 0;
    bottom: 0;
    border-inline: 1px solid var(--pv-selection-border);
    background: var(--pv-selection-fill);
    pointer-events: none;
  }
</style>
