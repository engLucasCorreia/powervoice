<script lang="ts">
  import { t } from "../i18n";
  import type { ParamGroupDto, ParamInfoDto, RackSlotDto } from "../ipc/bindings";
  import ParamGroupSection from "../rack/ParamGroupSection.svelte";
  import { slotTelemetry } from "../rack/rack.svelte";
  import TransferGraph from "../transfer/TransferGraph.svelte";
  import { operatingPointOf } from "./operatingPoint";

  /**
   * The custom Dynamics panel (H-77, SPEC-016 §2.6, ADR-005 §13): the global row (Detection,
   * Knee, Look-ahead, plus the latency the look-ahead costs), then the transfer graph with the
   * live operating point, then the four sections in **processing** order — AutoGate, Expander,
   * Compressor, Limiter — each with its enable toggle, its gain-reduction meter and, for the
   * AutoGate, its open lamp (those come from the group headers' generic telemetry widgets).
   *
   * Built only on the schema, Telemetry and `TransferCurve` (ADR-005 §13): the only thing it
   * knows that the generic panel doesn't is that the global row comes *above* the graph and
   * that the graph's operating point needs the compressor's makeup — a parameter, which no
   * telemetry channel carries.
   */
  let {
    slotIndex,
    slot,
    rateHz,
    visibleGroups,
    groupsByKey,
    ungrouped,
    shown,
  }: {
    slotIndex: number;
    slot: RackSlotDto;
    rateHz: number;
    /** The groups the slot body shows, in `groups()` order (= processing order, SPEC-016 §2.2). */
    visibleGroups: ParamGroupDto[];
    groupsByKey: Map<number, ParamGroupDto>;
    /** The global row's parameters (SPEC-012 §2.6: ungrouped parameters come first). */
    ungrouped: ParamInfoDto[];
    shown: (p: ParamInfoDto) => boolean;
  } = $props();

  const latencyMs = $derived((slot.latency_samples / Math.max(1, rateHz)) * 1000);
  const meterValues = $derived(slotTelemetry(slot.uid));
  const operatingPoint = $derived(operatingPointOf(slot, meterValues));
</script>

<div class="dynamics" data-testid="dynamics-panel">
  {#if ungrouped.length > 0}
    <section class="global" data-testid="dynamics-global">
      <ParamGroupSection {slotIndex} rackSlot={slot} group={null} {groupsByKey} params={ungrouped} />
      <!-- SPEC-016 §2.6: the look-ahead's cost, from the slot's own reported latency, and only
           while there is one. -->
      {#if slot.latency_samples > 0}
        <p class="latency" data-testid="dynamics-latency">
          {t("dynamics.latency", { ms: latencyMs.toFixed(1) })}
        </p>
      {/if}
    </section>
  {/if}
  <TransferGraph {slotIndex} rackSlot={slot} {operatingPoint} />
  {#each visibleGroups as group (group.id)}
    <ParamGroupSection
      {slotIndex}
      rackSlot={slot}
      {group}
      {groupsByKey}
      params={slot.params.filter((p) => p.group === group.id && shown(p))}
    />
  {/each}
</div>

<style>
  .dynamics {
    display: contents;
  }

  .global {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
  }

  .latency {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
  }
</style>
