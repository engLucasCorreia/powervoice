/**
 * Per-instance analyzer feed for the EQ graph's spectrum overlay (H-84, SPEC-015 §2.6.6
 * "Spectrum" / AC-21): decodes `VXSA` channel messages and de-dups an unchanged frame (H-43: the
 * idle heartbeat repeats the same at-rest levels, and a frame that changes nothing must not wake
 * the frame scheduler or re-announce anything). Kept independent of any Svelte/Channel plumbing
 * so `handleMessage` can be driven directly in tests (the same convention `rack.svelte.ts`'s
 * `onModuleTelemetry` uses for its own binary frame).
 */

import { type AnalyzerFrame, decodeVxsa } from "../ipc/analyzer";
import { toArrayBuffer } from "../ipc/telemetry";
import { sameSpectrumLevels } from "./spectrumOverlay";

export interface EqSpectrumFeed {
  /** Decodes one raw channel message; calls `onFrame` only when the decoded curve actually
   * differs from the last one (H-43 dedup). */
  handleMessage(message: unknown): void;
}

export function createEqSpectrumFeed(onFrame: (frame: AnalyzerFrame) => void): EqSpectrumFeed {
  let last: AnalyzerFrame | null = null;
  return {
    handleMessage(message: unknown): void {
      const buf = toArrayBuffer(message);
      const decoded = buf ? decodeVxsa(buf) : null;
      if (!decoded || sameSpectrumLevels(last, decoded)) {
        return;
      }
      last = decoded;
      onFrame(decoded);
    },
  };
}
