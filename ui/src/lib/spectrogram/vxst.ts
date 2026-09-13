/**
 * `VXST` spectrogram tile decoder (ADR-003 §2 + Amendment 1, SPEC-007 §4.2): little-endian,
 * 64-byte header (readers honour a larger `header_len`), then `frames × bins` u8 codes,
 * frame-major, bin 0 = DC. The payload is a zero-copy `Uint8Array` view over the message buffer,
 * ready for an `R8` texture upload (width = frames, height = bins, SPEC-007 §4.7).
 */

export const VXST_FLAGS = {
  /** Final tile of its request (after refinement). */
  LAST: 1 << 0,
  /** A fast preview; the refined tile of the same request and `tileIndex` replaces it. */
  PREVIEW: 1 << 1,
} as const;

/** Tile quantization range (ADR-003): code 0 = −150 dB, 255 = +6 dB, step 0.6118 dB. */
export const Q_FLOOR_DB = -150;
export const Q_CEIL_DB = 6;

export interface VxstFrame {
  requestId: number;
  flags: number;
  last: boolean;
  preview: boolean;
  audioRev: number;
  /** Frame `i` is centred at `firstFrameCenterSample + i * hopSamples`. */
  firstFrameCenterSample: number;
  hopSamples: number;
  fftSize: number;
  frames: number;
  bins: number;
  qFloorDb: number;
  qCeilDb: number;
  tileIndex: number;
  /** 0 = Hann. */
  window: number;
  /** `frames × bins` codes, frame-major (a view over the message buffer, no copy). */
  data: Uint8Array;
}

const VXST_V1_HEADER_LEN = 64;

/** u64 → number (exact below 2^53, ADR-003 §4). */
function u64(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true) + dv.getUint32(offset + 4, true) * 2 ** 32;
}

/** Decodes a `VXST` frame, or `null` if `buf` is not one (wrong magic/version, truncated). */
export function decodeVxst(buf: ArrayBuffer): VxstFrame | null {
  if (buf.byteLength < VXST_V1_HEADER_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  const headerLen = dv.getUint16(6, true);
  if (magic !== "VXST" || dv.getUint16(4, true) !== 1 || headerLen < VXST_V1_HEADER_LEN) {
    return null;
  }
  const frames = dv.getUint32(40, true);
  const bins = dv.getUint32(44, true);
  const payloadLen = frames * bins;
  if (headerLen + payloadLen > buf.byteLength) {
    return null;
  }
  const flags = dv.getUint32(12, true);
  return {
    requestId: dv.getUint32(8, true),
    flags,
    last: (flags & VXST_FLAGS.LAST) !== 0,
    preview: (flags & VXST_FLAGS.PREVIEW) !== 0,
    audioRev: u64(dv, 16),
    firstFrameCenterSample: u64(dv, 24),
    hopSamples: dv.getUint32(32, true),
    fftSize: dv.getUint32(36, true),
    frames,
    bins,
    qFloorDb: dv.getFloat32(48, true),
    qCeilDb: dv.getFloat32(52, true),
    tileIndex: dv.getUint32(56, true),
    window: dv.getUint32(60, true),
    data: new Uint8Array(buf, headerLen, payloadLen),
  };
}

/** Tile code → dB: `L̂ = floor + v·(ceil − floor)/255` (SPEC-007 §4.2). */
export function dequantizeDb(code: number, qFloorDb = Q_FLOOR_DB, qCeilDb = Q_CEIL_DB): number {
  return qFloorDb + (code * (qCeilDb - qFloorDb)) / 255;
}
