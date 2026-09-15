<script lang="ts">
  import type { RecordOffsetEntry } from "../ipc/bindings";
  import { ROLL_MAX_S, XFADE_MAX_MS, offsetReadout } from "../record/punch";
  import { formatBytes } from "../recovery/format";
  import { recordState, refreshOffset, setRecordPrefs } from "../state/record.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { t, tDynamic } from "../i18n";
  import type { ThemePref } from "../ipc/bindings";
  import { applyThemePref } from "../theme/theme.svelte";
  import { Button, Dialog, formatNumber, SegmentedControl, type SegmentOption } from "../ui";
  import { closePreferences, preferencesState } from "./preferences.svelte";
  import PluginFolders from "../plugins/PluginFolders.svelte";
  import { countPlugins } from "../plugins/pluginList";
  import { openPluginManager, pluginsState, refreshPlugins } from "../plugins/plugins.svelte";

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
   *
   * T-809: the Plugins section — how many plugins are installed (and how many need attention),
   * the user's own scan folders (add/remove, a rescan follows) and Manage plugins….
   */
  const pref = preferencesState();
  // H-25: Appearance → Theme applies at once and persists in Settings.
  const THEMES: SegmentOption<ThemePref>[] = [
    { value: "dark", label: t("preferences.theme.dark"), testid: "preferences-theme-dark" },
    { value: "light", label: t("preferences.theme.light"), testid: "preferences-theme-light" },
    { value: "system", label: t("preferences.theme.system"), testid: "preferences-theme-system" },
  ];

  function chooseTheme(theme: ThemePref): void {
    applyThemePref(theme);
    void saveSettings({ theme });
  }
  const settings = settingsState();
  const rec = recordState();
  const prefs = $derived(rec.prefs);
  const locked = $derived(rec.state.recording || rec.state.finishing);
  const offsets = $derived(settings.current?.record_offsets ?? []);
  const currentOffset = $derived(offsetReadout(rec.offset));

  $effect(() => {
    if (pref.open) {
      void refreshOffset();
      void refreshPlugins();
    }
  });

  const plugins = pluginsState();
  const pluginSummary = $derived.by(() => {
    if (plugins.list === null) {
      return t("preferences.plugins.loading");
    }
    const counts = countPlugins(plugins.list);
    const total = counts.total === 1 ? t("plugins.count.one") : t("plugins.count.many", { count: counts.total });
    const attention = counts.blocklisted + counts.flagged;
    return attention > 0 ? `${total} · ${t("preferences.plugins.attention", { count: attention })}` : total;
  });

  function managePlugins(): void {
    closePreferences();
    openPluginManager();
  }

  function numberFrom(event: Event, max: number): number | null {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    return Number.isFinite(value) ? Math.max(0, Math.min(max, value)) : null;
  }

  function offsetMs(ms: number): string {
    return formatNumber(ms, 2, { signed: true });
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
  <Dialog
    actions={[{ label: t("preferences.close"), role: "primary", testid: "preferences-close", onclick: closePreferences }]} size="lg" title={t("preferences.title")} titleId="preferences-title" testid="preferences-dialog" onkeydown={onKeydown}>
    <section data-testid="preferences-appearance">
      <h3>{t("preferences.section.appearance")}</h3>
      <div class="row">
        <span class="label">{t("preferences.theme")}</span>
        <SegmentedControl
          options={THEMES}
          value={settings.current?.theme ?? "dark"}
          label={t("preferences.theme")}
          size="sm"
          onchange={chooseTheme}
        />
      </div>
      <p class="hint">{t("preferences.theme.hint")}</p>
    </section>
    <section>
      <h3>{t("preferences.section.storage")}</h3>
      <div class="row">
        <label for="preferences-memory-budget">{t("preferences.memory_budget")}</label>
        <div class="range">
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
        </div>
      </div>
      <p class="hint">{t("preferences.memory_budget.hint")}</p>
    </section>
    <section data-testid="preferences-recording">
      <h3>{t("preferences.section.recording")}</h3>
      {#if locked}
        <p class="hint" data-testid="preferences-recording-locked">{t("preferences.recording.locked")}</p>
      {/if}
      <div class="row" role="radiogroup" aria-label={t("record.mode")} title={t("record.mode_title")}>
        <span class="label">{t("record.mode")}</span>
        <div class="options">
          <label class="option">
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
          <label class="option">
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
      </div>
      <div class="row">
        <label for="preferences-preroll-input">{t("preferences.recording.preroll_s")}</label>
        <input
          id="preferences-preroll-input"
          class="number"
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
      </div>
      <div class="row">
        <label for="preferences-postroll-input">{t("preferences.recording.postroll_s")}</label>
        <input
          id="preferences-postroll-input"
          class="number"
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
      </div>
      <div class="row">
        <label for="preferences-xfade-input">{t("preferences.recording.xfade_ms")}</label>
        <input
          id="preferences-xfade-input"
          class="number"
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
      </div>
      <div class="checks">
        <label class="option">
          <input
            type="checkbox"
            data-testid="preferences-punch-on-selection"
            checked={prefs.punch_on_selection}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ punch_on_selection: e.currentTarget.checked })}
          />
          {t("record.punch_on_selection")}
        </label>
        <label class="option">
          <input
            type="checkbox"
            data-testid="preferences-preroll-at-cursor"
            checked={prefs.preroll_at_cursor}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ preroll_at_cursor: e.currentTarget.checked })}
          />
          {t("record.preroll_at_cursor")}
        </label>
        <label class="option">
          <input
            type="checkbox"
            data-testid="preferences-hear-original"
            checked={prefs.hear_original}
            disabled={locked}
            onchange={(e) => void setRecordPrefs({ hear_original: e.currentTarget.checked })}
          />
          {t("record.hear_original")}
        </label>
      </div>
      <h3>{t("preferences.recording.offsets")}</h3>
      <div class="row">
        <span class="label">{t("record.offset.label")}</span>
        <span class="value" data-testid="preferences-offset-current">
          {tDynamic(currentOffset.key, currentOffset.params)}
        </span>
      </div>
      {#if offsets.length === 0}
        <p class="hint" data-testid="preferences-offsets-empty">{t("preferences.recording.offsets_empty")}</p>
      {:else}
        <ul class="offsets">
          {#each offsets as entry (`${entry.host}|${entry.input_device}|${entry.output_device}|${entry.device_rate_hz}`)}
            <li data-testid="preferences-offset-entry">
              <span class="devices">
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
              <Button
                variant="ghost"
                size="sm"
                icon="delete"
                testid="preferences-offset-remove"
                title={t("preferences.recording.offset_remove_title")}
                disabled={locked}
                onclick={() => void removeOffset(entry)}
              >
                {t("preferences.recording.offset_remove")}
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
    <section data-testid="preferences-plugins">
      <h3>{t("preferences.section.plugins")}</h3>
      <div class="row">
        <span class="label">{t("preferences.plugins.installed")}</span>
        <div class="plugins-summary">
          <span class="value" data-testid="preferences-plugins-summary">{pluginSummary}</span>
          <Button icon="plugin" size="sm" testid="preferences-manage-plugins" onclick={managePlugins}>
            {t("preferences.plugins.manage")}
          </Button>
        </div>
      </div>
      <PluginFolders showStandard={false} />
    </section>
  </Dialog>
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
  }

  section + section {
    padding-top: var(--pv-space-4);
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  section > h3:first-child {
    margin-top: 0;
  }

  /* One setting per row: label on the left (fixed column), control on the right. */
  .row {
    display: grid;
    grid-template-columns: 11rem minmax(0, 1fr);
    align-items: center;
    gap: var(--pv-space-3);
    min-height: var(--pv-control-h-md);
  }

  .row > label,
  .row > .label {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .range {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
  }

  .range input {
    flex: 1;
  }

  .number {
    width: 6rem;
    text-align: right;
  }

  .value {
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .checks {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    padding-left: calc(11rem + var(--pv-space-3));
  }

  .offsets {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .offsets li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2) var(--pv-space-3);
    padding: var(--pv-space-2) var(--pv-space-3);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-raised);
  }

  .plugins-summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--pv-space-2);
  }

  .devices {
    flex: 1;
    min-width: 12rem;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }
</style>
