<script lang="ts">
  import { t } from "../../i18n";
  import { formatNumber } from "../../ui/units";
  import {
    formatHoverFreqHz,
    freqForU,
    frequencyTicks,
    fullFreqRange,
    uForFreq,
  } from "../../spectrum/freqAxis";
  import { estimateLabelWidthPx, fitAxisLabels, type Rect } from "../../ui/axisLabels";
  import { dbAxisTicks, yForAnalyzerDb } from "../analyzerMath";
  import { columnize, levelAt } from "../plotGeometry";
  import { createFrameClient } from "../../render/frameScheduler";
  import { themeColors } from "../../theme/themeColors";
  import { themeState } from "../../theme/theme.svelte";
  import { CoalescedCurveRequest } from "../../eq/curveRequest";
  import type { ResponseCurveDto } from "../../ipc/bindings";
  import { rackResponseCurvePreview } from "../../ipc/commands";
  import { layoutExplainAnnotations } from "./explainAnnotations";
  import {
    computeDbRange,
    eqAdviceLevels,
    eqAdviceRequestFreqs,
    f0LabelTopPx,
    markerReservedRects,
  } from "./explainGraphMath";
  import { voiceBandLabel } from "./explainLabels";
  import { EQ_MODULE_ID, previewEqOverrides } from "../eqSuggest";
  import { explainFindings } from "./prose";
  import { regionsAt, voiceBands } from "./voiceBands";
  import type { EqAction } from "../diagnosticsHints";
  import type { VoiceFinding } from "./findings";
  import type { VoiceSnapshot } from "./snapshot";
  import ExplainFindingCard from "./ExplainFindingCard.svelte";
  import type { ExplainGraphExportFrame } from "./explainExport";

  /**
   * The annotated graph itself (H-92 ticket §3): the frozen curve pair on the shared log
   * frequency axis, subtle voice-region bands, the F0/harmonic/strongest-peak markers, and H-93's
   * annotation cards with their leader lines. Purely a *view* over H-91's frozen `VoiceSnapshot`
   * and H-93's `layoutAnnotations` — no analysis or layout logic of its own beyond mapping
   * measured Hz/dB to plot pixels (`explainAnnotations.ts` does that mapping, tested without a
   * canvas).
   *
   * H-115: `showAnnotations` hides the cards and their leader lines only — the curves, bands and
   * markers they would have annotated keep drawing, so turning it off gives an unobstructed view
   * of the smoothed curve (the owner's request). `exportFrame` is bound out for
   * `explainExport.ts`: the live canvas element (already drawn, whatever the current toggles are)
   * plus the same placed-card rects/leaders this component itself draws, so the export composes
   * from exactly the layout the user is looking at rather than recomputing anything.
   */

  const MARGIN = { left: 48, right: 10, top: 10, bottom: 20 };
  const BAND_LABEL_FONT = "10px var(--pv-font-sans, sans-serif)";
  const AXIS_LABEL_FONT = "10px var(--pv-font-sans, sans-serif)";

  let {
    snapshot,
    showRaw,
    showSmoothed,
    showHarmonics,
    showBands,
    showEqAdvice,
    showAnnotations = true,
    eqBands = [],
    maxLabels,
    beneath = $bindable([]),
    exportFrame = $bindable(null),
    testid,
  }: {
    snapshot: VoiceSnapshot;
    showRaw: boolean;
    showSmoothed: boolean;
    showHarmonics: boolean;
    showBands: boolean;
    showEqAdvice: boolean;
    /** H-115: hides the annotation cards and their leader lines only (curves/bands/markers keep
     * drawing) — the owner's "let me see the whole smoothed graph" request. */
    showAnnotations?: boolean;
    /** H-101: H-94's conservative EQ suggestions, drawn as a dashed curve over — never
     * modifying — the measured spectrum, behind `showEqAdvice`. */
    eqBands?: EqAction[];
    /** How many cards the graph itself tries to fit (desktop: more; phone: 3, per the ticket). */
    maxLabels: number;
    /** Findings with no card on the graph — bound out for the modal's "also measured" list. */
    beneath?: VoiceFinding[];
    /** H-115: the live canvas plus the current placed-card layout, bound out for
     * `explainExport.ts` — `null` until the canvas has a real size. */
    exportFrame?: ExplainGraphExportFrame | null;
    testid: string;
  } = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);
  let hover: { x: number; y: number } | null = $state(null);

  const maxHz = $derived(snapshot.sampleRateHz > 0 ? Math.min(snapshot.sampleRateHz / 2, 24_000) : 24_000);
  const range = $derived(fullFreqRange("log", maxHz));
  const dbRange = $derived(computeDbRange(snapshot.rawDb, snapshot.smoothedDb));
  const floorDb = $derived(dbRange[0]);
  const ceilDb = $derived(dbRange[1]);
  const bands = $derived(voiceBands(snapshot.pitch));

  const plot: Rect = $derived({
    x: MARGIN.left,
    y: MARGIN.top,
    width: Math.max(0, width - MARGIN.left - MARGIN.right),
    height: Math.max(0, height - MARGIN.top - MARGIN.bottom),
  });

  function xForFreq(freqHz: number): number {
    const [fLo, fHi] = range;
    return plot.x + uForFreq(freqHz, fLo, fHi, "log") * plot.width;
  }

  function yForDb(db: number): number {
    return plot.y + yForAnalyzerDb(db, floorDb, ceilDb, plot.height);
  }

  function freqForX(x: number): number {
    const [fLo, fHi] = range;
    return freqForU((x - plot.x) / Math.max(1, plot.width), fLo, fHi, "log");
  }

  const freqTicks = $derived.by(() => {
    if (plot.width <= 0) {
      return [];
    }
    const [fLo, fHi] = range;
    return frequencyTicks(fLo, fHi, "log", plot.width, 42);
  });
  const dbTicks = $derived.by(() => (plot.height > 0 ? dbAxisTicks(floorDb, ceilDb, plot.height, 20) : []));

  /** Which band names have room to draw without overlapping a neighbour, narrowest plot first
   * (H-102: on a phone-width plot all seven band names drawn unconditionally ran into each other
   * — "RumbFundamentalmidsMidrangePresencesibilancAir"). The shaded bands themselves always draw;
   * this only decides which get a text label, the same way `frequencyTicks` already drops axis
   * ticks that don't fit. */
  const visibleBandLabelIds = $derived.by(() => {
    if (plot.width <= 0) {
      return new Set<string>();
    }
    const specs = bands.map((band) => {
      const midHz = Math.sqrt(band.lowHz * band.highHz);
      const width = estimateLabelWidthPx(voiceBandLabel(band.id), 10);
      const x = Math.min(Math.max(xForFreq(midHz), plot.x + 24), plot.x + plot.width - 24) - plot.x;
      return { id: band.id, pos: x, size: width, align: "center" as const };
    });
    const kept = fitAxisLabels(specs, { length: plot.width, gapPx: 6 });
    return new Set(kept.map((k) => k.id));
  });

  const avoidRects: Rect[] = $derived(
    snapshot.peaks.map((p) => ({ x: xForFreq(p.freqHz) - 26, y: yForDb(p.levelDb) - 10, width: 52, height: 20 })),
  );

  /** Where the F0 dashed line's own label sits — shared by the reserved-rect computation below
   * and the harmonic-label collision check in `draw()`, so the two can never disagree. */
  const f0X = $derived(snapshot.pitch ? xForFreq(snapshot.pitch.fundamentalHz) : null);

  const harmonicMarkers = $derived(
    showHarmonics
      ? snapshot.harmonics
          .filter((h) => h.status === "supported" && h.peakHz !== null && Number.isFinite(h.levelDb))
          .map((h) => ({ n: h.n, x: xForFreq(h.peakHz!), y: yForDb(h.levelDb) }))
      : [],
  );

  const strongestPeakMarker = $derived(
    snapshot.strongestPeak && !snapshot.strongestPeak.isFundamental
      ? { x: xForFreq(snapshot.strongestPeak.freqHz), y: yForDb(snapshot.strongestPeak.levelDb) }
      : null,
  );

  /** Hard reservations for the graph's own chrome (H-102 ticket §3/§5): the band-label row, the
   * F0 line's label, every drawn harmonic marker's label and the strongest-peak marker's label —
   * an annotation card must never be placed on top of one of these. */
  const markerReserved: Rect[] = $derived(
    markerReservedRects({
      plot,
      f0X,
      harmonics: harmonicMarkers,
      strongestPeak: strongestPeakMarker,
      showBandLabels: showBands,
    }),
  );

  const hoverReadout = $derived.by(() => {
    if (!hover || plot.width <= 0) {
      return null;
    }
    const freqHz = freqForX(hover.x);
    const rawDb = levelAt({ freqsHz: snapshot.freqsHz, levelsDb: snapshot.rawDb }, freqHz);
    const envDb = levelAt({ freqsHz: snapshot.freqsHz, levelsDb: snapshot.smoothedDb }, freqHz);
    const region = regionsAt(freqHz, bands);
    const freq = formatHoverFreqHz(freqHz);
    const raw = Number.isFinite(rawDb) ? `${formatNumber(rawDb, 1)} dB` : t("meter.silence");
    const env = Number.isFinite(envDb) ? `${formatNumber(envDb, 1)} dB` : t("meter.silence");
    const text =
      region.length > 0
        ? t("explain.hover_region", { freq: freq, raw: raw, env: env, region: region.map(voiceBandLabel).join(" / ") })
        : t("explain.hover", { freq: freq, raw: raw, env: env });
    return { text, x: hover.x };
  });

  // --- EQ-suggestion overlay (H-101): a dashed curve = the measured envelope plus H-94's
  // conservative EQ moves, drawn over — never modifying — the measured spectrum. The backend
  // evaluates the actual `ResponseCurve` from parameters alone (SPEC-015 §2.6.3 amendment); this
  // component only asks for it and adds it to the already-measured envelope (AC-17: no filter
  // math here, only `+`).
  const eqOverrides = $derived(previewEqOverrides(eqBands));
  let eqCurve: ResponseCurveDto | null = $state(null);

  const eqFetcher = new CoalescedCurveRequest<ResponseCurveDto>(
    (points) => rackResponseCurvePreview(EQ_MODULE_ID, eqOverrides, points),
    (result) => (eqCurve = result),
  );

  $effect(() => {
    if (!showEqAdvice || eqOverrides.length === 0 || plot.width <= 0) {
      eqCurve = null;
      eqFetcher.cancel();
      return;
    }
    const [fLo, fHi] = range;
    eqFetcher.request(eqAdviceRequestFreqs(fLo, fHi, plot.width));
  });

  $effect(() => () => eqFetcher.cancel());

  const eqAdviceDrawn = $derived.by(() => {
    const curve = eqCurve;
    return showEqAdvice && !!curve && curve.total_db.length > 0;
  });

  /** Reserves a small corner box for the overlay's own legend text, the same way every other
   * piece of chrome this graph draws is reserved (H-102) — an annotation card must not land on
   * top of it either. */
  const eqAdviceLegendRect: Rect | null = $derived.by(() => {
    if (!eqAdviceDrawn || plot.width <= 0) {
      return null;
    }
    const label = t("explain.eq_advice_label");
    const width = estimateLabelWidthPx(label, 10) + 8;
    const height = 16;
    return { x: plot.x + plot.width - width, y: plot.y + plot.height - height, width, height };
  });

  const reserved: Rect[] = $derived([
    ...(hoverReadout ? [{ x: hoverReadout.x - 140, y: plot.y, width: 280, height: 22 }] : []),
    ...markerReserved,
    ...(eqAdviceLegendRect ? [eqAdviceLegendRect] : []),
  ]);

  const prose = $derived(explainFindings(snapshot));

  const layout = $derived.by(() =>
    plot.width > 0 && plot.height > 0
      ? layoutExplainAnnotations(
          snapshot.findings,
          prose,
          {
            xForFreq,
            yForCurveAtFreq: (freqHz: number) =>
              yForDb(levelAt({ freqsHz: snapshot.freqsHz, levelsDb: snapshot.smoothedDb }, freqHz)),
          },
          { rect: plot, avoid: avoidRects, reserved, maxLabels },
        )
      : { placed: [], beneath: snapshot.findings },
  );

  $effect(() => {
    beneath = layout.beneath;
  });

  // --- Drawing on demand (H-43), like every other canvas renderer's. ---------------------------

  const frames = createFrameClient(() => {
    draw();
    return false; // a frozen snapshot never animates on its own
  }, { name: "explain-graph" });

  function requestDraw(): void {
    frames.invalidate();
  }

  function strokeCurve(ctx: CanvasRenderingContext2D, levels: ArrayLike<number>): void {
    const [fLo, fHi] = range;
    const pts = columnize(snapshot.freqsHz, levels, xForFreq, fLo, fHi);
    ctx.beginPath();
    let started = false;
    for (const p of pts) {
      const y = Number.isFinite(p.db) ? yForDb(p.db) : plot.y + plot.height;
      if (!started) {
        ctx.moveTo(p.x, y);
        started = true;
      } else {
        ctx.lineTo(p.x, y);
      }
    }
    ctx.stroke();
  }

  function draw(): void {
    if (!canvasEl || width <= 0 || height <= 0) {
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // jsdom in tests
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

    // Grid at the labelled ticks.
    ctx.strokeStyle = colors.analyzer.grid.css;
    ctx.lineWidth = 1;
    ctx.globalAlpha = 0.55;
    for (const tick of freqTicks) {
      const x = Math.round(xForFreq(tick.freqHz)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, plot.y);
      ctx.lineTo(x, plot.y + plot.height);
      ctx.stroke();
    }
    for (const tick of dbTicks) {
      const y = Math.round(plot.y + tick.y) + 0.5;
      ctx.beginPath();
      ctx.moveTo(plot.x, y);
      ctx.lineTo(plot.x + plot.width, y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // Axis tick text.
    ctx.fillStyle = colors.eq.labelText.css;
    ctx.font = AXIS_LABEL_FONT;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const tick of freqTicks) {
      ctx.fillText(tick.label, xForFreq(tick.freqHz), plot.y + plot.height + 4);
    }
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const tick of dbTicks) {
      ctx.fillText(tick.label, plot.x - 4, plot.y + tick.y);
    }

    // Voice-region bands: subtle, overlap freely (H-92 ticket §3).
    if (showBands) {
      ctx.fillStyle = colors.analyzer.explainBand.css;
      for (const band of bands) {
        const x0 = Math.max(plot.x, xForFreq(band.lowHz));
        const x1 = Math.min(plot.x + plot.width, xForFreq(band.highHz));
        if (x1 > x0) {
          ctx.fillRect(x0, plot.y, x1 - x0, plot.height);
        }
      }
      ctx.fillStyle = colors.eq.labelText.css;
      ctx.font = BAND_LABEL_FONT;
      ctx.textAlign = "center";
      ctx.textBaseline = "top";
      for (const band of bands) {
        // H-102: on a narrow (phone) plot not every band name fits without overlapping its
        // neighbour — the shaded band still draws, just without illegible overlapping text.
        if (!visibleBandLabelIds.has(band.id)) {
          continue;
        }
        const midHz = Math.sqrt(band.lowHz * band.highHz);
        const x = Math.min(Math.max(xForFreq(midHz), plot.x + 24), plot.x + plot.width - 24);
        ctx.fillText(voiceBandLabel(band.id), x, plot.y + 2);
      }
    }

    ctx.save();
    ctx.beginPath();
    ctx.rect(plot.x, plot.y, plot.width, plot.height);
    ctx.clip();

    if (showRaw && snapshot.rawDb.length > 0) {
      ctx.strokeStyle = colors.analyzer.noise.css;
      ctx.globalAlpha = 0.5;
      ctx.lineWidth = 1;
      strokeCurve(ctx, snapshot.rawDb);
      ctx.globalAlpha = 1;
    }
    if (showSmoothed && snapshot.smoothedDb.length > 0) {
      ctx.strokeStyle = colors.analyzer.explainEnvelope.css;
      ctx.lineWidth = colors.emphasisStrokePx;
      strokeCurve(ctx, snapshot.smoothedDb);
    }

    // H-101: the dashed EQ-suggestion overlay — the measured envelope plus the previewed
    // filter's own response (`eqAdviceLevels`), drawn over the measured curve without touching
    // it. `eqCurve` came from the backend's `ResponseCurve` evaluation (H-101,
    // `rack_response_curve_preview`); this component only maps its numbers to pixels.
    const curveForAdvice = eqCurve;
    if (eqAdviceDrawn && curveForAdvice) {
      const levels = eqAdviceLevels(curveForAdvice.freqs_hz, curveForAdvice.total_db, (f) =>
        levelAt({ freqsHz: snapshot.freqsHz, levelsDb: snapshot.smoothedDb }, f),
      );
      ctx.strokeStyle = colors.eq.curve.css;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([5, 4]);
      ctx.beginPath();
      let penDown = false;
      for (let i = 0; i < curveForAdvice.freqs_hz.length; i++) {
        const db = levels[i];
        if (db === undefined || !Number.isFinite(db)) {
          penDown = false;
          continue;
        }
        const x = xForFreq(curveForAdvice.freqs_hz[i]!);
        const y = yForDb(db);
        if (!penDown) {
          ctx.moveTo(x, y);
          penDown = true;
        } else {
          ctx.lineTo(x, y);
        }
      }
      ctx.stroke();
      ctx.setLineDash([]);

      if (eqAdviceLegendRect) {
        ctx.fillStyle = colors.eq.curve.css;
        ctx.font = BAND_LABEL_FONT;
        ctx.textAlign = "right";
        ctx.textBaseline = "bottom";
        ctx.fillText(
          t("explain.eq_advice_label"),
          eqAdviceLegendRect.x + eqAdviceLegendRect.width - 4,
          eqAdviceLegendRect.y + eqAdviceLegendRect.height - 2,
        );
      }
    }

    // H-102: the F0 label's top, shared with the harmonic-collision check just below and with
    // `markerReservedRects` (via the `f0X`/`markerReserved` derived values) — one formula, so the
    // drawn label, the drawn harmonic offset and the reserved rect can never drift apart.
    const f0Top = f0LabelTopPx(plot, showBands);

    if (snapshot.pitch) {
      const x = Math.round(xForFreq(snapshot.pitch.fundamentalHz)) + 0.5;
      ctx.strokeStyle = colors.analyzer.compareA.css;
      ctx.setLineDash([4, 3]);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, plot.y);
      ctx.lineTo(x, plot.y + plot.height);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = colors.analyzer.compareA.css;
      ctx.font = BAND_LABEL_FONT;
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      // Drop below the band-name row (e.g. "Fundamental") instead of sitting on top of it — a
      // voice's F0 line is always inside that band, so the two collided every time.
      ctx.fillText(t("explain.f0_label"), x + 3, f0Top + 10);
    }

    if (showHarmonics) {
      for (const h of snapshot.harmonics) {
        if (h.status === "unresolved" || h.peakHz === null || !Number.isFinite(h.levelDb)) {
          continue;
        }
        const x = xForFreq(h.peakHz);
        const y = yForDb(h.levelDb);
        ctx.fillStyle = colors.analyzer.marker.css;
        ctx.globalAlpha = h.status === "supported" ? 1 : 0.4;
        ctx.beginPath();
        ctx.arc(x, y, 3, 0, Math.PI * 2);
        ctx.fill();
        if (h.status === "supported") {
          // H-102 ticket §5: H1 sits at (or very near) the fundamental itself, so its label above
          // the dot collided with the F0 dashed line's own label. Below the F0 label's own strip,
          // the two can never be at the same y — drop below the dot instead.
          const collidesWithF0Label = f0X !== null && Math.abs(x - f0X) < 20 && y - 5 < f0Top + 20;
          ctx.font = BAND_LABEL_FONT;
          ctx.textAlign = "center";
          ctx.textBaseline = collidesWithF0Label ? "top" : "bottom";
          ctx.fillText(t("explain.harmonic_label", { n: h.n }), x, collidesWithF0Label ? y + 6 : y - 5);
        }
        ctx.globalAlpha = 1;
      }
    }

    if (snapshot.strongestPeak && !snapshot.strongestPeak.isFundamental) {
      const x = xForFreq(snapshot.strongestPeak.freqHz);
      const y = yForDb(snapshot.strongestPeak.levelDb);
      ctx.strokeStyle = colors.analyzer.compareB.css;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x, y, 5, 0, Math.PI * 2);
      ctx.stroke();
      ctx.fillStyle = colors.analyzer.compareB.css;
      ctx.font = BAND_LABEL_FONT;
      ctx.textAlign = "center";
      ctx.textBaseline = "bottom";
      ctx.fillText(t("explain.strongest_label"), x, y - 7);
    }

    if (hover) {
      ctx.strokeStyle = colors.analyzer.crosshair.css;
      ctx.lineWidth = 1;
      const x = Math.round(hover.x) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, plot.y);
      ctx.lineTo(x, plot.y + plot.height);
      ctx.stroke();
    }

    ctx.restore(); // clip
    ctx.restore(); // outer save
  }

  $effect(() => {
    void [
      snapshot,
      showRaw,
      showSmoothed,
      showHarmonics,
      showBands,
      showEqAdvice,
      eqCurve,
      width,
      height,
      hover,
      themeState().revision,
    ];
    requestDraw();
  });

  $effect(() => () => frames.dispose());

  // H-115: the export frame is a thin snapshot of what is already reactive here — the canvas
  // element itself (a live reference: whatever it holds by the time a caller actually reads it,
  // not a copy taken now) plus the current placed cards' rects/leaders and the plot rect they are
  // relative to. `showAnnotations` decides whether the caller draws them at all, so the frame
  // always carries the data and lets the composer make that call, the same way this component
  // does for its own overlay above.
  $effect(() => {
    if (!canvasEl || width <= 0 || height <= 0) {
      exportFrame = null;
      return;
    }
    exportFrame = {
      canvas: canvasEl,
      widthPx: width,
      heightPx: height,
      plot,
      cards: layout.placed.map((p) => ({
        rect: p.rect,
        title: p.item.prose.title,
        measured: p.item.prose.measured,
        severity: p.item.prose.severity,
      })),
      leaders: layout.placed.map((p) => ({ from: p.leader.from, to: p.leader.to })),
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

  function handleMouseMove(e: MouseEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect) {
      return;
    }
    hover = { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function handleMouseLeave(): void {
    hover = null;
  }

  const ariaLabel = $derived(t("explain.title"));
</script>

<div class="explain-graph" data-testid={`${testid}-root`}>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div class="canvas-wrap" tabindex="0" role="img" aria-label={ariaLabel} data-testid={`${testid}-canvas-wrap`}>
    <canvas
      bind:this={canvasEl}
      onmousemove={handleMouseMove}
      onmouseleave={handleMouseLeave}
    ></canvas>
    {#if showAnnotations}
      <svg class="leaders" aria-hidden="true">
        {#each layout.placed as p (p.item.id)}
          <line x1={p.leader.from.x} y1={p.leader.from.y} x2={p.leader.to.x} y2={p.leader.to.y} />
          <circle cx={p.leader.to.x} cy={p.leader.to.y} r="2.5" />
        {/each}
      </svg>
      {#each layout.placed as p (p.item.id)}
        <div class="annotation" style:left="{p.rect.x}px" style:top="{p.rect.y}px" style:width="{p.rect.width}px">
          <ExplainFindingCard
            prose={p.item.prose}
            {showEqAdvice}
            compact
            testid={`${testid}-annotation-${p.item.id}`}
          />
        </div>
      {/each}
    {/if}
    {#if hoverReadout}
      <div class="hover" data-testid={`${testid}-hover`} style:left="{hoverReadout.x}px">{hoverReadout.text}</div>
    {/if}
  </div>
</div>

<style>
  .explain-graph {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .canvas-wrap {
    position: relative;
    flex: 1;
    min-height: 16rem;
    overflow: hidden;
    border-radius: var(--pv-radius-md);
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

  .leaders {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .leaders line {
    stroke: var(--pv-border);
    stroke-width: 1;
  }

  .leaders circle {
    fill: var(--pv-border);
  }

  .annotation {
    position: absolute;
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
