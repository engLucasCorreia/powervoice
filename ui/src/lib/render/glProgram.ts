/**
 * Small WebGL2 boilerplate shared by the waveform and spectrogram renderers (H-13): shader
 * compilation and a generic "colored triangles in device-pixel space" program used for every
 * overlay (selection, markers, playhead) and, for the waveform, the min/max column fill itself
 * (SPEC-006 §4.5). Thin and GL-only — the geometry it draws comes from the pure builders in
 * `quads.ts`/`overlayGeometry.ts`/`../waveform/webglGeometry.ts`, which is where the tested logic
 * lives (this file needs a real GL context, so it isn't unit tested directly).
 */

export function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) {
    throw new Error("WebGL2: createShader failed");
  }
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`WebGL2: shader compile failed: ${log ?? "unknown error"}`);
  }
  return shader;
}

export function linkProgram(gl: WebGL2RenderingContext, vsSource: string, fsSource: string): WebGLProgram {
  const vs = compileShader(gl, gl.VERTEX_SHADER, vsSource);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, fsSource);
  const program = gl.createProgram();
  if (!program) {
    throw new Error("WebGL2: createProgram failed");
  }
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`WebGL2: program link failed: ${log ?? "unknown error"}`);
  }
  return program;
}

const QUAD_VS = `#version 300 es
in vec2 aPos;
in vec4 aColor;
uniform vec2 uResolutionPx;
uniform float uPointSizePx;
out vec4 vColor;
void main() {
  vec2 ndc = (aPos / uResolutionPx) * 2.0 - 1.0;
  gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
  gl_PointSize = uPointSizePx;
  vColor = aColor;
}`;

const QUAD_FS = `#version 300 es
precision mediump float;
in vec4 vColor;
out vec4 outColor;
void main() {
  outColor = vColor;
}`;

/**
 * Draws a batch of `[x, y, r, g, b, a]` vertices (device pixels, straight-alpha color) built by
 * `quads.ts`. One program/VAO/buffer reused across frames; `bufferSubData` grows the backing
 * buffer only when a frame needs more vertices than the last (SPEC-006 §4.5: "uploaded via
 * `bufferSubData` ... rather than rebuilt from scratch" — here "rebuilt" refers to the GPU buffer
 * object, not the CPU-side vertex array, which is small and rebuilt every frame regardless).
 */
export class QuadProgram {
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly vao: WebGLVertexArrayObject;
  private readonly buffer: WebGLBuffer;
  private readonly uResolution: WebGLUniformLocation | null;
  private readonly uPointSize: WebGLUniformLocation | null;
  private capacityFloats = 0;

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.program = linkProgram(gl, QUAD_VS, QUAD_FS);
    const aPos = gl.getAttribLocation(this.program, "aPos");
    const aColor = gl.getAttribLocation(this.program, "aColor");
    this.uResolution = gl.getUniformLocation(this.program, "uResolutionPx");
    this.uPointSize = gl.getUniformLocation(this.program, "uPointSizePx");
    const vao = gl.createVertexArray();
    const buffer = gl.createBuffer();
    if (!vao || !buffer) {
      throw new Error("WebGL2: createVertexArray/createBuffer failed");
    }
    this.vao = vao;
    this.buffer = buffer;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    const stride = 6 * 4;
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, stride, 0);
    gl.enableVertexAttribArray(aColor);
    gl.vertexAttribPointer(aColor, 4, gl.FLOAT, false, stride, 2 * 4);
    gl.bindVertexArray(null);
  }

  /** `mode`: `gl.TRIANGLES` for filled batches (the default use), or `gl.LINE_STRIP`/`gl.POINTS`
   * for the waveform's raw-sample polyline/dots, which share this exact vertex layout.
   * `pointSizePx` is in real framebuffer pixels (`gl_PointSize` is never affected by
   * `uResolutionPx`'s coordinate-space trick) — pass `dotDiameterCssPx * devicePixelRatio` to get
   * a dot that's the same apparent CSS size as the Canvas2D fallback's `ctx.arc` dot. */
  draw(
    vertices: Float32Array,
    resolutionWidthPx: number,
    resolutionHeightPx: number,
    mode?: number,
    pointSizePx = 3,
  ): void {
    const gl = this.gl;
    const vertexCount = vertices.length / 6;
    if (vertexCount === 0) {
      return;
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    if (vertices.length > this.capacityFloats) {
      gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.DYNAMIC_DRAW);
      this.capacityFloats = vertices.length;
    } else {
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, vertices);
    }
    gl.useProgram(this.program);
    gl.uniform2f(this.uResolution, resolutionWidthPx, resolutionHeightPx);
    gl.uniform1f(this.uPointSize, pointSizePx);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(mode ?? gl.TRIANGLES, 0, vertexCount);
    gl.bindVertexArray(null);
  }

  dispose(): void {
    const gl = this.gl;
    gl.deleteBuffer(this.buffer);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
  }
}
