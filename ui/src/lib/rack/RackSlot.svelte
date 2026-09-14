<script lang="ts">
  import { Button, Icon, IconButton } from "../ui";
  import EqGraph from "../eq/EqGraph.svelte";
  import { t } from "../i18n";
  import type { ParamInfoDto, PresetEntryDto, RackSlotDto } from "../ipc/bindings";
  import GainReductionMeter from "./GainReductionMeter.svelte";
  import { localized } from "./localized";
  import NoiseReductionSection from "./NoiseReductionSection.svelte";
  import ParamGroupSection from "./ParamGroupSection.svelte";
  import {
    deleteModulePreset,
    listModulePresets,
    loadModulePreset,
    noteSlotFocused,
    removeSlot,
    resetSlotToDefault,
    restartSlot,
    saveModulePreset,
    setBypass,
    slotTelemetry,
  } from "./rack.svelte";

  /**
   * One rack slot (SPEC-012 §2.1): header (bypass, name, latency, menu, collapse) and the
   * generic parameter body, or a status message for a placeholder/failed slot. Drag-reorder is
   * plain HTML5 DnD on the header, delegated to the parent (`RackPanel`) so only it knows the
   * drop index among siblings.
   */
  let {
    slot,
    index,
    rateHz,
    dragOver,
    ondragstart,
    ondragover,
    ondrop,
    ondragend,
  }: {
    slot: RackSlotDto;
    index: number;
    rateHz: number;
    dragOver: boolean;
    ondragstart: (index: number) => void;
    ondragover: (index: number, event: DragEvent) => void;
    ondrop: (index: number) => void;
    ondragend: () => void;
  } = $props();

  let collapsed = $state(false);
  let menuOpen = $state(false);

  // T-802: a sandboxed plugin's status (Running / Restarting / Plugin failed), or "Not installed"
  // for a slot whose module isn't registered. In-process modules show no badge while active.
  const statusBadge = $derived.by((): { kind: string; label: string } | null => {
    switch (slot.status.kind) {
      case "active":
        return slot.sandboxed ? { kind: "running", label: t("rack.slot.status.running") } : null;
      case "restarting":
        return { kind: "restarting", label: t("rack.slot.status.restarting") };
      case "loading":
        // T-803: an out-of-process plugin is being started (off the control thread).
        return { kind: "loading", label: t("rack.slot.status.loading") };
      case "failed":
        return { kind: "failed", label: t("rack.slot.status.failed") };
      case "missing":
        return slot.status.too_new ? null : { kind: "not_installed", label: t("rack.slot.status.not_installed") };
    }
    return null;
  });
  /** Retry = the rack's Restart of a failed slot (a sandboxed plugin is respawned with its
   * last committed state). A missing module can't be restarted (SPEC-012 §2.9). */
  const canRetry = $derived(slot.status.kind === "failed");

  // --- Presets (T-406, SPEC-012 §2.7) -----------------------------------------------------
  let presetsOpen = $state(false);
  let presetEntries = $state<PresetEntryDto[] | null>(null);
  let savingPreset = $state(false);
  let presetName = $state("");
  let includeNoisePrint = $state(false);

  function closeMenu(): void {
    menuOpen = false;
    presetsOpen = false;
    savingPreset = false;
  }

  async function openPresetsSubmenu(): Promise<void> {
    presetsOpen = true;
    savingPreset = false;
    if (slot.module_id) {
      presetEntries = await listModulePresets(slot.module_id);
    }
  }

  async function refreshPresets(): Promise<void> {
    if (slot.module_id) {
      presetEntries = await listModulePresets(slot.module_id);
    }
  }

  function startSavePreset(): void {
    savingPreset = true;
    presetName = "";
    includeNoisePrint = false;
  }

  async function confirmSavePreset(): Promise<void> {
    const name = presetName.trim();
    if (!name || !slot.module_id) {
      return;
    }
    const saved = await saveModulePreset(index, name, includeNoisePrint);
    if (saved) {
      savingPreset = false;
      presetName = "";
      await refreshPresets();
    }
  }

  async function pickPreset(entry: PresetEntryDto): Promise<void> {
    if (!slot.module_id) {
      return;
    }
    closeMenu();
    await loadModulePreset(
      index,
      slot.module_id,
      entry.is_factory ? { kind: "factory", key: entry.key } : { kind: "user", name: entry.key },
    );
  }

  async function deletePreset(entry: PresetEntryDto, event: MouseEvent): Promise<void> {
    event.stopPropagation();
    if (!slot.module_id) {
      return;
    }
    if (await deleteModulePreset(slot.module_id, entry.key)) {
      await refreshPresets();
    }
  }

  const latencyLabel = $derived(
    slot.latency_samples > 0
      ? t("rack.slot.latency", {
          ms: ((slot.latency_samples / Math.max(1, rateHz)) * 1000).toFixed(1),
          samples: slot.latency_samples,
        })
      : "",
  );

  const groupsByKey = $derived(new Map(slot.groups.map((g) => [g.id, g])));

  // SPEC-012 §2.6 AC-12: the panel omits HIDDEN and BYPASS parameters (the latter is already the
  // slot's own power button) from every body — ungrouped list, group visibility check, and group
  // body — so no widget ever shows a redundant or internal control.
  const shown = (p: ParamInfoDto): boolean => !p.flags.hidden && !p.flags.bypass;

  // H-01/S3-02 handoff: hide a group whose enable param and all body params are HIDDEN, else an
  // empty section header appears (e.g. Dynamics' AutoGate/Expander stubs).
  const visibleGroups = $derived(
    slot.groups.filter((g) => {
      const enable =
        g.enable_param !== null ? slot.params.find((p) => p.id === g.enable_param) : undefined;
      const enableHidden = enable ? !shown(enable) : true;
      const bodyMembers = slot.params.filter((p) => p.group === g.id && p.id !== g.enable_param);
      const bodyAllHidden = bodyMembers.every((p) => !shown(p));
      return !(enableHidden && bodyAllHidden);
    }),
  );
  const ungrouped = $derived(slot.params.filter((p) => p.group === null && shown(p)));

  // H-03 (SPEC-017 §2.3 "Meter", SPEC-016 §4.12): every `gain_reduction` telemetry channel the
  // module places in its header (`group` null) is a meter here, fed by `VXMT` frames through the
  // rack store. Generic: any module with such a channel gets one (the true-peak limiter's GR,
  // Dynamics' total GR, the Noise Gate's gain).
  const headerMeters = $derived(
    (slot.telemetry ?? [])
      .map((channel, index) => ({ channel, index }))
      .filter(({ channel }) => channel.kind === "gain_reduction" && channel.group === null),
  );
  const meterValues = $derived(slotTelemetry(slot.uid));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<section
  class="slot"
  class:drag-over={dragOver}
  class:bypassed={slot.bypass}
  data-testid="rack-slot"
  data-status={slot.status.kind}
  role="group"
  aria-label={slot.name}
  draggable="true"
  ondragstart={() => ondragstart(index)}
  ondragover={(e) => ondragover(index, e)}
  ondrop={(e) => {
    e.preventDefault();
    ondrop(index);
  }}
  ondragend={ondragend}
  onfocusin={() => noteSlotFocused(index)}
