import type { PeaksRequestDto } from "../ipc/bindings";
import { pickLevel } from "./coords";
import { decodeVxpk } from "./vxpk";

/**
 * Issues `peaks_get` requests and applies the responses that survive SPEC-006 §2.3's staleness
 * rules: a response is dropped outright if its `audio_rev` isn't the document's current one
 * (ADR-003), and dropped if its `request_id` is older than the last one already applied (SPEC-006
 * §2.3 "Decided" — a rapid zoom/scroll can have an older, cheaper reply arrive after a newer one).
 * Pure bookkeeping class, independent of rendering, so it's testable with a fake fetch function.
 */

export interface PeaksState {
  /** The pyramid level (or `RAW_SPP`) these buckets were fetched at. */
  level: number;
  startSample: number;
  /** `count` `(min, max)` pairs (RAW: degenerate `(x, x)` pairs). */
  buckets: Array<[number, number]>;
  partial: boolean;
}

export type FetchPeaks = (request: PeaksRequestDto) => Promise<ArrayBuffer>;

export class PeaksRequester {
  private nextRequestId = 1;
  private lastAppliedRequestId = 0;
  private currentAudioRev = 0;
  private latest: PeaksState | null = null;

  constructor(private readonly fetchPeaks: FetchPeaks) {}

  /** The most recently applied response, if any. */
  get state(): PeaksState | null {
    return this.latest;
  }

  /**
   * Sets the document's current `audio_rev` (from `document_changed`/an open/save result).
   * Clears any cached buckets from a previous document — they'd otherwise show stale audio.
   */
  setAudioRev(audioRev: number): void {
    if (audioRev !== this.currentAudioRev) {
      this.currentAudioRev = audioRev;
      this.latest = null;
    }
  }

  /**
   * Requests `[startSample, startSample + count)` at the level `samplesPerPixel` picks (SPEC-006
   * §4.3). Fire-and-forget: call again (e.g. on the next view-state change) to refresh. Network/
   * IPC failures are swallowed here — the view just keeps showing the last good buckets, and the
   * next redraw's request will retry.
   */
  async request(startSample: number, count: number, samplesPerPixel: number): Promise<void> {
    const spp = pickLevel(samplesPerPixel);
    const requestId = this.nextRequestId++;
    const audioRevAtRequest = this.currentAudioRev;
    let buf: ArrayBuffer;
    try {
      buf = await this.fetchPeaks({
        request_id: requestId,
        audio_rev: audioRevAtRequest,
        spp,
        start_sample: startSample,
        count,
      });
    } catch {
      return;
    }
    const frame = decodeVxpk(buf);
    if (!frame) {
      return;
    }
    if (frame.requestId < this.lastAppliedRequestId) {
      return; // superseded by a response that already arrived
    }
    if (frame.audioRev !== this.currentAudioRev) {
      return; // stale: an edit/new document landed after this request was sent
    }
    this.lastAppliedRequestId = frame.requestId;
    this.latest = {
      level: frame.samplesPerBucket,
      startSample: frame.startSample,
      buckets: frame.buckets,
      partial: frame.partial,
    };
  }
}
