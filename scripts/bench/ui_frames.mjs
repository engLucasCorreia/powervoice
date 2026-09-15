#!/usr/bin/env node
/**
 * T-704: the headless UI frame-time sweep — PROMPT §2 "60 fps scroll/zoom", SPEC-006 AC-18
 * (waveform) and SPEC-007 AC-10 (split view, tiles cached) — on a 60-minute document.
 *
 * Starts its own Vite dev server (`ui/`, `--port 5193 --strictPort` by default) and a headless
 * Chromium over the DevTools protocol, opens the real App on mocked IPC
 * (`?preview&scene=document|spectral&doc=60min`: `ui/src/dev/previewIpc.ts` serves a 60-min
 * document's peaks from a precomputed pyramid) at 1280×720 and 2126×850, and drives a scripted
 * sweep through the waveform's real wheel handler: zoom in/out between the whole file and single
 * samples (Ctrl+wheel, one √2 step every second frame) for the first half, then scroll (wheel
 * deltaX) at a ~10 s view for the second half. A warm-up sweep runs first (it fills the tile and
 * pyramid caches: SPEC-007 AC-10 is "with tiles cached"); the measured sweep records every
 * `requestAnimationFrame` timestamp.
 *
 * Chromium runs with `--disable-frame-rate-limit --disable-gpu-vsync`, so a frame's rAF delta is
 * its real cost (script + style + layout + paint + composite), not the 16.7 ms vsync cadence, and
 * on the machine's GPU (`--enable-gpu --ignore-gpu-blocklist`: ANGLE over Mesa — the stack
 * WebKitGTK uses; `--no-gpu` falls back to SwiftShader, CPU-only). The GL renderer string is
 * logged.
 * Reported per case as `BENCH_RESULT` lines (→ stdout and `target/bench/ui.log`): p50/p95/p99/max
 * frame time, frames over 50 ms and the frame rate. Targets: p50 ≤ 16.7 ms and p99 ≤ 50 ms with at
 * most 1 frame over 50 ms (SPEC-006 AC-18 / SPEC-007 AC-10); p95 ≤ 16.7 ms (T-704's reading of
 * "60 fps").
 *
 * Idle baseline (H-43): before the sweeps, each case also sits idle for 4 s while the page's
 * main-thread task time (CDP `Performance.getMetrics` `TaskDuration`) and its rAF frames are
 * counted. With uncapped rAF the perpetual draw loops run as fast as they can, so the reported
 * `idle_*_main_thread_ms_per_frame` × 60 is the main-thread share of one core those loops cost at
 * a 60 Hz display (`idle_*_pct_core_at_60hz`); an app that drew only on demand would show ~0.
 *
 * Caveat: the owner's reference renderer is WebKitGTK (ADR-009); Chromium stands in for it
 * headlessly. The preview's Settings pick the Canvas2D waveform renderer.
 *
 * Usage (`just bench-ui`): node scripts/bench/ui_frames.mjs [--port 5193] [--seconds 10]
 *   [--out target/bench/ui.log] [--case document:2126x850]... [--profile] [--renderer canvas2d].
 *   `--renderer auto|webgl2|canvas2d` picks the renderer Setting (default: both canvas2d — the
 *   fallback, which does its per-pixel work in JS — and auto, the app default: WebGL2 first). `--case` limits the run
 *   to the given cases; `--profile` also samples the page's CPU profile during each measured sweep
 *   and prints the top self-time functions (diagnostics, not BENCH_RESULT lines). Needs `chromium`
 *   on PATH and `npm ci --prefix ui`.
 */
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { loadavg, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : fallback;
};
const PORT = Number(opt("port", "5193"));
const SECONDS = Number(opt("seconds", "10"));
const OUT = opt("out", join(ROOT, "target", "bench", "ui.log"));
const ALL_CASES = [
  { scene: "document", width: 1280, height: 720 },
  { scene: "document", width: 2126, height: 850 },
  { scene: "spectral", width: 1280, height: 720 },
  { scene: "spectral", width: 2126, height: 850 },
];
const wanted = args.flatMap((a, i) => (a === "--case" && i + 1 < args.length ? [args[i + 1]] : []));
const CASES = wanted.length
  ? ALL_CASES.filter((c) => wanted.includes(`${c.scene}:${c.width}x${c.height}`))
  : ALL_CASES;
