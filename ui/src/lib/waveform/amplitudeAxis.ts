/**
 * Canvas-free amplitude-ruler math for the waveform's left gutter (H-24 item 7; SPEC-006 §2.4/
 * §4.2). Fixed at `verticalZoom = 1` for now — SPEC-006 §2.2's `verticalZoom`/drag-to-zoom-the-
 * gutter is its own feature and out of this ticket's scope (the ticket only asks for "an
 * amplitude scale on the left gutter ... horizontal grid lines ... a zero line"), but the
 * function still takes `verticalZoom` so a later ticket can wire the gutter-drag zoom straight in
 * without reshaping this module.
 *
 * **Deviation from SPEC-006 §2.4:** the ticket's own enumeration ("0, −3, −6, −12, −18, −24, −∞")
 * both includes a −3 dB tick the spec's fixed ladder doesn't have and labels −∞, which §2.4
 * explicitly forbids ("−∞ is never labeled as a tick; the centerline ... is unlabeled"). This
 * module follows SPEC-006 §2.4's actual fixed ladder (0, −6, −12, −18, −24, −36, −48, ...) and
 * leaves the centerline unlabeled instead — see the H-24 ticket report for why.
 */

export interface AmplitudeTick {
  /** Pixel y (0 = top, `heightPx` = bottom). */
  y: number;
  label: string;
  db: number;
}

/** SPEC-006 §2.4's fixed dBFS tick set, most of a waveform view's usable dynamic range. Beyond
 * this the pixel gap under any sane `verticalZoom` is too small to label anyway. */
const DBFS_TICKS_DB = [0, -6, -12, -18, -24, -36, -48, -60, -72, -84, -96, -108, -120] as const;

/** Pixel y (0 = top, `heightPx` = bottom) for `db` at `verticalZoom` (SPEC-006 §2.4: `y = centerY
 * − amplitude × verticalZoom × halfHeightPx`, evaluated for the positive-amplitude side). */
export function yForAmplitudeDb(db: number, verticalZoom: number, heightPx: number): number {
  const centerY = heightPx / 2;
  const amp = 10 ** (db / 20) * verticalZoom;
  return centerY - amp * centerY;
}

/** The centerline (0 amplitude) — SPEC-006 §2.4: "unlabeled (it isn't a dB value)", drawn as the
 * ticket's "zero line" instead. */
export function centerlineY(heightPx: number): number {
  return heightPx / 2;
}

/**
 * dBFS ticks for the amplitude gutter (SPEC-006 §2.4/§4.2), mirrored above and below the
 * centerline (positive and negative peaks read the same dB magnitude), thinned by `minLabelGapPx`
 * working outward from the top/bottom edges toward the (always unlabeled) centerline so the
 * loudest, most useful ticks (0, −6, −12 dBFS) are the ones that survive thinning at a small
 * `heightPx`.
 */
export function amplitudeTicksDbfs(
  heightPx: number,
  verticalZoom: number,
  minLabelGapPx: number,
): AmplitudeTick[] {
  if (!(heightPx > 0) || !(verticalZoom > 0)) {
    return [];
  }
  const half = thinnedHalf(heightPx, verticalZoom, minLabelGapPx, false);
  const mirrored = thinnedHalf(heightPx, verticalZoom, minLabelGapPx, true);
  return [...half, ...mirrored].sort((a, b) => a.y - b.y);
}

function thinnedHalf(
  heightPx: number,
  verticalZoom: number,
  minLabelGapPx: number,
  bottom: boolean,
): AmplitudeTick[] {
  const centerY = heightPx / 2;
  const out: AmplitudeTick[] = [];
  let lastY: number | null = null;
  for (const db of DBFS_TICKS_DB) {
    const amp = 10 ** (db / 20) * verticalZoom;
    if (amp > 1) {
      continue; // beyond the top/bottom edge at this zoom — not visible
    }
    const y = bottom ? centerY + amp * centerY : centerY - amp * centerY;
    if (lastY === null || Math.abs(y - lastY) >= minLabelGapPx) {
      out.push({ y, label: db === 0 ? "0" : String(db), db });
      lastY = y;
    }
  }
  return out;
}
