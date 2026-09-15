<script lang="ts">
  import { documentState } from "../document/document.svelte";
  import { t } from "../i18n";
  import { selectionState, setSelectionFromResult } from "../state/selection.svelte";
  import { timeRulerFormatState } from "../state/waveformView.svelte";
  import { formatDocumentTime, parseDocumentTime } from "../waveform/timeFormat";

  /**
   * Selection start/end/length readouts (T-206, SPEC-006 §2.2/§2.9's "selection start/end/length
   * readouts that you can edit, with units"): three text fields in the toolbar, shown only while
   * there's a non-empty selection — same "disabled/absent with no selection" spirit as the Edit
   * menu's Cut/Copy/Delete (SPEC-008 §2.2). Each field goes through the shared
   * `waveform/timeFormat.ts` formatter/parser, so it always matches the ruler/toolbar clock's
   * current `time_ruler_format`.
   *
   * Editing Start or End moves that boundary directly (clamped to `[0, len_samples]`, and to
   * stay on the correct side of the other boundary); editing Length keeps Start fixed and moves
   * End. Selection is view/UI state (SPEC-006 §2.2), so an edit here never goes through IPC — it
   * updates `state/selection.svelte.ts` exactly like a pointer gesture would (and is picked up by
   * the same sidecar persistence effect, `EditorView.svelte`).
   */

  const doc = documentState();
  const selection = selectionState();
  const timeFormat = timeRulerFormatState();

  const rateHz = $derived(doc.current.sample_rate_hz);
  const lenSamples = $derived(doc.current.len_samples);
  const sel = $derived(selection.current);

  let draftStart = $state<string | null>(null);
  let draftEnd = $state<string | null>(null);
  let draftLength = $state<string | null>(null);

  const startText = $derived(
    draftStart ?? (sel ? formatDocumentTime(sel.startSample, rateHz, timeFormat.current) : ""),
  );
  const endText = $derived(
    draftEnd ?? (sel ? formatDocumentTime(sel.endSample, rateHz, timeFormat.current) : ""),
  );
  const lengthText = $derived(
    draftLength ??
      (sel ? formatDocumentTime(sel.endSample - sel.startSample, rateHz, timeFormat.current) : ""),
  );

  function clampSample(value: number): number {
    return Math.min(Math.max(Math.round(value), 0), lenSamples);
  }

  /** Invalid or out-of-order input is silently rejected (the field snaps back to the current
   * value on the next render) — same "no error, just don't apply it" spirit as SPEC-006 §2.10's
   * snap-with-no-crossing case, since a partially-typed value is a normal, frequent state here. */
  function commitStart(text: string): void {
    draftStart = null;
    if (!sel) {
      return;
    }
    const parsed = parseDocumentTime(text, rateHz, timeFormat.current);
    if (parsed === null) {
      return;
    }
    const next = clampSample(parsed);
    if (next >= sel.endSample) {
      return;
    }
    setSelectionFromResult([next, sel.endSample]);
  }

  function commitEnd(text: string): void {
    draftEnd = null;
    if (!sel) {
      return;
    }
    const parsed = parseDocumentTime(text, rateHz, timeFormat.current);
    if (parsed === null) {
      return;
    }
    const next = clampSample(parsed);
    if (next <= sel.startSample) {
      return;
    }
    setSelectionFromResult([sel.startSample, next]);
  }

  function commitLength(text: string): void {
    draftLength = null;
    if (!sel) {
      return;
    }
    const parsed = parseDocumentTime(text, rateHz, timeFormat.current);
    if (parsed === null || parsed <= 0) {
      return;
    }
    const next = clampSample(sel.startSample + parsed);
    if (next <= sel.startSample) {
      return;
    }
    setSelectionFromResult([sel.startSample, next]);
  }

  function onKeydown(event: KeyboardEvent, commit: () => void, cancel: () => void): void {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
    } else if (event.key === "Escape") {
      event.stopPropagation();
      cancel();
    }
  }
</script>

{#if sel}
  <div class="selection-readout" data-testid="selection-readout" aria-label={t("selection.readout_label")}>
    <label class="field">
      <span class="label">{t("selection.start")}</span>
      <input
        type="text"
        inputmode="decimal"
        autocomplete="off"
        spellcheck="false"
        data-testid="selection-start"
        value={startText}
        oninput={(e) => (draftStart = e.currentTarget.value)}
        onkeydown={(e) => onKeydown(e, () => commitStart(draftStart ?? startText), () => (draftStart = null))}
        onblur={() => draftStart !== null && commitStart(draftStart)}
      />
    </label>
    <label class="field">
      <span class="label">{t("selection.end")}</span>
      <input
        type="text"
        inputmode="decimal"
        autocomplete="off"
        spellcheck="false"
        data-testid="selection-end"
        value={endText}
        oninput={(e) => (draftEnd = e.currentTarget.value)}
        onkeydown={(e) => onKeydown(e, () => commitEnd(draftEnd ?? endText), () => (draftEnd = null))}
        onblur={() => draftEnd !== null && commitEnd(draftEnd)}
      />
    </label>
    <label class="field">
      <span class="label">{t("selection.length")}</span>
      <input
        type="text"
        inputmode="decimal"
        autocomplete="off"
        spellcheck="false"
        data-testid="selection-length"
        value={lengthText}
        oninput={(e) => (draftLength = e.currentTarget.value)}
        onkeydown={(e) =>
          onKeydown(e, () => commitLength(draftLength ?? lengthText), () => (draftLength = null))}
        onblur={() => draftLength !== null && commitLength(draftLength)}
      />
    </label>
  </div>
{/if}

<style>
  .selection-readout {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    font-family: var(--pv-font-sans);
  }

  .field {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    height: var(--pv-control-h-sm);
    padding-inline: var(--pv-control-px-sm);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
  }

  .field:focus-within {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .label {
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    color: var(--pv-text-tertiary);
    white-space: nowrap;
  }

  input {
    width: 8ch;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-variant-numeric: tabular-nums;
    outline: none;
  }
</style>
