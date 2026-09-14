import { mount } from "svelte";
import App from "./App.svelte";
import "./lib/theme/tokens.css";
// H-25: the design system's role tokens (`--pv-*`).
import "./lib/theme/design-tokens.css";
// H-25 phase 2: legacy token names → design-system roles (see the file header).
import "./lib/theme/theme-bridge.css";
import { detectSpike } from "./spike/detect";

const target = document.getElementById("app");
if (!target) {
  throw new Error("missing #app root element");
}

/**
 * T-007 / ADR-009: `detectSpike()` calls the `spike_env` command, which only exists when
 * `powervoice-app` is built with `--features spike` (`just spike`). A normal production build has
 * no such command, `detectSpike()` resolves `null`, and this falls straight through to the real
 * `App` below — unchanged from before the spike existed.
 */
async function bootstrap(root: HTMLElement): Promise<void> {
  // H-25: the component gallery, development builds only (`npm --prefix ui run dev`, then open
  // http://localhost:1420/?gallery). `import.meta.env.DEV` is statically false in a production
  // build, so the gallery chunk is never shipped.
  const params = new URLSearchParams(window.location.search);
  // H-25: the real App against mocked IPC, development builds only (`?preview`, `&theme=light`).
  if (import.meta.env.DEV && params.has("preview")) {
    const { installPreviewIpc } = await import("./dev/previewIpc");
    installPreviewIpc(params.get("theme") === "light" ? "light" : "dark");
    mount(App, { target: root });
    return;
  }
  if (import.meta.env.DEV && params.has("gallery")) {
    const { default: Gallery } = await import("./lib/ui/Gallery.svelte");
    mount(Gallery, { target: root });
    return;
  }
  const spikeEnv = await detectSpike();
  if (spikeEnv) {
    const { default: SpikeApp } = await import("./spike/SpikeApp.svelte");
    mount(SpikeApp, { target: root, props: { env: spikeEnv } });
    return;
  }
  mount(App, { target: root });
}

void bootstrap(target);
