<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../i18n";
  import { formatNumber } from "../ui/units";
  import { HOT_ZONE_DB, LOUD_ZONE_DB, METER_FLOOR_DB, meterFraction, meterScaleTicks } from "./meterScale";

  /**
   * Shared vertical level-meter widget (H-112, generalized from H-41/H-48's `OutputMeter.svelte`):
   * a fixed-width column, peak+RMS fill drawn "Audition style" (a fainter, wider peak fill with a
   * solid, saturated RMS fill nested inside it) over a static safe/loud/hot colour-zone
   * background, a peak-hold tick, a collision-free dBFS scale (`meterScale.ts`, reused rather than
   * reimplemented) and Peak/RMS numeric readouts.
   *
   * Both the output meter (fixed −60 dBFS floor, its own clip lamp) and the input meter (H-112: a
   * selectable −60/−80/−120 dBFS floor, no lamp of its own — the transport bar already has one,
   * `RecordControls.svelte` — plus its own Max readout via `children`) render through this one
   * component, so there is exactly one meter-bar implementation to keep correct (the ticket's own
   * instruction: reuse `ui/src/lib/meters/`, don't grow a second one).
   *
   * No inline style here ever sets a *width*: only `height`/`top`/`bottom` react to the meter's
   * values, and the column's width comes from the stylesheet alone — this is what keeps whatever
   * sits beside it from ever resizing when the levels do (H-41 item 1).
   */

  let {
    label,
    floorDb = METER_FLOOR_DB,
    peakDbfs,
    rmsDbfs,
    holdDbfs,
    clip,
    peakReadoutDbfs,
    rmsReadoutDbfs,
    testid,
    clipLamp,
    children,
  }: {
    /** Row label and `aria-label` (already localized by the caller). */
    label: string;
    /** Scale floor, dBFS. Defaults to the output meter's fixed floor (H-41/H-48). */
    floorDb?: number;
    /** Peak bar (with ballistics): instant attack, 20 dB/s release. */
    peakDbfs: number;
    /** RMS bar (engine-windowed). */
    rmsDbfs: number;
    /** Peak-hold tick. */
    holdDbfs: number;
    /** Tints the hold tick (and, if `clipLamp` is given, lights its lamp). */
    clip: boolean;
    /** Throttled/held numeric Peak readout. */
    peakReadoutDbfs: number;
    /** Throttled/smoothed numeric RMS readout. */
    rmsReadoutDbfs: number;
    /** `data-testid` root; child parts are `{testid}-peak-fill`/`-rms-fill`/`-hold`/`-clip`/`-peak`/`-rms`. */
    testid: string;
    /** A click-to-clear clip lamp in the readouts row (the output meter has one; the input meter
     * doesn't — its clip lamp already lives in the transport bar, `RecordControls.svelte`). */
    clipLamp?: { label: string; title: string; onclick: () => void };
    /** Extra readouts appended after Peak/RMS (the input meter's Max readout, H-112 item 3). */
    children?: Snippet;
  } = $props();

  // Matches the `.tick` rule's own `line-height: var(--pv-leading-xs)` below, so the collision
  // maths (`meterScaleTicks`) reasons about the same box the browser actually lays out (H-48
  // item 2: an inaccurate, too-small line height was part of why labels could look crowded).
  const LINE_HEIGHT_PX = 16;

  let trackEl: HTMLDivElement | undefined;
  let trackHeightPx = $state(0);

  // Svelte's own `bind:clientHeight` throws where `ResizeObserver` doesn't exist (jsdom in
  // vitest); every other renderer in this codebase measures itself the same guarded way
  // (`EqGraph.svelte`, `WaveformView.svelte`, ...) rather than relying on that binding.
  $effect(() => {
    const el = trackEl;
    if (!el) {
      trackHeightPx = 0;
      return;
    }
    trackHeightPx = el.clientHeight;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        trackHeightPx = Math.max(0, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const ticks = $derived(meterScaleTicks(trackHeightPx, LINE_HEIGHT_PX, undefined, floorDb));
  const loudPct = $derived(meterFraction(LOUD_ZONE_DB, floorDb) * 100);
  const hotPct = $derived(meterFraction(HOT_ZONE_DB, floorDb) * 100);

  const peakPct = $derived(meterFraction(peakDbfs, floorDb) * 100);
  const rmsPct = $derived(meterFraction(rmsDbfs, floorDb) * 100);
  const holdPct = $derived(meterFraction(holdDbfs, floorDb) * 100);
  const ariaNowDb = $derived(Number.isFinite(peakDbfs) ? peakDbfs : floorDb);

  function readoutLabel(db: number): string {
    return Number.isFinite(db) ? formatNumber(db, 1) : t("meter.silence");
  }

  /**
   * How a tick's label sits relative to its own `top: {y}px` (only the vertical offset — this
   * must never also set `top`, or it would win over the real position already in the same style
   * attribute and every tick would collapse onto the same spot; H-41 caught this from an actual
   * rendered screenshot, not a test — see OutputMeter.test.ts's collision-position test).
   */
  function alignTransform(align: "start" | "center" | "end"): string {
    switch (align) {
      case "start":
        return "translateY(0)";
      case "end":
        return "translateY(-100%)";
      default:
        return "translateY(-50%)";
    }
  }
</script>

<div class="vertical-meter" data-testid={testid}>
  <span class="row-label">{label}</span>
  <div class="body">
    <div class="scale" aria-hidden="true">
      {#each ticks as tick (tick.db)}
        <span class="tick" style="top: {tick.y}px; transform: {alignTransform(tick.align)};">{tick.label}</span>
      {/each}
    </div>
    <div
      class="track"
      bind:this={trackEl}
      role="meter"
      aria-label={label}
      aria-valuemin={floorDb}
      aria-valuemax={0}
      aria-valuenow={ariaNowDb}
      style:--zone-loud="{loudPct}%"
      style:--zone-hot="{hotPct}%"
      style:--track-h="{trackHeightPx}px"
    >
      <div class="fill fill-peak" data-testid="{testid}-peak-fill" style:height="{peakPct}%"></div>
      <div class="fill fill-rms" data-testid="{testid}-rms-fill" style:height="{rmsPct}%"></div>
      <div class="hold" class:clip data-testid="{testid}-hold" style:bottom="{holdPct}%"></div>
    </div>
  </div>
  <div class="readouts">
    <span class="readout" data-testid="{testid}-peak">{t("meter.peak", { value: readoutLabel(peakReadoutDbfs) })}</span>
    <span class="readout" data-testid="{testid}-rms">{t("meter.rms", { value: readoutLabel(rmsReadoutDbfs) })}</span>
    {#if clipLamp}
      <button
        type="button"
        class="clip-lamp"
        class:lit={clip}
        title={clipLamp.title}
        data-testid="{testid}-clip"
        onclick={clipLamp.onclick}
      >
        {clipLamp.label}
      </button>
    {/if}
    {@render children?.()}
  </div>
</div>

<style>
  /* H-41: fixed width, fixed structure — nothing here ever measures text to size a box. */
  .vertical-meter {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    gap: var(--pv-space-1);
  }

  .row-label {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-medium);
  }

  .body {
    display: flex;
    align-items: stretch;
    gap: var(--pv-space-2);
    flex: 1;
    min-height: 0;
  }

  .scale {
    position: relative;
    width: 1.7rem;
    flex: none;
  }

  .tick {
    position: absolute;
    left: 0;
    right: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
    line-height: var(--pv-leading-xs);
    white-space: nowrap;
  }

  .track {
    /* H-48 item 2: noticeably wider than before — the bar was thin relative to the column it
       lives in — while staying a fixed rem value (never measured or set from a live level, so the
       column itself still never resizes). */
    position: relative;
    width: 2.5rem;
    flex: none;
    overflow: hidden;
    border-radius: var(--pv-radius-sm);
    background: var(--pv-meter-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .fill {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    background: linear-gradient(
      to top,
      var(--pv-meter-safe) 0%,
      var(--pv-meter-safe) var(--zone-loud),
      var(--pv-meter-caution) var(--zone-loud),
      var(--pv-meter-caution) var(--zone-hot),
      var(--pv-meter-over) var(--zone-hot),
      var(--pv-meter-over) 100%
    );
    /* `--track-h` (the track's own pixel height, inherited from `.track`) sizes the gradient in
       absolute px rather than the usual "100% of this element's own box" — so as this fill's
       height shrinks, the colour band it shows stays pinned to the same absolute dB range
       instead of re-stretching to fit whatever's left. `background-position: bottom` keeps that
       fixed-size gradient's 0 dBFS end always flush with the track's own top edge. */
    background-size: 100% var(--track-h, 100%);
    background-position: left bottom;
    background-repeat: no-repeat;
    /* H-123 (owner: "they are very leggy and slow"): a CSS transition here doubled up on the
       ballistics in `meters/ballistics.ts`, which already compute a continuous, analytically
       correct value every ~16.7 ms (real telemetry frame or animation frame) — every new value
       re-triggered a fresh transition from wherever the last one had gotten to, so the rendered
       bar perpetually chased the true value ~50-100 ms behind it and an "instant attack" visibly
       ramped in over 100 ms. No transition: render exactly what was computed, like every canvas
       renderer in this app already does. */
  }

  .fill-peak {
    opacity: 0.35;
  }

  .fill-rms {
    opacity: 0.95;
  }

  .hold {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    transform: translateY(1px);
    background: var(--pv-meter-caution);
    /* H-123: no transition here either — see `.fill`'s comment above. */
  }

  .hold.clip {
    background: var(--pv-meter-over);
  }

  .clip-lamp {
    align-self: flex-start;
    min-height: 24px; /* H-25: hit targets ≥ 24 px */
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-inset);
    color: var(--pv-text-disabled);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-medium);
    letter-spacing: 0.02em;
    cursor: pointer;
  }

  .clip-lamp:hover {
    color: var(--pv-text-tertiary);
  }

  .clip-lamp.lit {
    border-color: var(--pv-meter-over);
    background: var(--pv-meter-over);
    color: var(--pv-text-on-danger);
  }

  .clip-lamp:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 1px;
  }

  .readouts {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--pv-space-1);
  }

  .readout {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
