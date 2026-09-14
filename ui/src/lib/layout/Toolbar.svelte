<script lang="ts">
  import AudioDevicesDialog from "../devices/AudioDevicesDialog.svelte";
  import { t } from "../i18n";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import NormalizeToolbarButtons from "../normalize/NormalizeToolbarButtons.svelte";
  import RecordControls from "../record/RecordControls.svelte";
  import { recordState } from "../state/record.svelte";
  import { spectralState } from "../state/spectral.svelte";
  import { playFromStart, playPause, returnToStart, stop, transportState } from "../state/transport.svelte";
  import { formatTime } from "../transport/playhead";
  import { IconButton, Separator } from "../ui";

  /**
   * H-25 transport bar (design-system §11): groups separated by hairlines — transport (icon keys
   * with tooltips + shortcuts), the playhead time (the chrome's one large element), recording,
   * input & monitoring, view — then the occasional actions (Normalize favourites, devices) on the
   * right. Groups wrap as whole groups on narrow windows; button labels never wrap. While
   * recording, a red tally line runs along the top edge. The app name/version live in Help → About.
   */
  // `version` is still passed by App.svelte; it's shown in Help → About now, not here.
  const props: { version?: string } = $props();

  const transport = transportState();
  const spectral = spectralState();
  const rec = recordState();
  let devicesOpen = $state(false);
  const time = $derived(formatTime(transport.playheadSamples, transport.state.doc_rate_hz));
  const playing = $derived(transport.state.playing);
</script>

<header class="toolbar" class:on-air={rec.state.recording} data-testid="toolbar">
  <div class="group" role="group" aria-label={t("transport.group")}>
    <IconButton
      icon="returnToStart"
      label={t("transport.return_to_start")}
      shortcut={shortcutLabelForAction("transport.return_to_start")}
      testid="transport-return"
      onclick={() => void returnToStart()}
    />
    <IconButton
      icon={playing ? "pause" : "play"}
      label={playing ? t("transport.pause") : t("transport.play")}
      shortcut={shortcutLabelForAction("transport.play_pause")}
      size="lg"
      pressed={playing}
      testid="transport-play"
      disabled={!playing && !transport.state.can_play}
      onclick={() => void playPause()}
    />
    <IconButton
      icon="stop"
      label={t("transport.stop")}
      testid="transport-stop"
      disabled={!playing}
      onclick={() => void stop()}
    />
    <IconButton
      icon="playFromStart"
      label={t("transport.play_from_start")}
      shortcut={shortcutLabelForAction("transport.play_from_start")}
      testid="transport-play-from-start"
      disabled={!transport.state.can_play}
      onclick={() => void playFromStart()}
    />
  </div>
  <Separator orientation="vertical" />
  <span class="time" data-testid="transport-time">{time}</span>
  <Separator orientation="vertical" />
  <RecordControls />
  <Separator orientation="vertical" />
  <div class="group">
    <IconButton
      icon="spectral"
      label={t("spectral.toggle")}
      shortcut={shortcutLabelForAction("spectral.toggle")}
      pressed={spectral.visible}
      testid="spectral-toggle"
      onclick={() => spectral.toggle()}
    />
  </div>
  <span class="spacer"></span>
  <div class="group">
    <NormalizeToolbarButtons />
    <IconButton
      icon="settings"
      label={t("devices.open")}
      testid="open-audio-devices"
      onclick={() => (devicesOpen = true)}
    />
  </div>
</header>
{#if devicesOpen}
  <AudioDevicesDialog onclose={() => (devicesOpen = false)} />
{/if}

<style>
  .toolbar {
    container: toolbar / inline-size;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-2);
    min-width: 0;
    min-height: var(--pv-toolbar-h);
    padding: var(--pv-space-1) var(--pv-space-3);
    background: var(--pv-bg-panel);
    border-bottom: var(--pv-border-width) solid var(--pv-border);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    transition: box-shadow var(--pv-duration-base) var(--pv-ease-standard);
  }

  /* On air: a broadcast-style tally line along the top of the transport. */
  .toolbar.on-air {
    box-shadow: inset 0 2px 0 var(--pv-record);
  }

  .group {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .time {
    flex: none;
    min-width: 8.4ch;
    padding-inline: var(--pv-space-1);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-xl);
    line-height: var(--pv-leading-xl);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }
</style>
