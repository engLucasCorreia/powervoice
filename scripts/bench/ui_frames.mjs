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
 * Idle pass (H-43): a second, *vsync-paced* (60 Hz, like a real display) headless Chromium —
 * never the uncapped one, where any perpetual loop turns into a busy loop — opens the preview App
 * (renderer `auto`) in four scenes (`empty`, `document`, `document_rack`, `spectral`), lets each
 * settle 4 s, then measures `--idle-seconds` (10) s of doing nothing: the main thread's busy share
 * of one core (CDP `Performance.getMetrics` `TaskDuration` / wall time) and the animation frames
 * the page ran per second (a `requestAnimationFrame` counter installed before the app loads). The
 * scenes with a document then press Play for 3 s: frames per second while playing (the display
 * rate, 60 fps). Targets: idle ≤ 10 % of a core in the dev build (Vite, debug JS) and ≤ 2 % in the
 * release build (`vite build` with `VITE_PV_BENCH_PREVIEW=1`, which keeps the mocked-IPC preview
 * in a production bundle; served by `vite preview` on port+1); playback ≥ 55 fps.
 *
 * Caveat: the owner's reference renderer is WebKitGTK (ADR-009); Chromium stands in for it
 * headlessly. The preview's Settings pick the Canvas2D waveform renderer.
 *
 * Capture-stall pass (H-68, SPEC-002 AC-8): given the capture-writer stalled by fault injection
 * for 12 s during recording, "the UI stays responsive, with no frame over 100 ms." Reuses the
 * H-43 vsync-paced browser (a real display's cadence — not the uncapped sweep, where the point is
 * raw per-frame cost) and `?preview&scene=recording`, which already reproduces the real
 * main-thread load a stalled capture puts on the UI while its writer is behind: H-43's 60 Hz
 * telemetry stream (meters) plus H-07's ~10 Hz `record_peaks_get` poll feeding the live, growing
 * waveform. It records every rAF delta for `--capture-stall-seconds` (12) once the recording
 * scene is up, and reports p50/p95/max/frames-over-100ms. Targets: p50/p95/max ≤ 100 ms, 0 frames
 * over 100 ms.
 *
 * Usage (`just bench-ui`): node scripts/bench/ui_frames.mjs [--port 5193] [--seconds 10]
 *   [--out target/bench/ui.log] [--case document:2126x850]... [--profile] [--renderer canvas2d]
 *   [--idle-only] [--no-idle] [--no-release] [--idle-seconds 10] [--no-capture-stall]
 *   [--capture-stall-seconds 12]. `--idle-only` skips the frame-time sweep, `--no-idle` the
 *   idle/playback passes (H-47 used it for quick renderer iterations), `--no-release` the release
 *   build's idle pass, and `--no-capture-stall` the H-68 capture-stall pass.
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
const IDLE_ONLY = args.includes("--idle-only");
const NO_IDLE = args.includes("--no-idle");
const NO_RELEASE = args.includes("--no-release");
const IDLE_SECONDS = Number(opt("idle-seconds", "10"));
const NO_CAPTURE_STALL = args.includes("--no-capture-stall");
/** H-68 (SPEC-002 AC-8): the fault-injected writer stall's duration and frame budget. */
const CAPTURE_STALL_SECONDS = Number(opt("capture-stall-seconds", "12"));
const CAPTURE_STALL_FRAME_BUDGET_MS = 100;
const IDLE_SCENES = [
  { name: "empty", query: "" },
  { name: "document", query: "scene=document" },
  { name: "document_rack", query: "scene=document,rack" },
  { name: "spectral", query: "scene=spectral" },
];
/** H-43 targets: % of one core while idle (debug / release JS), frames per second while playing. */
const IDLE_TARGET_PCT = { dev: 10, release: 2 };
const PLAYBACK_MIN_FPS = 55;
const PLAYBACK_SECONDS = 3;
/** Counts the page's animation frames from before the app loads (H-43 idle pass). */
const RAF_COUNTER = `(() => {
  window.__pvRafCount = 0;
  const raf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = (cb) => raf((t) => { window.__pvRafCount += 1; cb(t); });
})()`;

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

