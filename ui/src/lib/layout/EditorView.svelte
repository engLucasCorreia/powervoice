<script lang="ts">
  import SpectralView from "../spectrogram/SpectralView.svelte";
  import { spectralState } from "../state/spectral.svelte";
  import WaveformView from "../waveform/WaveformView.svelte";

  /**
   * The editor pane (T-207, SPEC-007 §2.1, §2.3): the waveform view, and — when toggled on
   * (Shift+D, `spectral.toggle`) — a spectral view below it, sharing one time axis
   * (`startSample`/`samplesPerPixel`, bound into both children) so a zoom/scroll gesture in
   * either pane updates both instantly. A 6 px divider between them drags the split ratio;
   * double-clicking it resets to 50 % (SPEC-007 §2.1). Both panes stay mounted with a `min-height`
   * floor at their collapsed ratio (0 or 100), rather than being removed, so "spectral only" and
   * "waveform only" are both reachable by dragging back (SPEC-007 §2.1).
   *
   * Deferred (see the ticket report): the shared top time ruler and bottom scrollbar living here
   * rather than inside `WaveformView` — this ticket keeps them where S1-03 put them, to avoid a
   * larger refactor of `WaveformView` while H-11 is concurrently changing it.
   */

  let startSample = $state(0);
  let samplesPerPixel = $state(1);

  const spectral = spectralState();

  let containerEl: HTMLElement | undefined = $state();
  let dragging = false;

  function onDividerPointerDown(event: PointerEvent): void {
    dragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  function onDividerPointerMove(event: PointerEvent): void {
    if (!dragging || !containerEl) {
      return;
    }
    const rect = containerEl.getBoundingClientRect();
    if (rect.height <= 0) {
      return;
    }
    const pct = ((event.clientY - rect.top) / rect.height) * 100;
    spectral.setSplitRatio(pct);
  }

  function onDividerPointerUp(event: PointerEvent): void {
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
  }

  function onDividerDoubleClick(): void {
    spectral.setSplitRatio(50);
  }
</script>

<main class="editor" data-testid="editor" bind:this={containerEl}>
  <div
    class="waveform-pane"
    data-testid="editor-waveform-pane"
    style={spectral.visible ? `flex: ${spectral.splitRatio} 1 0%` : "flex: 1 1 0%"}
  >
    <WaveformView bind:startSample bind:samplesPerPixel />
  </div>
  {#if spectral.visible}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="divider"
      data-testid="editor-divider"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize the spectral pane"
      onpointerdown={onDividerPointerDown}
      onpointermove={onDividerPointerMove}
      onpointerup={onDividerPointerUp}
      ondblclick={onDividerDoubleClick}
    ></div>
    <div
      class="spectral-pane"
      data-testid="editor-spectral-pane"
      style={`flex: ${100 - spectral.splitRatio} 1 0%`}
    >
      <SpectralView bind:startSample bind:samplesPerPixel />
    </div>
  {/if}
</main>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    flex: 1;
  }

  .waveform-pane,
  .spectral-pane {
    display: flex;
    min-height: 6px;
    min-width: 0;
    overflow: hidden;
  }

  .divider {
    flex: none;
    height: 6px;
    cursor: ns-resize;
    background: var(--surface-border);
  }

  .divider:hover {
    background: var(--accent);
  }
</style>
