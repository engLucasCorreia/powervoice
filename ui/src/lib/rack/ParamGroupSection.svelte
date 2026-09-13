<script lang="ts">
  import type { ParamGroupDto, ParamInfoDto, RackSlotDto } from "../ipc/bindings";
  import { localized } from "./localized";
  import ParamControl from "./ParamControl.svelte";
  import { setParamText } from "./rack.svelte";

  /**
   * One parameter-group section (SPEC-012 §2.6): a header toggle when the group has an
   * `enable_param`, one nesting level flattened into "Parent / Child" (deeper groups don't occur
   * in the built-ins yet), collapse state, and its body params dimmed (not disabled) while off.
   * `group === null` renders the ungrouped params with no header (SPEC-012 §2.6 "ungrouped
   * parameters come first").
   */
  let {
    slotIndex,
    rackSlot,
    group,
    groupsByKey,
    params,
  }: {
    slotIndex: number;
    rackSlot: RackSlotDto;
    group: ParamGroupDto | null;
    groupsByKey: Map<number, ParamGroupDto>;
    params: ParamInfoDto[];
  } = $props();

  // svelte-ignore state_referenced_locally -- intentional: `group` is a fixed identity per
  // instance (each `{#each ... (group.id)}` block key is its own component), so this is the
  // group's declared initial collapse state, not a value meant to track later prop changes.
  let collapsed = $state(group?.collapsed_by_default ?? false);

  const title = $derived.by(() => {
    if (!group) {
      return "";
    }
    const parent = group.parent !== null ? groupsByKey.get(group.parent) : null;
    return parent ? `${localized(parent.name)} / ${localized(group.name)}` : localized(group.name);
  });

  const enableParam = $derived(
    group?.enable_param !== null && group?.enable_param !== undefined
      ? rackSlot.params.find((p) => p.id === group.enable_param)
      : undefined,
  );
  const enableValue = $derived(
    enableParam ? rackSlot.values.find((v) => v.id === enableParam.id) : undefined,
  );
  const enabled = $derived(enableValue ? enableValue.value >= 0.5 : true);
  const bodyParams = $derived(params.filter((p) => p.id !== group?.enable_param));

  function toggleEnable(): void {
    if (!enableParam) {
      return;
    }
    void setParamText(slotIndex, enableParam.id, enabled ? "0" : "1");
  }

  function valueOf(id: number) {
    return rackSlot.values.find((v) => v.id === id);
  }
</script>

{#if group}
  <section class="group" data-testid="param-group" data-key={group.key}>
    <header>
      {#if enableParam}
        <input
          type="checkbox"
          data-testid="param-group-enable"
          checked={enabled}
          onchange={toggleEnable}
        />
      {/if}
      <button type="button" class="title" onclick={() => (collapsed = !collapsed)}>
        <span class="arrow">{collapsed ? "▸" : "▾"}</span>
        {title}
      </button>
    </header>
    {#if !collapsed}
      <div class="body" class:dimmed={!enabled}>
        {#each bodyParams as p (p.id)}
          <ParamControl slot={slotIndex} param={p} value={valueOf(p.id)} />
        {/each}
      </div>
    {/if}
  </section>
{:else}
  <div class="ungrouped">
    {#each bodyParams as p (p.id)}
      <ParamControl slot={slotIndex} param={p} value={valueOf(p.id)} />
    {/each}
  </div>
{/if}

<style>
  .group {
    border-top: 1px solid var(--surface-border);
    padding-top: 0.3rem;
    margin-top: 0.3rem;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .title {
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: 0.8rem;
    font-weight: 600;
    padding: 0.1rem 0;
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .arrow {
    color: var(--text-secondary);
    width: 0.8rem;
  }

  .body.dimmed {
    opacity: 0.5;
  }
</style>
