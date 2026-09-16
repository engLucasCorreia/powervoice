/**
 * WebGL2 spectrogram renderer (H-13, SPEC-007 §2.13/§4.7): each `VXST` tile is uploaded as an `R8`
 * texture (width = bins, height = frames — `tile.data` is frame-major, `data[f*bins+b]`, so this
 * upload needs no CPU transpose, per the T-204/T-207 MEMORY note), the colormap is a 256×1 LUT
 * texture built once per colormap name from `colormap.ts`'s exact LUT (never reimplemented in
 * GLSL, so there is no drift risk there), and floor/ceiling/frequency-scale/FFT size are uniforms
 * — changing any of them recolors on the next frame with no IPC and no texture re-upload (SPEC-007
 * §2.5).
 *
 * The fragment shader mirrors `shaderSampler.ts`'s bounded (≤16-tap) max-or-interpolate rule
 * (parity-tested there against `sampler.ts`) for both axes, and the frequency mapping from
 * `spectrum/freqAxis.ts`'s `freqForU` (log/linear). One quad is drawn per visible tile, spanning
 * that tile's device-pixel column range at full pane height; a pixel whose time-axis span crosses
 * into a neighboring tile is clamped to this tile's own frame range rather than sampling the
 * neighbor's texture — a deliberate simplification (SPEC-007 §4.7 leaves the exact mechanism to
 * "T-207 chooses") that only affects a sliver of columns exactly at each 256-frame tile boundary
 * when zoomed out enough to span multiple frames per pixel; see the H-13 ticket report.
 */

import type { FreqScale } from "../spectrum/freqAxis";
import { colormapLut, type ColormapName } from "./colormap";
import { TILE_FRAMES } from "./geometry";
import type { SpectroTile } from "./spectroRequester";
import { QuadProgram, linkProgram } from "../render/glProgram";
import type { Rgba } from "../render/quads";
import {
  DEFAULT_UPLOAD_BUDGET,
  planUploads,
  type UploadBudget,
  type UploadCandidate,
} from "../render/uploadBudget";

const TILE_VS = `#version 300 es
in vec2 aPos;
uniform vec2 uResolutionPx;
out vec2 vDevicePx;
void main() {
  vDevicePx = aPos;
  vec2 ndc = (aPos / uResolutionPx) * 2.0 - 1.0;
  gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}`;

