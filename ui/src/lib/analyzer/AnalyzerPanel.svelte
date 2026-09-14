<script lang="ts">
  import { t } from "../i18n";
  import type { AnalyzerResponseDto } from "../ipc/bindings";
  import {
    fullFreqRange,
    frequencyTicks,
    formatHoverFreqHz,
    uForFreq,
    freqForU,
  } from "../spectrum/freqAxis";
  import { analyzerState, initAnalyzer, setAnalyzerResponse } from "./analyzer.svelte";
  import { createPeakHold, resetPeakHold, updatePeakHold, type PeakHoldBand } from "./peakHold";

  /**
   * The live output analyzer panel (T-208, SPEC-007 §2.9): a filled spectrum curve on a log
   * frequency axis (20 Hz .. min(Nyquist, 24 kHz)), a fixed −120 .. 0 dB axis (the floor/ceiling
   * picker is a later hardening pass — see the ticket report), a Fast/Medium/Slow response
   * selector and a peak-hold toggle. Canvas2D (SPEC-007 §4.1: the panel is small, ≤ 246 points at
   * 60 Hz). Renders in the bottom dock, to the right of the meter bridge.
   */

  const FLOOR_DB = -120;
  const CEIL_DB = 0;
  const RESPONSES: AnalyzerResponseDto[] = ["fast", "medium", "slow"];

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);
  let peakHoldEnabled = $state(true);
  let peaks: PeakHoldBand[] = [];
  let hover: { x: number; y: number } | null = $state(null);

  const analyzer = analyzerState();
  const frame = $derived(analyzer.frame);
  const nyquistHz = $derived(frame ? frame.sampleRateHz / 2 : 24_000);
  const fRange = $derived(fullFreqRange("log", Math.min(nyquistHz, 24_000)));

  $effect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    void initAnalyzer(analyzer.response).then((c) => {
      if (cancelled) {
        c();
      } else {
        cleanup = c;
      }
    });
    return () => {
      cancelled = true;
      cleanup?.();
    };
  });

  $effect(() => {
    const el = canvasEl;
    if (!el) {
      width = 0;
      height = 0;
      return;
    }
    width = el.clientWidth;
    height = el.clientHeight;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        width = Math.max(0, Math.round(entry.contentRect.width));
        height = Math.max(0, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Peak-hold ballistics run per animation frame (SPEC-007 §4.8 step 7), decoupled from the
  // ~60 Hz frame arrival rate.
  $effect(() => {
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dtS = Math.max(0, Math.min(0.25, (now - last) / 1000));
      last = now;
      const f = frame;
      if (f) {
        if (f.reset) {
          resetPeakHold(peaks);
        }
        if (peaks.length !== f.levelsDb.length) {
          peaks = createPeakHold(f.levelsDb.length);
        }
        if (peakHoldEnabled) {
          peaks = updatePeakHold(peaks, f.levelsDb, dtS);
        }
      }
      draw();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  function colorToken(name: string, fallback: string): string {
    if (!canvasEl) {
      return fallback;
    }
    const value = getComputedStyle(canvasEl).getPropertyValue(name).trim();
    return value || fallback;
  }

  function yForDb(db: number): number {
    const t = (db - FLOOR_DB) / (CEIL_DB - FLOOR_DB);
    return (1 - Math.min(1, Math.max(0, t))) * height;
  }

  function xForFreq(freqHz: number): number {
    const [fLo, fHi] = fRange;
    return uForFreq(freqHz, fLo, fHi, "log") * width;
  }

  function draw(): void {
    if (!canvasEl || width <= 0 || height <= 0) {
      return;
    }
    const ctx = canvasEl.getContext("2d");
    if (!ctx) {
      return; // jsdom in tests, or a browser with no 2D canvas support
    }
    const dpr = window.devicePixelRatio || 1;
    const backingW = Math.max(1, Math.round(width * dpr));
    const backingH = Math.max(1, Math.round(height * dpr));
    if (canvasEl.width !== backingW || canvasEl.height !== backingH) {
      canvasEl.width = backingW;
      canvasEl.height = backingH;
    }
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = colorToken("--surface-inset", "#16171a");
    ctx.fillRect(0, 0, width, height);

    const gridColor = colorToken("--analyzer-grid", "#34373d");
    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 1;
    ctx.globalAlpha = 0.6;
    for (let db = CEIL_DB; db >= FLOOR_DB; db -= 12) {
      const y = Math.round(yForDb(db)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    const [fLo, fHi] = fRange;
    for (const tick of frequencyTicks(fLo, fHi, "log", width, 28)) {
      const x = Math.round(xForFreq(tick.freqHz)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    const f = frame;
    if (f && f.levelsDb.length > 0) {
      const bottomY = yForDb(FLOOR_DB);
      ctx.beginPath();
      f.levelsDb.forEach((db, k) => {
        const x = xForFreq(bandCenterHzLocal(k, f.f0Hz, f.bandsPerOctave));
        const y = yForDb(Number.isFinite(db) ? db : FLOOR_DB);
        if (k === 0) {
          ctx.moveTo(x, y);
        } else {
          ctx.lineTo(x, y);
        }
      });
      const lastX = xForFreq(
        bandCenterHzLocal(f.levelsDb.length - 1, f.f0Hz, f.bandsPerOctave),
      );
      ctx.lineTo(lastX, bottomY);
      ctx.lineTo(xForFreq(bandCenterHzLocal(0, f.f0Hz, f.bandsPerOctave)), bottomY);
      ctx.closePath();
      ctx.fillStyle = colorToken("--analyzer-fill", "rgba(127, 200, 255, 0.28)");
      ctx.fill();

      if (peakHoldEnabled && peaks.length === f.levelsDb.length) {
        ctx.strokeStyle = colorToken("--analyzer-peak", "#ffb454");
        ctx.lineWidth = 1;
        ctx.beginPath();
        peaks.forEach((band, k) => {
          if (!Number.isFinite(band.value)) {
            return;
          }
          const x = xForFreq(bandCenterHzLocal(k, f.f0Hz, f.bandsPerOctave));
          const y = yForDb(band.value);
          ctx.moveTo(x - 2, y);
          ctx.lineTo(x + 2, y);
        });
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  function bandCenterHzLocal(k: number, f0Hz: number, bandsPerOctave: number): number {
    return f0Hz * 2 ** (k / bandsPerOctave);
  }

  function handleClick(): void {
    resetPeakHold(peaks);
  }

  function handleMouseMove(e: MouseEvent): void {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect) {
      return;
    }
    hover = { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function handleMouseLeave(): void {
    hover = null;
  }

  const hoverText = $derived.by(() => {
    if (!hover || width <= 0) {
      return null;
    }
    const [fLo, fHi] = fRange;
    const freq = freqForU(hover.x / width, fLo, fHi, "log");
    const db = FLOOR_DB + (1 - hover.y / Math.max(1, height)) * (CEIL_DB - FLOOR_DB);
    return t("analyzer.hover", { freq: formatHoverFreqHz(freq), db: db.toFixed(1) });
  });

  async function chooseResponse(r: AnalyzerResponseDto): Promise<void> {
    await setAnalyzerResponse(r);
  }
</script>

<section class="analyzer-panel" data-testid="analyzer-panel">
  <div class="header">
    <span>{t("panel.analyzer.title")}</span>
    <div class="responses" role="group" aria-label={t("panel.analyzer.title")}>
      {#each RESPONSES as r (r)}
        <button
          type="button"
          class:active={analyzer.response === r}
          onclick={() => chooseResponse(r)}
        >
          {t(`analyzer.response.${r}` as `analyzer.response.${AnalyzerResponseDto}`)}
        </button>
      {/each}
    </div>
    <label class="peak-hold">
      <input type="checkbox" bind:checked={peakHoldEnabled} />
      {t("analyzer.peak_hold")}
    </label>
  </div>
  <div class="canvas-wrap">
    <canvas
      bind:this={canvasEl}
      onclick={handleClick}
      onmousemove={handleMouseMove}
      onmouseleave={handleMouseLeave}
    ></canvas>
    {#if !frame}
      <div class="overlay">{t("analyzer.no_device")}</div>
    {:else if hoverText}
      <div class="hover" style:left="{hover?.x ?? 0}px">{hoverText}</div>
    {/if}
  </div>
</section>

<style>
  .analyzer-panel {
    display: flex;
    flex-direction: column;
    min-width: 240px;
    flex: 1;
    background: var(--surface-panel);
    border-top: 1px solid var(--surface-border);
    border-left: 1px solid var(--surface-border);
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0.5rem;
  }

  .responses {
    display: flex;
    gap: 0.15rem;
  }

  .responses button {
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    color: var(--text-secondary);
    border-radius: 2px;
    font-size: 0.7rem;
    padding: 0.05rem 0.35rem;
    cursor: pointer;
  }

  .responses button.active {
    background: var(--accent);
    color: var(--text-on-accent);
    border-color: var(--accent);
  }

  .peak-hold {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    margin-left: auto;
  }

  .canvas-wrap {
    position: relative;
    flex: 1;
    min-height: 2.5rem;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-disabled);
    pointer-events: none;
  }

  .hover {
    position: absolute;
    top: 2px;
    transform: translateX(-50%);
    background: var(--surface-panel-raised);
    border: 1px solid var(--surface-border);
    border-radius: 2px;
    padding: 0.05rem 0.3rem;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
    white-space: nowrap;
  }
</style>
