<script lang="ts">
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
    <button
      type="button"
      class="power"
      class:on={!slot.bypass}
      aria-pressed={!slot.bypass}
      title={t("rack.slot.bypass")}
      data-testid="rack-slot-bypass"
      onclick={() => void setBypass(index, !slot.bypass)}
    >
      ⏻
    </button>
    <button
      type="button"
      class="collapse"
      title={collapsed ? t("rack.slot.expand") : t("rack.slot.collapse")}
      onclick={() => (collapsed = !collapsed)}
    >
      {collapsed ? "▸" : "▾"}
    </button>
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
      <button
        type="button"
        class="menu-button"
        aria-label={t("rack.slot.menu")}
        data-testid="rack-slot-menu"
        onclick={() => {
          menuOpen = !menuOpen;
          if (!menuOpen) {
            closeMenu();
          }
        }}
      >
        ⋯
      </button>
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
              {t("rack.slot.menu.presets")} ▸
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
                        >
                          ×
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
                      <button
                        type="button"
                        data-testid="rack-slot-preset-save-confirm"
                        onclick={() => void confirmSavePreset()}
                      >
                        {t("rack.slot.preset.save_button")}
                      </button>
                      <button type="button" onclick={() => (savingPreset = false)}>
                        {t("rack.slot.preset.cancel_button")}
                      </button>
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
  {#if slot.status.kind !== "active"}
    <div class="status-row">
      <p class="status-message" data-testid="rack-slot-status">{slot.status.message}</p>
      {#if canRetry}
        <button type="button" class="retry" data-testid="rack-slot-retry" onclick={() => void restartSlot(index)}>
          {t("rack.slot.retry")}
        </button>
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
  .slot {
    background: var(--surface-panel-raised);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    margin-bottom: 0.4rem;
    padding: 0.4rem 0.5rem;
  }

  .slot.drag-over {
    border-color: var(--accent);
  }

  .slot.bypassed {
    opacity: 0.6;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .power {
    background: var(--surface-inset);
    color: var(--text-disabled);
    border: 1px solid var(--surface-border);
    border-radius: 50%;
    width: 1.4rem;
    height: 1.4rem;
    line-height: 1;
    padding: 0;
  }

  .power.on {
    color: var(--meter-green);
  }

  .collapse {
    background: transparent;
    border: none;
    color: var(--text-secondary);
  }

  .name {
    font-weight: 600;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .latency {
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .menu-wrap {
    position: relative;
  }

  .menu-button {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 0.1rem 0.3rem;
  }

  .menu {
    position: absolute;
    right: 0;
    top: 100%;
    z-index: 10;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    min-width: 8rem;
  }

  .menu button {
    background: transparent;
    border: none;
    color: var(--text-primary);
    text-align: left;
    padding: 0.3rem 0.6rem;
  }

  .menu button:hover {
    background: var(--surface-panel-raised);
  }

  .menu hr {
    border: none;
    border-top: 1px solid var(--surface-border);
    margin: 0.2rem 0;
    width: 100%;
  }

  .submenu {
    display: flex;
    flex-direction: column;
    padding-left: 0.4rem;
    border-left: 2px solid var(--surface-border);
    margin: 0.15rem 0 0.15rem 0.6rem;
  }

  .preset-empty {
    color: var(--text-secondary);
    font-size: 0.8rem;
    padding: 0.2rem 0.6rem;
  }

  .preset-row {
    display: flex;
    align-items: center;
  }

  .preset-name {
    flex: 1;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text-primary);
    padding: 0.3rem 0.6rem;
  }

  .preset-name:hover {
    background: var(--surface-panel-raised);
  }

  .preset-delete {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 0 0.4rem;
  }

  .preset-delete:hover {
    color: var(--meter-yellow);
  }

  .save-form {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.3rem 0.6rem;
  }

  .save-form input[type="text"] {
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    color: var(--text-primary);
    border-radius: 3px;
    padding: 0.2rem 0.4rem;
  }

  .checkbox-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .save-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
  }

  .status-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .retry {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.1rem 0.6rem;
    flex: none;
  }

  .status-badge {
    font-size: 0.7rem;
    border: 1px solid var(--surface-border);
    border-radius: 999px;
    padding: 0 0.45rem;
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .status-badge.failed,
  .status-badge.not_installed {
    color: var(--text-primary);
    border-color: currentColor;
  }

  .status-message {
    color: var(--meter-yellow);
    font-size: 0.8rem;
    margin: 0.3rem 0 0;
  }

  .body {
    margin-top: 0.3rem;
  }
</style>
