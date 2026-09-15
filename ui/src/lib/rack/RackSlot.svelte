<script lang="ts">
  import { Button, Icon, IconButton, Menu } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import EqGraph from "../eq/EqGraph.svelte";
  import { t } from "../i18n";
  import type { ParamInfoDto, PresetEntryDto, RackSlotDto } from "../ipc/bindings";
  import GainReductionMeter from "./GainReductionMeter.svelte";
  import { localized } from "./localized";
  import NoiseReductionSection from "./NoiseReductionSection.svelte";
  import { openPluginManager, pluginCrashCount } from "../plugins/plugins.svelte";
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
  let menuTrigger: HTMLButtonElement | undefined = $state();

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
  // T-809 item 4: a plugin that has crashed at runtime (ADR-008 §5 "flagged") gets a small
  // warning key next to its name that opens the plugin manager on it.
  const crashCount = $derived(pluginCrashCount(slot.module_id));
  const flagLabel = $derived(
    crashCount === 1 ? t("rack.slot.flagged_once") : t("rack.slot.flagged", { count: crashCount }),
  );

  /** Retry = the rack's Restart of a failed slot (a sandboxed plugin is respawned with its
   * last committed state). A missing module can't be restarted (SPEC-012 §2.9). */
  const canRetry = $derived(slot.status.kind === "failed");

  // --- Presets (T-406, SPEC-012 §2.7) -----------------------------------------------------
  let presetEntries = $state<PresetEntryDto[] | null>(null);
  let savingPreset = $state(false);
  let presetName = $state("");
  let includeNoisePrint = $state(false);

  function closeMenu(): void {
    menuOpen = false;
    savingPreset = false;
  }

  async function openPresetsSubmenu(): Promise<void> {
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
    await loadModulePreset(
      index,
      slot.module_id,
      entry.is_factory ? { kind: "factory", key: entry.key } : { kind: "user", name: entry.key },
    );
  }

  async function deletePreset(entry: PresetEntryDto): Promise<void> {
    if (!slot.module_id) {
      return;
    }
    if (await deleteModulePreset(slot.module_id, entry.key)) {
      await refreshPresets();
    }
  }

  // H-26: the slot menu and its Presets submenu on the shared menu; the preset-name form is
  // inline content of the submenu.
  const presetItems = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [];
    if (presetEntries === null) {
      list.push({ kind: "note", id: "loading", label: "…" });
    } else if (presetEntries.length === 0 && !savingPreset) {
      list.push({ kind: "note", id: "none", label: t("rack.slot.preset.none") });
    } else {
      for (const entry of presetEntries) {
        list.push({
          kind: "item",
          id: `${entry.is_factory}:${entry.key}`,
          label: localized(entry.name),
          testid: `rack-slot-preset-${entry.key}`,
          onselect: () => void pickPreset(entry),
          trailing: entry.is_factory
            ? undefined
            : {
                icon: "delete",
                label: t("rack.slot.preset.delete"),
                testid: `rack-slot-preset-delete-${entry.key}`,
                onselect: () => void deletePreset(entry),
              },
        });
      }
    }
    list.push({ kind: "separator", id: "sep-save" });
    list.push(
      savingPreset
        ? { kind: "custom", id: "save-form", content: savePresetForm }
        : {
            kind: "item",
            id: "save-as",
            label: t("rack.slot.preset.save_as"),
            testid: "rack-slot-preset-save",
            keepOpen: true,
            onselect: startSavePreset,
          },
    );
    return list;
  });

  const menuItems = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [
      {
        kind: "item",
        id: "restart",
        label: t("rack.slot.menu.restart"),
        testid: "rack-slot-restart",
        onselect: () => void restartSlot(index),
      },
      {
        kind: "item",
        id: "remove",
        label: t("rack.slot.menu.remove"),
        testid: "rack-slot-remove",
        onselect: () => void removeSlot(index),
      },
    ];
    if (slot.module_id) {
      list.push(
        { kind: "separator", id: "sep-presets" },
        {
          kind: "submenu",
          id: "presets",
          label: t("rack.slot.menu.presets"),
          testid: "rack-slot-presets",
          menuTestid: "rack-slot-presets-submenu",
          minWidth: 208,
          onopen: () => void openPresetsSubmenu(),
          items: presetItems,
        },
        {
          kind: "item",
          id: "reset",
          label: t("rack.slot.menu.reset_default"),
          testid: "rack-slot-reset-default",
          onselect: () => void resetSlotToDefault(index),
        },
      );
    }
    return list;
  });

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

{#snippet savePresetForm()}
  <input
    type="text"
    placeholder={t("rack.slot.preset.name_placeholder")}
    aria-label={t("rack.slot.preset.name_placeholder")}
    data-testid="rack-slot-preset-name"
    bind:value={presetName}
    onkeydown={(e) => {
      if (e.key === "Enter") void confirmSavePreset();
    }}
    {@attach (node) => node.focus()}
  />
  {#if slot.noise_profile !== null}
    <label>
      <input type="checkbox" bind:checked={includeNoisePrint} />
      {t("rack.slot.preset.include_noise_print")}
    </label>
  {/if}
  <div class="actions">
    <Button size="sm" onclick={() => (savingPreset = false)}>
      {t("rack.slot.preset.cancel_button")}
    </Button>
    <Button size="sm" variant="primary" testid="rack-slot-preset-save-confirm" onclick={() => void confirmSavePreset()}>
      {t("rack.slot.preset.save_button")}
    </Button>
  </div>
{/snippet}

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
    {#if crashCount > 0}
      <span class="flag">
        <IconButton
          icon="warning"
          label={flagLabel}
          size="sm"
          testid="rack-slot-flagged"
          onclick={() => openPluginManager({ focus: slot.module_id })}
        />
      </span>
    {/if}
    {#if statusBadge}
      <span class="status-badge {statusBadge.kind}" data-testid="rack-slot-badge" data-badge={statusBadge.kind}
        >{statusBadge.label}</span
      >
    {/if}
    {#if latencyLabel}
      <span class="latency" title={latencyLabel}>{latencyLabel}</span>
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
    <IconButton
      icon="more"
      label={t("rack.slot.menu")}
      size="sm"
      testid="rack-slot-menu"
      aria-haspopup="menu"
      aria-expanded={menuOpen}
      bind:element={menuTrigger}
      onclick={() => (menuOpen ? closeMenu() : (menuOpen = true))}
    />
    <Menu
      open={menuOpen}
      anchor={menuTrigger}
      items={menuItems}
      label={t("rack.slot.menu")}
      testid="rack-slot-menu-popup"
      placement="bottom-end"
      onclose={closeMenu}
    />
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

  /* T-809: in a narrow rack the name keeps at least a few characters (a plugin slot also carries
     its status, a flagged key and a latency label); the latency text gives way first. */
  .name {
    flex: 1 1 auto;
    min-width: 4.5rem;
    margin-left: var(--pv-space-1);
    overflow: hidden;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* T-809: the flagged-plugin key keeps the warning colour at rest (it carries meaning, with its
     tooltip naming it), and the kit's hover/focus states. */
  .flag {
    display: inline-flex;
    color: var(--pv-warning-text);
  }

  .flag :global(.pv-icon-button) {
    color: inherit;
  }

  .latency {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
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
