<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n";
  import { Button, EmptyState, Icon, PanelHeader } from "../ui";
  import { refreshPlugins } from "../plugins/plugins.svelte";
  import { transportState } from "../state/transport.svelte";
  import AddModuleMenu from "./AddModuleMenu.svelte";
  import { addModule, loadRack, moveSlot, rackState, setAb } from "./rack.svelte";
  import RackSlot from "./RackSlot.svelte";
  import TourButton from "../tour/TourButton.svelte";
  import HelpButton from "../help/HelpButton.svelte";

  /**
   * The rack panel (right dock, SPEC-012 §2.1): header (A/B, latency), the Add-module menu, and
   * the slot list with drag-reorder.
   */
  const MAX_SLOTS = 16;

  const rs = rackState();
  const rateHz = $derived(transportState().state.doc_rate_hz || 48_000);
  const latencyLabel = $derived(
    rs.state.latency_samples > 0
      ? t("rack.latency", {
          ms: ((rs.state.latency_samples / rateHz) * 1000).toFixed(1),
          samples: rs.state.latency_samples,
        })
      : "",
  );

  // T-809 item 4: a sandboxed slot that just failed or restarted has been counted in the crash
  // store — reload the plugin list so its flagged affordance (and the manager) show it.
  const faultedSandboxSlots = $derived(
    rs.state.slots
      .filter((s) => s.sandboxed && (s.status.kind === "failed" || s.status.kind === "restarting"))
      .map((s) => s.uid)
      .join(","),
  );
  $effect(() => {
    if (faultedSandboxSlots) {
      void refreshPlugins();
    }
  });

  let dragIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  function onSlotDragStart(index: number): void {
    dragIndex = index;
  }

  function onSlotDragOver(index: number, event: DragEvent): void {
    event.preventDefault();
    dragOverIndex = index;
  }

  function onSlotDrop(index: number): void {
    const from = dragIndex;
    dragIndex = null;
    dragOverIndex = null;
    if (from !== null && from !== index) {
      void moveSlot(from, index);
    }
  }

  function onSlotDragEnd(): void {
    dragIndex = null;
    dragOverIndex = null;
  }

  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void loadRack().then((cleanup) => {
      if (disposed) {
        cleanup();
      } else {
        teardown = cleanup;
      }
    });
    return () => {
      disposed = true;
      teardown?.();
    };
  });
</script>

<aside class="rack" data-testid="rack" data-tour="rack">
  <PanelHeader title={t("panel.rack.title")}>
    {#snippet actions()}
      {#if latencyLabel}
        <span class="latency" data-testid="rack-latency">{latencyLabel}</span>
      {/if}
      <Button
        size="sm"
        variant="ghost"
        testid="rack-ab"
        data-tour="rack-ab"
        aria-pressed={rs.state.ab}
        onclick={() => void setAb(!rs.state.ab)}
      >
        {t("rack.ab")}
      </Button>
      <HelpButton doc="user-guide" section="cleaning-up-the-effects-rack" />
      <TourButton tour="rack" />
    {/snippet}
  </PanelHeader>
  <div class="content">
    {#if rs.state.ab}
      <p class="ab-badge" data-testid="rack-ab-badge"><Icon name="info" size="sm" />{t("rack.ab_badge")}</p>
    {/if}
    <AddModuleMenu
      modules={rs.modules}
      disabled={rs.state.slots.length >= MAX_SLOTS}
      onselect={(id) => void addModule(id, rs.state.slots.length)}
    />
    <div class="slots" data-tour="rack-slots">
      {#if rs.loading}
        <p class="note">{t("rack.loading")}</p>
      {:else if rs.unavailable}
        <p class="note" data-testid="rack-unavailable">{t("error.rack_unavailable")}</p>
      {:else if rs.state.slots.length === 0}
        <EmptyState icon="rack" title={t("rack.empty")} size="sm" level={3} />
      {:else}
        {#each rs.state.slots as slot, index (slot.uid)}
          <RackSlot
            {slot}
            {index}
            {rateHz}
            dragOver={dragOverIndex === index}
            ondragstart={onSlotDragStart}
            ondragover={onSlotDragOver}
            ondrop={onSlotDrop}
            ondragend={onSlotDragEnd}
          />
        {/each}
      {/if}
    </div>
  </div>
</aside>

<style>
  .rack {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--pv-bg-panel);
    font-family: var(--pv-font-sans);
  }

  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--pv-space-2);
    min-height: 0;
    padding: var(--pv-space-3);
    overflow-y: auto;
  }

  .latency {
    margin-right: var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .ab-badge {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    margin: 0;
    padding: var(--pv-space-2) var(--pv-space-3);
    border-radius: var(--pv-radius-md);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .ab-badge :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .slots {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
  }

  .note {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }
</style>
