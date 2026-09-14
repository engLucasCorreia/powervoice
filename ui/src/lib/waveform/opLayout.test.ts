import { describe, expect, it } from "vitest";
import type { MarkerDto, RecordPhaseDto, RecordStartedDto, RecordStateDto } from "../ipc/bindings";
import {
  displayOfDoc,
  opColumns,
  opLayout,
  opMarkers,
  opPeaksRequestStart,
  type Column,
  type OpLayout,
} from "./opLayout";

/**
 * H-21 (SPEC-022 §2.11 "Waveform display"): the live take drawn at `at`, Insert shifting the
 * existing waveform (and markers) after `at` by the take length, Overwrite/Punch drawing the take
 * over the old audio.
 */

const STATE: RecordStateDto = {
  input_device: "Mic",
  input_channel: 1,
  input_status: "healthy",
  armed: true,
  input_open: true,
  input_rate_hz: 48_000,
  recording: true,
  finishing: false,
  monitor: "off",
  monitoring: false,
  monitor_latency_us: null,
  monitor_dropouts: 0,
  dropout_count: 0,
  disk_remaining_s: null,
};

function started(op: RecordStartedDto["op"], at: number, end: number | null, aligned: boolean): RecordStartedDto {
  return {
    take_id: 1,
    op,
    at_samples: at,
    end_samples: end,
    preroll_samples: aligned ? 48_000 : 0,
    postroll_samples: end === null ? 0 : 24_000,
    aligned,
    state: STATE,
  };
}

function phase(p: RecordPhaseDto["phase"]): RecordPhaseDto {
  return { take_id: 1, op: "punch", phase: p, doc_pos_samples: 0, app_ns: 0 };
}

/** Columns that encode the display sample each pixel starts at (`[s, s]`), for shift checks. */
function encoded(spp: number, width: number): (start: number) => Column[] {
  return (start) => Array.from({ length: width }, (_, px) => [start + px * spp, start + px * spp]);
}

describe("opLayout (H-21, SPEC-022 §2.11)", () => {
  it("has no take during pre-roll, grows with the window, and never exceeds a punch's length", () => {
    const punch = started("punch", 240_000, 384_000, true);
    expect(opLayout(punch, phase("preroll"), 5_000)?.takeLen).toBe(0);
    expect(opLayout(punch, phase("recording"), 5_000)).toEqual({
      insert: false,
      at: 240_000,
      punchEnd: 384_000,
      takeLen: 5_000,
    });
    expect(opLayout(punch, phase("postroll"), 200_000)?.takeLen).toBe(144_000);
    expect(opLayout(started("insert", 10, null, false), null, 99)?.takeLen).toBe(99);
    expect(opLayout(null, null, 1)).toBeNull();
    expect(opLayout(started("new", 0, null, false), null, 1)).toBeNull();
  });

  it("Insert shifts the existing audio after `at` right by the take and leaves a gap for it", () => {
    const layout: OpLayout = { insert: true, at: 100, punchEnd: null, takeLen: 50 };
    expect(displayOfDoc(layout, 99)).toBe(99);
    expect(displayOfDoc(layout, 100)).toBe(150);
    const take: Column[] = Array.from({ length: 30 }, () => [-0.5, 0.5]);
    const { base, take: t } = opColumns(layout, encoded(10, 30), take, 0, 10, 30);
    // Pixels 0–9 (display 0–99): the document unshifted.
    expect(base[5]).toEqual([50, 50]);
    // Pixels 10–14 (display 100–149): the take, no document.
    expect(base[12]).toBeNull();
    expect(t[12]).toEqual([-0.5, 0.5]);
    // Pixel 20 (display 200) shows document sample 150.
    expect(base[20]).toEqual([150, 150]);
    expect(t[20]).toBeNull();
    expect(t[9]).toBeNull();
  });

  it("Overwrite and Punch draw the take over the unshifted old audio", () => {
    const layout: OpLayout = { insert: false, at: 100, punchEnd: 300, takeLen: 50 };
    expect(displayOfDoc(layout, 200)).toBe(200);
    const take: Column[] = Array.from({ length: 30 }, () => [-0.5, 0.5]);
    const { base, take: t } = opColumns(layout, encoded(10, 30), take, 0, 10, 30);
    expect(base[12]).toEqual([120, 120]);
    expect(t[12]).toEqual([-0.5, 0.5]);
    expect(base[20]).toEqual([200, 200]);
    expect(t[15]).toBeNull();
  });

  it("requests document peaks early enough for the shifted part, in coarse steps", () => {
    const layout: OpLayout = { insert: true, at: 1_000, punchEnd: null, takeLen: 1 };
    expect(opPeaksRequestStart(layout, 500, 4_000)).toBe(500);
    expect(opPeaksRequestStart(layout, 10_000, 4_000)).toBe(9_000);
    expect(opPeaksRequestStart({ ...layout, takeLen: 999 }, 10_000, 4_000)).toBe(9_000);
    expect(opPeaksRequestStart({ ...layout, takeLen: 1_001 }, 10_000, 4_000)).toBe(8_000);
    expect(opPeaksRequestStart({ ...layout, takeLen: 50_000 }, 10_000, 4_000)).toBe(1_000);
    expect(opPeaksRequestStart({ ...layout, insert: false }, 10_000, 4_000)).toBe(10_000);
    expect(opPeaksRequestStart(null, 10_000, 4_000)).toBe(10_000);
  });

  it("moves existing markers after `at` with the Insert take, but not markers added during it", () => {
    const layout: OpLayout = { insert: true, at: 100, punchEnd: null, takeLen: 50 };
    const m = (id: number, pos: number, len = 0): MarkerDto => ({ id, pos_samples: pos, len_samples: len, name: "m" });
    const out = opMarkers(layout, [m(1, 10), m(2, 100), m(3, 90, 20), m(4, 120)], (id) => id === 4);
    expect(out.map((x) => [x.id, x.pos_samples, x.len_samples])).toEqual([
      [1, 10, 0],
      [2, 150, 0],
      [3, 90, 70],
      [4, 120, 0],
    ]);
    const list = [m(1, 200)];
    expect(opMarkers({ ...layout, insert: false }, list, () => false)).toBe(list);
  });
});
