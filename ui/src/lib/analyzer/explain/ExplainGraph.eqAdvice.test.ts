import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { EQ_MODULE_ID } from "../eqSuggest";
import { buildVoiceSnapshot, type VoiceSnapshot } from "./snapshot";
import { balancedReport, combCurve } from "./voiceFixtures";
import ExplainGraph from "./ExplainGraph.svelte";

/**
 * H-101: the Explain modal's dashed EQ-suggestion overlay. `ExplainGraph`'s own canvas drawing
 * isn't exercised here (jsdom has no canvas context, MEMORY.md — the same limitation
 * `ExplainVoiceModal.test.ts` notes); what matters at this level is the wiring the drawing
 * depends on: the graph asks the backend (`rack_response_curve_preview`, never
 * `rack_response_curve` — no rack slot is involved) for a preview curve when there is a
 * suggestion and the toggle is on, with the exact overrides `eqSuggest.ts::previewEqOverrides`
 * computes, and asks for nothing otherwise.
 */

const WIDTH = 400;
const HEIGHT = 160;

function snapshot(): VoiceSnapshot {
  const curve = combCurve({ f0Hz: 120, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
  return buildVoiceSnapshot({
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport(),
    sampleRateHz: 48_000,
    origin: "average",
  });
}

let widthDescriptor: PropertyDescriptor | undefined;
let heightDescriptor: PropertyDescriptor | undefined;

function stubClientSize(widthPx: number, heightPx: number): void {
  widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => widthPx });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => heightPx });
}

function render(props: Partial<Parameters<typeof ExplainGraph>[1]> & { eqBands?: unknown } = {}) {
  stubClientSize(WIDTH, HEIGHT);
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ExplainGraph, {
    target,
    props: {
      snapshot: snapshot(),
      showRaw: true,
      showSmoothed: true,
      showHarmonics: true,
      showBands: true,
      showEqAdvice: true,
      eqBands: [],
      maxLabels: 9,
      testid: "explain-graph",
      ...props,
    },
  });
  flushSync();
  return { target, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 50));
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

afterEach(() => {
  clearMocks();
  document.body.innerHTML = "";
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
    widthDescriptor = undefined;
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
    heightDescriptor = undefined;
  }
});

const EMPTY_CURVE = { freqs_hz: [], sample_rate_hz: 48_000, total_db: [], components_db: [] };

describe("EQ-suggestion preview request (H-101, SPEC-015 §2.6.3 amendment)", () => {
  it("requests rack_response_curve_preview — never rack_response_curve — with the suggestion's overrides, when there is a suggestion and the toggle is on", async () => {
    const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
    mockIPC((cmd, args) => {
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
      return EMPTY_CURVE;
    });
    const { teardown } = render({
      eqBands: [{ kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 }],
    });
    await settle();

    const previewCalls = calls.filter((c) => c.cmd === "rack_response_curve_preview");
    expect(previewCalls.length).toBeGreaterThan(0);
    expect(calls.some((c) => c.cmd === "rack_response_curve")).toBe(false);
    const call = previewCalls[0]!;
    expect(call.args.moduleId).toBe(EQ_MODULE_ID);
    expect(call.args.overrides).toEqual([
      { id: 31, value: 300 },
      { id: 33, value: 1.4 },
      { id: 32, value: -3 },
      { id: 30, value: 1 },
    ]);
    expect(Array.isArray(call.args.points)).toBe(true);
    expect((call.args.points as number[]).length).toBeGreaterThan(0);
    teardown();
  });

  it("requests nothing when there is no suggestion", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return EMPTY_CURVE;
    });
    const { teardown } = render({ eqBands: [] });
    await settle();
    expect(calls).toEqual([]);
    teardown();
  });

  it("requests nothing while the EQ Advice toggle is off, even with a suggestion", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return EMPTY_CURVE;
    });
    const { teardown } = render({
      showEqAdvice: false,
      eqBands: [{ kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 }],
    });
    await settle();
    expect(calls).toEqual([]);
    teardown();
  });
});
