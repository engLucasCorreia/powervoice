import type { PeaksRequestDto } from "../ipc/bindings";
import { pickLevel } from "./coords";
import { decodeVxpk } from "./vxpk";

/**
 * H-71 (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 7): issues `import_peaks_get` requests
 * for one running import job and applies the responses that survive the same out-of-order rule
 * `PeaksRequester` uses for `peaks_get` (SPEC-006 §2.3 "Decided") — a response is dropped if its
 * `request_id` is older than one already applied. There is no `audio_rev` to check here (an
 * import isn't a document revision yet, ADR-003 Amendment 7): {@link ImportPeaksRequester.setJobId}
 * clears the cached buckets instead, the instant the running job's id changes — a new import
 * started, or this component moved on to the finished document.
 */

export interface ImportPeaksState {
  /** The pyramid level (or `RAW_SPP`) these buckets were fetched at. */
  level: number;
  startSample: number;
  /** `count` `(min, max)` pairs; a `PARTIAL` bucket not yet committed is `(NaN, NaN)`. */
  buckets: Array<[number, number]>;
  partial: boolean;
}

export type FetchImportPeaks = (jobId: number, request: PeaksRequestDto) => Promise<ArrayBuffer>;

export class ImportPeaksRequester {
  private nextRequestId = 1;
  private lastAppliedRequestId = 0;
  private currentJobId: number | null = null;
  private latest: ImportPeaksState | null = null;

  constructor(private readonly fetchPeaks: FetchImportPeaks) {}

  /** The most recently applied response, if any. */
  get state(): ImportPeaksState | null {
    return this.latest;
  }

  /**
   * Sets the running import job's id (`null`: none running). Clears any cached buckets from a
   * previous job — they'd otherwise show a stale, finished (or different) import's peaks for an
   * instant.
   */
  setJobId(jobId: number | null): void {
    if (jobId !== this.currentJobId) {
      this.currentJobId = jobId;
      this.latest = null;
      this.lastAppliedRequestId = 0;
    }
  }

  /**
   * Requests `[startSample, startSample + count)` at the level `samplesPerPixel` picks, for
   * whichever job {@link setJobId} last set. A no-op if none is set. Fire-and-forget, like
   * `PeaksRequester.request`: IPC failures are swallowed, and the next call retries.
   */
  async request(startSample: number, count: number, samplesPerPixel: number): Promise<void> {
    const jobId = this.currentJobId;
    if (jobId === null) {
      return;
    }
    const spp = pickLevel(samplesPerPixel);
    const requestId = this.nextRequestId++;
    let buf: ArrayBuffer;
    try {
      buf = await this.fetchPeaks(jobId, {
        request_id: requestId,
        audio_rev: 0,
        spp,
        start_sample: startSample,
        count,
      });
    } catch {
      return;
    }
    if (jobId !== this.currentJobId) {
      return; // the running job changed while this request was in flight
    }
    const frame = decodeVxpk(buf);
    if (!frame) {
      return;
    }
    if (frame.requestId < this.lastAppliedRequestId) {
      return; // superseded by a response that already arrived
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
