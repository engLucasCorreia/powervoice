<script lang="ts">
  import { t } from "../i18n";
  import { recoveryState } from "../recovery/recovery.svelte";
  import { recordState } from "../state/record.svelte";
  import { Dialog, Icon, type IconName } from "../ui";
  import { canShowOffer } from "./progress";
  import { acceptWelcomeOffer, dismissWelcomeOffer, postponeWelcomeOffer, tourState } from "./tour.svelte";

  /**
   * The first-run Welcome tour offer (T-709): Start tour / Later / Don't show again. It waits for
   * the start-up crash-recovery check and never appears over the recovery dialog, while a take is
   * being recorded, or during a tour; `delayMs` lets the window settle first so it never flashes
   * up during start-up.
   */
  let { delayMs = 700 }: { delayMs?: number } = $props();

  const tours = tourState();
  const recovery = recoveryState();
  const rec = recordState();

  const eligible = $derived(
    canShowOffer({
      armed: tours.offerArmed,
      recoveryChecked: recovery.checked,
      recoveryOpen: recovery.mode !== null,
      recording: rec.state.recording || rec.state.finishing,
      tourActive: tours.active,
    }),
  );

  let ready = $state(false);
  $effect(() => {
    if (!eligible) {
      ready = false;
      return;
    }
    const timer = setTimeout(() => {
      ready = true;
    }, delayMs);
    return () => clearTimeout(timer);
  });

  /** The tour's story in four tiles: record → edit → clean up → deliver. */
  const STORY: readonly IconName[] = ["input", "waveform", "cleanup", "loudness"];

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      postponeWelcomeOffer();
    }
  }
</script>

{#if eligible && ready}
  <Dialog
    size="sm"
    title={t("tour.offer.title")}
    testid="tour-offer"
    onkeydown={onKeydown}
    actions={[
      { label: t("tour.offer.start"), role: "primary", testid: "tour-offer-start", onclick: acceptWelcomeOffer },
      { label: t("tour.offer.later"), role: "cancel", testid: "tour-offer-later", onclick: postponeWelcomeOffer },
      {
        label: t("tour.offer.never"),
        role: "utility",
        testid: "tour-offer-never",
        onclick: () => void dismissWelcomeOffer(),
      },
    ]}
  >
    <div class="story" aria-hidden="true">
      {#each STORY as icon, i (icon)}
        {#if i > 0}<span class="link"></span>{/if}
        <span class="tile"><Icon name={icon} size="md" /></span>
      {/each}
    </div>
    <p>{t("tour.offer.body")}</p>
    <p class="hint">{t("tour.offer.hint")}</p>
  </Dialog>
{/if}

<style>
  .story {
    display: flex;
    align-items: center;
    margin: var(--pv-space-1) 0 var(--pv-space-1);
  }

  .tile {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-control-h-lg);
    height: var(--pv-control-h-lg);
    border-radius: var(--pv-radius-md);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  .link {
    flex: 1;
    height: var(--pv-border-width);
    margin: 0 var(--pv-space-1);
    background: var(--pv-border);
  }
</style>
