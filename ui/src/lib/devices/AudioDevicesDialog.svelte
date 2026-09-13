<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { t, tDynamic } from "../i18n";
  import type { DevicePrefsDto, DevicesDto, EventName, IpcError } from "../ipc/bindings";
  import { devicesList, devicesSelect } from "../ipc/commands";
  import { noticeFromIpcError } from "../notices/fromIpcError";
  import { pushNotice } from "../state/notices.svelte";

  /**
   * Settings → Audio Devices (SPEC-001 §2.1, minimal S1-01 form): host, output, input, sample
   * rate and buffer size from `devices_list`; every change applies immediately through
   * `devices_select` (which saves the prefs once applied).
   */
  let { onclose }: { onclose: () => void } = $props();

  let view = $state<DevicesDto | null>(null);
  let busy = $state(false);

  const outputs = $derived(view?.devices.filter((d) => d.output) ?? []);
  const inputs = $derived(view?.devices.filter((d) => d.input && !d.input_is_monitor) ?? []);
  const selectedOutput = $derived.by(() => {
    if (!view) {
      return null;
    }
    const name = view.output_device ?? view.prefs.output_device;
    return view.devices.find((d) => d.name === name) ?? null;
  });
  const selectedInput = $derived.by(() => {
    if (!view) {
      return null;
    }
    const name = view.input_device ?? view.prefs.input_device;
    return view.devices.find((d) => d.name === name) ?? null;
  });
  const inputChannels = $derived(
    Array.from(
      { length: Math.max(selectedInput?.input_channels ?? 0, view?.prefs.input_channel ?? 1, 1) },
      (_, i) => i + 1,
    ),
  );
  const inputStatus = $derived(
    view ? tDynamic(`devices.input_status.${view.input_status}`, { device: view.input_device ?? "" }) : "",
  );
  const rates = $derived(selectedOutput?.output_rates_hz ?? []);
  const buffers = $derived(selectedOutput?.output_buffer_sizes ?? []);
  const status = $derived(
    view
      ? tDynamic(`devices.status.${view.output_status}`, {
          device: view.output_device ?? "",
          rate: view.output_rate_hz ?? 0,
        })
      : "",
  );

  function report(err: unknown): void {
    if (typeof err === "object" && err !== null && "key" in err) {
      pushNotice(noticeFromIpcError(err as IpcError));
    }
  }

  async function apply(patch: Partial<DevicePrefsDto>): Promise<void> {
    if (!view) {
      return;
    }
    busy = true;
    try {
      view = await devicesSelect({ ...view.prefs, ...patch });
    } catch (err) {
      report(err);
    } finally {
      busy = false;
    }
  }

  const optionalNumber = (value: string): number | null => (value === "" ? null : Number(value));
  const optionalString = (value: string): string | null => (value === "" ? null : value);

  function onKeydown(event: KeyboardEvent): void {
    // Keep dialog keys (Space on a select, …) away from the global transport keymap.
    event.stopPropagation();
    if (event.key === "Escape") {
      onclose();
    }
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    devicesList()
      .then((v) => {
        view = v;
      })
      .catch(report);
    listen<DevicesDto>("devices_changed" satisfies EventName, (e) => {
      view = e.payload;
    })
      .then((u) => {
        if (disposed) {
          u();
        } else {
          unlisten = u;
        }
      })
      .catch(() => {});
    return () => {
      disposed = true;
      try {
        unlisten?.();
      } catch {
        // harmless during teardown
      }
    };
  });
</script>

<div class="backdrop">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="audio-devices-title"
    data-testid="audio-devices"
    tabindex="-1"
    onkeydown={onKeydown}
  >
    <h2 id="audio-devices-title">{t("devices.title")}</h2>
    {#if view}
      <label>
        <span>{t("devices.host")}</span>
        <select
          data-testid="devices-host"
          value={view.host ?? ""}
          disabled={busy}
          onchange={(e) => void apply({ host: e.currentTarget.value })}
        >
          {#each view.hosts as host (host)}
            <option value={host}>{tDynamic(`devices.host_name.${host}`)}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t("devices.output")}</span>
        <select
          data-testid="devices-output"
          value={view.prefs.output_device ?? ""}
          disabled={busy}
          onchange={(e) => void apply({ output_device: optionalString(e.currentTarget.value) })}
        >
          <option value="">{t("devices.system_default")}</option>
          {#each outputs as device (device.name)}
            <option value={device.name}>{device.name}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t("devices.input")}</span>
        <select
          data-testid="devices-input"
          value={view.prefs.input_device ?? ""}
          disabled={busy}
          onchange={(e) => void apply({ input_device: optionalString(e.currentTarget.value) })}
        >
          <option value="">{t("devices.none")}</option>
          {#each inputs as device (device.name)}
            <option value={device.name}>{device.name}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t("devices.input_channel")}</span>
        <select
          data-testid="devices-input-channel"
          value={String(view.prefs.input_channel)}
          disabled={busy || view.prefs.input_device === null}
          onchange={(e) => void apply({ input_channel: Number(e.currentTarget.value) })}
        >
          {#each inputChannels as channel (channel)}
            <option value={String(channel)}>{t("devices.channel", { n: channel })}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t("devices.sample_rate")}</span>
        <select
          data-testid="devices-rate"
          value={view.prefs.sample_rate_hz === null ? "" : String(view.prefs.sample_rate_hz)}
          disabled={busy}
          onchange={(e) => void apply({ sample_rate_hz: optionalNumber(e.currentTarget.value) })}
        >
          <option value="">{t("devices.device_default")}</option>
          {#each rates as rate (rate)}
            <option value={String(rate)}>{t("devices.rate_hz", { rate })}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>{t("devices.buffer_size")}</span>
        <select
          data-testid="devices-buffer"
          value={view.prefs.buffer_size_frames === null ? "" : String(view.prefs.buffer_size_frames)}
          disabled={busy}
          onchange={(e) => void apply({ buffer_size_frames: optionalNumber(e.currentTarget.value) })}
        >
          <option value="">{t("devices.auto")}</option>
          {#each buffers as frames (frames)}
            <option value={String(frames)}>{t("devices.frames", { frames })}</option>
          {/each}
        </select>
      </label>
      <p class="status" data-testid="devices-status" data-status={view.output_status}>{status}</p>
      <p class="status" data-testid="devices-input-status" data-status={view.input_status}>{inputStatus}</p>
    {:else}
      <p>{t("devices.loading")}</p>
    {/if}
    <div class="actions">
      <button type="button" data-testid="devices-close" onclick={onclose}>{t("devices.close")}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 900;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    min-width: 28rem;
    max-width: 90vw;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2 {
    margin: 0 0 0.25rem;
    font-size: 1rem;
  }

  label {
    display: grid;
    grid-template-columns: 8rem 1fr;
    align-items: center;
    gap: 0.5rem;
  }

  label span {
    color: var(--text-secondary);
  }

  select {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.4rem;
    min-width: 0;
  }

  .status {
    margin: 0.25rem 0 0;
    color: var(--text-secondary);
  }

  .status[data-status="lost"] {
    color: var(--meter-red);
  }

  .status[data-status="fallback"] {
    color: var(--meter-yellow);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }
</style>
