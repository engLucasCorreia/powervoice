import { mount } from "svelte";
import App from "./App.svelte";
import "./lib/theme/tokens.css";
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
  const spikeEnv = await detectSpike();
  if (spikeEnv) {
    const { default: SpikeApp } = await import("./spike/SpikeApp.svelte");
    mount(SpikeApp, { target: root, props: { env: spikeEnv } });
    return;
  }
  mount(App, { target: root });
}

void bootstrap(target);
