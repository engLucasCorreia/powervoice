<script lang="ts">
  import { formatBytes } from "../recovery/format";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { t } from "../i18n";
  import { closePreferences, preferencesState } from "./preferences.svelte";

  /**
   * H-17 item 5: a small, reusable Preferences dialog — Edit → Preferences… (File → Preferences…
   * on macOS). Only a Storage section today ("Memory for audio", SPEC-004 §2.4/§3: the backend
   * already applies a change live, no restart); later tickets add their own `<section>` here
   * (H-21's Recording page) rather than building a separate dialog.
   */
  const pref = preferencesState();
  const settings = settingsState();

  const MIN_MIB = 256;
  const MAX_MIB = 16_384;
  const STEP_MIB = 256;

  // A local draft so the slider tracks the drag smoothly; only `change` (drag end / arrow key)
  // actually saves, so dragging doesn't flood the backend with settings writes.
  let draftMib = $state(MIN_MIB);
  $effect(() => {
    const current = settings.current?.memory_budget_mib;
    if (current !== undefined) {
      draftMib = current;
    }
  });

  function onInput(event: Event): void {
    draftMib = Number((event.currentTarget as HTMLInputElement).value);
  }

  function onChange(): void {
    void saveSettings({ memory_budget_mib: draftMib });
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closePreferences();
    }
  }
</script>

{#if pref.open}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="preferences-title"
      data-testid="preferences-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="preferences-title">{t("preferences.title")}</h2>

      <section>
        <h3>{t("preferences.section.storage")}</h3>
        <label class="field" for="preferences-memory-budget">
          <span>{t("preferences.memory_budget")}</span>
          <input
            id="preferences-memory-budget"
            type="range"
            min={MIN_MIB}
            max={MAX_MIB}
            step={STEP_MIB}
            data-testid="preferences-memory-budget"
            value={draftMib}
            oninput={onInput}
            onchange={onChange}
          />
          <span class="value" data-testid="preferences-memory-budget-value">
            {formatBytes(draftMib * 1024 * 1024)}
          </span>
        </label>
        <p class="hint">{t("preferences.memory_budget.hint")}</p>
      </section>

      <div class="footer">
        <button type="button" data-testid="preferences-close" onclick={closePreferences}>
          {t("preferences.close")}
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
    z-index: 900;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    width: min(28rem, 90vw);
    max-height: 85vh;
    overflow: auto;
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

  h3 {
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
    color: var(--text-secondary);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .field input[type="range"] {
    width: 100%;
  }

  .value {
    color: var(--text-secondary);
    font-size: 0.85em;
  }

  .hint {
    margin: 0.35rem 0 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }
</style>
