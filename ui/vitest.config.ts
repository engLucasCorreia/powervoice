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
    // H-25: Vitest stubs every CSS import (even `?raw`) to "" unless it is listed here. The
    // design-tokens contrast test reads the real stylesheet text.
    css: { include: [/(design-tokens|tokens|theme-bridge)\.css/] },
    include: ["src/**/*.test.ts"],
  },
});
