/**
 * Decoders for the spike's binary frames (`src-tauri/src/spike/binary.rs`), following ADR-003 §2:
 * little-endian, an 8-byte common prefix (`magic`, `version`, `header_len`), typed-array views
 * into the payload with no copy. This file is the spike's throwaway stand-in for the production
 * `ui/src/lib/ipc/binary.ts` that T-203/T-204/T-108 will add for the real `VXPK`/`VXST`/`VXTM`
 * frames.
 */

export interface FrameHeader {
  magic: string;
  version: number;
  headerLen: number;
}

export function readHeader(buf: ArrayBuffer): FrameHeader {
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  return {
    magic,
    version: dv.getUint16(4, true),
    headerLen: dv.getUint16(6, true),
  };
}

export interface WaveformFrame {
  header: FrameHeader;
  count: number;
  spp: number;
  /** interleaved (min, max) pairs, `count * 2` floats, a zero-copy view into `buf` */
  minMax: Float32Array;
}

export function decodeWaveformFrame(buf: ArrayBuffer): WaveformFrame {
  const header = readHeader(buf);
  const dv = new DataView(buf);
  const count = dv.getUint32(8, true);
  const spp = dv.getUint32(12, true);
  return { header, count, spp, minMax: new Float32Array(buf, header.headerLen, count * 2) };
}

export interface SpectrogramFrame {
  header: FrameHeader;
  width: number;
  height: number;
  /** `width * height` u8 magnitudes, row-major, a zero-copy view into `buf` */
  pixels: Uint8Array;
}

export function decodeSpectrogramFrame(buf: ArrayBuffer): SpectrogramFrame {
  const header = readHeader(buf);
  const dv = new DataView(buf);
  const width = dv.getUint32(8, true);
  const height = dv.getUint32(12, true);
  return { header, width, height, pixels: new Uint8Array(buf, header.headerLen, width * height) };
}

export interface TelemetryFrame {
  header: FrameHeader;
  seq: number;
}

export function decodeTelemetryFrame(buf: ArrayBuffer): TelemetryFrame {
  const header = readHeader(buf);
  const dv = new DataView(buf);
  return { header, seq: dv.getUint32(8, true) };
}

/** MEMORY.md / ADR-003 follow-up (a): describe what actually arrived, so the spike records
 * reality instead of assuming the docs are right. */
export function describePayloadType(payload: unknown): string {
  if (payload instanceof ArrayBuffer) return "ArrayBuffer";
  if (ArrayBuffer.isView(payload)) return `TypedArray(${payload.constructor.name})`;
  if (Array.isArray(payload)) return "Array";
  return typeof payload;
}
