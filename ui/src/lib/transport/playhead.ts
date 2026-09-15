/**
 * Playhead display contract (SPEC-003 §2.2, ADR-003 §3), shared by the transport readout and the
 * waveform view (S1-03):
 * - the engine sends anchors `(sample, timeNs, rate)` in `VXTM` frames (heard position, app clock);
 * - each animation frame shows `sample + (now − timeNs) · rate / 1e9`, clamped to the document;
 * - a new anchor within 20 ms of audio of the prediction is slewed to over 100 ms; larger
 *   differences (seek, resume, loop wrap) jump;
 * - `now` is the UI clock mapped to the engine's app clock by {@link ClockSync};
 * - H-37: while looping, the extrapolated position wraps inside the loop range
 *   (`[start, end)`, the engine's `loop_range`) once it passes the loop end, so the displayed
 *   playhead jumps back to the loop start at each seam instead of running past it.
 */

export interface PlayheadAnchor {
  /** Document sample heard at `timeNs`. */
  sample: number;
  /** App-clock time (ns). */
  timeNs: number;
  /** Document samples per second (0 when stopped). */
  rate: number;
}

export const SLEW_MS = 100;
/** Differences below this much audio are slewed, larger ones jump. */
export const SLEW_THRESHOLD_S = 0.02;

/** H-37: the loop region `[start, end)` in document samples. */
export type LoopRange = readonly [number, number];

/** H-37 (SPEC-003 §2.2): `pos` extrapolated from an anchor at `anchorSample`, wrapped inside
 * `loop` once it passes the loop end — only when the anchor itself is before the loop end
 * (playback past the loop, or heading into it from before, is not looping yet / any more). */
export function wrapInLoop(anchorSample: number, pos: number, loop: LoopRange | null | undefined): number {
  if (!loop) {
    return pos;
  }
  const [start, end] = loop;
  if (end <= start || anchorSample >= end || pos < end) {
    return pos;
  }
  return start + ((pos - end) % (end - start));
}

function clamp(x: number, lenSamples: number): number {
  return Math.min(Math.max(x, 0), Math.max(lenSamples, 0));
}

/** Extrapolated position at `nowNs`. Holds the anchor until its time is reached (the first
 * frames of a start are heard only after the output latency). */
export function extrapolate(
  anchor: PlayheadAnchor,
  nowNs: number,
  lenSamples: number,
  loop?: LoopRange | null,
): number {
  let pos = anchor.sample;
  if (anchor.rate > 0 && nowNs > anchor.timeNs) {
    pos += ((nowNs - anchor.timeNs) * anchor.rate) / 1e9;
  }
  return clamp(wrapInLoop(anchor.sample, pos, loop), lenSamples);
}

/** Anchor-following playhead with the slew/jump rule. */
export class PlayheadExtrapolator {
  private anchor: PlayheadAnchor | null = null;
  private slewOffset = 0;
  private slewStartNs = 0;
  private loop: LoopRange | null = null;

  constructor(private readonly slewNs: number = SLEW_MS * 1e6) {}

  get hasAnchor(): boolean {
    return this.anchor !== null;
  }

  /** H-37: the loop range the extrapolation wraps inside (`null`: not looping). */
  setLoop(loop: LoopRange | null): void {
    this.loop = loop;
  }

  /** Takes a new anchor received at `nowNs`. */
  update(next: PlayheadAnchor, nowNs: number, lenSamples: number): void {
    let offset = 0;
    if (this.anchor && next.rate > 0 && this.anchor.rate > 0) {
      const predicted = this.position(nowNs, lenSamples);
      const target = extrapolate(next, nowNs, lenSamples, this.loop);
      const diff = predicted - target;
      if (Math.abs(diff) < next.rate * SLEW_THRESHOLD_S) {
        offset = diff;
      }
    }
    this.anchor = next;
    this.slewOffset = offset;
    this.slewStartNs = nowNs;
  }

  /** Displayed position at `nowNs`. */
  position(nowNs: number, lenSamples: number): number {
    if (!this.anchor) {
      return 0;
    }
    const base = extrapolate(this.anchor, nowNs, lenSamples, this.loop);
    if (this.slewOffset === 0) {
      return base;
    }
    const k = 1 - (nowNs - this.slewStartNs) / this.slewNs;
    if (k <= 0) {
      this.slewOffset = 0;
      return base;
    }
    return clamp(base + this.slewOffset * k, lenSamples);
  }

  reset(): void {
    this.anchor = null;
    this.slewOffset = 0;
    this.loop = null;
  }
}

/** UI ↔ engine clock offset: the sample with the smallest round trip of `samples` calls. */
export class ClockSync {
  offsetNs = 0;

  constructor(private readonly perfNowMs: () => number = () => performance.now()) {}

  async sync(fetchServerNs: () => Promise<number>, samples = 5): Promise<void> {
    let bestRtt = Number.POSITIVE_INFINITY;
    for (let i = 0; i < samples; i++) {
      const t0 = this.perfNowMs();
      const serverNs = await fetchServerNs();
      const t1 = this.perfNowMs();
      if (t1 - t0 < bestRtt) {
        bestRtt = t1 - t0;
        this.offsetNs = serverNs - ((t0 + t1) / 2) * 1e6;
      }
    }
  }

  /** The engine's app clock now (ns). */
  nowNs(): number {
    return this.perfNowMs() * 1e6 + this.offsetNs;
  }
}

/** `hh:mm:ss.fff` for a document position. */
export function formatTime(samples: number, rateHz: number): string {
  const totalMs = rateHz > 0 ? Math.floor((samples * 1000) / rateHz) : 0;
  const ms = totalMs % 1000;
  const s = Math.floor(totalMs / 1000) % 60;
  const m = Math.floor(totalMs / 60_000) % 60;
  const h = Math.floor(totalMs / 3_600_000);
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)}.${pad(ms, 3)}`;
}
