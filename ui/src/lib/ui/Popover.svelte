<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { placePopover, type AnchorRect, type Placement } from "./placement";
  import type { PopoverAnchor } from "./types";

  /**
   * Floating layer anchored to an element or a point (H-26): the one positioning, dismissal and
   * elevation behaviour behind every menu (`Menu.svelte`) and every popover panel (Punch &
   * pre-roll). `position: fixed`, so a panel's `overflow` never clips it; placed by
   * `placement.ts` (flip/shift, never outside the window, scrolls when it can't fit). A pointer
   * press outside it and its anchor closes it (`onclose("outside")`); Escape closes it and returns
   * focus to the anchor (`onclose("escape")`) unless a child handled the key first.
   *
   * Rendered in place (no portal): Svelte's delegated events keep working, and the anchor's
   * panel may scroll or resize — the popover follows it.
   */
  type Variant = "menu" | "panel";

  let {
    open,
    anchor,
    placement = "bottom-start",
    variant = "panel",
    role = "dialog",
    label,
    testid,
    minWidth,
    gapPx,
    alignOffsetPx,
    closeOnEscape = true,
    element = $bindable(),
    onclose,
    onkeydown,
    children,
    ...rest
  }: {
    open: boolean;
    anchor: PopoverAnchor | null | undefined;
    placement?: Placement;
    variant?: Variant;
    role?: "dialog" | "menu";
    label?: string;
    testid?: string;
    /** Minimum width in px, or `"anchor"` to match the anchor's width (a dropdown under a wide button). */
    minWidth?: number | "anchor";
    gapPx?: number;
    alignOffsetPx?: number;
    closeOnEscape?: boolean;
    element?: HTMLDivElement;
    onclose?: (reason: "escape" | "outside") => void;
    onkeydown?: (event: KeyboardEvent) => void;
    children: Snippet;
  } & Omit<HTMLAttributes<HTMLDivElement>, "role" | "onkeydown" | "onclose" | "children" | "class" | "style"> = $props();

  let left = $state(0);
  let top = $state(0);
  let maxHeight = $state<number | null>(null);
  let side = $state<Placement>("bottom-start");
  let anchorWidth = $state(0);
  let placed = $state(false);

  function anchorRect(): AnchorRect | null {
    if (!anchor) {
      return null;
    }
    if (anchor instanceof HTMLElement) {
      const r = anchor.getBoundingClientRect();
      return { left: r.left, top: r.top, width: r.width, height: r.height };
    }
    return { left: anchor.x, top: anchor.y, width: 0, height: 0 };
  }

  function position(): void {
    const el = element;
    const rect = anchorRect();
    if (!el || !rect) {
      return;
    }
    anchorWidth = rect.width;
    const root = document.documentElement;
    const viewport = {
      width: root.clientWidth || window.innerWidth,
      height: root.clientHeight || window.innerHeight,
    };
    // The natural height even while a previous `maxHeight` limits the box (content + borders).
    const size = { width: el.offsetWidth, height: el.scrollHeight + (el.offsetHeight - el.clientHeight) };
    const next = placePopover(rect, size, viewport, placement, { gapPx, alignOffsetPx });
    left = next.left;
    top = next.top;
    maxHeight = next.maxHeight;
    side = next.placement;
    placed = true;
  }

  $effect(() => {
    const el = element;
    if (!open || !el) {
      placed = false;
      return;
    }
    position();
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target;
      if (!(target instanceof Node) || el.contains(target)) {
        return;
      }
      if (anchor instanceof HTMLElement && anchor.contains(target)) {
        return;
      }
      onclose?.("outside");
    };
    const reflow = (): void => position();
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("resize", reflow);
    window.addEventListener("scroll", reflow, true);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(reflow);
    observer?.observe(el);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("resize", reflow);
      window.removeEventListener("scroll", reflow, true);
      observer?.disconnect();
    };
  });

  function handleKeydown(event: KeyboardEvent): void {
    onkeydown?.(event);
    if (event.defaultPrevented || !closeOnEscape || event.key !== "Escape") {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    onclose?.("escape");
    if (anchor instanceof HTMLElement && anchor.isConnected) {
      anchor.focus();
    }
  }

  const minWidthStyle = $derived(
    minWidth === "anchor" ? `${anchorWidth}px` : minWidth === undefined ? undefined : `${minWidth}px`,
  );
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    {...rest}
    bind:this={element}
    class="pv-popover"
    data-variant={variant}
    data-placement={side}
    data-testid={testid}
    {role}
    aria-label={label}
    tabindex="-1"
    style:left="{left}px"
    style:top="{top}px"
    style:max-height={maxHeight === null ? undefined : `${maxHeight}px`}
    style:min-width={minWidthStyle}
    style:opacity={placed ? undefined : "0"}
    onkeydown={handleKeydown}
  >
    {@render children()}
  </div>
{/if}

<style>
  /* Level-2 elevation (design-system §5): overlay surface, border, shadow. No transform in the
     enter animation — a transformed popover would become the containing block of a nested
     fixed submenu and drag it along. */
  .pv-popover {
    position: fixed;
    z-index: var(--pv-z-dropdown);
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    max-width: calc(100vw - 2 * var(--pv-space-2));
    overflow-y: auto;
    border: var(--pv-border-width) solid var(--pv-border);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    text-align: left;
    white-space: normal;
    outline: none;
    animation: pv-popover-in var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-popover[data-variant="menu"] {
    min-width: 12rem;
    padding: var(--pv-space-1);
    border-radius: var(--pv-radius-md);
  }

  .pv-popover[data-variant="panel"] {
    gap: var(--pv-space-2);
    padding: var(--pv-space-3);
    border-radius: var(--pv-radius-lg);
  }

  @keyframes pv-popover-in {
    from {
      opacity: 0;
    }
  }
</style>