// MAX_TAPS must match shaderSampler.ts's MAX_SHADER_TAPS.
const TILE_FS = `#version 300 es
precision highp float;
precision highp int;
in vec2 vDevicePx;
out vec4 outColor;

uniform sampler2D uTile;
uniform sampler2D uLut;
uniform int uTileFrames;
uniform int uTileBins;
uniform float uTileBaseFrame;
uniform float uFrameAtPx0;
uniform float uFramesPerPx;
uniform float uFloorDb;
uniform float uCeilDb;
uniform float uFreqLo;
uniform float uFreqHi;
uniform float uFreqIsLog;
uniform float uSampleRateHz;
uniform float uFftSize;
uniform float uHeightPx;
uniform vec3 uPendingColor;

const int MAX_TAPS = 16;

float dequantizeDb(float code01) {
  // SPEC-007 §4.2: L = -150 + v * 156/255; code01 already is v/255 (R8 normalized read).
  return -150.0 + code01 * 156.0;
}

bool validFrame(int frame) { return frame >= 0 && frame < uTileFrames; }
bool validBin(int bin) { return bin >= 0 && bin < uTileBins; }

float fetchDb(int frame, int bin) {
  return dequantizeDb(texelFetch(uTile, ivec2(bin, frame), 0).r);
}

// Bounded time-axis rule (shaderSampler.ts::sampleGridSpanBounded), at a fixed bin. Returns
// (value, valid) since GLSL has no nullable float.
vec2 timeAxis(float frameLoF, float frameHiF, int bin) {
  if (!validBin(bin)) {
    return vec2(0.0, 0.0);
  }
  float clampedLo = max(0.0, frameLoF);
  float clampedHi = min(float(uTileFrames), frameHiF);
  if (clampedHi <= clampedLo) {
    return vec2(0.0, 0.0);
  }
  if (clampedHi - clampedLo >= 1.0) {
    int i0 = int(max(0.0, floor(clampedLo)));
    int i1Full = min(uTileFrames - 1, int(ceil(clampedHi)) - 1);
    int i1 = min(i1Full, i0 + MAX_TAPS - 1);
    float maxV = 0.0;
    bool any = false;
    for (int i = 0; i < MAX_TAPS; i++) {
      int frame = i0 + i;
      if (frame > i1) break;
      if (!validFrame(frame)) continue;
      float db = fetchDb(frame, bin);
      if (!any || db > maxV) { maxV = db; any = true; }
    }
    return vec2(maxV, any ? 1.0 : 0.0);
  }
  float center = min(float(uTileFrames - 1), max(0.0, (frameLoF + frameHiF) * 0.5));
  int i0 = int(floor(center));
  int i1 = min(uTileFrames - 1, i0 + 1);
  bool v0ok = validFrame(i0);
  bool v1ok = validFrame(i1);
  float v0 = v0ok ? fetchDb(i0, bin) : 0.0;
  float v1 = v1ok ? fetchDb(i1, bin) : 0.0;
  if (!v0ok && !v1ok) return vec2(0.0, 0.0);
  if (!v0ok) return vec2(v1, 1.0);
  if (!v1ok) return vec2(v0, 1.0);
  float frac = center - float(i0);
  return vec2(v0 + (v1 - v0) * frac, 1.0);
}

// Bounded frequency-axis rule, combining timeAxis() results across the bin span.
vec2 binAxis(float binLoF, float binHiF, float frameLoF, float frameHiF) {
  float clampedLo = max(0.0, binLoF);
  float clampedHi = min(float(uTileBins), binHiF);
  if (clampedHi <= clampedLo) {
    return vec2(0.0, 0.0);
  }
  if (clampedHi - clampedLo >= 1.0) {
    int i0 = int(max(0.0, floor(clampedLo)));
    int i1Full = min(uTileBins - 1, int(ceil(clampedHi)) - 1);
    int i1 = min(i1Full, i0 + MAX_TAPS - 1);
    float maxV = 0.0;
    bool any = false;
    for (int i = 0; i < MAX_TAPS; i++) {
      int bin = i0 + i;
      if (bin > i1) break;
      vec2 r = timeAxis(frameLoF, frameHiF, bin);
      if (r.y > 0.5 && (!any || r.x > maxV)) { maxV = r.x; any = true; }
    }
    return vec2(maxV, any ? 1.0 : 0.0);
  }
  float center = min(float(uTileBins - 1), max(0.0, (binLoF + binHiF) * 0.5));
  int i0 = int(floor(center));
  int i1 = min(uTileBins - 1, i0 + 1);
  vec2 r0 = timeAxis(frameLoF, frameHiF, i0);
  vec2 r1 = timeAxis(frameLoF, frameHiF, i1);
  if (r0.y < 0.5 && r1.y < 0.5) return vec2(0.0, 0.0);
  if (r0.y < 0.5) return vec2(r1.x, 1.0);
  if (r1.y < 0.5) return vec2(r0.x, 1.0);
  float frac = center - float(i0);
  return vec2(r0.x + (r1.x - r0.x) * frac, 1.0);
}

// spectrum/freqAxis.ts::freqForU mirrored exactly (log: geometric interpolation from a 20 Hz
// floor; linear: plain lerp).
float freqForU(float u, float fLo, float fHi, float isLog) {
  float t = clamp(u, 0.0, 1.0);
  if (isLog > 0.5) {
    float lo = max(fLo, 20.0);
    return lo * pow(fHi / lo, t);
  }
  return fLo + t * (fHi - fLo);
}

void main() {
  float px = vDevicePx.x;
  float py = floor(vDevicePx.y);

  float frameLo = uFrameAtPx0 + uFramesPerPx * px - uTileBaseFrame;
  float frameHi = frameLo + uFramesPerPx;

  float uHi = 1.0 - py / uHeightPx;
  float uLo = 1.0 - (py + 1.0) / uHeightPx;
  float fLoPx = freqForU(uLo, uFreqLo, uFreqHi, uFreqIsLog);
  float fHiPx = freqForU(uHi, uFreqLo, uFreqHi, uFreqIsLog);
  float binLo = fLoPx * uFftSize / uSampleRateHz;
  float binHi = fHiPx * uFftSize / uSampleRateHz;

  vec2 result = binAxis(binLo, binHi, frameLo, frameHi);
  if (result.y < 0.5) {
    outColor = vec4(uPendingColor, 1.0);
    return;
  }
  float t = clamp((result.x - uFloorDb) / max(1e-6, uCeilDb - uFloorDb), 0.0, 1.0);
  vec3 color = texture(uLut, vec2(t, 0.5)).rgb;
  outColor = vec4(color, 1.0);
}`;

