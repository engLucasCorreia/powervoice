<script lang="ts">
  import { t, tDynamic } from "../i18n";
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
  <button type="button" data-testid="rack-add" {disabled} title={disabled ? t("rack.max_slots") : ""} onclick={toggle}>
    {t("rack.add")}
  </button>
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

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:disabled {
    color: var(--text-disabled);
    cursor: not-allowed;
  }

  .menu {
    position: absolute;
    left: 0;
    top: 100%;
    z-index: 20;
    margin-top: 0.2rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    min-width: 12rem;
    max-height: 20rem;
    overflow-y: auto;
  }

  .group-title {
    color: var(--text-secondary);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.3rem 0.6rem 0.1rem;
  }

  .item {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text-primary);
    padding: 0.3rem 0.6rem;
  }

  .item:hover {
    background: var(--surface-panel-raised);
  }
</style>
