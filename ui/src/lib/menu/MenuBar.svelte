<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * H-19: the visible `role="menubar"` bar — purely a semantic/visual wrapper. All coordination
   * (which menu is open, focus movement, mnemonics) lives in `menubar.svelte.ts` and each menu
   * component itself; this component holds no logic of its own.
   */
  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="menu-bar" role="menubar" aria-label={label} data-testid="menu-bar">
  {@render children()}
</div>

<style>
  .menu-bar {
    display: flex;
    align-items: center;
    gap: 0.1rem;
    padding: 0.2rem 0.5rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    font-size: 0.85em;
  }
</style>
