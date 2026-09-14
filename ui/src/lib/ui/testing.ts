/**
 * Test helpers for the component kit (H-25). Not imported by production code.
 * Svelte 5: synthetic events dispatched by tests need `{ bubbles: true }` (MEMORY: delegated
 * handlers listen at the root), and state changes need `flushSync()` before asserting.
 */
import { createRawSnippet, flushSync, mount, unmount, type Component, type Snippet } from "svelte";

export interface Rendered {
  target: HTMLElement;
  cleanup: () => void;
}

export function render<Props extends Record<string, unknown>>(
  component: Component<Props>,
  props: Props,
): Rendered {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const app = mount(component, { target, props });
  flushSync();
  return {
    target,
    cleanup: () => {
      unmount(app);
      target.remove();
    },
  };
}

function escapeHtml(text: string): string {
  return text.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** A `children`-style snippet rendering plain text in a span. */
export function textSnippet(text: string): Snippet {
  return createRawSnippet(() => ({ render: () => `<span>${escapeHtml(text)}</span>` }));
}

export function byTestId<T extends Element = HTMLElement>(root: ParentNode, id: string): T {
  const el = root.querySelector<T & Element>(`[data-testid="${id}"]`);
  if (!el) {
    throw new Error(`missing [data-testid="${id}"]`);
  }
  return el;
}

export function key(el: Element, keyName: string, init: KeyboardEventInit = {}): void {
  el.dispatchEvent(new KeyboardEvent("keydown", { key: keyName, bubbles: true, ...init }));
  flushSync();
}

export function click(el: Element): void {
  (el as HTMLElement).click();
  flushSync();
}
