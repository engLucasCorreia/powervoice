import { describe, expect, it } from "vitest";
import { SpectrogramGlRenderer, type SpectrogramGlDrawOptions, type SpectrogramTileEntry } from "./webglRenderer";
import { TILE_FRAMES } from "./geometry";
import type { SpectroTile } from "./spectroRequester";

/**
 * H-47: the tile-texture cache and the per-frame upload budget, against a recording fake WebGL2
 * context. jsdom has no WebGL (MEMORY.md), but every call this renderer makes is a plain method on
 * the context object, so a fake records exactly what reached the driver — which is what the
 * behaviour under test is about (how many uploads, of which kind, per frame).
 */
interface Call {
  name: string;
  args: unknown[];
}

function fakeGl(): { gl: WebGL2RenderingContext; calls: Call[]; count: (name: string) => number } {
  const calls: Call[] = [];
  const constants: Record<string, number> = {
    TEXTURE_2D: 1, R8: 2, RED: 3, UNSIGNED_BYTE: 4, RGB8: 5, RGB: 6, ARRAY_BUFFER: 7,
    DYNAMIC_DRAW: 8, FLOAT: 9, TRIANGLES: 10, COLOR_BUFFER_BIT: 11, TEXTURE0: 12, TEXTURE1: 13,
    TEXTURE_MIN_FILTER: 14, TEXTURE_MAG_FILTER: 15, TEXTURE_WRAP_S: 16, TEXTURE_WRAP_T: 17,
    NEAREST: 18, CLAMP_TO_EDGE: 19, COMPILE_STATUS: 20, LINK_STATUS: 21, VERTEX_SHADER: 22,
    FRAGMENT_SHADER: 23, UNPACK_ALIGNMENT: 24, LINE_STRIP: 25, POINTS: 26,
  };
  let nextObject = 1;
  const object = (name: string) => () => {
    calls.push({ name, args: [] });
    return { id: nextObject++, kind: name };
  };
  const record =
    (name: string) =>
    (...args: unknown[]) => {
      calls.push({ name, args });
    };
  const gl: Record<string, unknown> = {
    ...constants,
    createShader: object("createShader"),
    createProgram: object("createProgram"),
    createTexture: object("createTexture"),
    createBuffer: object("createBuffer"),
    createVertexArray: object("createVertexArray"),
    getShaderParameter: () => true,
    getProgramParameter: () => true,
    getUniformLocation: (_p: unknown, name: string) => ({ name }),
    getAttribLocation: () => 0,
  };
  for (const name of [
    "shaderSource", "compileShader", "attachShader", "linkProgram", "deleteShader", "useProgram",
    "bindVertexArray", "bindBuffer", "bufferData", "bufferSubData", "enableVertexAttribArray",
    "vertexAttribPointer", "bindTexture", "texParameteri", "texImage2D", "texSubImage2D",
    "pixelStorei", "activeTexture", "uniform1i", "uniform1f", "uniform2f", "uniform3f",
    "viewport", "clearColor", "clear", "drawArrays", "deleteTexture", "deleteBuffer",
    "deleteVertexArray", "deleteProgram",
  ]) {
    gl[name] = record(name);
  }
  return {
    gl: gl as unknown as WebGL2RenderingContext,
    calls,
    count: (name) => calls.filter((c) => c.name === name).length,
  };
}

const BINS = 65;

function tile(tileIndex: number, hop = 512, fftSize = 128, bins = BINS): SpectroTile {
  return {
    fftSize,
    hop,
    tileIndex,
    firstFrameCenterSample: tileIndex * TILE_FRAMES * hop,
    frames: TILE_FRAMES,
    bins,
    preview: false,
    audioRev: 1,
    data: new Uint8Array(TILE_FRAMES * bins),
  };
}

function entries(tiles: SpectroTile[], widthPx: number): SpectrogramTileEntry[] {
  const step = widthPx / Math.max(1, tiles.length);
  return tiles.map((t, i) => ({ tile: t, tileIndex: t.tileIndex, x0: i * step, x1: (i + 1) * step }));
}

function options(tiles: SpectrogramTileEntry[], over: Partial<SpectrogramGlDrawOptions> = {}): SpectrogramGlDrawOptions {
  return {
    backingWidthPx: 800,
    backingHeightPx: 200,
    background: [0, 0, 0, 1],
    pending: [0.1, 0.1, 0.1, 1],
    colormap: "inferno",
    floorDb: -100,
    ceilDb: 0,
    freqLo: 20,
    freqHi: 24_000,
    freqScale: "log",
    sampleRateHz: 48_000,
    fftSize: 128,
    frameAtPx0: 0,
    framesPerPx: 1,
    tiles,
    overlay: null,
    ...over,
  };
}

