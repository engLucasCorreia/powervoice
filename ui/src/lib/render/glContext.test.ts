import { describe, expect, it, vi } from "vitest";
import { GlContextHost } from "./glContext";

/**
 * jsdom has no real WebGL2 (MEMORY.md: "jsdom has no WebGL/2D canvas"), but `HTMLCanvasElement`
 * itself is a real DOM `EventTarget` there, so `webglcontextlost` dispatch/listening works exactly
 * like a browser — only `getContext` needs stubbing, which is exactly the seam
 * {@link GlContextHost} is built around (H-13 ticket: "context-loss handling (mocked)").
 */
function fakeCanvas(getContextImpl: (type: string) => unknown): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  const stubbable = canvas as unknown as { getContext: (type: string) => unknown };
  stubbable.getContext = vi.fn(getContextImpl);
  return canvas;
}

describe("GlContextHost (H-13, ADR-009 §4)", () => {
  it("creates WebGL2 and reports it once when available and preference is auto", () => {
    const fakeGl = {};
    const canvas = fakeCanvas(() => fakeGl);
    const decisions: Array<[string, string]> = [];
    const host = new GlContextHost(canvas, "auto", {
      onKindDecided: (kind, reason) => decisions.push([kind, reason]),
    });
    expect(host.kind).toBe("webgl2");
    expect(host.gl).toBe(fakeGl);
    expect(decisions).toEqual([["webgl2", "created"]]);
    host.dispose();
  });

  it("falls back to canvas2d and reports why when getContext returns null", () => {
    const canvas = fakeCanvas(() => null);
    const decisions: Array<[string, string]> = [];
    const host = new GlContextHost(canvas, "auto", {
      onKindDecided: (kind, reason) => decisions.push([kind, reason]),
    });
    expect(host.kind).toBe("canvas2d");
    expect(host.gl).toBeNull();
    expect(decisions).toEqual([["canvas2d", "unavailable"]]);
  });

  it("falls back to canvas2d when getContext throws", () => {
    const canvas = fakeCanvas(() => {
      throw new Error("boom");
    });
    const host = new GlContextHost(canvas, "auto");
    expect(host.kind).toBe("canvas2d");
    expect(host.gl).toBeNull();
  });

  it("creates the context with alpha:false by default (H-113: an alpha:true backbuffer lets a translucent overlay composite against the page instead of the canvas's own earlier content)", () => {
    const fakeGl = {};
    let capturedAttributes: WebGLContextAttributes | undefined;
    const canvas = fakeCanvas((_type: string) => fakeGl);
    const stubbable = canvas as unknown as { getContext: (type: string, attrs?: WebGLContextAttributes) => unknown };
    stubbable.getContext = vi.fn((_type: string, attrs?: WebGLContextAttributes) => {
      capturedAttributes = attrs;
      return fakeGl;
    });
    new GlContextHost(canvas, "auto");
    expect(capturedAttributes).toEqual({ alpha: false });
  });

  it("lets a caller override the default context attributes", () => {
    const fakeGl = {};
    let capturedAttributes: WebGLContextAttributes | undefined;
    const canvas = fakeCanvas(() => fakeGl);
    const stubbable = canvas as unknown as { getContext: (type: string, attrs?: WebGLContextAttributes) => unknown };
    stubbable.getContext = vi.fn((_type: string, attrs?: WebGLContextAttributes) => {
      capturedAttributes = attrs;
      return fakeGl;
    });
    new GlContextHost(canvas, "auto", { attributes: { alpha: true, antialias: true } });
    expect(capturedAttributes).toEqual({ alpha: true, antialias: true });
  });

  it("never calls getContext when the preference forces canvas2d", () => {
    const getContext = vi.fn(() => ({}));
    const canvas = fakeCanvas(getContext);
    const decisions: Array<[string, string]> = [];
    const host = new GlContextHost(canvas, "canvas2d", {
      onKindDecided: (kind, reason) => decisions.push([kind, reason]),
    });
    expect(host.kind).toBe("canvas2d");
    expect(getContext).not.toHaveBeenCalled();
    expect(decisions).toEqual([["canvas2d", "forced"]]);
  });

  it("latches to canvas2d on webglcontextlost and never re-creates the context (ADR-009 §4)", () => {
    const fakeGl = {};
    const canvas = fakeCanvas(() => fakeGl);
    let lostCalls = 0;
    const host = new GlContextHost(canvas, "auto", { onContextLost: () => lostCalls++ });
    expect(host.kind).toBe("webgl2");

    canvas.dispatchEvent(new Event("webglcontextlost"));

    expect(host.kind).toBe("canvas2d");
    expect(host.lost).toBe(true);
    expect(host.gl).toBeNull();
    expect(lostCalls).toBe(1);

    // A second loss event (shouldn't happen in a real browser once already lost, but the host
    // must stay latched regardless) doesn't flip anything back or call the callback again in a
    // way that would suggest a recreate attempt.
    canvas.dispatchEvent(new Event("webglcontextlost"));
    expect(host.kind).toBe("canvas2d");
    expect(lostCalls).toBe(2); // the listener still fires; it just keeps landing on canvas2d.
  });

  it("dispose() removes the listener so a later context-lost event is a no-op", () => {
    const fakeGl = {};
    const canvas = fakeCanvas(() => fakeGl);
    let lostCalls = 0;
    const host = new GlContextHost(canvas, "auto", { onContextLost: () => lostCalls++ });
    host.dispose();
    canvas.dispatchEvent(new Event("webglcontextlost"));
    expect(lostCalls).toBe(0);
    // kind is whatever it was at dispose time (webgl2) — disposal doesn't itself change the mode,
    // it just stops listening (the caller is tearing the view down anyway).
    expect(host.kind).toBe("webgl2");
  });

  it("a canvas2d-forced host never attaches a context-lost listener (nothing to lose)", () => {
    const canvas = document.createElement("canvas");
    let lostCalls = 0;
    new GlContextHost(canvas, "canvas2d", { onContextLost: () => lostCalls++ });
    canvas.dispatchEvent(new Event("webglcontextlost"));
    expect(lostCalls).toBe(0);
  });
});
