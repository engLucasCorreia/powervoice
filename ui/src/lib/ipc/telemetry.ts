/**
 * `VXTM` telemetry frame decoder (ADR-003 §2): little-endian, 8-byte common prefix, fields at
 * fixed offsets. Readers accept a larger `header_len` (fields may be appended without a version
 * bump) and reject other magics/versions.
 */

export const VXTM_FLAGS = {
  PLAYING: 1 << 0,
  RECORDING: 1 << 1,
  MONITORING: 1 << 2,
  LOOPING: 1 << 3,
  XRUN: 1 << 4,
  OUT_CLIP: 1 << 5,
  IN_CLIP: 1 << 6,
} as const;

export interface TelemetryFrame {
  seq: number;
  flags: number;
  /** Heard position (document samples) at `playheadTimeNs`. */
  playheadSample: number;
  /** App-clock time (ns) of `playheadSample`. */
  playheadTimeNs: number;
  /** Document samples per second while playing, 0 when stopped. */
  rate: number;
  outPeakDbfs: number;
  outRmsDbfs: number;
  inPeakDbfs: number;
  inRmsDbfs: number;
  audioRev: number;
  droppedRtEvents: number;
}

const VXTM_V1_LEN = 72;

/** u64 → number (exact below 2^53, ADR-003 §4). */
function u64(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true) + dv.getUint32(offset + 4, true) * 2 ** 32;
}

/** Decodes a `VXTM` frame, or `null` if `buf` is not one. */
export function decodeVxtm(buf: ArrayBuffer): TelemetryFrame | null {
  if (buf.byteLength < VXTM_V1_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  if (magic !== "VXTM" || dv.getUint16(4, true) !== 1 || dv.getUint16(6, true) < VXTM_V1_LEN) {
    return null;
  }
  return {
    seq: dv.getUint32(8, true),
    flags: dv.getUint32(12, true),
    playheadSample: u64(dv, 16),
    playheadTimeNs: u64(dv, 24),
    rate: dv.getFloat64(32, true),
    outPeakDbfs: dv.getFloat32(40, true),
    outRmsDbfs: dv.getFloat32(44, true),
    inPeakDbfs: dv.getFloat32(48, true),
    inRmsDbfs: dv.getFloat32(52, true),
    audioRev: u64(dv, 56),
    droppedRtEvents: dv.getUint32(64, true),
  };
}

/** Channel payloads arrive as `ArrayBuffer` on WebKitGTK (ADR-009); accept views and byte arrays too. */
export function toArrayBuffer(message: unknown): ArrayBuffer | null {
  if (message instanceof ArrayBuffer) {
    return message;
  }
  if (ArrayBuffer.isView(message)) {
    const bytes = new Uint8Array(message.buffer, message.byteOffset, message.byteLength);
    return bytes.slice().buffer;
  }
  if (Array.isArray(message)) {
    return Uint8Array.from(message as number[]).buffer;
  }
  return null;
}
