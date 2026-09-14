<script lang="ts" module>
  let uid = 0;
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import Kbd from "./Kbd.svelte";

  /**
   * Empty state (H-25): an empty area is an invitation to act — say what goes here, offer the
   * one or two actions that fill it, and teach the shortcuts. `lg` for the editor with no
   * document, `sm` inside panels (no markers, empty rack).
   */
  let {
    icon,
    title,
    description,
    shortcuts = [],
    size = "lg",
    level = 2,
    testid,
    actions,
  }: {
    icon?: IconName;
    title: string;
    description?: string;
    shortcuts?: { label: string; keys: string }[];
    size?: "sm" | "lg";
    level?: 2 | 3;
    testid?: string;
    actions?: Snippet;
  } = $props();

  uid += 1;
  const headingId = `pv-empty-${uid}`;
</script>

<section class="pv-empty" data-size={size} aria-labelledby={headingId} data-testid={testid}>
  {#if icon}
    <span class="glyph"><Icon name={icon} size={size === "lg" ? 28 : "md"} /></span>
  {/if}
  <svelte:element this={`h${level}`} id={headingId} class="title">{title}</svelte:element>
  {#if description}<p class="description">{description}</p>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
  {#if shortcuts.length > 0}
    <dl class="shortcuts">
      {#each shortcuts as s (s.label)}
        <div class="shortcut">
          <dt>{s.label}</dt>
          <dd><Kbd keys={s.keys} /></dd>
        </div>
      {/each}
    </dl>
  {/if}
</section>

<style>
  .pv-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--pv-space-2);
    max-width: 420px;
    margin: auto;
    padding: var(--pv-space-6);
    font-family: var(--pv-font-sans);
    text-align: center;
  }

  .pv-empty[data-size="sm"] {
    gap: var(--pv-space-1);
    padding: var(--pv-space-4) var(--pv-space-3);
  }

  .glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    margin-bottom: var(--pv-space-2);
    border-radius: var(--pv-radius-full);
    background: var(--pv-bg-raised);
    color: var(--pv-text-secondary);
  }

  .pv-empty[data-size="sm"] .glyph {
    width: 32px;
    height: 32px;
    margin-bottom: 0;
  }

  .title {
    margin: 0;
    font-size: var(--pv-text-xl);
    line-height: var(--pv-leading-xl);
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-primary);
  }

  /* In panels the empty state is a quiet hint, not a headline. */
  .pv-empty[data-size="sm"] .title {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-regular);
  }

  .description {
    margin: 0;
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-lg);
    color: var(--pv-text-secondary);
  }

  .pv-empty[data-size="sm"] .description {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--pv-space-2);
    margin-top: var(--pv-space-3);
  }

  .shortcuts {
    display: grid;
    grid-template-columns: auto auto;
    gap: var(--pv-space-2) var(--pv-space-4);
    margin: var(--pv-space-4) 0 0;
  }

  .shortcut {
    display: contents;
  }

  dt {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-secondary);
    text-align: right;
  }

  dd {
    margin: 0;
    text-align: left;
  }
</style>