interface TileUniforms {
  uResolutionPx: WebGLUniformLocation | null;
  uTile: WebGLUniformLocation | null;
  uLut: WebGLUniformLocation | null;
  uTileFrames: WebGLUniformLocation | null;
  uTileBins: WebGLUniformLocation | null;
  uTileBaseFrame: WebGLUniformLocation | null;
  uFrameAtPx0: WebGLUniformLocation | null;
  uFramesPerPx: WebGLUniformLocation | null;
  uFloorDb: WebGLUniformLocation | null;
  uCeilDb: WebGLUniformLocation | null;
  uFreqLo: WebGLUniformLocation | null;
  uFreqHi: WebGLUniformLocation | null;
  uFreqIsLog: WebGLUniformLocation | null;
  uSampleRateHz: WebGLUniformLocation | null;
  uFftSize: WebGLUniformLocation | null;
  uHeightPx: WebGLUniformLocation | null;
  uPendingColor: WebGLUniformLocation | null;
}

interface TileTextureEntry {
  texture: WebGLTexture;
  /** Allocated texture size; an upload of the same size is a `texSubImage2D`, not a realloc. */
  frames: number;
  bins: number;
  /** The tile object whose pixels are in the texture (identity, not contents — the requester
   * replaces the object when a tile is refined or re-fetched). */
  sourceTile: SpectroTile | null;
  bytes: number;
  /** Draw counter of the last frame that drew this tile (the cache's LRU key). */
  lastDrawn: number;
}

export interface SpectrogramTileEntry {
  tile: SpectroTile;
  tileIndex: number;
  /** Device-pixel column range `[x0, x1)` this tile's quad covers. */
  x0: number;
  x1: number;
}

export interface SpectrogramGlDrawOptions {
  backingWidthPx: number;
  backingHeightPx: number;
  background: Rgba;
  pending: Rgba;
  colormap: ColormapName;
  floorDb: number;
  ceilDb: number;
  freqLo: number;
  freqHi: number;
  freqScale: FreqScale;
  sampleRateHz: number;
  fftSize: number;
  /** SPEC-007 §4.3: frame at absolute device pixel 0 (`startSample / hop`). */
  frameAtPx0: number;
  /** Frames per device pixel, constant across the viewport (`samplesPerDevicePixel / hop`). */
  framesPerPx: number;
  tiles: readonly SpectrogramTileEntry[];
  /** Selection/marker/playhead, pre-batched by the caller with `../render/overlayGeometry.ts`. */
  overlay: Float32Array | null;
  /** H-47: per-frame upload budget; defaults to {@link DEFAULT_UPLOAD_BUDGET}. */
  uploadBudget?: UploadBudget;
}

/** H-47: what one {@link SpectrogramGlRenderer.draw} left for the following frames. */
export interface SpectrogramGlDrawResult {
  /** Tiles whose texture upload was deferred by the budget — draw again while this is > 0. */
  pendingUploads: number;
  /** Payload bytes still waiting (diagnostics). */
  pendingBytes: number;
  /** Tiles uploaded this frame. */
  uploaded: number;
}

/**
 * H-47: how much texture memory the tile cache keeps. Tiles are 256 KiB (2048-point FFT) to 2 MiB
 * (16 384-point), so this holds a few hundred of the small ones — far more than the ~10 a viewport
 * shows, which is the point: scrolling back over a tile, or a zoom step that returns to a hop
 * already seen, then costs nothing instead of a full re-upload. Least-recently-drawn first out.
 */
