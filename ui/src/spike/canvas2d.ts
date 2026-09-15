import { themeColors } from "../lib/theme/themeColors";
import { buildColormapLut } from "./colormap";
import type { DrawColumns } from "./webgl";

/** Canvas2D waveform renderer: one filled polygon (top edge = max, bottom edge = min) per frame. */
export function makeCanvas2dWaveformRenderer(canvas: HTMLCanvasElement): DrawColumns {
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas2D context unavailable");
  const midY = canvas.height / 2;
  const ampScale = canvas.height / 2;

  return (columns, numColumns) => {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = themeColors().wave.fill.css;
    ctx.beginPath();
    for (let col = 0; col < numColumns; col++) {
      const max = columns[col * 2 + 1] ?? 0;
      const y = midY - max * ampScale;
      if (col === 0) ctx.moveTo(col, y);
      else ctx.lineTo(col, y);
    }
    for (let col = numColumns - 1; col >= 0; col--) {
      const min = columns[col * 2] ?? 0;
      ctx.lineTo(col, midY - min * ampScale);
    }
    ctx.closePath();
    ctx.fill();
  };
}

export interface SpectrogramCanvas2dRenderer {
  /** One-time full-image upload (row-major, as fetched from `spike_spectrogram_texture`). */
  initialize(pixels: Uint8Array): void;
  pushColumnsAndDraw(columnData: Uint8Array, columnsToPush: number): void;
}

/**
 * Canvas2D spectrogram fallback: since there's no GPU sampler/shader, scrolling means physically
 * shifting every row's pixels (`copyWithin`, row-by-row so it doesn't bleed across rows) and
 * recoloring the newly exposed columns via a lookup table, then `putImageData`-ing the *whole*
 * canvas every frame — the CPU cost this measurement is meant to surface.
 */
export function makeCanvas2dSpectrogramRenderer(
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
): SpectrogramCanvas2dRenderer {
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas2D context unavailable");
  const lut = buildColormapLut();
  const imageData = ctx.createImageData(width, height);
  const rgba = imageData.data;

  return {
    initialize(pixels) {
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          const mag = pixels[y * width + x] ?? 0;
          const px = (y * width + x) * 4;
          rgba[px + 0] = lut[mag * 4 + 0] ?? 0;
          rgba[px + 1] = lut[mag * 4 + 1] ?? 0;
          rgba[px + 2] = lut[mag * 4 + 2] ?? 0;
          rgba[px + 3] = 255;
        }
      }
      ctx.putImageData(imageData, 0, 0);
    },
    pushColumnsAndDraw(columnData, columnsToPush) {
      const shift = Math.min(columnsToPush, width);
      if (shift > 0) {
        for (let y = 0; y < height; y++) {
          const rowStart = y * width * 4;
          rgba.copyWithin(rowStart, rowStart + shift * 4, rowStart + width * 4);
        }
        for (let c = 0; c < shift; c++) {
          const col = width - shift + c;
          for (let y = 0; y < height; y++) {
            const mag = columnData[c * height + y] ?? 0;
            const rowStart = y * width * 4;
            const px = rowStart + col * 4;
            rgba[px + 0] = lut[mag * 4 + 0] ?? 0;
            rgba[px + 1] = lut[mag * 4 + 1] ?? 0;
            rgba[px + 2] = lut[mag * 4 + 2] ?? 0;
            rgba[px + 3] = 255;
          }
        }
      }
      ctx.putImageData(imageData, 0, 0);
    },
  };
}
