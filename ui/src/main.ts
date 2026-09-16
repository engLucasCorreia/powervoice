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
  // H-26: `&scene=`, `&dialog=` and `&menu=` open fixture content (see `dev/previewIpc.ts`).
  // H-43: `VITE_PV_BENCH_PREVIEW=1 vite build` (`just bench-ui`'s release pass) keeps the preview
  // in a production bundle so the idle-CPU bench can measure release JS; an ordinary build leaves
  // the variable unset and drops it like the gallery.
  if ((import.meta.env.DEV || import.meta.env.VITE_PV_BENCH_PREVIEW === "1") && params.has("preview")) {
    const { installPreviewIpc } = await import("./dev/previewIpc");
    const { runPreviewScene } = await import("./dev/previewScenes");
    const { isThemePref } = await import("./lib/theme/theme.svelte");
    // T-708: `&theme=dark|light|system|high_contrast` (`high-contrast` also accepted).
    const wanted = (params.get("theme") ?? "dark").replace("-", "_");
    const options = {
      theme: isThemePref(wanted) ? wanted : ("dark" as const),
      scenes: (params.get("scene") ?? "").split(",").filter((s) => s !== ""),
      dialog: params.get("dialog"),
      // T-704: `&doc=60min` opens the 60-minute performance document (`scripts/bench/ui_frames.mjs`).
      longDocument: params.get("doc") === "60min",
      renderer: (["auto", "webgl2", "canvas2d"] as const).find((r) => r === params.get("renderer")),
    };
    installPreviewIpc(options);
    mount(App, { target: root });
    void runPreviewScene(options, params.get("menu"));
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
