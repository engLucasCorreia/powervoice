import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearActionHandlers } from "../shortcuts";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { resetWaveformViewForTest } from "../state/waveformView.svelte";
import { transportStateDto } from "../test/fixtures";
import Toolbar from "./Toolbar.svelte";

/** H-37 (SPEC-003 §2.1): the toolbar's Loop toggle button. */

let loopEnabled = false;
let loopRange: [number, number] | null = null;
const calls: unknown[] = [];

function stateDto() {
  return transportStateDto({
    doc_len_samples: 96_000,
    can_play: true,
    loop_enabled: loopEnabled,
    loop_range: loopEnabled ? loopRange : null,
  });
}

beforeEach(() => {
  loopEnabled = false;
  loopRange = null;
  calls.length = 0;
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "transport_set_loop":
          calls.push((args as { enabled: boolean }).enabled);
          loopEnabled = (args as { enabled: boolean }).enabled;
          return stateDto();
        case "transport_get":
          return stateDto();
        case "clock_now_ns":
          return 0;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  resetTransportForTest();
  resetWaveformViewForTest();
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("the Loop button", () => {
  it("toggles loop playback, shows its pressed state and the Ctrl+L shortcut", async () => {
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();

    const button = () => target.querySelector<HTMLButtonElement>('[data-testid="transport-loop"]')!;
    expect(button().getAttribute("aria-pressed")).toBe("false");
    expect(button().getAttribute("aria-label")).toBe("Loop playback");
    expect(button().getAttribute("aria-keyshortcuts")).toMatch(/Control\+L|Meta\+L/);

    button().click();
    await settle();
    expect(calls).toEqual([true]);
    expect(button().getAttribute("aria-pressed")).toBe("true");
    // Loop on without a selection is inert — the tooltip says why.
    expect(button().getAttribute("aria-label")).toBe("Loop playback — select a time range to loop");

    button().click();
    await settle();
    expect(calls).toEqual([true, false]);
    expect(button().getAttribute("aria-pressed")).toBe("false");

    unmount(app);
    target.remove();
    teardown();
  });

  it("the label is the plain one while a loop region is active", async () => {
    loopEnabled = true;
    loopRange = [1_000, 9_000];
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();
    const button = target.querySelector<HTMLButtonElement>('[data-testid="transport-loop"]')!;
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(button.getAttribute("aria-label")).toBe("Loop playback");
    unmount(app);
    target.remove();
    teardown();
  });
});
