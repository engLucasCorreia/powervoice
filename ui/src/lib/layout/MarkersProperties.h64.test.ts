import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { MarkerDto } from "../ipc/bindings";
import { clearActionHandlers, dispatchAction } from "../shortcuts";
import { initMarkers, resetMarkersForTest, selectMarker } from "../markers/markers.svelte";
import { clearNotices } from "../state/notices.svelte";
import MarkersProperties from "./MarkersProperties.svelte";

/**
 * H-64 (SPEC-009 §2.8): the Markers panel deferrals — text/type filter, sortable column headers,
 * the panel menu's Delete All/Filtered Markers and Go to Next/Previous Marker, the `/` rename
 * shortcut, and the virtualized list (AC-15).
 */

const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const heightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

function stubSize(width: number, height: number): void {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => width });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => height });
}

function unstubSize(): void {
  if (widthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", widthDescriptor);
  }
  if (heightDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", heightDescriptor);
  }
}

afterEach(() => {
  clearMocks();
  clearActionHandlers();
  clearNotices();
  resetMarkersForTest();
  unstubSize();
});

function marker(id: number, pos: number, len = 0, name = `m${id}`, kind: MarkerDto["kind"] = "user"): MarkerDto {
  return { id, pos_samples: pos, len_samples: len, name, kind };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function mountWith(markers: MarkerDto[]): Promise<{ target: HTMLElement; app: object }> {
  mockIPC((cmd, args) => {
    if (cmd === "markers_get") {
      return markers;
    }
    if (cmd === "marker_delete") {
      return null;
    }
    if (cmd === "transport_seek") {
      return null;
    }
    return null;
  });
  await initMarkers();
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(MarkersProperties, { target });
  await settle();
  return { target, app };
}

function rowIds(target: HTMLElement): number[] {
  return [...target.querySelectorAll('[data-testid^="marker-row-"]')].map((el) =>
    Number(el.getAttribute("data-testid")!.replace("marker-row-", "")),
  );
}

describe("MarkersProperties filter (SPEC-009 §2.8, AC-14)", () => {
  it("the text filter box narrows the rendered rows", async () => {
    const { target, app } = await mountWith([
      marker(1, 0, 0, "Take 1"),
      marker(2, 100, 0, "Take 2"),
      marker(3, 200, 0, "Intro"),
    ]);

    const input = target.querySelector<HTMLInputElement>('[data-testid="markers-filter-text"]')!;
    input.value = "take";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(rowIds(target)).toEqual([1, 2]);
    expect(target.querySelector('[data-testid="markers-count"]')).toBeNull(); // no such testid any more
    expect(target.querySelector(".pv-panel-header .meta")?.textContent).toBe("2 of 3");

    unmount(app);
    target.remove();
  });

  it("filtered to zero rows shows the filtered-empty message, not the 'no markers' one", async () => {
    const { target, app } = await mountWith([marker(1, 0, 0, "Intro")]);

    const input = target.querySelector<HTMLInputElement>('[data-testid="markers-filter-text"]')!;
    input.value = "nomatch";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(target.querySelector('[data-testid="markers-empty-filtered"]')).not.toBeNull();
    expect(target.querySelector('[data-testid="markers-empty"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("the type filter (Select) narrows to Dropouts", async () => {
    const { target, app } = await mountWith([
      marker(1, 0, 0, "Intro"),
      marker(2, 100, 0, "Dropout 10 ms", "dropout"),
    ]);

    const select = target.querySelector<HTMLSelectElement>('[data-testid="markers-filter-type"]')!;
    const dropoutsIndex = [...select.options].findIndex((o) => o.text === "Dropouts");
    select.value = String(dropoutsIndex);
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();

    expect(rowIds(target)).toEqual([2]);

    unmount(app);
    target.remove();
  });
});

describe("MarkersProperties sort (SPEC-009 §2.8, AC-12)", () => {
  it("clicking the Start header (already the default) reverses to descending", async () => {
    const { target, app } = await mountWith([marker(1, 300), marker(2, 100), marker(3, 200)]);
    expect(rowIds(target)).toEqual([2, 3, 1]);

    const startHeader = target.querySelector<HTMLButtonElement>('[data-testid="markers-sort-start"]')!;
    expect(startHeader.getAttribute("aria-sort")).toBe("ascending");
    startHeader.click();
    flushSync();

    expect(rowIds(target)).toEqual([1, 3, 2]);
    expect(startHeader.getAttribute("aria-sort")).toBe("descending");

    unmount(app);
    target.remove();
  });

  it("clicking a different header (Name) sorts by it, ascending, numeric-aware", async () => {
    const { target, app } = await mountWith([
      marker(1, 0, 0, "Marker 10"),
      marker(2, 10, 0, "Marker 2"),
    ]);

    target.querySelector<HTMLButtonElement>('[data-testid="markers-sort-name"]')!.click();
    flushSync();

    expect(rowIds(target)).toEqual([2, 1]); // "Marker 2" before "Marker 10"

    unmount(app);
    target.remove();
  });
});

describe("MarkersProperties panel menu (SPEC-009 §2.6/§2.7)", () => {
  async function openMenu(target: HTMLElement): Promise<void> {
    target.querySelector<HTMLButtonElement>('[data-testid="markers-menu-trigger"]')!.click();
    flushSync();
  }

  it("Delete Filtered Markers is hidden with no active filter, shown once one is", async () => {
    const { target, app } = await mountWith([marker(1, 0), marker(2, 100, 0, "Dropout", "dropout")]);
    await openMenu(target);
    expect(target.querySelector('[data-testid="markers-menu-delete-filtered"]')).toBeNull();

    // The menu stays open (its item list is reactive) — toggling the trigger again would close
    // it instead of reopening it, since it's already open.
    const select = target.querySelector<HTMLSelectElement>('[data-testid="markers-filter-type"]')!;
    const dropoutsIndex = [...select.options].findIndex((o) => o.text === "Dropouts");
    select.value = String(dropoutsIndex);
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();

    const item = target.querySelector('[data-testid="markers-menu-delete-filtered"]');
    expect(item?.textContent).toContain("Delete Filtered Markers (1)");

    unmount(app);
    target.remove();
  });

  it("Delete All Markers deletes every marker via one marker_delete call", async () => {
    const calls: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [marker(1, 0), marker(2, 100), marker(3, 200)];
      if (cmd === "marker_delete") {
        calls.push(args);
        return null;
      }
      return null;
    });
    await initMarkers();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(MarkersProperties, { target });
    await settle();

    await openMenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="markers-menu-delete-all"]')!.click();
    await settle();

    expect(calls).toEqual([{ ids: [1, 2, 3] }]);
    expect(rowIds(target)).toEqual([]);
    expect(target.querySelector('[data-testid="markers-empty"]')).not.toBeNull();

    unmount(app);
    target.remove();
  });

  it("Go to Next/Previous Marker items are wired to the same navigation as the keymap", async () => {
    const seeks: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === "markers_get") return [marker(1, 1_000), marker(2, 2_000)];
      if (cmd === "transport_seek") {
        seeks.push(args);
        return null;
      }
      return null;
    });
    await initMarkers();
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(MarkersProperties, { target });
    await settle();

    await openMenu(target);
    target.querySelector<HTMLButtonElement>('[data-testid="markers-menu-go-next"]')!.click();
    flushSync();
    expect(seeks).toEqual([{ positionSamples: 1_000 }]);

    unmount(app);
    target.remove();
  });
});

