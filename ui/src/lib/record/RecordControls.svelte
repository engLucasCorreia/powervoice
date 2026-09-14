<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import type { MonitorMode, RecordModePref } from "../ipc/bindings";
  import {
    clearClip,
    openCalibration,
    recordState,
    refreshOffset,
    setManualOffset,
    setMonitor,
    setRecordPrefs,
    toggleArm,
    toggleRecord,
  } from "../state/record.svelte";
  import { transportState } from "../state/transport.svelte";
  import {
    DISK_WARN_MINUTES,
    formatElapsed,
    formatLatencyMs,
    formatRemaining,
    monitorLatencyLevel,
  } from "./format";
  import { ROLL_MAX_S, XFADE_MAX_MS, bufferHint, offsetReadout, phaseLabel } from "./punch";

  /**
   * Record panel controls (S1-04, SPEC-002 §2.1–§2.2, §2.7): Input (arm) toggle, Record/Stop
   * button (Shift+R), elapsed time, clip lamp (click to clear), monitoring Off / Dry / Through
   * rack with the monitoring latency readout (T-107: amber ≥ 20 ms, red ≥ 40 ms), and (H-11) the
   * estimated remaining recording time on the session volume (amber below `DISK_WARN_MINUTES`).
   *
   * T-304 (SPEC-022 §2.3, §2.11): the operation's phase label with its countdown, a right-click
   * menu on Record with the mode items, and the Punch & pre-roll section (mode, punch on
   * selection, pre/post-roll, pre-roll at the cursor, hear original, crossfade, the recording
   * offset readout with Calibrate… and manual entry) — locked while recording.
   */
  const rec = recordState();
  const transport = transportState();
  const noInput = $derived(rec.state.input_device === null);
  const busy = $derived(rec.state.recording || rec.state.finishing);
  const elapsed = $derived(
    formatElapsed(
      rec.elapsedSamples,
      rec.op ? transport.state.doc_rate_hz : (rec.state.input_rate_hz ?? 0),
    ),
  );
  const monitor = $derived<MonitorMode>(rec.state.monitor);
  const diskRemaining = $derived(rec.state.disk_remaining_s);
  const diskLow = $derived(diskRemaining !== null && diskRemaining < DISK_WARN_MINUTES * 60);
  const latencyUs = $derived(rec.state.monitor_latency_us);
  const latencyLevel = $derived(latencyUs === null ? "ok" : monitorLatencyLevel(latencyUs / 1000));
  const latencyTitle = $derived(
    latencyLevel === "red"
      ? t("record.monitor_warn_red")
      : latencyLevel === "amber"
        ? t("record.monitor_warn_amber")
        : t("record.monitor_latency_title"),
  );
  const prefs = $derived(rec.prefs);
  const phase = $derived(phaseLabel(rec.op, rec.phase, rec.heardSamples, transport.state.doc_rate_hz));
  const readout = $derived(offsetReadout(rec.offset));
  const hint = $derived(bufferHint(rec.offset));

  let panelOpen = $state(false);
  let menuOpen = $state(false);
  let offsetText = $state("");
  let offsetInvalid = $state(false);

  function togglePanel(): void {
    panelOpen = !panelOpen;
    if (panelOpen) {
      void refreshOffset();
    }
  }

  function onRecordContextMenu(event: MouseEvent): void {
    event.preventDefault();
    if (!busy) {
      menuOpen = !menuOpen;
    }
  }

  function chooseMode(mode: RecordModePref): void {
    menuOpen = false;
    void setRecordPrefs({ mode });
  }

  function numberFrom(event: Event, max: number): number | null {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    return Number.isFinite(value) ? Math.max(0, Math.min(max, value)) : null;
  }

  async function submitOffset(): Promise<void> {
    offsetInvalid = !(await setManualOffset(offsetText));
    if (!offsetInvalid) {
      offsetText = "";
    }
  }
</script>

