/**
 * Owns one canvas's renderer choice for its whole lifetime (H-13, ADR-009 §4): tries to create a
 * WebGL2 context unless the preference forces Canvas2D, watches for `webglcontextlost` and
 * latches to Canvas2D forever once it fires (no recreate-on-`webglcontextrestored` attempts, per
 * ADR-009 §4's explicit choice), and exposes a `notice`-shaped payload once so the caller can tell
 * the user which path is active (ADR-009 §4: "Log a notice ... so the owner can see which path is
 * active").
 *
 * DOM-touching by nature (real `getContext`/event listeners), so {@link rendererMode.ts} carries
 * the actual decision logic for unit testing; this class is exercised in tests via a real
 * `<canvas>` element (jsdom provides one, including working `addEventListener`/`dispatchEvent`)
 * with `getContext` stubbed to return a fake WebGL2-shaped object, since jsdom itself has no GL.
 */

import { chooseRenderer, rendererKindAfterLoss, type RendererKind, type RendererPreference } from "./rendererMode";

/** H-113: both views paint an opaque background every frame, so the context is created with
 * `alpha: false` by default — an `alpha: true` (the WebGL2 default) backbuffer lets the browser
 * composite the canvas against whatever's behind it in the page using *our own* fragment alpha,
 * which is exactly wrong for a translucent overlay (the selection wash): its low alpha (e.g. 0.28)
 * ends up controlling how much of the *page* shows through the whole canvas element at that pixel,
 * not how much of the content drawn earlier in the same canvas shows through — the bug reported as
 * a flat, washed-out gray tint. `attributes` can still override this per caller if a future canvas
 * genuinely needs to composite with the page. */
const DEFAULT_CONTEXT_ATTRIBUTES: WebGLContextAttributes = { alpha: false };

export interface GlContextHostOptions {
  /** Extra `getContext("webgl2", ...)` attributes, merged over {@link DEFAULT_CONTEXT_ATTRIBUTES}. */
  attributes?: WebGLContextAttributes;
  /** Called once, the first time the effective kind is known (fresh creation, or a fallback
   * decided from a failed/missing context) — the caller turns this into a one-shot notice. Not
   * called again for the same reason a second time (ADR-009 §4 is about telling the owner once,
   * not spamming toasts on every redraw). */
  onKindDecided?: (kind: RendererKind, reason: "created" | "unavailable" | "forced") => void;
  /** Called exactly once, when `webglcontextlost` fires (ADR-009 §4). The host has already
   * switched `kind` to `"canvas2d"` by the time this runs. */
  onContextLost?: () => void;
}

export class GlContextHost {
  private _kind: RendererKind;
  private _gl: WebGL2RenderingContext | null = null;
  private _lost = false;
  private readonly canvas: HTMLCanvasElement;
  private readonly onKindDecided: GlContextHostOptions["onKindDecided"];
  private readonly onContextLost: GlContextHostOptions["onContextLost"];
  private readonly boundOnLost = (event: Event) => this.handleContextLost(event);

  constructor(canvas: HTMLCanvasElement, preference: RendererPreference, options: GlContextHostOptions = {}) {
    this.canvas = canvas;
    this.onKindDecided = options.onKindDecided;
    this.onContextLost = options.onContextLost;

    if (preference === "canvas2d") {
      this._kind = "canvas2d";
      this.onKindDecided?.("canvas2d", "forced");
    } else {
      let gl: WebGL2RenderingContext | null = null;
      try {
        gl = canvas.getContext("webgl2", {
          ...DEFAULT_CONTEXT_ATTRIBUTES,
          ...options.attributes,
        }) as WebGL2RenderingContext | null;
      } catch {
        gl = null;
      }
      this._gl = gl;
      this._kind = chooseRenderer(preference, gl !== null);
      this.onKindDecided?.(this._kind, gl !== null ? "created" : "unavailable");
    }

    // Only a real WebGL2 context can lose itself; a Canvas2D-forced host never attaches this.
    if (this._gl) {
      canvas.addEventListener("webglcontextlost", this.boundOnLost, false);
    }
  }

  private handleContextLost(event: Event): void {
    // Deliberately no `event.preventDefault()`: per ADR-009 §4 we do not attempt to recreate the
    // context on `webglcontextrestored`, so there is nothing to opt back into.
    this._lost = true;
    this._kind = rendererKindAfterLoss("auto", true, true);
    this._gl = null;
    this.onContextLost?.();
  }

  get kind(): RendererKind {
    return this._kind;
  }

  get lost(): boolean {
    return this._lost;
  }

  /** The live WebGL2 context, or `null` when the active kind is `"canvas2d"` (never available,
   * or lost). */
  get gl(): WebGL2RenderingContext | null {
    return this._kind === "webgl2" ? this._gl : null;
  }

  dispose(): void {
    this.canvas.removeEventListener("webglcontextlost", this.boundOnLost, false);
  }
}
