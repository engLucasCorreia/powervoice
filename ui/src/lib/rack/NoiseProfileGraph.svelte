<script lang="ts">
  import { Channel } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { t } from "../i18n";
  import type { NoiseProfileCurveDto, NoiseProfileStatusDto, RackSlotDto } from "../ipc/bindings";
  import { analyzerSubscribe, analyzerUnsubscribe, noiseProfileCurve } from "../ipc/commands";
  import { type AnalyzerFrame, decodeVxsa } from "../ipc/analyzer";
  import { toArrayBuffer } from "../ipc/telemetry";
  import { CoalescedCurveRequest } from "../eq/curveRequest";
  import { columnize, levelAt } from "../analyzer/plotGeometry";
  import { dbAxisTicks, yForAnalyzerDb } from "../analyzer/analyzerMath";
  import { initOutputDeviceStatus, outputDeviceStatus } from "../analyzer/outputDeviceStatus.svelte";
  import { formatNumber } from "../ui/units";
  import { formatHoverFreqHz, freqForU, frequencyTicks, uForFreq } from "../spectrum/freqAxis";
  import { createFrameClient } from "../render/frameScheduler";
  import { themeColors } from "../theme/themeColors";
  import { themeState } from "../theme/theme.svelte";
  import {
    NR_GRAPH_DEFAULT_MAX_DB,
    NR_GRAPH_DEFAULT_MIN_DB,
    liveBandFreqsHz,
    noiseProfileFreqRange,
    paramValueByKey,
    profileDbRange,
    reducedToLevelsDb,
  } from "./noiseProfilePlot";

  /**
   * The NR profile graph (H-85, SPEC-014 §2.8 item 2, §4.10): the captured print (filled), the
   * "reduced to" line (dashed, print − reduction_db × amount_pct/100) and the rack-output
   * analyzer's live spectrum, on the shared log-frequency axis (`spectrum/freqAxis.ts`, the same
   * one the analyzer panel uses — SPEC-007 §2.10 — so the print lines up with the live curve
   * pixel for pixel). Rendered in the NR slot body, above the generic parameter panel, in the
   * canvas style the EQ response graph and the transfer graph established (H-63/H-77): a
   * bordered canvas drawn on demand from the shared frame scheduler, theme-aware colours, no DSP
   * math in this file — the print's own points always come from Rust's `NoiseProfile::describe()`
   * (`noise_profile_curve`).
   *
   * H-87 adds the two §2.8 details H-85 left out: a hover readout (frequency, print level, live
   * level) in the same floating-tag style `SpectrumPlot.svelte` uses for the Analyzer panel and
   * Spectrum Inspector, and an honest "no output device" state for the live curve — reusing the
   * analyzer panel's own device-status store and its exact wording (`analyzer.no_device`, H-59)
   * rather than inventing new language for the same fact.
   */
  let {
    slotIndex,
    rackSlot,
    status,
  }: { slotIndex: number; rackSlot: RackSlotDto; status: NoiseProfileStatusDto } = $props();

  const GRAPH_HEIGHT_PX = 120;
  const DB_LABEL_GUTTER_PX = 34;
  const FREQ_LABEL_GAP_PX = 40;
  const AXIS_FONT_PX = 10;

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let curve = $state<NoiseProfileCurveDto | null>(null);
  let liveFrame = $state<AnalyzerFrame | undefined>(undefined);
  let subscriberId: number | undefined;
  /** The pointer (or keyboard-moved) crosshair position, canvas pixels; `null` = no hover. */
  let hover = $state<{ x: number; y: number } | null>(null);

  const hasPrint = $derived(status === "loaded" && (curve?.freqs_hz?.length ?? 0) > 0);
  // The live analyzer frame carries the rack's actual sample rate; without one yet, the graph
  // still shows the full 20 Hz – 24 kHz range (SPEC-014 §2.8's cap) rather than guessing.
  const freqRange = $derived(noiseProfileFreqRange(liveFrame?.sampleRateHz ?? 0));
  const dbRange: readonly [number, number] = $derived(
    hasPrint ? profileDbRange(curve!.levels_dbfs) : [NR_GRAPH_DEFAULT_MIN_DB, NR_GRAPH_DEFAULT_MAX_DB],
  );
  const reducedTo = $derived(
    hasPrint
      ? reducedToLevelsDb(
          curve!.levels_dbfs,
          paramValueByKey(rackSlot, "reduction_db"),
          paramValueByKey(rackSlot, "amount_pct"),
        )
      : [],
  );

  // H-87 (SPEC-014 §2.8: "greyed while the analyzer has no output device"): the same shared,
  // ref-counted device-status store the analyzer panel uses (`outputDeviceStatus.svelte.ts`), not
  // a second subscription — this is just another consumer.
  const device = outputDeviceStatus();
  const noOutputDevice = $derived(device.current === "not_selected" || device.current === "lost");

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

  // The live frame's band centre frequencies, shared by the draw pass and the hover readout.
  const liveFreqsHzArr = $derived(
    liveFrame ? liveBandFreqsHz(liveFrame.levelsDb.length, liveFrame.f0Hz, liveFrame.bandsPerOctave) : [],
  );
  // No output device: the live curve is never drawn or read from, even if a frame lingers from
  // just before the device went away (stale data would look like it's still live).
  const showLive = $derived(!noOutputDevice && (liveFrame?.levelsDb?.length ?? 0) > 0);

  const fetcher = new CoalescedCurveRequest<NoiseProfileCurveDto, number>(
    (slot) => noiseProfileCurve(slot),
    (result) => (curve = result),
  );

  // The print changes only on a capture or a Clear (both a `status` transition), never on a
  // parameter drag — unlike the EQ/transfer graphs, this isn't refetched every frame.
  $effect(() => {
    void status;
    fetcher.request(slotIndex);
  });
  $effect(() => () => fetcher.cancel());

  function onAnalyzerMessage(message: unknown): void {
    const buf = toArrayBuffer(message);
    const decoded = buf ? decodeVxsa(buf) : null;
    if (decoded) {
      liveFrame = decoded;
    }
  }

  $effect(() => {
    let cancelled = false;
    let id: number | undefined;
    analyzerSubscribe(new Channel<ArrayBuffer>((m) => onAnalyzerMessage(m)), "medium")
      .then((subId) => {
        if (cancelled) {
          void analyzerUnsubscribe(subId).catch(() => {});
          return;
        }
        id = subId;
        subscriberId = subId;
      })
      .catch(() => {
        // No live Tauri window (Vitest) or the rack output isn't open yet — the print still
        // draws; the live overlay simply stays absent.
      });
    return () => {
      cancelled = true;
      liveFrame = undefined;
      if (id !== undefined) {
        void analyzerUnsubscribe(id).catch(() => {});
      }
      subscriberId = undefined;
    };
  });
  onDestroy(() => {
    if (subscriberId !== undefined) {
      void analyzerUnsubscribe(subscriberId).catch(() => {});
    }
  });

  $effect(() => {
    const el = canvasEl;
    if (!el) {
      width = 0;
      return;
    }
    width = el.clientWidth;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        width = Math.max(0, Math.round(entry.contentRect.width));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const frames = createFrameClient(() => draw(), { name: "nr-profile-graph" });
  $effect(() => {
    void [
      canvasEl,
      width,
      curve,
      liveFrame,
      reducedTo,
      dbRange,
      freqRange,
      noOutputDevice,
      hover,
      themeState().revision,
    ];
    frames.invalidate();
  });
  $effect(() => () => frames.dispose());

  function fontFamily(): string {
    return (
      (canvasEl ? getComputedStyle(canvasEl).getPropertyValue("--pv-font-sans").trim() : "") ||
      "sans-serif"
    );
  }

  function draw(): void {
    if (!canvasEl || width <= 0) {
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // jsdom in tests, or a browser with no 2D canvas support
    }
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(width * dpr));
    const backingH = Math.max(1, Math.round(GRAPH_HEIGHT_PX * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    ctx.save();
    try {
      drawInner(ctx, dpr);
    } finally {
      ctx.restore();
    }
  }

  function drawInner(ctx: CanvasRenderingContext2D, dpr: number): void {
    const colors = themeColors();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, GRAPH_HEIGHT_PX);

    const [fLo, fHi] = freqRange;
    const [floorDb, ceilDb] = dbRange;
    const plotW = Math.max(1, width - DB_LABEL_GUTTER_PX);
    const xForFreq = (freqHz: number): number =>
      DB_LABEL_GUTTER_PX + uForFreq(freqHz, fLo, fHi, "log") * plotW;
    const yForDb = (db: number): number => yForAnalyzerDb(db, floorDb, ceilDb, GRAPH_HEIGHT_PX);

    // Grid: dB gutter ticks and frequency ticks (T-208 look, shared with the analyzer panel).
    ctx.strokeStyle = colors.eq.grid.css;
    ctx.lineWidth = 1;
    ctx.globalAlpha = 0.5;
    const dbTicks = dbAxisTicks(floorDb, ceilDb, GRAPH_HEIGHT_PX, 14);
    for (const tick of dbTicks) {
      const y = Math.round(tick.y) + 0.5;
      ctx.beginPath();
      ctx.moveTo(DB_LABEL_GUTTER_PX, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    const freqTicks = frequencyTicksFor(fLo, fHi, plotW);
    for (const tick of freqTicks) {
      const x = Math.round(DB_LABEL_GUTTER_PX + tick.u * plotW) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, GRAPH_HEIGHT_PX);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    ctx.font = `${AXIS_FONT_PX}px ${fontFamily()}`;
    ctx.fillStyle = colors.eq.labelText.css;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const tick of dbTicks) {
      ctx.fillText(tick.label, DB_LABEL_GUTTER_PX - 4, tick.y);
    }
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (const tick of freqTicks) {
      const x = DB_LABEL_GUTTER_PX + tick.u * plotW;
      ctx.fillText(tick.label, x, GRAPH_HEIGHT_PX - AXIS_FONT_PX - 2);
    }
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";

    // The noise print: a filled curve (SPEC-014 §2.8 "a filled line"), the analyzer/"noise"
    // reference colour — it *is* the captured noise.
    const c = curve;
    if (hasPrint && c) {
      const pts = columnize(c.freqs_hz, c.levels_dbfs, xForFreq, fLo, fHi);
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
        });
        ctx.lineTo(pts[pts.length - 1]!.x, bottomY);
        ctx.lineTo(pts[0]!.x, bottomY);
        ctx.closePath();
        ctx.globalAlpha = 0.35;
        ctx.fillStyle = colors.analyzer.noise.css;
        ctx.fill();
        ctx.globalAlpha = 1;
        ctx.strokeStyle = colors.analyzer.noise.css;
        ctx.lineWidth = colors.strokePx;
        ctx.beginPath();
        pts.forEach((p, i) => {
          const y = yForDb(Number.isFinite(p.db) ? p.db : floorDb);
          if (i === 0) {
            ctx.moveTo(p.x, y);
          } else {
            ctx.lineTo(p.x, y);
          }
        });
        ctx.stroke();
      }

      // The "reduced to" line: dashed, same colour family as the print it targets.
      if (reducedTo.length === c.freqs_hz.length) {
        const rpts = columnize(c.freqs_hz, reducedTo, xForFreq, fLo, fHi);
        ctx.save();
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = colors.analyzer.noise.css;
        ctx.lineWidth = colors.strokePx;
        ctx.beginPath();
        rpts.forEach((p, i) => {
          const y = yForDb(Number.isFinite(p.db) ? p.db : floorDb);
          if (i === 0) {
            ctx.moveTo(p.x, y);
          } else {
            ctx.lineTo(p.x, y);
          }
        });
        ctx.stroke();
        ctx.restore();
      }
    }

    // Live spectrum: the rack-output analyzer stream, labelled "Output (rack)" (§2.8). H-87:
    // never drawn without an output device — a lingering last frame would look like it's still
    // live when it's actually frozen.
    const live = liveFrame;
    if (showLive && live) {
      const pts = columnize(liveFreqsHzArr, live.levelsDb, xForFreq, fLo, fHi);
      if (pts.length > 0) {
        ctx.strokeStyle = colors.analyzer.compareA.css;
        ctx.lineWidth = colors.strokePx;
        ctx.beginPath();
        pts.forEach((p, i) => {
          const y = yForDb(Number.isFinite(p.db) ? p.db : floorDb);
          if (i === 0) {
            ctx.moveTo(p.x, y);
          } else {
            ctx.lineTo(p.x, y);
          }
        });
        ctx.stroke();
      }
    }

    // H-87 hover crosshair: a single vertical line at the pointer/keyboard position, matching
    // the Analyzer panel / Spectrum Inspector's `SpectrumPlot.svelte` crosshair (frequency only —
    // the readout's levels come from the curves, not the pointer's y).
    if (hover && hover.x >= DB_LABEL_GUTTER_PX) {
      ctx.strokeStyle = colors.analyzer.crosshair.css;
      ctx.lineWidth = 1;
      const x = Math.round(hover.x) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, GRAPH_HEIGHT_PX);
      ctx.stroke();
    }
  }

  /** Frequency ticks as a fraction `u` of the plot width (log axis), reusing the shared ladder
   * (`spectrum/freqAxis.ts`) rather than a bespoke one. */
  function frequencyTicksFor(
    fLo: number,
    fHi: number,
    plotW: number,
  ): { u: number; label: string }[] {
    return frequencyTicks(fLo, fHi, "log", plotW, FREQ_LABEL_GAP_PX).map((tick) => ({
      u: 1 - tick.y / plotW,
      label: tick.label,
    }));
  }

  // --- Hover readout (H-87, SPEC-014 §2.8: "frequency, print level, live level") ----------------

  function plotWidthPx(): number {
    return Math.max(1, width - DB_LABEL_GUTTER_PX);
  }

  /** The frequency at canvas x-coordinate `x`, or `null` outside the plot area (the dB gutter). */
  function freqAtX(x: number): number | null {
    const plotW = plotWidthPx();
    const frac = (x - DB_LABEL_GUTTER_PX) / plotW;
    if (frac < 0 || frac > 1) {
      return null;
    }
    const [fLo, fHi] = freqRange;
    return freqForU(frac, fLo, fHi, "log");
  }

  // Same floating tag the Analyzer panel / Spectrum Inspector show (`SpectrumPlot.svelte`'s
  // `.hover`), but with both the print and the live level under the cursor instead of one.
  const hoverReadout = $derived.by(() => {
    if (!hover || width <= 0) {
      return null;
    }
    const freqHz = freqAtX(hover.x);
    if (freqHz === null) {
      return null;
    }
    const c = curve;
    const printDb = levelAt({ freqsHz: c?.freqs_hz ?? [], levelsDb: c?.levels_dbfs ?? [] }, freqHz);
    const liveDb =
      showLive && liveFrame
        ? levelAt({ freqsHz: liveFreqsHzArr, levelsDb: liveFrame.levelsDb }, freqHz)
        : Number.NEGATIVE_INFINITY;
    return {
      x: hover.x,
      text: t("module.noise_reduction.graph.hover", {
        freq: formatHoverFreqHz(freqHz),
        print: formatNumber(printDb, 1),
        live: formatNumber(liveDb, 1),
      }),
    };
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

  // Keyboard reachability (the ticket: "keyboard-reachable if the neighbouring graphs are" — the
  // Analyzer panel / Spectrum Inspector plots are focusable): focusing the graph shows the
  // readout at its centre frequency; arrows step it; Escape or blur clears it.
  function handleFocus(): void {
    if (!hover) {
      hover = { x: DB_LABEL_GUTTER_PX + plotWidthPx() / 2, y: GRAPH_HEIGHT_PX / 2 };
    }
  }

  function handleBlur(): void {
    hover = null;
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey || e.metaKey || e.altKey) {
      return;
    }
    if (e.key === "Escape") {
      hover = null;
      e.preventDefault();
      return;
    }
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") {
      return;
    }
    const plotW = plotWidthPx();
    const currentX = hover?.x ?? DB_LABEL_GUTTER_PX + plotW / 2;
    const currentU = Math.min(1, Math.max(0, (currentX - DB_LABEL_GUTTER_PX) / plotW));
    const nextU = Math.min(1, Math.max(0, currentU + (e.key === "ArrowLeft" ? -0.05 : 0.05)));
    hover = { x: DB_LABEL_GUTTER_PX + nextU * plotW, y: GRAPH_HEIGHT_PX / 2 };
    e.preventDefault();
  }
