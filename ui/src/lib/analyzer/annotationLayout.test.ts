import { describe, expect, it } from "vitest";
import { rectsOverlap, type Rect } from "../ui/axisLabels";
import { layoutAnnotations, type AnnotationItem } from "./annotationLayout";

/** A deterministic PRNG (mulberry32), so a failure reproduces (house pattern, see samplerColumn.test.ts). */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function item(id: string, x: number, y: number, priority: number, width = 120, height = 44): AnnotationItem {
  return { id, anchor: { x, y }, width, height, priority };
}

const PLOT: Rect = { x: 0, y: 0, width: 900, height: 500 };

describe("layoutAnnotations (H-93)", () => {
  it("places a lone label clear of its anchor, with a leader line reaching the anchor exactly", () => {
    const anchor = { x: 400, y: 250 };
    const { placed, dropped } = layoutAnnotations([item("a", anchor.x, anchor.y, 0)], { rect: PLOT });
    expect(dropped).toEqual([]);
    expect(placed).toHaveLength(1);
    const [p] = placed;
    expect(p?.leader.to).toEqual(anchor);
    // The label box must not itself cover the anchor point (it moved away, not onto it).
    const [rx, ry] = [p!.rect.x, p!.rect.y];
    const coversAnchor = anchor.x >= rx && anchor.x <= rx + p!.rect.width && anchor.y >= ry && anchor.y <= ry + p!.rect.height;
    expect(coversAnchor).toBe(false);
  });

  it("never mutates or moves the anchor", () => {
    const anchor = { x: 123.5, y: 77.25 };
    const it1 = item("only", anchor.x, anchor.y, 0);
    const { placed } = layoutAnnotations([it1], { rect: PLOT });
    expect(placed[0]?.item.anchor).toEqual({ x: 123.5, y: 77.25 });
    expect(it1.anchor).toEqual({ x: 123.5, y: 77.25 });
  });

  it("property: across randomised inputs, no two placed boxes overlap, every box stays inside the rect, and every leader line reaches its real anchor", () => {
    for (let seed = 0; seed < 60; seed++) {
      const random = rng(seed);
      const rect: Rect = { x: 0, y: 0, width: 400 + random() * 800, height: 300 + random() * 500 };
      const count = 3 + Math.floor(random() * 12);
      const items: AnnotationItem[] = [];
      for (let i = 0; i < count; i++) {
        items.push(
          item(
            `i${i}`,
            rect.x + random() * rect.width,
            rect.y + random() * rect.height,
            Math.floor(random() * 5),
            60 + random() * 140,
            28 + random() * 40,
          ),
        );
      }
      const { placed, dropped } = layoutAnnotations(items, { rect, maxLabels: 7 });

      expect(placed.length + dropped.length).toBe(items.length);

      for (const p of placed) {
        // Inside the rect.
        expect(p.rect.x).toBeGreaterThanOrEqual(rect.x - 1e-6);
        expect(p.rect.y).toBeGreaterThanOrEqual(rect.y - 1e-6);
        expect(p.rect.x + p.rect.width).toBeLessThanOrEqual(rect.x + rect.width + 1e-6);
        expect(p.rect.y + p.rect.height).toBeLessThanOrEqual(rect.y + rect.height + 1e-6);
        // Leader line's far end is the exact, unmoved anchor.
        expect(p.leader.to).toEqual(p.item.anchor);
      }

      for (let i = 0; i < placed.length; i++) {
        for (let j = i + 1; j < placed.length; j++) {
          expect(rectsOverlap(placed[i]!.rect, placed[j]!.rect)).toBe(false);
        }
      }
    }
  });

  it("caps primary annotations at maxLabels (default 7), dropping the rest by priority", () => {
    const items = Array.from({ length: 12 }, (_, i) => item(`i${i}`, 100 + i * 60, 100, i));
    const { placed, dropped } = layoutAnnotations(items, { rect: { x: 0, y: 0, width: 1600, height: 600 } });
    expect(placed).toHaveLength(7);
    expect(placed.map((p) => p.item.id)).toEqual(["i0", "i1", "i2", "i3", "i4", "i5", "i6"]);
    expect(dropped.map((d) => d.id)).toEqual(["i7", "i8", "i9", "i10", "i11"]);
  });

  it("honours a caller-supplied lower cap", () => {
    const items = Array.from({ length: 5 }, (_, i) => item(`i${i}`, 100 + i * 60, 100, i));
    const { placed, dropped } = layoutAnnotations(items, { rect: { x: 0, y: 0, width: 1600, height: 600 }, maxLabels: 3 });
    expect(placed).toHaveLength(3);
    expect(dropped.map((d) => d.id)).toEqual(["i3", "i4"]);
  });

  it("when space runs out, drops the lowest-priority items first and keeps the higher-priority ones placed", () => {
    // Ten anchors crammed into a small, single-row box: not all ten labels can be non-overlapping.
    const items = Array.from({ length: 10 }, (_, i) => item(`i${i}`, 20 + i * 8, 60, i, 100, 40));
    const { placed, dropped } = layoutAnnotations(items, { rect: { x: 0, y: 0, width: 200, height: 140 }, maxLabels: 10 });
    expect(placed.length).toBeGreaterThan(0);
    expect(placed.length).toBeLessThan(items.length);
    const placedPriorities = placed.map((p) => p.item.priority).sort((a, b) => a - b);
    const droppedPriorities = dropped.map((d) => d.priority).sort((a, b) => a - b);
    // Every kept priority is lower (more important) than every dropped priority: no lower-priority
    // item is kept ahead of a higher-priority one that could have taken its spot.
    expect(Math.max(...placedPriorities)).toBeLessThan(Math.min(...droppedPriorities));
  });

  it("narrow viewport: the same solver, capped to 3 on a small rect, places the top 3 without tangling", () => {
    // Mirrors the mobile layout (H-92): "mobile shows the top three annotations on the graph and
    // the rest as cards beneath" — the caller passes maxLabels: 3 on the narrow rect; the solver
    // must still find non-overlapping spots for those three and report the rest as dropped.
    const items = [
      item("i0", 40, 70, 0, 90, 40),
      item("i1", 150, 90, 1, 90, 40),
      item("i2", 260, 60, 2, 90, 40),
      item("i3", 60, 100, 3, 90, 40),
      item("i4", 200, 40, 4, 90, 40),
      item("i5", 100, 50, 5, 90, 40),
      item("i6", 300, 100, 6, 90, 40),
    ];
    const narrow: Rect = { x: 0, y: 0, width: 340, height: 180 };
    const { placed, dropped } = layoutAnnotations(items, { rect: narrow, maxLabels: 3 });

    expect(placed).toHaveLength(3);
    expect(placed.map((p) => p.item.id)).toEqual(["i0", "i1", "i2"]);
    expect(dropped.map((d) => d.id)).toEqual(["i3", "i4", "i5", "i6"]);
    for (const p of placed) {
      expect(p.rect.x).toBeGreaterThanOrEqual(0);
      expect(p.rect.y).toBeGreaterThanOrEqual(0);
      expect(p.rect.x + p.rect.width).toBeLessThanOrEqual(narrow.width);
      expect(p.rect.y + p.rect.height).toBeLessThanOrEqual(narrow.height);
    }
    for (let i = 0; i < placed.length; i++) {
      for (let j = i + 1; j < placed.length; j++) {
        expect(rectsOverlap(placed[i]!.rect, placed[j]!.rect)).toBe(false);
      }
    }
  });

  it("narrow viewport, over capacity: even a tiny rect degrades by dropping instead of overlapping", () => {
    // Even when 3 don't all fit, the solver never resolves the shortage by letting boxes overlap:
    // it keeps as many top-priority items as fit and drops the rest.
    const items = Array.from({ length: 5 }, (_, i) => item(`i${i}`, 20 + i * 10, 40, i, 120, 44));
    const tiny: Rect = { x: 0, y: 0, width: 130, height: 90 };
    const { placed, dropped } = layoutAnnotations(items, { rect: tiny, maxLabels: 3 });

    expect(placed.length + dropped.length).toBe(items.length);
    expect(placed.length).toBeLessThanOrEqual(3);
    for (let i = 0; i < placed.length; i++) {
      for (let j = i + 1; j < placed.length; j++) {
        expect(rectsOverlap(placed[i]!.rect, placed[j]!.rect)).toBe(false);
      }
    }
  });

  it("never places a label on top of a hard-reserved rect (e.g. a legend or hover readout)", () => {
    const reserved: Rect[] = [{ x: 0, y: 0, width: 900, height: 120 }];
    const { placed } = layoutAnnotations([item("a", 450, 60, 0)], { rect: PLOT, reserved });
    for (const p of placed) {
      expect(rectsOverlap(p.rect, reserved[0]!)).toBe(false);
    }
  });

  it("prefers a spot clear of an avoidable region (a significant curve peak) when one exists", () => {
    // A peak rectangle sits directly above the anchor; a free spot exists elsewhere, e.g. below.
    const avoid: Rect[] = [{ x: 350, y: 50, width: 200, height: 100 }];
    const { placed } = layoutAnnotations([item("a", 450, 200, 0, 120, 40)], {
      rect: PLOT,
      avoid,
    });
    expect(placed).toHaveLength(1);
    expect(rectsOverlap(placed[0]!.rect, avoid[0]!)).toBe(false);
  });

  it("still places a label over an avoidable region rather than dropping it, when nothing else fits", () => {
    // The avoid rect covers almost the whole plot; only overlapping it (never a reserved rect,
    // never another label) leaves any room at all.
    const avoid: Rect[] = [{ x: 0, y: 0, width: 900, height: 500 }];
    const { placed, dropped } = layoutAnnotations([item("a", 450, 250, 0, 100, 30)], {
      rect: PLOT,
      avoid,
    });
    expect(dropped).toEqual([]);
    expect(placed).toHaveLength(1);
  });

  it("handles no items without error", () => {
    const { placed, dropped } = layoutAnnotations([], { rect: PLOT });
    expect(placed).toEqual([]);
    expect(dropped).toEqual([]);
  });

  it("drops a label that is simply too big for the rect, without throwing", () => {
    const { placed, dropped } = layoutAnnotations([item("huge", 450, 250, 0, 2000, 2000)], { rect: PLOT });
    expect(placed).toEqual([]);
    expect(dropped.map((d) => d.id)).toEqual(["huge"]);
  });

  it("snapshot: a realistic voice-spectrum arrangement places every label without overlap", () => {
    // Mirrors annotated_voice_spectrum.png: F0 marker, strongest-peak marker, and several
    // region bands spread across a log-frequency (already-mapped-to-px) axis.
    const items: AnnotationItem[] = [
      item("strongest-peak", 520, 90, 0, 220, 60),
      item("f0", 300, 140, 1, 200, 50),
      item("low-mids", 360, 300, 2, 210, 50),
      item("presence", 1180, 340, 3, 190, 50),
      item("sibilance", 1360, 420, 4, 220, 50),
      item("air", 1520, 380, 5, 200, 50),
      item("rumble", 120, 460, 6, 200, 50),
    ];
    const rect: Rect = { x: 0, y: 0, width: 1700, height: 520 };
    const { placed, dropped } = layoutAnnotations(items, { rect, maxLabels: 7 });

    expect(dropped).toEqual([]);
    expect(placed).toHaveLength(7);
    for (let i = 0; i < placed.length; i++) {
      expect(placed[i]!.rect.x).toBeGreaterThanOrEqual(0);
      expect(placed[i]!.rect.y).toBeGreaterThanOrEqual(0);
      expect(placed[i]!.rect.x + placed[i]!.rect.width).toBeLessThanOrEqual(rect.width);
      expect(placed[i]!.rect.y + placed[i]!.rect.height).toBeLessThanOrEqual(rect.height);
      for (let j = i + 1; j < placed.length; j++) {
        expect(rectsOverlap(placed[i]!.rect, placed[j]!.rect)).toBe(false);
      }
      // Each leader line still points at the real anchor.
      expect(placed[i]!.leader.to).toEqual(items[i]!.anchor);
    }
  });
});
