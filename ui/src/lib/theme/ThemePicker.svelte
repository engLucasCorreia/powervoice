<script lang="ts">
  import type { ThemePref } from "../ipc/bindings";
  import { tDynamic } from "../i18n";
  import { nextRovingIndex } from "../ui/roving";
  import { resolveTheme, THEMES } from "./theme.svelte";

  /**
   * Preferences → Appearance (T-708): one card per theme, each with a live miniature of the app
   * — toolbar, a waveform well with a selection and the playhead, a level meter — drawn by
   * setting `data-theme` on the miniature itself, so it renders with that theme's real tokens
   * (nothing to keep in sync). "Match system" shows Dark and Light split on a diagonal.
   *
   * A WAI-ARIA radio group with a roving tab stop: arrows move AND select (the theme applies at
   * once), Home/End jump. Selection is shown by the card's accent border and a filled radio dot
   * (shape as well as colour).
   */
  let {
    value,
    label,
    onchange,
  }: {
    value: ThemePref;
    label: string;
    onchange: (pref: ThemePref) => void;
  } = $props();

  let root: HTMLElement | undefined = $state();

  // A still waveform: bar heights (% of the well) of a spoken phrase with a breath.
  const BARS = [18, 34, 62, 48, 88, 70, 42, 16, 8, 26, 58, 94, 66, 38, 52, 30, 12];

  const selectedIndex = $derived(Math.max(0, THEMES.findIndex((c) => c.pref === value)));

  function select(index: number): void {
    const choice = THEMES[index];
    if (choice && choice.pref !== value) {
      onchange(choice.pref);
    }
  }

  function onKeydown(event: KeyboardEvent, index: number): void {
    const next = nextRovingIndex(
      index,
      event.key,
      THEMES.map(() => false),
    );
    if (next === null) {
      return;
    }
    event.preventDefault();
    select(next);
    root?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  }
</script>

{#snippet scene(theme: string, extraClass: string)}
  <span class="scene {extraClass}" data-theme={theme}>
    <span class="bar-strip">
      <span class="lamp"></span>
      <span class="line"></span>
      <span class="pill"></span>
    </span>
    <span class="well">
      <span class="sel"></span>
      {#each BARS as height, i (i)}
        <span class="wave" style="height: {height}%"></span>
      {/each}
      <span class="head"></span>
    </span>
    <span class="meter">
      <span class="level"></span>
      <span class="hold"></span>
    </span>
  </span>
{/snippet}

<div class="picker" role="radiogroup" aria-label={label} bind:this={root} data-testid="theme-picker">
  {#each THEMES as choice, i (choice.pref)}
    {@const checked = choice.pref === value}
    <button
      type="button"
      class="card"
      role="radio"
      aria-checked={checked}
      tabindex={i === selectedIndex ? 0 : -1}
      data-testid="preferences-theme-{choice.pref}"
      onclick={() => select(i)}
      onkeydown={(e) => onKeydown(e, i)}
    >
      <span class="swatch" aria-hidden="true" data-testid="theme-swatch-{choice.pref}">
        {#if choice.pref === "system"}
          {@render scene("dark", "")}
          {@render scene("light", "split")}
        {:else}
          {@render scene(resolveTheme(choice.pref, false), "")}
        {/if}
      </span>
      <span class="text">
        <span class="radio" class:on={checked}></span>
        <span class="name">{tDynamic(choice.labelKey)}</span>
      </span>
      <span class="caption">{tDynamic(choice.captionKey)}</span>
    </button>
  {/each}
</div>

<style>
  .picker {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(8.5rem, 1fr));
    gap: var(--pv-space-3);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    min-width: 0;
    padding: var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-raised);
    color: var(--pv-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--pv-duration-fast) var(--pv-ease-standard),
      box-shadow var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .card:hover {
    border-color: var(--pv-border-strong);
  }

  .card[aria-checked="true"] {
    border-color: var(--pv-accent);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-accent);
  }

  .card:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .swatch {
    position: relative;
    display: block;
    height: 4.5rem;
    margin-bottom: var(--pv-space-1);
    overflow: hidden;
    border-radius: var(--pv-radius-md);
    box-shadow: 0 0 0 var(--pv-border-width) var(--pv-border-subtle);
  }

  /* The miniature: every colour below resolves inside its own `data-theme`. */
  .scene {
    position: absolute;
    inset: 0;
    display: grid;
    grid-template-rows: 0.75rem minmax(0, 1fr) 0.375rem;
    gap: var(--pv-space-1);
    padding: var(--pv-space-1);
    background: var(--pv-bg-app);
  }

  .scene.split {
    clip-path: polygon(58% 0, 100% 0, 100% 100%, 42% 100%);
  }

  .bar-strip {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
    padding: 0 var(--pv-space-1);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-panel);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .lamp {
    width: 0.375rem;
    height: 0.375rem;
    border-radius: var(--pv-radius-full);
    background: var(--pv-record);
  }

  .line {
    flex: 1;
    max-width: 1.75rem;
    height: 0.1875rem;
    border-radius: var(--pv-radius-full);
    background: var(--pv-text-secondary);
  }

  .pill {
    width: 1rem;
    height: 0.3125rem;
    margin-left: auto;
    border-radius: var(--pv-radius-full);
    background: var(--pv-accent-fill);
  }

  .well {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.125rem;
    padding: 0 var(--pv-space-1);
    overflow: hidden;
    border-radius: var(--pv-radius-sm);
    background: var(--wave-bg);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border-subtle);
  }

  .wave {
    flex: 1;
    min-height: 0.125rem;
    border-radius: 0.0625rem;
    background: var(--wave-fill);
  }

  .sel {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 36%;
    width: 28%;
    background: var(--wave-selection-fill);
  }

  .head {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 64%;
    width: var(--pv-stroke-content);
    background: var(--wave-playhead);
  }

  .meter {
    position: relative;
    overflow: hidden;
    border-radius: var(--pv-radius-sm);
    background: var(--pv-meter-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border-subtle);
  }

  .level {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 58%;
    background: var(--pv-meter-safe);
  }

  .hold {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 72%;
    width: 0.125rem;
    background: var(--pv-meter-caution);
  }

  .text {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .radio {
    position: relative;
    flex: none;
    width: 0.875rem;
    height: 0.875rem;
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-full);
    background: var(--pv-field-bg);
  }

  .radio.on {
    border-color: var(--pv-accent);
    background: var(--pv-accent);
    box-shadow: inset 0 0 0 0.1875rem var(--pv-bg-raised);
  }

  .name {
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-medium);
    line-height: var(--pv-leading-md);
  }

  .caption {
    padding-left: calc(0.875rem + var(--pv-space-2));
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
  }
</style>
