<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { documentState, hasDocument, isImportRunning } from "../document/document.svelte";
  import { t } from "../i18n";
  import { importPeaksGet, peaksGet } from "../ipc/commands";
  import { recordPeaksGet } from "../ipc/record_commands";
  import { registerAction } from "../shortcuts";
  import type { MarkerDto, MarkerRangeKindDto } from "../ipc/bindings";
  import { activateMarker, isTakeMarker, markersState, setMarkerRange } from "../markers/markers.svelte";
  import { openNewRecordingPrompt, recordState } from "../state/record.svelte";
  import { settingsState } from "../state/settings.svelte";
  import { dispatchAction } from "../shortcuts";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import type { DefaultFormatDto } from "../ipc/bindings";
  import { Button, EmptyState, Menu, type PopoverAnchor } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import { hasClipboard, silence } from "../state/edit.svelte";
  import { openInsertSilenceDialog } from "../state/insertSilence.svelte";
  import {
    beginDrag,
    beginHandleDrag,
    clearSelection,
    dragTo,
    endDrag,
    endHandleDrag,
    handleDragTo,
    hasSelection,
    isSelectionLocked,
    selectAllOf,
    selectionState,
    setSelectionFromResult,
    shiftClickTo,
  } from "../state/selection.svelte";
  import { isPlayheadMoving, seek, transportState } from "../state/transport.svelte";
  import { createFrameClient } from "../render/frameScheduler";
  import { themeState } from "../theme/theme.svelte";
  import {
    amplitudeRulerModeState,
    audioKeyFor,
    consumePendingRestore,
    setVerticalZoom,
    verticalZoomState,
  } from "../state/waveformView.svelte";
  import { amplitudeTicksDbfs, amplitudeTicksPercent, centerlineY } from "./amplitudeAxis";
  import { extendSelectionEdge, hitTestHandle, normalizeSelection, nudgeSelectionRange } from "./selection";
  import {
    advanceMarkerAutoscroll,
    DRAG_THRESHOLD_PX as MARKER_DRAG_THRESHOLD_PX,
    dragPointMarker,
    dragRegionEnd,
    dragRegionStart,
    dragRegionWhole,
    FLAG_HIT_HEIGHT_PX,
    hitTestMarkerFlag,
    markerAutoscrollDirection,
    markerMagnetTargets,
    snapToMarkerMagnet,
    type MarkerFlagEdge,
  } from "./markerDrag";
  import { snapSampleToZeroCrossing } from "./zeroCrossing";
  import { fitGutterLabels } from "../ui/axisLabels";
  import {
    clampSamplesPerPixel,
    clampStartSample,
    columnYRange,
    isPendingColumn,
    MIN_SAMPLES_PER_PIXEL,
    pickLevel,
    pixelAtSample,
    RAW_SPP,
    reduceColumns,
    sampleAtPixel,
    showsDots,
    zoomAroundSample,
    ZOOM_STEP_FACTOR,
    zoomFullSamplesPerPixel,
    zoomFullViewport,
    zoomStep,
    zoomToSelectionViewport,
  } from "./coords";
  import { DEFAULT_VERTICAL_ZOOM, verticalZoomStep } from "./verticalZoom";
  import { GlContextHost } from "../render/glContext";
  import { buildOverlayBatch, buildSelectionUnderlay } from "../render/overlayGeometry";
  import { QuadBatch } from "../render/quads";
  import { crispOffset, themeColors } from "../theme/themeColors";
  import { LOOP_STRIP_PX, loopFromRange, loopGeometry } from "../render/loopOverlay";
  import { selectionGeometry } from "../render/selectionOverlay";
  import { pushNotice } from "../state/notices.svelte";
  import { rendererPref } from "../state/rendererPref.svelte";
  import { decodeVxpk } from "./vxpk";
  import {
    type Column,
    type OpLayout,
    opColumns,
    opLayout,
    opMarkers,
    opPeaksRequestStart,
  } from "./opLayout";
  import { ImportPeaksRequester } from "./importPeaksRequester";
  import { PeaksRequester } from "./peaksRequester";
  import {
    followPlayhead,
    INITIAL_PLAYBACK_FOLLOW_STATE,
    suspendPlaybackFollow,
    type PlaybackFollowState,
  } from "./playbackFollow";
  import {
    followRecordHead,
    INITIAL_RECORD_FOLLOW_STATE,
    suspendRecordFollow,
    type RecordFollowState,
  } from "./recordFollow";
  import { ViewportWriter } from "./viewportFollow";
  import { buildColumnQuads, buildRawPolyline } from "./webglGeometry";
  import { type WaveformGlContent, WaveformGlRenderer } from "./webglRenderer";

  /**
   * The waveform view (S1-03, SPEC-006 essential subset; H-07 adds the live view while recording;
   * H-13 adds the WebGL2 renderer): min/max fill and raw-sample polyline, drawn by WebGL2
   * (`webglRenderer.ts`/`webglGeometry.ts`) when available, Canvas2D otherwise (ADR-009 §2/§4) —
   * `drawWebgl2`/`drawCanvas2d` share the same pixel math (`coords.ts`, `../render/
   * overlayGeometry.ts`) so the two renderers agree pixel-for-pixel. Horizontal zoom/scroll, the
   * shared playhead (SPEC-003 §2.2's extrapolation, read from the transport store — never
   * re-derived here), and click-to-seek. HiDPI aware. Selection (S2-01), vertical zoom (H-35) and
   * markers (SPEC-009) have since landed; the overview strip is still deferred (ticket's "Out"
   * list, unclaimed).
   *
   * H-12: the time ruler and scrollbar that used to live here now live in `EditorView` (shared
   * with `SpectralView`, SPEC-007 §2.1's ruler → waveform → divider → spectral → scrollbar
   * stack) — this view only owns the canvas. `EditorView` also owns the persisted viewport
   * (`state/waveformView.svelte.ts`) this component's `startSample`/`samplesPerPixel` bind to;
   * this component still decides *when* to zoom-to-fit a newly opened document (it's the one that
   * knows the canvas's pixel width), consuming a pending restored viewport instead when the just
   * opened document's sidecar had one (`consumePendingRestore`, SPEC-018 §2.6.5).
   *
   * H-07: while `recordState` is recording, the document has no committed audio yet (S1-04: an
   * open take's document is empty until Stop), so instead of the normal `peaks_get` path this
   * view polls `record_peaks_get` at [`LIVE_POLL_MS`] and draws the growing take, zoomed to fit
   * (at least [`LIVE_MIN_WINDOW_SECONDS`]), plus a record-head line at the take's current length.
   *
   * H-21 (SPEC-022 §2.11): during a record operation on a document with audio the normal view
   * stays (no zoom-to-fit) and the live take is drawn at the record point `at` in the record
   * colour (`opLayout.ts`): Insert shifts the existing waveform and markers after `at` right by the
   * current take length; Overwrite and Punch draw the take over the old audio, a punch also
   * shading its region `[S, E)`; a record-head line marks the take's end.
   *
   * H-23 (A-016): while that operation runs, the view also page-flips to keep the record head in
   * view, respecting a user scroll/zoom until the head reaches the next page (`recordFollow.ts`).
   *
   * H-27 (SPEC-006 §2.8): while playing (and `Settings.playhead_follow` is on), the view instead
   * continuously scrolls to keep the extrapolated playhead inside a follow band spanning the
   * middle 80% of the viewport (`playbackFollow.ts`), respecting a user scroll/zoom until the
   * playhead re-enters the band. Recording and playback are mutually exclusive transport states,
   * so exactly one of the two follow policies is ever active; one `ViewportWriter`
   * (`viewportFollow.ts`) tells a user-driven viewport change apart from either policy's own last
   * write, shared so neither policy duplicates that diff.
   */

  /** Must match `vox_engine::record::LIVE_PEAKS_SPB` (H-07). */
  // H-25 empty state: File → New Recording…'s factory default (SPEC-002 §3), used only if
  // settings haven't loaded yet — the same fallback `DocumentMenu.svelte` uses.
  const NEW_RECORDING_FALLBACK: DefaultFormatDto = { sample_rate_hz: 48_000, bit_depth: "24" };
  const recState = recordState();
  const LIVE_PEAKS_SPB = 256;
  /** The live view never zooms in tighter than this many seconds of the take. */
  const LIVE_MIN_WINDOW_SECONDS = 10;
  /** Live take peaks poll rate (H-07 ticket: "~10 Hz"). */
  const LIVE_POLL_MS = 100;
  /** Same cap as `document_commands::peaks_get`'s `MAX_BUCKETS`. */
  const LIVE_MAX_BUCKETS = 65_536;

  /**
   * T-207: `startSample`/`samplesPerPixel` are bindable so `EditorView` can share one time axis
   * between this view and the spectral pane (SPEC-007 §2.3 "one viewport") — a zoom or scroll
   * gesture in either pane updates both instantly, since both bind the same parent variables.
   * Unbound (no parent passes them), they behave exactly like the local state they replace.
   */
  let {
    startSample = $bindable(0),
    samplesPerPixel = $bindable(1),
  }: { startSample?: number; samplesPerPixel?: number } = $props();

  let containerEl: HTMLDivElement | undefined = $state();
  let canvasEl: HTMLCanvasElement | undefined = $state();
  /** H-66: the waveform's right-click menu — opened at the pointer (or under the canvas container
   * when opened from the keyboard, like the Record context menu, H-26). */
  let contextMenuAnchor = $state<PopoverAnchor | null>(null);
  let viewportPx = $state(0);
  let heightPx = $state(200);
  let fittedForAudio = $state<string | null>(null);
  let pointerDownClientX: number | null = null;
  /** The mousedown sample and modifier (S2-01, SPEC-006 §2.9): distinguishes click/drag/Shift+click
   * on pointerup, without re-deriving the down position from a possibly-stale pixel. */
  let pointerDownSample: number | null = null;
  let pointerDownShiftKey = false;
  let dragging = false;
  /** T-206 (SPEC-006 §2.9): `true` from a handle-hit pointerdown until pointerup — routes
   * pointermove/pointerup to the handle-drag store functions instead of the plain-drag ones. */
  let handleDragActive = false;
  /** H-109: the pointer id captured for the current plain selection or handle drag (`null`
   * otherwise) — the non-marker counterpart of `markerDragPointerId` below. Without this, a drag
   * released outside the waveform (over another panel) sent its `pointerup` there instead of
   * here, so the drag never ended (the owner's report; H-64 already fixed this for marker
   * drags). Released on `onPointerUp` or the browser's own end-of-gesture events
   * (`onPointerCaptureLost`: `pointercancel`/`lostpointercapture`). */
  let dragPointerId: number | null = null;
  /** T-206: hovering within `SELECTION_HANDLE_HIT_PX` of a selection boundary shows a resize
   * cursor (SPEC-006 §2.9). */
  let nearHandle = $state(false);
  /** T-206 (SPEC-006 §2.10): invalidates a stale async zero-crossing snap result — e.g. a new
   * drag starts, or the selection is cleared, before the previous drag's snap resolves. */
  let selectionSnapSeq = 0;
  /** H-57 (SPEC-009 §2.5): the in-progress marker drag, or `null`. `grabOffsetSamples` is the
   * grabbed edge's sample minus the pointer's sample at grab time, so every later frame recomputes
   * an *absolute* target (`sampleAtClientX(now) + grabOffsetSamples`) instead of accumulating
   * deltas — no drift at any zoom. `moved` flips once the pointer has moved
   * `DRAG_THRESHOLD_PX`; before that, release is a click (activates the marker, SPEC-009 §2.5). */
  let markerDrag = $state<{
    id: number;
    edge: MarkerFlagEdge;
    wholeRegion: boolean;
    grabOffsetSamples: number;
    original: { pos_samples: number; len_samples: number };
    preview: { pos_samples: number; len_samples: number };
    moved: boolean;
  } | null>(null);
  /** H-64 (SPEC-009 §2.5/§3 `drag_autoscroll_rate`): `-1`/`1` while the pointer sits beyond the
   * canvas's left/right edge during a marker drag, `0` otherwise — recomputed on every
   * pointermove (`updateMarkerDragPreview`) and consumed by the frame scheduler client below,
   * which is the only thing that actually moves `startSample` (so scrolling continues even while
   * the pointer itself isn't moving). Not `$state`: read only from within the frame callback and
   * pointer handlers, never from a template. */
  let markerAutoscrollDir: -1 | 0 | 1 = 0;
  /** The last frame's timestamp while auto-scrolling, so each tick advances by a real `dt` instead
   * of an assumed frame period — `null` right after auto-scroll starts (that first tick only
   * records `now`, per H-43's frame contract of never assuming a fixed frame rate). */
  let autoscrollLastNow: number | null = null;
  /** The pointer position auto-scroll re-evaluates the drag preview against every tick, since the
   * document sample under an unmoving pointer changes as the view scrolls underneath it. */
  let lastMarkerPointerClientX: number | null = null;
  let lastMarkerPointerAltKey = false;
  /** H-64: the pointer id captured for the current marker drag (`null` otherwise), released on
   * drag end/cancel. */
  let markerDragPointerId: number | null = null;
  /** H-109 (SPEC-009 §3 `drag_autoscroll_rate`): the same auto-scroll H-64 gave marker drags,
   * extended to a plain selection or handle drag — `-1`/`1` while the pointer sits beyond the
   * canvas's left/right edge during one of those drags, `0` otherwise. Kept separate from
   * `markerAutoscrollDir` (never active at once: `onPointerDown` clears `markerDrag` before
   * either of these two drag kinds can start). */
  let selectionAutoscrollDir: -1 | 0 | 1 = 0;
  /** The non-marker counterpart of `autoscrollLastNow` above. */
  let selectionAutoscrollLastNow: number | null = null;
  /** The non-marker counterpart of `lastMarkerPointerClientX` above. */
  let lastSelectionPointerClientX: number | null = null;
  /** H-07: the last `record_peaks_get` response applied (`null`: none polled yet). */
  let liveBuckets = $state<Array<[number, number]>>([]);
  let liveStartSample = $state(0);
  /** H-10 item 6: the response's own bucket size — `LivePeaks` doubles it as a multi-hour take
   * decimates, so this must never be assumed to stay at `LIVE_PEAKS_SPB`. */
  let liveSpb = $state(LIVE_PEAKS_SPB);

  const doc = documentState();
  const transport = transportState();
  const rec = recordState();
  const selection = selectionState();
  const markers = markersState();
  const requester = new PeaksRequester(peaksGet);
  /** H-71 (SPEC-005 §2.3, ADR-003 Amendment 7): the running import job's own growing peaks —
   * never the normal `peaks_get` state, which has nothing to show until the import commits. */
  const importRequester = new ImportPeaksRequester(importPeaksGet);
  /** H-35 (SPEC-006 §2.4): the amplitude ruler/waveform vertical scale, per-document sidecar
   * state (`state/waveformView.svelte.ts`) — no viewport width to wait on (unlike
   * `startSample`/`samplesPerPixel`), so it's read directly here instead of through a bindable
   * prop. */
  const vzoom = verticalZoomState();
  /** H-72 (SPEC-006 §2.4): dBFS (default) vs. percent — same "read directly, no bindable prop"
   * shape as `vzoom` above. */
  const ampRulerMode = amplitudeRulerModeState();

  // H-43: the canvas draws on demand from the shared frame scheduler (`render/frameScheduler.ts`,
  // replacing H-32's perpetual rAF loop): one frame whenever an input of `draw()` changes (the
  // dependency effect below, peaks arriving, the GL context), and every frame while the playhead
  // moves or a take records. A draw that throws is retried by the scheduler, and any later change
  // draws again.
  const frames = createFrameClient(
    (now) => {
      draw();
      // H-64 (SPEC-009 §2.5): while a marker drag holds the pointer beyond the canvas edge, keep
      // scrolling every frame — not just on pointermove, since the pointer itself isn't moving.
      let stillAutoscrolling = false;
      if (markerDrag?.moved && markerAutoscrollDir !== 0) {
        const last = autoscrollLastNow;
        autoscrollLastNow = now;
        if (last !== null) {
          const dt = (now - last) / 1000;
          const next = advanceMarkerAutoscroll(
            startSample,
            markerAutoscrollDir,
            dt,
            samplesPerPixel,
            lenSamples,
            viewportPx,
          );
          if (next !== startSample) {
            startSample = next;
            if (lastMarkerPointerClientX !== null) {
              applyMarkerDragPreview(lastMarkerPointerClientX, lastMarkerPointerAltKey);
            }
            stillAutoscrolling = true;
          } else {
            // Already at the document edge in this direction: nothing left to animate until the
            // pointer moves again (which re-evaluates the direction from scratch).
            markerAutoscrollDir = 0;
          }
        } else {
          stillAutoscrolling = true; // first tick: only the baseline timestamp was recorded
        }
      } else {
        autoscrollLastNow = null;
      }
      // H-109: the same auto-scroll, extended to a plain selection or handle drag (scope item 3
      // — "the same auto-scroll behaviour H-64 gave marker drags").
      let stillSelectionAutoscrolling = false;
      if ((dragging || handleDragActive) && selectionAutoscrollDir !== 0) {
        const last = selectionAutoscrollLastNow;
        selectionAutoscrollLastNow = now;
        if (last !== null) {
          const dt = (now - last) / 1000;
          const next = advanceMarkerAutoscroll(
            startSample,
            selectionAutoscrollDir,
            dt,
            samplesPerPixel,
            lenSamples,
            viewportPx,
          );
          if (next !== startSample) {
            startSample = next;
            if (lastSelectionPointerClientX !== null) {
              const sample = sampleAtClientX(lastSelectionPointerClientX);
              if (sample !== null) {
                if (handleDragActive) {
                  handleDragTo(sample);
                } else {
                  dragTo(sample);
                }
              }
            }
            stillSelectionAutoscrolling = true;
          } else {
            // Already at the document edge in this direction: nothing left to animate until the
            // pointer moves again (which re-evaluates the direction from scratch).
            selectionAutoscrollDir = 0;
          }
        } else {
          stillSelectionAutoscrolling = true;
        }
      } else {
        selectionAutoscrollLastNow = null;
      }
      return isPlayheadMoving() || isRecording || stillAutoscrolling || stillSelectionAutoscrolling;
    },
    { name: "waveform" },
  );

  const lenSamples = $derived(doc.current.len_samples);
  const rateHz = $derived(doc.current.sample_rate_hz);
  /** H-71 (SPEC-005 §2.3): the running import job, if any — `null` once it's done/cancelled/
   * failed (H-20's `applyImportJobProgress` clears it back to a terminal state only briefly; the
   * `ImportProgressBar` then dismisses it, but this view only cares about "running"). */
  const importJob = $derived(doc.importJob);
  const isImporting = $derived(isImportRunning());
  const isOpen = $derived(hasDocument(doc.current) || isImporting);
  const isRecording = $derived(rec.state.recording);
  /**
   * H-76 (SPEC-005 §2.3 item 4, "while importing: zoom, scroll and selection work"): the sample
   * range every clamp/zoom/selection helper below treats as "the document" while an import is
   * running. `lenSamples` (`doc.current.len_samples`) is the *previous* document's length — 0 if
   * none was open, or a completely unrelated file's length otherwise — so using it here would
   * leave the view stuck at start=0/max-zoom-in (nothing to scroll or zoom out to) for the common
   * "first document" case, or bound it to the wrong file's length otherwise.
   *
   * Chosen bound: the import job's own `len_samples` (`import_started`'s probe result, SPEC-005
   * §2.3 step 1 — known before the decode loop even starts, for every format that reports a frame
   * count). That's exactly the length the progressive fill (H-71) is filling *toward*: scrolling
   * or zooming out to a not-yet-committed part of it shows `--wave-pending` columns
   * (`drawWebgl2`/`drawCanvas2d`'s `isImporting` branches), never a hard wall. A container that
   * reports no frame count at all (`len_samples: null`, rare) has no better bound available yet —
   * falls back to `lenSamples` (pre-H-76 behavior) until the import completes and a real document
   * replaces it.
   */
  const interactionLenSamples = $derived(
    isImporting ? (importJob?.lenSamples ?? lenSamples) : lenSamples,
  );
  // H-66 (SPEC-008 §2.11): the right-click menu's enablement must exactly match `EditMenu.svelte`'s
  // — a document open, a non-empty selection, and not while recording (`error.not_while_recording`
  // covers "or a document job", since the job's own modal dialog owns the window meanwhile).
  // H-82: an import is the one job that *isn't* modal (zoom/scroll/selection stay live, per
  // SPEC-005 §2.3 item 4), so it needs its own explicit `isImporting` check here too — see
  // `isImportRunning`'s doc comment for why editing must stay blocked even though the previous
  // document itself is untouched.
  const hasDoc = $derived(hasDocument(doc.current));
  const editSelected = $derived(hasSelection() && !isRecording && !isImporting);
  const editPasteEnabled = $derived(hasClipboard() && !isRecording && !isImporting);
  /** H-07: a new recording into an empty document (its take isn't committed until Stop). */
  const liveNewTake = $derived(isRecording && lenSamples === 0);
  /** H-21: a running record operation on a document with audio (`null`: none). */
  const layout = $derived(lenSamples > 0 ? opLayout(rec.op, rec.phase, rec.elapsedSamples) : null);
  /** H-21: the first document sample `peaks_get` must cover (an Insert's shifted part). A number,
   * so the request effect below re-runs only when it actually changes, not every frame. */
  const peaksFrom = $derived(
    opPeaksRequestStart(layout, Math.max(0, Math.floor(startSample)), viewportPx * samplesPerPixel),
  );

  // H-24 item 7 / H-35 / H-72: the amplitude ruler gutter, scaled by the real `verticalZoom`
  // (SPEC-006 §2.4/§2.2 — drag-to-zoom the gutter itself is still out of scope, see
  // amplitudeAxis.ts's doc comment), in whichever of the two ruler modes is current. `heightPx`
  // is this view's own measured canvas height (below).
  const ampTicks = $derived.by(() =>
    heightPx > 0
      ? ampRulerMode.current === "percent"
        ? amplitudeTicksPercent(heightPx, vzoom.current, 16)
        : amplitudeTicksDbfs(heightPx, vzoom.current, 16)
      : [],
  );
  // H-26: the ruler's labels, fitted (`fitGutterLabels`): the 0 dBFS labels at the top and
  // bottom edges align inward instead of being cut in half, and none touches the unit. The grid
  // lines still use every tick. H-72: percent mode has no separate unit corner label — each tick
  // already carries its own `%` (amplitudeAxis.ts's `amplitudeTicksPercent`), so a second "unit"
  // box would be redundant (and, unlike dBFS's plain digits, would collide with the sign).
  const ampLabels = $derived(
    fitGutterLabels(
      ampTicks.map((tick) => ({ ...tick, pos: tick.y, text: tick.label })),
      {
        length: heightPx,
        width: 48,
        fontPx: 10,
        lineHeightPx: 12,
        unit:
          ampRulerMode.current === "percent" ? undefined : { text: t("waveform.amp_unit"), fontPx: 10 },
      },
    ),
  );
  const zeroLineY = $derived(centerlineY(heightPx));

  // Zoom-full the first time a newly opened document's audio (rate + length — not just its path,
  // so Save As to a new path/format doesn't re-fit the still-unchanged audio) gets a known
  // viewport width (SPEC-006 §2.6 "zoom full at open"). Re-fits if the viewport wasn't known yet
  // when the document opened. H-12 (SPEC-018 §2.6.5): a document opened with a saved
  // `waveform_view` restores that viewport instead — clamped to this width, falling back to zoom
  // full when the restored `samples_per_pixel` is out of range ("an out-of-range
  // `samples_per_pixel` -> zoom full").
  // H-76: gated on `hasDoc` (the *real* document), not `isOpen` — `isOpen` also covers "an import
  // is running, no real document yet" (H-71), and fitting *that* to `lenSamples` (always 0 with
  // none open before) forced `samplesPerPixel` to 0 the instant an import's canvas first mounted.
  // The import gets its own, separate initial-fit effect right below, keyed off the job id instead
  // of `audioKeyFor` (there's no real audio revision to key on yet).
  $effect(() => {
    if (!hasDoc) {
      fittedForAudio = null;
      return;
    }
    const audioKey = audioKeyFor(rateHz, lenSamples);
    if (audioKey !== fittedForAudio && viewportPx > 0) {
      fittedForAudio = audioKey;
      const pending = consumePendingRestore(audioKey);
      const maxSpp = zoomFullSamplesPerPixel(lenSamples, viewportPx);
      if (
        pending &&
        pending.samplesPerPixel >= MIN_SAMPLES_PER_PIXEL &&
        pending.samplesPerPixel <= maxSpp
      ) {
        samplesPerPixel = pending.samplesPerPixel;
        startSample = clampStartSample(pending.startSample, samplesPerPixel, lenSamples, viewportPx);
      } else {
        samplesPerPixel = maxSpp;
        startSample = 0;
      }
    }
  });

  /** H-76 (SPEC-005 §2.3 item 4): fits the view to the *importing* document's own probed length
   * once, the moment its canvas first gets a known viewport width — mirrors the real-document fit
   * effect above, but keyed on the job id (an import has no `audio_rev` to key on, ADR-003
   * Amendment 7) and kept fully separate from `fittedForAudio` so cancelling/finishing an import
   * never re-fits whatever real document is (or was) open: that document's own fit is untouched
   * by this effect, in either direction. */
  let fittedImportJobId = $state<number | null>(null);
  $effect(() => {
    if (!isImporting || viewportPx <= 0) {
      fittedImportJobId = null;
      return;
    }
    const job = importJob!;
    if (fittedImportJobId !== job.jobId) {
      fittedImportJobId = job.jobId;
      samplesPerPixel = zoomFullSamplesPerPixel(interactionLenSamples, viewportPx);
      startSample = 0;
    }
  });

  // Issues a peaks_get request whenever the visible range or the document's audio_rev changes
  // (SPEC-006 §4.3). The response is applied asynchronously and redrawn on the next frame.
  // H-76: also skipped while an import job is running — nothing in `drawWebgl2`/`drawCanvas2d`'s
  // `isImporting` branches ever reads `requester.state`, so fetching it would only be wasted IPC
  // work racing the import for the backend's attention (and, if a previous document happened to
  // still be open, `startSample`/`samplesPerPixel` are by then bounded to the *importing*
  // document's length, not this one's — a request built from them would be meaningless anyway).
  $effect(() => {
    requester.setAudioRev(doc.current.audio_rev);
    if (viewportPx <= 0 || lenSamples <= 0 || rateHz <= 0 || isImporting) {
      return;
    }
    const spp = samplesPerPixel;
    const viewStart = Math.max(0, Math.floor(startSample));
    const start = Math.min(peaksFrom, viewStart);
    // H-21: an Insert operation also shows document audio from before the viewport start.
    const extra = viewStart - start;
    const level = pickLevel(spp);
    if (level === RAW_SPP) {
      const count = Math.min(Math.ceil(viewportPx * spp) + 2 + extra, 1 << 20);
      void requester.request(start, count, spp).then(frames.invalidate);
    } else {
      const fetchStart = Math.floor(start / level) * level;
      const count = Math.min(Math.ceil((viewportPx * spp + extra) / level) + 1, 65_536);
      void requester.request(fetchStart, count, spp).then(frames.invalidate);
    }
  });

  // H-07: while recording, keep the whole growing take zoomed to fit (floored at
  // LIVE_MIN_WINDOW_SECONDS so a very short take doesn't start over-zoomed).
  $effect(() => {
    if (!liveNewTake || viewportPx <= 0 || rateHz <= 0) {
      return;
    }
    const windowSamples = Math.max(rec.elapsedSamples, LIVE_MIN_WINDOW_SECONDS * rateHz);
    samplesPerPixel = zoomFullSamplesPerPixel(windowSamples, viewportPx);
    startSample = 0;
  });

  // H-23 (A-016, `recordFollow.ts`) + H-27 (SPEC-006 §2.8, `playbackFollow.ts`): one
  // viewport-writer effect drives whichever follow policy is active — page-flip during a record
  // operation on a document with audio (`layout` non-null — a plain new recording is handled by
  // the zoom-to-fit effect above instead), or continuous band-follow while playing. The two are
  // mutually exclusive transport states, so a single `ViewportWriter` (shared, not duplicated per
  // policy) is enough to tell a user-initiated viewport change (scroll, zoom, click-to-seek, the
  // shared scrollbar) apart from either policy's own last write.
  let recordFollowState = $state<RecordFollowState>(INITIAL_RECORD_FOLLOW_STATE);
  let playbackFollowState = $state<PlaybackFollowState>(INITIAL_PLAYBACK_FOLLOW_STATE);
  const viewportWriter = new ViewportWriter();
  const playheadFollowEnabled = $derived(settingsState().current?.playhead_follow ?? true);
  $effect(() => {
    if (viewportPx <= 0 || samplesPerPixel <= 0) {
      recordFollowState = INITIAL_RECORD_FOLLOW_STATE;
      playbackFollowState = INITIAL_PLAYBACK_FOLLOW_STATE;
      viewportWriter.reset();
      return;
    }
    const viewportSamples = viewportPx * samplesPerPixel;
    const userChanged = viewportWriter.isUserChange(startSample);
    if (layout) {
      playbackFollowState = INITIAL_PLAYBACK_FOLLOW_STATE;
      if (userChanged) {
        recordFollowState = suspendRecordFollow(startSample, viewportSamples);
      }
      const head = layout.at + layout.takeLen;
      const flip = followRecordHead(startSample, viewportSamples, head, recordFollowState);
      recordFollowState = flip.state;
      if (flip.startSample !== startSample) {
        startSample = flip.startSample;
      }
    } else if (transport.state.playing && playheadFollowEnabled) {
      recordFollowState = INITIAL_RECORD_FOLLOW_STATE;
      if (userChanged) {
        playbackFollowState = suspendPlaybackFollow();
      }
      const result = followPlayhead(
        startSample,
        viewportSamples,
        lenSamples,
        transport.playheadSamples,
        playbackFollowState,
      );
      playbackFollowState = result.state;
      if (result.startSample !== startSample) {
        startSample = clampStartSample(result.startSample, samplesPerPixel, lenSamples, viewportPx);
      }
    } else {
      // Not recording or playing (or the setting is off): neither policy runs, and its state
      // resets so the next Play/record operation starts fresh rather than resuming a stale
      // suspension (SPEC-006 §2.8's fallback resume rule — "or at the next play").
      recordFollowState = INITIAL_RECORD_FOLLOW_STATE;
      playbackFollowState = INITIAL_PLAYBACK_FOLLOW_STATE;
    }
    viewportWriter.set(startSample);
  });

  // H-07: polls record_peaks_get at ~10 Hz while recording (the document has no committed audio
  // yet, so the normal peaks_get effect above never fires: lenSamples stays 0 until Stop).
  $effect(() => {
    if (!isRecording) {
      liveBuckets = [];
      liveStartSample = 0;
      liveSpb = LIVE_PEAKS_SPB;
      return;
    }
    let disposed = false;
    const poll = async (): Promise<void> => {
      // H-10 item 6: request sizing uses the last known bucket size, not always the starting
      // `LIVE_PEAKS_SPB` — once the take has decimated, fewer (coarser) buckets cover the same
      // span, and this still safely over-requests otherwise (the backend just returns fewer).
      const count = Math.min(Math.ceil(rec.elapsedSamples / liveSpb) + 2, LIVE_MAX_BUCKETS);
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
        liveSpb = frame.samplesPerBucket || LIVE_PEAKS_SPB;
      }
    };
    void poll();
    const id = setInterval(() => void poll(), LIVE_POLL_MS);
    return () => {
      disposed = true;
      clearInterval(id);
    };
  });

  // H-71 (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 7): while an import job is running,
  // the document has no committed audio of its own to show yet either (the real document is only
  // swapped in on success, unchanged by this ticket) — this re-requests `import_peaks_get` for
  // the visible range on every zoom/scroll *and* every `job_progress` tick (read here, so this
  // effect re-runs on each one — no polling loop of its own, unlike H-07's live-take path, since
  // `job_progress` already ticks at SPEC-005 §2.3's 4-10 Hz). A response landing calls
  // `frames.invalidate()` directly (H-43: "invalidate() where non-reactive data lands").
  $effect(() => {
    const job = importJob;
    importRequester.setJobId(job && job.state === "running" ? job.jobId : null);
    if (!job || job.state !== "running" || viewportPx <= 0) {
      return;
    }
    void job.fraction; // re-run (and re-fetch) on every job_progress tick
    const spp = samplesPerPixel;
    const viewStart = Math.max(0, Math.floor(startSample));
    const level = pickLevel(spp);
    if (level === RAW_SPP) {
      const count = Math.min(Math.ceil(viewportPx * spp) + 2, 1 << 20);
      void importRequester.request(viewStart, count, spp).then(frames.invalidate);
    } else {
      const fetchStart = Math.floor(viewStart / level) * level;
      const count = Math.min(Math.ceil((viewportPx * spp) / level) + 1, 65_536);
      void importRequester.request(fetchStart, count, spp).then(frames.invalidate);
    }
  });

  // H-13 (ADR-009 §2/§4): WebGL2 primary renderer, Canvas2D automatic fallback (context creation
  // failure, `webglcontextlost`, or the `rendererPref` setting). Owns the canvas's context choice
  // for its whole lifetime — re-created only when the canvas element itself is re-created (the
  // `{#if isOpen}` branch closing/opening), matching the size-observer effect below (S1-03
  // gotcha: an element bound inside a closed `{#if}` is `undefined` in `onMount`).
  let glHost: GlContextHost | null = null;
  let glRenderer: WaveformGlRenderer | null = null;

  $effect(() => {
    const el = canvasEl;
    if (!el) {
      return;
    }
    const host = new GlContextHost(el, rendererPref().value, {
      // Deferred to a microtask (Svelte 5: mutating unrelated `$state` — here `notices.svelte.ts`'s
      // toast list — synchronously from inside an `$effect`'s own body can re-trigger that same
      // effect during the current flush; pushing the notice after this flush settles avoids it).
      onKindDecided: (kind, reason) => {
        if (kind === "canvas2d" && reason === "unavailable") {
          queueMicrotask(() =>
            pushNotice({ level: "info", key: "notice.renderer.fallback_waveform", params: {}, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null }),
          );
        }
      },
      onContextLost: () => {
        glRenderer?.dispose();
        glRenderer = null;
        frames.invalidate(); // redraw with the Canvas2D fallback
        queueMicrotask(() =>
          pushNotice({ level: "warning", key: "notice.renderer.context_lost_waveform", params: {}, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null }),
        );
      },
    });
    glHost = host;
    glRenderer = host.gl ? new WaveformGlRenderer(host.gl) : null;
    frames.invalidate();
    return () => {
      glRenderer?.dispose();
      glRenderer = null;
      host.dispose();
      glHost = null;
    };
  });

  // H-43: every input of `draw()` that can change, read in full on every run (no early return, so
  // no dependency is ever dropped the way H-32's `$effect(draw)` lost them); a change requests one
  // frame. Non-reactive inputs (peaks responses, the GL context) invalidate where they land.
  $effect(() => {
    void [canvasEl, viewportPx, heightPx, isOpen, lenSamples, rateHz, startSample, samplesPerPixel];
    void [vzoom.current, liveNewTake, liveBuckets, liveStartSample, liveSpb, rec.elapsedSamples, layout];
    void [isImporting, importJob?.jobId, importJob?.state];
    void [markers.list, selection.current, transport.state.loop_range, transport.playheadSamples];
    void [markerDrag];
    void [doc.current.audio_rev, themeState().revision];
    frames.invalidate();
  });

  function draw(): void {
    if (!canvasEl || viewportPx <= 0) {
      return;
    }
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(viewportPx * dpr));
    const backingH = Math.max(1, Math.round(heightPx * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    if (glHost?.kind === "webgl2" && glRenderer) {
      drawWebgl2(glRenderer, dpr, backingW, backingH);
      return;
    }
    drawCanvas2d(dpr);
  }

  /** SPEC-006 §4.5: the WebGL2 path shares its geometry with the Canvas2D fallback (`coords.ts`'s
   * `columnYRange`/`pixelAtSample`, `../render/overlayGeometry.ts`) rather than reimplementing the
   * pixel math, so the two renderers agree by construction. */
  function drawWebgl2(renderer: WaveformGlRenderer, dpr: number, backingW: number, backingH: number): void {
    const centerY = heightPx / 2;
    const bgColor = themeColors().wave.bg.rgba;
    const fillColor = themeColors().wave.fill.rgba;
    const overlay = new QuadBatch();
    let content: WaveformGlContent | null = null;
    // H-79 (SPEC-006 §2.12 Amendment 2): the selection wash is drawn as its own "underlay" pass,
    // before the wave content, so a same-hue wash never gets painted over an already-drawn wave
    // (the original bug) — the content below draws fully opaque on top of it. Populated only in
    // the branches that used to include the selection fill in `overlay` (the plain document view
    // and the recording operation view below) — never during import/live-take, matching the
    // Canvas2D fallback (`drawCanvas2d`) and the pre-H-79 behavior.
    const underlay = new QuadBatch();

    const vz = vzoom.current;
    if (isImporting) {
      // H-71 (SPEC-005 §2.3, SPEC-006 AC-13): the growing import's own peaks — never the normal
      // `peaks_get` state, which has nothing to show until the import commits (mirrors H-07's
      // `liveNewTake` below). `PARTIAL`/`NaN` buckets render as `--wave-pending` per column.
      const state = importRequester.state;
      const level = pickLevel(samplesPerPixel);
      if (state && state.level === level && state.buckets.length > 0) {
        if (level === RAW_SPP) {
          if (state.partial) {
            overlay.rect(0, 0, viewportPx, heightPx, themeColors().wave.pending.rgba);
          } else {
            const geometry = buildRawPolyline(
              state.buckets,
              state.startSample,
              startSample,
              samplesPerPixel,
              centerY,
              fillColor,
              showsDots(samplesPerPixel),
              themeColors().strokePx,
              vz,
            );
            content = { mode: "raw", geometry };
          }
        } else {
          const columns = reduceColumns(state.buckets, state.startSample, level, startSample, samplesPerPixel, Math.ceil(viewportPx));
          content = {
            mode: "columns",
            vertices: buildColumnQuads(columns, centerY, fillColor, vz, themeColors().wave.pending.rgba).toFloat32Array(),
          };
        }
      } else if (state?.partial) {
        overlay.rect(0, 0, viewportPx, heightPx, themeColors().wave.pending.rgba);
      }
    } else if (liveNewTake) {
      if (liveBuckets.length > 0) {
        const columns = reduceColumns(liveBuckets, liveStartSample, liveSpb, startSample, samplesPerPixel, Math.ceil(viewportPx));
        content = { mode: "columns", vertices: buildColumnQuads(columns, centerY, fillColor, vz).toFloat32Array() };
      }
      const recordHeadColor = themeColors().wave.recordHead.rgba;
      const px = pixelAtSample(rec.elapsedSamples, startSample, samplesPerPixel);
      overlay.vLine(px, 0, heightPx, recordHeadColor, themeColors().strokePx);
    } else if (layout) {
      // H-21: the operation view — the existing audio (Insert: shifted past `at`) plus the live
      // take at `at` in the record colour, the punch region, the record head.
      underlay.append(
        buildSelectionUnderlay({
          startSample,
          samplesPerPixel,
          viewportPx,
          heightPx,
          selection: selection.current,
          color: themeColors().wave.selectionFill.rgba,
        }),
      );
      const cols = opViewColumns(layout);
      const quads = buildColumnQuads(cols.base, centerY, fillColor, vz);
      quads.append(buildColumnQuads(cols.take, centerY, themeColors().wave.record.rgba, vz));
      content = { mode: "columns", vertices: quads.toFloat32Array() };
      if (layout.punchEnd !== null) {
        const x0 = Math.max(0, pixelAtSample(layout.at, startSample, samplesPerPixel));
        const x1 = Math.min(viewportPx, pixelAtSample(layout.punchEnd, startSample, samplesPerPixel));
        overlay.rect(x0, 0, x1, heightPx, themeColors().wave.punchRegion.rgba);
      }
      overlay.append(overlayBatch(opMarkers(layout, markers.list, isTakeMarker)));
      const headPx = pixelAtSample(layout.at + layout.takeLen, startSample, samplesPerPixel);
      overlay.vLine(headPx, 0, heightPx, themeColors().wave.recordHead.rgba, themeColors().strokePx);
    } else {
      underlay.append(
        buildSelectionUnderlay({
          startSample,
          samplesPerPixel,
          viewportPx,
          heightPx,
          selection: selection.current,
          color: themeColors().wave.selectionFill.rgba,
        }),
      );
      const selectionFill = selectionGeometry(selection.current, startSample, samplesPerPixel, viewportPx).fill;
      const highlight = selectionFill
        ? { startPx: selectionFill.x0, endPx: selectionFill.x1, color: themeColors().wave.fillSelected.rgba }
        : undefined;
      const state = requester.state;
      const level = pickLevel(samplesPerPixel);
      if (state && state.level === level && state.buckets.length > 0) {
        if (level === RAW_SPP) {
          const geometry = buildRawPolyline(
            state.buckets,
            state.startSample,
            startSample,
            samplesPerPixel,
            centerY,
            fillColor,
            showsDots(samplesPerPixel),
            themeColors().strokePx,
            vz,
            highlight,
          );
          content = { mode: "raw", geometry };
        } else {
          const columns = reduceColumns(state.buckets, state.startSample, level, startSample, samplesPerPixel, Math.ceil(viewportPx));
          content = {
            mode: "columns",
            vertices: buildColumnQuads(columns, centerY, fillColor, vz, undefined, highlight).toFloat32Array(),
          };
        }
      } else if (state?.partial) {
        overlay.rect(0, 0, viewportPx, heightPx, themeColors().wave.pending.rgba);
      }
      overlay.append(overlayBatch(markersForDraw()));
    }

    renderer.draw({
      backingWidthPx: backingW,
      backingHeightPx: backingH,
      cssWidthPx: viewportPx,
      cssHeightPx: heightPx,
      devicePixelRatio: dpr,
      background: bgColor,
      underlay: underlay.vertexCount > 0 ? underlay.toFloat32Array() : null,
      content,
      overlay: overlay.vertexCount > 0 ? overlay.toFloat32Array() : null,
    });
  }

  /** Selection border/markers (`markerList`)/playhead as one WebGL2 overlay batch, drawn *after*
   * the wave content — the selection *fill* is a separate underlay drawn before it (H-79, see
   * `drawWebgl2`), so `omitSelectionFill` keeps this batch from drawing it a second time. */
  function overlayBatch(markerList: MarkerDto[]): QuadBatch {
    return buildOverlayBatch({
      startSample,
      samplesPerPixel,
      viewportPx,
      heightPx,
      selection: selection.current,
      omitSelectionFill: true,
      loop: loopFromRange(transport.state.loop_range),
      markers: markerList,
      playheadSample: isOpen ? transport.playheadSamples : null,
      lineWidthPx: themeColors().strokePx,
      colors: {
        selectionFill: themeColors().wave.selectionFill.rgba,
        selectionBorder: themeColors().wave.selectionBorder.rgba,
        marker: themeColors().wave.marker.rgba,
        markerRegionFill: themeColors().wave.markerRegion.rgba,
        playhead: themeColors().wave.playhead.rgba,
        loop: themeColors().wave.loop.rgba,
      },
    });
  }

  /** H-21: the operation view's per-pixel columns (see `opLayout.ts`). */
  function opViewColumns(l: OpLayout): { base: Column[]; take: Column[] } {
    const width = Math.ceil(viewportPx);
    const state = requester.state;
    const level = pickLevel(samplesPerPixel);
    const empty = (): Column[] => new Array<Column>(width).fill(null);
    const docAt = (s: number): Column[] =>
      state && state.level === level && state.buckets.length > 0
        ? reduceColumns(state.buckets, state.startSample, level, s, samplesPerPixel, width)
        : empty();
    const takeCols =
      liveBuckets.length > 0
        ? reduceColumns(liveBuckets, liveStartSample, liveSpb, startSample - l.at, samplesPerPixel, width)
        : empty();
    return opColumns(l, docAt, takeCols, startSample, samplesPerPixel, width);
  }

  function drawCanvas2d(dpr: number): void {
    const ctx = canvasEl?.getContext("2d");
    if (!ctx) {
      return; // e.g. jsdom in tests, or a browser with no 2D canvas support
    }
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const bg = themeColors().wave.bg.css;
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, viewportPx, heightPx);

    const centerY = heightPx / 2;
    const vz = vzoom.current;
    // H-79: the part of the wave inside the selection is recolored so it still reads as
    // "selected" even where it fully covers the wash — scoped to the plain document view (the
    // common case a selection is made in); the import/live-recording/operation-view branches
    // below keep their existing single-color content unchanged.
    const selectionFillPx = selectionGeometry(selection.current, startSample, samplesPerPixel, viewportPx).fill;
    const highlightCss = selectionFillPx
      ? { startPx: selectionFillPx.x0, endPx: selectionFillPx.x1, color: themeColors().wave.fillSelected.css }
      : undefined;
    if (isImporting) {
      // H-71 (SPEC-005 §2.3, SPEC-006 AC-13): the growing import's own peaks (see `drawWebgl2`'s
      // matching branch for the full rationale) — `PARTIAL`/`NaN` buckets render as
      // `--wave-pending` per column via `drawColumns`'s `pendingColor`.
      const state = importRequester.state;
      const level = pickLevel(samplesPerPixel);
      if (state && state.level === level && state.buckets.length > 0) {
        if (level === RAW_SPP) {
          if (state.partial) {
            ctx.fillStyle = themeColors().wave.pending.css;
            ctx.fillRect(0, 0, viewportPx, heightPx);
          } else {
            drawRawPolyline(ctx, state.buckets, state.startSample, centerY, vz);
          }
        } else {
          drawColumns(ctx, state.buckets, state.startSample, level, centerY, vz, themeColors().wave.pending.css);
        }
      } else if (state?.partial) {
        ctx.fillStyle = themeColors().wave.pending.css;
        ctx.fillRect(0, 0, viewportPx, heightPx);
      }
      ctx.restore();
      return;
    }
    if (liveNewTake) {
      // H-07: the growing take (from record_peaks_get), plus a record-head line — never the
      // normal peaks_get state, which has nothing to show until the take is committed at Stop.
      if (liveBuckets.length > 0) {
        drawColumns(ctx, liveBuckets, liveStartSample, liveSpb, centerY, vz);
      }
      drawRecordHead(ctx, centerY);
      ctx.restore();
      return;
    }
    if (layout) {
      // H-21: the operation view (see `drawWebgl2`).
      // H-79 (SPEC-006 §2.12 Amendment 2): the selection wash is drawn before the wave content, so
      // a same-hue wash never paints over an already-drawn wave (the original bug) — the content
      // drawn below is fully opaque on top of it. The border lines are drawn later, alongside the
      // other overlays, so they stay crisp over the content.
      drawSelectionFill(ctx);
      const cols = opViewColumns(layout);
      if (layout.punchEnd !== null) {
        const x0 = Math.max(0, pixelAtSample(layout.at, startSample, samplesPerPixel));
        const x1 = Math.min(viewportPx, pixelAtSample(layout.punchEnd, startSample, samplesPerPixel));
        if (x1 > x0) {
          ctx.fillStyle = themeColors().wave.punchRegion.css;
          ctx.fillRect(x0, 0, x1 - x0, heightPx);
        }
      }
      fillColumns(ctx, cols.base, themeColors().wave.fill.css, centerY, vz);
      fillColumns(ctx, cols.take, themeColors().wave.record.css, centerY, vz);
      drawSelectionBorder(ctx);
      drawLoop(ctx);
      drawMarkers(ctx, opMarkers(layout, markers.list, isTakeMarker));
      drawPlayhead(ctx, centerY);
      const headPx = pixelAtSample(layout.at + layout.takeLen, startSample, samplesPerPixel);
      ctx.strokeStyle = themeColors().wave.recordHead.css;
      ctx.lineWidth = themeColors().strokePx;
      ctx.beginPath();
      ctx.moveTo(headPx + crispOffset(ctx.lineWidth), 0);
      ctx.lineTo(headPx + crispOffset(ctx.lineWidth), heightPx);
      ctx.stroke();
      ctx.restore();
      return;
    }
    // H-79 (SPEC-006 §2.12 Amendment 2): the selection wash is drawn before the wave content, so a
    // same-hue wash never paints over an already-drawn wave (the original bug) — the content drawn
    // below is fully opaque on top of it. The border lines are drawn after, with the other
    // overlays, so they stay crisp over the content.
    drawSelectionFill(ctx);
    const state = requester.state;
    const level = pickLevel(samplesPerPixel);
    if (state && state.level === level && state.buckets.length > 0) {
      if (level === RAW_SPP) {
        drawRawPolyline(ctx, state.buckets, state.startSample, centerY, vz, highlightCss);
      } else {
        drawColumns(ctx, state.buckets, state.startSample, level, centerY, vz, undefined, highlightCss);
      }
    } else if (state?.partial) {
      ctx.fillStyle = themeColors().wave.pending.css;
      ctx.fillRect(0, 0, viewportPx, heightPx);
    }
    drawSelectionBorder(ctx);
    drawLoop(ctx);
    drawMarkers(ctx, markersForDraw());
    drawPlayhead(ctx, centerY);
    ctx.restore();
  }

  /** Marker flags (S2-03, SPEC-009 §2.5's essential subset: drawing only — no drag, no flag
   * hit-testing on the canvas; the panel is the click-to-jump/rename/delete surface). A region
   * also gets a filled band and an end flag. Clipped to the visible viewport. */
  function drawMarkers(ctx: CanvasRenderingContext2D, list: MarkerDto[] = markers.list): void {
    if (!isOpen || list.length === 0) {
      return;
    }
    const flagColor = themeColors().wave.marker.css;
    const regionFill = themeColors().wave.markerRegion.css;
    for (const marker of list) {
      const startPx = pixelAtSample(marker.pos_samples, startSample, samplesPerPixel);
      if (marker.len_samples > 0) {
        const endPx = pixelAtSample(
          marker.pos_samples + marker.len_samples,
          startSample,
          samplesPerPixel,
        );
        if (endPx >= 0 && startPx <= viewportPx) {
          ctx.fillStyle = regionFill;
          ctx.fillRect(startPx, 0, endPx - startPx, heightPx);
          drawFlag(ctx, endPx, flagColor);
        }
      }
      if (startPx >= -6 && startPx <= viewportPx + 6) {
        ctx.strokeStyle = flagColor;
        ctx.lineWidth = themeColors().strokePx;
        ctx.beginPath();
        ctx.moveTo(startPx + crispOffset(ctx.lineWidth), 0);
        ctx.lineTo(startPx + crispOffset(ctx.lineWidth), heightPx);
        ctx.stroke();
        drawFlag(ctx, startPx, flagColor);
      }
    }
  }

  /** A small filled triangle at the top of the canvas (SPEC-006 §2.11's "flag"). */
  function drawFlag(ctx: CanvasRenderingContext2D, px: number, color: string): void {
    const w = 6;
    const h = 8;
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px + w, 0);
    ctx.lineTo(px, h);
    ctx.closePath();
    ctx.fill();
  }

  /** The time selection (S2-01, SPEC-006 §2.1/§2.9), clipped to the visible viewport. */
  /** H-37 (SPEC-006 §2.12 amendment): the loop brace and boundaries while looping. */
  function drawLoop(ctx: CanvasRenderingContext2D): void {
    const loop = loopFromRange(transport.state.loop_range);
    if (!loop) {
      return;
    }
    const geometry = loopGeometry(loop, startSample, samplesPerPixel, viewportPx);
    ctx.fillStyle = themeColors().wave.loop.css;
    if (geometry.strip) {
      ctx.fillRect(geometry.strip.x0, 0, geometry.strip.x1 - geometry.strip.x0, LOOP_STRIP_PX);
    }
    ctx.strokeStyle = themeColors().wave.loop.css;
    ctx.lineWidth = themeColors().strokePx;
    for (const px of geometry.lines) {
      ctx.beginPath();
      ctx.moveTo(px + crispOffset(ctx.lineWidth), 0);
      ctx.lineTo(px + crispOffset(ctx.lineWidth), heightPx);
      ctx.stroke();
    }
  }

  /** H-79: recolors a pixel `px` when it falls inside `highlight`'s range (the current
   * selection) — used by `fillColumns`/`drawRawPolyline` so the wave stays legible, and visibly
   * marked as selected, inside the selection (SPEC-006 §2.12 Amendment 2). */
  function colorAtPx(px: number, base: string, highlight?: { startPx: number; endPx: number; color: string }): string {
    return highlight && px >= highlight.startPx && px < highlight.endPx ? highlight.color : base;
  }

  /** The selection wash (S2-01, SPEC-006 §2.1/§2.9), clipped to the visible viewport. H-79: drawn
   * *before* the wave content (see `drawCanvas2d`) so it never paints over an already-drawn wave. */
  function drawSelectionFill(ctx: CanvasRenderingContext2D): void {
    const geometry = selectionGeometry(selection.current, startSample, samplesPerPixel, viewportPx);
    if (!geometry.fill) {
      return;
    }
    ctx.fillStyle = themeColors().wave.selectionFill.css;
    ctx.fillRect(geometry.fill.x0, 0, geometry.fill.x1 - geometry.fill.x0, heightPx);
  }

  /** H-79: the selection's boundary lines (`--wave-selection-handle`), drawn over the wave content
   * alongside the other overlays (loop/markers/playhead) so the edges stay visible against both
   * the selected and unselected background. */
  function drawSelectionBorder(ctx: CanvasRenderingContext2D): void {
    const geometry = selectionGeometry(selection.current, startSample, samplesPerPixel, viewportPx);
    if (geometry.lines.length === 0) {
      return;
    }
    ctx.strokeStyle = themeColors().wave.selectionBorder.css;
    ctx.lineWidth = themeColors().strokePx;
    for (const px of geometry.lines) {
      ctx.beginPath();
      ctx.moveTo(px + crispOffset(ctx.lineWidth), 0);
      ctx.lineTo(px + crispOffset(ctx.lineWidth), heightPx);
      ctx.stroke();
    }
  }

  function drawColumns(
    ctx: CanvasRenderingContext2D,
    buckets: Array<[number, number]>,
    bucketsStartSample: number,
    level: number,
    centerY: number,
    verticalZoom = 1,
    pendingColor?: string,
    highlight?: { startPx: number; endPx: number; color: string },
  ): void {
    const columns = reduceColumns(
      buckets,
      bucketsStartSample,
      level,
      startSample,
      samplesPerPixel,
      Math.ceil(viewportPx),
    );
    fillColumns(ctx, columns, themeColors().wave.fill.css, centerY, verticalZoom, pendingColor, highlight);
  }

  /** One `color` min/max column per pixel (`null`: nothing drawn there). `verticalZoom` (H-35,
   * SPEC-006 §2.4) defaults to `1` (unscaled). H-71 (SPEC-006 AC-13): a `PENDING_COLUMN` (see
   * `coords.ts::isPendingColumn`) draws full-height in `pendingColor` instead — omit it and such
   * a column is skipped like `null`, unchanged from before H-71. H-79: `highlight` recolors the
   * columns inside the current selection instead of skipping them (see {@link colorAtPx}). */
  function fillColumns(
    ctx: CanvasRenderingContext2D,
    columns: ReadonlyArray<Column>,
    color: string,
    centerY: number,
    verticalZoom = 1,
    pendingColor?: string,
    highlight?: { startPx: number; endPx: number; color: string },
  ): void {
    for (let px = 0; px < columns.length; px++) {
      const column = columns[px];
      if (!column) {
        continue;
      }
      if (isPendingColumn(column)) {
        if (pendingColor) {
          ctx.fillStyle = pendingColor;
          ctx.fillRect(px, 0, 1, centerY * 2);
        }
        continue;
      }
      const [mn, mx] = column;
      const [yTop, yBot] = columnYRange(mn, mx, centerY, verticalZoom);
      ctx.fillStyle = colorAtPx(px + 0.5, color, highlight);
      ctx.fillRect(px, yTop, 1, yBot - yTop);
    }
  }

  function drawRawPolyline(
    ctx: CanvasRenderingContext2D,
    samples: Array<[number, number]>,
    fetchStartSample: number,
    centerY: number,
    verticalZoom = 1,
    highlight?: { startPx: number; endPx: number; color: string },
  ): void {
    const baseColor = themeColors().wave.fill.css;
    ctx.lineWidth = themeColors().strokePx;
    // H-79: without a `highlight` this draws exactly as before — one path, one `stroke()` call
    // (performance-critical at raw zoom, ~100k segments, T-704). A highlight only splits the path
    // at its two color transitions (entering/leaving the selection), so it's still at most 3
    // `stroke()` calls total rather than one per segment. Each segment takes the color at its
    // later endpoint — same rule as the WebGL2 path's `buildRawPolyline` (`webglGeometry.ts`), so
    // the two renderers agree.
    let currentColor: string | null = null;
    let prevPx: number | null = null;
    let prevY: number | null = null;
    let pathOpen = false;
    for (let i = 0; i < samples.length; i++) {
      const sample = samples[i];
      if (!sample) {
        continue;
      }
      const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
      const y = centerY - sample[0] * verticalZoom * centerY;
      const color = colorAtPx(px, baseColor, highlight);
      if (prevPx === null || prevY === null) {
        // first point: nothing to connect yet.
      } else if (!pathOpen || color !== currentColor) {
        if (pathOpen) {
          ctx.stroke();
        }
        ctx.strokeStyle = color;
        ctx.beginPath();
        ctx.moveTo(prevPx, prevY);
        ctx.lineTo(px, y);
        pathOpen = true;
      } else {
        ctx.lineTo(px, y);
      }
      currentColor = color;
      prevPx = px;
      prevY = y;
    }
    if (pathOpen) {
      ctx.stroke();
    }
    if (showsDots(samplesPerPixel)) {
      for (let i = 0; i < samples.length; i++) {
        const sample = samples[i];
        if (!sample) {
          continue;
        }
        const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
        const y = centerY - sample[0] * verticalZoom * centerY;
        ctx.fillStyle = colorAtPx(px, baseColor, highlight);
        ctx.beginPath();
        ctx.arc(px, y, 1 + themeColors().strokePx / 2, 0, Math.PI * 2);
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
    ctx.strokeStyle = themeColors().wave.playhead.css;
    ctx.lineWidth = themeColors().strokePx;
    ctx.beginPath();
    ctx.moveTo(px + crispOffset(ctx.lineWidth), 0);
    ctx.lineTo(px + crispOffset(ctx.lineWidth), centerY * 2);
    ctx.stroke();
  }

  /** H-07: a line at the take's current length (the view is always zoomed so it's on-screen). */
  function drawRecordHead(ctx: CanvasRenderingContext2D, centerY: number): void {
    const px = pixelAtSample(rec.elapsedSamples, startSample, samplesPerPixel);
    ctx.strokeStyle = themeColors().wave.recordHead.css;
    ctx.lineWidth = themeColors().strokePx;
    ctx.beginPath();
    ctx.moveTo(px + crispOffset(ctx.lineWidth), 0);
    ctx.lineTo(px + crispOffset(ctx.lineWidth), centerY * 2);
    ctx.stroke();
  }

  // H-76: zoom/scroll/selection clamp against `interactionLenSamples` — the previous document's
  // length while nothing is importing (unchanged), the importing document's own probed length
  // while one is (see that derived value's doc comment).
  function zoomAt(anchorSample: number, anchorPx: number, nextSpp: number): void {
    const clampedSpp = clampSamplesPerPixel(nextSpp, interactionLenSamples, viewportPx);
    samplesPerPixel = clampedSpp;
    startSample = clampStartSample(
      zoomAroundSample(anchorSample, anchorPx, clampedSpp),
      clampedSpp,
      interactionLenSamples,
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
    zoomAt(anchorSample, anchorPx, zoomStep(samplesPerPixel, direction, interactionLenSamples, viewportPx));
  }

  /** H-35 (SPEC-006 §2.6): "Zoom to selection" — a no-op (per spec) with no document open or no
   * selection. Menu/toolbar-only today (no keyboard binding, see `actions.ts`'s doc comment). */
  function zoomToSelectionCommand(): void {
    if (!isOpen) {
      return;
    }
    const result = zoomToSelectionViewport(selection.current, interactionLenSamples, viewportPx);
    if (!result) {
      return;
    }
    samplesPerPixel = result.samplesPerPixel;
    startSample = result.startSample;
  }

  /** H-35 (SPEC-006 §2.6): "Zoom full" — fits the whole document to the viewport. */
  function zoomFullCommand(): void {
    if (!isOpen || viewportPx <= 0) {
      return;
    }
    const result = zoomFullViewport(interactionLenSamples, viewportPx);
    samplesPerPixel = result.samplesPerPixel;
    startSample = result.startSample;
  }

  /** `Alt+=`/`Alt+-` (SPEC-006 §2.4/§2.6): vertical (amplitude) zoom, power-of-two steps. */
  function zoomVerticalKeyboard(direction: 1 | -1): void {
    if (!isOpen) {
      return;
    }
    setVerticalZoom(verticalZoomStep(vzoom.current, direction));
  }

  /** `Alt+0` (H-35, no source-documented binding — see `actions.ts`): resets vertical zoom to 1×. */
  function resetVerticalZoomKeyboard(): void {
    if (!isOpen) {
      return;
    }
    setVerticalZoom(DEFAULT_VERTICAL_ZOOM);
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
    } else if (event.altKey) {
      // SPEC-006 §2.6: "Alt+wheel zooms vertically, centered on the ruler's current center
      // line" — true by construction (the y-mapping is always centred on `centerY`), so no
      // anchor math is needed here, just the scale factor.
      setVerticalZoom(verticalZoomStep(vzoom.current, event.deltaY > 0 ? -1 : 1));
    } else {
      const delta = event.deltaX !== 0 ? event.deltaX : event.deltaY;
      startSample = clampStartSample(
        startSample + delta * samplesPerPixel,
        samplesPerPixel,
        interactionLenSamples,
        viewportPx,
      );
    }
  }

  /** The document sample under `clientX`, clamped to the document (SPEC-006 §4.1). H-76: clamped
   * to `interactionLenSamples` so a selection/handle-drag/click-seek made while importing is
   * bounded to the importing document, not the previous one. */
  function sampleAtClientX(clientX: number): number | null {
    if (!containerEl) {
      return null;
    }
    const rect = containerEl.getBoundingClientRect();
    const px = clientX - rect.left;
    return Math.max(0, Math.min(sampleAtPixel(px, startSample, samplesPerPixel), interactionLenSamples));
  }

  /** The device-pixel x of `clientX` inside the canvas container, or `null` if it isn't mounted. */
  function pxAtClientX(clientX: number): number | null {
    if (!containerEl) {
      return null;
    }
    return clientX - containerEl.getBoundingClientRect().left;
  }

  /** The device-pixel y of `clientY` inside the canvas container (H-57's flag hit-testing),
   * or `null` if it isn't mounted. */
  function pyAtClientY(clientY: number): number | null {
    if (!containerEl) {
      return null;
    }
    return clientY - containerEl.getBoundingClientRect().top;
  }

  /** Hit-tests a pointerdown against every marker's flag (SPEC-009 §2.5) — takes priority over
   * the selection handle/plain-drag hit-testing below (only inside the flag's own hit box). */
  function flagHitAtClient(clientX: number, clientY: number): ReturnType<typeof hitTestMarkerFlag> {
    const px = pxAtClientX(clientX);
    const py = pyAtClientY(clientY);
    if (px === null || py === null) {
      return null;
    }
    return hitTestMarkerFlag(px, py, markers.list, startSample, samplesPerPixel);
  }

  /** The marker list as it should be drawn (H-57): the dragged marker's committed position is
   * replaced by its live preview once the drag has moved past the click threshold — the document
   * itself never changes mid-drag (SPEC-009 §2.5: "only the UI draws the marker at its preview
   * position"). */
  function markersForDraw(): MarkerDto[] {
    const drag = markerDrag;
    if (!drag || !drag.moved) {
      return markers.list;
    }
    return markers.list.map((m) =>
      m.id === drag.id ? { ...m, pos_samples: drag.preview.pos_samples, len_samples: drag.preview.len_samples } : m,
    );
  }

  /** Recomputes the drag preview from a client-x position (H-57, SPEC-009 §2.5): an *absolute*
   * target every time (`sampleAtClientX` + the grab offset), never an accumulated delta, so there
   * is no drift at any zoom. Applies the magnet (cursor/selection/other markers' edges within
   * `MARKER_MAGNET_PX`) unless Alt is held, then the shape-specific clamp. Never reads audio
   * samples — SPEC-009 §2.5: markers have no zero-crossing snap. Shared by `updateMarkerDragPreview`
   * (a real pointermove) and the auto-scroll frame tick below (H-64: the view moves under an
   * unmoving pointer, so the preview must be recomputed even without a new pointer event). */
  function applyMarkerDragPreview(clientX: number, altKey: boolean): void {
    const drag = markerDrag;
    if (!drag) {
      return;
    }
    const pointerSample = sampleAtClientX(clientX);
    if (pointerSample === null) {
      return;
    }
    const raw = pointerSample + drag.grabOffsetSamples;
    const cursorSample = transport.state.playing ? null : transport.playheadSamples;
    const targets = markerMagnetTargets(drag.id, markers.list, cursorSample, selection.current);
    const snapped = altKey ? raw : snapToMarkerMagnet(raw, targets, samplesPerPixel);
    let preview: { pos_samples: number; len_samples: number };
    if (drag.edge === "point") {
      preview = dragPointMarker(snapped, lenSamples);
    } else if (drag.wholeRegion) {
      preview = dragRegionWhole(snapped, drag.original, drag.edge, lenSamples);
    } else if (drag.edge === "start") {
      preview = dragRegionStart(snapped, drag.original.pos_samples + drag.original.len_samples);
    } else {
      preview = dragRegionEnd(snapped, drag.original.pos_samples, lenSamples);
    }
    markerDrag = { ...drag, preview };
  }

  /** A real pointermove during a marker drag: applies the preview, then re-evaluates auto-scroll
   * (H-64, SPEC-009 §2.5) from the pointer's raw (unclamped) canvas-relative x — beyond the left
   * or right edge scrolls the view every frame (below) until it moves back inside or the drag
   * ends. */
  function updateMarkerDragPreview(event: PointerEvent): void {
    if (!markerDrag) {
      return;
    }
    lastMarkerPointerClientX = event.clientX;
    lastMarkerPointerAltKey = event.altKey;
    applyMarkerDragPreview(event.clientX, event.altKey);
    const rawPx = pxAtClientX(event.clientX);
    markerAutoscrollDir = rawPx === null ? 0 : markerAutoscrollDirection(rawPx, viewportPx);
    if (markerAutoscrollDir !== 0) {
      autoscrollLastNow = null; // a fresh baseline, so the very next tick doesn't use a stale dt
      frames.invalidate();
    }
  }

  /** Resets every auto-scroll tracking field and releases the drag's captured pointer, if any
   * (drag end/cancel, H-64). */
  function resetMarkerAutoscroll(): void {
    markerAutoscrollDir = 0;
    autoscrollLastNow = null;
    lastMarkerPointerClientX = null;
    if (markerDragPointerId !== null) {
      try {
        containerEl?.releasePointerCapture?.(markerDragPointerId);
      } catch {
        // Already released (e.g. the browser auto-released it on pointerup) — harmless.
      }
      markerDragPointerId = null;
    }
  }

  /** Esc mid-drag (SPEC-009 §2.5): cancels without committing anything. */
  function cancelMarkerDrag(): void {
    markerDrag = null;
    resetMarkerAutoscroll();
  }

  /** H-109: the non-marker counterpart of `resetMarkerAutoscroll` — resets the plain/handle-drag
   * auto-scroll tracking and releases that drag's captured pointer, if any. */
  function resetSelectionAutoscroll(): void {
    selectionAutoscrollDir = 0;
    selectionAutoscrollLastNow = null;
    lastSelectionPointerClientX = null;
    if (dragPointerId !== null) {
      try {
        containerEl?.releasePointerCapture?.(dragPointerId);
      } catch {
        // Already released (e.g. the browser auto-released it on pointerup) — harmless.
      }
      dragPointerId = null;
    }
  }

  /** H-109: re-evaluates auto-scroll direction for a plain/handle drag from a real pointermove —
   * the non-marker counterpart of `updateMarkerDragPreview`'s auto-scroll half (H-64). */
  function updateSelectionAutoscroll(clientX: number): void {
    lastSelectionPointerClientX = clientX;
    const rawPx = pxAtClientX(clientX);
    selectionAutoscrollDir = rawPx === null ? 0 : markerAutoscrollDirection(rawPx, viewportPx);
    if (selectionAutoscrollDir !== 0) {
      selectionAutoscrollLastNow = null; // a fresh baseline for the next tick's dt
      frames.invalidate();
    }
  }

  /** H-109 (scope item 1): a captured pointer's own end-of-gesture events — `pointercancel` (the
   * platform aborts the gesture, e.g. a touch/pen interaction) and `lostpointercapture` (capture
   * ends for any reason, including the browser's implicit release right after a `pointerup` that
   * already ran, which makes this a no-op in the ordinary case). Ends whichever plain/handle drag
   * is still in progress at its last known position instead of leaving it stuck forever if a
   * `pointerup` is somehow never delivered at all. Deliberately scoped to the plain/handle-drag
   * path only — a marker drag's own end-of-gesture handling (pointerup, Esc) is untouched, exactly
   * as it was before this ticket (H-64). */
  function onPointerCaptureLost(): void {
    if (markerDrag || (!dragging && !handleDragActive)) {
      return;
    }
    dragging = false;
    handleDragActive = false;
    pointerDownClientX = null;
    pointerDownSample = null;
    pointerDownShiftKey = false;
    endHandleDrag();
    endDrag();
    resetSelectionAutoscroll();
  }

  /** Pointerup on a marker drag (SPEC-009 §2.5): a release before the drag threshold is a click
   * (activates the marker, §2.8); past it, commits one undo entry — `history.marker_move` for a
   * point drag or a Shift-drag of a region, `history.marker_resize` for a single-edge region
   * drag — unless the preview equals the original position ("releasing at the original position
   * commits nothing"). */
  function finishMarkerDrag(): void {
    const drag = markerDrag;
    markerDrag = null;
    resetMarkerAutoscroll();
    if (!drag) {
      return;
    }
    if (!drag.moved) {
      activateMarker(drag.id);
      return;
    }
    const { pos_samples, len_samples } = drag.preview;
    if (pos_samples === drag.original.pos_samples && len_samples === drag.original.len_samples) {
      return;
    }
    const kind: MarkerRangeKindDto = drag.edge === "point" || drag.wholeRegion ? "move" : "resize";
    void setMarkerRange(drag.id, pos_samples, len_samples, kind);
  }

  /** Hit-tests `clientX` against the current selection's two handles (SPEC-006 §2.9). `null`
   * with no selection, while locked (T-304), or when the pointer isn't within the hit width. */
  function handleHitAtClientX(clientX: number): "start" | "end" | null {
    const sel = selection.current;
    const px = pxAtClientX(clientX);
    if (!sel || px === null || isSelectionLocked()) {
      return null;
    }
    const startPx = pixelAtSample(sel.startSample, startSample, samplesPerPixel);
    const endPx = pixelAtSample(sel.endSample, startSample, samplesPerPixel);
    return hitTestHandle(px, startPx, endPx);
  }

  /** SPEC-006 §2.10: async zero-crossing snap, gated on `Settings.snap_to_zero_crossing`. A
   * no-op (returns `sample` unchanged) when the setting is off — kept synchronous in that (by
   * far the more common) case so existing callers/tests that don't await it still see the
   * unsnapped result applied immediately. H-76: also a no-op while an import job is running — the
   * snap reads real audio through the normal `peaksGet` (never `import_peaks_get`), which would
   * either read the wrong (previous) document or, with none open before, nothing at all. */
  function maybeSnapToZeroCrossing(sample: number): number | Promise<number> {
    if (!isOpen || isImporting || !settingsState().current?.snap_to_zero_crossing) {
      return sample;
    }
    return snapSampleToZeroCrossing(peaksGet, doc.current.audio_rev, lenSamples, sample);
  }

  /** T-701/A-020: one keyboard step — one *view* pixel worth of samples, so a nudge/extend moves
   * exactly as far as it visibly looks like it should at the current zoom (never less than one
   * sample, so it's never a no-op even fully zoomed in). */
  function keyboardStepSamples(): number {
    return Math.max(1, Math.round(samplesPerPixel));
  }

  /** Left/Right Arrow (T-701/A-020): with a selection, nudges the whole selection (length
   * unchanged); with none, nudges the cursor/playhead instead (the same `seek` a click uses). No
   * zero-crossing snap — only the extend commands below snap (T-206: "applies to
   * keyboard-extended selection edges"). */
  function nudgeKeyboard(direction: 1 | -1): void {
    if (!isOpen || isSelectionLocked()) {
      return;
    }
    const delta = keyboardStepSamples() * direction;
    const current = selection.current;
    if (current && hasSelection()) {
      const moved = nudgeSelectionRange(current, delta, interactionLenSamples);
      setSelectionFromResult([moved.startSample, moved.endSample]);
      return;
    }
    const next = Math.max(0, Math.min(transport.playheadSamples + delta, interactionLenSamples));
    void seek(next);
  }

  /** Shift+Left/Right Arrow (T-701/A-020): grows the selection from the edge in `direction`
   * (`waveform/selection.ts::extendSelectionEdge`), then applies the same zero-crossing snap as a
   * handle-drag-end/Shift+click (`maybeSnapToZeroCrossing`) to the edge that moved — never to the
   * edge that stayed put. */
  function extendKeyboard(direction: 1 | -1): void {
    if (!isOpen || isSelectionLocked()) {
      return;
    }
    const step = keyboardStepSamples();
    const target = extendSelectionEdge(
      selection.current,
      transport.playheadSamples,
      direction,
      step,
      interactionLenSamples,
    );
    if (!target) {
      return;
    }
    const movedEdge = direction === 1 ? target.endSample : target.startSample;
    const fixedEdge = direction === 1 ? target.startSample : target.endSample;
    const applyEdge = (finalEdge: number): void => {
      const range = normalizeSelection(fixedEdge, finalEdge);
      setSelectionFromResult(range ? [range.startSample, range.endSample] : null);
    };
    const snapped = maybeSnapToZeroCrossing(movedEdge);
    if (typeof snapped === "number") {
      applyEdge(snapped);
      return;
    }
    const seq = ++selectionSnapSeq;
    void snapped.then((sample) => {
      if (seq !== selectionSnapSeq) {
        return; // superseded by a newer gesture (SPEC-006 §2.3-style staleness guard)
      }
      applyEdge(sample);
    });
  }

  /** H-82 (SPEC-005 §2.3 item 4): the tooltip named in the spec, shown on every edit item this
   * menu disables *because an import is running* (as opposed to no selection/an empty clipboard/
   * recording, which show no tooltip here, matching pre-H-82 behavior). */
  const importingTitle = $derived(isImporting ? t("edit.unavailable_while_importing") : undefined);

  /** H-66 (SPEC-008 §2.11): an item that runs a keymap action, with its shortcut chip — same
   * routing (`dispatchAction`) and same labels/shortcuts as `EditMenu.svelte`'s equivalent row, so
   * the two menus can never drift apart. */
  function editMenuAction(
    id: string,
    label: string,
    actionId: Parameters<typeof dispatchAction>[0],
    disabled: boolean,
  ): MenuEntry {
    return {
      kind: "item",
      id,
      label,
      shortcut: shortcutLabelForAction(actionId),
      disabled,
      title: disabled ? importingTitle : undefined,
      testid: `waveform-menu-${id}`,
      onselect: () => dispatchAction(actionId),
    };
  }

  /** H-66 (SPEC-008 §2.11): Cut, Copy, Paste, Delete, Trim, Silence, Insert Silence — the same
   * seven ops as the Edit menu, same order, same enablement (`editSelected`/`editPasteEnabled`/
   * `hasDoc` above mirror `EditMenu.svelte` exactly). Silence and Insert Silence have no keymap
   * binding (menu-only, `registry.ts`), so they call the store directly like the Edit menu does.
   * H-82: all seven are additionally disabled while `isImporting` (`editSelected`/
   * `editPasteEnabled` already fold it in; Insert Silence checks it explicitly, like `hasDoc`/
   * `isRecording`). */
  const contextMenuItems = $derived<MenuEntry[]>([
    editMenuAction("cut", t("edit.cut"), "edit.cut", !editSelected),
    editMenuAction("copy", t("edit.copy"), "edit.copy", !editSelected),
    editMenuAction("paste", t("edit.paste"), "edit.paste", !editPasteEnabled),
    editMenuAction("delete", t("edit.delete"), "edit.delete", !editSelected),
    editMenuAction("trim", t("edit.trim"), "edit.trim", !editSelected),
    {
      kind: "item",
      id: "silence",
      label: t("edit.silence"),
      disabled: !editSelected,
      title: !editSelected ? importingTitle : undefined,
      testid: "waveform-menu-silence",
      onselect: () => void silence(),
    },
    {
      kind: "item",
      id: "insert-silence",
      label: t("edit.insert_silence"),
      disabled: !hasDoc || isRecording || isImporting,
      title: isImporting ? importingTitle : undefined,
      testid: "waveform-menu-insert-silence",
      onselect: openInsertSilenceDialog,
    },
  ]);

  /** H-66 (SPEC-008 §2.11, H-26): opens the right-click menu at the pointer, or under the canvas
   * container when opened from the keyboard (the context-menu key / Shift+F10 fire a `contextmenu`
   * event with `clientX`/`clientY` at 0,0 — same detection the Record context menu uses). Never
   * touches the selection or a marker drag: `onPointerDown` above already ignores non-primary
   * buttons, so no drag/handle-drag/marker-drag can have started for this gesture. */
  function onWaveformContextMenu(event: MouseEvent): void {
    event.preventDefault();
    if (!isOpen) {
      return;
    }
    const fromKeyboard = event.clientX === 0 && event.clientY === 0;
    contextMenuAnchor =
      fromKeyboard && containerEl ? containerEl : { x: event.clientX, y: event.clientY };
  }

  /** Mousedown (SPEC-006 §2.9): a hit on an existing selection's handle starts a handle drag;
   * Shift+click extends the far selection edge on pointerup; a plain mousedown starts a
   * live-updating click-drag (`state/selection.svelte.ts`), which a plain click (no movement)
   * undoes on pointerup by clearing the selection and seeking instead. */
  function onPointerDown(event: PointerEvent): void {
    // H-66: only the primary button drives selection/handle/marker drags — a right-click (or a
    // secondary-button press) must reach `onWaveformContextMenu` untouched, leaving the selection
    // and any marker exactly where they were.
    if (!isOpen || event.button !== 0) {
      return;
    }
    pointerDownClientX = event.clientX;
    pointerDownShiftKey = event.shiftKey;
    pointerDownSample = sampleAtClientX(event.clientX);
    dragging = false;
    handleDragActive = false;
    markerDrag = null;
    resetMarkerAutoscroll();
    // H-57 (SPEC-009 §2.5): a flag hit takes priority over the selection handle/plain-drag
    // hit-testing below, and works during recording too (only the drag itself is disabled then).
    // H-76 (SPEC-005 §2.3 item 4, "markers ... disabled" while importing): `markers.list` still
    // holds the *previous* document's markers here — they're already never drawn while importing
    // (`drawWebgl2`/`drawCanvas2d`'s `isImporting` branches), so hit-testing them too would only
    // let a click start an invisible drag against a marker that isn't even on screen.
    const flagHit =
      recordState().state.recording || isImporting
        ? null
        : flagHitAtClient(event.clientX, event.clientY);
    if (flagHit && pointerDownSample !== null) {
      const marker = markers.list.find((m) => m.id === flagHit.id);
      if (marker) {
        const grabbedSample = flagHit.edge === "end" ? marker.pos_samples + marker.len_samples : marker.pos_samples;
        markerDrag = {
          id: marker.id,
          edge: flagHit.edge,
          wholeRegion: event.shiftKey && flagHit.edge !== "point",
          grabOffsetSamples: grabbedSample - pointerDownSample,
          original: { pos_samples: marker.pos_samples, len_samples: marker.len_samples },
          preview: { pos_samples: marker.pos_samples, len_samples: marker.len_samples },
          moved: false,
        };
        markerDragPointerId = event.pointerId;
        // H-64: keeps pointermove/pointerup targeting this element even once the pointer strays
        // outside it (off-window included) — otherwise auto-scroll (SPEC-009 §2.5) would stop the
        // moment the cursor left the canvas, instead of continuing while held past its edge.
        containerEl?.setPointerCapture?.(event.pointerId);
      }
      return;
    }
    if (event.shiftKey || pointerDownSample === null) {
      return;
    }
    const hit = handleHitAtClientX(event.clientX);
    if (hit) {
      const sel = selection.current!;
      handleDragActive = true;
      nearHandle = true;
      beginHandleDrag(hit === "start" ? sel.endSample : sel.startSample);
      // H-109: same fix H-64 already gave marker drags — without capture, releasing over another
      // panel sends `pointerup` there instead of here, so the drag never ends.
      dragPointerId = event.pointerId;
      containerEl?.setPointerCapture?.(event.pointerId);
      return;
    }
    dragPointerId = event.pointerId;
    containerEl?.setPointerCapture?.(event.pointerId);
    beginDrag(pointerDownSample);
  }

  /** Live-updates the drag selection (SPEC-006 §2.9: "live-updating" while dragging) — a handle
   * drag or a plain click-drag, whichever `onPointerDown` started; otherwise just updates the
   * hover cursor (SPEC-006 §2.9: "hovering within 6 px of a boundary shows a resize cursor"). */
  function onPointerMove(event: PointerEvent): void {
    if (markerDrag) {
      if (!markerDrag.moved && pointerDownClientX !== null && Math.abs(event.clientX - pointerDownClientX) >= MARKER_DRAG_THRESHOLD_PX) {
        markerDrag = { ...markerDrag, moved: true };
      }
      if (markerDrag.moved) {
        updateMarkerDragPreview(event);
      }
      return;
    }
    if (handleDragActive) {
      const sample = sampleAtClientX(event.clientX);
      if (sample !== null) {
        handleDragTo(sample);
      }
      updateSelectionAutoscroll(event.clientX); // H-109, SPEC-009 §3 parity with marker drags
      return;
    }
    if (pointerDownShiftKey || pointerDownSample === null || pointerDownClientX === null) {
      nearHandle = handleHitAtClientX(event.clientX) !== null;
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
      updateSelectionAutoscroll(event.clientX); // H-109, SPEC-009 §3 parity with marker drags
    }
  }

  function onPointerLeave(): void {
    nearHandle = false;
  }

  function onPointerUp(event: PointerEvent): void {
    if (markerDrag) {
      pointerDownClientX = null;
      pointerDownSample = null;
      pointerDownShiftKey = false;
      finishMarkerDrag();
      return;
    }
    // H-109: release this drag's captured pointer (a no-op if none was captured, e.g. a Shift+
    // click or an out-of-range pointerdown) up front — every path below already computes its own
    // final selection from `event`, which capture guarantees still targets this element even when
    // the button was released elsewhere (over another panel, or outside the window).
    resetSelectionAutoscroll();
    const wasDragging = dragging;
    const wasHandleDrag = handleDragActive;
    const shiftKey = pointerDownShiftKey;
    const downSample = pointerDownSample;
    pointerDownClientX = null;
    pointerDownSample = null;
    pointerDownShiftKey = false;
    dragging = false;
    handleDragActive = false;
    if (!isOpen || downSample === null) {
      return;
    }
    const upSample = sampleAtClientX(event.clientX) ?? downSample;

    if (wasHandleDrag) {
      const fixedSample = endHandleDrag();
      if (fixedSample === null) {
        return;
      }
      const snapped = maybeSnapToZeroCrossing(upSample);
      if (typeof snapped === "number") {
        const range = normalizeSelection(fixedSample, snapped);
        setSelectionFromResult(range ? [range.startSample, range.endSample] : null);
        return;
      }
      const seq = ++selectionSnapSeq;
      void snapped.then((sample) => {
        if (seq !== selectionSnapSeq) {
          return; // superseded by a newer gesture (SPEC-006 §2.3-style staleness guard)
        }
        const range = normalizeSelection(fixedSample, sample);
        setSelectionFromResult(range ? [range.startSample, range.endSample] : null);
      });
      return;
    }

    if (shiftKey) {
      const snapped = maybeSnapToZeroCrossing(upSample);
      if (typeof snapped === "number") {
        shiftClickTo(snapped, transport.playheadSamples);
        return;
      }
      const seq = ++selectionSnapSeq;
      void snapped.then((sample) => {
        if (seq !== selectionSnapSeq) {
          return;
        }
        shiftClickTo(sample, transport.playheadSamples);
      });
      return;
    }

    if (wasDragging) {
      dragTo(upSample);
      endDrag();
      const anchorSnap = maybeSnapToZeroCrossing(downSample);
      const endSnap = maybeSnapToZeroCrossing(upSample);
      if (typeof anchorSnap === "number" && typeof endSnap === "number") {
        return; // snap disabled: dragTo's unsnapped result above already stands
      }
      const seq = ++selectionSnapSeq;
      void Promise.all([anchorSnap, endSnap]).then(([a, b]) => {
        if (seq !== selectionSnapSeq) {
          return;
        }
        const range = normalizeSelection(a, b);
        if (range) {
          setSelectionFromResult([range.startSample, range.endSample]);
        }
        // `a === b` (snapping collapsed the selection to a point): keep the unsnapped drag
        // result rather than replacing a real selection with nothing.
      });
      return;
    }
    // A plain click (no drag): clears the selection and moves the cursor (SPEC-006 §2.9) —
    // never snapped (SPEC-006 §2.10: only selection boundaries snap, not cursor placement).
    endDrag();
    clearSelection();
    void seek(upSample);
  }

  /** Double-click selects the entire document (SPEC-006 §2.9, same as Ctrl+A). */
  function onDoubleClick(): void {
    if (isOpen) {
      selectAllOf(interactionLenSamples);
    }
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
      registerAction("waveform.zoom_to_selection", zoomToSelectionCommand),
      registerAction("waveform.zoom_full", zoomFullCommand),
      registerAction("waveform.zoom_in_vertical", () => zoomVerticalKeyboard(1)),
      registerAction("waveform.zoom_out_vertical", () => zoomVerticalKeyboard(-1)),
      registerAction("waveform.zoom_reset_vertical", resetVerticalZoomKeyboard),
      registerAction("waveform.select_all", () => {
        if (isOpen) {
          selectAllOf(interactionLenSamples);
        }
      }),
      // H-57 (SPEC-009 §2.5): Esc cancels an in-progress marker drag instead of clearing the
      // time selection — the drag hasn't committed anything for Esc to "undo" either way.
      registerAction("waveform.deselect", () => {
        if (markerDrag) {
          cancelMarkerDrag();
          return;
        }
        clearSelection();
      }),
      registerAction("selection.nudge_left", () => nudgeKeyboard(-1)),
      registerAction("selection.nudge_right", () => nudgeKeyboard(1)),
      registerAction("selection.extend_left", () => extendKeyboard(-1)),
      registerAction("selection.extend_right", () => extendKeyboard(1)),
    ];

    cleanups.push(() => frames.dispose());

    return () => {
      for (const cleanup of cleanups) {
        cleanup();
      }
    };
  });
