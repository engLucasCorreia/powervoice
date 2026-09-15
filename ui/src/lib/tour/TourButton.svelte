<script lang="ts">
  import { t } from "../i18n";
  import { IconButton } from "../ui";
  import { startTour } from "./tour.svelte";
  import { TOURS, type TourId } from "./tours";

  /** The "?" in a panel header (T-709): starts that panel's short tour. */
  let { tour, size = "sm" }: { tour: TourId; size?: "sm" | "md" } = $props();

  const label = $derived(t("tour.button_label", { tour: t(TOURS[tour].nameKey) }));
</script>

<IconButton icon="help" {size} {label} testid={`tour-start-${tour}`} onclick={() => startTour(tour)} />
