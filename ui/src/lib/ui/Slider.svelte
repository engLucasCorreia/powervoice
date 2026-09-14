<script lang="ts">
  import { fractionOf, keyToValue, snapToStep, valueAtPosition } from "./slider";
  import { formatWithUnit } from "./units";

  /**
   * Horizontal slider (H-25, WAI-ARIA slider): drag, click-to-jump, arrows ±step, Shift+arrows /
   * PageUp/PageDown ±bigStep, Home/End, double-click resets to `defaultValue`. The value text
   * always carries its unit (aria-valuetext + the readout). Ranges spanning zero (gain, EQ) fill
   * from the zero point. `oninput` fires live while dragging; `onchange` commits (keyboard steps,
   * pointer release, reset).
   */
  let {
    value = $bindable(0),
    min,
    max,
    step = 1,
    bigStep,
    unit = "",
    decimals,
    format,
    defaultValue,
    label,
    showValue = true,
    disabled = false,
    width,
    testid,
    oninput,
    onchange,
  }: {
    value?: number;
    min: number;
    max: number;
    step?: number;
    bigStep?: number;
    unit?: string;
    decimals?: number;
    format?: (value: number) => string;
    defaultValue?: number;
    label: string;
    showValue?: boolean;
    disabled?: boolean;
    width?: string;
    testid?: string;
    oninput?: (value: number) => void;
    onchange?: (value: number) => void;
  } = $props();

  const places = $derived(decimals ?? Math.max(0, (String(step).split(".")[1] ?? "").length));
  const text = $derived(format ? format(value) : formatWithUnit(value, unit, places));
  const f = $derived(fractionOf(value, min, max));
  const bipolar = $derived(min < 0 && max > 0);
  const origin = $derived(bipolar ? fractionOf(0, min, max) : 0);
  const fillLeft = $derived(Math.min(origin, f) * 100);
  const fillWidth = $derived(Math.abs(f - origin) * 100);

  let dragging = false;

  function set(next: number, commit: boolean): void {
    if (next !== value) {
      value = next;
      oninput?.(next);
    }
    if (commit) {
      onchange?.(next);
    }
  }

  function fromPointer(event: PointerEvent, el: HTMLElement): number {
    const rect = el.getBoundingClientRect();
    return valueAtPosition(event.clientX - rect.left, rect.width, min, max, step);
  }

  function onPointerDown(event: PointerEvent): void {
    if (disabled || event.button !== 0) {
      return;
    }
    const el = event.currentTarget as HTMLElement;
    dragging = true;
    el.setPointerCapture?.(event.pointerId);
    el.focus();
    set(fromPointer(event, el), false);
  }

  function onPointerMove(event: PointerEvent): void {
    if (dragging) {
      set(fromPointer(event, event.currentTarget as HTMLElement), false);
    }
  }

  function onPointerUp(event: PointerEvent): void {
    if (!dragging) {
      return;
    }
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
    onchange?.(value);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (disabled) {
      return;
    }
    const next = keyToValue(event.key, event.shiftKey, value, {
      min,
      max,
      step,
      bigStep: bigStep ?? step * 10,
    });
    if (next === null) {
      return;
    }
    event.preventDefault();
    set(next, true);
  }

  function onDoubleClick(): void {
    if (!disabled && defaultValue !== undefined) {
      set(snapToStep(defaultValue, min, max, step), true);
    }
  }
</script>

<div class="pv-slider" style:width data-disabled={disabled ? "true" : undefined}>
  <div
    class="track"
    role="slider"
    tabindex={disabled ? -1 : 0}
    aria-label={label}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={value}
    aria-valuetext={text}
    aria-orientation="horizontal"
    aria-disabled={disabled ? "true" : undefined}
    data-bipolar={bipolar ? "true" : undefined}
    data-testid={testid}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onkeydown={onKeydown}
    ondblclick={onDoubleClick}
  >
    <span class="rail"></span>
    <span class="fill" style:left="{fillLeft}%" style:width="{fillWidth}%"></span>
    {#if bipolar}<span class="zero" style:left="{origin * 100}%"></span>{/if}
    <span class="thumb" style:left="{f * 100}%"></span>
  </div>
  {#if showValue}<span class="value">{text}</span>{/if}
</div>

<style>
  .pv-slider {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-width: 0;
    font-family: var(--pv-font-sans);
  }

  .track {
    position: relative;
    flex: 1;
    min-width: 64px;
    height: var(--pv-control-h-sm);
    touch-action: none;
    cursor: default;
    outline: none;
  }

  .rail,
  .fill {
    position: absolute;
    top: 50%;
    height: 4px;
    border-radius: var(--pv-radius-full);
    transform: translateY(-50%);
  }

  .rail {
    left: 0;
    right: 0;
    background: var(--pv-control-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .fill {
    background: var(--pv-accent);
  }

  .zero {
    position: absolute;
    top: 50%;
    width: 1px;
    height: 10px;
    background: var(--pv-border-strong);
    transform: translate(-50%, -50%);
  }

  .thumb {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    border: 2px solid var(--pv-accent);
    border-radius: var(--pv-radius-full);
    background: var(--pv-text-primary);
    box-shadow: var(--pv-shadow-1);
    transform: translate(-50%, -50%);
    transition: transform var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .track:hover .thumb {
    transform: translate(-50%, -50%) scale(1.15);
  }

  .track:focus-visible .thumb {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .pv-slider[data-disabled="true"] .fill,
  .pv-slider[data-disabled="true"] .thumb {
    background: var(--pv-text-disabled);
    border-color: var(--pv-text-disabled);
  }

  .value {
    flex: none;
    min-width: 5.5em;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-variant-numeric: tabular-nums;
    color: var(--pv-text-secondary);
    text-align: right;
  }
</style>
