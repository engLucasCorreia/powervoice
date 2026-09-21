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
  import { Button, Icon, IconButton, Menu, Popover, Separator, type PopoverAnchor } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import {
    DISK_WARN_MINUTES,
    formatElapsed,
    formatLatencyMs,
    formatRemaining,
    monitorLatencyLevel,
  } from "./format";
  import { ROLL_MAX_S, XFADE_MAX_MS, bufferHint, offsetReadout, phaseLabel } from "./punch";
  import TourButton from "../tour/TourButton.svelte";
  import HelpButton from "../help/HelpButton.svelte";

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
   *
   * H-25: two clusters in the transport bar — the Record key (lamp at rest, solid red on air)
   * with the take's time, status chips and the CLIP lamp; then Input arm, Monitoring and the
   * Punch & pre-roll popover behind an icon key.
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
  // H-26: the Record context menu opens at the pointer (or under the key when opened from the
  // keyboard); the Punch & pre-roll panel is a Popover under its icon key.
  let menuAnchor = $state<PopoverAnchor | null>(null);
  let recordButton: HTMLButtonElement | undefined = $state();
  let punchToggle: HTMLButtonElement | undefined = $state();
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
    if (busy) {
      return;
    }
    const fromKeyboard = event.clientX === 0 && event.clientY === 0;
    menuAnchor = fromKeyboard && recordButton ? recordButton : { x: event.clientX, y: event.clientY };
  }

  function chooseMode(mode: RecordModePref): void {
    void setRecordPrefs({ mode });
  }

  const recordMenuItems = $derived<MenuEntry[]>([
    {
      kind: "radio",
      id: "insert",
      label: t("record.mode.insert"),
      checked: prefs.mode === "insert",
      testid: "record-menu-insert",
      onselect: () => chooseMode("insert"),
    },
    {
      kind: "radio",
      id: "overwrite",
      label: t("record.mode.overwrite"),
      checked: prefs.mode === "overwrite",
      testid: "record-menu-overwrite",
      onselect: () => chooseMode("overwrite"),
    },
    { kind: "separator", id: "sep-punch" },
    {
      kind: "checkbox",
      id: "punch",
      label: t("record.punch_on_selection"),
      checked: prefs.punch_on_selection,
      testid: "record-menu-punch",
      onselect: () => void setRecordPrefs({ punch_on_selection: !prefs.punch_on_selection }),
    },
  ]);

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
  <div class="cluster" data-tour="record">
    <span class="rec-wrap">
      <Button
        variant="record"
        active={rec.state.recording}
        icon={rec.state.recording ? "stop" : "record"}
        testid="record-button"
        bind:element={recordButton}
        disabled={noInput || rec.state.finishing}
        title={noInput ? t("record.no_input_hint") : t("record.record_title")}
        onclick={() => void toggleRecord()}
        oncontextmenu={onRecordContextMenu}
      >
        {rec.state.recording ? t("record.stop") : t("record.record")}
      </Button>
      <Menu
        open={menuAnchor !== null}
        anchor={menuAnchor}
        items={recordMenuItems}
        label={t("record.menu_title")}
        testid="record-context-menu"
        onclose={() => (menuAnchor = null)}
      />
    </span>
    <span class="elapsed" class:live={rec.state.recording} data-testid="record-elapsed" title={t("record.elapsed_title")}>{elapsed}</span>
    {#if phase}
      <span class="chip record" data-testid="record-phase">{tDynamic(phase.key, phase.params)}</span>
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
      <span class="chip warning" data-testid="record-dropouts" title={t("record.dropouts_title")}>
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
  </div>
  <Separator orientation="vertical" />
  <div class="cluster">
    <Button
      variant="ghost"
      icon={rec.state.armed ? "input" : "inputOff"}
      testid="record-arm"
      aria-pressed={rec.state.armed}
      disabled={noInput || busy}
      title={noInput ? t("record.no_input_hint") : t("record.arm_title")}
      onclick={() => void toggleArm()}
    >
      {t("record.arm")}
    </Button>
    <label class="monitor" title={t("record.monitor")}>
      <Icon name="monitor" size="sm" />
      <span class="monitor-label">{t("record.monitor")}</span>
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
    <span class="punch-wrap" data-tour="punch">
      <IconButton
        icon="punch"
        label={t("record.punch_section")}
        testid="record-punch-toggle"
        aria-haspopup="dialog"
        bind:element={punchToggle}
        pressed={panelOpen}
        aria-expanded={panelOpen}
        onclick={togglePanel}
      />
      <Popover
        open={panelOpen}
        anchor={punchToggle}
        placement="bottom-end"
        label={t("record.punch_section")}
        testid="record-punch-panel"
        onclose={() => (panelOpen = false)}
      >
        <div class="panel">
          <div class="panel-title">
            <span>{t("record.punch_section")}</span>
            <span class="panel-title-actions">
              <HelpButton doc="user-guide" section="re-recording-part-of-a-take-punch-in" />
              <TourButton tour="punch" />
            </span>
          </div>
          <div class="row" role="group" aria-label={t("record.mode")} title={t("record.mode_title")}>
            <span class="row-label">{t("record.mode")}</span>
            <div class="segments">
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
          </div>
          <label class="row">
            <span class="row-label">{t("record.preroll")}</span>
            <span class="number">
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
              <span class="unit">{t("record.seconds")}</span>
            </span>
          </label>
          <label class="row">
            <span class="row-label">{t("record.postroll")}</span>
            <span class="number">
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
              <span class="unit">{t("record.seconds")}</span>
            </span>
          </label>
          <label class="row">
            <span class="row-label">{t("record.xfade")}</span>
            <span class="number">
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
              <span class="unit">{t("record.milliseconds")}</span>
            </span>
          </label>
          <label class="check-row">
            <input
              type="checkbox"
              data-testid="record-punch-on-selection"
              checked={prefs.punch_on_selection}
              disabled={busy}
              onchange={(e) => void setRecordPrefs({ punch_on_selection: e.currentTarget.checked })}
            />
            <span>{t("record.punch_on_selection")}</span>
          </label>
          <label class="check-row">
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
          <label class="check-row">
            <input
              type="checkbox"
              data-testid="record-hear-original"
              checked={prefs.hear_original}
              disabled={busy}
              onchange={(e) => void setRecordPrefs({ hear_original: e.currentTarget.checked })}
            />
            <span>{t("record.hear_original")}</span>
          </label>
          <div class="divider"></div>
          <div class="row">
            <span class="row-label">{t("record.offset.label")}</span>
            <span class="offset-value" data-testid="record-offset">{tDynamic(readout.key, readout.params)}</span>
            <Button
              size="sm"
              testid="record-calibrate"
              disabled={busy || rec.offset?.available === false}
              onclick={openCalibration}
            >
              {t("record.offset.calibrate")}
            </Button>
          </div>
          {#if hint}
            <p class="hint warning" data-testid="record-offset-hint">
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
              class="offset-entry"
              data-testid="record-offset-entry"
              title={t("record.offset.entry_title")}
              placeholder={t("record.offset.placeholder")}
              bind:value={offsetText}
              disabled={busy || rec.offset?.available === false}
            />
            <Button size="sm" type="submit" testid="record-offset-set" disabled={busy}>
              {t("record.offset.set")}
            </Button>
          </form>
          {#if offsetInvalid}
            <p class="hint warning">{t("record.offset.invalid")}</p>
          {/if}
        </div>
      </Popover>
    </span>
  </div>
</div>

<style>
  .record {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-2);
    font-family: var(--pv-font-sans);
  }

  .cluster {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .rec-wrap,
  .punch-wrap {
    position: relative;
    display: inline-flex;
  }

  /* The take's elapsed time: muted at rest, the record colour on air. */
  .elapsed {
    min-width: 7ch;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-md);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .elapsed.live {
    color: var(--pv-record-text);
    font-weight: var(--pv-weight-medium);
  }

  .chip,
  .disk-remaining,
  .monitor-latency {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding-inline: calc(var(--pv-space-1) + var(--pv-space-half));
    border-radius: var(--pv-radius-sm);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .chip.record {
    background: var(--pv-record-soft);
    color: var(--pv-record-text);
    font-weight: var(--pv-weight-medium);
  }

  .chip.warning {
    background: var(--pv-warning-soft);
    color: var(--pv-warning-text);
  }

  .disk-remaining {
    padding-inline: 0;
    color: var(--pv-text-tertiary);
  }

  .disk-remaining.low {
    padding-inline: calc(var(--pv-space-1) + var(--pv-space-half));
    background: var(--pv-warning-soft);
    color: var(--pv-warning-text);
  }

  /* CLIP: a hardware-style lamp — dim when clear, solid red when latched (click to reset). */
  .clip {
    height: 20px;
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-tertiary);
    font-family: inherit;
    font-size: 10px;
    font-weight: var(--pv-weight-semibold);
    letter-spacing: 0.04em;
    cursor: default;
  }

  .clip.lit {
    border-color: var(--pv-record-fill);
    background: var(--pv-record-fill);
    color: var(--pv-text-on-record);
  }

  .clip:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .monitor {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    white-space: nowrap;
  }

  .monitor select {
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
  }

  .monitor select:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .monitor-latency {
    padding-inline: 0;
    color: var(--pv-text-tertiary);
  }

  .monitor-latency.amber,
  .monitor-latency.red {
    padding-inline: calc(var(--pv-space-1) + var(--pv-space-half));
  }

  .monitor-latency.amber {
    background: var(--pv-warning-soft);
    color: var(--pv-warning-text);
  }

  .monitor-latency.red {
    background: var(--pv-danger-soft);
    color: var(--pv-danger-text);
  }

  /* Narrow transport bars drop the words that have an icon next to them. */
  @container toolbar (max-width: 1240px) {
    .monitor-label {
      display: none;
    }
  }

  /* The Punch & pre-roll panel's content (the Popover draws the surface). */
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    width: 22rem;
    max-width: 100%;
  }

  .panel-title-actions {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .panel-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--pv-space-2);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-semibold);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-control-h-md);
  }

  .row-label {
    flex: none;
    width: 7.5rem;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .segments {
    display: inline-flex;
    gap: var(--pv-space-half);
    padding: var(--pv-space-half);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-track);
  }

  .segments button {
    height: 22px;
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-secondary);
    font: inherit;
    font-size: var(--pv-text-sm);
    cursor: default;
  }

  .segments button.active {
    background: var(--pv-control-bg-selected);
    color: var(--pv-text-primary);
    box-shadow: var(--pv-shadow-1);
  }

  .segments button:disabled {
    color: var(--pv-text-disabled);
  }

  .number {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .number input,
  .offset-entry {
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    font-variant-numeric: tabular-nums;
  }

  .number input {
    width: 5rem;
    text-align: right;
  }

  .offset-entry {
    flex: 1;
    min-width: 0;
  }

  .unit {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-hit-min);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
  }

  .check-row input {
    width: 14px;
    height: 14px;
    margin: 0;
    accent-color: var(--pv-accent);
  }

  .offset-value {
    flex: 1;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
  }

  .divider {
    height: var(--pv-border-width);
    margin-block: var(--pv-space-1);
    background: var(--pv-border-subtle);
  }

  .hint {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }

  .hint.warning {
    color: var(--pv-warning-text);
  }
</style>
