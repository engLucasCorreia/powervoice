<script lang="ts">
  import type { MultichannelPolicy, RecordOffsetEntry, Settings } from "../ipc/bindings";
  import { getSettingsDefaults } from "../ipc/commands";
  import { ROLL_MAX_S, XFADE_MAX_MS, offsetReadout } from "../record/punch";
  import { formatBytes } from "../recovery/format";
  import { openCalibration, recordState, refreshOffset, setRecordPrefs } from "../state/record.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { t, tDynamic } from "../i18n";
  import { chooseTheme } from "../theme/chooseTheme";
  import ThemePicker from "../theme/ThemePicker.svelte";
  import { Button, Dialog, Select, formatNumber } from "../ui";
  import { closePreferences, preferencesState } from "./preferences.svelte";
  import PluginFolders from "../plugins/PluginFolders.svelte";
  import { countPlugins } from "../plugins/pluginList";
  import { openPluginManager, pluginsState, refreshPlugins } from "../plugins/plugins.svelte";

  /**
   * H-17 item 5: a small, reusable Preferences dialog — Edit → Preferences… (File → Preferences…
   * on macOS). T-703 (settings audit) fixed the section order/naming to a consistent taxonomy —
   * Recording, Editing, Display/Appearance, Plugins, Advanced (no separate "Audio" section: every
   * audio-category setting — devices/buffer, default format, monitor mode — already has a working
   * home the spec names explicitly: the Audio Devices dialog off the transport bar, the New
   * Recording dialog, and the record panel, respectively; see the T-703 report for the audit) —
   * and added Reset to defaults (per section and for the whole dialog, H-26 confirm).
   *
   * H-21 item 6 (SPEC-022 §2.3, §2.13): the Recording section — the same Punch & pre-roll
   * preferences as the record panel (mode, punch on selection, pre-/post-roll, pre-roll at the
   * cursor, hear original, crossfade), the current device setup's recording offset (+ Calibrate,
   * SPEC-022 §2.14) and the offsets stored per device setup (each can be forgotten). Locked while
   * recording, like the panel.
   *
   * T-703: the Editing section — Multichannel files (SPEC-005 §2.4/§3 `multichannel_policy`,
   * "Settings → Files") and Snap to Zero Crossing (SPEC-006 §2.10; mirrors the View menu toggle).
   *
   * T-809: the Plugins section — how many plugins are installed (and how many need attention),
   * the user's own scan folders (add/remove, a rescan follows) and Manage plugins…. No blanket
   * reset here (folder add/remove already has its own rescan side effect a reset would bypass).
   *
   * T-703: the Advanced section — Memory for audio (renamed from "Storage", SPEC-004 §2.4/§3
   * "Settings → Performance") and the playhead/meter update rate (SPEC-003 §3
   * `telemetry_rate_hz`, "kept as a Settings option" — had no UI at all before this ticket).
   */
  const pref = preferencesState();
  // H-25/T-708: Appearance → Theme (a card per theme with a live preview) applies at once and
  // persists in Settings — the same `chooseTheme` as View → Theme ▸.
  const settings = settingsState();
  const rec = recordState();
  const prefs = $derived(rec.prefs);
  const locked = $derived(rec.state.recording || rec.state.finishing);
  const offsets = $derived(settings.current?.record_offsets ?? []);
  const currentOffset = $derived(offsetReadout(rec.offset));
  // T-703: Editing → Multichannel files (SPEC-005 §2.4/§3 "Settings → Files"; previously only
  // settable in passing, via the "remember my choice" checkbox on the open-time dialog itself).
  const multichannelPolicy = $derived(settings.current?.multichannel_policy ?? "ask");
  // T-703: Advanced → playhead/meter update rate (SPEC-003 §3 `telemetry_rate_hz`, "kept as a
  // Settings option" — had no UI at all before this ticket).
  const telemetryRateHz = $derived(settings.current?.telemetry_rate_hz ?? 60);
  const snapToZeroCrossing = $derived(settings.current?.snap_to_zero_crossing ?? false);
  const multichannelPolicyOptions = $derived(
    [
      { value: "ask", label: t("preferences.multichannel_policy.ask") },
      { value: "always_mix", label: t("preferences.multichannel_policy.always_mix") },
      { value: "always_first_channel", label: t("preferences.multichannel_policy.always_first_channel") },
    ] satisfies { value: MultichannelPolicy; label: string }[],
  );

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

  function calibrate(): void {
    closePreferences();
    openCalibration();
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

  // T-703 item 3: "Reset to defaults, per section and for everything". Scoped to exactly the
  // fields this dialog lets you edit directly — record OFFSETS (per-entry "Remove" above),
  // recent files, tours, the plugin scan folders (their own add/remove, which also triggers a
  // rescan) and the rest of `Settings` are untouched by any reset here.
  type ResetScope = "recording" | "editing" | "appearance" | "advanced";
  const RESET_SECTION_FIELDS: Record<ResetScope, (keyof Settings)[]> = {
    recording: ["record"],
    editing: ["multichannel_policy", "snap_to_zero_crossing"],
    appearance: ["theme"],
    advanced: ["memory_budget_mib", "telemetry_rate_hz"],
  };
  let resetPrompt = $state<ResetScope | "all" | null>(null);
  let resetBusy = $state(false);

  async function confirmReset(): Promise<void> {
    if (!resetPrompt) {
      return;
    }
    resetBusy = true;
    try {
      const defaults = await getSettingsDefaults();
      const scopes: ResetScope[] =
        resetPrompt === "all" ? (Object.keys(RESET_SECTION_FIELDS) as ResetScope[]) : [resetPrompt];
      const patch: Partial<Settings> = {};
      for (const scope of scopes) {
        for (const field of RESET_SECTION_FIELDS[scope]) {
          (patch as Record<string, unknown>)[field] = defaults[field];
        }
      }
      await saveSettings(patch);
    } finally {
      resetBusy = false;
      resetPrompt = null;
    }
  }
</script>

{#if pref.open}
  <Dialog
    actions={[
      { label: t("preferences.reset_all"), role: "utility", testid: "preferences-reset-all", onclick: () => (resetPrompt = "all") },
      { label: t("preferences.close"), role: "primary", testid: "preferences-close", onclick: closePreferences },
    ]} size="lg" title={t("preferences.title")} titleId="preferences-title" testid="preferences-dialog" onkeydown={onKeydown}>
    <section data-testid="preferences-recording">
      <div class="section-head">
        <h3>{t("preferences.section.recording")}</h3>
        <Button variant="ghost" size="sm" testid="preferences-reset-recording" onclick={() => (resetPrompt = "recording")}>
          {t("preferences.reset_section")}
        </Button>
      </div>
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
        <Button
          variant="ghost"
          size="sm"
          testid="preferences-calibrate"
          disabled={rec.offset?.available === false}
          onclick={calibrate}
        >
          {t("record.offset.calibrate")}
        </Button>
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
    <section data-testid="preferences-editing">
      <div class="section-head">
        <h3>{t("preferences.section.editing")}</h3>
        <Button variant="ghost" size="sm" testid="preferences-reset-editing" onclick={() => (resetPrompt = "editing")}>
          {t("preferences.reset_section")}
        </Button>
      </div>
      <div class="row">
        <span class="label" id="preferences-multichannel-policy-label">{t("preferences.multichannel_policy")}</span>
        <Select
          label={t("preferences.multichannel_policy")}
          hideLabel
          testid="preferences-multichannel-policy"
          options={multichannelPolicyOptions}
          value={multichannelPolicy}
          onchange={(value) => void saveSettings({ multichannel_policy: value })}
        />
      </div>
      <p class="hint">{t("preferences.multichannel_policy.hint")}</p>
      <label class="option">
        <input
          type="checkbox"
          data-testid="preferences-snap-to-zero-crossing"
          checked={snapToZeroCrossing}
          onchange={(e) => void saveSettings({ snap_to_zero_crossing: e.currentTarget.checked })}
        />
        {t("menu.view.snap_to_zero_crossing")}
      </label>
    </section>
    <section data-testid="preferences-appearance">
      <div class="section-head">
        <h3>{t("preferences.section.appearance")}</h3>
        <Button variant="ghost" size="sm" testid="preferences-reset-appearance" onclick={() => (resetPrompt = "appearance")}>
          {t("preferences.reset_section")}
        </Button>
      </div>
      <span class="label">{t("preferences.theme")}</span>
      <ThemePicker value={settings.current?.theme ?? "dark"} label={t("preferences.theme")} onchange={chooseTheme} />
      <p class="hint">{t("preferences.theme.hint")}</p>
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
    <section data-testid="preferences-advanced">
      <div class="section-head">
        <h3>{t("preferences.section.advanced")}</h3>
        <Button variant="ghost" size="sm" testid="preferences-reset-advanced" onclick={() => (resetPrompt = "advanced")}>
          {t("preferences.reset_section")}
        </Button>
      </div>
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
      <div class="row">
        <span class="label" id="preferences-telemetry-rate-label">{t("preferences.telemetry_rate")}</span>
        <Select
          label={t("preferences.telemetry_rate")}
          hideLabel
          testid="preferences-telemetry-rate"
          options={[
            { value: 30, label: t("preferences.telemetry_rate.30") },
            { value: 60, label: t("preferences.telemetry_rate.60") },
          ]}
          value={telemetryRateHz}
          onchange={(value) => void saveSettings({ telemetry_rate_hz: value })}
        />
      </div>
      <p class="hint">{t("preferences.telemetry_rate.hint")}</p>
    </section>
  </Dialog>
{/if}

{#if resetPrompt}
  <Dialog
    size="sm"
    role="alertdialog"
    title={tDynamic(`preferences.reset.${resetPrompt}.title`)}
    testid="preferences-reset-dialog"
    onkeydown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") resetPrompt = null;
    }}
    actions={[
      {
        label: t("preferences.reset.cancel"),
        role: "cancel",
        testid: "preferences-reset-cancel",
        disabled: resetBusy,
        onclick: () => (resetPrompt = null),
      },
      {
        label: t("preferences.reset.confirm"),
        role: "primary",
        variant: "danger",
        testid: "preferences-reset-confirm",
        loading: resetBusy,
        onclick: () => void confirmReset(),
      },
    ]}
  >
    <p data-testid="preferences-reset-message">{t("preferences.reset.message")}</p>
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

  section > h3:first-child,
  section > .section-head:first-child h3 {
    margin-top: 0;
  }

  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--pv-space-3);
  }

  .section-head h3 {
    margin: 0;
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
  .label {
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