describe("SpectrogramGlRenderer tile textures (H-47)", () => {
  it("uploads each tile once and never re-uploads or deletes an unchanged one", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const tiles = entries([tile(0), tile(1)], 800);
    renderer.draw(options(tiles, { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } }));
    const uploadsAfterFirst = count("texImage2D") + count("texSubImage2D");
    for (let i = 0; i < 5; i++) {
      renderer.draw(options(tiles, { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } }));
    }
    expect(uploadsAfterFirst).toBe(2 + 1); // two tiles + the colormap LUT
    expect(count("texImage2D") + count("texSubImage2D")).toBe(uploadsAfterFirst);
    expect(count("deleteTexture")).toBe(0);
    expect(renderer.cachedTextures).toBe(2);
  });

  it("keeps a texture for a tile that scrolled off screen, so scrolling back costs no upload", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const first = entries([tile(0), tile(1)], 800);
    const second = entries([tile(2), tile(3)], 800);
    const budget = { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } };
    renderer.draw(options(first, budget));
    renderer.draw(options(second, budget));
    const uploads = count("texImage2D") + count("texSubImage2D");
    renderer.draw(options(first, budget)); // scroll back
    expect(count("texImage2D") + count("texSubImage2D")).toBe(uploads);
    expect(count("deleteTexture")).toBe(0);
    expect(renderer.cachedTextures).toBe(4);
  });

  it("re-uploads a refined tile into the same texture with texSubImage2D (same size)", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const budget = { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } };
    renderer.draw(options(entries([tile(0)], 800), budget));
    const creates = count("createTexture");
    const allocs = count("texImage2D");
    renderer.draw(options(entries([tile(0)], 800), budget)); // a fresh tile object, same geometry
    expect(count("createTexture")).toBe(creates);
    expect(count("texImage2D")).toBe(allocs);
    expect(count("texSubImage2D")).toBe(1);
  });

  it("re-allocates with texImage2D when the tile's dimensions changed", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const budget = { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } };
    renderer.draw(options(entries([tile(0, 512, 128, 65)], 800), budget));
    const allocs = count("texImage2D");
    const wider = tile(0, 512, 128, 129);
    renderer.draw(options(entries([wider], 800), budget));
    expect(count("texImage2D")).toBe(allocs + 1);
    expect(count("texSubImage2D")).toBe(0);
  });

  it("evicts least-recently-drawn textures over the cache cap, never the current frame's", () => {
    const { gl } = fakeGl();
    const bytes = TILE_FRAMES * BINS;
    const renderer = new SpectrogramGlRenderer(gl, bytes * 3);
    const budget = { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } };
    for (let i = 0; i < 6; i++) {
      renderer.draw(options(entries([tile(i)], 800), budget));
      expect(renderer.cachedBytes).toBeLessThanOrEqual(bytes * 3);
    }
    expect(renderer.cachedTextures).toBeLessThanOrEqual(3);
  });

  it("frees every texture on dispose", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    renderer.draw(options(entries([tile(0), tile(1)], 800), { uploadBudget: { maxTiles: 8, maxBytes: 1 << 30 } }));
    renderer.dispose();
    expect(count("deleteTexture")).toBe(3); // two tiles + the LUT
    expect(renderer.cachedBytes).toBe(0);
  });
});

describe("SpectrogramGlRenderer per-frame upload budget (H-47)", () => {
  it("uploads at most the budget per frame and reports the backlog", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const tiles = entries([tile(0), tile(1), tile(2), tile(3), tile(4)], 800);
    const budget = { maxTiles: 2, maxBytes: 1 << 30 };
    const first = renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(first.uploaded).toBe(2);
    expect(first.pendingUploads).toBe(3);
    expect(first.pendingBytes).toBe(3 * TILE_FRAMES * BINS);
    expect(count("texImage2D")).toBe(2 + 1); // + the LUT

    const second = renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(second.uploaded).toBe(2);
    expect(second.pendingUploads).toBe(1);
    const third = renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(third.uploaded).toBe(1);
    expect(third.pendingUploads).toBe(0);
    const fourth = renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(fourth.uploaded).toBe(0);
    expect(fourth.pendingUploads).toBe(0);
  });

  it("draws only the tiles whose pixels are on the GPU, and all of them once the backlog drains", () => {
    const { gl, count } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    const tiles = entries([tile(0), tile(1), tile(2)], 800);
    const budget = { maxTiles: 1, maxBytes: 1 << 30 };
    renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(count("drawArrays")).toBe(1);
    renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(count("drawArrays")).toBe(1 + 2);
    renderer.draw(options(tiles, { uploadBudget: budget }));
    expect(count("drawArrays")).toBe(1 + 2 + 3);
  });

  it("reports no backlog and no upload for an empty or zero-sized frame", () => {
    const { gl } = fakeGl();
    const renderer = new SpectrogramGlRenderer(gl);
    expect(renderer.draw(options([]))).toEqual({ pendingUploads: 0, pendingBytes: 0, uploaded: 0 });
    expect(renderer.draw(options([], { backingWidthPx: 0 }))).toEqual({
      pendingUploads: 0,
      pendingBytes: 0,
      uploaded: 0,
    });
  });
});