describe("MarkersProperties rename shortcut (H-64, SPEC-009 §2.4)", () => {
  it("'/' opens the rename editor on the single panel selection, and is a no-op with none", async () => {
    const { target, app } = await mountWith([marker(1, 0, 0, "Marker 01")]);

    // No selection: a no-op.
    dispatchAction("marker.rename");
    flushSync();
    expect(target.querySelector('[data-testid="marker-rename-1"]')).toBeNull();

    selectMarker(1);
    dispatchAction("marker.rename");
    flushSync();

    const input = target.querySelector<HTMLInputElement>('[data-testid="marker-rename-1"]');
    expect(input).not.toBeNull();
    expect(input?.value).toBe("Marker 01");

    unmount(app);
    target.remove();
  });
});

describe("MarkersProperties virtualization (SPEC-009 §2.8/AC-15)", () => {
  it("renders far fewer DOM rows than the marker count, bounded by visible + 2*overscan", async () => {
    const many = Array.from({ length: 10_000 }, (_, i) => marker(i + 1, i * 10));
    stubSize(300, 220); // ~10 visible 22px rows
    const { target, app } = await mountWith(many);

    const rendered = rowIds(target);
    expect(rendered.length).toBeGreaterThan(0);
    // visible (ceil(220/22)+1 = 11) + 2*overscan(10) = 31.
    expect(rendered.length).toBeLessThanOrEqual(31);
    expect(rendered[0]).toBe(1); // scrolled to the top initially

    unmount(app);
    target.remove();
  });

  it("scrolling the list changes which markers are rendered", async () => {
    const many = Array.from({ length: 500 }, (_, i) => marker(i + 1, i * 10));
    stubSize(300, 220);
    const { target, app } = await mountWith(many);

    const list = target.querySelector<HTMLElement>('[data-testid="marker-list"]')!;
    Object.defineProperty(list, "scrollTop", { configurable: true, value: 22 * 200 });
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
    flushSync();

    const rendered = rowIds(target);
    expect(rendered).toContain(201); // firstVisible = 200 -> marker id 201 (1-based)
    expect(rendered).not.toContain(1);

    unmount(app);
    target.remove();
  });
});
