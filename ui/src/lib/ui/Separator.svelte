<script lang="ts">
  /**
   * Hairline divider (H-25). Vertical ones split toolbar groups (transport | record | view);
   * horizontal ones split sections inside a panel or menu. Spacing comes from the parent's `gap`,
   * never from margins here. `decorative` for purely visual lines.
   */
  let {
    orientation = "horizontal",
    decorative = false,
  }: { orientation?: "horizontal" | "vertical"; decorative?: boolean } = $props();
</script>

<div
  class="pv-separator"
  data-orientation={orientation}
  role={decorative ? undefined : "separator"}
  aria-orientation={decorative ? undefined : orientation}
  aria-hidden={decorative ? "true" : undefined}
></div>

<style>
  .pv-separator {
    flex: none;
    background: var(--pv-border);
  }

  .pv-separator[data-orientation="horizontal"] {
    height: var(--pv-border-width);
    width: 100%;
  }

  .pv-separator[data-orientation="vertical"] {
    width: var(--pv-border-width);
    height: var(--pv-icon-lg);
    align-self: center;
  }
</style>
