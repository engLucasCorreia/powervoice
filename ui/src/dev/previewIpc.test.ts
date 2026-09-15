import { invoke } from "@tauri-apps/api/core";
import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DocumentDto, TransportStateDto } from "../lib/ipc/bindings";
import { decodeVxpk } from "../lib/waveform/vxpk";
import { installPreviewIpc, PREVIEW_LONG_LEN_SAMPLES, type PreviewOptions } from "./previewIpc";

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

/**
 * T-704: `&doc=60min` — the frame-time sweep's 60-minute document. Its peaks come from one
 * precomputed pyramid (a slice copy per `peaks_get`, like the backend), so every level must be the
 * union of the level below it, exactly as ADR-004 §5's per-chunk pyramids are.
 */
describe("previewIpc 60-minute document (T-704)", () => {
  const LONG: PreviewOptions = { ...OPTIONS, longDocument: true };
  const request = (spp: number, start: number, count: number) =>
    invoke<ArrayBuffer>("peaks_get", { request: { request_id: 1, audio_rev: 1, spp, start_sample: start, count } });

  it("opens a 60-minute document zoomed to fit", async () => {
    installPreviewIpc(LONG);
    const doc = await invoke<DocumentDto>("document_open", { path: "/x.wav" });
    expect(doc.len_samples).toBe(3600 * 48_000);
    expect(doc.len_samples).toBe(PREVIEW_LONG_LEN_SAMPLES);
    expect(doc.waveform_view).toBeNull();
  });

  it("serves every pyramid level as the union of the four buckets below it", async () => {
    installPreviewIpc(LONG);
    const start = 1_234 * 65_536;
    for (const fine of [64, 256, 1024, 4096, 16_384]) {
      const f = decodeVxpk(await request(fine, start, 64))!;
      const c = decodeVxpk(await request(fine * 4, start, 16))!;
      expect(f.count).toBe(64);
      expect(c.count).toBe(16);
      for (let k = 0; k < 16; k++) {
        const kids = f.buckets.slice(4 * k, 4 * k + 4);
        expect(c.buckets[k]![0]).toBe(Math.min(...kids.map((b) => b[0])));
        expect(c.buckets[k]![1]).toBe(Math.max(...kids.map((b) => b[1])));
      }
    }
  });

  it("clamps the whole-file overview to the document: 2 637 top-level buckets, the last partial", async () => {
    installPreviewIpc(LONG);
    const overview = decodeVxpk(await request(65_536, 0, 3000))!;
    expect(overview.count).toBe(Math.ceil(PREVIEW_LONG_LEN_SAMPLES / 65_536));
    expect(overview.buckets.every(([mn, mx]) => mn <= mx)).toBe(true);
    expect(overview.buckets.some(([mn, mx]) => mn < 0 && mx > 0)).toBe(true);
  });

  it("serves raw samples and spectrogram tiles for the long document, each tile headed with its own index", async () => {
    installPreviewIpc(LONG);
    const raw = decodeVxpk(await request(1, 3_000 * 48_000, 5_000))!;
    expect(raw.raw).toBe(true);
    expect(raw.count).toBe(5_000);
    expect(raw.buckets.every(([mn, mx]) => Number.isFinite(mn) && mn === mx)).toBe(true);

    const got: ArrayBuffer[] = [];
    await invoke("spectro_attach", { viewId: 7, channel: { onmessage: (m: ArrayBuffer) => got.push(m) } });
    await invoke("spectro_request", { viewId: 7, request: { request_id: 3, audio_rev: 1, fft_size: 512, hop: 128, tiles: [5, 21] } });
    await new Promise((r) => setTimeout(r, 10));
    expect(got).toHaveLength(2);
    const header = (buf: ArrayBuffer) => {
      const v = new DataView(buf);
      return { tile: v.getUint32(56, true), start: Number(v.getBigUint64(24, true)), request: v.getUint32(8, true), last: v.getUint32(12, true) };
    };
    expect(header(got[0]!)).toEqual({ tile: 5, start: 5 * 256 * 128, request: 3, last: 0 });
    expect(header(got[1]!)).toEqual({ tile: 21, start: 21 * 256 * 128, request: 3, last: 1 });
  });

  it("keeps the 95 s chapter for every other preview", async () => {
    installPreviewIpc(OPTIONS);
    const doc = await invoke<DocumentDto>("document_open", { path: "/x.wav" });
    expect(doc.len_samples).toBe(95 * 48_000);
  });
});
