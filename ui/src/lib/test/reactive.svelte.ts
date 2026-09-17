/**
 * A reactive box for component tests (H-77): runes only work inside `.svelte`/`.svelte.ts`
 * files, so a test that has to change a prop after mounting keeps its `$state` here.
 */
export function box<T>(initial: T): { value: T } {
  let value = $state(initial);
  return {
    get value() {
      return value;
    },
    set value(next: T) {
      value = next;
    },
  };
}