const PROFILE = args.includes("--profile");
const GPU_FLAGS = args.includes("--no-gpu") ? [] : ["--enable-gpu", "--ignore-gpu-blocklist"];
const RENDERERS = args.includes("--renderer") ? [opt("renderer", "canvas2d")] : ["canvas2d", "auto"];
const FRAME_BUDGET_MS = 16.7;
const LONG_FRAME_MS = 50;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lines = [];
const say = (line) => {
  console.log(line);
  lines.push(line);
};

function result(name, value, unit, target, op) {
  const status = target === undefined ? "info" : (op === "le" ? value <= target : value >= target) ? "pass" : "fail";
  say(
    `BENCH_RESULT crate=ui name=${name} value=${value} unit=${unit} target=${target ?? "-"} op=${target === undefined ? "-" : op} status=${status}`,
  );
}

function percentile(sorted, p) {
  if (sorted.length === 0) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))];
}

const children = [];
function killAll() {
  for (const child of children) {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      /* already gone */
    }
  }
}
process.on("SIGINT", () => {
  killAll();
  process.exit(130);
});

async function waitFor(fn, timeoutMs, what) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {
      /* not yet */
    }
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}

// The sweep, evaluated in the page: returns the measured rAF deltas.
const SWEEP = `(async (seconds) => {
  const canvas = document.querySelector('[data-testid="waveform-canvas"]');
  if (!canvas) throw new Error("no waveform canvas");
  const rect = canvas.getBoundingClientRect();
  const at = { clientX: rect.left + rect.width * 0.37, clientY: rect.top + rect.height / 2 };
  const wheel = (init) =>
    canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, ...at, ...init }));
  const durationMs = seconds * 1000;
  const deltas = [];
  let scrollReady = false;
  return await new Promise((resolve) => {
    let start = -1;
    let last = -1;
    let frame = 0;
    const step = (now) => {
      if (start < 0) {
        start = now;
        last = now;
        requestAnimationFrame(step);
        return;
      }
      deltas.push(now - last);
      last = now;
      frame += 1;
      const t = now - start;
      if (t >= durationMs) {
        resolve({ deltas, canvasWidth: rect.width, canvasHeight: rect.height });
        return;
      }
      if (t < durationMs / 2) {
        if (frame % 2 === 0) {
          const k = Math.floor(frame / 2) % 72;
          wheel({ deltaY: k < 36 ? -100 : 100, ctrlKey: true });
        }
      } else {
        if (!scrollReady) {
          for (let i = 0; i < 40; i++) wheel({ deltaY: 100, ctrlKey: true });
          for (let i = 0; i < 17; i++) wheel({ deltaY: -100, ctrlKey: true });
          scrollReady = true;
        }
        wheel({ deltaX: 24 });
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
})`;

