import { afterEach, describe, expect, it } from "vitest";
import { blockingModal, findTourTarget, visibleRect } from "./targets";

type Box = { left: number; top: number; width: number; height: number };

function boxed<T extends HTMLElement>(el: T, box: Box): T {
  el.getBoundingClientRect = () =>
    ({ ...box, x: box.left, y: box.top, right: box.left + box.width, bottom: box.top + box.height, toJSON: () => ({}) }) as DOMRect;
  return el;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tour targets (T-709)", () => {
  it("finds the first anchor that is on screen, skipping hidden and zero-size ones", () => {
    const hidden = document.createElement("div");
    hidden.hidden = true;
    const inHidden = boxed(document.createElement("span"), { left: 0, top: 0, width: 10, height: 10 });
    inHidden.dataset.tour = "a";
    hidden.appendChild(inHidden);
    const empty = boxed(document.createElement("span"), { left: 0, top: 0, width: 0, height: 0 });
    empty.dataset.tour = "b";
    const shown = boxed(document.createElement("span"), { left: 5, top: 5, width: 10, height: 10 });
    shown.dataset.tour = "c";
    document.body.append(hidden, empty, shown);

    expect(findTourTarget(["a", "b", "c"])).toBe(shown);
    expect(findTourTarget(["a", "b"])).toBeNull();
    expect(findTourTarget(undefined)).toBeNull();
  });

  it("cuts the target at its scrolling ancestors' edges", () => {
    const scroller = boxed(document.createElement("div"), { left: 0, top: 100, width: 300, height: 200 });
    scroller.style.overflowY = "auto";
    const plain = boxed(document.createElement("div"), { left: 0, top: 0, width: 50, height: 50 });
    const list = boxed(document.createElement("div"), { left: 20, top: 150, width: 260, height: 600 });
    plain.appendChild(list);
    scroller.appendChild(plain);
    document.body.appendChild(scroller);

    expect(visibleRect(list)).toEqual({ left: 20, top: 150, width: 260, height: 150 });

    const below = boxed(document.createElement("div"), { left: 20, top: 400, width: 100, height: 20 });
    scroller.appendChild(below);
    expect(visibleRect(below)).toBeNull();
  });

  it("pauses for a modal that doesn't hold the target, but not for one that does", () => {
    const modal = document.createElement("div");
    modal.setAttribute("aria-modal", "true");
    const inside = document.createElement("button");
    modal.appendChild(inside);
    const outside = document.createElement("button");
    document.body.append(modal, outside);

    expect(blockingModal(inside, null)).toBe(false);
    expect(blockingModal(outside, null)).toBe(true);
    expect(blockingModal(null, null)).toBe(true);
    modal.remove();
    expect(blockingModal(outside, null)).toBe(false);
  });
});
