import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { RackSlotDto, VoiceReportDto } from "../ipc/bindings";
import { resetRackForTest } from "../rack/rack.svelte";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import DiagnosticsPanel from "./DiagnosticsPanel.svelte";
import { EQ_IDS, EQ_MODULE_ID } from "./eqSuggest";

const report: VoiceReportDto = {
  f0: { current_hz: 131, median_hz: 128.9, low_hz: 112, high_hz: 152, voiced_fraction: 0.64, confidence: 0.9, octave_corrected: 0 },
  tone: { mud_db: 11, presence_db: -9, air_db: -24 },
  sibilance: { ratio_db: -15, centre_hz: 6310 },
  hum: { mains_hz: 50, harmonics: [1, 2, 3], strongest_hz: 100.1, prominence_db: 21, level_db: -71 },
  rumble_db: -31,
  noise_floor_dbfs: -66.4,
  active_level_dbfs: -20.8,
  snr_db: 45.6,
  span_s: 9.7,
};

function eqSlot(): RackSlotDto {
  const freqs = [200, 500, 1200, 3000, 6000];
  const values = EQ_IDS.peaks.flatMap((ids, i) => [
    { id: ids.on, value: 1, normalized: 1, text: "On" },
    { id: ids.freq, value: freqs[i]!, normalized: 0, text: "" },
    { id: ids.gain, value: 0, normalized: 0.5, text: "" },
    { id: ids.q, value: 1, normalized: 0.5, text: "" },
  ]);
  return rackSlotDto({ module_id: EQ_MODULE_ID, name: "Parametric EQ", values });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
});

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function mountPanel(props: Record<string, unknown>) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(DiagnosticsPanel, {
    target,
    props: { report, scopeText: "Live · 9.7 s of audio", emptyText: "Play or monitor audio.", ...props },
  });
  flushSync();
  return { target, app };
}

describe("DiagnosticsPanel (H-42)", () => {
  it("shows every statistic with a plain-language hint", () => {
    const { target, app } = mountPanel({});
    const text = (id: string) => target.querySelector(`[data-testid="diagnostics-${id}"]`)?.textContent ?? "";
    expect(text("f0")).toContain("C3");
    expect(text("f0")).toContain("64 % voiced");
    expect(text("tone-mud")).toContain("A bit boomy");
    expect(text("tone-presence")).toContain("Presence OK");
    expect(text("sibilance")).toContain("6.3 kHz");
    expect(text("hum")).toContain("Mains hum at 50 Hz (+harmonics)");
    expect(text("rumble")).toContain("Little energy below 80 Hz");
    expect(text("noise")).toContain("Meets ACX");
    expect(text("noise")).toContain("Clean separation");
    // Severity is never colour alone: each dot is labelled with its hint.
    expect(target.querySelectorAll('[role="img"], [aria-label]').length).toBeGreaterThan(0);
    unmount(app);
  });

  it("shows the empty text without a report", () => {
    const { target, app } = mountPanel({ report: null });
    expect(target.querySelector('[data-testid="diagnostics-empty"]')?.textContent).toBe("Play or monitor audio.");
    unmount(app);
  });

  it("Add EQ band here puts a notch on the hum through the rack commands", async () => {
    const calls: Array<[string, Record<string, unknown>]> = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
      if (cmd === "rack_add" || cmd === "param_set_plain") {
        return rackStateDto([eqSlot()]);
      }
      return null;
    });
    const { target, app } = mountPanel({});
    const button = target.querySelector<HTMLButtonElement>('[data-testid="diagnostics-add-eq-hum"]')!;
    expect(button.textContent).toContain("Add EQ band here");
    button.click();
    for (let i = 0; i < 10; i++) {
      await settle();
    }
    const cmds = calls.map(([c]) => c).filter((c) => c !== "plugin:event|listen");
    expect(cmds[0]).toBe("rack_add");
    expect(calls.find(([c]) => c === "rack_add")![1]).toEqual({ moduleId: EQ_MODULE_ID, index: 0 });
    const sets = calls.filter(([c]) => c === "param_set_plain").map(([, a]) => [a.id, a.value]);
    expect(sets).toEqual([
      [31, 100.1],
      [33, 20],
      [32, -20],
      [30, 1],
    ]);
    unmount(app);
  });

  it("the de-esser finding copies its frequency instead", () => {
    const { target, app } = mountPanel({});
    const copy = target.querySelector('[data-testid="diagnostics-copy-sibilance"]');
    expect(copy?.textContent).toContain("Copy 6.3 kHz");
    expect(target.querySelector('[data-testid="diagnostics-add-eq-sibilance"]')).toBeNull();
    unmount(app);
  });

  it("a firm pitch reading (H-97) shows the note plainly, with no estimate marker", () => {
    const { target, app } = mountPanel({});
    const value = target.querySelector('[data-testid="diagnostics-f0"] .value.strong')!;
    expect(value.textContent?.trim()).not.toMatch(/^≈/);
    expect(value.textContent).toContain("C3");
    expect(value.getAttribute("title")).toBeNull();
    unmount(app);
  });

  it("a shaky pitch reading is marked an estimate, and octave_corrected never appears as a raw percentage (H-97)", () => {
    // The owner's own take: 17.7 % of frames needed octave correction, aperiodicity high enough
    // that confidence dropped well below the panel's threshold.
    const shaky: VoiceReportDto = {
      ...report,
      f0: { current_hz: 131, median_hz: 128.9, low_hz: 112, high_hz: 152, voiced_fraction: 0.64, confidence: 0.4, octave_corrected: 0.177 },
    };
    const { target, app } = mountPanel({ report: shaky });
    const section = target.querySelector('[data-testid="diagnostics-f0"]')!;
    const value = section.querySelector(".value.strong")!;
    // Marked as an estimate right on the readout, not just buried in a footnote.
    expect(value.textContent).toContain("≈");
    expect(value.textContent).toContain("C3");
    // The tooltip is where the tracker honesty lives — worded for a human, never a bare number.
    const title = value.getAttribute("title") ?? "";
    expect(title.length).toBeGreaterThan(0);
    expect(title).not.toMatch(/17\.7|0\.177|18\s*%/);
    // Nowhere in the section — main text or tooltip — does octave_corrected show up as a percentage.
    expect(section.textContent ?? "").not.toMatch(/17\.7|0\.177|18\s*%/);
    // The one small added signal: a status line explaining the estimate.
    expect(section.textContent).toContain("Estimate");
    unmount(app);
  });

  it("a notable octave correction is only ever mentioned in the hover tooltip, never inline", () => {
    // Confidence stays firm even though a share of frames needed folding — the two signals are
    // independent (H-91), so the octave-correction mention must not depend on low confidence.
    const corrected: VoiceReportDto = {
      ...report,
      f0: { current_hz: 131, median_hz: 128.9, low_hz: 112, high_hz: 152, voiced_fraction: 0.64, confidence: 0.9, octave_corrected: 0.177 },
    };
    const { target, app } = mountPanel({ report: corrected });
    const section = target.querySelector('[data-testid="diagnostics-f0"]')!;
    const value = section.querySelector(".value.strong")!;
    // Confidence is firm, so no estimate marker on the number itself.
    expect(value.textContent).not.toContain("≈");
    const title = value.getAttribute("title") ?? "";
    expect(title.length).toBeGreaterThan(0);
    expect(title).not.toMatch(/17\.7|0\.177|18\s*%/);
    expect(section.textContent ?? "").not.toMatch(/17\.7|0\.177|18\s*%/);
    unmount(app);
  });
});
