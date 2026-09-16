<script lang="ts">
  import { t } from "../i18n";
  import { HOT_ZONE_DB, LOUD_ZONE_DB, METER_FLOOR_DB, meterFraction, meterScaleTicks } from "../meters/meterScale";
  import { clearOutputClip, transportState } from "../state/transport.svelte";
  import { formatNumber } from "../ui/units";

  /**
   * The output meter (H-41 owner request): vertical, fixed-width, readable. Peak and RMS are
   * drawn "Audition style" — a fainter, wider peak fill with a solid, saturated RMS fill nested
   * inside it (peak decays slower than the windowed RMS, so it naturally reads as the taller of
   * the two) — over a static safe/loud/hot colour-zone background, with a peak-hold tick on top.
   *
   * No inline style here ever sets a *width*: only `height`/`top`/`bottom` react to the meter's
   * values, and the column's width comes from the stylesheet alone (`.output-meter`/`.track`) —
   * this is what keeps the analyzer beside it from ever resizing when the levels do (H-41 item 1;
   * OutputMeter.test.ts asserts it structurally, not just visually).
   */

  const transport = transportState();
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

  const ticks = $derived(meterScaleTicks(trackHeightPx, LINE_HEIGHT_PX));
  const loudPct = meterFraction(LOUD_ZONE_DB) * 100;
  const hotPct = meterFraction(HOT_ZONE_DB) * 100;

  const peakPct = $derived(meterFraction(transport.meter.peakDbfs) * 100);
  const rmsPct = $derived(meterFraction(transport.meter.rmsDbfs) * 100);
  const holdPct = $derived(meterFraction(transport.meter.holdDbfs) * 100);
  const ariaNowDb = $derived(Number.isFinite(transport.meter.peakDbfs) ? transport.meter.peakDbfs : METER_FLOOR_DB);

  function label(db: number): string {
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

<div class="output-meter" data-testid="output-meter">
  <span class="row-label">{t("meter.output")}</span>
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
      aria-label={t("meter.output")}
      aria-valuemin={METER_FLOOR_DB}
      aria-valuemax={0}
      aria-valuenow={ariaNowDb}
      style:--zone-loud="{loudPct}%"
      style:--zone-hot="{hotPct}%"
      style:--track-h="{trackHeightPx}px"
    >
      <div class="fill fill-peak" data-testid="output-meter-peak-fill" style:height="{peakPct}%"></div>
      <div class="fill fill-rms" data-testid="output-meter-rms-fill" style:height="{rmsPct}%"></div>
      <div class="hold" class:clip={transport.meter.clip} data-testid="output-meter-hold" style:bottom="{holdPct}%"></div>
    </div>
  </div>
  <div class="readouts">
    <span class="readout" data-testid="meter-peak">{t("meter.peak", { value: label(transport.meter.peakReadoutDbfs) })}</span>
    <span class="readout" data-testid="meter-rms">{t("meter.rms", { value: label(transport.meter.rmsReadoutDbfs) })}</span>
    <button
      type="button"
      class="clip-lamp"
      class:lit={transport.meter.clip}
      title={t("meter.clip_title")}
      data-testid="output-meter-clip"
      onclick={clearOutputClip}
    >
      {t("meter.clip")}
    </button>
  </div>
</div>

<style>
  /* H-41: fixed width, fixed structure — nothing here ever measures text to size a box. */
  .output-meter {
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
    /* H-48 item 2: noticeably wider than before — the bar was thin relative to the 13rem column
       it lives in — while staying a fixed rem value (never measured or set from a live level, so
       the column itself still never resizes). */
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
    transition: height 100ms linear;
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
    transition: bottom 100ms linear;
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

  @media (prefers-reduced-motion: reduce) {
    .fill,
    .hold {
      transition: none;
    }
  }
</style>