// H-68: records rAF deltas for `seconds` without driving any input — the recording scene's own
// 60 Hz telemetry stream and ~10 Hz live-peaks poll (H-07) are the only thing moving, exactly
// like a real stalled capture where nobody is touching the mouse or keyboard.
const RECORD_SWEEP = `(async (seconds) => {
  const durationMs = seconds * 1000;
  const deltas = [];
  return await new Promise((resolve) => {
    let start = -1;
    let last = -1;
    const step = (now) => {
      if (start < 0) {
        start = now;
        last = now;
        requestAnimationFrame(step);
        return;
      }
      deltas.push(now - last);
      last = now;
      if (now - start >= durationMs) {
        resolve({ deltas });
        return;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
})`;

/** Runs `cmd` to completion (inheriting stdio), rejecting on a non-zero exit. */
function run(cmd, cmdArgs, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, cmdArgs, { stdio: "inherit", ...options });
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} ${cmdArgs.join(" ")} exited ${code}`))));
    child.on("error", reject);
  });
}

/** A headless Chromium with `flags`, driven over CDP. */
async function openBrowser(flags) {
  const profile = mkdtempSync(join(tmpdir(), "pv-ui-frames-"));
  const chrome = spawn(
    "chromium",
    [
      "--headless=new",
      "--remote-debugging-port=0",
      "--no-first-run",
      "--no-default-browser-check",
      "--hide-scrollbars",
      "--disable-background-timer-throttling",
      "--disable-renderer-backgrounding",
      ...GPU_FLAGS,
      ...flags,
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { detached: true, stdio: "ignore" },
  );
  children.push(chrome);
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
  const close = async () => {
    ws.close();
    try {
      process.kill(-chrome.pid, "SIGTERM");
    } catch {
      /* already gone */
    }
    await sleep(300);
    rmSync(profile, { recursive: true, force: true });
  };
  return { send, evaluate, consoleErrors, consoleTexts, close };
}

/** T-704: the uncapped zoom/scroll frame-time sweep. */
async function sweepPass(base) {
  const { send, evaluate, consoleErrors, consoleTexts, close } = await openBrowser([
    "--disable-frame-rate-limit",
    "--disable-gpu-vsync",
  ]);
  try {
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
      const runResult = await evaluate(`${SWEEP}(${SECONDS})`, sessionId);
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

      const sorted = [...runResult.deltas].sort((a, b) => a - b);
      const total = runResult.deltas.reduce((a, b) => a + b, 0);
      const tag = `frame_${c.scene}_${renderer}_${c.width}x${c.height}`;
      const p50 = percentile(sorted, 0.5);
      const p95 = percentile(sorted, 0.95);
      const p99 = percentile(sorted, 0.99);
      const max = sorted[sorted.length - 1] ?? 0;
      const over50 = sorted.filter((d) => d > LONG_FRAME_MS).length;
      say(
        `# ${c.scene} ${c.width}x${c.height}, renderer setting ${renderer} (drew with ${used}) (canvas ${Math.round(runResult.canvasWidth)}x${Math.round(runResult.canvasHeight)}): ` +
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
    }
  } finally {
    await close();
  }
}

