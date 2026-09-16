<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * H-19: the visible `role="menubar"` bar — purely a semantic/visual wrapper. All coordination
   * (which menu is open, focus movement, mnemonics) lives in `menu.svelte.ts` and each menu
   * component itself; this component holds no logic of its own.
   */
  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="menu-bar" role="menubar" aria-label={label} data-testid="menu-bar" data-tour="menubar">
  {@render children()}
</div>

<style>
  .menu-bar {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--pv-space-half);
    height: var(--pv-menubar-h);
    padding: 0 var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-panel);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
  }
</style>
