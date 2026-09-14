<script lang="ts">
  import Icon from "../ui/Icon.svelte";
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
        <Icon name={collapsed ? "chevronRight" : "chevronLeft"} size={12} />
      {:else}
        <Icon name={collapsed ? "chevronUp" : "chevronDown"} size={12} />
      {/if}
    </button>
  {/if}
</div>

<style>
  /* H-25: a 1 px hairline drawn inside a 6 px hit area; it lights up in the accent while hovered,
     dragged or focused. The collapse tab is a small pill that sits on the line. */
  .splitter {
    flex: none;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--pv-bg-app);
    outline: none;
  }

  .splitter::before {
    content: "";
    position: absolute;
    background: var(--pv-border);
    transition: background-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .splitter.vertical {
    width: 6px;
    cursor: col-resize;
  }

  .splitter.vertical::before {
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
  }

  .splitter.horizontal {
    height: 6px;
    cursor: row-resize;
  }

  .splitter.horizontal::before {
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    transform: translateY(-50%);
  }

  .splitter:hover::before,
  .splitter:focus-visible::before,
  .splitter:active::before {
    background: var(--pv-accent);
  }

  .splitter.vertical:hover::before,
  .splitter.vertical:focus-visible::before {
    width: 2px;
  }

  .splitter.horizontal:hover::before,
  .splitter.horizontal:focus-visible::before {
    height: 2px;
  }

  .collapse-toggle {
    position: absolute;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 32px;
    padding: 0;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-overlay);
    color: var(--pv-text-secondary);
    box-shadow: var(--pv-shadow-1);
    cursor: default;
    pointer-events: auto;
  }

  .collapse-toggle:hover {
    color: var(--pv-text-primary);
    border-color: var(--pv-border-strong);
  }

  .splitter.horizontal .collapse-toggle {
    width: 32px;
    height: 16px;
  }
</style>
