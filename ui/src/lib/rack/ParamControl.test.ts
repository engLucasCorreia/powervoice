import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { LocalizedTextDto, ParamInfoDto, ParamValueDto } from "../ipc/bindings";
import { resetRackForTest } from "./rack.svelte";
import ParamControl from "./ParamControl.svelte";

/**
 * Generic parameter widget tests (SPEC-012 §2.6): the widget a parameter gets is derived only
 * from its schema (flags/taper/enum_labels) — never a per-module special case. Covers the
 * READ_ONLY readout, the BOOL toggle, an enum dropdown, the continuous/stepped slider, double
 * -click reset, wheel stepping, and typed-text entry (Rust parses it; invalid text shows an
 * error outline instead of a toast).
 */

function text(s: string): LocalizedTextDto {
  return { text: s, key: null };
}

function param(overrides: Partial<ParamInfoDto> = {}): ParamInfoDto {
  return {
    id: 0,
    key: "gain_db",
    name: text("Gain"),
    group: null,
    unit: { kind: "db" },
    min: -60,
    max: 12,
    default: 0,
    taper: { kind: "linear" },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 20,
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...overrides,
  };
}

function value(overrides: Partial<ParamValueDto> = {}): ParamValueDto {
  return { id: 0, value: 0, normalized: 0.83, text: "0.0 dB", ...overrides };
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
  document.body.innerHTML = "";
});

function render(props: { param: ParamInfoDto; value: ParamValueDto | undefined }) {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(ParamControl, { target, props: { slot: 0, param: props.param, value: props.value } });
  flushSync();
  const el = (id: string): HTMLElement => {
    const found = target.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    if (!found) {
      throw new Error(`missing ${id}`);
    }
    return found;
  };
  return { target, el, teardown: () => unmount(app) };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("widget selection from the schema", () => {
  it("READ_ONLY renders a plain readout, no slider or button", () => {
    const p = param({ flags: { ...param().flags, read_only: true } });
    const { el, target, teardown } = render({ param: p, value: value({ text: "−18.2 dBFS" }) });
    expect(el("param-readout").textContent).toBe("−18.2 dBFS");
    expect(target.querySelector('[data-testid="param-slider"]')).toBeNull();
    teardown();
  });

  it("BOOL renders a toggle switch", async () => {
    mockIPC((cmd, args) => {
      expect(cmd).toBe("param_set_text");
      expect(args).toEqual({ slot: 0, id: 0, text: "1" });
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const p = param({ flags: { ...param().flags, boolean: true } });
    const { el, teardown } = render({ param: p, value: value({ value: 0, text: "Off" }) });
    const toggle = el("param-toggle") as HTMLButtonElement;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    toggle.click();
    await settle();
    teardown();
  });

  it("an enum schema renders a dropdown with the localized labels", () => {
    const p = param({
      enum_labels: [text("Peak"), text("RMS")],
      min: 0,
      max: 1,
      step: 1,
    });
    const { el, teardown } = render({ param: p, value: value({ value: 1 }) });
    const select = el("param-enum") as HTMLSelectElement;
    expect(select.value).toBe("1");
    expect([...select.options].map((o) => o.textContent)).toEqual(["Peak", "RMS"]);
    teardown();
  });

  it("onchange on an enum sends param_set_text with the raw index", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const p = param({ enum_labels: [text("Peak"), text("RMS")], min: 0, max: 1, step: 1 });
    const { el, teardown } = render({ param: p, value: value({ value: 0 }) });
    const select = el("param-enum") as HTMLSelectElement;
    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(calls).toEqual([{ slot: 0, id: 0, text: "1" }]);
    teardown();
  });

  it("a continuous param renders a slider plus a value field", () => {
    const { el, teardown } = render({ param: param(), value: value() });
    expect(el("param-slider")).toBeTruthy();
    expect(el("param-value").textContent?.trim()).toBe("0.0 dB");
    teardown();
  });

  it("STEPPED adds the detented slider styling", () => {
    const p = param({ flags: { ...param().flags, stepped: true }, step: 1 });
    const { el, teardown } = render({ param: p, value: value() });
    expect(el("param-slider").classList.contains("stepped")).toBe(true);
    teardown();
  });
});

describe("double-click reset (SPEC-012 §2.6)", () => {
  it("resets to the parameter's default via param_set_text", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      calls.push(args);
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const p = param({ default: -6 });
    const { el, teardown } = render({ param: p, value: value({ value: 3 }) });
    el("param-slider").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    expect(calls).toEqual([{ slot: 0, id: 0, text: "-6" }]);
    teardown();
  });

  it("does nothing for a READ_ONLY parameter (no slider to double-click)", () => {
    const p = param({ flags: { ...param().flags, read_only: true } });
    const { target, teardown } = render({ param: p, value: value() });
    expect(target.querySelector('[data-testid="param-slider"]')).toBeNull();
    teardown();
  });
});

describe("typed text entry (Rust parses it, SPEC-012 §2.6)", () => {
  it("commits on Enter and shows Rust's formatted text back", async () => {
    mockIPC((cmd, args) => {
      expect(cmd).toBe("param_set_text");
      expect(args).toEqual({ slot: 0, id: 0, text: "-6" });
      return {
        slots: [
          {
            uid: 1,
            module: "org.powervoice.gain@1.0.0",
            name: "Gain",
            bypass: false,
            latency_samples: 0,
            status: { kind: "active" },
            params: [param()],
            groups: [],
            values: [{ id: 0, value: -6, normalized: 0.6, text: "-6.0 dB" }],
          },
        ],
        ab: false,
        latency_samples: 0,
      };
    });
    const { el, target, teardown } = render({ param: param(), value: value() });
    el("param-value").click();
    flushSync();
    const input = target.querySelector<HTMLInputElement>('[data-testid="param-value-input"]');
    expect(input).toBeTruthy();
    input!.value = "-6";
    input!.dispatchEvent(new Event("input", { bubbles: true }));
    input!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();
    expect(target.querySelector('[data-testid="param-value-input"]')).toBeNull();
    teardown();
  });

  it("unparseable text shows an error outline instead of a toast, and stays in edit mode", async () => {
    mockIPC(() => {
      throw { code: "invalid_argument", key: "error.rack_rejected", params: { message: "bad" } };
    });
    const { el, target, teardown } = render({ param: param(), value: value() });
    el("param-value").click();
    flushSync();
    const input = target.querySelector<HTMLInputElement>('[data-testid="param-value-input"]')!;
    input.value = "not a number";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();
    expect(el("param-value-input").classList.contains("invalid")).toBe(true);
    expect(target.querySelector('[data-testid="param-error"]')).toBeTruthy();
    teardown();
  });

  it("Escape cancels the edit without sending a command", async () => {
    let called = false;
    mockIPC(() => {
      called = true;
      return { slots: [], ab: false, latency_samples: 0 };
    });
    const { el, target, teardown } = render({ param: param(), value: value() });
    el("param-value").click();
    flushSync();
    const input = target.querySelector<HTMLInputElement>('[data-testid="param-value-input"]')!;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    expect(target.querySelector('[data-testid="param-value-input"]')).toBeNull();
    expect(called).toBe(false);
    teardown();
  });
});
