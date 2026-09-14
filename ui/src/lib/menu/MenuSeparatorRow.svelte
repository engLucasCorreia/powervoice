<script lang="ts">
  /** H-19: a visual divider between item groups. No `data-menu-owner`/role="menuitem*" — it's
   * never a keyboard-navigation stop (`menuKeyboard.ts`'s selector only matches `menuitem*`). */
</script>

<div role="separator" aria-orientation="horizontal" class="menu-separator"></div>

<style>
  .menu-separator {
    height: 1px;
    margin: 0.3rem 0.25rem;
    background: var(--surface-border);
  }
</style>
