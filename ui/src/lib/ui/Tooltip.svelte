<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import Kbd from "./Kbd.svelte";
  import type { TooltipTriggerProps } from "./types";
  import {
    claimTooltip,
    nextTooltipId,
    openDelayMs,
    placeTooltip,
    releaseTooltip,
    TOOLTIP_DELAY_MS,
  } from "./tooltip";

  /**
   * Tooltip (H-25, WAI-ARIA tooltip pattern): opens after 500 ms hover (instantly when moving
   * between neighbours), immediately on keyboard focus; closes on leave, blur, click and Escape
   * (the first Escape only closes the tooltip — a dialog behind it stays open). Positioned
   * `fixed` so panel `overflow` never clips it; flips/clamps to stay in the viewport.
   *
   * The trigger is rendered by the `children` snippet, which receives `aria-describedby` to spread
   * on the focusable element. `describe={false}` when the text only repeats the trigger's name.
   */
  let {
    text,
    shortcut,
    placement = "bottom",
    delayMs = TOOLTIP_DELAY_MS,
    describe = true,
    disabled = false,
    children,
  }: {
    text: string;
    shortcut?: string;
    placement?: "top" | "bottom";
    delayMs?: number;
    describe?: boolean;
    disabled?: boolean;
    children: Snippet<[TooltipTriggerProps]>;
  } = $props();

  const id = nextTooltipId();
  let open = $state(false);
  let left = $state(0);
  let top = $state(0);
  let side = $state<"top" | "bottom">("bottom");
  let anchorEl: HTMLElement | undefined = $state();
  let tipEl: HTMLElement | undefined = $state();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pointerFocus = false;

  function clearTimer(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function close(): void {
    clearTimer();
    if (open) {
      open = false;
      releaseTooltip(close);
    }
  }

  function show(): void {
    if (disabled) {
      return;
    }
    clearTimer();
    claimTooltip(close);
    open = true;
  }

  function scheduleShow(): void {
    if (disabled || open) {
      return;
    }
    clearTimer();
    const wait = openDelayMs(delayMs);
    if (wait === 0) {
      show();
    } else {
      timer = setTimeout(show, wait);
    }
  }

  function onFocusIn(): void {
    if (pointerFocus) {
      pointerFocus = false;
      return;
    }
    show();
  }

  function onPointerDown(): void {
    pointerFocus = true;
    close();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && open) {
      event.stopPropagation();
      close();
    }
  }

  // Position once the tooltip is visible and measurable.
  $effect(() => {
    if (!open || !anchorEl || !tipEl) {
      return;
    }
    const a = anchorEl.getBoundingClientRect();
    const t = tipEl.getBoundingClientRect();
    const p = placeTooltip(
      { left: a.left, top: a.top, width: a.width, height: a.height },
      { width: t.width, height: t.height },
      { width: window.innerWidth, height: window.innerHeight },
      placement,
    );
    left = p.left;
    top = p.top;
    side = p.placement;
  });

  onDestroy(close);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<span
  class="pv-tooltip-anchor"
  bind:this={anchorEl}
  onpointerenter={scheduleShow}
  onpointerleave={close}
  onpointerdown={onPointerDown}
  onfocusin={onFocusIn}
  onfocusout={close}
  onkeydown={onKeydown}
>
  {@render children({ "aria-describedby": describe ? id : undefined })}
  <span
    bind:this={tipEl}
    {id}
    role="tooltip"
    class="pv-tooltip"
    data-placement={side}
    hidden={!open}
    style:left="{left}px"
    style:top="{top}px"
  >
    <span class="text">{text}</span>
    {#if shortcut}<Kbd keys={shortcut} size="xs" />{/if}
  </span>
</span>

<style>
  .pv-tooltip-anchor {
    display: inline-flex;
    flex: none;
  }

  .pv-tooltip {
    position: fixed;
    z-index: var(--pv-z-tooltip);
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-2);
    max-width: 280px;
    padding: var(--pv-space-1) var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-regular);
    white-space: normal;
    pointer-events: none;
    animation: pv-tip-in var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-tooltip[hidden] {
    display: none;
  }

  @keyframes pv-tip-in {
    from {
      opacity: 0;
    }
  }
</style>
