import { Channel } from "@tauri-apps/api/core";
import type { SpectroRequestDto } from "../ipc/bindings";
import { spectroAttach, spectroRequest } from "../ipc/commands";
import { toArrayBuffer } from "../ipc/telemetry";
import { WINDOW_HANN, hopForZoom, tilesForView } from "./geometry";
import { decodeVxst } from "./vxst";

/**
 * Client side of the spectrogram tile service (SPEC-007 §2.8, §4.6; ADR-003 `VXST`): builds
 * `spectro_request`s for a viewport, applies the `VXST` tiles that arrive on the view's channel
 * and keeps them in an LRU capped at 192 MiB (SPEC-007 §3 `ui_texture_cache_mib`). Pure
 * bookkeeping, independent of rendering (T-207 draws from {@link SpectroRequester.tile}).
 *
 * Rules:
 * - a tile whose `audio_rev` isn't the document's current one is dropped (ADR-003);
 * - a `PREVIEW` never replaces a refined tile of the same (FFT size, hop, index);
 * - a request is only sent for tiles not held refined, and not while the outstanding request
 *   already covers them (so a scroll every frame doesn't keep cancelling work in flight);
 * - tiles of other FFT sizes/hops stay until evicted, so the renderer can stretch them while the
 *   new ones load (SPEC-007 §2.8, AC-14).
 */

export interface SpectroTile {
  fftSize: number;
  hop: number;
  tileIndex: number;
  /** Frame `i` is centred at `firstFrameCenterSample + i * hop`. */
  firstFrameCenterSample: number;
  frames: number;
  bins: number;
  preview: boolean;
  audioRev: number;
  /** `frames × bins` codes, frame-major, bin 0 = DC. */
  data: Uint8Array;
}

/** What a request needs to know about the view (SPEC-006 view state in device pixels). */
export interface SpectroViewport {
  startSample: number;
  endSample: number;
  samplesPerDevicePixel: number;
  lenSamples: number;
  fftSize: number;
}

export type SendSpectroRequest = (request: SpectroRequestDto) => Promise<void>;

export interface SpectroRequesterOptions {
  /** LRU cap in payload bytes (default 192 MiB). */
  maxBytes?: number;
  /** Called for every tile applied. */
  onTile?: (tile: SpectroTile) => void;
}

export const UI_TILE_CACHE_BYTES = 192 * 1024 * 1024;

export function spectroTileKey(fftSize: number, hop: number, tileIndex: number): string {
  return `${fftSize}:${hop}:${tileIndex}`;
}

interface InFlight {
  requestId: number;
  fftSize: number;
  hop: number;
  tiles: Set<number>;
}

export class SpectroRequester {
  private nextRequestId = 1;
  private currentAudioRev = 0;
  private readonly held = new Map<string, SpectroTile>();
  private heldBytes = 0;
  private inFlight: InFlight | null = null;
  private readonly maxBytes: number;

  constructor(
    private readonly send: SendSpectroRequest,
    private readonly options: SpectroRequesterOptions = {},
  ) {
    this.maxBytes = options.maxBytes ?? UI_TILE_CACHE_BYTES;
  }

  get audioRev(): number {
    return this.currentAudioRev;
  }

  /** Payload bytes currently held. */
  get bytes(): number {
    return this.heldBytes;
  }

  /**
   * Sets the document's current `audio_rev` (from `document_changed`). A change drops every
   * held tile — they show audio that no longer exists — and forgets the outstanding request.
   */
  setAudioRev(audioRev: number): void {
    if (audioRev !== this.currentAudioRev) {
      this.currentAudioRev = audioRev;
      this.held.clear();
      this.heldBytes = 0;
      this.inFlight = null;
    }
  }

  /** The held tile, if any (marks it most recently used). */
  tile(fftSize: number, hop: number, tileIndex: number): SpectroTile | undefined {
    const key = spectroTileKey(fftSize, hop, tileIndex);
    const tile = this.held.get(key);
    if (tile) {
      this.held.delete(key);
      this.held.set(key, tile);
    }
    return tile;
  }

  /**
   * Requests the tiles `view` needs that aren't held refined yet (visible first, SPEC-007 §2.8).
   * Returns the request id, or `null` when nothing is missing, the outstanding request already
   * covers it, or the IPC call failed (the next view change retries).
   */
  async request(view: SpectroViewport): Promise<number | null> {
    const hop = hopForZoom(view.samplesPerDevicePixel, view.fftSize);
    const missing = tilesForView(view.startSample, view.endSample, view.lenSamples, hop).filter(
      (k) => {
        const held = this.held.get(spectroTileKey(view.fftSize, hop, k));
        return !held || held.preview;
      },
    );
    if (missing.length === 0) {
      return null;
    }
    const inFlight = this.inFlight;
    if (
      inFlight &&
      inFlight.fftSize === view.fftSize &&
      inFlight.hop === hop &&
      missing.every((k) => inFlight.tiles.has(k))
    ) {
      return null;
    }
    const requestId = this.nextRequestId++;
    this.inFlight = { requestId, fftSize: view.fftSize, hop, tiles: new Set(missing) };
    try {
      await this.send({
        request_id: requestId,
        audio_rev: this.currentAudioRev,
        fft_size: view.fftSize,
        hop,
        window: WINDOW_HANN,
        tiles: missing,
      });
    } catch {
      if (this.inFlight?.requestId === requestId) {
        this.inFlight = null;
      }
      return null;
    }
    return requestId;
  }

  /** Applies one channel message; returns the tile applied, or `null` if it was dropped. */
  handleMessage(message: unknown): SpectroTile | null {
    const buf = toArrayBuffer(message);
    const frame = buf ? decodeVxst(buf) : null;
    if (!frame || frame.audioRev !== this.currentAudioRev) {
      return null;
    }
    const inFlight = this.inFlight;
    if (inFlight && frame.requestId === inFlight.requestId) {
      if (!frame.preview) {
        inFlight.tiles.delete(frame.tileIndex);
      }
      if (frame.last) {
        this.inFlight = null;
      }
    }
    const key = spectroTileKey(frame.fftSize, frame.hopSamples, frame.tileIndex);
    const existing = this.held.get(key);
    if (frame.preview && existing && !existing.preview) {
      return null;
    }
    const tile: SpectroTile = {
      fftSize: frame.fftSize,
      hop: frame.hopSamples,
      tileIndex: frame.tileIndex,
      firstFrameCenterSample: frame.firstFrameCenterSample,
      frames: frame.frames,
      bins: frame.bins,
      preview: frame.preview,
      audioRev: frame.audioRev,
      data: frame.data,
    };
    if (existing) {
      this.held.delete(key);
      this.heldBytes -= existing.data.byteLength;
    }
    this.held.set(key, tile);
    this.heldBytes += tile.data.byteLength;
    for (const [oldKey, old] of this.held) {
      if (this.heldBytes <= this.maxBytes || oldKey === key) {
        break;
      }
      this.held.delete(oldKey);
      this.heldBytes -= old.data.byteLength;
    }
    this.options.onTile?.(tile);
    return tile;
  }
}

/**
 * Production wiring: attaches spectral view `viewId`'s channel (`spectro_attach`) and returns a
 * requester sending `spectro_request`s for it.
 */
export async function createSpectroRequester(
  viewId: number,
  options: SpectroRequesterOptions = {},
): Promise<SpectroRequester> {
  const requester = new SpectroRequester((request) => spectroRequest(viewId, request), options);
  await spectroAttach(
    viewId,
    new Channel<ArrayBuffer>((message) => {
      requester.handleMessage(message);
    }),
  );
  return requester;
}
