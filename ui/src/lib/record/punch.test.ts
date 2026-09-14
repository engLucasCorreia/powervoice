import { describe, expect, it } from "vitest";
import type { RecordOffsetDto, RecordPhaseDto, RecordStartedDto } from "../ipc/bindings";
import { bufferHint, formatClock, offsetReadout, parseOffsetText, phaseLabel } from "./punch";

const offset = (patch: Partial<RecordOffsetDto>): RecordOffsetDto => ({
  available: true,
  host: "pipewire",
  input_device: "Mic",
  output_device: "DAC",
  device_rate_hz: 48_000,
  offset_ms: 0,
  source: null,
  updated_unix_ms: null,
  confidence: null,
  buffer_frames: null,
  current_buffer_frames: null,
  ...patch,
});

describe("punch helpers (T-304, SPEC-022)", () => {
  it("AC-18: manual offset entry — ms or N smp at the device rate, clamped to ±500 ms", () => {
    expect(parseOffsetText("150 smp", 48_000)).toBeCloseTo(3.125, 9);
    expect(parseOffsetText("144 samples", 48_000)).toBeCloseTo(3.0, 9);
    expect(parseOffsetText("3.2", 48_000)).toBe(3.2);
    expect(parseOffsetText("-1,5 ms", 48_000)).toBe(-1.5);
    expect(parseOffsetText("600", 48_000)).toBe(500);
    expect(parseOffsetText("-600 ms", 48_000)).toBe(-500);
    expect(parseOffsetText("abc", 48_000)).toBeNull();
    expect(parseOffsetText("150 smp", 0)).toBeNull();
  });

  it("§2.11: the phase label counts the pre-roll down and shows the punch progress", () => {
    const op: RecordStartedDto = {
      take_id: 1,
      op: "punch",
      at_samples: 240_000,
      end_samples: 384_000,
      preroll_samples: 96_000,
      postroll_samples: 48_000,
      aligned: true,
      state: {
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
      },
    };
    const phase = (p: RecordPhaseDto["phase"]): RecordPhaseDto => ({
      take_id: 1,
      op: "punch",
      phase: p,
      doc_pos_samples: 0,
      app_ns: 0,
    });
    expect(phaseLabel(op, phase("preroll"), 144_000, 48_000)).toEqual({
      key: "record.phase.preroll",
      params: { time: "2.0" },
    });
    expect(phaseLabel(op, phase("recording"), 302_400, 48_000)).toEqual({
      key: "record.phase.punch",
      params: { elapsed: "0:01.3", total: "0:03.0" },
    });
    expect(phaseLabel({ ...op, op: "insert", end_samples: null }, phase("recording"), 288_000, 48_000)).toEqual({
      key: "record.phase.recording",
      params: { elapsed: "0:01.0" },
    });
    expect(phaseLabel(null, phase("preroll"), 0, 48_000)).toBeNull();
    expect(formatClock(-5, 48_000)).toBe("0:00.0");
    expect(formatClock(48_000 * 75, 48_000)).toBe("1:15.0");
  });

  it("§2.13: the offset readout and the recalibrate hint", () => {
    expect(offsetReadout(null).key).toBe("record.offset.not_calibrated");
    expect(offsetReadout(offset({})).key).toBe("record.offset.not_calibrated");
    const cal = offsetReadout(offset({ source: "calibrated", offset_ms: 3, updated_unix_ms: 1_789_000_000_000 }));
    expect(cal.key).toBe("record.offset.calibrated");
    expect(cal.params.ms).toBe("+3.00");
    expect(cal.params.samples).toBe("144");
    expect(offsetReadout(offset({ source: "manual", offset_ms: -1.25 })).params).toEqual({
      ms: "-1.25",
      samples: "-60",
    });
    expect(
      bufferHint(offset({ source: "calibrated", buffer_frames: 256, current_buffer_frames: 512 })),
    ).toEqual({ was: "256", now: "512" });
    expect(bufferHint(offset({ source: "calibrated", buffer_frames: 256, current_buffer_frames: 256 }))).toBeNull();
    expect(bufferHint(offset({ source: "manual", buffer_frames: 256, current_buffer_frames: 512 }))).toBeNull();
  });
});
