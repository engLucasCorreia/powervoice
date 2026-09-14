<script lang="ts">
  import type { BitDepth, SaveContainerDto, SaveDitherPref } from "../ipc/bindings";
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
  import { cancelSaveAsPrompt, confirmSaveAsPrompt, documentState } from "./document.svelte";

  /**
   * Save As format/bit-depth/dither prompt (SPEC-005 §2.7: WAV 16/24/32-bit float, or FLAC
   * 16/24 — T-209 adds the format row, H-20 the Dither row; S1-03 had bit depth only). Confirming
   * shows the native save dialog (`tauri-plugin-dialog`) and, if a path is chosen, saves (running
   * the clip and multichannel-source prompts if they apply). H-25: Dialog shell; each choice is a
   * compact row of radios.
   */
  const doc = documentState();
  let container = $state<SaveContainerDto>("wav");
  let bits = $state<BitDepth>("24");
  let dither = $state<SaveDitherPref>("tpdf");
  let busy = $state(false);

  $effect(() => {
    if (doc.saveAsPrompt) {
      container = doc.saveAsPrompt.defaultContainer;
      bits = doc.saveAsPrompt.defaultBits;
      dither = doc.saveAsPrompt.defaultDither;
    }
  });

  const CONTAINERS: SaveContainerDto[] = ["wav", "flac"];
  const BIT_DEPTHS: Record<SaveContainerDto, BitDepth[]> = {
    wav: ["16", "24", "32f"],
    flac: ["16", "24"],
  };
  const DITHER_MODES: SaveDitherPref[] = ["tpdf", "none"];
  // SPEC-005 §2.7: the Dither row only applies to integer targets — 32-bit float never dithers.
  const showsDither = $derived(bits !== "32f");

  // FLAC has no 32-bit float (SPEC-005 §2.6/§2.11) — switching format away from a bit depth it
  // doesn't support falls back to 24-bit.
  function chooseContainer(next: SaveContainerDto): void {
    container = next;
    if (!BIT_DEPTHS[next].includes(bits)) {
      bits = "24";
    }
  }

  async function confirm(): Promise<void> {
    busy = true;
    try {
      await confirmSaveAsPrompt(container, bits, dither);
    } finally {
      busy = false;
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelSaveAsPrompt();
    }
  }
</script>

{#if doc.saveAsPrompt}
  <Dialog title={t("dialog.save_as.title")} titleId="save-as-title" testid="save-as-dialog" onkeydown={onKeydown}>
    <fieldset>
      <legend>{t("dialog.save_as.format")}</legend>
      <div class="options">
        {#each CONTAINERS as option (option)}
          <label class="option">
            <input
              type="radio"
              name="save-as-format"
              value={option}
              checked={container === option}
              onchange={() => chooseContainer(option)}
            />
            {t(`dialog.save_as.format.${option}` as const)}
          </label>
        {/each}
      </div>
    </fieldset>
    <fieldset>
      <legend>{t("dialog.save_as.bit_depth")}</legend>
      <div class="options">
        {#each BIT_DEPTHS[container] as depth (depth)}
          <label class="option">
            <input
              type="radio"
              name="save-as-bits"
              value={depth}
              checked={bits === depth}
              onchange={() => (bits = depth)}
            />
            {t(`dialog.save_as.bit_depth.${depth}` as const)}
          </label>
        {/each}
      </div>
    </fieldset>
    {#if showsDither}
      <fieldset>
        <legend>{t("dialog.save_as.dither")}</legend>
        <div class="options">
          {#each DITHER_MODES as mode (mode)}
            <label class="option">
              <input
                type="radio"
                name="save-as-dither"
                value={mode}
                checked={dither === mode}
                onchange={() => (dither = mode)}
              />
              {t(`dialog.save_as.dither.${mode}` as const)}
            </label>
          {/each}
        </div>
      </fieldset>
    {/if}
    {#snippet footer()}
      <Button testid="save-as-cancel" onclick={cancelSaveAsPrompt}>
        {t("dialog.save_as.cancel")}
      </Button>
      <Button variant="primary" icon="save" testid="save-as-choose" loading={busy} onclick={() => void confirm()}>
        {t("dialog.save_as.choose_location")}
      </Button>
    {/snippet}
  </Dialog>
{/if}
