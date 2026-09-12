import { COLORMAP_GLSL } from "./colormap";

export function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("createShader failed");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`shader compile failed: ${log ?? "unknown error"}`);
  }
  return shader;
}

export function linkProgram(gl: WebGL2RenderingContext, vsSource: string, fsSource: string): WebGLProgram {
  const vs = compileShader(gl, gl.VERTEX_SHADER, vsSource);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, fsSource);
  const program = gl.createProgram();
  if (!program) throw new Error("createProgram failed");
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`program link failed: ${log ?? "unknown error"}`);
  }
  return program;
}

export type DrawColumns = (columns: Float32Array, numColumns: number) => void;

/**
 * WebGL2 waveform renderer: one (min, max) triangle-strip vertex pair per display column, updated
 * with `bufferSubData` into a buffer sized once for the canvas width (no per-frame allocation on
 * the GPU side).
 */
export function makeWebgl2WaveformRenderer(canvas: HTMLCanvasElement): DrawColumns {
  const gl = canvas.getContext("webgl2");
  if (!gl) throw new Error("WebGL2 context unavailable");
  const program = linkProgram(
    gl,
    `#version 300 es
     in vec2 aPos;
     void main() { gl_Position = vec4(aPos, 0.0, 1.0); }`,
    `#version 300 es
     precision mediump float;
     out vec4 outColor;
     void main() { outColor = vec4(0.498, 0.784, 1.0, 1.0); }`,
  );
  const aPos = gl.getAttribLocation(program, "aPos");
  const vao = gl.createVertexArray();
  const buffer = gl.createBuffer();
  const maxColumns = Math.max(1, canvas.width);
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, maxColumns * 4 * 4, gl.DYNAMIC_DRAW);
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
  gl.bindVertexArray(null);

  const verts = new Float32Array(maxColumns * 4);

  return (columns, numColumns) => {
    const n = Math.min(numColumns, maxColumns);
    for (let col = 0; col < n; col++) {
      const x = (col / Math.max(1, n - 1)) * 2 - 1;
      verts[col * 4 + 0] = x;
      verts[col * 4 + 1] = columns[col * 2] ?? 0;
      verts[col * 4 + 2] = x;
      verts[col * 4 + 3] = columns[col * 2 + 1] ?? 0;
    }
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0.086, 0.09, 0.102, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(program);
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, verts.subarray(0, n * 4));
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, n * 2);
    gl.bindVertexArray(null);
  };
}

export interface SpectrogramGl2Renderer {
  /** One-time full-image upload (row-major, as fetched from `spike_spectrogram_texture`). */
  initialize(pixels: Uint8Array): void;
  /** Uploads `columnData` (a `height`-tall strip) at the scrolling write head, then draws. */
  pushColumnsAndDraw(columnData: Uint8Array, columnsToPush: number): void;
}

/**
 * WebGL2 spectrogram renderer: an R8 texture written progressively (`texSubImage2D` for the new
 * strip each frame — the "scrolled/updated per frame" behaviour the ticket describes), sampled
 * with a scrolling horizontal offset and colored entirely in the fragment shader (the colormap
 * never causes a refetch, per ADR-003's spectrogram design note).
 */
export function makeWebgl2SpectrogramRenderer(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
): SpectrogramGl2Renderer {
  const gl = canvas.getContext("webgl2");
  if (!gl) throw new Error("WebGL2 context unavailable");
  const program = linkProgram(
    gl,
    `#version 300 es
     in vec2 aPos;
     out vec2 vUv;
     void main() {
       vUv = aPos * 0.5 + 0.5;
       gl_Position = vec4(aPos, 0.0, 1.0);
     }`,
    `#version 300 es
     precision mediump float;
     in vec2 vUv;
     out vec4 outColor;
     uniform sampler2D uTex;
     uniform float uScroll;
     ${COLORMAP_GLSL}
     void main() {
       float x = fract(vUv.x + uScroll);
       float mag = texture(uTex, vec2(x, vUv.y)).r;
       outColor = vec4(colormap(mag), 1.0);
     }`,
  );
  const aPos = gl.getAttribLocation(program, "aPos");
  const uTex = gl.getUniformLocation(program, "uTex");
  const uScroll = gl.getUniformLocation(program, "uScroll");

  const vao = gl.createVertexArray();
  const quadBuffer = gl.createBuffer();
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, quadBuffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
  gl.bindVertexArray(null);

  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(
    gl.TEXTURE_2D,
    0,
    gl.R8,
    width,
    height,
    0,
    gl.RED,
    gl.UNSIGNED_BYTE,
    new Uint8Array(width * height),
  );
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);

  let writeHead = 0;

  return {
    initialize(pixels) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RED, gl.UNSIGNED_BYTE, pixels);
    },
    pushColumnsAndDraw(columnData, columnsToPush) {
      // columnData is `columnsToPush * height` bytes, column-major (one column of `height`
      // samples at a time), matching how a real STFT would deliver newly hopped frames.
      for (let c = 0; c < columnsToPush; c++) {
        const x = (writeHead + c) % width;
        gl.texSubImage2D(
          gl.TEXTURE_2D,
          0,
          x,
          0,
          1,
          height,
          gl.RED,
          gl.UNSIGNED_BYTE,
          columnData.subarray(c * height, (c + 1) * height),
        );
      }
      writeHead = (writeHead + columnsToPush) % width;

      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.useProgram(program);
      gl.bindVertexArray(vao);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.uniform1i(uTex, 0);
      gl.uniform1f(uScroll, writeHead / width);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      gl.bindVertexArray(null);
    },
  };
}
