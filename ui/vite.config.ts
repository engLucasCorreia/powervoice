import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Tauri expects a fixed dev server port (tauri.conf.json `build.devUrl`) and needs the
// dev server to keep running when Rust source changes trigger a rebuild, so it ignores
// `src-tauri/**` (see https://v2.tauri.app/start/frontend/).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
  },
});
