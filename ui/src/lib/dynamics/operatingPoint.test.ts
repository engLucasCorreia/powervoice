import { describe, expect, it } from "vitest";
import type { ParamInfoDto, RackSlotDto, TelemetryChannelDto } from "../ipc/bindings";
import { rackSlotDto } from "../test/fixtures";
import { effectiveMakeupDb, operatingPointOf } from "./operatingPoint";

/** SPEC-016 §2.6 item 2: x = input level, y = x + total gain reduction + effective makeup. */

const NAME = { text: "n", key: null };
const FLAGS: ParamInfoDto["flags"] = {
  automatable: true,
  stepped: false,
  boolean: false,
  read_only: false,
  hidden: false,
  bypass: false,
};

function param(id: number, key: string, group: number | null, value: number): ParamInfoDto {
  return {
    id,
    key,
    name: NAME,
    group,
    unit: { kind: "db" },
    min: -80,
    max: 30,
    default: value,
    taper: { kind: "linear" },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 0,
    flags: FLAGS,
  };
}

const CHANNELS: TelemetryChannelDto[] = [
  {
    id: 0,
    key: "gr_total_db",
    name: NAME,
    unit: { kind: "db" },
    min: -60,
    max: 0,
    kind: "gain_reduction",
    group: null,
  },
  {
    id: 5,
    key: "input_level_dbfs",
    name: NAME,
    unit: { kind: "dbfs" },
    min: -100,
    max: 6,
    kind: "level",
    group: null,
  },
];

function slot(compressorOn: boolean, makeupDb = 4): RackSlotDto {
  const params = [param(30, "compressor_enabled", 3, 1), param(35, "compressor_makeup_db", 3, makeupDb)];
  return rackSlotDto({
    params,
    groups: [
      {
        id: 3,
        key: "compressor",
        name: NAME,
        parent: null,
        enable_param: 30,
        collapsed_by_default: false,
      },
    ],
    values: [
      { id: 30, value: compressorOn ? 1 : 0, normalized: 1, text: "on" },
      { id: 35, value: makeupDb, normalized: 0.5, text: String(makeupDb) },
    ],
    telemetry: CHANNELS,
  });
}

describe("effectiveMakeupDb", () => {
  it("is the makeup while its section is on, and 0 while it is off", () => {
    expect(effectiveMakeupDb(slot(true))).toBe(4);
    expect(effectiveMakeupDb(slot(false))).toBe(0);
  });

  it("is 0 for a module without the parameter", () => {
    expect(effectiveMakeupDb(rackSlotDto({}))).toBe(0);
  });
});

describe("operatingPointOf", () => {
  // The frame's values are in channel order: total GR, then the input level.
  it("reads the input level and the total reduction from the frame", () => {
    expect(operatingPointOf(slot(true), [-4, -12])).toEqual({
      inputDbfs: -12,
      grTotalDb: -4,
      makeupDb: 4,
    });
  });

  it("is hidden without a frame, below the graph floor, and for a module without the channels", () => {
    expect(operatingPointOf(slot(true), undefined)).toBeNull();
    expect(operatingPointOf(slot(true), [-4, -80])).toBeNull();
    expect(operatingPointOf(slot(true), [-4, -120])).toBeNull();
    expect(operatingPointOf(rackSlotDto({ telemetry: [] }), [-4, -12])).toBeNull();
  });
});