</script>

<div class="waveform-view" data-testid="waveform-view">
  {#if isOpen}
    <div class="body" data-testid="waveform-body">
      <div class="amp-ruler" data-testid="waveform-amp-ruler">
        {#if ampRulerMode.current !== "percent"}
          <span class="unit">{t("waveform.amp_unit")}</span>
        {/if}
        {#each ampLabels as tick, i (tick.y + "-" + i)}
          <span class="tick" data-align={tick.align} style={`top: ${tick.y}px`}>{tick.label}</span>
        {/each}
      </div>
      <!-- H-66: `role="application"` + `tabindex="0"` (same device SpectrumPlot.svelte uses for its
           canvas) makes this a real keyboard focus target, so the context-menu key/Shift+F10 fire a
           native `contextmenu` event here instead of wherever focus happened to be — Svelte's a11y
           list doesn't count "application" as interactive, hence the ignores. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="canvas-container"
        class:resize-cursor={nearHandle}
        role="application"
        tabindex="0"
        aria-label={t("waveform.canvas_label")}
        bind:this={containerEl}
        onwheel={onWheel}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerCaptureLost}
        onlostpointercapture={onPointerCaptureLost}
        onpointerleave={onPointerLeave}
        ondblclick={onDoubleClick}
        oncontextmenu={onWaveformContextMenu}
      >
        <canvas
          bind:this={canvasEl}
          aria-label={t("waveform.canvas_label")}
          data-testid="waveform-canvas"
        ></canvas>
        <div class="amp-grid" data-testid="waveform-amp-grid">
          {#each ampTicks as tick, i (tick.y + "-" + i)}
            <div class="grid-line" style={`top: ${tick.y}px`}></div>
          {/each}
          <div class="zero-line" data-testid="waveform-zero-line" style={`top: ${zeroLineY}px`}></div>
        </div>
      </div>
      <Menu
        open={contextMenuAnchor !== null}
        anchor={contextMenuAnchor}
        items={contextMenuItems}
        label={t("menu.edit")}
        testid="waveform-context-menu"
        onclose={() => (contextMenuAnchor = null)}
      />
    </div>
  {:else}
    <!-- H-25: no document yet — an invitation to act, not a grey sentence. Same actions as
         File → Open… and File → New Recording…. -->
    <EmptyState
      icon="waveform"
      title={t("empty.document.title")}
      description={t("empty.document.body")}
      testid="waveform-empty"
      shortcuts={[
        { label: t("empty.document.shortcut.open"), keys: shortcutLabelForAction("file.open") ?? "" },
        { label: t("empty.document.shortcut.record"), keys: shortcutLabelForAction("record.toggle") ?? "" },
        { label: t("empty.document.shortcut.play"), keys: shortcutLabelForAction("transport.play_pause") ?? "" },
      ].filter((s) => s.keys !== "")}
    >
      {#snippet actions()}
        <Button variant="primary" icon="open" testid="empty-open" onclick={() => dispatchAction("file.open")}>
          {t("empty.document.open")}
        </Button>
        <Button
          icon="record"
          testid="empty-new-recording"
          disabled={recState.state.recording || recState.state.finishing}
          onclick={() => openNewRecordingPrompt(settingsState().current?.default_format ?? NEW_RECORDING_FALLBACK)}
        >
          {t("empty.document.record")}
        </Button>
      {/snippet}
    </EmptyState>
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


  /* H-24 item 7: the amplitude gutter — same width/style convention as the spectral pane's
   * frequency ruler (`spectrogram/SpectralView.svelte`'s `.ruler`), so the shared time ruler
   * above both panes (`EditorView.svelte`) can align to one fixed inset. */
  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .amp-ruler {
    position: relative;
    width: 48px;
    flex: none;
    border-right: 1px solid var(--wave-ruler-grid);
    background: var(--surface-panel);
    overflow: hidden;
  }

  .amp-ruler .unit {
    position: absolute;
    top: 2px;
    left: 2px;
    color: var(--wave-ruler-text);
    font-size: 0.6rem;
  }

  .amp-ruler .tick {
    position: absolute;
    right: 2px;
    color: var(--wave-ruler-text);
    font-size: 10px;
    line-height: 12px;
    font-variant-numeric: tabular-nums;
    transform: translateY(-50%);
    white-space: nowrap;
  }

  .amp-ruler .tick[data-align="start"] {
    transform: translateY(0);
  }

  .amp-ruler .tick[data-align="end"] {
    transform: translateY(-100%);
  }

  .canvas-container {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  /* T-206 (SPEC-006 §2.9): a resize cursor within SELECTION_HANDLE_HIT_PX of a selection edge. */
  .canvas-container.resize-cursor {
    cursor: ew-resize;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  /* DOM-positioned grid lines (not canvas-drawn): they stay correct under both the WebGL2 and
   * Canvas2D renderers without touching either one's drawing code (item 4: never sized from the
   * canvas's own content — these come straight from `amplitudeAxis.ts`'s pure tick math). */
  .amp-grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .grid-line {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: var(--wave-ruler-grid);
    opacity: 0.5;
  }

  .zero-line {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: var(--wave-zero-line);
  }
</style>