/** H-43: idle main-thread CPU and playback frame rate, vsync-paced. */
async function idlePass(base, build) {
  const { send, evaluate, consoleErrors, consoleTexts, close } = await openBrowser(["--window-size=1600,900"]);
  try {
    say(
      `# H-43 idle pass (${build}) ${new Date().toISOString()}: ${IDLE_SECONDS} s idle per scene, vsync-paced 60 Hz ` +
        `headless Chromium, 1600x900, renderer auto; load average ${loadavg().map((l) => l.toFixed(1)).join(" ")}`,
    );
    for (const scene of IDLE_SCENES) {
      const { targetId } = await send("Target.createTarget", { url: "about:blank" });
      const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
      await send("Page.enable", {}, sessionId);
      await send("Runtime.enable", {}, sessionId);
      await send("Page.addScriptToEvaluateOnNewDocument", { source: RAF_COUNTER }, sessionId);
      await send("Page.navigate", { url: `${base}?preview&renderer=auto${scene.query ? `&${scene.query}` : ""}` }, sessionId);
      await waitFor(() => evaluate(`!!document.querySelector('[data-testid="transport-play"]')`, sessionId), 30_000, `${scene.name} scene`);
      await sleep(4_000);
      await send("Performance.enable", {}, sessionId);
      const taskSeconds = async () =>
        (await send("Performance.getMetrics", {}, sessionId)).metrics.find((m) => m.name === "TaskDuration")?.value ?? 0;
      const frames = () => evaluate("window.__pvRafCount", sessionId);
      const measure = async (seconds) => {
        const t0 = await taskSeconds();
        const f0 = await frames();
        const w0 = Date.now();
        await sleep(seconds * 1000);
        const t1 = await taskSeconds();
        const f1 = await frames();
        const wall = (Date.now() - w0) / 1000;
        return { pct: ((t1 - t0) / wall) * 100, fps: (f1 - f0) / wall };
      };
      const idle = await measure(IDLE_SECONDS);
      say(
        `# idle ${build} ${scene.name}: main thread ${idle.pct.toFixed(2)} % busy, ${idle.fps.toFixed(1)} animation frames/s`,
      );
      result(`idle_${build}_${scene.name}_main_thread_pct`, Number(idle.pct.toFixed(3)), "pct_core", IDLE_TARGET_PCT[build], "le");
      result(`idle_${build}_${scene.name}_raf_per_s`, Number(idle.fps.toFixed(2)), "per_s");
      if (scene.name !== "empty") {
        await evaluate(`document.querySelector('[data-testid="transport-play"]').click()`, sessionId);
        await sleep(1_000);
        const playing = await measure(PLAYBACK_SECONDS);
        await evaluate(`document.querySelector('[data-testid="transport-stop"]')?.click()`, sessionId);
        say(
          `# playback ${build} ${scene.name}: ${playing.fps.toFixed(1)} animation frames/s, main thread ${playing.pct.toFixed(1)} % busy`,
        );
        result(`playback_${build}_${scene.name}_fps`, Number(playing.fps.toFixed(2)), "fps", PLAYBACK_MIN_FPS, "ge");
        result(`playback_${build}_${scene.name}_main_thread_pct`, Number(playing.pct.toFixed(2)), "pct_core");
      }
      for (const text of consoleTexts.get(sessionId) ?? []) say(`#   console.error: ${text}`);
      if ((consoleErrors.get(sessionId) ?? 0) > 0) say(`#   console errors: ${consoleErrors.get(sessionId)}`);
      await send("Target.closeTarget", { targetId });
    }
  } finally {
    await close();
  }
}

/** H-68 (SPEC-002 AC-8): while a fault-injected capture stall lasts 12 s during recording, no UI
 * frame may exceed 100 ms. Vsync-paced like the H-43 idle pass (a real display's cadence), on
 * `?preview&scene=recording`, which already streams telemetry at 60 Hz and polls
 * `record_peaks_get` at ~10 Hz for the growing take (H-07) — the same main-thread load a real
 * stalled capture puts on the UI. */
