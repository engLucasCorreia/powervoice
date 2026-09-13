<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n";
  import { transportState } from "../state/transport.svelte";
  import AddModuleMenu from "./AddModuleMenu.svelte";
  import { addModule, loadRack, moveSlot, rackState, setAb } from "./rack.svelte";
  import RackSlot from "./RackSlot.svelte";

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

<aside class="rack" data-testid="rack">
  <header class="rack-header">
    <h2>{t("panel.rack.title")}</h2>
    <button
      type="button"
      class="ab"
      class:on={rs.state.ab}
      aria-pressed={rs.state.ab}
      data-testid="rack-ab"
      onclick={() => void setAb(!rs.state.ab)}
    >
      {t("rack.ab")}
    </button>
  </header>
  {#if rs.state.ab}
    <p class="ab-badge" data-testid="rack-ab-badge">{t("rack.ab_badge")}</p>
  {/if}
  {#if latencyLabel}
    <p class="latency" data-testid="rack-latency">{latencyLabel}</p>
  {/if}
  <AddModuleMenu
    modules={rs.modules}
    disabled={rs.state.slots.length >= MAX_SLOTS}
    onselect={(id) => void addModule(id, rs.state.slots.length)}
  />
  <div class="slots">
    {#if rs.loading}
      <p class="empty">{t("rack.loading")}</p>
    {:else if rs.unavailable}
      <p class="empty" data-testid="rack-unavailable">{t("error.rack_unavailable")}</p>
    {:else if rs.state.slots.length === 0}
      <p class="empty">{t("rack.empty")}</p>
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
</aside>

<style>
  .rack {
    background: var(--surface-panel);
    border-left: 1px solid var(--surface-border);
    padding: 0.75rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .rack-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h2 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin: 0;
  }

  .ab {
    background: var(--surface-panel-raised);
    color: var(--text-secondary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.15rem 0.5rem;
    font-size: 0.75rem;
  }

  .ab.on {
    background: var(--accent);
    color: var(--text-on-accent);
    border-color: var(--accent);
  }

  .ab-badge {
    margin: 0;
    color: var(--meter-yellow);
    font-size: 0.75rem;
  }

  .latency {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .slots {
    margin-top: 0.3rem;
  }

  .empty {
    color: var(--text-secondary);
    font-size: 0.8rem;
  }
</style>