export const TILE_TEXTURE_CACHE_BYTES = 64 * 1024 * 1024;

export class SpectrogramGlRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly uniforms: TileUniforms;
  private readonly vao: WebGLVertexArrayObject;
  private readonly quadBuffer: WebGLBuffer;
  private readonly lutTexture: WebGLTexture;
  private lutName: ColormapName | null = null;
  private readonly tileTextures = new Map<string, TileTextureEntry>();
  private readonly overlayProgram: QuadProgram;
  private readonly maxTextureBytes: number;
  private textureBytes = 0;
  /** Monotonic draw counter: the cache's LRU clock and the upload priority's arrival order. */
  private drawSeq = 0;
  /** Reused quad vertices — the draw loop allocated a `Float32Array` per tile per frame. */
  private readonly quadVerts = new Float32Array(12);
  /** Reused per-frame scratch, so a draw allocates nothing per tile. */
  private readonly pendingCandidates: UploadCandidate[] = [];
  private readonly pendingTiles = new Map<string, SpectroTile>();
  /** When each tile object was first offered to a draw — the upload priority's "newest first".
   * Weak, so a tile the requester dropped is collected with its entry. */
  private readonly seenTiles = new WeakMap<SpectroTile, number>();

  constructor(gl: WebGL2RenderingContext, maxTextureBytes = TILE_TEXTURE_CACHE_BYTES) {
    this.gl = gl;
    this.maxTextureBytes = maxTextureBytes;
    this.program = linkProgram(gl, TILE_VS, TILE_FS);
    this.uniforms = {
      uResolutionPx: gl.getUniformLocation(this.program, "uResolutionPx"),
      uTile: gl.getUniformLocation(this.program, "uTile"),
      uLut: gl.getUniformLocation(this.program, "uLut"),
      uTileFrames: gl.getUniformLocation(this.program, "uTileFrames"),
      uTileBins: gl.getUniformLocation(this.program, "uTileBins"),
      uTileBaseFrame: gl.getUniformLocation(this.program, "uTileBaseFrame"),
      uFrameAtPx0: gl.getUniformLocation(this.program, "uFrameAtPx0"),
      uFramesPerPx: gl.getUniformLocation(this.program, "uFramesPerPx"),
      uFloorDb: gl.getUniformLocation(this.program, "uFloorDb"),
      uCeilDb: gl.getUniformLocation(this.program, "uCeilDb"),
      uFreqLo: gl.getUniformLocation(this.program, "uFreqLo"),
      uFreqHi: gl.getUniformLocation(this.program, "uFreqHi"),
      uFreqIsLog: gl.getUniformLocation(this.program, "uFreqIsLog"),
      uSampleRateHz: gl.getUniformLocation(this.program, "uSampleRateHz"),
      uFftSize: gl.getUniformLocation(this.program, "uFftSize"),
      uHeightPx: gl.getUniformLocation(this.program, "uHeightPx"),
      uPendingColor: gl.getUniformLocation(this.program, "uPendingColor"),
    };

    const aPos = gl.getAttribLocation(this.program, "aPos");
    const vao = gl.createVertexArray();
    const quadBuffer = gl.createBuffer();
    if (!vao || !quadBuffer) {
      throw new Error("WebGL2: createVertexArray/createBuffer failed");
    }
    this.vao = vao;
    this.quadBuffer = quadBuffer;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, quadBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, 6 * 2 * 4, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);

    const lutTexture = gl.createTexture();
    if (!lutTexture) {
      throw new Error("WebGL2: createTexture (LUT) failed");
    }
    this.lutTexture = lutTexture;
    gl.bindTexture(gl.TEXTURE_2D, lutTexture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

    this.overlayProgram = new QuadProgram(gl);
  }

  private ensureLut(name: ColormapName): void {
    if (this.lutName === name) {
      return;
    }
    const gl = this.gl;
    const lut = colormapLut(name);
    gl.bindTexture(gl.TEXTURE_2D, this.lutTexture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB8, 256, 1, 0, gl.RGB, gl.UNSIGNED_BYTE, lut);
    this.lutName = name;
  }

  /**
   * H-47: uploads `tile`'s pixels into `key`'s texture, creating it only the first time and
   * re-allocating it only when the tile's dimensions changed — a tile of the same size (the
   * common case: every tile of one FFT size and hop is 256 × bins) is a `texSubImage2D` into the
   * texture that is already there. The old code created **and deleted** a texture per tile per
   * draw; the T-704 sweep did 847 `createTexture` + 847 `deleteTexture` for 847 uploads.
   */
  private uploadTile(key: string, tile: SpectroTile): void {
    const gl = this.gl;
    const existing = this.tileTextures.get(key);
    const texture = existing?.texture ?? gl.createTexture();
    if (!texture) {
      throw new Error("WebGL2: createTexture (tile) failed");
    }
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    if (existing && existing.frames === tile.frames && existing.bins === tile.bins) {
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, tile.bins, tile.frames, gl.RED, gl.UNSIGNED_BYTE, tile.data);
    } else {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, tile.bins, tile.frames, 0, gl.RED, gl.UNSIGNED_BYTE, tile.data);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    const bytes = tile.data.byteLength;
    this.textureBytes += bytes - (existing?.bytes ?? 0);
    this.tileTextures.delete(key);
    this.tileTextures.set(key, {
      texture,
      frames: tile.frames,
      bins: tile.bins,
      sourceTile: tile,
      bytes,
      lastDrawn: this.drawSeq,
    });
  }

  /**
   * H-47: keeps the cache under {@link TILE_TEXTURE_CACHE_BYTES}, least-recently-drawn first.
   * Textures drawn in the current frame are never evicted. (The old rule deleted every texture
   * that the latest draw did not use, so every scroll re-uploaded what it had just thrown away.)
   */
  private evictOverCap(): void {
    if (this.textureBytes <= this.maxTextureBytes) {
      return;
    }
    const byAge = [...this.tileTextures.entries()].sort((a, b) => a[1].lastDrawn - b[1].lastDrawn);
    for (const [key, entry] of byAge) {
      if (this.textureBytes <= this.maxTextureBytes || entry.lastDrawn === this.drawSeq) {
        break;
      }
      this.gl.deleteTexture(entry.texture);
      this.tileTextures.delete(key);
      this.textureBytes -= entry.bytes;
    }
  }

  /** Textures currently held (diagnostics/tests). */
  get cachedTextures(): number {
    return this.tileTextures.size;
  }

  /** Texture payload bytes currently held (diagnostics/tests). */
  get cachedBytes(): number {
    return this.textureBytes;
  }

  /**
   * Draws one frame and returns what the per-frame upload budget left over (H-47): while
   * `pendingUploads > 0` the caller must ask the H-43 frame scheduler for another frame, so the
   * backlog drains at the display rate instead of in one spike.
   */
  draw(opts: SpectrogramGlDrawOptions): SpectrogramGlDrawResult {
    const gl = this.gl;
    this.drawSeq += 1;
    gl.viewport(0, 0, opts.backingWidthPx, opts.backingHeightPx);
    const [br, bg, bb, ba] = opts.background;
    gl.clearColor(br, bg, bb, ba);
    gl.clear(gl.COLOR_BUFFER_BIT);

    if (opts.backingWidthPx <= 0 || opts.backingHeightPx <= 0) {
      return { pendingUploads: 0, pendingBytes: 0, uploaded: 0 };
    }

    this.ensureLut(opts.colormap);
    const u = this.uniforms;
    gl.useProgram(this.program);
    gl.bindVertexArray(this.vao);
    gl.uniform2f(u.uResolutionPx, opts.backingWidthPx, opts.backingHeightPx);
    gl.uniform1f(u.uFrameAtPx0, opts.frameAtPx0);
    gl.uniform1f(u.uFramesPerPx, opts.framesPerPx);
    gl.uniform1f(u.uFloorDb, opts.floorDb);
    gl.uniform1f(u.uCeilDb, opts.ceilDb);
    gl.uniform1f(u.uFreqLo, opts.freqLo);
    gl.uniform1f(u.uFreqHi, opts.freqHi);
    gl.uniform1f(u.uFreqIsLog, opts.freqScale === "log" ? 1 : 0);
    gl.uniform1f(u.uSampleRateHz, opts.sampleRateHz);
    gl.uniform1f(u.uFftSize, opts.fftSize);
    gl.uniform1f(u.uHeightPx, opts.backingHeightPx);
    gl.uniform3f(u.uPendingColor, opts.pending[0], opts.pending[1], opts.pending[2]);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.lutTexture);
    gl.uniform1i(u.uLut, 1);

    // H-47 pass 1: which visible tiles need their pixels uploaded (a tile whose texture already
    // holds this exact tile object needs nothing), and how the frame's budget splits them.
    const candidates = this.pendingCandidates;
    const wanted = this.pendingTiles;
    candidates.length = 0;
    wanted.clear();
    for (const entry of opts.tiles) {
      const key = `${entry.tile.fftSize}:${entry.tile.hop}:${entry.tileIndex}`;
      const cached = this.tileTextures.get(key);
      if (cached?.sourceTile === entry.tile) {
        continue;
      }
      wanted.set(key, entry.tile);
      candidates.push({ key, bytes: entry.tile.data.byteLength, x0: entry.x0, x1: entry.x1, seq: this.tileSeq(entry.tile) });
    }
    const plan = planUploads(candidates, opts.uploadBudget ?? DEFAULT_UPLOAD_BUDGET, opts.backingWidthPx);
    for (const candidate of plan.upload) {
      const tile = wanted.get(candidate.key);
      if (tile) {
        this.uploadTile(candidate.key, tile);
      }
    }

    // Pass 2: draw every tile that has a texture. A tile still waiting for its upload keeps the
    // pixels it had (a preview under a refined tile) or, with nothing uploaded yet, is simply not
    // drawn — the pending background SPEC-007 §2.8 already specifies for a tile that hasn't come.
    const verts = this.quadVerts;
    for (const entry of opts.tiles) {
      const x0 = Math.max(0, entry.x0);
      const x1 = Math.min(opts.backingWidthPx, entry.x1);
      if (x1 <= x0) {
        continue;
      }
      const key = `${entry.tile.fftSize}:${entry.tile.hop}:${entry.tileIndex}`;
      const cached = this.tileTextures.get(key);
      if (!cached) {
        continue;
      }
      cached.lastDrawn = this.drawSeq;
      gl.uniform1i(u.uTileFrames, cached.frames);
      gl.uniform1i(u.uTileBins, cached.bins);
      gl.uniform1f(u.uTileBaseFrame, entry.tileIndex * TILE_FRAMES);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, cached.texture);
      gl.uniform1i(u.uTile, 0);
      verts[0] = x0; verts[1] = 0;
      verts[2] = x1; verts[3] = 0;
      verts[4] = x0; verts[5] = opts.backingHeightPx;
      verts[6] = x1; verts[7] = 0;
      verts[8] = x1; verts[9] = opts.backingHeightPx;
      verts[10] = x0; verts[11] = opts.backingHeightPx;
      gl.bindBuffer(gl.ARRAY_BUFFER, this.quadBuffer);
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, verts);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }
    gl.bindVertexArray(null);
    this.evictOverCap();

    if (opts.overlay && opts.overlay.length > 0) {
      this.overlayProgram.draw(opts.overlay, opts.backingWidthPx, opts.backingHeightPx, gl.TRIANGLES);
    }
    return { pendingUploads: plan.deferred.length, pendingBytes: plan.deferredBytes, uploaded: plan.upload.length };
  }

  /** Arrival order of a tile object: the draw that first saw it (newest = largest). */
  private tileSeq(tile: SpectroTile): number {
    let seq = this.seenTiles.get(tile);
    if (seq === undefined) {
      seq = this.drawSeq;
      this.seenTiles.set(tile, seq);
    }
    return seq;
  }

  dispose(): void {
    const gl = this.gl;
    for (const entry of this.tileTextures.values()) {
      gl.deleteTexture(entry.texture);
    }
    this.tileTextures.clear();
    this.textureBytes = 0;
    gl.deleteTexture(this.lutTexture);
    gl.deleteBuffer(this.quadBuffer);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
    this.overlayProgram.dispose();
  }
}
