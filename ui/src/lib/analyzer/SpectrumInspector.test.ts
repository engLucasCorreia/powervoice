import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import { resetDiagnosticsForTest, setInspectorOpen } from "./diagnostics.svelte";
import { resetInspectorStreamForTest } from "./inspectorStream.svelte";
import SpectrumInspector from "./SpectrumInspector.svelte";

/** H-42 (SPEC-007 §8.3): the Spectrum Inspector window's lifecycle and controls. */

afterEach(() => {
  clearMocks();
  resetInspectorStreamForTest();
  resetDiagnosticsForTest();
  document.body.innerHTML = "";
});

const settle = async () => {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
};

function recordCalls(): Array<[string, Record<string, unknown>]> {
  const calls: Array<[string, Record<string, unknown>]> = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
    if (cmd === "analyzer_inspector_subscribe") {
      return 42;
    }
    if (cmd === "analyzer_voice_subscribe") {
      return 43;
    }
    return null;
  });
  return calls;
}

describe("SpectrumInspector (H-42)", () => {
  it("opens from the store, streams at its settings, reconfigures, and closes with Escape", async () => {
    const calls = recordCalls();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(SpectrumInspector, { target });
    flushSync();
    expect(target.querySelector('[data-testid="spectrum-inspector"]')).toBeNull();

    setInspectorOpen(true);
    await settle();
    const win = target.querySelector<HTMLElement>('[data-testid="spectrum-inspector"]')!;
    expect(win).not.toBeNull();
    expect(win.getAttribute("role")).toBe("dialog");
    expect(win.getAttribute("aria-modal")).toBe("false");
    const sub = calls.find(([c]) => c === "analyzer_inspector_subscribe")!;
    expect(sub[1].config).toEqual({ fft_size: 16_384, window: "hann", response: "medium" });
    expect(calls.some(([c]) => c === "analyzer_voice_subscribe")).toBe(true);
    expect(target.querySelector('[data-testid="inspector-resolution"]')!.textContent).toContain("Hz per bin");

    // FFT size → the same stream is reconfigured, not re-subscribed.
    const fft = target.querySelector<HTMLSelectElement>('select[data-testid="inspector-fft"], [data-testid="inspector-fft"] select')!;
    fft.value = [...fft.options].find((o) => o.textContent?.replace(/\D/g, "") === "4096")!.value;
    fft.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    const configure = calls.filter(([c]) => c === "analyzer_inspector_configure");
    expect(configure.at(-1)?.[1]).toEqual({ id: 42, config: { fft_size: 4096, window: "hann", response: "medium" } });
    expect(calls.filter(([c]) => c === "analyzer_inspector_subscribe").length).toBe(1);

    win.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(target.querySelector('[data-testid="spectrum-inspector"]')).toBeNull();
    const unsubscribed = calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id);
    expect(unsubscribed).toContain(42);
    expect(unsubscribed).toContain(43);
    unmount(app);
  });

  it("leaving the Live source closes the engine stream", async () => {
    const calls = recordCalls();
    const target = document.createElement("div");
    document.body.appendChild(target);
    setInspectorOpen(true);
    const app = mount(SpectrumInspector, { target });
    await settle();
    const average = [...target.querySelectorAll<HTMLElement>('[data-testid="inspector-source"] [role="radio"], [data-testid="inspector-source"] button')].find(
      (el) => el.textContent?.trim() === "Average",
    )!;
    average.click();
    await settle();
    expect(calls.filter(([c]) => c === "analyzer_unsubscribe").map(([, a]) => a.id)).toContain(42);
    expect(target.querySelector('[data-testid="inspector-analyze"]')).not.toBeNull();
    unmount(app);
  });
});
