import { describe, expect, it } from "vitest";
import type { VoiceReportDto } from "../ipc/bindings";
import { assessReport, formatFreqShort, HUM_NOTCH_GAIN_DB, type Finding } from "./diagnosticsHints";

function report(overrides: Partial<VoiceReportDto> = {}): VoiceReportDto {
  return {
    f0: { current_hz: 131, median_hz: 128, low_hz: 110, high_hz: 152, voiced_fraction: 0.6, confidence: 0.9, octave_corrected: 0 },
    tone: { mud_db: 3, presence_db: -7, air_db: -22 },
    sibilance: { ratio_db: -26, centre_hz: 6300 },
    hum: null,
    rumble_db: -34,
    noise_floor_dbfs: -68,
    active_level_dbfs: -21,
    snr_db: 47,
    span_s: 10,
    ...overrides,
  };
}

const byId = (findings: Finding[], id: string) => findings.find((f) => f.id === id);

describe("diagnostic hints (H-42)", () => {
  it("a healthy voice reads OK everywhere", () => {
    const f = assessReport(report());
    expect(f.map((x) => x.id)).toEqual(["f0", "mud", "presence", "air", "sibilance", "hum", "rumble", "noise", "snr"]);
    expect(f.filter((x) => x.severity === "warn")).toEqual([]);
    expect(byId(f, "presence")?.hintKey).toBe("analyzer.hint.presence_ok");
  });

  it("flags boomy, dull and harsh voices with an EQ move", () => {
    const boomy = assessReport(report({ tone: { mud_db: 11, presence_db: -18, air_db: -40 } }));
    expect(byId(boomy, "mud")).toMatchObject({ severity: "warn", hintKey: "analyzer.hint.mud_high" });
    expect(byId(boomy, "mud")?.action).toEqual({ type: "eq", eq: { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 } });
    expect(byId(boomy, "presence")?.action).toMatchObject({ type: "eq", eq: { kind: "boost" } });
    expect(byId(boomy, "air")).toMatchObject({ severity: "info", hintKey: "analyzer.hint.air_low" });
    const harsh = assessReport(report({ tone: { mud_db: 0, presence_db: 1, air_db: null } }));
    expect(byId(harsh, "presence")?.action).toMatchObject({ type: "eq", eq: { kind: "cut", freqHz: 3500 } });
    expect(byId(harsh, "air")).toBeUndefined();
  });

  it("sibilance gives the de-esser target to copy", () => {
    const f = byId(assessReport(report({ sibilance: { ratio_db: -9, centre_hz: 6310 } })), "sibilance")!;
    expect(f.severity).toBe("warn");
    expect(f.params).toEqual({ freq: "6.3 kHz" });
    expect(f.action).toEqual({ type: "copy", freqHz: 6310 });
  });

  it("hum offers a notch at the strongest line", () => {
    const f = byId(
      assessReport(
        report({ hum: { mains_hz: 50, harmonics: [1, 2, 3], strongest_hz: 100.1, prominence_db: 24, level_db: -70 } }),
      ),
      "hum",
    )!;
    expect(f).toMatchObject({ severity: "warn", hintKey: "analyzer.hint.hum_harmonics", params: { freq: "50 Hz" } });
    expect(f.action).toEqual({ type: "eq", eq: { kind: "notch", freqHz: 100.1, gainDb: HUM_NOTCH_GAIN_DB, q: 20 } });
    const single = byId(
      assessReport(report({ hum: { mains_hz: 60, harmonics: [1], strongest_hz: 60, prominence_db: 22, level_db: -60 } })),
      "hum",
    )!;
    expect(single.hintKey).toBe("analyzer.hint.hum");
  });

  it("rumble suggests an 80 Hz high-pass; noise and SNR follow ACX", () => {
    const f = assessReport(report({ rumble_db: -15, noise_floor_dbfs: -55, snr_db: 25 }));
    expect(byId(f, "rumble")?.action).toEqual({
      type: "eq",
      eq: { kind: "high_pass", freqHz: 80, gainDb: 0, q: 0.7071 },
    });
    expect(byId(f, "noise")).toMatchObject({ severity: "warn", hintKey: "analyzer.hint.noise_acx_fail" });
    expect(byId(f, "snr")).toMatchObject({ severity: "warn", hintKey: "analyzer.hint.snr_low" });
  });

  it("an empty report has nothing to say", () => {
    const empty: VoiceReportDto = {
      f0: null,
      tone: null,
      sibilance: null,
      hum: null,
      rumble_db: null,
      noise_floor_dbfs: null,
      active_level_dbfs: null,
      snr_db: null,
      span_s: 0,
    };
    expect(assessReport(empty)).toEqual([]);
  });

  it("formats short frequencies", () => {
    expect(formatFreqShort(50)).toBe("50 Hz");
    expect(formatFreqShort(315.4)).toBe("315 Hz");
    expect(formatFreqShort(6310)).toBe("6.3 kHz");
    expect(formatFreqShort(12_000)).toBe("12 kHz");
  });
});
