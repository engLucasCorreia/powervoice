<script lang="ts">
  import { onMount } from "svelte";
  import { initOutputDeviceStatus, outputDeviceStatus } from "../analyzer/outputDeviceStatus.svelte";
  import { t, tDynamic } from "../i18n";
  import type { DeviceStatusDto } from "../ipc/bindings";
  import { StatusDot, Tooltip } from "../ui";

  /**
   * SPEC-001 §2.1: the transport bar's small device-status control — the current output device's
   * name with its §2.3 status lamp; clicking it opens Settings → Audio Devices. Before H-59 the
   * toolbar only had an unlabelled gear, so nothing outside the dialog ever told the user which
   * device was playing, or that it was in fallback or lost.
   */
  let { onopen }: { onopen: () => void } = $props();

  const device = outputDeviceStatus();

  onMount(() => {
    let stop: (() => void) | null = null;
    let disposed = false;
    void initOutputDeviceStatus().then((s) => {
      if (disposed) {
        s();
      } else {
        stop = s;
      }
    });
    return () => {
      disposed = true;
      stop?.();
    };
  });

  const TONES = {
    not_selected: "neutral",
    healthy: "success",
    fallback: "warning",
    lost: "danger",
  } as const satisfies Record<DeviceStatusDto, string>;

  const status = $derived(device.current);
  const tone = $derived(status ? TONES[status] : "neutral");
  // With no device selected (or before the first fetch) the control names the action instead,
  // so the button is never a bare dot.
  const name = $derived(device.name ?? t("devices.no_output"));
  const statusText = $derived(
    status
      ? tDynamic(`devices.status.${status}`, { device: device.name ?? "", rate: 0 })
      : t("devices.open"),
  );
</script>

<Tooltip text={statusText} describe={false}>
  {#snippet children(trigger)}
    <button
      {...trigger}
      type="button"
      class="device-status"
      data-testid="open-audio-devices"
      data-tour="devices"
      data-status={status ?? "unknown"}
      aria-label={statusText}
      onclick={onopen}
    >
      <StatusDot {tone} label={statusText} />
      <span class="name">{name}</span>
    </button>
  {/snippet}
</Tooltip>

<style>
  .device-status {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    max-width: 14ch;
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid transparent;
    border-radius: var(--pv-radius-md);
    background: transparent;
    color: var(--pv-text-secondary);
    font: inherit;
    font-size: var(--pv-text-sm);
    cursor: pointer;
  }

  .device-status:hover {
    background: var(--pv-bg-hover);
    color: var(--pv-text-primary);
  }

  .device-status:focus-visible {
    outline: var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .device-status[data-status="lost"] {
    color: var(--pv-danger-text);
  }

  .device-status[data-status="fallback"] {
    color: var(--pv-warning-text);
  }

  .name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
</style>