</script>

<div class="nr-graph" data-testid="nr-profile-graph">
  <div class="legend-row">
    <span class="legend-item" data-testid="nr-graph-legend-print">
      <i class="swatch print"></i>{t("module.noise_reduction.graph.legend.print")}
    </span>
    <span class="legend-item" data-testid="nr-graph-legend-reduced">
      <i class="swatch reduced"></i>{t("module.noise_reduction.graph.legend.reduced_to")}
    </span>
    <span
      class="legend-item"
      class:legend-item-disabled={noOutputDevice}
      data-testid="nr-graph-legend-live"
    >
      <i class="swatch live" class:swatch-disabled={noOutputDevice}></i>{t(
        "module.noise_reduction.graph.legend.live",
      )}
      {#if noOutputDevice}
        <!-- H-59's own wording for this fact (`analyzer.no_device`), not a new phrase for it. -->
        <span class="legend-note" data-testid="nr-graph-live-no-device">
          · {t("analyzer.no_device")}
        </span>
      {/if}
    </span>
  </div>
  <!-- ARIA's "application" role is an interactive widget role (the plot takes arrow keys and
       reports a hover readout), but Svelte's a11y list doesn't count it as one — same exception
       `SpectrumPlot.svelte`'s canvas-wrap takes. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="canvas-wrap"
    role="application"
    tabindex="0"
    aria-label={t("module.noise_reduction.graph.label")}
    data-testid="nr-profile-canvas-wrap"
    onmousemove={handleMouseMove}
    onmouseleave={handleMouseLeave}
    onfocus={handleFocus}
    onblur={handleBlur}
    onkeydown={handleKeydown}
  >
    <canvas
      bind:this={canvasEl}
      class="graph"
      style={`height: ${GRAPH_HEIGHT_PX}px`}
      data-testid="nr-profile-canvas"
    ></canvas>
    {#if hoverReadout}
      <div class="hover" data-testid="nr-profile-hover" style:left="{hoverReadout.x}px">
        {hoverReadout.text}
      </div>
    {/if}
  </div>
</div>

<style>
  .nr-graph {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
  }

  .legend-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
    flex-wrap: wrap;
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
  }

  .swatch {
    display: inline-block;
    width: 10px;
    height: 2px;
    border-radius: var(--pv-radius-full);
    background: var(--analyzer-noise);
  }

  .swatch.reduced {
    background: repeating-linear-gradient(
      to right,
      var(--analyzer-noise) 0 3px,
      transparent 3px 5px
    );
  }

  /* H-87: the print/reduced swatches are thin lines; the live swatch is a dot, so "which curve is
   * which" doesn't rely on the noise/compare-a colours alone (H-85 found them nearly identical at
   * this size). */
  .swatch.live {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--analyzer-compare-a);
  }

  .swatch-disabled {
    background: var(--pv-text-tertiary) !important;
  }

  .legend-item-disabled {
    opacity: 0.5;
  }

  .legend-note {
    color: var(--pv-text-tertiary);
    white-space: nowrap;
  }

  .canvas-wrap {
    position: relative;
  }

  .canvas-wrap:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
    border-radius: var(--pv-radius-md);
  }

  .graph {
    display: block;
    width: 100%;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
  }

  /* Matches `SpectrumPlot.svelte`'s `.hover` (Analyzer panel / Spectrum Inspector) — same
   * floating tag, so the readout style is one system across every plot in the app. */
  .hover {
    position: absolute;
    top: var(--pv-space-1);
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
