<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import { Button, Menu } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import type { ModuleDescriptorDto } from "../ipc/bindings";
  import { localized } from "./localized";

  /** The Add-module menu (SPEC-012 §2.1): the registry's modules grouped by feature (EQ,
   * dynamics, restoration, utility, …); installed CLAP effects (`clap:*`, T-803), VST3 effects
   * (`vst3:*`, T-806) and LV2 effects (`lv2:*`, T-807) in their own "Plugins (CLAP)" /
   * "Plugins (VST3)" / "Plugins (LV2)" groups after the built-in categories. H-26: on the shared menu (group
   * headings, keyboard, typeahead, viewport clamping), as wide as its button. */
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
  let trigger: HTMLButtonElement | undefined = $state();

  const CATEGORY_ORDER = [
    "eq",
    "restoration",
    "dynamics",
    "mastering",
    "utility",
    "analyzer",
  ] as const;
  type Category = (typeof CATEGORY_ORDER)[number] | "other" | "plugins_clap" | "plugins_vst3" | "plugins_lv2";
  const CATEGORY_FEATURES: Record<(typeof CATEGORY_ORDER)[number], string[]> = {
    eq: ["equalizer", "filter"],
    restoration: ["restoration"],
    dynamics: ["compressor", "expander", "gate", "limiter"],
    mastering: ["mastering"],
    utility: ["utility"],
    analyzer: ["analyzer"],
  };

  function categoryOf(m: ModuleDescriptorDto): Category {
    if (m.id.startsWith("clap:")) {
      return "plugins_clap";
    }
    if (m.id.startsWith("vst3:")) {
      return "plugins_vst3";
    }
    if (m.id.startsWith("lv2:")) {
      return "plugins_lv2";
    }
    for (const cat of CATEGORY_ORDER) {
      if (CATEGORY_FEATURES[cat].some((f) => m.features.includes(f))) {
        return cat;
      }
    }
    return "other";
  }

  // A fixed category order (EQ, restoration, dynamics, ...), not registry order — the menu's
  // layout doesn't depend on which module happens to be registered first.
  const ALL_CATEGORIES: readonly Category[] = [
    ...CATEGORY_ORDER,
    "other",
    "plugins_clap",
    "plugins_vst3",
    "plugins_lv2",
  ];

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

  const items = $derived.by((): MenuEntry[] =>
    groups.flatMap(([cat, mods], i): MenuEntry[] => [
      ...(i > 0 ? [{ kind: "separator" as const, id: `sep-${cat}` }] : []),
      { kind: "heading", id: `heading-${cat}`, label: tDynamic(`rack.category.${cat}`) },
      ...mods.map(
        (m): MenuEntry => ({
          kind: "item",
          id: m.id,
          label: localized(m.name),
          testid: "rack-add-item",
          attrs: { "data-module-id": m.id },
          onselect: () => onselect(m.id),
        }),
      ),
    ]),
  );
</script>

<div class="add-module" data-tour="rack-add">
  <Button
    icon="add"
    iconEnd="chevronDown"
    testid="rack-add"
    {disabled}
    title={disabled ? t("rack.max_slots") : ""}
    aria-haspopup="menu"
    aria-expanded={open}
    bind:element={trigger}
    onclick={() => {
      if (!disabled) open = !open;
    }}
  >
    {t("rack.add")}
  </Button>
  <Menu
    {open}
    anchor={trigger}
    {items}
    label={t("rack.add")}
    testid="rack-add-menu"
    minWidth="anchor"
    onclose={() => (open = false)}
  />
</div>

<style>
  .add-module > :global(.pv-button) {
    width: 100%;
    justify-content: flex-start;
  }

  .add-module > :global(.pv-button .label) {
    flex: 1;
    text-align: left;
  }
</style>
