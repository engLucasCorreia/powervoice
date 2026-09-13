import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { TransportStateDto } from "../ipc/bindings";
import { attachKeymap, clearActionHandlers } from "../keymap";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import Toolbar from "./Toolbar.svelte";

let calls: string[] = [];
let playing = false;

function stateDto(): TransportStateDto {
  return {
    playing,
    playhead_samples: 0,
    play_start_samples: 0,
    doc_len_samples: 96_000,
    doc_rate_hz: 48_000,
    can_play: true,
  };
}

beforeEach(() => {
  calls = [];
  playing = false;
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "transport_play":
        case "transport_play_from_start":
          playing = true;
          return stateDto();
        case "transport_pause":
        case "transport_stop":
          playing = false;
          return stateDto();
        case "transport_get":
        case "transport_return_to_start":
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
});

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

const transportCalls = () => calls.filter((c) => c.startsWith("transport_") && c !== "transport_get");

describe("transport bar (S1-01)", () => {
  it("buttons call the transport commands", async () => {
    const teardown = await initTransport();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(Toolbar, { target, props: { version: "1" } });
    flushSync();
    const button = (id: string) => {
      const el = target.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`);
      if (!el) {
        throw new Error(`missing ${id}`);
      }
      return el;
    };

    button("transport-play").click();
    await settle();
    expect(button("transport-play").textContent?.trim()).toBe("Pause");
    button("transport-play").click();
    await settle();
    button("transport-play-from-start").click();
    await settle();
    button("transport-stop").click();
    await settle();
    button("transport-return").click();
    await settle();

    expect(transportCalls()).toEqual([
      "transport_play",
      "transport_pause",
      "transport_play_from_start",
      "transport_stop",
      "transport_return_to_start",
    ]);
    expect(calls).toContain("telemetry_subscribe");

    unmount(app);
    target.remove();
    teardown();
  });

  it("Space, Shift+Space and Home call the transport commands", async () => {
    const teardown = await initTransport();
    const detach = attachKeymap(window, { isMac: false });
    const press = async (code: string, shiftKey = false) => {
      window.dispatchEvent(new KeyboardEvent("keydown", { code, shiftKey, bubbles: true }));
      await settle();
    };

    await press("Space");
    await press("Space");
    await press("Space", true);
    await press("Home");

    expect(transportCalls()).toEqual([
      "transport_play",
      "transport_pause",
      "transport_play_from_start",
      "transport_return_to_start",
    ]);
    detach();
    teardown();
  });
});
