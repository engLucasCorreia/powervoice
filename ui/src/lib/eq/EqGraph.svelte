<script lang="ts">
  import { Channel } from "@tauri-apps/api/core";
  import {
    eqAxisLayout,
    EQ_AXIS_FONT_PX,
    EQ_AXIS_GAP_PX,
    EQ_AXIS_INSET_PX,
    EQ_AXIS_LINE_PX,
  } from "./axisLayout";
  import { t } from "../i18n";
  import type { RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
  import { analyzerSubscribe, analyzerUnsubscribe, rackResponseCurve } from "../ipc/commands";
  import type { AnalyzerFrame } from "../ipc/analyzer";
  import { setParamPlain, setParamPlainDragged } from "../rack/rack.svelte";
  import { totalCurveToScreen } from "./curvePoints";
  import { CoalescedCurveRequest } from "./curveRequest";
  import { wheelNotches, wheelQFactor, dragPosition, type DragStart } from "./drag";
  import {
    analyzerBandFreqsHz,
    curveRequestFreqs,
    freqForX,
    graphMaxHz,
    EQ_MIN_HZ,
    xForFreq,
  } from "./freqAxis";
  import {
    dbForY,
    gainAxisTicks,
    yForDb,
    EQ_GAIN_RANGE_DEFAULT_DB,
    EQ_GAIN_RANGE_WIDE_DB,
  } from "./gainAxis";
  import {
    keyFreqHz,
    keyGainDb,
    keyQValue,
    stepSlopeIndex,
  } from "./keyboardNav";
  import { EqLiveAnnouncer } from "./liveRegion";
  import {
    buildEqNodes,
    hitTestNode,
    nodeFullName,
    nodeGainDb,
    nodeFreqsHz,
    nodeShortLabel,
    nodeValueText,
    paramRangeOf,
    type EqNode,
  } from "./nodes";
  import { openEqExpanded } from "./eqExpanded.svelte";
  import { createEqSpectrumFeed } from "./spectrumFeed";
  import {
    EQ_SPECTRUM_FLOOR_DBFS,
    spectrumOverlayPoints,
    yForSpectrumDbfs,
  } from "./spectrumOverlay";
  import { formatRulerFreqHz } from "../spectrum/freqAxis";
  import { eqBandColor, themeColors } from "../theme/themeColors";
  import { themeState } from "../theme/theme.svelte";
  import { createFrameClient } from "../render/frameScheduler";
  import { IconButton } from "../ui";
  import { formatNumber } from "../ui/units";

  /**
   * The EQ graph panel (S3-07 + H-84, SPEC-015 §2.6): log-frequency axis, ±12/±24 dB gain axis,
   * the total response curve drawn only from the module's `ResponseCurve` (via
   * `rack_response_curve`, never evaluated in the UI — AC-17), draggable/keyboard-operable nodes
   * per band, an optional live spectrum overlay, and an expanded floating view. Rendered in the
   * slot body above the generic parameter panel (which stays available below) for any module
   * exposing `curve_handles` (currently only the Parametric EQ) — `mode="expanded"` renders the
   * same graph larger inside `EqExpandedView.svelte`.
   *
   * H-86 (SPEC-015 §2.6.4) aligned the mouse gestures with the keyboard's: double-click now
   * *resets* a band's frequency/gain/Q (or slope) to defaults, same as Home, on/off unchanged;
   * Alt+click toggles the band on/off (the header toggles still do the same thing); and the wheel
   * steps HP/LP slope one notch per notch, reusing `keyboardNav.ts`'s `stepSlopeIndex` like ↑/↓
   * does.
   *
   * Still out of scope (S3-07 "Out", not part of H-84 or H-86 either): the right-click context
   * menu, and hover tooltips (H-84 adds the same Rust-sourced text as `aria-valuetext`/the live
   * region, but not a visual hover tooltip).
   */
  let {
    slotIndex,
    rackSlot,
    rateHz,
    mode = "compact",
    graphHeightPx: graphHeightPxProp,
  }: {
    slotIndex: number;
    rackSlot: RackSlotDto;
    rateHz: number;
    /** `"expanded"` renders inside `EqExpandedView.svelte`: a caller-sized graph and no Expand
     * button (SPEC-015 §2.6.1). */
    mode?: "compact" | "expanded";
    /** Required (and used) only in `"expanded"` mode. */
    graphHeightPx?: number;
  } = $props();

  const COMPACT_GRAPH_HEIGHT_PX = 160; // SPEC-015 §2.6 "graph_height_compact"
  const graphHeight = $derived(
    mode === "expanded" ? Math.max(120, graphHeightPxProp ?? COMPACT_GRAPH_HEIGHT_PX) : COMPACT_GRAPH_HEIGHT_PX,
  );

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let curve = $state<ResponseCurveDto | null>(null);
  let gainRangeDb = $state(EQ_GAIN_RANGE_DEFAULT_DB);
  let selected = $state<number | null>(null);

  // --- Live spectrum overlay (H-84, SPEC-015 §2.6.3 item 1 / §2.6.6 "Spectrum" / AC-21) --------

  let spectrumOn = $state(true); // "Spectrum toggle on, the default" (SPEC-015 §2.6.3 item 1)
  let spectrumFrame = $state<AnalyzerFrame | null>(null);

  // Subscribes only while `spectrumOn` (and this graph instance is mounted, i.e. visible —
  // RackSlot only renders EqGraph for an open, active slot); unsubscribes on toggle-off, on
  // unmount (hidden/closed), or if the effect re-runs for any other reason. AC-21's "the tap
  // turns off once the last subscriber leaves" is the backend's job (analyzer.svelte.ts's own
  // doc comment); this only has to hold up its own end of that contract.
  $effect(() => {
    if (!spectrumOn) {
      spectrumFrame = null;
      return;
    }
    let cancelled = false;
    let subscriberId: number | undefined;
    const feed = createEqSpectrumFeed((frame) => {
      if (!cancelled) {
        spectrumFrame = frame;
      }
    });
    analyzerSubscribe(new Channel<ArrayBuffer>((message) => feed.handleMessage(message)), "medium")
      .then((id) => {
        if (cancelled) {
          analyzerUnsubscribe(id).catch(() => {
            // Torn down already; a failed unsubscribe during teardown is harmless.
          });
          return;
        }
        subscriberId = id;
      })
      .catch(() => {
        // No subscription: the overlay just stays empty (never blocks the rest of the graph).
      });
    return () => {
      cancelled = true;
      spectrumFrame = null;
      if (subscriberId !== undefined) {
        analyzerUnsubscribe(subscriberId).catch(() => {
          // Harmless during teardown.
        });
      }
    };
  });

  // --- Keyboard nodes (H-84, SPEC-015 §2.6.5 / AC-19) -------------------------------------------

  let focusedComponent: number | null = $state(null);
  let liveText = $state("");
  const announcer = new EqLiveAnnouncer(undefined, (text) => (liveText = text));
  $effect(() => () => announcer.dispose());

  const fLo = EQ_MIN_HZ;
  const fHi = $derived(graphMaxHz(rateHz));
  const nodes = $derived(
    buildEqNodes(rackSlot.curve_handles ?? [], rackSlot.params, rackSlot.values),
  );

  const fetcher = new CoalescedCurveRequest<ResponseCurveDto>(
    (points) => rackResponseCurve(slotIndex, points),
    (result) => (curve = result),
  );

  // Re-request the curve whenever the node values, the graph's frequency range, or its width
  // change (SPEC-015 §2.6.6 "when the curve is requested"), coalesced to one request per
  // animation frame by `fetcher`.
  $effect(() => {
    if (nodes.length === 0 || width <= 0) {
      return;
    }
    const points = curveRequestFreqs(fLo, fHi, width, nodeFreqsHz(nodes), 512);
    fetcher.request(points);
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

  // H-43: draw on demand from the shared frame scheduler (`render/frameScheduler.ts`), replacing
  // H-32's perpetual rAF loop. H-32's lesson still holds: a reactive `$effect(() => draw())` only
  // re-subscribes to what its *last* run read, so a draw that bailed out at an unsettled width (or
  // threw) before reading `curve`/`nodes` never woke up for their arrival. Here the dependency list
  // is read in full on every run, separately from the draw, and a draw that throws is retried by
  // the scheduler on the next frames (and any later change draws again).
  const frames = createFrameClient(() => draw(), { name: "eq-graph" });
  $effect(() => {
    void [
      canvasEl,
      width,
      graphHeight,
      curve,
      nodes,
      gainRangeDb,
      selected,
      fHi,
      themeState().revision,
      spectrumOn,
      spectrumFrame,
    ];
    frames.invalidate();
  });
  $effect(() => () => frames.dispose());

  /** The UI font for canvas labels (a font stack, not a colour). */
  function fontFamily(): string {
    return (canvasEl ? getComputedStyle(canvasEl).getPropertyValue("--pv-font-sans").trim() : "") || "sans-serif";
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
    const backingH = Math.max(1, Math.round(graphHeight * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    ctx.save();
    try {
      drawInner(ctx, dpr);
    } finally {
      // A throw propagates to the frame scheduler, which logs it (dev builds) and retries on the
      // next frame (H-32/H-43). Always balances the `ctx.save()` above, even after a throw — an unbalanced context stack
      // would otherwise distort every later frame's transform.
      ctx.restore();
    }
  }

  function drawInner(ctx: CanvasRenderingContext2D, dpr: number): void {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, graphHeight);

    // Live spectrum overlay, drawn first — behind the grid, curves and nodes (SPEC-015 §2.6.3
    // "back to front" item 1), on its own fixed −90…0 dBFS scale independent of the gain range
    // (§2.6.2 "Spectrum scale", AC-21).
    const frame = spectrumFrame;
    if (spectrumOn && frame && frame.levelsDb.length > 0) {
      const bandFreqs = analyzerBandFreqsHz(frame.bandCount, frame.f0Hz, frame.bandsPerOctave);
      const points = spectrumOverlayPoints(
        bandFreqs,
        frame.levelsDb,
        (f) => xForFreq(f, width, fLo, fHi),
        fLo,
        fHi,
      ).filter((p) => Number.isFinite(p.x));
      if (points.length > 0) {
        const bottomY = yForSpectrumDbfs(EQ_SPECTRUM_FLOOR_DBFS, graphHeight);
        ctx.beginPath();
        points.forEach((p, i) => {
          const y = yForSpectrumDbfs(Number.isFinite(p.db) ? p.db : EQ_SPECTRUM_FLOOR_DBFS, graphHeight);
          if (i === 0) {
            ctx.moveTo(p.x, y);
          } else {
            ctx.lineTo(p.x, y);
          }
        });
        ctx.lineTo(points[points.length - 1]!.x, bottomY);
        ctx.lineTo(points[0]!.x, bottomY);
        ctx.closePath();
        ctx.fillStyle = themeColors().analyzer.fill.css;
        ctx.fill();
      }
    }

    // Grid: 0 dB emphasised, every 3 dB (±12) or 6 dB (±24) otherwise (SPEC-015 §2.6.2).
    const gainTicks = gainAxisTicks(graphHeight, gainRangeDb);
    ctx.strokeStyle = themeColors().eq.grid.css;
    ctx.lineWidth = 1;
    for (const tick of gainTicks) {
      const y = Math.round(tick.y) + 0.5;
      ctx.globalAlpha = tick.db === 0 ? 1 : 0.4;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // H-26: scale labels from the tested layout (`axisLayout.ts`) — gain labels down the left
    // edge, frequencies along the bottom, "Hz" in its reserved slot at the end of that row, the
    // gain unit as the toolbar's axis title; nothing overlaps. Each label sits on a small patch
    // of the graph background so a grid line never runs through its text.
    const labels = eqAxisLayout(
      width,
      graphHeight,
      gainRangeDb,
      fLo,
      fHi,
      formatRulerFreqHz,
      t("eq.graph.freq_unit"),
    );
    const family = fontFamily();
    const patch = themeColors().eq.labelPatch.css;
    ctx.font = `${EQ_AXIS_FONT_PX}px ${family}`;
    for (const label of [...labels.gain, ...labels.freq, labels.freqUnit]) {
      ctx.fillStyle = patch;
      ctx.fillRect(label.rect.x - 1, label.rect.y, label.rect.width + 2, label.rect.height);
      ctx.fillStyle = themeColors().eq.labelText.css;
      ctx.textAlign = label.align;
      ctx.textBaseline = label.baseline;
      ctx.fillText(label.text, label.x, label.y);
    }
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";

    // The expanded view labels the fixed spectrum scale on its right edge (§2.6.2); the compact
    // view doesn't (too little room, and the gain axis already owns the left edge there). Drawn
    // here — after the grid and the main axis labels, each on its own patch — so a grid line
    // never runs through a digit, and the bottom tick clears the frequency label row instead of
    // fighting the "Hz" unit slot for the same corner.
    if (mode === "expanded" && spectrumOn && spectrumFrame) {
      ctx.font = `${EQ_AXIS_FONT_PX}px ${family}`;
      ctx.textAlign = "right";
      const rightX = width - EQ_AXIS_INSET_PX;
      const freqRowTopY = graphHeight - EQ_AXIS_INSET_PX - EQ_AXIS_LINE_PX;
      for (const dbfs of [0, -45, EQ_SPECTRUM_FLOOR_DBFS]) {
        // House convention (units.ts): a real minus sign, not an ASCII hyphen — the dB axis on
        // the left already reads "−12", so these must match it.
        const label = `${formatNumber(dbfs, 0)} dBFS`;
        const baseline: CanvasTextBaseline =
          dbfs === 0 ? "top" : dbfs === EQ_SPECTRUM_FLOOR_DBFS ? "bottom" : "middle";
        const y =
          dbfs === EQ_SPECTRUM_FLOOR_DBFS
            ? Math.min(yForSpectrumDbfs(dbfs, graphHeight), freqRowTopY - EQ_AXIS_GAP_PX)
            : yForSpectrumDbfs(dbfs, graphHeight);
        const textWidth = ctx.measureText(label).width;
        const boxTop =
          baseline === "top" ? y : baseline === "bottom" ? y - EQ_AXIS_LINE_PX : y - EQ_AXIS_LINE_PX / 2;
        ctx.fillStyle = patch;
        ctx.fillRect(rightX - textWidth - 1, boxTop, textWidth + 2, EQ_AXIS_LINE_PX);
        ctx.fillStyle = themeColors().eq.labelText.css;
        ctx.textBaseline = baseline;
        ctx.fillText(label, rightX, y);
      }
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
    }

    // Total response curve, filled to 0 dB (SPEC-015 §2.6.3 "Total response").
    const c = curve;
    if (c && c.freqs_hz.length > 0) {
      const zeroY = yForDb(0, graphHeight, gainRangeDb);
      // A point with a non-finite coordinate (a param/curve value briefly mid-update) is
      // dropped rather than handed to `moveTo`/`lineTo` — one bad sample must not blank the
      // whole polyline (H-32).
      const finitePoints = totalCurveToScreen(c, width, graphHeight, fLo, fHi, gainRangeDb).filter(
        ({ x, y }) => Number.isFinite(x) && Number.isFinite(y),
      );
      ctx.beginPath();
      finitePoints.forEach(({ x, y }, i) => {
        if (i === 0) {
          ctx.moveTo(x, y);
        } else {
          ctx.lineTo(x, y);
        }
      });
      ctx.strokeStyle = themeColors().eq.curve.css;
      ctx.lineWidth = themeColors().emphasisStrokePx;
      ctx.stroke();
      if (finitePoints.length > 0) {
        ctx.lineTo(width, zeroY);
        ctx.lineTo(0, zeroY);
        ctx.closePath();
        ctx.fillStyle = themeColors().eq.fill.css;
        ctx.fill();
      }
    }

    // Nodes (SPEC-015 §2.6.3 "Nodes").
    for (const node of nodes) {
      const x = xForFreq(node.freqHz, width, fLo, fHi);
      const y = yForDb(nodeGainDb(node, curve) ?? 0, graphHeight, gainRangeDb);
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        continue; // same rationale as the curve above — skip, don't abort the whole graph
      }
      const color = eqBandColor(themeColors(), String(node.bandKey || node.component)).css;
      ctx.beginPath();
      ctx.arc(x, y, 6, 0, Math.PI * 2);
      if (node.enabled) {
        ctx.fillStyle = color;
        ctx.fill();
      } else {
        ctx.globalAlpha = 0.5;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        ctx.stroke();
        ctx.globalAlpha = 1;
      }
      if (selected === node.component) {
        ctx.strokeStyle = themeColors().eq.selectedRing.css;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(x, y, 9, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
  }

  interface Dragging {
    component: number;
    startClientX: number;
    startClientY: number;
    start: DragStart;
  }

  let dragging: Dragging | null = null;

  function pointerPos(event: PointerEvent | MouseEvent | WheelEvent): { x: number; y: number } {
    const rect = canvasEl!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function nodeAt(x: number, y: number): EqNode | null {
    return hitTestNode(nodes, x, y, width, graphHeight, fLo, fHi, gainRangeDb, curve);
  }

  function onPointerDown(event: PointerEvent): void {
    if (!canvasEl) {
      return;
    }
    const { x, y } = pointerPos(event);
    const node = nodeAt(x, y);
    if (!node) {
      selected = null;
      return;
    }
    selected = node.component;
    if (event.altKey) {
      // Alt+click toggles the band on/off (SPEC-015 §2.6.4); it never starts a drag. Chosen over
      // inventing a new gesture because it's the mouse toggle the spec itself names, and it was
      // only ever left unimplemented (S3-07 used double-click for this instead — H-86 corrects
      // that, see the on/off note above).
      if (node.enableId !== null) {
        void setParamPlain(slotIndex, node.enableId, node.enabled ? 0 : 1);
      }
      return;
    }
    dragging = {
      component: node.component,
      startClientX: event.clientX,
      startClientY: event.clientY,
      start: {
        freqPx: xForFreq(node.freqHz, width, fLo, fHi),
        gainPx:
          node.gainId === null ? null : yForDb(node.gainDb ?? 0, graphHeight, gainRangeDb),
      },
    };
    canvasEl.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragging) {
      return;
    }
    const node = nodes.find((n) => n.component === dragging!.component);
    if (!node) {
      return;
    }
    const deltaX = event.clientX - dragging.startClientX;
    const deltaY = event.clientY - dragging.startClientY;
    const pos = dragPosition(dragging.start, deltaX, deltaY, event.shiftKey);
    const freq = freqForX(pos.freqPx, width, fLo, fHi);
    setParamPlainDragged(slotIndex, node.freqId, freq);
    if (pos.gainPx !== null && node.gainId !== null) {
      const gain = dbForY(pos.gainPx, graphHeight, gainRangeDb);
      setParamPlainDragged(slotIndex, node.gainId, gain);
    }
  }

  function onPointerUp(event: PointerEvent): void {
    dragging = null;
    canvasEl?.releasePointerCapture?.(event.pointerId);
  }

  function onWheel(event: WheelEvent): void {
    if (!canvasEl) {
      return;
    }
    const { x, y } = pointerPos(event);
    const node = nodeAt(x, y);
    if (!node) {
      return; // empty area: let the rack panel scroll as usual
    }
    if (node.qId !== null) {
      event.preventDefault();
      const factor = wheelQFactor(event.deltaY, event.shiftKey);
      void setParamPlain(slotIndex, node.qId, (node.q ?? 1) * factor);
      return;
    }
    if (node.slopeId !== null) {
      // HP/LP have no Q — the wheel steps slope instead, one notch per notch (SPEC-015 §2.6.4
      // "for HP/LP, the next or previous slope"), reusing H-84's keyboard step math so the wheel
      // and ↑/↓ agree on direction and clamping.
      const notches = wheelNotches(event.deltaY);
      if (notches === 0) {
        return;
      }
      const range = paramRangeOf(rackSlot.params, node.slopeId);
      if (!range) {
        return;
      }
      event.preventDefault();
      const nextIndex = stepSlopeIndex(node.slope ?? 0, notches, range.max);
      void setParamPlain(slotIndex, node.slopeId, nextIndex);
      return;
    }
    // A band with neither Q nor slope: leave the wheel free to scroll the panel.
  }

  function onDblClick(event: MouseEvent): void {
    if (!canvasEl) {
      return;
    }
    const { x, y } = pointerPos(event);
    const node = nodeAt(x, y);
    if (!node) {
      return;
    }
    // SPEC-015 §2.6.4: double-click resets the band's frequency/gain/Q (or slope) to defaults,
    // same as the keyboard's Home (§2.6.5) — its on/off state is unchanged. S3-07 used
    // double-click to toggle on/off instead; H-86 corrects that (Alt+click is the mouse toggle).
    resetNode(node);
  }

  function bandLabel(node: EqNode): string {
    return nodeShortLabel(node);
  }

  // --- Keyboard nodes (H-84, SPEC-015 §2.6.5 / AC-19) -------------------------------------------

  /** Screen position of each node's focus target, using the same axis calls `drawInner` draws
   * with, so the invisible focusable layer always lines up with the drawn circle. */
  const nodePositions = $derived(
    nodes.map((node) => ({
      node,
      x: xForFreq(node.freqHz, width, fLo, fHi),
      y: yForDb(nodeGainDb(node, curve) ?? 0, graphHeight, gainRangeDb),
    })),
  );

  function onNodeFocus(node: EqNode): void {
    focusedComponent = node.component;
    selected = node.component;
  }

  function onNodeBlur(node: EqNode): void {
    if (focusedComponent === node.component) {
      focusedComponent = null;
    }
  }

  /** Every parameter Home resets, unchanged (SPEC-015 §2.6.4 "Its on/off state is unchanged"). */
  function resetNode(node: EqNode): void {
    const freqRange = paramRangeOf(rackSlot.params, node.freqId);
    if (freqRange) {
      void setParamPlain(slotIndex, node.freqId, freqRange.default);
    }
    if (node.gainId !== null) {
      const range = paramRangeOf(rackSlot.params, node.gainId);
      if (range) {
        void setParamPlain(slotIndex, node.gainId, range.default);
      }
    }
    if (node.qId !== null) {
      const range = paramRangeOf(rackSlot.params, node.qId);
      if (range) {
        void setParamPlain(slotIndex, node.qId, range.default);
      }
    }
    if (node.slopeId !== null) {
      const range = paramRangeOf(rackSlot.params, node.slopeId);
      if (range) {
        void setParamPlain(slotIndex, node.slopeId, range.default);
      }
    }
  }

  /** SPEC-015 §2.6.5's key table. `Space` is deliberately not handled here (and never
   * `preventDefault`ed) so it keeps its global play/stop meaning. */
  function onNodeKeydown(event: KeyboardEvent, node: EqNode): void {
    const fine = event.shiftKey;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowRight": {
        const range = paramRangeOf(rackSlot.params, node.freqId);
        if (!range) {
          return;
        }
        event.preventDefault();
        const direction = event.key === "ArrowRight" ? 1 : -1;
        void setParamPlain(slotIndex, node.freqId, keyFreqHz(node.freqHz, direction, fine, range.min, range.max));
        break;
      }
      case "ArrowUp":
      case "ArrowDown": {
        const direction = event.key === "ArrowUp" ? 1 : -1;
        if (node.slopeId !== null) {
          const range = paramRangeOf(rackSlot.params, node.slopeId);
          if (!range) {
            return;
          }
          event.preventDefault();
          const nextIndex = stepSlopeIndex(node.slope ?? 0, direction, range.max);
          void setParamPlain(slotIndex, node.slopeId, nextIndex);
        } else if (node.gainId !== null) {
          const range = paramRangeOf(rackSlot.params, node.gainId);
          if (!range) {
            return;
          }
          event.preventDefault();
          const next = keyGainDb(node.gainDb ?? 0, direction, fine, range.min, range.max);
          void setParamPlain(slotIndex, node.gainId, next);
        } else {
          return;
        }
        break;
      }
      case "PageUp":
      case "PageDown": {
        if (node.qId === null) {
          return;
        }
        const range = paramRangeOf(rackSlot.params, node.qId);
        if (!range) {
          return;
        }
        event.preventDefault();
        const direction = event.key === "PageUp" ? 1 : -1;
        void setParamPlain(slotIndex, node.qId, keyQValue(node.q ?? 1, direction, range.min, range.max));
        break;
      }
      case "Enter": {
        if (node.enableId === null) {
          return;
        }
        event.preventDefault();
        void setParamPlain(slotIndex, node.enableId, node.enabled ? 0 : 1);
        break;
      }
      case "Home": {
        event.preventDefault();
        resetNode(node);
        break;
      }
      default:
        return; // includes Space: left alone for the global transport shortcut (T-104)
    }
    // Not announced here: `node` is this event's *pre*-change snapshot (the command is async),
    // so announcing it now would read the old value. The effect below announces once Rust's own
    // echoed text actually lands in `rackSlot.values` — always the true, settled value.
  }

  // The focused node's value text, announced through the throttled live region whenever it
  // actually changes: on focus (SPEC-015 §2.6.5), and again once a keyboard/drag/other client's
  // change is echoed back into `rackSlot.values` and `nodeValueText` reflects it — never the
  // stale pre-echo estimate a handler could compute locally.
  $effect(() => {
    const node = nodes.find((n) => n.component === focusedComponent);
    if (node) {
      announcer.announce(nodeValueText(node, rackSlot));
    }
  });

  function openExpanded(): void {
    openEqExpanded(rackSlot.uid);
  }
</script>

<div class="eq-graph" data-testid="eq-graph">
  <div class="toolbar">
    <span class="axis-title" data-testid="eq-gain-unit">{t("eq.graph.gain_unit")}</span>
    <span class="spacer"></span>
    <IconButton
      icon="analyzer"
      size="sm"
      pressed={spectrumOn}
      label={t("eq.graph.spectrum_toggle")}
      testid="eq-spectrum-toggle"
      onclick={() => (spectrumOn = !spectrumOn)}
    />
    {#if mode === "compact"}
      <IconButton
        icon="expand"
        size="sm"
        label={t("eq.graph.expand")}
        testid="eq-expand-button"
        onclick={openExpanded}
      />
    {/if}
    <button
      type="button"
      class="range-toggle"
      data-testid="eq-range-toggle"
      title={t("eq.graph.range_toggle")}
      onclick={() =>
        (gainRangeDb =
          gainRangeDb === EQ_GAIN_RANGE_DEFAULT_DB
            ? EQ_GAIN_RANGE_WIDE_DB
            : EQ_GAIN_RANGE_DEFAULT_DB)}
    >
      {gainRangeDb === EQ_GAIN_RANGE_WIDE_DB ? t("eq.graph.range_24") : t("eq.graph.range_12")}
    </button>
  </div>
  <div class="graph-wrap">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <canvas
      bind:this={canvasEl}
      class="graph"
      style={`height: ${graphHeight}px`}
      data-testid="eq-canvas"
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      onwheel={onWheel}
      ondblclick={onDblClick}
    ></canvas>
    <!-- H-84 (SPEC-015 §2.6.5, AC-19): one focusable target per node, positioned exactly where
         the canvas draws its circle. `pointer-events: none` keeps every mouse gesture on the
         canvas above untouched — this layer exists for Tab/keyboard only. -->
    <div class="node-layer" data-testid="eq-node-layer">
      {#each nodePositions as { node, x, y } (node.component)}
        <div
          class="node-target"
          style:left="{x}px"
          style:top="{y}px"
          role="slider"
          tabindex="0"
          aria-roledescription={t("eq.graph.node_roledescription")}
          aria-label={nodeFullName(node, rackSlot.groups)}
          aria-orientation="horizontal"
          aria-valuemin={fLo}
          aria-valuemax={fHi}
          aria-valuenow={node.freqHz}
          aria-valuetext={nodeValueText(node, rackSlot)}
          data-testid={`eq-node-${node.component}`}
          onfocus={() => onNodeFocus(node)}
          onblur={() => onNodeBlur(node)}
          onkeydown={(e) => onNodeKeydown(e, node)}
        ></div>
      {/each}
    </div>
  </div>
  <div class="band-labels" data-testid="eq-band-labels">
    {#each nodes as node (node.component)}
      <span class="band-label" class:selected={selected === node.component}>
        {bandLabel(node)}
      </span>
    {/each}
  </div>
  <!-- SPEC-015 §2.6.5: value changes announced at most every 250 ms (`liveRegion.ts`). -->
  <div class="sr-only" role="status" aria-live="polite" data-testid="eq-live-region">{liveText}</div>
</div>

<style>
  .eq-graph {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
  }

  /* H-26: the gain unit is the axis title, above the gain labels (canvas border + label inset).
   * H-84: the Spectrum/Expand buttons and the Range toggle sit to its right, pushed there by
   * `.spacer` rather than `justify-content: space-between` (which would spread space between
   * every pair of children once there is more than one on the right). */
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .spacer {
    flex: 1;
  }

  .axis-title {
    padding-left: calc(var(--pv-border-width) + 3px);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
  }

  .range-toggle {
    height: 20px;
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-secondary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    cursor: default;
  }

  .range-toggle:hover {
    color: var(--pv-text-primary);
  }

  .graph-wrap {
    position: relative;
  }

  .graph {
    display: block;
    width: 100%;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
    touch-action: none;
    cursor: crosshair;
  }

  /* H-84 (SPEC-015 §2.6.5): keyboard-only focus targets, laid exactly over the canvas's own
   * circles. `pointer-events: none` on the layer keeps every mouse gesture on the canvas below;
   * Tab/focus works regardless (it isn't a pointer interaction). */
  .node-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .node-target {
    position: absolute;
    width: 20px;
    height: 20px;
    border-radius: var(--pv-radius-full);
    transform: translate(-50%, -50%);
    outline: none;
  }

  .node-target:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 1px;
  }

  /* SPEC-015 §2.6.5: an off-screen, always-present live region (never `[hidden]`, which an AT
   * would stop announcing from). */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .band-labels {
    display: flex;
    flex-wrap: wrap;
    gap: var(--pv-space-1) var(--pv-space-2);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
  }

  .band-label.selected {
    color: var(--pv-accent-text);
    font-weight: var(--pv-weight-semibold);
  }
</style>