>
  <header>
    <span class="grip" aria-hidden="true"><Icon name="drag" size="sm" /></span>
    <IconButton
      icon="bypass"
      label={t("rack.slot.bypass")}
      size="sm"
      pressed={!slot.bypass}
      testid="rack-slot-bypass"
      onclick={() => void setBypass(index, !slot.bypass)}
    />
    <IconButton
      icon={collapsed ? "chevronRight" : "chevronDown"}
      label={collapsed ? t("rack.slot.expand") : t("rack.slot.collapse")}
      size="sm"
      aria-expanded={!collapsed}
      onclick={() => (collapsed = !collapsed)}
    />
    <span class="name" data-testid="rack-slot-name">{slot.name}</span>
    {#if statusBadge}
      <span class="status-badge {statusBadge.kind}" data-testid="rack-slot-badge" data-badge={statusBadge.kind}
        >{statusBadge.label}</span
      >
    {/if}
    {#if latencyLabel}
      <span class="latency">{latencyLabel}</span>
    {/if}
    {#if slot.status.kind === "active"}
      {#each headerMeters as { channel, index } (channel.id)}
        <GainReductionMeter
          value={meterValues?.[index]}
          min={channel.min}
          max={channel.max}
          name={localized(channel.name)}
        />
      {/each}
    {/if}
    <div class="menu-wrap">
      <IconButton
        icon="more"
        label={t("rack.slot.menu")}
        size="sm"
        testid="rack-slot-menu"
        aria-expanded={menuOpen}
        onclick={() => {
          menuOpen = !menuOpen;
          if (!menuOpen) {
            closeMenu();
          }
        }}
      />
      {#if menuOpen}
        <div class="menu" role="menu">
          <button
            type="button"
            role="menuitem"
            data-testid="rack-slot-restart"
            onclick={() => {
              closeMenu();
              void restartSlot(index);
            }}
          >
            {t("rack.slot.menu.restart")}
          </button>
          <button
            type="button"
            role="menuitem"
            data-testid="rack-slot-remove"
            onclick={() => {
              closeMenu();
              void removeSlot(index);
            }}
          >
            {t("rack.slot.menu.remove")}
          </button>
          {#if slot.module_id}
            <hr />
            <button
              type="button"
              role="menuitem"
              data-testid="rack-slot-presets"
              aria-expanded={presetsOpen}
              onclick={() => (presetsOpen ? (presetsOpen = false) : void openPresetsSubmenu())}
            >
              <span class="grow">{t("rack.slot.menu.presets")}</span>
              <Icon name={presetsOpen ? "chevronDown" : "chevronRight"} size="sm" />
            </button>
            {#if presetsOpen}
              <div class="submenu" data-testid="rack-slot-presets-submenu">
                {#if presetEntries === null}
                  <span class="preset-empty">…</span>
                {:else if presetEntries.length === 0 && !savingPreset}
                  <span class="preset-empty">{t("rack.slot.preset.none")}</span>
                {:else}
                  {#each presetEntries as entry (entry.is_factory + ":" + entry.key)}
                    <div class="preset-row">
                      <button
                        type="button"
                        role="menuitem"
                        class="preset-name"
                        data-testid="rack-slot-preset-{entry.key}"
                        onclick={() => void pickPreset(entry)}
                      >
                        {localized(entry.name)}
                      </button>
                      {#if !entry.is_factory}
                        <button
                          type="button"
                          class="preset-delete"
                          title={t("rack.slot.preset.delete")}
                          data-testid="rack-slot-preset-delete-{entry.key}"
                          onclick={(e) => void deletePreset(entry, e)}
                          aria-label={t("rack.slot.preset.delete")}
                        >
                          <Icon name="close" size="sm" />
                        </button>
                      {/if}
                    </div>
                  {/each}
                {/if}
                <hr />
                {#if savingPreset}
                  <div class="save-form">
                    <input
                      type="text"
                      placeholder={t("rack.slot.preset.name_placeholder")}
                      data-testid="rack-slot-preset-name"
                      bind:value={presetName}
                      onkeydown={(e) => {
                        if (e.key === "Enter") void confirmSavePreset();
                      }}
                    />
                    {#if slot.noise_profile !== null}
                      <label class="checkbox-row">
                        <input type="checkbox" bind:checked={includeNoisePrint} />
                        {t("rack.slot.preset.include_noise_print")}
                      </label>
                    {/if}
                    <div class="save-actions">
                      <Button size="sm" onclick={() => (savingPreset = false)}>
                        {t("rack.slot.preset.cancel_button")}
                      </Button>
                      <Button
                        size="sm"
                        variant="primary"
                        testid="rack-slot-preset-save-confirm"
                        onclick={() => void confirmSavePreset()}
                      >
                        {t("rack.slot.preset.save_button")}
                      </Button>
                    </div>
                  </div>
                {:else}
                  <button
                    type="button"
                    role="menuitem"
                    data-testid="rack-slot-preset-save"
                    onclick={startSavePreset}
                  >
                    {t("rack.slot.preset.save_as")}
                  </button>
                {/if}
              </div>
            {/if}
            <button
              type="button"
              role="menuitem"
              data-testid="rack-slot-reset-default"
              onclick={() => {
                closeMenu();
                void resetSlotToDefault(index);
              }}
            >
              {t("rack.slot.menu.reset_default")}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </header>
  {#if slot.status.kind !== "active" && slot.status.kind !== "loading"}
    <div class="status-row">
      <p class="status-message" data-testid="rack-slot-status">{slot.status.message}</p>
      {#if canRetry}
        <Button size="sm" testid="rack-slot-retry" onclick={() => void restartSlot(index)}>
          {t("rack.slot.retry")}
        </Button>
      {/if}
    </div>
  {/if}
  {#if !collapsed && slot.status.kind === "active"}
    <div class="body">
      {#if slot.curve_handles !== null}
        <EqGraph slotIndex={index} rackSlot={slot} {rateHz} />
      {/if}
      {#if slot.noise_profile !== null}
        <NoiseReductionSection slotIndex={index} status={slot.noise_profile} />
      {/if}
      {#if ungrouped.length > 0}
        <ParamGroupSection slotIndex={index} rackSlot={slot} group={null} {groupsByKey} params={ungrouped} />
      {/if}
      {#each visibleGroups as group (group.id)}
        <ParamGroupSection
          slotIndex={index}
          rackSlot={slot}
          {group}
          {groupsByKey}
          params={slot.params.filter((p) => p.group === group.id && shown(p))}
        />
      {/each}
    </div>
  {/if}
</section>

<style>
  /* H-25: a slot is a raised card — header (grip, power, disclosure, name, status, latency, gain
     reduction, more) over its body. Bypassed dims the body only; the header stays readable so
     the power key that brings it back is never faded. */
  .slot {
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-raised);
    font-family: var(--pv-font-sans);
    transition: border-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .slot.drag-over {
    border-color: var(--pv-accent);
    box-shadow: 0 0 0 1px var(--pv-accent);
  }

  .slot.bypassed .body {
    opacity: 0.45;
  }

  .slot.bypassed .name {
    color: var(--pv-text-secondary);
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
    min-height: var(--pv-panel-header-h);
    padding: 0 var(--pv-space-1);
  }

  .grip {
    display: inline-flex;
    color: var(--pv-text-tertiary);
    cursor: grab;
  }

  .name {
    flex: 1;
    min-width: 0;
    margin-left: var(--pv-space-1);
    overflow: hidden;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .latency {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* T-802 status: soft tone chips with the word (never colour alone). */
  .status-badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding-inline: var(--pv-space-1);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-medium);
    white-space: nowrap;
  }

  .status-badge.running {
    background: var(--pv-success-soft);
    color: var(--pv-success-text);
  }

  .status-badge.restarting {
    background: var(--pv-warning-soft);
    color: var(--pv-warning-text);
  }

  .status-badge.failed,
  .status-badge.not_installed {
    background: var(--pv-danger-soft);
    color: var(--pv-danger-text);
  }

  .menu-wrap {
    position: relative;
    display: inline-flex;
  }

  .menu {
    position: absolute;
    top: calc(100% + var(--pv-space-1));
    right: 0;
    z-index: var(--pv-z-dropdown);
    display: flex;
    flex-direction: column;
    min-width: 12rem;
    padding: var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
  }

  .menu button[role="menuitem"],
  .preset-name {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    text-align: left;
    white-space: nowrap;
    cursor: default;
  }

  .menu button[role="menuitem"]:hover,
  .menu button[role="menuitem"]:focus-visible,
  .preset-name:hover {
    background: var(--pv-control-bg-active);
    outline: none;
  }

  .grow {
    flex: 1;
  }

  .menu hr {
    width: 100%;
    margin: var(--pv-space-1) 0;
    border: none;
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .submenu {
    display: flex;
    flex-direction: column;
    margin: var(--pv-space-half) 0 var(--pv-space-half) var(--pv-space-3);
    padding-left: var(--pv-space-1);
    border-left: var(--pv-border-width) solid var(--pv-border);
  }

  .preset-empty {
    padding: var(--pv-space-1) var(--pv-space-2);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }

  .preset-row {
    display: flex;
    align-items: center;
  }

  .preset-name {
    flex: 1;
  }

  .preset-delete {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--pv-control-h-sm);
    height: var(--pv-control-h-sm);
    padding: 0;
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-tertiary);
    cursor: default;
  }

  .preset-delete:hover {
    background: var(--pv-danger-soft);
    color: var(--pv-danger-text);
  }

  .save-form {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    padding: var(--pv-space-2);
  }

  .save-form input[type="text"] {
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
  }

  .checkbox-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .checkbox-row input {
    accent-color: var(--pv-accent);
  }

  .save-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--pv-space-2);
  }

  .status-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    padding: 0 var(--pv-space-3) var(--pv-space-3);
  }

  .status-message {
    flex: 1;
    margin: 0;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .slot[data-status="failed"] .status-message,
  .slot[data-status="not_installed"] .status-message {
    color: var(--pv-danger-text);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    padding: var(--pv-space-1) var(--pv-space-3) var(--pv-space-3);
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
    padding-top: var(--pv-space-2);
    transition: opacity var(--pv-duration-fast) var(--pv-ease-standard);
  }
</style>
