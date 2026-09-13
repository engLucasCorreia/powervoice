<script lang="ts">
  import AudioDevicesDialog from "../devices/AudioDevicesDialog.svelte";
  import { t } from "../i18n";
  import { playFromStart, playPause, returnToStart, stop, transportState } from "../state/transport.svelte";
  import { formatTime } from "../transport/playhead";

  let { version = "" }: { version?: string } = $props();

  const transport = transportState();
  let devicesOpen = $state(false);
  const time = $derived(formatTime(transport.playheadSamples, transport.state.doc_rate_hz));
</script>

<header class="toolbar" data-testid="toolbar">
  <span class="app-name">{t("app.title")}</span>
  <span class="version" data-testid="app-version">{version}</span>
  <div class="transport" role="group" aria-label={t("transport.group")}>
    <button type="button" data-testid="transport-return" onclick={() => void returnToStart()}>
      {t("transport.return_to_start")}
    </button>
    <button
      type="button"
      data-testid="transport-play"
      class:active={transport.state.playing}
      disabled={!transport.state.playing && !transport.state.can_play}
      onclick={() => void playPause()}
    >
      {transport.state.playing ? t("transport.pause") : t("transport.play")}
    </button>
    <button
      type="button"
      data-testid="transport-stop"
      disabled={!transport.state.playing}
      onclick={() => void stop()}
    >
      {t("transport.stop")}
    </button>
    <button
      type="button"
      data-testid="transport-play-from-start"
      disabled={!transport.state.can_play}
      onclick={() => void playFromStart()}
    >
      {t("transport.play_from_start")}
    </button>
    <span class="time" data-testid="transport-time">{time}</span>
  </div>
  <button type="button" data-testid="open-audio-devices" onclick={() => (devicesOpen = true)}>
    {t("devices.open")}
  </button>
</header>
{#if devicesOpen}
  <AudioDevicesDialog onclose={() => (devicesOpen = false)} />
{/if}

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
  }

  .app-name {
    font-weight: 600;
  }

  .version {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .transport {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-left: auto;
  }

  .time {
    min-width: 7.5rem;
    padding: 0.2rem 0.5rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  button.active {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
