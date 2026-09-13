<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { documentState, hasDocument } from "../document/document.svelte";
  import { t } from "../i18n";
  import { peaksGet } from "../ipc/commands";
  import { recordPeaksGet } from "../ipc/record_commands";
  import { registerAction } from "../keymap";
  import { formatTime } from "../transport/playhead";
  import { recordState } from "../state/record.svelte";
  import {
    beginDrag,
    clearSelection,
    dragTo,
    endDrag,
    selectAllOf,
    selectionState,
    shiftClickTo,
  } from "../state/selection.svelte";
  import { seek, transportState } from "../state/transport.svelte";
  import {
    clampSamplesPerPixel,
    clampStartSample,
    pickLevel,
    pixelAtSample,
    RAW_SPP,
    reduceColumns,
    sampleAtPixel,
    showsDots,
    timeTicks,
    zoomAroundSample,
    ZOOM_STEP_FACTOR,
    zoomFullSamplesPerPixel,
    zoomStep,
  } from "./coords";
  import { decodeVxpk } from "./vxpk";
  import { PeaksRequester } from "./peaksRequester";

  /**
   * The waveform view (S1-03, SPEC-006 essential subset; H-07 adds the live view while recording):
   * Canvas2D min/max fill and raw-sample polyline (ADR-009's WebGL2 primary / Canvas2D fallback
   * choice is deferred — this ticket starts with Canvas2D, per its own scope note), horizontal
   * zoom/scroll, a timecode ruler, the shared playhead (SPEC-003 §2.2's extrapolation, read from
   * the transport store — never re-derived here), and click-to-seek. HiDPI aware. Selection,
   * vertical zoom, markers and the overview strip are deferred to hardening/Slice 2 (ticket's
   * "Out" list).
   *
   * H-07: while `recordState` is recording, the document has no committed audio yet (S1-04: an
   * open take's document is empty until Stop), so instead of the normal `peaks_get` path this
   * view polls `record_peaks_get` at [`LIVE_POLL_MS`] and draws the growing take, zoomed to fit
   * (at least [`LIVE_MIN_WINDOW_SECONDS`]), plus a record-head line at the take's current length.
   */

  /** Must match `vox_engine::record::LIVE_PEAKS_SPB` (H-07). */
  const LIVE_PEAKS_SPB = 256;
  /** The live view never zooms in tighter than this many seconds of the take. */
  const LIVE_MIN_WINDOW_SECONDS = 10;
  /** Live take peaks poll rate (H-07 ticket: "~10 Hz"). */
  const LIVE_POLL_MS = 100;
  /** Same cap as `document_commands::peaks_get`'s `MAX_BUCKETS`. */
  const LIVE_MAX_BUCKETS = 65_536;

  let containerEl: HTMLDivElement | undefined = $state();
  let canvasEl: HTMLCanvasElement | undefined = $state();
  let viewportPx = $state(0);
  let heightPx = $state(200);
  let startSample = $state(0);
  let samplesPerPixel = $state(1);
  let fittedForAudio = $state<string | null>(null);
  let pointerDownClientX: number | null = null;
  /** The mousedown sample and modifier (S2-01, SPEC-006 §2.9): distinguishes click/drag/Shift+click
   * on pointerup, without re-deriving the down position from a possibly-stale pixel. */
  let pointerDownSample: number | null = null;
  let pointerDownShiftKey = false;
  let dragging = false;
  /** H-07: the last `record_peaks_get` response applied (`null`: none polled yet). */
  let liveBuckets = $state<Array<[number, number]>>([]);
  let liveStartSample = $state(0);

  const doc = documentState();
  const transport = transportState();
  const rec = recordState();
  const selection = selectionState();
  const requester = new PeaksRequester(peaksGet);

  const lenSamples = $derived(doc.current.len_samples);
  const rateHz = $derived(doc.current.sample_rate_hz);
  const isOpen = $derived(hasDocument(doc.current));
  const isRecording = $derived(rec.state.recording);

  // Zoom-full the first time a newly opened document's audio (rate + length — not just its path,
  // so Save As to a new path/format doesn't re-fit the still-unchanged audio) gets a known
  // viewport width (SPEC-006 §2.6 "zoom full at open"). Re-fits if the viewport wasn't known yet
  // when the document opened.
  $effect(() => {
    if (!isOpen) {
      fittedForAudio = null;
      return;
    }
    const audioKey = `${rateHz}:${lenSamples}`;
    if (audioKey !== fittedForAudio && viewportPx > 0) {
      fittedForAudio = audioKey;
      samplesPerPixel = zoomFullSamplesPerPixel(lenSamples, viewportPx);
      startSample = 0;
    }
  });

  // Issues a peaks_get request whenever the visible range or the document's audio_rev changes
  // (SPEC-006 §4.3). The response is applied asynchronously and redrawn on the next frame.
  $effect(() => {
    requester.setAudioRev(doc.current.audio_rev);
    if (viewportPx <= 0 || lenSamples <= 0 || rateHz <= 0) {
      return;
    }
    const spp = samplesPerPixel;
    const start = Math.max(0, Math.floor(startSample));
    const level = pickLevel(spp);
    if (level === RAW_SPP) {
      const count = Math.min(Math.ceil(viewportPx * spp) + 2, 1 << 20);
      void requester.request(start, count, spp);
    } else {
      const fetchStart = Math.floor(start / level) * level;
      const count = Math.min(Math.ceil((viewportPx * spp) / level) + 1, 65_536);
      void requester.request(fetchStart, count, spp);
    }
  });

  // H-07: while recording, keep the whole growing take zoomed to fit (floored at
  // LIVE_MIN_WINDOW_SECONDS so a very short take doesn't start over-zoomed).
  $effect(() => {
    if (!isRecording || viewportPx <= 0 || rateHz <= 0) {
      return;
    }
    const windowSamples = Math.max(rec.elapsedSamples, LIVE_MIN_WINDOW_SECONDS * rateHz);
    samplesPerPixel = zoomFullSamplesPerPixel(windowSamples, viewportPx);
    startSample = 0;
  });

  // H-07: polls record_peaks_get at ~10 Hz while recording (the document has no committed audio
  // yet, so the normal peaks_get effect above never fires: lenSamples stays 0 until Stop).
  $effect(() => {
    if (!isRecording) {
      liveBuckets = [];
      liveStartSample = 0;
      return;
    }
    let disposed = false;
    const poll = async (): Promise<void> => {
      const count = Math.min(
        Math.ceil(rec.elapsedSamples / LIVE_PEAKS_SPB) + 2,
        LIVE_MAX_BUCKETS,
      );
      let buf: ArrayBuffer;
      try {
        buf = await recordPeaksGet(0, count);
      } catch {
        return; // keep showing the last good buckets; the next poll retries
      }
      if (disposed) {
        return;
      }
      const frame = decodeVxpk(buf);
      if (frame) {
        liveBuckets = frame.buckets;
        liveStartSample = frame.startSample;
      }
    };
    void poll();
    const id = setInterval(() => void poll(), LIVE_POLL_MS);
    return () => {
      disposed = true;
      clearInterval(id);
    };
  });

  const maxStart = $derived(Math.max(0, lenSamples - samplesPerPixel * viewportPx));

  const ticks = $derived.by(() => {
    if (rateHz <= 0 || viewportPx <= 0) {
      return [];
    }
    return timeTicks(startSample, samplesPerPixel, Math.ceil(viewportPx), rateHz, 70).map(
      (tick) => ({
        px: pixelAtSample(tick.sample, startSample, samplesPerPixel),
        label: formatTime(tick.sample, rateHz),
      }),
    );
  });

  function colorToken(name: string, fallback: string): string {
    if (!canvasEl) {
      return fallback;
    }
    const value = getComputedStyle(canvasEl).getPropertyValue(name).trim();
    return value || fallback;
  }

  function draw(): void {
    if (!canvasEl || viewportPx <= 0) {
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // e.g. jsdom in tests, or a browser with no 2D canvas support
    }
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(viewportPx * dpr));
    const backingH = Math.max(1, Math.round(heightPx * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const bg = colorToken("--wave-bg", "#16171a");
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, viewportPx, heightPx);

    const centerY = heightPx / 2;
    if (isRecording) {
      // H-07: the growing take (from record_peaks_get), plus a record-head line — never the
      // normal peaks_get state, which has nothing to show until the take is committed at Stop.
      if (liveBuckets.length > 0) {
        drawColumns(ctx, liveBuckets, liveStartSample, LIVE_PEAKS_SPB, centerY);
      }
      drawRecordHead(ctx, centerY);
      ctx.restore();
      return;
    }
    const state = requester.state;
    const level = pickLevel(samplesPerPixel);
    if (state && state.level === level && state.buckets.length > 0) {
      if (level === RAW_SPP) {
        drawRawPolyline(ctx, state.buckets, state.startSample, centerY);
      } else {
        drawColumns(ctx, state.buckets, state.startSample, level, centerY);
      }
    } else if (state?.partial) {
      ctx.fillStyle = colorToken("--wave-pending", "#3a3d44");
      ctx.fillRect(0, 0, viewportPx, heightPx);
    }
    drawSelection(ctx);
    drawPlayhead(ctx, centerY);
    ctx.restore();
  }

  /** The time selection (S2-01, SPEC-006 §2.1/§2.9), clipped to the visible viewport. */
  function drawSelection(ctx: CanvasRenderingContext2D): void {
    const sel = selection.current;
    if (!sel) {
      return;
    }
    const x0 = Math.max(0, pixelAtSample(sel.startSample, startSample, samplesPerPixel));
    const x1 = Math.min(viewportPx, pixelAtSample(sel.endSample, startSample, samplesPerPixel));
    if (x1 <= x0) {
      return;
    }
    ctx.fillStyle = colorToken("--wave-selection-fill", "rgba(77, 163, 255, 0.22)");
    ctx.fillRect(x0, 0, x1 - x0, heightPx);
  }

  function drawColumns(
    ctx: CanvasRenderingContext2D,
    buckets: Array<[number, number]>,
    bucketsStartSample: number,
    level: number,
    centerY: number,
  ): void {
    const columns = reduceColumns(
      buckets,
      bucketsStartSample,
      level,
      startSample,
      samplesPerPixel,
      Math.ceil(viewportPx),
    );
    ctx.fillStyle = colorToken("--wave-fill", "#7fc8ff");
    for (let px = 0; px < columns.length; px++) {
      const column = columns[px];
      if (!column) {
        continue;
      }
      const [mn, mx] = column;
      const yTop = centerY - mx * centerY;
      const yBot = centerY - mn * centerY;
      ctx.fillRect(px, yTop, 1, Math.max(1, yBot - yTop));
    }
  }

  function drawRawPolyline(
    ctx: CanvasRenderingContext2D,
    samples: Array<[number, number]>,
    fetchStartSample: number,
    centerY: number,
  ): void {
    ctx.strokeStyle = colorToken("--wave-fill", "#7fc8ff");
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 0; i < samples.length; i++) {
      const sample = samples[i];
      if (!sample) {
        continue;
      }
      const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
      const y = centerY - sample[0] * centerY;
      if (i === 0) {
        ctx.moveTo(px, y);
      } else {
        ctx.lineTo(px, y);
      }
    }
    ctx.stroke();
    if (showsDots(samplesPerPixel)) {
      ctx.fillStyle = colorToken("--wave-fill", "#7fc8ff");
      for (let i = 0; i < samples.length; i++) {
        const sample = samples[i];
        if (!sample) {
          continue;
        }
        const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
        const y = centerY - sample[0] * centerY;
        ctx.beginPath();
        ctx.arc(px, y, 1.5, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }

  function drawPlayhead(ctx: CanvasRenderingContext2D, centerY: number): void {
    if (!isOpen) {
      return;
    }
    const px = pixelAtSample(transport.playheadSamples, startSample, samplesPerPixel);
    if (px < -1 || px > viewportPx + 1) {
      return;
    }
    ctx.strokeStyle = colorToken("--wave-playhead", "#ffb454");
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px + 0.5, 0);
    ctx.lineTo(px + 0.5, centerY * 2);
    ctx.stroke();
  }

  /** H-07: a line at the take's current length (the view is always zoomed so it's on-screen). */
  function drawRecordHead(ctx: CanvasRenderingContext2D, centerY: number): void {
    const px = pixelAtSample(rec.elapsedSamples, startSample, samplesPerPixel);
    ctx.strokeStyle = colorToken("--wave-record-head", "#ff5c5c");
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px + 0.5, 0);
    ctx.lineTo(px + 0.5, centerY * 2);
    ctx.stroke();
  }

  function zoomAt(anchorSample: number, anchorPx: number, nextSpp: number): void {
    const clampedSpp = clampSamplesPerPixel(nextSpp, lenSamples, viewportPx);
    samplesPerPixel = clampedSpp;
    startSample = clampStartSample(
      zoomAroundSample(anchorSample, anchorPx, clampedSpp),
      clampedSpp,
      lenSamples,
      viewportPx,
    );
  }

  /** `=`/`-` (SPEC-006 §2.6): centred on the playhead if visible, else the viewport centre. */
  function zoomKeyboard(direction: 1 | -1): void {
    if (viewportPx <= 0 || !isOpen) {
      return;
    }
    const playheadPx = pixelAtSample(transport.playheadSamples, startSample, samplesPerPixel);
    const anchorPx = playheadPx >= 0 && playheadPx <= viewportPx ? playheadPx : viewportPx / 2;
    const anchorSample = sampleAtPixel(anchorPx, startSample, samplesPerPixel);
    zoomAt(anchorSample, anchorPx, zoomStep(samplesPerPixel, direction, lenSamples, viewportPx));
  }

  function onWheel(event: WheelEvent): void {
    if (!isOpen || viewportPx <= 0 || !containerEl) {
      return;
    }
    event.preventDefault();
    if (event.ctrlKey) {
      const rect = containerEl.getBoundingClientRect();
      const anchorPx = event.clientX - rect.left;
      const anchorSample = sampleAtPixel(anchorPx, startSample, samplesPerPixel);
      const factor = event.deltaY > 0 ? ZOOM_STEP_FACTOR : 1 / ZOOM_STEP_FACTOR;
      zoomAt(anchorSample, anchorPx, samplesPerPixel * factor);
    } else {
      const delta = event.deltaX !== 0 ? event.deltaX : event.deltaY;
      startSample = clampStartSample(
        startSample + delta * samplesPerPixel,
        samplesPerPixel,
        lenSamples,
        viewportPx,
      );
    }
  }

  /** The document sample under `clientX`, clamped to the document (SPEC-006 §4.1). */
  function sampleAtClientX(clientX: number): number | null {
    if (!containerEl) {
      return null;
    }
    const rect = containerEl.getBoundingClientRect();
    const px = clientX - rect.left;
    return Math.max(0, Math.min(sampleAtPixel(px, startSample, samplesPerPixel), lenSamples));
  }

  /** Mousedown (SPEC-006 §2.9): Shift+click extends the far selection edge on pointerup; a plain
   * mousedown starts a live-updating drag (`state/selection.svelte.ts`), which a plain click
   * (no movement) undoes on pointerup by clearing the selection and seeking instead. */
  function onPointerDown(event: PointerEvent): void {
    if (!isOpen) {
      return;
    }
    pointerDownClientX = event.clientX;
    pointerDownShiftKey = event.shiftKey;
    pointerDownSample = sampleAtClientX(event.clientX);
    dragging = false;
    if (!event.shiftKey && pointerDownSample !== null) {
      beginDrag(pointerDownSample);
    }
  }

  /** Live-updates the drag selection (SPEC-006 §2.9: "live-updating" while dragging). */
  function onPointerMove(event: PointerEvent): void {
    if (pointerDownShiftKey || pointerDownSample === null || pointerDownClientX === null) {
      return;
    }
    if (!dragging && Math.abs(event.clientX - pointerDownClientX) >= 3) {
      dragging = true;
    }
    if (dragging) {
      const sample = sampleAtClientX(event.clientX);
      if (sample !== null) {
        dragTo(sample);
      }
    }
  }

  function onPointerUp(event: PointerEvent): void {
    const wasDragging = dragging;
    const shiftKey = pointerDownShiftKey;
    const downSample = pointerDownSample;
    pointerDownClientX = null;
    pointerDownSample = null;
    pointerDownShiftKey = false;
    dragging = false;
    if (!isOpen || downSample === null) {
      return;
    }
    const upSample = sampleAtClientX(event.clientX) ?? downSample;
    if (shiftKey) {
      shiftClickTo(upSample, transport.playheadSamples);
      return;
    }
    if (wasDragging) {
      dragTo(upSample);
      endDrag();
      return;
    }
    // A plain click (no drag): clears the selection and moves the cursor (SPEC-006 §2.9).
    endDrag();
    clearSelection();
    void seek(upSample);
  }

  /** Double-click selects the entire document (SPEC-006 §2.9, same as Ctrl+A). */
  function onDoubleClick(): void {
    if (isOpen) {
      selectAllOf(lenSamples);
    }
  }

  function onScrollbarInput(event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    startSample = clampStartSample(value, samplesPerPixel, lenSamples, viewportPx);
  }

  // The canvas container only exists while a document is open (`{#if isOpen}`), so the size
  // observer must be (re)attached whenever the element appears — not once at mount, when no
  // document is open yet (that left viewportPx at 0 and the waveform blank).
  $effect(() => {
    const el = containerEl;
    if (!el) {
      viewportPx = 0;
      return;
    }
    untrack(() => {
      viewportPx = el.clientWidth;
      heightPx = el.clientHeight || heightPx;
    });
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        viewportPx = Math.max(0, Math.round(entry.contentRect.width));
        heightPx = Math.max(1, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  onMount(() => {
    const cleanups: Array<() => void> = [
      registerAction("waveform.zoom_in", () => zoomKeyboard(1)),
      registerAction("waveform.zoom_out", () => zoomKeyboard(-1)),
      registerAction("waveform.select_all", () => {
        if (isOpen) {
          selectAllOf(lenSamples);
        }
      }),
      registerAction("waveform.deselect", () => clearSelection()),
    ];

    const requestFrame: (cb: () => void) => number =
      typeof requestAnimationFrame === "function"
        ? (cb) => requestAnimationFrame(cb)
        : (cb) => setTimeout(cb, 16) as unknown as number;
    const cancelFrame: (id: number) => void =
      typeof cancelAnimationFrame === "function"
        ? (id) => cancelAnimationFrame(id)
        : (id) => clearTimeout(id);
    let disposed = false;
    let frameId = 0;
    const loop = () => {
      if (disposed) {
        return;
      }
      draw();
      frameId = requestFrame(loop);
    };
    frameId = requestFrame(loop);
    cleanups.push(() => cancelFrame(frameId));
    cleanups.push(() => {
      disposed = true;
    });

    return () => {
      for (const cleanup of cleanups) {
        cleanup();
      }
    };
  });
</script>

<div class="waveform-view" data-testid="waveform-view">
  {#if isOpen}
    <div class="ruler" data-testid="waveform-ruler">
      {#each ticks as tick (tick.px)}
        <span class="tick" style={`left: ${tick.px}px`}>{tick.label}</span>
      {/each}
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="canvas-container"
      bind:this={containerEl}
      onwheel={onWheel}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      ondblclick={onDoubleClick}
    >
      <canvas
        bind:this={canvasEl}
        aria-label={t("waveform.canvas_label")}
        data-testid="waveform-canvas"
      ></canvas>
    </div>
    <input
      class="scrollbar"
      type="range"
      data-testid="waveform-scrollbar"
      min="0"
      max={maxStart}
      step="1"
      value={startSample}
      oninput={onScrollbarInput}
    />
  {:else}
    <p class="empty" data-testid="waveform-empty">{t("waveform.empty")}</p>
  {/if}
</div>

<style>
  .waveform-view {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    flex: 1;
    background: var(--wave-bg);
  }

  .empty {
    margin: auto;
    color: var(--text-secondary);
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

  .canvas-container {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .scrollbar {
    flex: none;
    width: 100%;
    margin: 0;
  }
</style>
