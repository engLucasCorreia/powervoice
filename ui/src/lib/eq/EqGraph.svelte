<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import type { RackSlotDto, ResponseCurveDto } from "../ipc/bindings";
  import { rackResponseCurve } from "../ipc/commands";
  import { setParamPlain, setParamPlainDragged } from "../rack/rack.svelte";
  import { totalCurveToScreen } from "./curvePoints";
  import { CoalescedCurveRequest } from "./curveRequest";
  import { wheelQFactor, dragPosition, type DragStart } from "./drag";
  import { curveRequestFreqs, eqFrequencyTicks, freqForX, graphMaxHz, EQ_MIN_HZ, xForFreq } from "./freqAxis";
  import {
    dbForY,
    gainAxisTicks,
    yForDb,
    EQ_GAIN_RANGE_DEFAULT_DB,
    EQ_GAIN_RANGE_WIDE_DB,
  } from "./gainAxis";
  import { buildEqNodes, hitTestNode, nodeGainDb, nodeFreqsHz, type EqNode } from "./nodes";
  import { formatRulerFreqHz } from "../spectrum/freqAxis";

  /**
   * The EQ graph panel (S3-07, SPEC-015 §2.6, lean slice): log-frequency axis, ±12/±24 dB gain
   * axis, the total response curve drawn only from the module's `ResponseCurve` (via
   * `rack_response_curve`, never evaluated in the UI — AC-17), and draggable nodes per band.
   * Rendered in the slot body above the generic parameter panel (which stays available below)
   * for any module exposing `curve_handles` (currently only the Parametric EQ).
   *
   * Out of this lean slice (SPEC-015 §2.6.3–§2.6.5, ticket S3-07 "Out"): the live spectrum
   * overlay (needs the analyzer, T-208), the expanded floating view, the right-click menu,
   * keyboard node navigation, and hover tooltips sourced from Rust's `param_changed` text.
   * Double-click toggles the band (ticket S3-07), not the fuller spec's "reset to defaults" —
   * see the ticket vs. SPEC-015 §2.6.4 note in the S3-07 report.
   */
  let { slotIndex, rackSlot, rateHz }: { slotIndex: number; rackSlot: RackSlotDto; rateHz: number } =
    $props();

  const GRAPH_HEIGHT_PX = 160; // SPEC-015 §2.6 "graph_height_compact"

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let curve = $state<ResponseCurveDto | null>(null);
  let gainRangeDb = $state(EQ_GAIN_RANGE_DEFAULT_DB);
  let selected = $state<number | null>(null);

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

  $effect(() => {
    draw();
  });

  function colorToken(name: string, fallback: string): string {
    if (!canvasEl) {
      return fallback;
    }
    const value = getComputedStyle(canvasEl).getPropertyValue(name).trim();
    return value || fallback;
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
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, GRAPH_HEIGHT_PX);

    // Grid: 0 dB emphasised, every 3 dB (±12) or 6 dB (±24) otherwise (SPEC-015 §2.6.2).
    const gainTicks = gainAxisTicks(GRAPH_HEIGHT_PX, gainRangeDb);
    ctx.strokeStyle = colorToken("--eq-grid", "#34373d");
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

    // H-24 item 8: Hz/kHz labels at the standard decades and dB labels at the grid lines, unit
    // shown once each (SPEC-007 §2.4's "unit appears once" convention, reused here).
    const textColor = colorToken("--text-secondary", "#9a9da4");
    ctx.fillStyle = textColor;
    ctx.font = "10px sans-serif";
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    for (const tick of gainTicks) {
      ctx.fillText(tick.label, 2, Math.round(tick.y) + (tick.db === 0 ? -6 : 0));
    }
    ctx.fillText(t("eq.graph.gain_unit"), 2, 8);

    ctx.textBaseline = "bottom";
    for (const tick of eqFrequencyTicks(fLo, fHi, width, 30, formatRulerFreqHz)) {
      ctx.textAlign = tick.x < 12 ? "left" : tick.x > width - 12 ? "right" : "center";
      ctx.fillText(tick.label, tick.x, GRAPH_HEIGHT_PX - 2);
    }
    ctx.textAlign = "right";
    ctx.fillText(t("eq.graph.freq_unit"), width - 2, GRAPH_HEIGHT_PX - 2);
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";

    // Total response curve, filled to 0 dB (SPEC-015 §2.6.3 "Total response").
    const c = curve;
    if (c && c.freqs_hz.length > 0) {
      const zeroY = yForDb(0, GRAPH_HEIGHT_PX, gainRangeDb);
      const points = totalCurveToScreen(c, width, GRAPH_HEIGHT_PX, fLo, fHi, gainRangeDb);
      ctx.beginPath();
      points.forEach(({ x, y }, i) => {
        if (i === 0) {
          ctx.moveTo(x, y);
        } else {
          ctx.lineTo(x, y);
        }
      });
      ctx.strokeStyle = colorToken("--eq-curve", "#e6e7ea");
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.lineTo(width, zeroY);
      ctx.lineTo(0, zeroY);
      ctx.closePath();
      ctx.fillStyle = colorToken("--eq-fill", "rgba(230, 231, 234, 0.15)");
      ctx.fill();
    }

    // Nodes (SPEC-015 §2.6.3 "Nodes").
    for (const node of nodes) {
      const x = xForFreq(node.freqHz, width, fLo, fHi);
      const y = yForDb(nodeGainDb(node, curve) ?? 0, GRAPH_HEIGHT_PX, gainRangeDb);
      const color = colorToken(`--eq-band-${node.bandKey || node.component}`, "#7fc8ff");
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
        ctx.strokeStyle = colorToken("--focus-ring", "#7fc8ff");
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(x, y, 9, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
    ctx.restore();
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
    return hitTestNode(nodes, x, y, width, GRAPH_HEIGHT_PX, fLo, fHi, gainRangeDb, curve);
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
    dragging = {
      component: node.component,
      startClientX: event.clientX,
      startClientY: event.clientY,
      start: {
        freqPx: xForFreq(node.freqHz, width, fLo, fHi),
        gainPx:
          node.gainId === null ? null : yForDb(node.gainDb ?? 0, GRAPH_HEIGHT_PX, gainRangeDb),
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
      const gain = dbForY(pos.gainPx, GRAPH_HEIGHT_PX, gainRangeDb);
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
    if (!node || node.qId === null) {
      return; // empty area, or a band with no Q (HP/LP): let the rack panel scroll as usual
    }
    event.preventDefault();
    const factor = wheelQFactor(event.deltaY, event.shiftKey);
    void setParamPlain(slotIndex, node.qId, (node.q ?? 1) * factor);
  }

  function onDblClick(event: MouseEvent): void {
    if (!canvasEl) {
      return;
    }
    const { x, y } = pointerPos(event);
    const node = nodeAt(x, y);
    if (!node || node.enableId === null) {
      return;
    }
    void setParamPlain(slotIndex, node.enableId, node.enabled ? 0 : 1);
  }

  function bandLabel(node: EqNode): string {
    return node.bandKey ? tDynamic(`eq.band.${node.bandKey}`) : String(node.component);
  }
</script>

<div class="eq-graph" data-testid="eq-graph">
  <div class="toolbar">
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
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <canvas
    bind:this={canvasEl}
    class="graph"
    style={`height: ${GRAPH_HEIGHT_PX}px`}
    data-testid="eq-canvas"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onwheel={onWheel}
    ondblclick={onDblClick}
  ></canvas>
  <div class="band-labels" data-testid="eq-band-labels">
    {#each nodes as node (node.component)}
      <span class="band-label" class:selected={selected === node.component}>
        {bandLabel(node)}
      </span>
    {/each}
  </div>
</div>

<style>
  .eq-graph {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
  }

  .toolbar {
    display: flex;
    justify-content: flex-end;
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

  .graph {
    display: block;
    width: 100%;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
    touch-action: none;
    cursor: crosshair;
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
