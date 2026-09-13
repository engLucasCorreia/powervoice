<script lang="ts">
  import type { BitDepth } from "../ipc/bindings";
  import { t } from "../i18n";
  import { cancelSaveAsPrompt, confirmSaveAsPrompt, documentState } from "./document.svelte";

  /**
   * Save As bit-depth prompt (ticket: "Save/Save As with bit-depth choice"). Confirming shows the
   * native save dialog (`tauri-plugin-dialog`) and, if a path is chosen, saves.
   */
  const doc = documentState();
  let bits = $state<BitDepth>("24");
  let busy = $state(false);

  $effect(() => {
    if (doc.saveAsPrompt) {
      bits = doc.saveAsPrompt.defaultBits;
    }
  });

  const BIT_DEPTHS: BitDepth[] = ["16", "24", "32f"];

  async function confirm(): Promise<void> {
    busy = true;
    try {
      await confirmSaveAsPrompt(bits);
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
        <legend>{t("dialog.save_as.bit_depth")}</legend>
        {#each BIT_DEPTHS as depth (depth)}
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
