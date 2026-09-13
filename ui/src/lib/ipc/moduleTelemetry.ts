/**
 * `VXMT` module-telemetry frame decoder (SPEC-016 §4.12, H-03): the rack slots' meter values
 * (e.g. the true-peak limiter's gain reduction), streamed by `module_telemetry_subscribe`.
 *
 * Little-endian. A 32-byte header — `"VXMT"`, u16 version = 1, u16 header_len, u32 seq,
 * u32 flags, u64 frame_time_ns, u32 record count, u32 reserved — then per record
 * `{u32 slot_uid, u16 count, u16 reserved, f32[count]}` (values in the slot's `telemetry`
 * channel order). Like `decodeVxtm`, readers accept a larger `header_len` and reject other
 * magics/versions; truncated frames are rejected too.
 */

export interface ModuleTelemetryRecord {
  /** The slot's `RackSlotDto.uid`. */
  slotUid: number;
  /** One value per telemetry channel, in the slot's `telemetry` order. */
  values: number[];
}

export interface ModuleTelemetryFrame {
  seq: number;
  /** App-clock time (ns) of the read. */
  frameTimeNs: number;
  records: ModuleTelemetryRecord[];
}

const VXMT_V1_HEADER_LEN = 32;
const RECORD_HEADER_LEN = 8;

/** u64 → number (exact below 2^53, ADR-003 §4). */
function u64(dv: DataView, offset: number): number {
  return dv.getUint32(offset, true) + dv.getUint32(offset + 4, true) * 2 ** 32;
}

/** Decodes a `VXMT` frame, or `null` if `buf` is not a complete one. */
export function decodeVxmt(buf: ArrayBuffer): ModuleTelemetryFrame | null {
  if (buf.byteLength < VXMT_V1_HEADER_LEN) {
    return null;
  }
  const dv = new DataView(buf);
  const magic = String.fromCharCode(dv.getUint8(0), dv.getUint8(1), dv.getUint8(2), dv.getUint8(3));
  const headerLen = dv.getUint16(6, true);
  if (
    magic !== "VXMT" ||
    dv.getUint16(4, true) !== 1 ||
    headerLen < VXMT_V1_HEADER_LEN ||
    headerLen > buf.byteLength
  ) {
    return null;
  }
  const count = dv.getUint32(24, true);
  const records: ModuleTelemetryRecord[] = [];
  let offset = headerLen;
  for (let r = 0; r < count; r++) {
    if (offset + RECORD_HEADER_LEN > buf.byteLength) {
      return null;
    }
    const slotUid = dv.getUint32(offset, true);
    const n = dv.getUint16(offset + 4, true);
    offset += RECORD_HEADER_LEN;
    if (offset + 4 * n > buf.byteLength) {
      return null;
    }
    const values: number[] = [];
    for (let k = 0; k < n; k++) {
      values.push(dv.getFloat32(offset + 4 * k, true));
    }
    offset += 4 * n;
    records.push({ slotUid, values });
  }
  return { seq: dv.getUint32(8, true), frameTimeNs: u64(dv, 16), records };
}
