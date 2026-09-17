import { describe, expect, it } from "vitest";
import { WaveformGlRenderer, type WaveformGlDrawOptions } from "./webglRenderer";

/**
 * H-79 (SPEC-006 §2.12 Amendment 2): the draw order — selection underlay, then wave content, then
 * the rest of the overlay (border/markers/playhead) — against a recording fake WebGL2 context.
 * jsdom has no WebGL (MEMORY.md, see `spectrogram/webglRenderer.test.ts`'s matching fake), but
 * every call `QuadProgram`/`WaveformGlRenderer` makes is a plain method on the context object, so
 * a fake records exactly what reached the driver.
 */
function fakeGl(): { gl: WebGL2RenderingContext; drawnVertexArrays: Float32Array[] } {
  const constants: Record<string, number> = {
    ARRAY_BUFFER: 1,
    DYNAMIC_DRAW: 2,
    FLOAT: 3,
    TRIANGLES: 4,
    COLOR_BUFFER_BIT: 5,
    COMPILE_STATUS: 6,
    LINK_STATUS: 7,
    VERTEX_SHADER: 8,
    FRAGMENT_SHADER: 9,
    POINTS: 10,
  };
  let nextObject = 1;
  const object = () => () => ({ id: nextObject++ });
  const noop = () => () => {};
  // The vertex buffer most recently uploaded via bufferData/bufferSubData — drawArrays records a
  // (shallow) copy of it, since QuadProgram re-binds/re-uploads once per `.draw()` call.
  let pendingVertices: Float32Array | null = null;
  const drawnVertexArrays: Float32Array[] = [];
  const gl: Record<string, unknown> = {
    ...constants,
    createShader: object(),
    createProgram: object(),
    createVertexArray: object(),
    createBuffer: object(),
    shaderSource: noop(),
    compileShader: noop(),
    getShaderParameter: () => true,
    getProgramParameter: () => true,
    attachShader: noop(),
    linkProgram: noop(),
    deleteShader: noop(),
    deleteProgram: noop(),
    deleteBuffer: noop(),
    deleteVertexArray: noop(),
    getAttribLocation: () => 0,
    getUniformLocation: object(),
    bindVertexArray: noop(),
    bindBuffer: noop(),
    vertexAttribPointer: noop(),
    enableVertexAttribArray: noop(),
    useProgram: noop(),
    uniform2f: noop(),
    uniform1f: noop(),
    viewport: noop(),
    clearColor: noop(),
    clear: noop(),
    bufferData: (_target: number, data: Float32Array) => {
      pendingVertices = data;
    },
    bufferSubData: (_target: number, _offset: number, data: Float32Array) => {
      pendingVertices = data;
    },
    drawArrays: () => {
      if (pendingVertices) {
        drawnVertexArrays.push(pendingVertices);
      }
    },
  };
  return { gl: gl as unknown as WebGL2RenderingContext, drawnVertexArrays };
}

function rgbaVertices(r: number, count: number): Float32Array {
  // A tiny triangle whose color channel `r` distinguishes it from the other batches in this test.
  const out = new Float32Array(count * 6);
  for (let i = 0; i < count; i++) {
    out.set([i, i, r, 0, 0, 1], i * 6);
  }
  return out;
}

const BASE: WaveformGlDrawOptions = {
  backingWidthPx: 100,
  backingHeightPx: 100,
  cssWidthPx: 100,
  cssHeightPx: 100,
  devicePixelRatio: 1,
  background: [0, 0, 0, 1],
  underlay: null,
  content: null,
  overlay: null,
};

describe("WaveformGlRenderer draw order (H-79, SPEC-006 §2.12 Amendment 2)", () => {
  it("draws the underlay before the content, and the content before the overlay", () => {
    const { gl, drawnVertexArrays } = fakeGl();
    const renderer = new WaveformGlRenderer(gl);
    const underlay = rgbaVertices(0.1, 3);
    const contentVertices = rgbaVertices(0.5, 3);
    const overlay = rgbaVertices(0.9, 3);
    renderer.draw({
      ...BASE,
      underlay,
      content: { mode: "columns", vertices: contentVertices },
      overlay,
    });
    expect(drawnVertexArrays).toHaveLength(3);
    expect(drawnVertexArrays[0]).toBe(underlay);
    expect(drawnVertexArrays[1]).toBe(contentVertices);
    expect(drawnVertexArrays[2]).toBe(overlay);
  });

  it("skips the underlay draw call entirely with no selection (null/empty)", () => {
    const { gl, drawnVertexArrays } = fakeGl();
    const renderer = new WaveformGlRenderer(gl);
    const contentVertices = rgbaVertices(0.5, 3);
    renderer.draw({ ...BASE, underlay: null, content: { mode: "columns", vertices: contentVertices } });
    expect(drawnVertexArrays).toEqual([contentVertices]);

    renderer.draw({
      ...BASE,
      underlay: new Float32Array(0),
      content: { mode: "columns", vertices: contentVertices },
    });
    expect(drawnVertexArrays).toEqual([contentVertices, contentVertices]);
  });
});
