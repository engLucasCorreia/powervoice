<script lang="ts">
  import { documentState, hasDocument } from "../document/document.svelte";
  import SpectralView from "../spectrogram/SpectralView.svelte";
  import { selectionState } from "../state/selection.svelte";
  import { spectralState } from "../state/spectral.svelte";
  import { transportState } from "../state/transport.svelte";
  import { schedulePersistWaveformView, waveformViewApi } from "../state/waveformView.svelte";
  import { formatTime } from "../transport/playhead";
  import { clampStartSample, pixelAtSample, timeTicks } from "../waveform/coords";
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

  const lenSamples = $derived(doc.current.len_samples);
  const rateHz = $derived(doc.current.sample_rate_hz);
  const isOpen = $derived(hasDocument(doc.current));

  let containerEl: HTMLElement | undefined = $state();
  /** The editor's own measured width — the same width both panes' canvases render at, since
   * they're full-width block children of this flex column (there is no left gutter yet: an
   * amplitude ruler for the waveform pane, matching the spectral pane's frequency ruler width,
   * is deferred to hardening, SPEC-007 §2.1). */
  let viewportPx = $state(0);
  let dragging = false;

  // H-12: persists the shared viewport plus the selection and the edit cursor (SPEC-018 §2.6.5),
  // debounced (never marks the document modified, §2.4). The cursor is the transport's
  // last-known, non-extrapolated position (`state.playhead_samples`) rather than the
  // continuously-extrapolated `playheadSamples` — the latter changes every animation frame while
  // playing, which would starve the debounce and never persist anything.
  $effect(() => {
    if (!isOpen) {
      return;
    }
    schedulePersistWaveformView(
      wv.startSample,
      wv.samplesPerPixel,
      selection.current,
      transport.state.playhead_samples,
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

  const maxStart = $derived(Math.max(0, lenSamples - wv.samplesPerPixel * viewportPx));

  const ticks = $derived.by(() => {
    if (rateHz <= 0 || viewportPx <= 0) {
      return [];
    }
    return timeTicks(wv.startSample, wv.samplesPerPixel, Math.ceil(viewportPx), rateHz, 70).map(
      (tick) => ({
        px: pixelAtSample(tick.sample, wv.startSample, wv.samplesPerPixel),
        label: formatTime(tick.sample, rateHz),
      }),
    );
  });

  function onScrollbarInput(event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    wv.startSample = clampStartSample(value, wv.samplesPerPixel, lenSamples, viewportPx);
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

<main class="editor" data-testid="editor" bind:this={containerEl}>
  {#if isOpen}
    <div class="ruler" data-testid="editor-ruler">
      {#each ticks as tick (tick.px)}
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
  }

  .waveform-pane,
  .spectral-pane {
    display: flex;
    min-height: 6px;
    min-width: 0;
    overflow: hidden;
  }

  .divider {
    flex: none;
    height: 6px;
    cursor: ns-resize;
    background: var(--surface-border);
  }

  .divider:hover {
    background: var(--accent);
  }

  .ruler {
    position: relative;
    height: 20px;
    flex: none;
    border-bottom: 1px solid var(--wave-ruler-grid);
    background: var(--surface-panel);
    overflow: hidden;
  }

  .tick {
    position: absolute;
    top: 2px;
    color: var(--wave-ruler-text);
    font-size: 0.7rem;
    white-space: nowrap;
    transform: translateX(2px);
  }

  .scrollbar {
    flex: none;
    width: 100%;
    margin: 0;
  }
</style>