<div class="record" role="group" aria-label={t("record.group")} data-testid="record-controls">
  <button
    type="button"
    data-testid="record-arm"
    class:active={rec.state.armed}
    aria-pressed={rec.state.armed}
    disabled={noInput || busy}
    title={noInput ? t("record.no_input_hint") : t("record.arm_title")}
    onclick={() => void toggleArm()}
  >
    {t("record.arm")}
  </button>
  <span class="rec-wrap">
    <button
      type="button"
      data-testid="record-button"
      class="rec"
      class:recording={rec.state.recording}
      disabled={noInput || rec.state.finishing}
      title={noInput ? t("record.no_input_hint") : t("record.record_title")}
      onclick={() => void toggleRecord()}
      oncontextmenu={onRecordContextMenu}
    >
      {rec.state.recording ? t("record.stop") : t("record.record")}
    </button>
    {#if menuOpen}
      <div class="menu" role="menu" aria-label={t("record.menu_title")} data-testid="record-context-menu">
        <button
          type="button"
          role="menuitemradio"
          aria-checked={prefs.mode === "insert"}
          data-testid="record-menu-insert"
          onclick={() => chooseMode("insert")}
        >
          {t("record.mode.insert")}
        </button>
        <button
          type="button"
          role="menuitemradio"
          aria-checked={prefs.mode === "overwrite"}
          data-testid="record-menu-overwrite"
          onclick={() => chooseMode("overwrite")}
        >
          {t("record.mode.overwrite")}
        </button>
        <button
          type="button"
          role="menuitemcheckbox"
          aria-checked={prefs.punch_on_selection}
          data-testid="record-menu-punch"
          onclick={() => {
            menuOpen = false;
            void setRecordPrefs({ punch_on_selection: !prefs.punch_on_selection });
          }}
        >
          {t("record.punch_on_selection")}
        </button>
      </div>
    {/if}
  </span>
  <span class="elapsed" data-testid="record-elapsed" title={t("record.elapsed_title")}>{elapsed}</span>
  {#if phase}
    <span class="phase" data-testid="record-phase">{tDynamic(phase.key, phase.params)}</span>
  {/if}
  {#if diskRemaining !== null}
    <span
      class="disk-remaining"
      class:low={diskLow}
      data-testid="record-disk-remaining"
      title={t("record.disk_remaining_title")}
    >
      {t("record.disk_remaining", { time: formatRemaining(diskRemaining) })}
    </span>
  {/if}
  {#if rec.state.dropout_count > 0}
    <span
      class="dropouts"
      data-testid="record-dropouts"
      title={t("record.dropouts_title")}
    >
      {t("record.dropouts", { count: String(rec.state.dropout_count) })}
    </span>
  {/if}
  <button
    type="button"
    data-testid="record-clip"
    class="clip"
    class:lit={rec.clipLatched}
    title={t("record.clip_title")}
    onclick={clearClip}
  >
    {t("record.clip")}
  </button>
  <label class="monitor">
    <span>{t("record.monitor")}</span>
    <select
      data-testid="record-monitor"
      value={monitor}
      onchange={(e) => void setMonitor(e.currentTarget.value as MonitorMode)}
    >
      <option value="off">{t("record.monitor.off")}</option>
      <option value="dry">{t("record.monitor.dry")}</option>
      <option value="through_rack">{t("record.monitor.through_rack")}</option>
    </select>
  </label>
  {#if latencyUs !== null}
    <span
      class="monitor-latency"
      class:amber={latencyLevel === "amber"}
      class:red={latencyLevel === "red"}
      data-testid="record-monitor-latency"
      title={latencyTitle}
    >
      {t("record.monitor_latency", { ms: formatLatencyMs(latencyUs) })}
    </span>
  {/if}
  <span class="punch-wrap">
    <button
      type="button"
      data-testid="record-punch-toggle"
      aria-expanded={panelOpen}
      onclick={togglePanel}
    >
      {t("record.punch_section")}
    </button>
    {#if panelOpen}
      <div class="panel" data-testid="record-punch-panel">
        <div class="row" role="group" aria-label={t("record.mode")} title={t("record.mode_title")}>
          <span>{t("record.mode")}</span>
          <button
            type="button"
            data-testid="record-mode-insert"
            class:active={prefs.mode === "insert"}
            aria-pressed={prefs.mode === "insert"}
            disabled={busy}
            onclick={() => void setRecordPrefs({ mode: "insert" })}
          >
            {t("record.mode.insert")}
          </button>
          <button
            type="button"
            data-testid="record-mode-overwrite"
            class:active={prefs.mode === "overwrite"}
            aria-pressed={prefs.mode === "overwrite"}
            disabled={busy}
            onclick={() => void setRecordPrefs({ mode: "overwrite" })}
          >
            {t("record.mode.overwrite")}
          </button>
        </div>
        <label class="row">
          <input
            type="checkbox"
            data-testid="record-punch-on-selection"
            checked={prefs.punch_on_selection}
            disabled={busy}
            onchange={(e) => void setRecordPrefs({ punch_on_selection: e.currentTarget.checked })}
          />
          <span>{t("record.punch_on_selection")}</span>
        </label>
        <label class="row">
          <span>{t("record.preroll")}</span>
          <input
            type="number"
            min="0"
            max={ROLL_MAX_S}
            step="0.1"
            data-testid="record-preroll"
            value={prefs.preroll_s}
            disabled={busy}
            onchange={(e) => {
              const v = numberFrom(e, ROLL_MAX_S);
              if (v !== null) void setRecordPrefs({ preroll_s: v });
            }}
          />
          <span>{t("record.seconds")}</span>
        </label>
        <label class="row">
          <span>{t("record.postroll")}</span>
          <input
            type="number"
            min="0"
            max={ROLL_MAX_S}
            step="0.1"
            data-testid="record-postroll"
            value={prefs.postroll_s}
            disabled={busy}
            onchange={(e) => {
              const v = numberFrom(e, ROLL_MAX_S);
              if (v !== null) void setRecordPrefs({ postroll_s: v });
            }}
          />
          <span>{t("record.seconds")}</span>
        </label>
        <label class="row">
          <input
            type="checkbox"
            data-testid="record-preroll-at-cursor"
            checked={prefs.preroll_at_cursor}
            disabled={busy}
            onchange={(e) => void setRecordPrefs({ preroll_at_cursor: e.currentTarget.checked })}
          />
          <span>{t("record.preroll_at_cursor")}</span>
        </label>
        {#if !transport.state.can_play}
          <p class="hint" data-testid="record-preroll-needs-output">{t("record.preroll_needs_output")}</p>
        {/if}
        <label class="row">
          <input
            type="checkbox"
            data-testid="record-hear-original"
            checked={prefs.hear_original}
            disabled={busy}
            onchange={(e) => void setRecordPrefs({ hear_original: e.currentTarget.checked })}
          />
          <span>{t("record.hear_original")}</span>
        </label>
        <label class="row">
          <span>{t("record.xfade")}</span>
          <input
            type="number"
            min="0"
            max={XFADE_MAX_MS}
            step="1"
            data-testid="record-xfade"
            value={prefs.punch_xfade_ms}
            disabled={busy}
            onchange={(e) => {
              const v = numberFrom(e, XFADE_MAX_MS);
              if (v !== null) void setRecordPrefs({ punch_xfade_ms: v });
            }}
          />
          <span>{t("record.milliseconds")}</span>
        </label>
        <div class="row offset">
          <span>{t("record.offset.label")}</span>
          <span data-testid="record-offset">{tDynamic(readout.key, readout.params)}</span>
          <button
            type="button"
            data-testid="record-calibrate"
            disabled={busy || rec.offset?.available === false}
            onclick={openCalibration}
          >
            {t("record.offset.calibrate")}
          </button>
        </div>
        {#if hint}
          <p class="hint amber" data-testid="record-offset-hint">
            {t("record.offset.buffer_hint", hint)}
          </p>
        {/if}
        <form
          class="row"
          onsubmit={(e) => {
            e.preventDefault();
            void submitOffset();
          }}
        >
          <input
            type="text"
            data-testid="record-offset-entry"
            title={t("record.offset.entry_title")}
            placeholder="0.00 / 150 smp"
            bind:value={offsetText}
            disabled={busy || rec.offset?.available === false}
          />
          <button type="submit" data-testid="record-offset-set" disabled={busy}>
            {t("record.offset.set")}
          </button>
        </form>
        {#if offsetInvalid}
          <p class="hint amber">{t("record.offset.invalid")}</p>
        {/if}
      </div>
    {/if}
  </span>
</div>

<style>
  .record {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  button.active {
    border-color: var(--accent);
    color: var(--accent);
  }

  button.rec.recording {
    background: var(--meter-red);
    border-color: var(--meter-red);
    color: var(--text-primary);
  }

  .rec-wrap,
  .punch-wrap {
    position: relative;
  }

  .menu,
  .panel {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.5rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    min-width: 12rem;
  }

  .panel {
    min-width: 22rem;
    right: 0;
    left: auto;
  }

  .menu button[aria-checked="true"] {
    color: var(--accent);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .row input[type="number"] {
    width: 4.5rem;
  }

  .row input[type="text"] {
    flex: 1;
  }

  .hint {
    margin: 0;
    font-size: 0.75rem;
    color: var(--text-secondary);
  }

  .hint.amber {
    color: var(--meter-yellow);
  }

  .phase {
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    background: var(--meter-red);
    color: var(--text-primary);
  }

  .elapsed {
    min-width: 5.5rem;
    padding: 0.2rem 0.5rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  button.clip {
    padding: 0.25rem 0.4rem;
    font-size: 0.7rem;
    color: var(--text-disabled);
  }

  button.clip.lit {
    background: var(--meter-red);
    border-color: var(--meter-red);
    color: var(--text-primary);
  }

  .dropouts {
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    background: var(--meter-yellow);
    color: var(--surface-panel);
  }

  .disk-remaining {
    font-size: 0.75rem;
    color: var(--text-secondary);
  }

  .disk-remaining.low {
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: var(--meter-yellow);
    color: var(--surface-panel);
  }

  .monitor-latency {
    font-size: 0.75rem;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .monitor-latency.amber,
  .monitor-latency.red {
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    color: var(--surface-panel);
  }

  .monitor-latency.amber {
    background: var(--meter-yellow);
  }

  .monitor-latency.red {
    background: var(--meter-red);
    color: var(--text-primary);
  }

  .monitor {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  select {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.15rem 0.3rem;
  }
</style>
