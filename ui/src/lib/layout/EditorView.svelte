<script lang="ts">
  import WaveformView from "../waveform/WaveformView.svelte";
</script>

<main class="editor" data-testid="editor">
  <WaveformView />
</main>

<style>
  .editor {
    display: flex;
    min-width: 0;
    min-height: 0;
  }
</style>