async function main() {
  mkdirSync(dirname(OUT), { recursive: true });
  const vite = spawn("npm", ["--prefix", join(ROOT, "ui"), "run", "dev", "--", "--port", String(PORT), "--strictPort"], {
    detached: true,
    stdio: "ignore",
  });
  children.push(vite);
  const base = `http://localhost:${PORT}/`;
  await waitFor(async () => (await fetch(base)).ok, 60_000, `Vite on ${base}`);

  const profile = mkdtempSync(join(tmpdir(), "pv-ui-frames-"));
  const chrome = spawn(
    "chromium",
    [
      "--headless=new",
      "--remote-debugging-port=0",
      "--no-first-run",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--disable-frame-rate-limit",
      "--disable-gpu-vsync",
      "--disable-background-timer-throttling",
      "--disable-renderer-backgrounding",
      ...GPU_FLAGS,
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { detached: true, stdio: "ignore" },
  );
  children.push(chrome);
  try {
    const portFile = join(profile, "DevToolsActivePort");
    const debugPort = await waitFor(
      () => existsSync(portFile) && readFileSync(portFile, "utf8").split("\n")[0],
      20_000,
      "Chromium DevTools port",
    );
    const version = await waitFor(
      async () => (await fetch(`http://127.0.0.1:${debugPort}/json/version`)).json(),
      20_000,
      "DevTools endpoint",
    );
    const ws = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((r) => ws.addEventListener("open", r, { once: true }));
    let id = 0;
    const pending = new Map();
    const consoleErrors = new Map();
    const consoleTexts = new Map();
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      } else if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") {
        consoleErrors.set(msg.sessionId, (consoleErrors.get(msg.sessionId) ?? 0) + 1);
        const text = msg.params.args.map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 200);
        const seen = consoleTexts.get(msg.sessionId) ?? new Set();
        seen.add(text);
        consoleTexts.set(msg.sessionId, seen);
      }
    });
    const send = (method, params = {}, sessionId) =>
      new Promise((resolve, reject) => {
        const mid = ++id;
        pending.set(mid, (msg) => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
        ws.send(JSON.stringify({ id: mid, method, params, sessionId }));
      });
    const evaluate = async (expression, sessionId) => {
      const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
      return r.result.value;
    };

    const probe = await send("Target.createTarget", { url: "about:blank" });
    const probeSession = (await send("Target.attachToTarget", { targetId: probe.targetId, flatten: true })).sessionId;
    const glRenderer = await evaluate(
      `(() => { const gl = document.createElement("canvas").getContext("webgl2"); if (!gl) return "no WebGL2"; ` +
        `const d = gl.getExtension("WEBGL_debug_renderer_info"); return d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); })()`,
      probeSession,
    );
    await send("Target.closeTarget", { targetId: probe.targetId });
    say(
      `# T-704 UI frame-time sweep ${new Date().toISOString()} (${SECONDS} s per case, Chromium headless, uncapped rAF; ` +
        `load average ${loadavg().map((l) => l.toFixed(1)).join(" ")} — frame times rise with background load)`,
    );
    say(`# GL renderer: ${glRenderer}`);
    for (const renderer of RENDERERS) for (const c of CASES) {
      const { targetId } = await send("Target.createTarget", { url: "about:blank" });
      const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
      await send("Page.enable", {}, sessionId);
      await send("Runtime.enable", {}, sessionId);
      await send("Emulation.setDeviceMetricsOverride", { width: c.width, height: c.height, deviceScaleFactor: 1, mobile: false }, sessionId);
      await send("Page.navigate", { url: `${base}?preview&scene=${c.scene}&doc=60min&renderer=${renderer}` }, sessionId);
      await waitFor(
        () => evaluate(`!!document.querySelector('[data-testid="waveform-canvas"]') && (${c.scene !== "spectral"} || !!document.querySelector('[data-testid="spectral-view"]'))`, sessionId),
        30_000,
        `${c.scene} scene`,
      );
      await sleep(2_000);
      await send("Performance.enable", {}, sessionId);
      const taskSeconds = async () =>
        (await send("Performance.getMetrics", {}, sessionId)).metrics.find((m) => m.name === "TaskDuration")?.value ?? 0;
      const idleBefore = await taskSeconds();
      const idleFrames = await evaluate(
        `new Promise((resolve) => { let n = 0; const t0 = performance.now(); ` +
          `const f = () => { n += 1; if (performance.now() - t0 < 4000) requestAnimationFrame(f); else resolve(n); }; requestAnimationFrame(f); })`,
        sessionId,
      );
      const idleBusyMs = ((await taskSeconds()) - idleBefore) * 1000;
      const msPerFrame = idleBusyMs / Math.max(1, idleFrames);
      await evaluate(`${SWEEP}(3)`, sessionId);
      // Which renderer actually drew (a canvas holding a WebGL2 context has no 2D context).
      const used = await evaluate(
        `document.querySelector('[data-testid="waveform-canvas"]').getContext("2d") ? "canvas2d" : "webgl2"`,
        sessionId,
      );
      if (PROFILE) {
        await send("Profiler.enable", {}, sessionId);
        await send("Profiler.setSamplingInterval", { interval: 200 }, sessionId);
        await send("Profiler.start", {}, sessionId);
      }
      const run = await evaluate(`${SWEEP}(${SECONDS})`, sessionId);
      if (PROFILE) {
        const { profile } = await send("Profiler.stop", {}, sessionId);
        const self = new Map();
        const totalHits = profile.nodes.reduce((a, n) => a + (n.hitCount ?? 0), 0) || 1;
        for (const n of profile.nodes) {
          const f = n.callFrame;
          const key = `${f.functionName || "(anonymous)"} ${f.url.split("/").pop()}:${f.lineNumber + 1}`;
          self.set(key, (self.get(key) ?? 0) + (n.hitCount ?? 0));
        }
        const top = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 18);
        say(`# profile ${c.scene} ${renderer} ${c.width}x${c.height} (self time share):`);
        for (const [key, hits] of top) say(`#   ${((100 * hits) / totalHits).toFixed(1).padStart(5)} %  ${key}`);
      }
      await send("Target.closeTarget", { targetId });

      const sorted = [...run.deltas].sort((a, b) => a - b);
      const total = run.deltas.reduce((a, b) => a + b, 0);
      const tag = `frame_${c.scene}_${renderer}_${c.width}x${c.height}`;
      const p50 = percentile(sorted, 0.5);
      const p95 = percentile(sorted, 0.95);
      const p99 = percentile(sorted, 0.99);
      const max = sorted[sorted.length - 1] ?? 0;
      const over50 = sorted.filter((d) => d > LONG_FRAME_MS).length;
      say(
        `# ${c.scene} ${c.width}x${c.height}, renderer setting ${renderer} (drew with ${used}) (canvas ${Math.round(run.canvasWidth)}x${Math.round(run.canvasHeight)}): ` +
          `${sorted.length} frames, ${(sorted.length / (total / 1000)).toFixed(0)} fps, p50 ${p50.toFixed(2)} ms, ` +
          `p95 ${p95.toFixed(2)} ms, p99 ${p99.toFixed(2)} ms, max ${max.toFixed(1)} ms, >50 ms: ${over50}, ` +
          `console errors: ${consoleErrors.get(sessionId) ?? 0}`,
      );
      for (const text of consoleTexts.get(sessionId) ?? []) say(`#   console.error: ${text}`);
      result(`${tag}_p50_ms`, p50, "ms", FRAME_BUDGET_MS, "le");
      result(`${tag}_p95_ms`, p95, "ms", FRAME_BUDGET_MS, "le");
      result(`${tag}_p99_ms`, p99, "ms", LONG_FRAME_MS, "le");
      result(`${tag}_max_ms`, max, "ms");
      result(`${tag}_frames_over_50ms`, over50, "frames", 1, "le");
      result(`${tag}_fps`, sorted.length / (total / 1000), "fps");
      const idleTag = `idle_${c.scene}_${renderer}_${c.width}x${c.height}`;
      say(
        `# idle ${c.scene} ${renderer} ${c.width}x${c.height}: ${idleFrames} rAF frames in 4 s uncapped, main thread ` +
          `${((idleBusyMs / 4000) * 100).toFixed(0)} % busy, ${msPerFrame.toFixed(2)} ms per frame → ` +
          `${(msPerFrame * 6).toFixed(1)} % of a core at 60 Hz`,
      );
      result(`${idleTag}_main_thread_ms_per_frame`, msPerFrame, "ms");
      result(`${idleTag}_pct_core_at_60hz`, msPerFrame * 6, "pct_core");
    }
    ws.close();
  } finally {
    killAll();
    await sleep(300);
    rmSync(profile, { recursive: true, force: true });
    writeFileSync(OUT, lines.join("\n") + "\n");
  }
}

main().catch((err) => {
  console.error(err);
  killAll();
  process.exit(1);
});
