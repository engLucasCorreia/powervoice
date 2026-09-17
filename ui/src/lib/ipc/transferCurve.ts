/**
 * `VXTC` transfer-curve frame decoder (H-77, SPEC-016 §4.12): the level-in → level-out curve of
 * a module's `TransferCurve` extension, answered by `module_transfer_curve`.
 *
 * Little-endian. A 40-byte header — `"VXTC"`, u16 version = 1, u16 header_len, u32 seq (echoed
 * from the request), u32 flags (bit0 `HAS_FALLING`), f32 x_min_db, f32 x_max_db, u32 points P,
 * u32 components C, u32 handles K, u32 reserved — then `f32[P]` Rising output levels, `f32[P]`
 * Falling when `HAS_FALLING`, `f32[C·P]` component gains (component-major) and K ×
 * `{u32 param_id, f32 x_dbfs, f32 offset_db, u32 flags (bit0 ENABLED)}`. Levels may be −∞ (the
 * module mutes there) but never NaN. Like the other decoders, a larger `header_len` is accepted
 * and other magics/versions, truncated frames and impossible counts are rejected.
 */

/** One draggable threshold handle, already placed on the graph's x axis. */
export interface TransferCurveHandle {
  /** The threshold parameter a drag writes (through `param_set_plain`). */
  param: number;
  /** Graph x position (dBFS) = the parameter's target value + `offsetDb`. */
  xDbfs: number;
  /** The detector offset applied (+3.0103 dB for an RMS-detected section, else 0). */
  offsetDb: number;
  /** The section is enabled (SPEC-016 §2.6 draws handles of enabled sections only). */
  enabled: boolean;
}

export interface TransferCurveFrame {
  /** Echo of the request's `seq`. */
  seq: number;
  /** Input levels (dBFS), evenly spaced: `x_i = xMinDb + i·(xMaxDb − xMinDb)/(P − 1)`. */
  inDbfs: number[];
  /** Output level (dBFS) per input level, level rising. */
  rising: number[];
  /** The hysteresis branch (level falling), or `null` without `HAS_FALLING`. */
  falling: number[] | null;
  /** One row per component (section), in the module's order; gains in dB, Rising branch. */
  components: number[][];
  handles: TransferCurveHandle[];
}

const VXTC_V1_HEADER_LEN = 40;
const HANDLE_LEN = 16;
const HAS_FALLING = 1;
const HANDLE_ENABLED = 1;
/** `vox_engine::MAX_TRANSFER_CURVE_POINTS`; a larger count means a corrupt frame. */
const MAX_POINTS = 1024;

/** Decodes a `VXTC` frame, or `null` if `buf` is not a complete one. */
export function decodeVxtc(buf: ArrayBuffer): TransferCurveFrame | null {
  if (buf.byteLength < VXTC_V1_HEADER_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  const headerLen = dv.getUint16(6, true);
  if (
    magic !== "VXTC" ||
    dv.getUint16(4, true) !== 1 ||
    headerLen < VXTC_V1_HEADER_LEN ||
    headerLen > buf.byteLength
  ) {
    return null;
  }
  const flags = dv.getUint32(12, true);
  const points = dv.getUint32(24, true);
  const componentCount = dv.getUint32(28, true);
  const handleCount = dv.getUint32(32, true);
  if (points < 2 || points > MAX_POINTS) {
    return null;
  }
  const hasFalling = (flags & HAS_FALLING) !== 0;
  const branches = componentCount + (hasFalling ? 2 : 1);
  const need = headerLen + 4 * branches * points + HANDLE_LEN * handleCount;
  if (need > buf.byteLength) {
    return null;
  }
  const xMinDb = dv.getFloat32(16, true);
  const xMaxDb = dv.getFloat32(20, true);
  const step = (xMaxDb - xMinDb) / (points - 1);
  const inDbfs = Array.from({ length: points }, (_, i) => xMinDb + step * i);

  let offset = headerLen;
  const levels = (): number[] => {
    const v = Array.from({ length: points }, (_, i) => dv.getFloat32(offset + 4 * i, true));
    offset += 4 * points;
    return v;
  };
  const rising = levels();
  const falling = hasFalling ? levels() : null;
  const components = Array.from({ length: componentCount }, () => levels());
  const handles: TransferCurveHandle[] = [];
  for (let k = 0; k < handleCount; k++) {
    const base = offset + HANDLE_LEN * k;
    handles.push({
      param: dv.getUint32(base, true),
      xDbfs: dv.getFloat32(base + 4, true),
      offsetDb: dv.getFloat32(base + 8, true),
      enabled: (dv.getUint32(base + 12, true) & HANDLE_ENABLED) !== 0,
    });
  }
  return { seq: dv.getUint32(8, true), inDbfs, rising, falling, components, handles };
}
