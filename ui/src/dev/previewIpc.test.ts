import { invoke } from "@tauri-apps/api/core";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TransportStateDto } from "../lib/ipc/bindings";
import { installPreviewIpc, type PreviewOptions } from "./previewIpc";

/**
 * H-31: `previewIpc.ts` had no cases for the six transport commands (`transport_play`, `pause`,
 * `stop`, `play_from_start`, `return_to_start`, `transport_seek`) — they fell through to
 * `default`'s `null`, which is exactly the shape of bug H-32 found (a `null` transport reply
 * corrupting `transport.svelte.ts`'s store: see that file's `applyState` comment). These tests
 * pin every one of them to a real `TransportStateDto`, and pin the new "no silent null" default.
 */
const OPTIONS: PreviewOptions = { theme: "dark", scenes: ["document"], dialog: null };

afterEach(() => {
  clearMocks();
});

describe("previewIpc transport commands (H-31)", () => {
  it("play/pause/stop/play_from_start/return_to_start each answer with a TransportStateDto, never null", async () => {
    installPreviewIpc(OPTIONS);
    for (const cmd of [
      "transport_play",
      "transport_pause",
      "transport_stop",
      "transport_play_from_start",
      "transport_return_to_start",
    ]) {
      const state = await invoke<TransportStateDto | null>(cmd);
      expect(state, cmd).not.toBeNull();
      expect(typeof state!.playhead_samples, cmd).toBe("number");
      expect(typeof state!.doc_len_samples, cmd).toBe("number");
      expect(typeof state!.doc_rate_hz, cmd).toBe("number");
      expect(typeof state!.playing, cmd).toBe("boolean");
    }
  });

  it("transport_play and transport_play_from_start report playing: true; the rest report false", async () => {
    installPreviewIpc(OPTIONS);
    expect((await invoke<TransportStateDto>("transport_play")).playing).toBe(true);
    expect((await invoke<TransportStateDto>("transport_play_from_start")).playing).toBe(true);
    expect((await invoke<TransportStateDto>("transport_pause")).playing).toBe(false);
    expect((await invoke<TransportStateDto>("transport_stop")).playing).toBe(false);
    expect((await invoke<TransportStateDto>("transport_return_to_start")).playing).toBe(false);
  });

  it("stop/play_from_start/return_to_start reset the playhead to sample 0", async () => {
    installPreviewIpc(OPTIONS);
    for (const cmd of ["transport_stop", "transport_play_from_start", "transport_return_to_start"]) {
      const state = await invoke<TransportStateDto>(cmd);
      expect(state.playhead_samples, cmd).toBe(0);
      expect(state.play_start_samples, cmd).toBe(0);
    }
  });

  it("transport_seek moves the playhead to the requested sample", async () => {
    installPreviewIpc(OPTIONS);
    const state = await invoke<TransportStateDto>("transport_seek", { positionSamples: 12_345 });
    expect(state.playhead_samples).toBe(12_345);
    expect(state.play_start_samples).toBe(12_345);
  });
});

describe("previewIpc: an unhandled command fails loudly instead of answering null (H-31)", () => {
  it("logs a console error and throws under Vitest", async () => {
    installPreviewIpc(OPTIONS);
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(invoke("totally_unknown_command")).rejects.toThrow(/totally_unknown_command/);
    expect(errorSpy).toHaveBeenCalledWith(expect.stringContaining("totally_unknown_command"));
    errorSpy.mockRestore();
  });
});
