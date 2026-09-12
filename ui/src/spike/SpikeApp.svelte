<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../lib/i18n";
  import InputLog from "./InputLog.svelte";
  import { exitApp, makeFailureResults, runFullSpikeSuite, writeResults } from "./results";
  import { withTimeout } from "./timeout";
  import type { SpikeEnv, SpikeResults } from "./types";

  let { env }: { env: SpikeEnv } = $props();

  let status = $state<"idle" | "running" | "done" | "error">("idle");
  let step = $state("");
  let resultPath = $state("");
  let errorMessage = $state("");
  let results = $state<SpikeResults | null>(null);

  let waveformWebgl2Canvas: HTMLCanvasElement;
  let waveformCanvas2dCanvas: HTMLCanvasElement;
  let spectrogramWebgl2Canvas: HTMLCanvasElement;
  let spectrogramCanvas2dCanvas: HTMLCanvasElement;

  /** Every step inside `runFullSpikeSuite` already has its own bounded timeout (waveform/
   * spectrogram fetch + rAF benches, IPC throughput, telemetry) and catches its own failures — see
   * ADR-009 on why (rAF can be entirely withheld for an unfocused-but-visible Hyprland window).
   * This is one more layer of defense so a completely unforeseen hang still resolves and
   * `POWERVOICE_SPIKE_EXIT=1` still closes the app rather than leaving it stuck open forever. */
  const OVERALL_SUITE_TIMEOUT_MS = 4 * 60 * 1000;

  async function runSuite(): Promise<void> {
    status = "running";
    errorMessage = "";
    step = "";

    const suitePromise = runFullSpikeSuite(
      {
        waveformWebgl2: waveformWebgl2Canvas,
        waveformCanvas2d: waveformCanvas2dCanvas,
        spectrogramWebgl2: spectrogramWebgl2Canvas,
        spectrogramCanvas2d: spectrogramCanvas2dCanvas,
      },
      env.webkitDmabufDisabled,
      (s) => {
        step = s;
      },
    );

    let r: SpikeResults;
    try {
      r = await withTimeout(suitePromise, OVERALL_SUITE_TIMEOUT_MS, () =>
        makeFailureResults(
          env.webkitDmabufDisabled,
          `overall suite watchdog fired after ${OVERALL_SUITE_TIMEOUT_MS}ms (stuck at step "${step}")`,
        ),
      );
    } catch (e) {
      r = makeFailureResults(env.webkitDmabufDisabled, e instanceof Error ? e.message : String(e));
    }

    results = r;
    try {
      resultPath = await writeResults(r);
      status = r.incomplete ? "error" : "done";
      errorMessage = r.incomplete ? "incomplete run — see the written JSON for per-step errors" : "";
    } catch (e) {
      status = "error";
      errorMessage = `failed to write results: ${e instanceof Error ? e.message : String(e)}`;
    }

    if (env.exitAfter) {
      try {
        await exitApp();
      } catch {
        // best effort — nothing more we can do from here
      }
    }
  }

  onMount(() => {
    if (env.autoRun) {
      void runSuite();
    }
  });
</script>

<div class="spike" data-testid="spike-app">
  <header>
    <h1>{t("spike.title")}</h1>
    <p>{t("spike.subtitle")}</p>
    <p class="env" data-testid="spike-env">
      {t("spike.env.autoRun", { value: String(env.autoRun) })} ·
      {t("spike.env.exitAfter", { value: String(env.exitAfter) })} ·
      {t("spike.env.webkitDmabufDisabled", { value: String(env.webkitDmabufDisabled) })}
    </p>
  </header>

  <section class="controls">
    <button onclick={runSuite} disabled={status === "running"}>{t("spike.runButton")}</button>
    <button onclick={exitApp}>{t("spike.exitButton")}</button>
    <p class="status" data-testid="spike-status">
      {#if status === "running"}
        {t("spike.status.running", { step })}
      {:else if status === "done"}
        {t("spike.status.done", { path: resultPath })}
      {:else if status === "error"}
        {t("spike.status.error", { message: errorMessage })}
      {:else}
        {t("spike.status.idle")}
      {/if}
    </p>
  </section>

  <section class="bench">
    <h2>{t("spike.waveform.title")}</h2>
    <div class="canvas-row">
      <div>
        <p>{t("spike.canvas.webgl2")}</p>
        <canvas bind:this={waveformWebgl2Canvas} width="800" height="160"></canvas>
      </div>
      <div>
        <p>{t("spike.canvas.canvas2d")}</p>
        <canvas bind:this={waveformCanvas2dCanvas} width="800" height="160"></canvas>
      </div>
    </div>
  </section>

  <section class="bench">
    <h2>{t("spike.spectrogram.title")}</h2>
    <div class="canvas-row">
      <div>
        <p>{t("spike.canvas.webgl2")}</p>
        <canvas class="spectro" bind:this={spectrogramWebgl2Canvas} width="1024" height="512"></canvas>
      </div>
      <div>
        <p>{t("spike.canvas.canvas2d")}</p>
        <canvas class="spectro" bind:this={spectrogramCanvas2dCanvas} width="1024" height="512"></canvas>
      </div>
    </div>
  </section>

  <InputLog />

  {#if results}
    <details>
      <summary>{t("spike.rawResults")}</summary>
      <pre data-testid="spike-results-json">{JSON.stringify(results, null, 2)}</pre>
    </details>
  {/if}
</div>

<style>
  .spike {
    padding: 16px;
    max-width: 960px;
    margin: 0 auto;
    overflow-y: auto;
    height: 100vh;
  }

  .env {
    color: var(--text-secondary);
    font-size: 12px;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 12px 0;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 6px 12px;
    cursor: pointer;
  }

  button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .status {
    color: var(--text-secondary);
  }

  .bench {
    margin: 16px 0;
  }

  .canvas-row {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
  }

  canvas {
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    max-width: 100%;
  }

  canvas.spectro {
    width: 400px;
    height: 200px;
  }

  pre {
    max-height: 300px;
    overflow: auto;
    background: var(--surface-inset);
    padding: 8px;
    font-size: 11px;
  }
</style>
