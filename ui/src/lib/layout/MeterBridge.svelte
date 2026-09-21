<script lang="ts">
  import InputMeter from "../record/InputMeter.svelte";
  import OutputMeter from "./OutputMeter.svelte";

  /**
   * H-41/H-112 (owner request): the whole bridge is a fixed-width row — `width`/`flex: none`,
   * never `min-width` (which still lets a flex item grow to fit its content) — so neither meter's
   * numeric readouts can ever change the analyzer's width next door, no matter how the levels
   * swing. H-112: the input meter is now the same vertical form as the output meter (the owner's
   * request — "input and output side by side is the natural arrangement for setting a recording
   * level"), so the two sit next to each other, each filling the dock's height and resizing with
   * it (H-24's dock splitter). `InputMeter` renders nothing (no DOM at all) while disarmed
   * (`rec.state.input_open` is false), so the bridge is output-meter-only width until armed.
   */
</script>

<footer class="meter-bridge" data-testid="meter-bridge">
  <InputMeter />
  <OutputMeter />
</footer>

<style>
  .meter-bridge {
    display: flex;
    flex-direction: row;
    align-items: stretch;
    flex: none;
    width: 21rem;
    min-height: 0;
    gap: var(--pv-space-4);
    padding: var(--pv-space-3);
    background: var(--pv-bg-panel);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
  }
</style>
