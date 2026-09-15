<script lang="ts">
  import { fitGutterLabels } from "../ui/axisLabels";
  import { formatNumber } from "../ui/units";
  import { onMount } from "svelte";
  import { documentState, hasDocument } from "../document/document.svelte";
  import { t } from "../i18n";
  import { spectroDetach } from "../ipc/commands";
  import { markersState } from "../markers/markers.svelte";
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
  import { CEIL_RANGE_DB, FLOOR_RANGE_DB, spectralState } from "../state/spectral.svelte";
  import { seek, transportState } from "../state/transport.svelte";
  import { formatTime } from "../transport/playhead";
  import {
    clampFreqRange,
    formatHoverFreqHz,
    freqForU,
    freqForY,
    frequencyTicks,
    fullFreqRange,
    panFreqRange,
    zoomFreqRange,
  } from "../spectrum/freqAxis";
  import {
    clampSamplesPerPixel,
    clampStartSample,
    pixelAtSample,
    sampleAtPixel,
    zoomAroundSample,
    ZOOM_STEP_FACTOR,
  } from "../waveform/coords";
  import { GlContextHost } from "../render/glContext";
  import { buildOverlayBatch } from "../render/overlayGeometry";
  import { crispOffset, themeColors } from "../theme/themeColors";
  import { pushNotice } from "../state/notices.svelte";
  import { rendererPref } from "../state/rendererPref.svelte";
  import { colorForT, type ColormapName, cssGradientFor, normalizeDb } from "./colormap";
  import { detectMaxTextureSize, isFftSizeDisabled } from "./fftLimit";
  import {
    autoFftSize,
    FFT_SIZES,
    frameColumnBounds,
    frameLinearMapping,
    hopForZoom,
    tileCount as tileCountFor,
    tileDevicePxRange,
    totalFrames,
    visibleTileIndices,
  } from "./geometry";
  import { formatLevelDb } from "./hoverFormat";
  import { nearestCode, pixelDb, type TileLookup } from "./sampler";
  import { createSpectroRequester, type SpectroRequester } from "./spectroRequester";
  import { type SpectrogramTileEntry, SpectrogramGlRenderer } from "./webglRenderer";

  /**
   * The spectral pane (T-207, SPEC-007 essential subset; T-306/H-12 add persistence and HiDPI):
   * the STFT spectrogram, colored through a shader-equivalent CPU colormap pass (Canvas2D — see
   * the T-207 ticket report for why it starts with Canvas2D rather than WebGL2), a log/linear
   * frequency ruler, and a hover readout. Shares the waveform's time axis via the bindable
   * `startSample`/`samplesPerPixel` props (SPEC-007 §2.3 "one viewport"), now backed by
   * `state/waveformView.svelte.ts` so `EditorView` can persist and restore it (H-12).
   *
   * **H-12 (HiDPI):** the spectrogram is drawn one column per **device** pixel, not per CSS pixel
   * (`geometry.ts::frameColumnBounds`) — the canvas's backing store (`canvasEl.width`/`height`)
   * is sized in device pixels too, independent of whether a 2D context is available.
   *
   * Deferred (see the T-207 ticket report): WebGL2 R8-texture rendering (ADR-009 hardening item);
   * a distinct "importing" freeze message (SPEC-007 §2.1) — the app still has no import-progress
   * state (T-202's import stays synchronous), only "no document open", which this view already
   * handles; a native right-click menu for the frequency scale (a toolbar toggle button
   * substitutes).
   */

  let {
    startSample = $bindable(0),
    samplesPerPixel = $bindable(1),
  }: { startSample?: number; samplesPerPixel?: number } = $props();

  /** One spectral pane exists in the app; a fixed id is enough for `spectro_attach`. */
  const SPECTRAL_VIEW_ID = 1;

  let containerEl: HTMLDivElement | undefined = $state();
  let canvasEl: HTMLCanvasElement | undefined = $state();
  let rulerEl: HTMLDivElement | undefined = $state();
  let viewportPx = $state(0);
  let heightPx = $state(160);
  let maxTextureSize = $state(Infinity);
  let requester: SpectroRequester | null = $state(null);

  let freqLo = $state(20);
  let freqHi = $state(24_000);
  let fittedFreqKey: string | null = $state(null);

  let hoverX: number | null = $state(null);
  let hoverY: number | null = $state(null);

  let pointerDownClientX: number | null = null;
  let pointerDownSample: number | null = null;
  let pointerDownShiftKey = false;
  let dragging = false;
  let rulerDragStartY: number | null = null;
  let rulerDragStartRange: [number, number] | null = null;

  const doc = documentState();
  const transport = transportState();
  const rec = recordState();
  const selection = selectionState();
  const markers = markersState();
  const spectral = spectralState();

  const lenSamples = $derived(doc.current.len_samples);
  const rateHz = $derived(doc.current.sample_rate_hz);
  const isOpen = $derived(hasDocument(doc.current));
  const isRecording = $derived(rec.state.recording);

  function nyquistHz(): number {
    return rateHz > 0 ? rateHz / 2 : 24_000;
  }

  // Resets the visible frequency range to the full axis whenever the document's Nyquist rate or
  // the log/linear scale changes (mirrors WaveformView's own `fittedForAudio` zoom-to-fit — the
  // frequency range is "per document and not persisted", SPEC-007 §2.4).
  $effect(() => {
    const key = `${spectral.freqScale}:${nyquistHz()}`;
    if (key !== fittedFreqKey) {
      fittedFreqKey = key;
      const [lo, hi] = fullFreqRange(spectral.freqScale, nyquistHz());
      freqLo = lo;
      freqHi = hi;
    }
  });

  // Requests the tiles the current viewport needs (SPEC-007 §4.6), skipped entirely while
  // recording (§2.1: "requests no tiles" — the last held tiles simply keep being redrawn, which
  // is how the pane "keeps its last image").
  $effect(() => {
    requester?.setAudioRev(doc.current.audio_rev);
    if (!requester || !isOpen || viewportPx <= 0 || lenSamples <= 0 || rateHz <= 0 || isRecording) {
      return;
    }
    const dpr = window.devicePixelRatio || 1;
    const fftSize = spectral.fftSize ?? autoFftSize(rateHz);
    const sppDev = samplesPerPixel / dpr;
    const endSample = startSample + samplesPerPixel * viewportPx;
    void requester.request({
      startSample,
      endSample,
      samplesPerDevicePixel: sppDev,
      lenSamples,
      fftSize,
    });
  });

  // H-24 item 6: a small colour-bar legend with its dB range (the pane's floor/ceiling — the
  // colormap itself carries no scale otherwise).
  const legendGradient = $derived(cssGradientFor(spectral.colormap));

  const ticks = $derived.by(() => {
    if (heightPx <= 0) {
      return [];
    }
    return frequencyTicks(freqLo, freqHi, spectral.freqScale, heightPx, 24);
  });

  // H-26: ruler labels that fit — edge-aligned at the top and bottom (never cut off), clear of
  // each other and of the "Hz" unit in the corner (`fitGutterLabels`). Grid lines keep every tick.
  const rulerLabels = $derived(
    fitGutterLabels(
      ticks.map((tick) => ({ ...tick, pos: tick.y, text: tick.label })),
      {
        length: heightPx,
        width: 48,
        fontPx: 10,
        lineHeightPx: 12,
        unit: { text: t("spectral.freq_unit"), fontPx: 10 },
      },
    ),
  );

  const hoverInfo = $derived.by(() => {
    if (
      hoverX === null ||
      hoverY === null ||
      !isOpen ||
      lenSamples <= 0 ||
      rateHz <= 0 ||
      !requester
    ) {
      return null;
    }
    const dpr = window.devicePixelRatio || 1;
    const fftSize = spectral.fftSize ?? autoFftSize(rateHz);
    const bins = fftSize / 2 + 1;
    const sppDev = samplesPerPixel / dpr;
    const hop = hopForZoom(sppDev, fftSize);
    const total = totalFrames(lenSamples, hop);
    const sample = Math.max(0, Math.min(sampleAtPixel(hoverX, startSample, samplesPerPixel), lenSamples));
    const frame = sample / hop;
    const freqHz = freqForY(hoverY, heightPx, freqLo, freqHi, spectral.freqScale);
    const bin = (freqHz * fftSize) / rateHz;
    const getTile: TileLookup = (i) => requester?.tile(fftSize, hop, i);
    const code = nearestCode(getTile, total, bins, frame, bin);
    return {
      timeText: formatTime(sample, rateHz),
      freqText: formatHoverFreqHz(freqHz),
      levelText: formatLevelDb(code) ?? t("spectral.hover.no_data"),
    };
  });

  const hoverBoxStyle = $derived.by(() => {
    if (hoverX === null || hoverY === null) {
      return "display: none";
    }
    const offset = 12;
    const boxW = 170;
    const boxH = 60;
    const flipX = hoverX + offset + boxW > viewportPx;
    const flipY = hoverY + offset + boxH > heightPx;
    const left = flipX ? hoverX - offset - boxW : hoverX + offset;
    const top = flipY ? hoverY - offset - boxH : hoverY + offset;
    return `left: ${Math.max(0, left)}px; top: ${Math.max(0, top)}px;`;
  });



  // H-13 (ADR-009 §2/§4): WebGL2 primary renderer, Canvas2D automatic fallback. See
  // WaveformView.svelte's identical pattern for the rationale (context lifetime tied to the
  // canvas element, not the component).
  let glHost: GlContextHost | null = null;
  let glRenderer: SpectrogramGlRenderer | null = null;

  $effect(() => {
    const el = canvasEl;
    if (!el) {
      return;
    }
    const host = new GlContextHost(el, rendererPref().value, {
      // Deferred to a microtask — see WaveformView.svelte's identical comment: mutating
      // `notices.svelte.ts`'s `$state` synchronously from inside this `$effect` can re-trigger it
      // during the current flush.
      onKindDecided: (kind, reason) => {
        if (kind === "canvas2d" && reason === "unavailable") {
          queueMicrotask(() =>
            pushNotice({ level: "info", key: "notice.renderer.fallback_spectral", params: {}, persistent: false, id: null, cleared: false }),
          );
        }
      },
      onContextLost: () => {
        glRenderer?.dispose();
        glRenderer = null;
        queueMicrotask(() =>
          pushNotice({ level: "warning", key: "notice.renderer.context_lost_spectral", params: {}, persistent: false, id: null, cleared: false }),
        );
      },
    });
    glHost = host;
    glRenderer = host.gl ? new SpectrogramGlRenderer(host.gl) : null;
    return () => {
      glRenderer?.dispose();
      glRenderer = null;
      host.dispose();
      glHost = null;
    };
  });

  /** SPEC-007 §4.7: tiles as R8 textures, colormap as a 256×1 LUT, floor/ceiling/scale as
   * uniforms — see `webglRenderer.ts`'s doc comment for the sampling rule and its known
   * tile-boundary approximation. Overlays reuse the same `../render/overlayGeometry.ts` builder
   * as the waveform, with `markerStyle: "lines"` to match this pane's own (simpler) Canvas2D
   * overlay look. */
  function drawSpectrogramWebgl2(renderer: SpectrogramGlRenderer, backingW: number, backingH: number, dpr: number): void {
    const fftSize = spectral.fftSize ?? autoFftSize(rateHz);
    const sppDev = samplesPerPixel / dpr;
    const hop = hopForZoom(sppDev, fftSize);
    const { frameAtPx0, framesPerPx } = frameLinearMapping(startSample, samplesPerPixel, dpr, hop);
    const count = tileCountFor(lenSamples, hop);
    const tiles: SpectrogramTileEntry[] = [];
    for (const tileIndex of visibleTileIndices(frameAtPx0, framesPerPx, backingW, count)) {
      const tile = requester?.tile(fftSize, hop, tileIndex);
      if (!tile) {
        continue;
      }
      const { x0, x1 } = tileDevicePxRange(tileIndex, frameAtPx0, framesPerPx, backingW);
      if (x1 > x0) {
        tiles.push({ tile, tileIndex, x0, x1 });
      }
    }

    // The tile quads are in device-pixel space (H-12's one-column-per-device-pixel convention), so
    // the overlay must be built in the same space: pass the device-pixel `samplesPerPixel`
    // (`sppDev`) and the backing (device-pixel) width/height rather than the CSS ones, or the
    // overlay would be scaled by `devicePixelRatio` relative to the tiles it's drawn over.
    const overlay = buildOverlayBatch({
      startSample,
      samplesPerPixel: sppDev,
      viewportPx: backingW,
      heightPx: backingH,
      selection: selection.current,
      markers: markers.list,
      playheadSample: transport.playheadSamples,
      markerStyle: "lines",
      lineWidthPx: themeColors().strokePx * dpr,
      colors: {
        selectionFill: themeColors().wave.selectionFill.rgba,
        marker: themeColors().wave.marker.rgba,
        markerRegionFill: themeColors().wave.markerRegion.rgba,
        playhead: themeColors().wave.playhead.rgba,
      },
    });

    renderer.draw({
      backingWidthPx: backingW,
      backingHeightPx: backingH,
      background: themeColors().spec.bg.rgba,
      pending: themeColors().spec.pending.rgba,
      colormap: spectral.colormap,
      floorDb: spectral.floorDb,
      ceilDb: spectral.ceilDb,
      freqLo,
      freqHi,
      freqScale: spectral.freqScale,
      sampleRateHz: rateHz,
      fftSize,
      frameAtPx0,
      framesPerPx,
      tiles,
      overlay: overlay.vertexCount > 0 ? overlay.toFloat32Array() : null,
    });
  }

  function drawSpectrogram(ctx: CanvasRenderingContext2D, backingW: number, backingH: number, dpr: number): void {
    if (!requester) {
      return;
    }
    const fftSize = spectral.fftSize ?? autoFftSize(rateHz);
    const bins = fftSize / 2 + 1;
    const sppDev = samplesPerPixel / dpr;
    const hop = hopForZoom(sppDev, fftSize);
    const total = totalFrames(lenSamples, hop);
    const getTile: TileLookup = (i) => requester?.tile(fftSize, hop, i);
    const scale = spectral.freqScale;
    const colormap = spectral.colormap;
    const floorDb = spectral.floorDb;
    const ceilDb = spectral.ceilDb;
    const [pr, pg, pb] = themeColors().spec.pending.rgba;
    const pendingRgb = [Math.round(pr * 255), Math.round(pg * 255), Math.round(pb * 255)] as const;

    const image = ctx.createImageData(backingW, backingH);
    const data = image.data;

    // H-12 (HiDPI): one column per device pixel (`geometry.ts::frameColumnBounds`) — `backingW`
    // is already the device-pixel canvas width (`draw()` below).
    const { lo: frameLoArr, hi: frameHiArr } = frameColumnBounds(backingW, startSample, samplesPerPixel, dpr, hop);
    const binLoArr = new Float64Array(backingH);
    const binHiArr = new Float64Array(backingH);
    for (let py = 0; py < backingH; py++) {
      const uHi = 1 - py / backingH;
      const uLo = 1 - (py + 1) / backingH;
      const fLoPx = freqForU(uLo, freqLo, freqHi, scale);
      const fHiPx = freqForU(uHi, freqLo, freqHi, scale);
      binLoArr[py] = (fLoPx * fftSize) / rateHz;
      binHiArr[py] = (fHiPx * fftSize) / rateHz;
    }

    for (let px = 0; px < backingW; px++) {
      const frameLo = frameLoArr[px]!;
      const frameHi = frameHiArr[px]!;
      for (let py = 0; py < backingH; py++) {
        const db = pixelDb(getTile, total, bins, frameLo, frameHi, binLoArr[py]!, binHiArr[py]!);
        const idx = (py * backingW + px) * 4;
        if (db === null) {
          data[idx] = pendingRgb[0];
          data[idx + 1] = pendingRgb[1];
          data[idx + 2] = pendingRgb[2];
        } else {
          const value = normalizeDb(db, floorDb, ceilDb);
          const [r, g, b] = colorForT(colormap, value);
          data[idx] = r;
          data[idx + 1] = g;
          data[idx + 2] = b;
        }
        data[idx + 3] = 255;
      }
    }
    ctx.putImageData(image, 0, 0);
  }

  function drawOverlays(ctx: CanvasRenderingContext2D): void {
    const sel = selection.current;
    if (sel) {
      const x0 = Math.max(0, pixelAtSample(sel.startSample, startSample, samplesPerPixel));
      const x1 = Math.min(viewportPx, pixelAtSample(sel.endSample, startSample, samplesPerPixel));
      if (x1 > x0) {
        ctx.fillStyle = themeColors().wave.selectionFill.css;
        ctx.fillRect(x0, 0, x1 - x0, heightPx);
      }
    }
    if (markers.list.length > 0) {
      ctx.strokeStyle = themeColors().wave.marker.css;
      ctx.lineWidth = themeColors().strokePx;
      for (const marker of markers.list) {
        const px = pixelAtSample(marker.pos_samples, startSample, samplesPerPixel);
        if (px >= -1 && px <= viewportPx + 1) {
          ctx.beginPath();
          ctx.moveTo(px + crispOffset(ctx.lineWidth), 0);
          ctx.lineTo(px + crispOffset(ctx.lineWidth), heightPx);
          ctx.stroke();
        }
      }
    }
    const playheadPx = pixelAtSample(transport.playheadSamples, startSample, samplesPerPixel);
    if (playheadPx >= -1 && playheadPx <= viewportPx + 1) {
      ctx.strokeStyle = themeColors().wave.playhead.css;
      ctx.lineWidth = themeColors().strokePx;
      ctx.beginPath();
      ctx.moveTo(playheadPx + crispOffset(ctx.lineWidth), 0);
      ctx.lineTo(playheadPx + crispOffset(ctx.lineWidth), heightPx);
      ctx.stroke();
    }
  }

  function draw(): void {
    if (!canvasEl || viewportPx <= 0 || heightPx <= 0) {
      return;
    }
    // H-12 (HiDPI): the canvas's backing store is sized in device pixels regardless of whether a
    // 2D context is available, so the element itself is always HiDPI-correct (and this part is
    // testable in jsdom, which has no 2D context at all).
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(viewportPx * dpr));
    const backingH = Math.max(1, Math.round(heightPx * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    if (glHost?.kind === "webgl2" && glRenderer) {
      if (isOpen && lenSamples > 0 && rateHz > 0) {
        drawSpectrogramWebgl2(glRenderer, backingW, backingH, dpr);
      } else {
        // No document (or not enough info yet): still clear to the background so the pane never
        // shows a stale frame from a previously open document.
        glRenderer.draw({
          backingWidthPx: backingW,
          backingHeightPx: backingH,
          background: themeColors().spec.bg.rgba,
          pending: themeColors().spec.pending.rgba,
          colormap: spectral.colormap,
          floorDb: spectral.floorDb,
          ceilDb: spectral.ceilDb,
          freqLo,
          freqHi,
          freqScale: spectral.freqScale,
          sampleRateHz: rateHz > 0 ? rateHz : 48_000,
          fftSize: spectral.fftSize ?? autoFftSize(48_000),
          frameAtPx0: 0,
          framesPerPx: 1,
          tiles: [],
          overlay: null,
        });
      }
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // jsdom in tests, or a browser with no 2D canvas support
    }
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = themeColors().spec.bg.css;
    ctx.fillRect(0, 0, backingW, backingH);
    if (isOpen && lenSamples > 0 && rateHz > 0) {
      drawSpectrogram(ctx, backingW, backingH, dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (isOpen) {
      drawOverlays(ctx);
    }
    ctx.restore();
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

  /** Plain wheel scrolls, Ctrl+wheel zooms (SPEC-006 §2.6, shared with the waveform); Alt+wheel
   * over the spectral pane's body does nothing (SPEC-007 §2.3). */
  function onWheel(event: WheelEvent): void {
    if (!isOpen || viewportPx <= 0 || !containerEl || event.altKey) {
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

  /** Wheel over the ruler zooms the visible frequency range around the pointer's frequency
   * (SPEC-007 §2.4: √2 per notch, via the shared `ZOOM_STEP_FACTOR`). */
  function onRulerWheel(event: WheelEvent): void {
    if (!rulerEl || heightPx <= 0) {
      return;
    }
    event.preventDefault();
    const rect = rulerEl.getBoundingClientRect();
    const y = event.clientY - rect.top;
    const anchorHz = freqForY(y, heightPx, freqLo, freqHi, spectral.freqScale);
    const factor = event.deltaY > 0 ? ZOOM_STEP_FACTOR : 1 / ZOOM_STEP_FACTOR;
    const [lo, hi] = zoomFreqRange(freqLo, freqHi, spectral.freqScale, anchorHz, factor, nyquistHz());
    freqLo = lo;
    freqHi = hi;
  }

  function onRulerPointerDown(event: PointerEvent): void {
    rulerDragStartY = event.clientY;
    rulerDragStartRange = [freqLo, freqHi];
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  /** Dragging the ruler pans the visible frequency range (SPEC-007 §2.4). */
  function onRulerPointerMove(event: PointerEvent): void {
    if (rulerDragStartY === null || !rulerDragStartRange || heightPx <= 0) {
      return;
    }
    const deltaFrac = (event.clientY - rulerDragStartY) / heightPx;
    const [lo, hi] = panFreqRange(
      rulerDragStartRange[0],
      rulerDragStartRange[1],
      spectral.freqScale,
      deltaFrac,
      nyquistHz(),
    );
    freqLo = lo;
    freqHi = hi;
  }

  function onRulerPointerUp(event: PointerEvent): void {
    rulerDragStartY = null;
    rulerDragStartRange = null;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
  }

  /** Double-clicking the ruler resets to the full range (SPEC-007 §2.4). */
  function resetFreqRange(): void {
    const [lo, hi] = fullFreqRange(spectral.freqScale, nyquistHz());
    freqLo = lo;
    freqHi = hi;
  }

  /** The document sample under `clientX` (SPEC-007 §2.2: clicks/drags act on time exactly as in
   * the waveform pane). */
  function sampleAtClientX(clientX: number): number | null {
    if (!containerEl) {
      return null;
    }
    const rect = containerEl.getBoundingClientRect();
    const px = clientX - rect.left;
    return Math.max(0, Math.min(sampleAtPixel(px, startSample, samplesPerPixel), lenSamples));
  }

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

  function onPointerMove(event: PointerEvent): void {
    if (containerEl) {
      const rect = containerEl.getBoundingClientRect();
      hoverX = event.clientX - rect.left;
      hoverY = event.clientY - rect.top;
    }
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

  function onPointerLeave(): void {
    hoverX = null;
    hoverY = null;
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
    endDrag();
    clearSelection();
    void seek(upSample);
  }

  function onDoubleClick(): void {
    if (isOpen) {
      selectAllOf(lenSamples);
    }
  }

  function onFftSizeChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    spectral.setFftSize(value === "auto" ? null : Number(value));
  }

  function onColormapChange(event: Event): void {
    spectral.setColormap((event.currentTarget as HTMLSelectElement).value as ColormapName);
  }

  function toggleScale(): void {
    spectral.setFreqScale(spectral.freqScale === "log" ? "linear" : "log");
  }

  // The canvas container only exists once a document is open (`{#if isOpen}`), so the size
  // observer must be (re)attached whenever the element appears, not once at mount (S1-03 gotcha:
  // an element bound inside a closed `{#if}` branch is `undefined` in `onMount`).
  $effect(() => {
    const el = containerEl;
    if (!el) {
      viewportPx = 0;
      return;
    }
    viewportPx = el.clientWidth;
    heightPx = el.clientHeight || heightPx;
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
    maxTextureSize = detectMaxTextureSize();

    let disposed = false;
    let attached = false;
    createSpectroRequester(SPECTRAL_VIEW_ID, {})
      .then((r) => {
        attached = true;
        if (disposed) {
          void spectroDetach(SPECTRAL_VIEW_ID).catch(() => {});
          return;
        }
        requester = r;
      })
      .catch(() => {
        // No spectral view without a working IPC channel — the pane just stays "pending".
      });

    const requestFrame: (cb: () => void) => number =
      typeof requestAnimationFrame === "function"
        ? (cb) => requestAnimationFrame(cb)
        : (cb) => setTimeout(cb, 16) as unknown as number;
    const cancelFrame: (id: number) => void =
      typeof cancelAnimationFrame === "function"
        ? (id) => cancelAnimationFrame(id)
        : (id) => clearTimeout(id);
    let frameId = 0;
    const loop = () => {
      if (disposed) {
        return;
      }
      try {
        draw();
      } finally {
        // H-32: reschedule unconditionally — a transient bad read (e.g. the shared `transport`
        // store, guarded at the source in `transport.svelte.ts`) must never stop this loop from
        // trying again next frame.
        frameId = requestFrame(loop);
      }
    };
    frameId = requestFrame(loop);

    return () => {
      disposed = true;
      cancelFrame(frameId);
      if (attached) {
        void spectroDetach(SPECTRAL_VIEW_ID).catch(() => {});
      }
    };
  });
</script>

<div class="spectral-view" data-testid="spectral-view">
  {#if isOpen}
    <div class="toolbar" data-testid="spectral-toolbar">
      <button type="button" data-testid="spectral-scale-toggle" onclick={toggleScale}>
        {spectral.freqScale === "log" ? t("spectral.scale_log") : t("spectral.scale_linear")}
      </button>
      <label>
        {t("spectral.colormap_label")}
        <select data-testid="spectral-colormap" value={spectral.colormap} onchange={onColormapChange}>
          <option value="inferno">{t("spectral.colormap.inferno")}</option>
          <option value="viridis">{t("spectral.colormap.viridis")}</option>
          <option value="gray">{t("spectral.colormap.gray")}</option>
        </select>
      </label>
      <label>
        {t("spectral.fft_size_label")}
        <select data-testid="spectral-fft-size" value={spectral.fftSize === null ? "auto" : String(spectral.fftSize)} onchange={onFftSizeChange}>
          <option value="auto">{t("spectral.fft_auto")}</option>
          {#each FFT_SIZES as size (size)}
            <option
              value={size}
              disabled={isFftSizeDisabled(size, maxTextureSize)}
              title={isFftSizeDisabled(size, maxTextureSize) ? t("spectral.fft_disabled_tooltip") : undefined}
            >
              {size}
            </option>
          {/each}
        </select>
      </label>
      <label>
        {t("spectral.floor_label")}
        <input
          type="number"
          data-testid="spectral-floor"
          min={FLOOR_RANGE_DB[0]}
          max={FLOOR_RANGE_DB[1]}
          value={spectral.floorDb}
          oninput={(e) => spectral.setFloorDb(Number((e.currentTarget as HTMLInputElement).value))}
        />
      </label>
      <label>
        {t("spectral.ceiling_label")}
        <input
          type="number"
          data-testid="spectral-ceiling"
          min={CEIL_RANGE_DB[0]}
          max={CEIL_RANGE_DB[1]}
          value={spectral.ceilDb}
          oninput={(e) => spectral.setCeilDb(Number((e.currentTarget as HTMLInputElement).value))}
        />
      </label>
      <div class="legend" data-testid="spectral-legend">
        <span class="legend-value">{t("spectral.legend_value", { value: formatNumber(spectral.floorDb, 0) })}</span>
        <span class="legend-bar" style={`background: ${legendGradient}`}></span>
        <span class="legend-value">{t("spectral.legend_value", { value: formatNumber(spectral.ceilDb, 0) })}</span>
      </div>
    </div>
    <div class="body">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="ruler"
        data-testid="spectral-ruler"
        bind:this={rulerEl}
        onwheel={onRulerWheel}
        onpointerdown={onRulerPointerDown}
        onpointermove={onRulerPointerMove}
        onpointerup={onRulerPointerUp}
        ondblclick={resetFreqRange}
      >
        <span class="unit">{t("spectral.freq_unit")}</span>
        {#each rulerLabels as tick (tick.freqHz)}
          <span class="tick" data-align={tick.align} style={`top: ${tick.y}px`}>{tick.label}</span>
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
        onpointerleave={onPointerLeave}
        ondblclick={onDoubleClick}
      >
        <canvas bind:this={canvasEl} aria-label={t("spectral.canvas_label")} data-testid="spectral-canvas"></canvas>
        {#if isRecording}
          <div class="frozen-overlay" data-testid="spectral-recording-overlay">
            {t("spectral.recording_frozen")}
          </div>
        {/if}
        {#if hoverInfo}
          <div class="hover-readout" data-testid="spectral-hover" style={hoverBoxStyle}>
            <div>{hoverInfo.timeText}</div>
            <div>{hoverInfo.freqText}</div>
            <div>{hoverInfo.levelText}</div>
          </div>
        {/if}
      </div>
    </div>
  {:else}
    <p class="empty" data-testid="spectral-empty">{t("spectral.empty")}</p>
  {/if}
</div>

<style>
  .spectral-view {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    flex: 1;
    background: var(--spec-bg);
  }

  .empty {
    margin: auto;
    color: var(--pv-text-tertiary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
  }

  .toolbar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-3);
    min-height: 28px;
    padding: var(--pv-space-half) var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-panel);
    color: var(--pv-text-tertiary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
  }

  .toolbar label {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .toolbar select,
  .toolbar input,
  .toolbar button {
    height: 22px;
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    cursor: default;
  }

  .toolbar button {
    border-color: var(--pv-border);
  }

  .toolbar button:hover {
    background: var(--pv-control-bg-hover);
  }

  .toolbar select:focus-visible,
  .toolbar input:focus-visible,
  .toolbar button:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  /* H-26: room for "−120" plus the spin arrows (3.5rem clipped it). */
  .toolbar input[type="number"] {
    width: 4.75rem;
  }

  .legend {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin-left: auto;
  }

  .legend-bar {
    display: inline-block;
    width: 4.5rem;
    height: 0.6rem;
    border: 1px solid var(--surface-border);
    border-radius: 2px;
  }

  .legend-value {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .ruler {
    position: relative;
    width: 48px;
    flex: none;
    border-right: 1px solid var(--spec-ruler-grid);
    background: var(--surface-panel);
    overflow: hidden;
    cursor: ns-resize;
  }

  .unit {
    position: absolute;
    top: 2px;
    left: 2px;
    color: var(--spec-ruler-text);
    font-size: 10px;
    line-height: 12px;
  }

  .tick {
    position: absolute;
    right: 2px;
    color: var(--spec-ruler-text);
    font-size: 10px;
    line-height: 12px;
    font-variant-numeric: tabular-nums;
    transform: translateY(-50%);
    white-space: nowrap;
  }

  .tick[data-align="start"] {
    transform: translateY(0);
  }

  .tick[data-align="end"] {
    transform: translateY(-100%);
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

  .frozen-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--spec-scrim);
    color: var(--spec-scrim-text);
    font-size: 0.8rem;
    text-align: center;
    padding: 0.5rem;
    pointer-events: none;
  }

  .hover-readout {
    position: absolute;
    z-index: 1;
    pointer-events: none;
    background: var(--surface-panel-raised);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.4rem;
    font-size: 0.7rem;
    color: var(--text-primary);
    white-space: nowrap;
  }
</style>
