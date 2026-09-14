<script lang="ts" generics="T extends string">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { nextRovingIndex } from "./roving";
  import type { TabItem } from "./types";

  /**
   * Tabs (H-25, WAI-ARIA tabs with automatic activation): switch between views that share one
   * area — the bottom dock (Meters | Analyzer | Loudness), dialog sections. Underline style, 32 px
   * tall so a tab row can be a panel header. Left/Right/Home/End move focus and select. Pass
   * `panel` to render the tabpanel here, or render it yourself with the same ids
   * (`{idPrefix}-tab-{id}` / `{idPrefix}-panel-{id}`).
   */
  let {
    tabs,
    selected = $bindable(),
    label,
    idPrefix,
    panel,
    testid,
    onchange,
  }: {
    tabs: TabItem<T>[];
    selected: T;
    label: string;
    idPrefix: string;
    panel?: Snippet<[T]>;
    testid?: string;
    onchange?: (id: T) => void;
  } = $props();

  let list: HTMLElement | undefined = $state();
  const disabledFlags = $derived(tabs.map((t) => t.disabled === true));
  const selectedIndex = $derived(tabs.findIndex((t) => t.id === selected));
  const tabStop = $derived(selectedIndex >= 0 ? selectedIndex : disabledFlags.findIndex((d) => !d));

  function choose(index: number): void {
    const tab = tabs[index];
    if (!tab || tab.disabled) {
      return;
    }
    if (tab.id !== selected) {
      selected = tab.id;
      onchange?.(tab.id);
    }
  }

  function onKeydown(event: KeyboardEvent, index: number): void {
    const next = nextRovingIndex(index, event.key, disabledFlags, "horizontal");
    if (next === null) {
      return;
    }
    event.preventDefault();
    choose(next);
    list?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
  }
</script>

<div class="pv-tabs" data-testid={testid}>
  <div bind:this={list} class="list" role="tablist" aria-label={label} aria-orientation="horizontal">
    {#each tabs as tab, i (tab.id)}
      <button
        type="button"
        role="tab"
        class="tab"
        id="{idPrefix}-tab-{tab.id}"
        aria-selected={tab.id === selected}
        aria-controls="{idPrefix}-panel-{tab.id}"
        tabindex={i === tabStop ? 0 : -1}
        disabled={tab.disabled}
        onclick={() => choose(i)}
        onkeydown={(e) => onKeydown(e, i)}
      >
        {#if tab.icon}<Icon name={tab.icon} size="sm" />{/if}
        <span>{tab.label}</span>
        {#if tab.badge}<span class="badge">{tab.badge}</span>{/if}
      </button>
    {/each}
  </div>
  {#if panel}
    <div
      class="panel"
      role="tabpanel"
      id="{idPrefix}-panel-{selected}"
      aria-labelledby="{idPrefix}-tab-{selected}"
      tabindex="0"
    >
      {@render panel(selected)}
    </div>
  {/if}
</div>

<style>
  .pv-tabs {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .list {
    display: flex;
    flex: none;
    align-items: stretch;
    gap: var(--pv-space-1);
    height: var(--pv-panel-header-h);
    padding-inline: var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border);
  }

  .tab {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: calc(var(--pv-space-1) + var(--pv-space-half));
    padding-inline: var(--pv-space-2);
    border: none;
    background: transparent;
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-medium);
    white-space: nowrap;
    cursor: default;
    transition: color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .tab::after {
    content: "";
    position: absolute;
    inset: auto var(--pv-space-2) -1px;
    height: 2px;
    border-radius: 1px;
    background: transparent;
    transition: background-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .tab:hover:not(:disabled) {
    color: var(--pv-text-primary);
  }

  .tab[aria-selected="true"] {
    color: var(--pv-text-primary);
  }

  .tab[aria-selected="true"]::after {
    background: var(--pv-accent);
  }

  .tab:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -4px;
    border-radius: var(--pv-radius-sm);
  }

  .tab:disabled {
    color: var(--pv-text-disabled);
  }

  .badge {
    min-width: 16px;
    padding-inline: var(--pv-space-1);
    border-radius: var(--pv-radius-full);
    background: var(--pv-control-bg-selected);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .panel {
    flex: 1;
    min-height: 0;
    outline: none;
  }

  .panel:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
  }
</style>
