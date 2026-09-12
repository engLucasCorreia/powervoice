import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [svelte()],
  // Vitest's default resolve conditions favor the server/SSR export condition, which for
  // Svelte resolves to the server-only runtime (no `mount`/`onMount`). Force the browser
  // condition so tests exercise the same client build the app ships.
  resolve: {
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
