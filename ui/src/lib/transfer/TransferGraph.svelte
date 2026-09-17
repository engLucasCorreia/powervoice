<script lang="ts">
  import { t } from "../i18n";
  import type { RackSlotDto } from "../ipc/bindings";
  import { moduleTransferCurve } from "../ipc/commands";
  import { decodeVxtc, type TransferCurveFrame, type TransferCurveHandle } from "../ipc/transferCurve";
  import { CoalescedCurveRequest } from "../eq/curveRequest";
  import { setParamPlain, setParamPlainDragged } from "../rack/rack.svelte";
  import { createFrameClient } from "../render/frameScheduler";
  import { themeColors } from "../theme/themeColors";
  import { themeState } from "../theme/theme.svelte";
  import type { OperatingPoint } from "./operatingPoint";
  import {
    branchToScreen,
    componentIsActive,
    componentToScreen,
    differingRanges,
    sliceScreen,
    type ScreenPoint,
  } from "./curvePoints";
  import {
    TRANSFER_MAX_DBFS,
    TRANSFER_MIN_DBFS,
    levelForX,
    transferAxisTicks,
    transferCurvePointCount,
    transferSidePx,
    xForLevel,
    yForLevel,
  } from "./levelAxis";

  /**
   * The transfer graph (H-63, H-77; SPEC-016 §2.6 / §4.11 / §4.12, SPEC-013 §2.7): a square
   * input→output level plot drawn only from the module's `TransferCurve` (the binary `VXTC` frame
   * of `module_transfer_curve`, never evaluated in the UI), with one draggable, labelled threshold
   * handle per enabled section, a faint overlay per active section, and the live operating-point
   * dot. Rendered in the slot body above the generic parameter panel for any module exposing
   * `transfer_handles` — today Dynamics and the Noise Gate, tomorrow any module that answers the
   * extension.
   */
  let {
    slotIndex,
    rackSlot,
    operatingPoint = null,
  }: {
    slotIndex: number;
    rackSlot: RackSlotDto;
    /**
     * H-77 (SPEC-016 §2.6): the live operating point, or `null` while it is hidden (no telemetry
     * frame for 250 ms, or the module has no such channels). Panel-local, because the effective
     * makeup is a parameter, not a telemetry channel — see `DynamicsPanel.svelte`.
     */
    operatingPoint?: OperatingPoint | null;
  } = $props();

  /** Hit radius for a threshold handle, px on the x axis. */
  const HANDLE_HIT_PX = 10;
  /** Half-width of the triangular handle. */
  const HANDLE_HALF_PX = 5;
  const HANDLE_HEIGHT_PX = 8;
  const AXIS_FONT_PX = 10;
  /** Shift gives fine ×0.1 movement (SPEC-016 §2.6). */
  const FINE_FACTOR = 0.1;

  /** Dot radius, px. */
  const DOT_RADIUS_PX = 3.5;

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let curve = $state<TransferCurveFrame | null>(null);

  const side = $derived(transferSidePx(width));
  const handles = $derived(curve?.handles ?? []);
  /** `VXTC` handles come in `handles()` order, the same order as the slot's `transfer_handles`,
   * which is where the component index of each one lives. */
  const handleComponents = $derived((rackSlot.transfer_handles ?? []).map((h) => h.component));

  const fetcher = new CoalescedCurveRequest<TransferCurveFrame | null, number>(
    async (points, seq) =>
      decodeVxtc(
        await moduleTransferCurve(slotIndex, seq, TRANSFER_MIN_DBFS, TRANSFER_MAX_DBFS, points),
      ),
    (result) => {
      if (result) {
        curve = result;
      }
    },
  );

  // Re-request whenever a parameter value or the graph width changes, coalesced to one request
  // per animation frame; a response older than the newest request is dropped by `fetcher`.
  $effect(() => {
    void rackSlot.values;
    if (width <= 0) {
      return;
    }
    fetcher.request(transferCurvePointCount(side));
  });
  $effect(() => () => fetcher.cancel());

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

  // H-43: draw on demand from the shared frame scheduler — the whole dependency list is read
  // here, separately from the draw, so an arriving curve always wakes the renderer even if a
  // previous draw bailed out early or threw.
  const frames = createFrameClient(() => draw(), { name: "transfer-graph" });
  $effect(() => {
    void [canvasEl, width, side, curve, dragging, operatingPoint, themeState().revision];
    frames.invalidate();
  });
  $effect(() => () => frames.dispose());

  /** The UI font for canvas labels (a font stack, not a colour). */
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
    const backing = Math.max(1, Math.round(side * dpr));
    if (canvasEl.width !== backing || canvasEl.height !== backing) {
      canvasEl.width = backing;
      canvasEl.height = backing;
    }
    ctx.save();
    try {
      drawInner(ctx, dpr);
    } finally {
      // Always balances the save, even after a throw: an unbalanced context stack would distort
      // every later frame. The scheduler logs and retries the throw (H-43).
      ctx.restore();
    }
  }

  /** Strokes a polyline, starting a new sub-path at every gap (a muted level). */
  function strokePath(ctx: CanvasRenderingContext2D, points: (ScreenPoint | null)[]): void {
    ctx.beginPath();
    let pen = false;
    for (const point of points) {
      if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)) {
        pen = false;
        continue;
      }
      if (pen) {
        ctx.lineTo(point.x, point.y);
      } else {
        ctx.moveTo(point.x, point.y);
        pen = true;
      }
    }
    ctx.stroke();
  }

  function drawInner(ctx: CanvasRenderingContext2D, dpr: number): void {
    const colors = themeColors();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, side, side);

    // Grid every 6 dB, emphasised every 12 (SPEC-016 §2.6).
    const ticks = transferAxisTicks();
    ctx.strokeStyle = colors.transfer.grid.css;
    ctx.lineWidth = 1;
    for (const tick of ticks) {
      ctx.globalAlpha = tick.major ? 0.9 : 0.35;
      const x = Math.round(xForLevel(tick.db, side)) + 0.5;
      const y = Math.round(yForLevel(tick.db, side)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, side);
      ctx.moveTo(0, y);
      ctx.lineTo(side, y);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // The dashed 1:1 diagonal: the two axes share one scale, so it is the square's diagonal.
    ctx.save();
    ctx.setLineDash([3, 3]);
    ctx.strokeStyle = colors.transfer.unity.css;
    ctx.beginPath();
    ctx.moveTo(0, side);
    ctx.lineTo(side, 0);
    ctx.stroke();
    ctx.restore();

    // Axis labels: input along the bottom, output up the left, each on a patch of the graph
    // background so no grid line runs through the text.
    ctx.font = `${AXIS_FONT_PX}px ${fontFamily()}`;
    const patch = colors.transfer.labelPatch.css;
    const label = (text: string, x: number, y: number, align: CanvasTextAlign): void => {
      const w = ctx.measureText(text).width;
      const left = align === "right" ? x - w : align === "center" ? x - w / 2 : x;
      ctx.fillStyle = patch;
      ctx.fillRect(left - 1, y - AXIS_FONT_PX, w + 2, AXIS_FONT_PX + 2);
      ctx.fillStyle = colors.transfer.labelText.css;
      ctx.textAlign = align;
      ctx.textBaseline = "bottom";
      ctx.fillText(text, x, y);
    };
    // The input row sits above the handle band, so a triangle never lands on a label.
    const inputRowY = side - HANDLE_HEIGHT_PX - 2;
    for (const tick of ticks) {
      if (!tick.major || tick.db === TRANSFER_MIN_DBFS) {
        continue;
      }
      label(tick.label, xForLevel(tick.db, side), inputRowY, "center");
      const y = yForLevel(tick.db, side) + AXIS_FONT_PX / 2;
      // The bottom-left corner belongs to the input row: an output label that would collide
      // with it is dropped (the same rule the EQ graph's `axisLayout` applies).
      if (y < inputRowY - AXIS_FONT_PX) {
        label(tick.label, 3, y, "left");
      }
    }
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";

    const c = curve;
    if (c && c.inDbfs.length > 0) {
      // H-77: each active section's own contribution, faint and behind the total curve, so it is
      // clear which section bends which part of it (SPEC-016 §2.6).
      ctx.save();
      ctx.lineWidth = colors.strokePx;
      ctx.globalAlpha = 0.55;
      for (const [index, gains] of c.components.entries()) {
        if (!componentIsActive(gains) || !componentEnabled(index)) {
          continue;
        }
        ctx.strokeStyle = componentColor(colors, index);
        strokePath(ctx, componentToScreen(c, gains, side, side));
      }
      ctx.restore();

      const rising = branchToScreen(c, c.rising, side, side);
      // The falling branch, dashed and only where it differs (the hysteresis loop).
      if (c.falling) {
        const falling = branchToScreen(c, c.falling, side, side);
        ctx.save();
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = colors.transfer.curve.css;
        ctx.lineWidth = colors.strokePx;
        for (const range of differingRanges(c.rising, c.falling)) {
          strokePath(ctx, sliceScreen(falling, range));
        }
        ctx.restore();
      }
      ctx.strokeStyle = colors.transfer.curve.css;
      ctx.lineWidth = colors.emphasisStrokePx;
      strokePath(ctx, rising);
    }

    // Threshold handles: one small triangle on the x axis per enabled section (SPEC-016 §2.6).
    for (const [index, handle] of handles.entries()) {
      if (!handle.enabled) {
        continue;
      }
      const x = xForLevel(handle.xDbfs, side);
      if (!Number.isFinite(x)) {
        continue;
      }
      ctx.fillStyle = componentColor(colors, handleComponents[index] ?? index);
      ctx.beginPath();
      ctx.moveTo(x, side);
      ctx.lineTo(x - HANDLE_HALF_PX, side - HANDLE_HEIGHT_PX);
      ctx.lineTo(x + HANDLE_HALF_PX, side - HANDLE_HEIGHT_PX);
      ctx.closePath();
      ctx.fill();
    }

    // H-77: the thresholds' own text (Rust's formatting, SPEC-016 §2.6) stays on screen — but
    // four labels rarely fit a 200–320 px graph, so one that would collide with an already
    // placed label is dropped. The dragged handle is placed first, so it is never the one that
    // gives way.
    const labelY = inputRowY - AXIS_FONT_PX - 2;
    const placed: Array<[number, number]> = [];
    const order = handles
      .map((handle, index) => ({ handle, index }))
      .sort((a, b) => Number(dragging?.param === b.handle.param) - Number(dragging?.param === a.handle.param));
    for (const { handle } of order) {
      if (!handle.enabled) {
        continue;
      }
      const text = paramText(handle.param);
      const x = xForLevel(handle.xDbfs, side);
      if (!text || !Number.isFinite(x)) {
        continue;
      }
      const half = ctx.measureText(text).width / 2 + 3;
      const centre = Math.min(Math.max(x, half), side - half);
      const box: [number, number] = [centre - half, centre + half];
      if (placed.some(([start, end]) => box[0] < end && start < box[1])) {
        continue;
      }
      placed.push(box);
      label(text, centre, labelY, "center");
    }

    // The operating point: input level against what comes out of the module right now
    // (SPEC-016 §2.6 item 2). Drawn last, so it sits on top of the curve it rides.
    const point = operatingPoint;
    if (point && point.inputDbfs > TRANSFER_MIN_DBFS) {
      const x = xForLevel(point.inputDbfs, side);
      const y = yForLevel(point.inputDbfs + point.grTotalDb + point.makeupDb, side);
      ctx.beginPath();
      ctx.arc(x, y, DOT_RADIUS_PX, 0, Math.PI * 2);
      ctx.fillStyle = colors.transfer.handle.css;
      ctx.fill();
      ctx.lineWidth = colors.strokePx;
      ctx.strokeStyle = colors.transfer.labelPatch.css;
      ctx.stroke();
    }
  }

  /** The colour of one component (section): the transfer palette, wrapping for a module with
   * more sections than colours. */
  function componentColor(colors: ReturnType<typeof themeColors>, component: number): string {
    const palette = colors.transfer.components;
    return palette.length > 0
      ? palette[component % palette.length]!.css
      : colors.transfer.curve.css;
  }

  /** A component is drawn while its own section is enabled (its handle carries the state). */
  function componentEnabled(component: number): boolean {
    const index = handleComponents.indexOf(component);
    return index < 0 || (handles[index]?.enabled ?? true);
  }

  interface Dragging {
    param: number;
    offsetDb: number;
    startClientX: number;
    startX: number;
  }

  let dragging = $state<Dragging | null>(null);

  function handleAt(x: number): TransferCurveHandle | null {
    let best: TransferCurveHandle | null = null;
    let bestDistance = HANDLE_HIT_PX;
    for (const handle of handles) {
      if (!handle.enabled) {
        continue;
      }
      const distance = Math.abs(xForLevel(handle.xDbfs, side) - x);
      if (distance <= bestDistance) {
        best = handle;
        bestDistance = distance;
      }
    }
    return best;
  }

  function pointerX(event: PointerEvent | MouseEvent): number {
    return event.clientX - canvasEl!.getBoundingClientRect().left;
  }

  /** Rust's own display text for a parameter value (SPEC-012 §2.6). */
  function paramText(id: number): string | null {
    return rackSlot.values.find((v) => v.id === id)?.text ?? null;
  }

  function paramDefault(id: number): number | null {
    return rackSlot.params.find((p) => p.id === id)?.default ?? null;
  }

  function onPointerDown(event: PointerEvent): void {
    if (!canvasEl) {
      return;
    }
    const handle = handleAt(pointerX(event));
    if (!handle) {
      return;
    }
    dragging = {
      param: handle.param,
      offsetDb: handle.offsetDb,
      startClientX: event.clientX,
      startX: xForLevel(handle.xDbfs, side),
    };
    canvasEl.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragging) {
      return;
    }
    const delta = (event.clientX - dragging.startClientX) * (event.shiftKey ? FINE_FACTOR : 1);
    // The handle sits at `threshold + offset`, so the threshold is the axis value minus it.
    const value = levelForX(dragging.startX + delta, side) - dragging.offsetDb;
    setParamPlainDragged(slotIndex, dragging.param, value);
  }

  function onPointerUp(event: PointerEvent): void {
    dragging = null;
    canvasEl?.releasePointerCapture?.(event.pointerId);
  }

  function onDblClick(event: MouseEvent): void {
    if (!canvasEl) {
      return;
    }
    const handle = handleAt(pointerX(event));
    const fallback = handle ? paramDefault(handle.param) : null;
    if (handle && fallback !== null) {
      void setParamPlain(slotIndex, handle.param, fallback);
    }
  }
</script>

<div class="transfer-graph" data-testid="transfer-graph">
  <div class="axis-row">
    <span class="axis-title">{t("transfer.graph.out_unit")}</span>
    {#if curve?.falling}
      <span class="axis-note" data-testid="transfer-hysteresis-note"
        >{t("transfer.graph.hysteresis")}</span
      >
    {/if}
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <canvas
    bind:this={canvasEl}
    class="graph"
    style={`height: ${side}px`}
    aria-label={t("transfer.graph.label")}
    data-testid="transfer-canvas"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    ondblclick={onDblClick}
  ></canvas>
  <div class="axis-row end">
    <span class="axis-title">{t("transfer.graph.in_unit")}</span>
  </div>
</div>

<style>
  .transfer-graph {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
  }

  .axis-row {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--pv-space-2);
  }

  .axis-row.end {
    justify-content: flex-end;
  }

  .axis-title,
  .axis-note {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
  }

  .axis-title {
    padding-left: calc(var(--pv-border-width) + 3px);
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
</style>
