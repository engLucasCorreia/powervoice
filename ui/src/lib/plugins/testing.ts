/**
 * Plugin manager test helpers that aren't DTO fixtures (those moved to `ui/src/lib/test/fixtures.ts`
 * in H-29, the H-18 pattern — `pluginEntry`/`pluginFixtures`/`folderFixture`/`FLAGGED_ID`). Not
 * imported by production code.
 */

/** Lets pending IPC promises and effects settle. */
export async function settle(rounds = 6): Promise<void> {
  const { flushSync } = await import("svelte");
  for (let i = 0; i < rounds; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}
