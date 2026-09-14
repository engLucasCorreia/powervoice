<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import { Button } from "../ui";
  import type { ModuleDescriptorDto } from "../ipc/bindings";
  import { localized } from "./localized";

  /** The Add-module menu (SPEC-012 §2.1): the registry's modules grouped by feature (EQ,
   * dynamics, restoration, utility, …). */
  let {
    modules,
    disabled,
    onselect,
  }: {
    modules: ModuleDescriptorDto[];
    disabled: boolean;
    onselect: (moduleId: string) => void;
  } = $props();

  let open = $state(false);

  const CATEGORY_ORDER = [
    "eq",
    "restoration",
    "dynamics",
    "mastering",
    "utility",
    "analyzer",
  ] as const;
  type Category = (typeof CATEGORY_ORDER)[number] | "other";
  const CATEGORY_FEATURES: Record<(typeof CATEGORY_ORDER)[number], string[]> = {
    eq: ["equalizer", "filter"],
    restoration: ["restoration"],
    dynamics: ["compressor", "expander", "gate", "limiter"],
    mastering: ["mastering"],
    utility: ["utility"],
    analyzer: ["analyzer"],
  };

  function categoryOf(m: ModuleDescriptorDto): Category {
    for (const cat of CATEGORY_ORDER) {
      if (CATEGORY_FEATURES[cat].some((f) => m.features.includes(f))) {
        return cat;
      }
    }
    return "other";
  }

  // A fixed category order (EQ, restoration, dynamics, ...), not registry order — the menu's
  // layout doesn't depend on which module happens to be registered first.
  const ALL_CATEGORIES: readonly Category[] = [...CATEGORY_ORDER, "other"];

  const groups = $derived.by(() => {
    const byCat = new Map<Category, ModuleDescriptorDto[]>();
    for (const m of modules) {
      const cat = categoryOf(m);
      const list = byCat.get(cat) ?? [];
      list.push(m);
      byCat.set(cat, list);
    }
    return ALL_CATEGORIES.filter((cat) => byCat.has(cat)).map((cat) => [cat, byCat.get(cat)!] as const);
  });

  function toggle(): void {
    if (!disabled) {
      open = !open;
    }
  }

  function pick(id: string): void {
    open = false;
    onselect(id);
  }

  function onWindowClick(event: MouseEvent): void {
    if (open && !(event.target as HTMLElement).closest(".add-module")) {
      open = false;
    }
  }
</script>

<svelte:window onclick={onWindowClick} />

<div class="add-module">
  <Button
    icon="add"
    iconEnd="chevronDown"
    testid="rack-add"
    {disabled}
    title={disabled ? t("rack.max_slots") : ""}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={toggle}
  >
    {t("rack.add")}
  </Button>
  {#if open}
    <div class="menu" role="menu" data-testid="rack-add-menu">
      {#each groups as [cat, mods] (cat)}
        <div class="group">
          <div class="group-title">{tDynamic(`rack.category.${cat}`)}</div>
          {#each mods as m (m.id)}
            <button
              type="button"
              role="menuitem"
              class="item"
              data-testid="rack-add-item"
              data-module-id={m.id}
              onclick={() => pick(m.id)}
            >
              {localized(m.name)}
            </button>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .add-module {
    position: relative;
  }

  .add-module > :global(.pv-button) {
    width: 100%;
    justify-content: flex-start;
  }

  .add-module > :global(.pv-button .label) {
    flex: 1;
    text-align: left;
  }

  .menu {
    position: absolute;
    top: calc(100% + var(--pv-space-1));
    left: 0;
    right: 0;
    z-index: var(--pv-z-dropdown);
    max-height: 22rem;
    padding: var(--pv-space-1);
    overflow-y: auto;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
  }

  .group + .group {
    margin-top: var(--pv-space-1);
    padding-top: var(--pv-space-1);
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .group-title {
    padding: var(--pv-space-1) var(--pv-space-2);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-semibold);
  }

  .item {
    display: flex;
    align-items: center;
    width: 100%;
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    text-align: left;
    cursor: default;
  }

  .item:hover,
  .item:focus-visible {
    background: var(--pv-control-bg-active);
    outline: none;
  }
</style>
