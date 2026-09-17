/**
 * Test-only `VXTC` encoder (H-77, SPEC-016 §4.12): builds the frame `module_transfer_curve`
 * answers, so component tests can mock the binary IPC path the way `WaveformView.test.ts` mocks
 * `VXPK`. The layout contract itself is pinned by the Rust-generated golden fixture
 * (`ui/src/lib/ipc/vxtc_fixture.ts`), not by this helper.
 */

export interface VxtcHandleFields {
  param: number;
  xDbfs: number;
  offsetDb?: number;
  enabled?: boolean;
}

export interface VxtcFields {
  seq?: number;
  xMinDb: number;
  xMaxDb: number;
  rising: number[];
  falling?: number[] | null;
  components?: number[][];
  handles?: VxtcHandleFields[];
}

const HEADER_LEN = 40;

export function encodeVxtc(fields: VxtcFields): ArrayBuffer {
  const { rising } = fields;
  const falling = fields.falling ?? null;
  const components = fields.components ?? [];
  const handles = fields.handles ?? [];
  const points = rising.length;
  const rows = [rising, ...(falling ? [falling] : []), ...components];
  const buf = new ArrayBuffer(HEADER_LEN + 4 * rows.length * points + 16 * handles.length);
  const dv = new DataView(buf);
  for (const [i, ch] of [..."VXTC"].entries()) {
    dv.setUint8(i, ch.charCodeAt(0));
  }
  dv.setUint16(4, 1, true);
  dv.setUint16(6, HEADER_LEN, true);
  dv.setUint32(8, fields.seq ?? 1, true);
  dv.setUint32(12, falling ? 1 : 0, true);
  dv.setFloat32(16, fields.xMinDb, true);
  dv.setFloat32(20, fields.xMaxDb, true);
  dv.setUint32(24, points, true);
  dv.setUint32(28, components.length, true);
  dv.setUint32(32, handles.length, true);
  dv.setUint32(36, 0, true);
  let offset = HEADER_LEN;
  for (const row of rows) {
    for (let i = 0; i < points; i++) {
      dv.setFloat32(offset + 4 * i, row[i] ?? Number.NEGATIVE_INFINITY, true);
    }
    offset += 4 * points;
  }
  for (const handle of handles) {
    dv.setUint32(offset, handle.param, true);
    dv.setFloat32(offset + 4, handle.xDbfs, true);
    dv.setFloat32(offset + 8, handle.offsetDb ?? 0, true);
    dv.setUint32(offset + 12, handle.enabled === false ? 0 : 1, true);
    offset += 16;
  }
  return buf;
}
