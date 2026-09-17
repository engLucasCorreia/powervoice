/**
 * Test-only `VXSA` (SPEC-007 §4.9) encoder (H-88). Builds a real, decodable little-endian wire
 * frame so a live-frame test drives the production `decodeVxsa` path end to end (via
 * `deliverChannelMessage`, `./liveFrame`) instead of injecting a hand-rolled `AnalyzerFrame`
 * object straight into a component's state — the ticket's "built from real fixture bytes, not a
 * hand-rolled object".
 *
 * The wire layout itself is pinned by the Rust-generated golden fixture (`ipc/vxsa_fixture.ts`,
 * `ipc/analyzer.test.ts`); this encoder mirrors that layout for callers that need arbitrary band
 * counts/levels a single golden fixture can't give them (H-84's `spectrumFeed.test.ts` had its own
 * private copy of this before H-88 promoted it here for every caller).
 */

export interface VxsaFields {
  seq?: number;
  /** History/averaging restarted (device reopen or rate change). */
  reset?: boolean;
  /** Tap samples dropped since the previous frame. */
  dropped?: boolean;
  /** The analysis window is digital silence. */
  silent?: boolean;
  frameTimeNs?: number;
  sampleRateHz?: number;
  fftSize?: number;
  /** Band-centre base frequency (Hz); SPEC-007 always uses 20. */
  f0Hz?: number;
  /** Bands per octave; SPEC-007 always uses 24. */
  bandsPerOctave?: number;
  /** Wire code: 0 fast, 1 medium, 2 slow. */
  response?: number;
  /** `bandCount` is implied by this array's length. `-Infinity` is a valid level; `NaN` is not. */
  levelsDb: number[];
}

const VXSA_HEADER_LEN = 48;

/** Encodes a `VXSA` v1 frame from `fields`, matching `AnalyzerFrame::encode`'s Rust layout. */
export function encodeVxsa(fields: VxsaFields): ArrayBuffer {
  const bandCount = fields.levelsDb.length;
  const buf = new ArrayBuffer(VXSA_HEADER_LEN + 4 * bandCount);
  const dv = new DataView(buf);
  for (const [i, ch] of [..."VXSA"].entries()) {
    dv.setUint8(i, ch.charCodeAt(0));
  }
  dv.setUint16(4, 1, true); // version
  dv.setUint16(6, VXSA_HEADER_LEN, true);
  dv.setUint32(8, fields.seq ?? 1, true);
  const flags =
    (fields.reset ? 1 << 0 : 0) | (fields.dropped ? 1 << 1 : 0) | (fields.silent ? 1 << 2 : 0);
  dv.setUint32(12, flags, true);
  const frameTimeNs = fields.frameTimeNs ?? 0;
  dv.setUint32(16, frameTimeNs % 2 ** 32, true);
  dv.setUint32(20, Math.floor(frameTimeNs / 2 ** 32), true);
  dv.setUint32(24, fields.sampleRateHz ?? 48_000, true);
  dv.setUint32(28, fields.fftSize ?? 2_048, true);
  dv.setFloat32(32, fields.f0Hz ?? 20, true);
  dv.setUint32(36, fields.bandsPerOctave ?? 24, true);
  dv.setUint32(40, bandCount, true);
  dv.setUint32(44, fields.response ?? 1, true);
  for (let k = 0; k < bandCount; k++) {
    dv.setFloat32(VXSA_HEADER_LEN + 4 * k, fields.levelsDb[k]!, true);
  }
  return buf;
}
