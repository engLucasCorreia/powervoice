import { flushSync, mount, unmount, type ComponentProps } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { resetRackForTest } from "../../rack/rack.svelte";
import { clearNotices } from "../../state/notices.svelte";
import { EQ_IDS, EQ_MODULE_ID } from "../eqSuggest";
import { rackSlotDto, rackStateDto } from "../../test/fixtures";
import type { FindingProse } from "./prose";
import ExplainFindingCard from "./ExplainFindingCard.svelte";

/**
 * H-92 (rendering H-94's words): a finding card shows title + measured always, and
 * interpretation + recommendation only when not `compact`. The severity dot's tone follows the
 * orchestrator's emphasis rule: a `nearThreshold` `attention` finding must not read as loud as an
 * ordinary one, so it downgrades to the `info` tone (`accent`) instead of `warning`.
 */

function prose(overrides: Partial<FindingProse> = {}): FindingProse {
  return {
    id: "body",
    category: "tone",
    severity: "attention",
    title: "Low-mid body (200–500 Hz)",
    measured: "+10.8 dB relative to the 1 kHz octave",
    interpretation: "Elevated, 1.8 dB past the usual point.",
    recommendation: "A wide cut of about −3 dB around 300 Hz is the conservative move.",
    action: { type: "eq", eq: { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 } },
    nearThreshold: false,
    ...overrides,
  };
}

function mountCard(props: ComponentProps<typeof ExplainFindingCard>) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ExplainFindingCard, { target, props });
  flushSync();
  return { target, app };
}

afterEach(() => {
  clearMocks();
  clearNotices();
  resetRackForTest();
  document.body.innerHTML = "";
});

describe("ExplainFindingCard (H-92)", () => {
  it("always shows the title and the measured sentence", () => {
    const { target, app } = mountCard({ prose: prose(), showEqAdvice: true, testid: "card" });
    const text = target.querySelector('[data-testid="card"]')!.textContent ?? "";
    expect(text).toContain("Low-mid body");
    expect(text).toContain("+10.8 dB");
    unmount(app);
  });

  it("compact hides the interpretation and the recommendation", () => {
    const { target, app } = mountCard({ prose: prose(), showEqAdvice: true, compact: true, testid: "card" });
    const text = target.querySelector('[data-testid="card"]')!.textContent ?? "";
    expect(text).not.toContain("Elevated");
    expect(text).not.toContain("wide cut");
    unmount(app);
  });

  it("the full card shows the interpretation and, with EQ Advice on, the recommendation and its action", () => {
    const { target, app } = mountCard({ prose: prose(), showEqAdvice: true, testid: "card" });
    const text = target.querySelector('[data-testid="card"]')!.textContent ?? "";
    expect(text).toContain("Elevated");
    expect(text).toContain("wide cut");
    expect(target.querySelector('[data-testid="card-action-add-eq"]')).not.toBeNull();
    unmount(app);
  });

  it("turning EQ Advice off hides the recommendation but keeps the interpretation", () => {
    const { target, app } = mountCard({ prose: prose(), showEqAdvice: false, testid: "card" });
    const text = target.querySelector('[data-testid="card"]')!.textContent ?? "";
    expect(text).toContain("Elevated");
    expect(text).not.toContain("wide cut");
    unmount(app);
  });

  it("an ordinary attention finding gets the warning tone", () => {
    const { target, app } = mountCard({
      prose: prose({ severity: "attention", nearThreshold: false }),
      showEqAdvice: true,
      testid: "card",
    });
    expect(target.querySelector('[data-testid="card"] [data-tone]')?.getAttribute("data-tone")).toBe("warning");
    unmount(app);
  });

  it("a near-threshold attention finding is downgraded to the info tone — the words don't shout, so the colour must not either", () => {
    const { target, app } = mountCard({
      prose: prose({ severity: "attention", nearThreshold: true }),
      showEqAdvice: true,
      testid: "card",
    });
    expect(target.querySelector('[data-testid="card"] [data-tone]')?.getAttribute("data-tone")).toBe("accent");
    unmount(app);
  });

  it("significant stays danger regardless (significant and nearThreshold cannot co-occur)", () => {
    const { target, app } = mountCard({
      prose: prose({ severity: "significant", nearThreshold: false }),
      showEqAdvice: true,
      testid: "card",
    });
    expect(target.querySelector('[data-testid="card"] [data-tone]')?.getAttribute("data-tone")).toBe("danger");
    unmount(app);
  });

  it("good and info map to success/accent", () => {
    const good = mountCard({ prose: prose({ severity: "good" }), showEqAdvice: true, testid: "good" });
    expect(good.target.querySelector('[data-testid="good"] [data-tone]')?.getAttribute("data-tone")).toBe("success");
    unmount(good.app);
    const info = mountCard({ prose: prose({ severity: "info" }), showEqAdvice: true, testid: "info" });
    expect(info.target.querySelector('[data-testid="info"] [data-tone]')?.getAttribute("data-tone")).toBe("accent");
    unmount(info.app);
  });

  it("a copy action (sibilance's de-esser target) renders a Copy button instead", async () => {
    const { target, app } = mountCard({
      prose: prose({
        id: "sibilance",
        recommendation: "A de-esser at the measured centre.",
        action: { type: "copy", freqHz: 6300 },
      }),
      showEqAdvice: true,
      testid: "card",
    });
    const button = target.querySelector('[data-testid="card-action-copy"]');
    expect(button?.textContent).toContain("6.3 kHz");
    unmount(app);
  });

  it("Add EQ band here applies through the existing rack path", async () => {
    mockIPC((cmd) => {
      if (cmd === "rack_list_modules") {
        return [];
      }
      if (cmd === "rack_add" || cmd === "param_set_plain") {
        return rackStateDto([rackSlotDto({ module_id: EQ_MODULE_ID, values: EQ_IDS.peaks.flatMap((ids) => [
          { id: ids.on, value: 0, normalized: 0, text: "Off" },
          { id: ids.freq, value: 1000, normalized: 0, text: "" },
          { id: ids.gain, value: 0, normalized: 0.5, text: "" },
          { id: ids.q, value: 1, normalized: 0.5, text: "" },
        ]) })]);
      }
      return null;
    });
    const { target, app } = mountCard({ prose: prose(), showEqAdvice: true, testid: "card" });
    target.querySelector<HTMLButtonElement>('[data-testid="card-action-add-eq"]')!.click();
    for (let i = 0; i < 10; i++) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    flushSync();
    unmount(app);
  });
});
