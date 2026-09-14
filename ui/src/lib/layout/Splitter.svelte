<script lang="ts">
  /**
   * A draggable, keyboard-accessible splitter (H-24 items 1/2): resizes a column (vertical
   * divider, drags horizontally) or the bottom dock (horizontal divider, drags vertically).
   * `role="separator"` + arrow keys per the ticket ("keyboard-accessible splitters"). Pure
   * presentation/interaction — the caller (`App.svelte`) owns the actual size, using
   * `splitterMath.ts` to clamp/step it, so this component has no feature-specific state.
   *
   * An optional collapse toggle (a small chevron button riding on the splitter) covers the
   * ticket's "collapse buttons for the side panels" without a second, separately-positioned
   * button element.
   */
  let {
    orientation,
    ariaLabel,
    testid,
    collapsed = false,
    collapsible = false,
    onDrag,
    onDragEnd,
    onReset,
    onStep,
    onToggleCollapse,
  }: {
    orientation: "vertical" | "horizontal";
    ariaLabel: string;
    testid: string;
    collapsed?: boolean;
    collapsible?: boolean;
    onDrag: (deltaPx: number) => void;
    onDragEnd?: () => void;
    onReset: () => void;
    onStep: (direction: 1 | -1) => void;
    onToggleCollapse?: () => void;
  } = $props();

  let dragging = false;
  let dragStartClientPx = 0;

  function clientPx(event: PointerEvent): number {
    return orientation === "vertical" ? event.clientX : event.clientY;
  }

  function onPointerDown(event: PointerEvent): void {
    dragging = true;
    dragStartClientPx = clientPx(event);
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event: PointerEvent): void {
    if (!dragging) {
      return;
    }
    const now = clientPx(event);
    onDrag(now - dragStartClientPx);
    dragStartClientPx = now;
  }

  function onPointerUp(event: PointerEvent): void {
    if (!dragging) {
      return;
    }
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
    onDragEnd?.();
  }

  function onKeydown(event: KeyboardEvent): void {
    const growKey = orientation === "vertical" ? "ArrowRight" : "ArrowDown";
    const shrinkKey = orientation === "vertical" ? "ArrowLeft" : "ArrowUp";
    if (event.key === growKey) {
      event.preventDefault();
      onStep(1);
    } else if (event.key === shrinkKey) {
      event.preventDefault();
      onStep(-1);
    } else if (event.key === "Enter" || event.key === " ") {
      if (collapsible && onToggleCollapse) {
        event.preventDefault();
        onToggleCollapse();
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="splitter {orientation}"
  class:collapsed
  data-testid={testid}
  role="separator"
  aria-orientation={orientation === "vertical" ? "vertical" : "horizontal"}
  aria-label={ariaLabel}
  tabindex="0"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  ondblclick={onReset}
  onkeydown={onKeydown}
>
  {#if collapsible}
    <button
      type="button"
      class="collapse-toggle"
      data-testid={`${testid}-collapse`}
      aria-label={ariaLabel}
      tabindex="-1"
      onclick={(e) => {
        e.stopPropagation();
        onToggleCollapse?.();
      }}
    >
      {#if orientation === "vertical"}
        {collapsed ? "›" : "‹"}
      {:else}
        {collapsed ? "▴" : "▾"}
      {/if}
    </button>
  {/if}
</div>

<style>
  .splitter {
    flex: none;
    position: relative;
    background: var(--surface-border);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .splitter.vertical {
    width: 6px;
    cursor: col-resize;
  }

  .splitter.horizontal {
    height: 6px;
    cursor: row-resize;
  }

  .splitter:hover,
  .splitter:focus-visible {
    background: var(--accent);
  }

  .collapse-toggle {
    position: absolute;
    width: 14px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-panel-raised);
    color: var(--text-secondary);
    border: 1px solid var(--surface-border);
    border-radius: 3px;
    font-size: 0.7rem;
    line-height: 1;
    pointer-events: auto;
  }

  .splitter.horizontal .collapse-toggle {
    width: 28px;
    height: 14px;
  }
</style>
