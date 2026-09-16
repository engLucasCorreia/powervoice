/**
 * H-47: the per-frame GPU upload budget and the order tiles are uploaded in.
 *
 * A burst of spectrogram tiles (SPEC-007 §4.6 serves up to 64 per request) all arriving between
 * two frames would otherwise be uploaded in a single frame — several MiB of `texImage2D` on the
 * renderer's critical path. Instead each frame uploads at most {@link UploadBudget.maxTiles} tiles
 * and {@link UploadBudget.maxBytes} bytes, and the rest are uploaded on the following frames; the
 * renderer keeps asking the H-43 frame scheduler for frames while a backlog remains, so the work
 * drains at the display rate instead of in one spike. A tile whose texture hasn't been uploaded
 * yet simply isn't drawn — the same "pending" look SPEC-007 §2.8 already specifies for a tile that
 * hasn't arrived.
 *
 * Priority (SPEC-007 §2.8's own "visible first" order, extended to the newest arrivals):
 * 1. tiles overlapping the viewport, before tiles in the off-screen margin;
 * 2. visible tiles **newest first** — the tiles the user just scrolled or zoomed onto are the ones
 *    whose absence is visible — with the distance from the viewport centre as the tie-break;
 * 3. off-screen tiles nearest the viewport first (their age doesn't matter; nothing shows them).
 *
 * Pure functions with no WebGL, so the order and the budget are testable without a GL context.
 */

/** One tile waiting for its texture upload. */
export interface UploadCandidate {
  /** The tile's cache key (FFT size, hop, index). */
  key: string;
  /** Payload bytes the upload will transfer. */
  bytes: number;
  /** Device-pixel column range `[x0, x1)` the tile covers this frame (may fall outside it). */
  x0: number;
  x1: number;
  /** Arrival order: larger is newer. */
  seq: number;
}

export interface UploadBudget {
  /** At most this many tiles per frame. */
  maxTiles: number;
  /** At most this many payload bytes per frame (the first tile always goes, see {@link planUploads}). */
  maxBytes: number;
}

/**
 * Two tiles and 2 MiB per frame. A 2048-point FFT tile is 256 × 1025 = 256 KiB, a 16 384-point one
 * 2 MiB, so this is "a few tiles per frame at every FFT size" — enough to fill a fresh viewport in
 * two or three frames, small enough that no single frame pays for a whole 64-tile request.
 */
export const DEFAULT_UPLOAD_BUDGET: UploadBudget = { maxTiles: 2, maxBytes: 2 * 1024 * 1024 };

export interface UploadPlan {
  /** To upload this frame, in the order they should go. */
  upload: UploadCandidate[];
  /** Left for the following frames, in priority order (feed straight back in next frame). */
  deferred: UploadCandidate[];
  /** Bytes still waiting after this frame (diagnostics). */
  deferredBytes: number;
}

/** The upload order described in the module comment. Does not mutate `candidates`. */
export function prioritizeUploads(
  candidates: readonly UploadCandidate[],
  viewportWidthPx: number,
): UploadCandidate[] {
  const centre = viewportWidthPx / 2;
  const visible = (c: UploadCandidate): boolean => c.x1 > 0 && c.x0 < viewportWidthPx;
  /** How far outside the viewport the tile is (0 when it overlaps it). */
  const outside = (c: UploadCandidate): number => Math.max(0, c.x0 - viewportWidthPx, -c.x1);
  const centreDistance = (c: UploadCandidate): number => Math.abs((c.x0 + c.x1) / 2 - centre);
  return [...candidates].sort((a, b) => {
    const av = visible(a);
    const bv = visible(b);
    if (av !== bv) {
      return av ? -1 : 1;
    }
    if (av) {
      return b.seq - a.seq || centreDistance(a) - centreDistance(b) || (a.key < b.key ? -1 : 1);
    }
    return outside(a) - outside(b) || (a.key < b.key ? -1 : 1);
  });
}

/**
 * Splits `candidates` into what fits this frame's `budget` and what waits, in
 * {@link prioritizeUploads} order. The highest-priority candidate is always uploaded even if it
 * alone exceeds `maxBytes`, so a tile larger than the whole budget can never stall the backlog.
 */
export function planUploads(
  candidates: readonly UploadCandidate[],
  budget: UploadBudget,
  viewportWidthPx: number,
): UploadPlan {
  const ordered = prioritizeUploads(candidates, viewportWidthPx);
  const upload: UploadCandidate[] = [];
  const deferred: UploadCandidate[] = [];
  let bytes = 0;
  for (const candidate of ordered) {
    const fits =
      upload.length === 0 ||
      (upload.length < budget.maxTiles && bytes + candidate.bytes <= budget.maxBytes);
    if (fits && deferred.length === 0) {
      upload.push(candidate);
      bytes += candidate.bytes;
    } else {
      deferred.push(candidate);
    }
  }
  return { upload, deferred, deferredBytes: deferred.reduce((sum, c) => sum + c.bytes, 0) };
}
