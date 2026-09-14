<script lang="ts" module>
  let uid = 0;
</script>

<script lang="ts">
  import { t } from "../i18n";
  import { snapToStep } from "./slider";
  import { formatNumber, formatWithUnit, parseNumber } from "./units";

  /**
   * Numeric entry with a unit (H-25): normalize targets, pre/post-roll, crossfade, offsets. A
   * text input with `role="spinbutton"` (so "−3.5 dB", "−3,5" and "2.5 kHz" parse) — never
   * `type="number"` (no locale-safe minus, spinner arrows that don't fit the look). Enter or blur
   * commits (out-of-range clamps), Escape reverts, ArrowUp/Down step (Shift ×10). Unparseable
   * text is flagged with a message naming the range and unit.
   */
  let {
    value = $bindable(0),
    min = Number.NEGATIVE_INFINITY,
    max = Number.POSITIVE_INFINITY,
    step = 1,
    bigStep,
    decimals,
    unit = "",
    signed = false,
    label,
    hideLabel = false,
    layout = "inline",
    size = "md",
    width = "6em",
    disabled = false,
    testid,
    onchange,
  }: {
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    bigStep?: number;
    decimals?: number;
    unit?: string;
    signed?: boolean;
    label: string;
    hideLabel?: boolean;
    layout?: "inline" | "stacked";
    size?: "sm" | "md";
    width?: string;
    disabled?: boolean;
    testid?: string;
    onchange?: (value: number) => void;
  } = $props();

  uid += 1;
  const id = `pv-number-${uid}`;
  const places = $derived(decimals ?? Math.max(0, (String(step).split(".")[1] ?? "").length));
  const formatted = $derived(formatNumber(value, places, { signed }));
  let draft = $state<string | null>(null);
  const shown = $derived(draft ?? formatted);
  const invalid = $derived(draft !== null && parseNumber(draft, unit) === null);
  const message = $derived(
    t("ui.number.invalid", {
      min: Number.isFinite(min) ? formatWithUnit(min, unit, places) : "−∞",
      max: Number.isFinite(max) ? formatWithUnit(max, unit, places) : "+∞",
    }),
  );

  function commitValue(next: number): void {
    const clamped = Number.isFinite(min) || Number.isFinite(max)
      ? Math.min(max, Math.max(min, next))
      : next;
    const snapped = Number.isFinite(min) && Number.isFinite(max) ? snapToStep(clamped, min, max, step) : clamped;
    draft = null;
    if (snapped !== value) {
      value = snapped;
    }
    onchange?.(snapped);
  }

  function commitDraft(): boolean {
    if (draft === null) {
      return true;
    }
    const parsed = parseNumber(draft, unit);
    if (parsed === null) {
      return false;
    }
    commitValue(parsed);
    return true;
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      commitDraft();
    } else if (event.key === "Escape") {
      if (draft !== null) {
        event.stopPropagation();
        draft = null;
      }
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const base = draft !== null ? (parseNumber(draft, unit) ?? value) : value;
      const delta = (event.shiftKey ? (bigStep ?? step * 10) : step) * (event.key === "ArrowUp" ? 1 : -1);
      commitValue(base + delta);
    }
  }

  function onBlur(): void {
    if (!commitDraft()) {
      draft = null;
    }
  }
</script>

<div class="pv-number" data-layout={layout} data-size={size}>
  <label for={id} class="label" class:visually-hidden={hideLabel}>{label}</label>
  <span class="field" class:invalid style:width>
    <input
      {id}
      type="text"
      inputmode="decimal"
      role="spinbutton"
      autocomplete="off"
      spellcheck="false"
      value={shown}
      aria-valuenow={value}
      aria-valuemin={Number.isFinite(min) ? min : undefined}
      aria-valuemax={Number.isFinite(max) ? max : undefined}
      aria-valuetext={formatWithUnit(value, unit, places, { signed })}
      aria-invalid={invalid ? "true" : undefined}
      aria-describedby={invalid ? `${id}-msg` : undefined}
      data-testid={testid}
      {disabled}
      oninput={(e) => (draft = e.currentTarget.value)}
      onkeydown={onKeydown}
      onblur={onBlur}
    />
    {#if unit}<span class="unit">{unit}</span>{/if}
  </span>
  {#if invalid}
    <span class="message" id="{id}-msg" aria-live="polite">{message}</span>
  {/if}
</div>

<style>
  .pv-number {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
    min-width: 0;
    font-family: var(--pv-font-sans);
  }

  .pv-number[data-layout="stacked"] {
    flex-direction: column;
    align-items: stretch;
    gap: var(--pv-space-1);
  }

  .label {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-secondary);
    white-space: nowrap;
  }

  .field {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-sm) var(--pv-control-px-md);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-field-bg);
    transition: border-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-number[data-size="sm"] .field {
    height: var(--pv-control-h-sm);
    border-radius: var(--pv-radius-sm);
  }

  .field:focus-within {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .field.invalid {
    border-color: var(--pv-danger-text);
  }

  input {
    flex: 1;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    font-variant-numeric: tabular-nums;
    text-align: right;
    outline: none;
  }

  .pv-number[data-size="sm"] input {
    font-size: var(--pv-text-sm);
  }

  input:disabled {
    color: var(--pv-text-disabled);
  }

  .unit {
    flex: none;
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    color: var(--pv-text-tertiary);
  }

  .message {
    flex-basis: 100%;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-danger-text);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
