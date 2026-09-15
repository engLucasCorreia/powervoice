import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { attachKeymap, clearActionHandlers } from "../keymap";
import { initTransport, resetTransportForTest } from "../state/transport.svelte";
import { transportStateDto } from "../test/fixtures";
import Toolbar from "./Toolbar.svelte";

let calls: string[] = [];
let playing = false;

function stateDto() {
  return transportStateDto({ playing, doc_len_samples: 96_000, can_play: true });
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
    // H-25: an icon key now — its accessible name (and tooltip) says Pause while playing.
    expect(button("transport-play").getAttribute("aria-label")).toBe("Pause");
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
