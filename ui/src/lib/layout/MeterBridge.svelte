<script lang="ts">
  import InputMeter from "../record/InputMeter.svelte";
  import OutputMeter from "./OutputMeter.svelte";

  /**
   * H-41 (owner request): the whole bridge is a fixed-width column — `width`/`flex: none`, never
   * `min-width` (which still lets a flex item grow to fit its content) — so neither its own
   * numeric readouts nor the vertical output meter beside it can ever change the analyzer's
   * width next door, no matter how the levels swing. The input meter (SPEC-002, unchanged by
   * this ticket) stays a horizontal row on top; the output meter (H-41) fills the rest of the
   * dock's height vertically and resizes with it (H-24's dock splitter).
   */
</script>

<footer class="meter-bridge" data-testid="meter-bridge">
  <InputMeter />
  <OutputMeter />
</footer>

<style>
  .meter-bridge {
    display: flex;
    flex-direction: column;
    flex: none;
    width: 13rem;
    min-height: 0;
    gap: var(--pv-space-3);
    padding: var(--pv-space-3);
    background: var(--pv-bg-panel);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
  }
</style>
