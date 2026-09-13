/**
 * `VXPK` peaks/raw-samples decoder (ADR-003 §2): little-endian, 48-byte header, then `count`
 * raw `f32` samples (`RAW` flag) or `count` `(min, max)` `f32` pairs. Mirrors
 * `ui/src/lib/ipc/telemetry.ts`'s `decodeVxtm` conventions (accept a larger `header_len`, reject
 * other magics/versions).
 */

export const VXPK_FLAGS = {
  RAW: 1 << 0,
  PARTIAL: 1 << 1,
} as const;

export interface VxpkFrame {
  requestId: number;
  flags: number;
  raw: boolean;
  partial: boolean;
  audioRev: number;
  startSample: number;
  samplesPerBucket: number;
  count: number;
  sampleRateHz: number;
  /** `count` samples for `RAW`, else `count` `(min, max)` pairs. */
  buckets: Array<[number, number]>;
}

const VXPK_V1_HEADER_LEN = 48;

/** u64 → number (exact below 2^53, ADR-003 §4). */
function u64(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true) + dv.getUint32(offset + 4, true) * 2 ** 32;
}

/** Decodes a `VXPK` frame, or `null` if `buf` is not one. */
export function decodeVxpk(buf: ArrayBuffer): VxpkFrame | null {
  if (buf.byteLength < VXPK_V1_HEADER_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  const headerLen = dv.getUint16(6, true);
  if (magic !== "VXPK" || dv.getUint16(4, true) !== 1 || headerLen < VXPK_V1_HEADER_LEN) {
    return null;
  }
  const flags = dv.getUint32(12, true);
  const raw = (flags & VXPK_FLAGS.RAW) !== 0;
  const count = dv.getUint32(36, true);
  const buckets: Array<[number, number]> = new Array(count);
  let offset = headerLen;
  for (let i = 0; i < count; i++) {
    if (raw) {
      const v = dv.getFloat32(offset, true);
      buckets[i] = [v, v];
      offset += 4;
    } else {
      const mn = dv.getFloat32(offset, true);
      const mx = dv.getFloat32(offset + 4, true);
      buckets[i] = [mn, mx];
      offset += 8;
    }
  }
  return {
    requestId: dv.getUint32(8, true),
    flags,
    raw,
    partial: (flags & VXPK_FLAGS.PARTIAL) !== 0,
    audioRev: u64(dv, 16),
    startSample: u64(dv, 24),
    samplesPerBucket: dv.getUint32(32, true),
    count,
    sampleRateHz: dv.getUint32(40, true),
    buckets,
  };
}
