<script lang="ts">
  import type { RecordOffsetEntry } from "../ipc/bindings";
  import { ROLL_MAX_S, XFADE_MAX_MS, offsetReadout } from "../record/punch";
  import { formatBytes } from "../recovery/format";
  import { recordState, refreshOffset, setRecordPrefs } from "../state/record.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { t, tDynamic } from "../i18n";
  import { closePreferences, preferencesState } from "./preferences.svelte";

  /**
   * H-17 item 5: a small, reusable Preferences dialog — Edit → Preferences… (File → Preferences…
   * on macOS). A Storage section ("Memory for audio", SPEC-004 §2.4/§3: the backend already
   * applies a change live, no restart); later tickets add their own `<section>` here rather than
   * building a separate dialog.
   *
   * H-21 item 6 (SPEC-022 §2.3, §2.13): the Recording section — the same Punch & pre-roll
   * preferences as the record panel (mode, punch on selection, pre-/post-roll, pre-roll at the
   * cursor, hear original, crossfade), the current device setup's recording offset and the offsets
   * stored per device setup (each can be forgotten). Locked while recording, like the panel.
   */
  const pref = preferencesState();
  const settings = settingsState();
  const rec = recordState();
  const prefs = $derived(rec.prefs);
  const locked = $derived(rec.state.recording || rec.state.finishing);
  const offsets = $derived(settings.current?.record_offsets ?? []);
  const currentOffset = $derived(offsetReadout(rec.offset));

  $effect(() => {
    if (pref.open) {
      void refreshOffset();
    }
  });

  function numberFrom(event: Event, max: number): number | null {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    return Number.isFinite(value) ? Math.max(0, Math.min(max, value)) : null;
  }

  function offsetMs(ms: number): string {
    return `${ms >= 0 ? "+" : ""}${ms.toFixed(2)}`;
  }

  function offsetDate(unixMs: number): string {
    return new Date(unixMs).toLocaleDateString(undefined, {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }

  /** Forgets one device setup's stored offset (that setup then records with δ = 0). */
  async function removeOffset(entry: RecordOffsetEntry): Promise<void> {
    await saveSettings({ record_offsets: offsets.filter((e) => e !== entry) });
    void refreshOffset();
  }

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

      <section data-testid="preferences-recording">
        <h3>{t("preferences.section.recording")}</h3>
        {#if locked}
          <p class="hint" data-testid="preferences-recording-locked">{t("preferences.recording.locked")}</p>
        {/if}
        <div class="choice" role="radiogroup" aria-label={t("record.mode")} title={t("record.mode_title")}>
          <span>{t("record.mode")}</span>
          <label>
            <input
              type="radio"
              name="preferences-record-mode"
              data-testid="preferences-record-mode-insert"
              checked={prefs.mode === "insert"}
              disabled={locked}
              onchange={() => void setRecordPrefs({ mode: "insert" })}
            />
            {t("record.mode.insert")}
          </label>
          <label>
            <input
              type="radio"
              name="preferences-record-mode"
              data-testid="preferences-record-mode-overwrite"
              checked={prefs.mode === "overwrite"}
              disabled={locked}
              onchange={() => void setRecordPrefs({ mode: "overwrite" })}
            />
            {t("record.mode.overwrite")}
          </label>
        </div>
        <label class="check">
          <input
            type="checkbox"
            data-testid="preferences-punch-on-selection"
            checked={prefs.punch_on_selection}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ punch_on_selection: e.currentTarget.checked })}
          />
          {t("record.punch_on_selection")}
        </label>
        <label class="number">
          <span>{t("preferences.recording.preroll_s")}</span>
          <input
            type="number"
            min="0"
            max={ROLL_MAX_S}
            step="0.1"
            data-testid="preferences-preroll"
            value={prefs.preroll_s}
            disabled={locked}
            onchange={(e) => {
              const v = numberFrom(e, ROLL_MAX_S);
              if (v !== null) void setRecordPrefs({ preroll_s: v });
            }}
          />
        </label>
        <label class="number">
          <span>{t("preferences.recording.postroll_s")}</span>
          <input
            type="number"
            min="0"
            max={ROLL_MAX_S}
            step="0.1"
            data-testid="preferences-postroll"
            value={prefs.postroll_s}
            disabled={locked}
            onchange={(e) => {
              const v = numberFrom(e, ROLL_MAX_S);
              if (v !== null) void setRecordPrefs({ postroll_s: v });
            }}
          />
        </label>
        <label class="check">
          <input
            type="checkbox"
            data-testid="preferences-preroll-at-cursor"
            checked={prefs.preroll_at_cursor}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ preroll_at_cursor: e.currentTarget.checked })}
          />
          {t("record.preroll_at_cursor")}
        </label>
        <label class="check">
          <input
            type="checkbox"
            data-testid="preferences-hear-original"
            checked={prefs.hear_original}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ hear_original: e.currentTarget.checked })}
          />
          {t("record.hear_original")}
        </label>
        <label class="number">
          <span>{t("preferences.recording.xfade_ms")}</span>
          <input
            type="number"
            min="0"
            max={XFADE_MAX_MS}
            step="1"
            data-testid="preferences-xfade"
            value={prefs.punch_xfade_ms}
            disabled={locked}
            onchange={(e) => {
              const v = numberFrom(e, XFADE_MAX_MS);
              if (v !== null) void setRecordPrefs({ punch_xfade_ms: v });
            }}
          />
        </label>

        <h4>{t("preferences.recording.offsets")}</h4>
        <p class="current">
          <span>{t("record.offset.label")}</span>
          <span class="value" data-testid="preferences-offset-current">
            {tDynamic(currentOffset.key, currentOffset.params)}
          </span>
        </p>
        {#if offsets.length === 0}
          <p class="hint" data-testid="preferences-offsets-empty">{t("preferences.recording.offsets_empty")}</p>
        {:else}
          <ul class="offsets">
            {#each offsets as entry (`${entry.host}|${entry.input_device}|${entry.output_device}|${entry.device_rate_hz}`)}
              <li data-testid="preferences-offset-entry">
                <span>
                  {t("preferences.recording.offset_entry", {
                    input: entry.input_device,
                    output: entry.output_device,
                    host: entry.host,
                    rate: String(entry.device_rate_hz / 1000),
                  })}
                </span>
                <span class="value">
                  {entry.source === "manual"
                    ? t("preferences.recording.offset_manual", { ms: offsetMs(entry.offset_ms) })
                    : t("preferences.recording.offset_calibrated", {
                        ms: offsetMs(entry.offset_ms),
                        date: offsetDate(entry.updated_unix_ms),
                      })}
                </span>
                <button
                  type="button"
                  data-testid="preferences-offset-remove"
                  title={t("preferences.recording.offset_remove_title")}
                  disabled={locked}
                  onclick={() => void removeOffset(entry)}
                >
                  {t("preferences.recording.offset_remove")}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
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

  section + section {
    border-top: 1px solid var(--surface-border);
    padding-top: 0.75rem;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  h4 {
    margin: 0.5rem 0 0;
    font-size: 0.85rem;
    color: var(--text-secondary);
  }

  .choice,
  .check,
  .current {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0;
  }

  .number {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .number input {
    width: 5.5rem;
  }

  .offsets {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .offsets li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
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
