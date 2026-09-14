<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  /**
   * The one panel header (H-25): 32 px, sentence-case title in 12 px semibold secondary text,
   * optional meta (count, latency), actions on the right (ghost IconButtons), optional collapse
   * disclosure (WAI-ARIA: the button lives inside the heading and names itself by the title).
   * Every docked panel — Markers, Rack, Loudness, Analyzer — uses it so they align.
   */
  let {
    title,
    level = 2,
    meta,
    collapsible = false,
    expanded = $bindable(true),
    controls,
    testid,
    ontoggle,
    actions,
  }: {
    title: string;
    level?: 2 | 3;
    meta?: string;
    collapsible?: boolean;
    expanded?: boolean;
    controls?: string;
    testid?: string;
    ontoggle?: (expanded: boolean) => void;
    actions?: Snippet;
  } = $props();

  function toggle(): void {
    expanded = !expanded;
    ontoggle?.(expanded);
  }
</script>

<header class="pv-panel-header" data-testid={testid}>
  <svelte:element this={`h${level}`} class="title">
    {#if collapsible}
      <button
        type="button"
        class="disclosure"
        aria-expanded={expanded}
        aria-controls={controls}
        onclick={toggle}
      >
        <span class="chevron" class:open={expanded}><Icon name="chevronRight" size="sm" /></span>
        <span>{title}</span>
      </button>
    {:else}
      {title}
    {/if}
  </svelte:element>
  {#if meta}<span class="meta">{meta}</span>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  .pv-panel-header {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
    height: var(--pv-panel-header-h);
    padding-inline: var(--pv-space-3) var(--pv-space-1);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    font-family: var(--pv-font-sans);
  }

  .title {
    display: flex;
    align-items: center;
    min-width: 0;
    margin: 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    height: var(--pv-control-h-sm);
    margin-left: calc(-1 * var(--pv-space-1));
    padding: 0 var(--pv-space-1);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: default;
  }

  .disclosure:hover {
    color: var(--pv-text-primary);
  }

  .disclosure:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .chevron {
    display: inline-flex;
    transition: transform var(--pv-duration-base) var(--pv-ease-standard);
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  .meta {
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--pv-space-half);
    margin-left: auto;
  }
</style>
