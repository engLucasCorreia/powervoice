<script lang="ts">
  import { estimateLabelWidthPx, fitAxisLabels } from "../ui/axisLabels";
  import { documentState, hasDocument } from "../document/document.svelte";
  import SpectralView from "../spectrogram/SpectralView.svelte";
  import { selectionState } from "../state/selection.svelte";
  import { spectralState } from "../state/spectral.svelte";
  import { transportState } from "../state/transport.svelte";
  import {
    schedulePersistWaveformView,
    timeRulerFormatState,
    verticalZoomState,
    waveformViewApi,
  } from "../state/waveformView.svelte";
  import {
    clampStartSample,
    niceTickStepSeconds,
    pixelAtSample,
    sampleTicks,
    timeTicks,
  } from "../waveform/coords";
  import { formatSamplesValue } from "../waveform/timeFormat";
  import { formatRulerSeconds, formatRulerTime } from "../waveform/timeRuler";
  import WaveformView from "../waveform/WaveformView.svelte";

  /**
   * The editor pane (T-207, SPEC-007 §2.1/§2.3; H-12 lifts the shared ruler/scrollbar and
   * viewport persistence here): the shared time ruler, the waveform view, and — when toggled on
   * (Shift+D, `spectral.toggle`) — a spectral view below it, sharing one time axis
   * (`startSample`/`samplesPerPixel`, bound into both children) so a zoom/scroll gesture in
   * either pane updates both instantly. A 6 px divider between them drags the split ratio;
   * double-clicking it resets to 50 % (SPEC-007 §2.1). Both panes stay mounted with a `min-height`
   * floor at their collapsed ratio (0 or 100), rather than being removed, so "spectral only" and
   * "waveform only" are both reachable by dragging back (SPEC-007 §2.1). The shared scrollbar
   * sits below both panes: ruler → waveform → divider → spectral → scrollbar (SPEC-007 §2.1).
   *
   * **H-12 (SPEC-018 §2.6.5):** the shared viewport now lives in `state/waveformView.svelte.ts`
   * instead of local `$state`, so it can be persisted (debounced, per document, never marks the
   * document modified, SPEC-018 §2.4) alongside the selection and the edit cursor. Restoring it
   * on open is `WaveformView`'s job — it's the one that knows the canvas's pixel width, needed to
   * clamp (or fall back to zoom-full) the restored `samples_per_pixel` (SPEC-018 §2.6.5).
   */

  const wv = waveformViewApi();

  const doc = documentState();
  const transport = transportState();
  const selection = selectionState();
  const spectral = spectralState();
  const timeFormat = timeRulerFormatState();
  const vzoom = verticalZoomState();

  const lenSamples = $derived(doc.current.len_samples);
  const rateHz = $derived(doc.current.sample_rate_hz);
  const isOpen = $derived(hasDocument(doc.current));

  let containerEl: HTMLElement | undefined = $state();
  /** The editor's own measured width — the full pane width, including both panes' left gutter
   * (H-24 item 7: `WaveformView`'s amplitude ruler and `SpectralView`'s frequency ruler, both
   * `RULER_GUTTER_PX` wide). `canvasWidthPx` below is what the ticks/scrollbar actually need:
   * the canvas area *excluding* that gutter, matching SPEC-006 §2.1 ("the time ruler ... spans
   * the same horizontal extent as the waveform canvas, not the amplitude gutter"). */
  let viewportPx = $state(0);
  let dragging = false;

  /** Matches `WaveformView.svelte`'s `.amp-ruler` and `SpectralView.svelte`'s `.ruler` width. */
  const RULER_GUTTER_PX = 48;
  const canvasWidthPx = $derived(Math.max(0, viewportPx - RULER_GUTTER_PX));

  // H-12: persists the shared viewport plus the selection and the edit cursor (SPEC-018 §2.6.5),
  // debounced (never marks the document modified, §2.4). The cursor is the transport's
  // last-known, non-extrapolated position (`state.playhead_samples`) rather than the
  // continuously-extrapolated `playheadSamples` — the latter changes every animation frame while
  // playing, which would starve the debounce and never persist anything. H-35 adds the vertical
  // zoom (`WaveformView` owns the actual zoom gesture; this effect just re-fires when it changes).
  $effect(() => {
    if (!isOpen) {
      return;
    }
    schedulePersistWaveformView(
      wv.startSample,
      wv.samplesPerPixel,
      selection.current,
      transport.state.playhead_samples,
      timeFormat.current,
      vzoom.current,
    );
  });

  $effect(() => {
    const el = containerEl;
    if (!el) {
      viewportPx = 0;
      return;
    }
    viewportPx = el.clientWidth;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        viewportPx = Math.max(0, Math.round(entry.contentRect.width));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const maxStart = $derived(Math.max(0, lenSamples - wv.samplesPerPixel * canvasWidthPx));

  /** H-24 item 7: the document is >= 1 hour — SPEC-006 §2.5's own timecode rule ("the `hh:`
   * group only shown once the document is >= 1 hour"), reused here for the ruler's compact
   * labels so every tick in one ruler has the same shape. */
  const includeHours = $derived(rateHz > 0 && lenSamples / rateHz >= 3600);

  // T-206 (SPEC-006 §2.5): `timecode`/`seconds` share the seconds-based tick ladder (§4.2), only
  // the label differs; `samples` uses its own sample-space ladder (`coords.ts::sampleTicks`) so a
  // tick's position is never a rounded-then-reconverted seconds value.
  const ticks = $derived.by(() => {
    if (rateHz <= 0 || canvasWidthPx <= 0) {
      return [];
    }
    const viewportPxInt = Math.ceil(canvasWidthPx);
    if (timeFormat.current === "samples") {
      return sampleTicks(wv.startSample, wv.samplesPerPixel, viewportPxInt, 70).map((sample) => ({
        px: pixelAtSample(sample, wv.startSample, wv.samplesPerPixel) + RULER_GUTTER_PX,
        label: formatSamplesValue(sample),
      }));
    }
    const minGapSeconds = (70 * wv.samplesPerPixel) / rateHz;
    const step = niceTickStepSeconds(minGapSeconds);
    return timeTicks(wv.startSample, wv.samplesPerPixel, viewportPxInt, rateHz, 70).map((tick) => ({
      px: pixelAtSample(tick.sample, wv.startSample, wv.samplesPerPixel) + RULER_GUTTER_PX,
      label:
        timeFormat.current === "seconds"
          ? formatRulerSeconds(tick.seconds, step)
          : formatRulerTime(tick.seconds, step, includeHours),
    }));
  });

  // H-26: labels start 2 px right of their tick; one that would run past the ruler's right end
  // (and be clipped) or into its neighbour is dropped — the tick's grid line stays.
  const rulerLabels = $derived(
    fitAxisLabels(
      ticks.map((tick) => ({
        ...tick,
        pos: tick.px + 2,
        size: estimateLabelWidthPx(tick.label, 10),
        align: "start" as const,
      })),
      { length: viewportPx, gapPx: 6 },
    ),
  );

  function onScrollbarInput(event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    wv.startSample = clampStartSample(value, wv.samplesPerPixel, lenSamples, canvasWidthPx);
  }

  function onDividerPointerDown(event: PointerEvent): void {
    dragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  function onDividerPointerMove(event: PointerEvent): void {
    if (!dragging || !containerEl) {
      return;
    }
    const rect = containerEl.getBoundingClientRect();
    if (rect.height <= 0) {
      return;
    }
    const pct = ((event.clientY - rect.top) / rect.height) * 100;
    spectral.setSplitRatio(pct);
  }

  function onDividerPointerUp(event: PointerEvent): void {
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
  }

  function onDividerDoubleClick(): void {
    spectral.setSplitRatio(50);
  }
</script>

<main class="editor" data-testid="editor" data-tour="editor" bind:this={containerEl}>
  {#if isOpen}
    <div class="ruler" data-testid="editor-ruler">
      <div class="ruler-gutter" data-testid="editor-ruler-gutter"></div>
      {#each rulerLabels as tick (tick.px)}
        <span class="tick" style={`left: ${tick.px}px`}>{tick.label}</span>
      {/each}
    </div>
  {/if}
  <div
    class="waveform-pane"
    data-testid="editor-waveform-pane"
    style={spectral.visible ? `flex: ${spectral.splitRatio} 1 0%` : "flex: 1 1 0%"}
  >
    <WaveformView bind:startSample={wv.startSample} bind:samplesPerPixel={wv.samplesPerPixel} />
  </div>
  {#if spectral.visible}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="divider"
      data-testid="editor-divider"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize the spectral pane"
      onpointerdown={onDividerPointerDown}
      onpointermove={onDividerPointerMove}
      onpointerup={onDividerPointerUp}
      ondblclick={onDividerDoubleClick}
    ></div>
    <div
      class="spectral-pane"
      data-testid="editor-spectral-pane"
      style={`flex: ${100 - spectral.splitRatio} 1 0%`}
    >
      <SpectralView bind:startSample={wv.startSample} bind:samplesPerPixel={wv.samplesPerPixel} />
    </div>
  {/if}
  {#if isOpen}
    <input
      class="scrollbar"
      type="range"
      data-testid="editor-scrollbar"
      min="0"
      max={maxStart}
      step="1"
      value={wv.startSample}
      oninput={onScrollbarInput}
    />
  {/if}
</main>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    flex: 1;
    background: var(--pv-bg-inset);
  }

  .waveform-pane,
  .spectral-pane {
    display: flex;
    min-height: 6px;
    min-width: 0;
    overflow: hidden;
  }

  /* H-25: the waveform/spectral divider is a hairline inside a 6 px hit area, like the splitters. */
  .divider {
    position: relative;
    flex: none;
    height: 6px;
    background: var(--pv-bg-app);
    cursor: ns-resize;
  }

  .divider::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    background: var(--pv-border);
    transform: translateY(-50%);
  }

  .divider:hover::before {
    height: 2px;
    background: var(--pv-accent);
  }

  .ruler {
    position: relative;
    height: 20px;
    flex: none;
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-panel);
    overflow: hidden;
  }

  /* H-24 item 7: matches both panes' 48 px left gutter; ticks are offset past it. */
  .ruler-gutter {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 48px;
    border-right: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .tick {
    position: absolute;
    top: 3px;
    color: var(--pv-text-tertiary);
    font-family: var(--pv-font-sans);
    font-size: 10px;
    line-height: 14px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    transform: translateX(2px);
  }

  /* H-25: the horizontal scroll control drawn as a slim scrollbar, not a volume slider. */
  .scrollbar {
    flex: none;
    width: 100%;
    height: 12px;
    margin: 0;
    appearance: none;
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-panel);
    cursor: default;
  }

  .scrollbar::-webkit-slider-runnable-track {
    height: 11px;
    background: transparent;
  }

  .scrollbar::-webkit-slider-thumb {
    width: 56px;
    height: 6px;
    margin-top: 2.5px;
    appearance: none;
    border-radius: var(--pv-radius-full);
    background: var(--pv-border-strong);
  }

  .scrollbar:hover::-webkit-slider-thumb {
    background: var(--pv-text-tertiary);
  }

  .scrollbar::-moz-range-track {
    height: 11px;
    background: transparent;
  }

  .scrollbar::-moz-range-thumb {
    width: 56px;
    height: 6px;
    border: none;
    border-radius: var(--pv-radius-full);
    background: var(--pv-border-strong);
  }

  .scrollbar:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
  }
</style>
