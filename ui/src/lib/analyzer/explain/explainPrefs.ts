/**
 * The one "Explain My Voice" preference that persists across sessions (H-115 ticket §2): whether
 * the graph's annotation cards/leader lines are shown. Mirrors `theme.svelte.ts`'s `localStorage`
 * pattern — best-effort, wrapped in `try/catch` (private browsing, tests, a blocked store).
 */
const ANNOTATIONS_STORAGE_KEY = "powervoice.explain.annotations";

/** `true` (the default, nothing stored yet or storage unavailable) unless the user last turned
 * annotations off. */
export function loadShowAnnotationsPref(): boolean {
  try {
    return localStorage.getItem(ANNOTATIONS_STORAGE_KEY) !== "0";
  } catch {
    return true;
  }
}

export function saveShowAnnotationsPref(show: boolean): void {
  try {
    localStorage.setItem(ANNOTATIONS_STORAGE_KEY, show ? "1" : "0");
  } catch {
    // Storage can be unavailable (private mode, tests); only the remembered choice is affected.
  }
}