async function captureStallPass(base) {
  const { send, evaluate, consoleErrors, consoleTexts, close } = await openBrowser(["--window-size=1600,900"]);
  try {
    const { targetId } = await send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
    await send("Page.enable", {}, sessionId);
    await send("Runtime.enable", {}, sessionId);
    await send("Page.navigate", { url: `${base}?preview&scene=recording&renderer=auto` }, sessionId);
    await waitFor(
      () =>
        evaluate(
          `document.querySelector('[data-testid="record-elapsed"]')?.classList.contains("live") === true && ` +
            `!!document.querySelector('[data-testid="waveform-canvas"]')`,
          sessionId,
        ),
      30_000,
      "recording scene",
    );
    // Let the take grow past its first buckets before measuring, like the other passes' warm-up.
    await sleep(2_000);
    say(
      `# H-68 capture-stall pass ${new Date().toISOString()}: ${CAPTURE_STALL_SECONDS} s recording, ` +
        `vsync-paced 60 Hz headless Chromium, 1600x900, renderer auto; load average ${loadavg().map((l) => l.toFixed(1)).join(" ")}`,
    );
    const runResult = await evaluate(`${RECORD_SWEEP}(${CAPTURE_STALL_SECONDS})`, sessionId);
    await send("Target.closeTarget", { targetId });

    const sorted = [...runResult.deltas].sort((a, b) => a - b);
    const total = runResult.deltas.reduce((a, b) => a + b, 0);
    const p50 = percentile(sorted, 0.5);
    const p95 = percentile(sorted, 0.95);
    const max = sorted[sorted.length - 1] ?? 0;
    const over100 = sorted.filter((d) => d > CAPTURE_STALL_FRAME_BUDGET_MS).length;
    say(
      `# recording, capture stall (${CAPTURE_STALL_SECONDS} s): ${sorted.length} frames, ` +
        `${(sorted.length / (total / 1000)).toFixed(1)} fps, p50 ${p50.toFixed(2)} ms, p95 ${p95.toFixed(2)} ms, ` +
        `max ${max.toFixed(1)} ms, >100 ms: ${over100}, console errors: ${consoleErrors.get(sessionId) ?? 0}`,
    );
    for (const text of consoleTexts.get(sessionId) ?? []) say(`#   console.error: ${text}`);
    result("capture_stall_recording_p50_ms", p50, "ms", CAPTURE_STALL_FRAME_BUDGET_MS, "le");
    result("capture_stall_recording_p95_ms", p95, "ms", CAPTURE_STALL_FRAME_BUDGET_MS, "le");
    result("capture_stall_recording_max_ms", max, "ms", CAPTURE_STALL_FRAME_BUDGET_MS, "le");
    result("capture_stall_recording_frames_over_100ms", over100, "frames", 0, "le");
    result("capture_stall_recording_fps", sorted.length / (total / 1000), "fps");
  } finally {
    await close();
  }
}

async function main() {
  mkdirSync(dirname(OUT), { recursive: true });
  const vite = spawn("npm", ["--prefix", join(ROOT, "ui"), "run", "dev", "--", "--port", String(PORT), "--strictPort"], {
    detached: true,
    stdio: "ignore",
  });
  children.push(vite);
  const base = `http://localhost:${PORT}/`;
  try {
    await waitFor(async () => (await fetch(base)).ok, 60_000, `Vite on ${base}`);
    if (!IDLE_ONLY) {
      await sweepPass(base);
    }
    if (!NO_IDLE) {
      await idlePass(base, "dev");
    }
    if (!NO_CAPTURE_STALL) {
      await captureStallPass(base);
    }
    if (!NO_RELEASE && !NO_IDLE) {
      const outDir = join(ROOT, "target", "bench", "ui-dist");
      await run("npx", ["vite", "build", "--outDir", outDir, "--emptyOutDir", "--logLevel", "warn"], {
        cwd: join(ROOT, "ui"),
        env: { ...process.env, VITE_PV_BENCH_PREVIEW: "1" },
      });
      const previewPort = PORT + 1;
      const preview = spawn(
        "npx",
        ["vite", "preview", "--outDir", outDir, "--port", String(previewPort), "--strictPort", "--host", "localhost"],
        { cwd: join(ROOT, "ui"), detached: true, stdio: "ignore" },
      );
      children.push(preview);
      const releaseBase = `http://localhost:${previewPort}/`;
      await waitFor(async () => (await fetch(releaseBase)).ok, 60_000, `vite preview on ${releaseBase}`);
      await idlePass(releaseBase, "release");
    }
  } finally {
    killAll();
    await sleep(300);
    writeFileSync(OUT, lines.join("\n") + "\n");
  }
}

main().catch((err) => {
  console.error(err);
  killAll();
  process.exit(1);
});
