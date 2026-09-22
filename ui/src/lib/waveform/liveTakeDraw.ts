/**
 * H-114: live-take bucket extension, shared by both waveform renderers (`WaveformView.svelte`'s
 * `drawCanvas2d`/`drawWebgl2` — ADR-009 §2/§4 "the two renderers agree pixel-for-pixel", so this
 * lives here once rather than being duplicated per renderer).
 *
 * `record_peaks_get` polls at `LIVE_POLL_MS`, so the last response applied (`liveBuckets`) can be
 * up to one whole poll interval stale by the time a frame draws it — meanwhile the record head
 * (`rec.elapsedSamples`) moves every telemetry tick (H-43: full rate while recording). Left alone,
 * the drawn wave visibly lags the head by that gap and "catches up" in a visible step each time a
 * response lands, instead of animating at display rate (H-114 Scope item 3: "the growing take
 * should animate at display rate, not advance in visible jumps every poll").
 *
 * {@link extendLiveBucketsToHead} closes that gap every frame by appending one synthetic bucket —
 * a hold of the last real bucket's amplitude — out to wherever the head currently is. It's a
 * cosmetic best-effort extrapolation only (never fed back into `liveBuckets`/`liveStartSample`
 * state, which stay exactly what the backend reported): the true amplitude for the gap is simply
 * not known yet, so holding the last known one reads as continuous growth rather than a stall, and
 * self-corrects the moment the next poll's real data arrives.
 */
export function extendLiveBucketsToHead(
  buckets: ReadonlyArray<readonly [number, number]>,
  startSample: number,
  spb: number,
  headSample: number,
): ReadonlyArray<readonly [number, number]> {
  if (buckets.length === 0 || spb <= 0) {
    return buckets;
  }
  // The real data's own bucket size is a fixed-width grid; the last bucket may be the
  // in-progress one (fewer than `spb` samples actually captured), so this slightly overestimates
  // where real data ends — a few tens of samples out of a poll interval's worth, imperceptible.
  const dataEndSample = startSample + buckets.length * spb;
  const gapSamples = headSample - dataEndSample;
  if (gapSamples <= 0) {
    return buckets;
  }
  const extraBuckets = Math.ceil(gapSamples / spb);
  const held = buckets[buckets.length - 1];
  const extension: Array<readonly [number, number]> = new Array(extraBuckets).fill(held);
  return buckets.concat(extension);
}
