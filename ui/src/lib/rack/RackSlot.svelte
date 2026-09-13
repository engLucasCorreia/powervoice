<script lang="ts">
  import { t } from "../i18n";
  import type { ParamInfoDto, RackSlotDto } from "../ipc/bindings";
  import NoiseReductionSection from "./NoiseReductionSection.svelte";
  import ParamGroupSection from "./ParamGroupSection.svelte";
  import { noteSlotFocused, removeSlot, restartSlot, setBypass } from "./rack.svelte";

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

  function closeMenu(): void {
    menuOpen = false;
  }
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
    {#if latencyLabel}
      <span class="latency">{latencyLabel}</span>
    {/if}
    <div class="menu-wrap">
      <button
        type="button"
        class="menu-button"
        aria-label={t("rack.slot.menu")}
        data-testid="rack-slot-menu"
        onclick={() => (menuOpen = !menuOpen)}
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
        </div>
      {/if}
    </div>
  </header>
  {#if slot.status.kind !== "active"}
    <p class="status-message" data-testid="rack-slot-status">{slot.status.message}</p>
  {/if}
  {#if !collapsed && slot.status.kind === "active"}
    <div class="body">
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

  .status-message {
    color: var(--meter-yellow);
    font-size: 0.8rem;
    margin: 0.3rem 0 0;
  }

  .body {
    margin-top: 0.3rem;
  }
</style>
