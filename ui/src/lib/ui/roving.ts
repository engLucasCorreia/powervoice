/**
 * Roving-tabindex navigation shared by Tabs and SegmentedControl (WAI-ARIA APG tabs/radio group
 * patterns): arrows move to the next enabled item and wrap, Home/End jump to the first/last
 * enabled item. `orientation` limits which arrows apply; `"both"` (default) accepts all four.
 */
export type Orientation = "horizontal" | "vertical" | "both";

export function nextRovingIndex(
  current: number,
  key: string,
  disabled: readonly boolean[],
  orientation: Orientation = "both",
): number | null {
  const count = disabled.length;
  const enabled = (i: number) => disabled[i] === false;
  if (count === 0 || !disabled.some((d) => !d)) {
    return null;
  }
  const horizontal = orientation !== "vertical";
  const vertical = orientation !== "horizontal";
  let step = 0;
  if ((key === "ArrowRight" && horizontal) || (key === "ArrowDown" && vertical)) {
    step = 1;
  } else if ((key === "ArrowLeft" && horizontal) || (key === "ArrowUp" && vertical)) {
    step = -1;
  } else if (key === "Home") {
    for (let i = 0; i < count; i++) {
      if (enabled(i)) return i;
    }
    return null;
  } else if (key === "End") {
    for (let i = count - 1; i >= 0; i--) {
      if (enabled(i)) return i;
    }
    return null;
  } else {
    return null;
  }
  let index = current;
  for (let n = 0; n < count; n++) {
    index = (index + step + count) % count;
    if (enabled(index)) {
      return index;
    }
  }
  return null;
}
