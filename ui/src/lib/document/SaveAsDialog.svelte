<script lang="ts">
  import type { BitDepth, SaveContainerDto, SaveDitherPref } from "../ipc/bindings";
  import { t } from "../i18n";
  import { cancelSaveAsPrompt, confirmSaveAsPrompt, documentState } from "./document.svelte";

  /**
   * Save As format/bit-depth/dither prompt (SPEC-005 §2.7: WAV 16/24/32-bit float, or FLAC
   * 16/24 — T-209 adds the format row, H-20 the Dither row; S1-03 had bit depth only). Confirming
   * shows the native save dialog (`tauri-plugin-dialog`) and, if a path is chosen, saves (running
   * the clip and multichannel-source prompts if they apply).
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
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="save-as-title"
      data-testid="save-as-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="save-as-title">{t("dialog.save_as.title")}</h2>
      <fieldset>
        <legend>{t("dialog.save_as.format")}</legend>
        {#each CONTAINERS as option (option)}
          <label>
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
      </fieldset>
      <fieldset>
        <legend>{t("dialog.save_as.bit_depth")}</legend>
        {#each BIT_DEPTHS[container] as depth (depth)}
          <label>
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
      </fieldset>
      {#if showsDither}
        <fieldset>
          <legend>{t("dialog.save_as.dither")}</legend>
          {#each DITHER_MODES as mode (mode)}
            <label>
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
        </fieldset>
      {/if}
      <div class="actions">
        <button type="button" data-testid="save-as-cancel" onclick={cancelSaveAsPrompt}>
          {t("dialog.save_as.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="save-as-choose"
          disabled={busy}
          onclick={() => void confirm()}
        >
          {t("dialog.save_as.choose_location")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 1000;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 24rem;
    max-width: 90vw;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
  }

  legend {
    color: var(--text-secondary);
    padding: 0 0.25rem;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button.primary {
    border-color: var(--accent);
    color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
