/** Pure slider math (H-25): kept out of `Slider.svelte` so it's testable without layout. */

function decimalsOf(step: number): number {
  const text = String(step);
  const dot = text.indexOf(".");
  return dot < 0 ? 0 : text.length - dot - 1;
}

/** Clamps to `[min, max]` and snaps to `min + k·step` (no float dust: 0.1 + 0.2 → 0.3). */
export function snapToStep(value: number, min: number, max: number, step: number): number {
  const clamped = Math.min(max, Math.max(min, value));
  if (step <= 0) {
    return clamped;
  }
  const snapped = min + Math.round((clamped - min) / step) * step;
  const rounded = Number(snapped.toFixed(decimalsOf(step) + decimalsOf(min)));
  return Math.min(max, Math.max(min, rounded));
}

export function fractionOf(value: number, min: number, max: number): number {
  if (max <= min) {
    return 0;
  }
  return Math.min(1, Math.max(0, (value - min) / (max - min)));
}

/** Pointer x (px from the track's left edge) → snapped value. */
export function valueAtPosition(
  x: number,
  width: number,
  min: number,
  max: number,
  step: number,
): number {
  if (width <= 0) {
    return min;
  }
  const f = Math.min(1, Math.max(0, x / width));
  return snapToStep(min + f * (max - min), min, max, step);
}

export interface SliderRange {
  min: number;
  max: number;
  step: number;
  bigStep: number;
}

/** Keyboard (WAI-ARIA slider pattern + Shift for a big step) → next value, or `null`. */
export function keyToValue(
  key: string,
  shift: boolean,
  value: number,
  { min, max, step, bigStep }: SliderRange,
): number | null {
  const small = shift ? bigStep : step;
  switch (key) {
    case "ArrowRight":
    case "ArrowUp":
      return snapToStep(value + small, min, max, step);
    case "ArrowLeft":
    case "ArrowDown":
      return snapToStep(value - small, min, max, step);
    case "PageUp":
      return snapToStep(value + bigStep, min, max, step);
    case "PageDown":
      return snapToStep(value - bigStep, min, max, step);
    case "Home":
      return min;
    case "End":
      return max;
    default:
      return null;
  }
}
