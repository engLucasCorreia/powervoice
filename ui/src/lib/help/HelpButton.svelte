<script lang="ts">
  import { t } from "../i18n";
  import { IconButton } from "../ui";
  import { resolveHelpTopic } from "./content";
  import { openHelpCentre } from "./helpCentre.svelte";

  /**
   * H-107: the "?" that opens the Help Centre straight to one topic — next to `TourButton`
   * (`icon="help"`, starts a *tour*) in the panels named in the ticket (the Plugin Manager
   * especially): Rack, Loudness, Noise Reduction, Punch & pre-roll. Deliberately a different icon
   * from `TourButton` so the two don't read as the same control doing the same thing. `section` can
   * name either a top-level section or one of its nested subheadings (`resolveHelpTopic`).
   */
  let { doc, section, size = "sm" }: { doc: string; section: string; size?: "sm" | "md" } = $props();

  const topic = $derived(resolveHelpTopic(doc, section)?.title ?? "");
  const label = $derived(t("help.button_label", { topic }));
</script>

<IconButton icon="info" {size} {label} testid={`help-open-${doc}-${section}`} onclick={() => openHelpCentre({ doc, section })} />
