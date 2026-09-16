<script lang="ts">
  import { t } from "../i18n";
  import type { RackSlotDto, TransferCurveDto, TransferCurveHandleDto } from "../ipc/bindings";
  import { rackTransferCurve } from "../ipc/commands";
  import { CoalescedCurveRequest } from "../eq/curveRequest";
  import { setParamPlain, setParamPlainDragged } from "../rack/rack.svelte";
  import { createFrameClient } from "../render/frameScheduler";
  import { themeColors } from "../theme/themeColors";
  import { themeState } from "../theme/theme.svelte";
  import { branchToScreen, differingRanges, sliceScreen, type ScreenPoint } from "./curvePoints";
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
   * The transfer graph (H-63, SPEC-016 §2.6 / §4.11, SPEC-013 §2.7): a square input→output level
   * plot drawn only from the module's `TransferCurve` (via `rack_transfer_curve`, never evaluated
   * in the UI), with one draggable threshold handle per enabled section. Rendered in the slot body
   * above the generic parameter panel for any module exposing `transfer_handles` — today Dynamics
   * and the Noise Gate, tomorrow any module that answers the extension.
   *
   * Not here (T-410): the custom Dynamics panel around it, per-component curves, and the live
   * operating-point dot (SPEC-016 §2.6 item 2 — it needs the section makeup, which no telemetry
   * channel carries).
   */
  let { slotIndex, rackSlot }: { slotIndex: number; rackSlot: RackSlotDto } = $props();

  /** Hit radius for a threshold handle, px on the x axis. */
  const HANDLE_HIT_PX = 10;
  /** Half-width of the triangular handle. */
  const HANDLE_HALF_PX = 5;
  const HANDLE_HEIGHT_PX = 8;
  const AXIS_FONT_PX = 10;
  /** Shift gives fine ×0.1 movement (SPEC-016 §2.6). */
  const FINE_FACTOR = 0.1;

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let curve = $state<TransferCurveDto | null>(null);

  const side = $derived(transferSidePx(width));
  const handles = $derived(curve?.handles ?? []);

  const fetcher = new CoalescedCurveRequest<TransferCurveDto, number>(
    (points) => rackTransferCurve(slotIndex, TRANSFER_MIN_DBFS, TRANSFER_MAX_DBFS, points),
    (result) => (curve = result),
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
    void [canvasEl, width, side, curve, dragging, themeState().revision];
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
    if (c && c.in_dbfs.length > 0) {
      const rising = branchToScreen(c, c.rising_db, side, side);
      // The falling branch, dashed and only where it differs (the hysteresis loop).
      if (c.falling_db) {
        const falling = branchToScreen(c, c.falling_db, side, side);
        ctx.save();
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = colors.transfer.curve.css;
        ctx.lineWidth = colors.strokePx;
        for (const range of differingRanges(c.rising_db, c.falling_db)) {
          strokePath(ctx, sliceScreen(falling, range));
        }
        ctx.restore();
      }
      ctx.strokeStyle = colors.transfer.curve.css;
      ctx.lineWidth = colors.emphasisStrokePx;
      strokePath(ctx, rising);
    }

    // Threshold handles: one small triangle on the x axis per enabled section (SPEC-016 §2.6).
    for (const handle of handles) {
      if (!handle.enabled) {
        continue;
      }
      const x = xForLevel(handle.x_dbfs, side);
      if (!Number.isFinite(x)) {
        continue;
      }
      ctx.fillStyle = colors.transfer.handle.css;
      ctx.beginPath();
      ctx.moveTo(x, side);
      ctx.lineTo(x - HANDLE_HALF_PX, side - HANDLE_HEIGHT_PX);
      ctx.lineTo(x + HANDLE_HALF_PX, side - HANDLE_HEIGHT_PX);
      ctx.closePath();
      ctx.fill();
      // The threshold's own text (Rust's formatting, SPEC-016 §2.6), while it is being dragged —
      // four labels at once would not fit a 200–320 px graph.
      const text = dragging?.param === handle.param ? paramText(handle.param) : null;
      if (text) {
        label(text, Math.min(Math.max(x, 2), side - 2), inputRowY - AXIS_FONT_PX - 2, "center");
      }
    }
  }

  interface Dragging {
    param: number;
    offsetDb: number;
    startClientX: number;
    startX: number;
  }

  let dragging = $state<Dragging | null>(null);

  function handleAt(x: number): TransferCurveHandleDto | null {
    let best: TransferCurveHandleDto | null = null;
    let bestDistance = HANDLE_HIT_PX;
    for (const handle of handles) {
      if (!handle.enabled) {
        continue;
      }
      const distance = Math.abs(xForLevel(handle.x_dbfs, side) - x);
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
      offsetDb: handle.offset_db,
      startClientX: event.clientX,
      startX: xForLevel(handle.x_dbfs, side),
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
    {#if curve?.falling_db}
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
