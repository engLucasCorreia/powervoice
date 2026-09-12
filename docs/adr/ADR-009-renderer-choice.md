# ADR-009 — Renderer choice & Linux workarounds
- Status: accepted (owner, M0 checkpoint 2026-09-12)
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
PROMPT §3.2 needs a waveform view (min/max peak pyramid, smooth zoom) and a spectral view (STFT
spectrogram, colormap), both scrolling/zooming at the §4 target of 60 fps. §4 also says spectrograms
are computed in Rust as `u8` tiles and colored/drawn on the GPU in the UI. ADR-003 fixes the IPC
shapes (`Channel`/`ipc::Response`, binary, LE) but left two things unverified on the owner's actual
machine (Arch, Hyprland/Wayland, AMD Phoenix iGPU/Mesa, WebKitGTK 4.1, Tauri 2.11.5): whether raw
`Channel`/`ipc::Response` payloads really arrive in JS as `ArrayBuffer`, and the cost of a 30 Hz vs
60 Hz telemetry `Channel` (MEMORY.md follow-ups). MEMORY.md's risk list also flags "WebKitGTK
rendering performance" with `WEBKIT_DISABLE_DMABUF_RENDERER=1` as a candidate workaround, previously
only linked to NVIDIA.

T-007 builds a dev-only spike (`--features spike` on `powervoice-app`, `ui/src/spike/`, `just spike`)
that generates synthetic data in Rust matching these shapes, renders it with both WebGL2 and
Canvas2D, and measures frame times, IPC throughput and telemetry cost automatically
(`POWERVOICE_SPIKE=1`, `POWERVOICE_SPIKE_EXIT=1`), on the owner's real hardware, with and without
`WEBKIT_DISABLE_DMABUF_RENDERER=1`. This ADR records the results and the decision.

## Decision

### 1. Payload types (ADR-003 follow-up (a)) — confirmed
Both `ipc::Response` (as returned directly from `spike_spectrogram_texture` /
`spike_ipc_response_10mb`) and a raw `Channel` message (`spike_waveform_peaks` /
`spike_ipc_channel_10mb` / `spike_telemetry_run`) arrive in JS as **`ArrayBuffer`**, on every one of
the 4 automated runs (`payloadType`/`channelPayloadType`/`responsePayloadType` fields in the
appendix are all `"ArrayBuffer"`, checked with `instanceof ArrayBuffer` in
`ui/src/spike/binary.ts`'s `describePayloadType`). This matches the Tauri IPC source: a raw
`InvokeResponseBody::Raw` payload either gets inlined as `new Uint8Array(...).buffer` for small
messages or fetched via `response.arrayBuffer()` for larger ones
(`tauri-2.11.5/src/ipc/channel.rs`, `scripts/ipc-protocol.js`) — never a JSON number array. `ui/src/lib/ipc/binary.ts` (T-203/T-204/T-108) can decode headers with `DataView` and view
payloads with typed arrays with no copy, as ADR-003 assumes.

### 2. Renderer per view

| View | Renderer | Fallback | Rationale |
|---|---|---|---|
| Waveform | **WebGL2** | Canvas2D | Both hit the same frame times here (§4 below) — the bottleneck on this machine is the compositor path, not the draw API — but WebGL2 is the architecturally correct choice at production scale: the vertex buffer approach amortizes the reduce-to-columns cost across a `bufferSubData` instead of Canvas2D's per-frame path fill, and it composes with the spectrogram's shader-based colormap infrastructure. Canvas2D is kept as an automatic fallback (see §4) for a WebGL2-less environment. |
| Spectrogram | **WebGL2** | Canvas2D | Same measured frame times as the waveform case, but the *mechanism* differs meaningfully: WebGL2 uploads only the new strip per hop (`texSubImage2D`) and recolors via a fragment-shader colormap that never needs a refetch when the floor/ceiling/colormap changes (matches ADR-003 §2's spectrogram design note). The Canvas2D fallback must manually `copyWithin`-scroll and re-`putImageData` the *whole* frame every update — measurably more CPU work per frame at production tile sizes/update rates than this spike's 1024×512 synthetic texture exercises, so WebGL2's advantage will widen, not narrow, under real STFT tile traffic. |

Both views target 60 fps (PROMPT §4). See §3: that target is met, with real headroom, once
`WEBKIT_DISABLE_DMABUF_RENDERER=1` is set.

### 3. Measurements

All runs: synthetic 60-minute 48 kHz mono waveform peaks (337 500 buckets, spp 512, ≈2.7 MB,
delivered over a `Channel`), a synthetic 1024×512 u8 spectrogram texture (≈512 KB, delivered via
`ipc::Response`), 10 s zoom/scroll sweeps, 10 MB IPC throughput both ways, and 30 Hz/60 Hz telemetry
(72-byte `VXTM`-shaped frames) for 5 s each. Two full runs per configuration (`just spike`, i.e.
`npm run tauri dev --features spike`, `POWERVOICE_SPIKE=1 POWERVOICE_SPIKE_EXIT=1`), window mapped and on
the active workspace throughout but **not focused** (launched non-interactively) — recorded per run
via `document.visibilityState`/`document.hasFocus()`; every run below had
`visibilityState: "visible"` start-to-end and was **not** flagged `likelyThrottled` (see §5). Full
JSON is in `bench-results/` (gitignored); numbers only, below.

**Frame times (ms), 10 s sweep each — default (no env var) vs `WEBKIT_DISABLE_DMABUF_RENDERER=1`**

| View / renderer | Config | Run | frames | p50 | p95 | p99 | max | dropped(>25ms) |
|---|---|---|---:|---:|---:|---:|---:|---:|
| Waveform WebGL2 | default | 1 | 602 | 17 | 18 | 18 | 20 | 0 |
| Waveform WebGL2 | default | 2 | 602 | 17 | 18 | 18 | 19 | 0 |
| Waveform WebGL2 | dmabuf-off | 1 | 972 | 10 | 11 | 11 | 87 | 2 |
| Waveform WebGL2 | dmabuf-off | 2 | 978 | 10 | 11 | 11 | 29 | 2 |
| Waveform Canvas2D | default | 1 | 601 | 17 | 18 | 19 | 21 | 0 |
| Waveform Canvas2D | default | 2 | 601 | 17 | 18 | 19 | 20 | 0 |
| Waveform Canvas2D | dmabuf-off | 1 | 974 | 10 | 11 | 11 | 64 | 2 |
| Waveform Canvas2D | dmabuf-off | 2 | 979 | 10 | 11 | 11 | 28 | 1 |
| Spectrogram WebGL2 | default | 1 | 602 | 17 | 18 | 19 | 20 | 0 |
| Spectrogram WebGL2 | default | 2 | 601 | 17 | 18 | 19 | 22 | 0 |
| Spectrogram WebGL2 | dmabuf-off | 1 | 980 | 10 | 11 | 11 | 29 | 1 |
| Spectrogram WebGL2 | dmabuf-off | 2 | 979 | 10 | 11 | 11 | 31 | 1 |
| Spectrogram Canvas2D | default | 1 | 599 | 17 | 18 | 19 | 52 | 1 |
| Spectrogram Canvas2D | default | 2 | 601 | 17 | 18 | 19 | 20 | 0 |
| Spectrogram Canvas2D | dmabuf-off | 1 | 979 | 10 | 11 | 11 | 28 | 1 |
| Spectrogram Canvas2D | dmabuf-off | 2 | 979 | 10 | 11 | 11 | 28 | 1 |

p50 17 ms ≈ **59 fps** (default) vs p50 10 ms ≈ **100 fps** (dmabuf-off) — both meet the PROMPT §4
60 fps target, but dmabuf-off does so with roughly double the headroom, and the ~600→~975 frame
count over the same 10 s window is the same story from the other side. **WebGL2 and Canvas2D are
statistically indistinguishable in every row** — on this AMD Phoenix/Mesa/WebKitGTK 4.1 stack, the
bottleneck at this content size is the compositor/presentation path, not the draw API.

**IPC throughput, 10 MB each way (ms, MB/s)**

| Config | Run | Response ms | Response MB/s | Channel ms | Channel MB/s |
|---|---|---:|---:|---:|---:|
| default | 1 | 124 | 80.6 | 123 | 81.3 |
| default | 2 | 116 | 86.2 | 117 | 85.5 |
| dmabuf-off | 1 | 113 | 88.5 | 114 | 87.7 |
| dmabuf-off | 2 | 114 | 87.7 | 113 | 88.5 |

`Response` and `Channel` are within noise of each other for a 10 MB one-shot payload; no reason to
prefer one over the other on throughput grounds. ADR-003's per-mechanism choice (§1: `Response` for
on-demand peaks/samples, `Channel` for streamed/ordered data) stands on its other merits (ordering,
one connection for many messages), not raw speed.

**Telemetry: 72-byte `VXTM`-shaped frame over a `Channel`, 30 Hz vs 60 Hz, 5 s each (ADR-003
follow-up (b))**

| Config | Run | Hz | frames received / expected | avg JS handler time/frame (µs) | rAF frames observed / expected (baseline) | rAF dropped |
|---|---|---:|---|---:|---|---:|
| default | 1 | 30 | 150 / 150 | 20.0 | 514 / 520 | 6 |
| default | 1 | 60 | 299 / 300 | 0.0 | 515 / 520 | 5 |
| default | 2 | 30 | 149 / 150 | 26.8 | 514 / 520 | 6 |
| default | 2 | 60 | 295 / 300 | 10.2 | 515 / 520 | 5 |
| dmabuf-off | 1 | 30 | 149 / 150 | 13.4 | 514 / 520 | 6 |
| dmabuf-off | 1 | 60 | 294 / 300 | 3.4 | 513 / 520 | 7 |
| dmabuf-off | 2 | 30 | 149 / 150 | 13.4 | 514 / 519 | 5 |
| dmabuf-off | 2 | 60 | 294 / 300 | 6.8 | 514 / 519 | 5 |

Frame delivery is ≥98% at both rates in every run. Per-message JS handling (decode only, a stand-in
for a real handler) is a few *microseconds* — three orders of magnitude below a 16.7 ms frame
budget. rAF drops during the telemetry window (5-7 of ~515-520, i.e. ~1%) are essentially identical
at 30 Hz and 60 Hz, so they are baseline jitter, not a cost of the higher rate. **60 Hz telemetry is
free on this hardware.** Per ADR-003's open question, the default should move to **60 Hz**, with a
settings toggle back to 30 Hz kept only for headroom on lower-end machines (untested here).

### 4. Detection / fallback strategy for production code
- Feature-detect, don't sniff the platform: at startup, try `canvas.getContext("webgl2")`. If it
  returns `null` (or throws), use the Canvas2D renderer for both views. Log a `notice` (ADR-003
  event) so the owner can see which path is active.
- If a WebGL2 context is lost mid-session (`webglcontextlost`), fall back to Canvas2D for the
  remainder of that view's lifetime rather than attempting to recreate the context repeatedly.
- **`WEBKIT_DISABLE_DMABUF_RENDERER=1`**: MEMORY.md previously linked this workaround only to
  NVIDIA. This spike shows a clear, repeatable ~1.7x frame-time improvement on **AMD Phoenix/Mesa**
  too (§3), so the gotcha is broader than one vendor. `just dev` already exposes
  `POWERVOICE_WEBKIT_SAFE=1` for this; recommend making it the **documented default** for Linux
  builds/packaging (an env var set by the launcher script/`.desktop` file, not a code change), with
  an escape hatch to unset it if a future driver update changes the balance. This is an environment
  workaround, not a renderer-choice fork: it affects both WebGL2 and Canvas2D equally (§3), so it
  does not change §2's decision.

### 5. Wayland/Hyprland rAF caveat
Every run above was confirmed **not** throttled: `document.visibilityState` was `"visible"` for the
whole run and the rAF-driven sweep delivered close to its expected frame count (never flagged
`likelyThrottled` by `ui/src/spike/frameBench.ts`, which checks exactly this rather than trust the
numbers blindly). Note: the window was mapped and on its monitor's *active* workspace throughout
(`hyprctl clients`/`monitors`) but **not focused** (`document.hasFocus()` was `false` in every run,
since the spike was launched non-interactively) — and that made no measurable difference to frame
delivery, i.e. Hyprland does not throttle rAF for a visible-but-unfocused window on this setup.

That said, during development one run of the automated suite hung indefinitely (>15 minutes, zero
output, the window mapped/visible/unfocused on its active workspace) before this behavior was
reproduced reliably; the cause was not conclusively identified (candidates: a first-run WebKitGTK/
JIT warm-up hiccup, a transient compositor issue — genuine rAF starvation was not observed in any
of the 4 recorded runs). Since a hang either way would silently block `POWERVOICE_SPIKE_EXIT=1` forever
in automation, `ui/src/spike/frameBench.ts`/`telemetry.ts`/`results.ts` now race every rAF- and
IPC-driven step against a plain `setTimeout` watchdog independent of rAF, and `results.ts` catches
each step's failure independently so the suite always writes a JSON (flagging `incomplete`/
`timedOut` per step) and exits rather than hanging. **Manual checklist item for the owner:** run
`just spike` interactively (focused, visible window) at least once and compare — if numbers differ
meaningfully from this appendix, rAF throttling-when-unfocused is real on your session and should be
called out before M2.

### 6. Manual input checklist (owner, at the M0 checkpoint)
Run `just spike` (window stays open without `POWERVOICE_SPIKE_EXIT`) and, with the window focused:
1. Press **Space** — confirm a `keydown Space` / `keyup Space` line appears in the input log.
2. Press **Shift+Space** — confirm it logs as `Shift+Space` (not just `Space`, i.e. the modifier is
   captured).
3. Press **Ctrl+Z** — confirm it logs as `Ctrl+KeyZ`.
4. Press **Ctrl+Shift+Z** — confirm it logs as `Ctrl+Shift+KeyZ` (both modifiers captured
   together).
5. Click-drag inside the "Drag here" box — confirm `pointerdown`/`pointermove`/`pointerup` lines
   show both page-relative and element-local coordinates, and that `local` stays within
   `[0, box size]` while the pointer is inside the box (pointer capture is enabled, so drag
   coordinates keep updating even if the pointer briefly leaves the box).
6. Note anything that looks wrong (wrong key codes, missing modifiers, coordinates that jump/don't
   track the cursor) — Wayland/Hyprland-specific pointer quirks were flagged as a risk in MEMORY.md
   and this is the check for them.

## Consequences
**Positive**
- WebGL2 is confirmed viable and fast enough on the owner's actual hardware for both views, with a
  working Canvas2D fallback for a WebGL2-less environment.
- `Channel`/`ipc::Response` raw payloads are confirmed to arrive as `ArrayBuffer`, unblocking
  `ui/src/lib/ipc/binary.ts` (T-203/T-204/T-108) to decode with `DataView`/typed arrays as ADR-003
  assumes, with no JSON detour.
- 60 Hz telemetry is confirmed cheap; ADR-003's open question is resolved in favor of 60 Hz default.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` is confirmed to help broadly (not just NVIDIA) on this Linux
  stack, with hard numbers to justify making it the packaged default.
- The automated harness now degrades gracefully (bounded watchdogs, `incomplete`/`timedOut` flags)
  instead of hanging, which will make it reusable for later spikes.

**Negative**
- The spike's peak/spectrogram data is synthetic and reduces at a single spp level (§3 in
  `ui/src/spike/reduce.ts`), so the "whole file → ~1 sample/px" zoom sweep is approximated by
  varying buckets-per-column rather than true multi-resolution LOD switching; production's actual
  LOD-switch cost (ADR-003 "UI choice") is not measured here.
- The Canvas2D spectrogram fallback's true relative cost is understated: this spike's 1024×512
  texture with a 4-column-per-frame update is smaller/slower-updating than some production tiles
  will be, and §2 argues (but does not measure) that this widens WebGL2's advantage.
- Only one machine (owner's) was measured; other GPUs/drivers are not covered (MEMORY.md risk list).
- The one unreproduced hang (§5) was mitigated, not root-caused.

**Follow-ups**
- M2 renderer implementation: use the WebGL2/Canvas2D detection strategy in §4; wire the packaged
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` default (T-705 packaging, or earlier if `just dev`/`just build`
  gain a Linux launch wrapper before then).
- ADR-003: telemetry default becomes 60 Hz (§3), with a settings toggle to 30 Hz.
- Owner: complete the manual checklist (§6) at the M0 checkpoint, including the focused-window
  comparison run (§5).
- If the M0 checkpoint surfaces a different GPU/driver to test on, rerun `just spike` there before
  relying on §3's numbers for that machine.

## Alternatives considered
- **Canvas2D as the primary renderer**: rejected. Frame times are statistically identical to WebGL2
  here, but Canvas2D's spectrogram path requires a full-frame CPU recolor + `putImageData` per
  update (§2), which will not scale as well to production tile sizes/update rates, and it forecloses
  the shader-based floor/ceiling/colormap-without-refetch design ADR-003 already assumes.
  Kept as the fallback.
- **A custom `register_uri_scheme_protocol` fetch path for peaks** (ADR-003's fallback-in-waiting,
  in case `Response` was slow): not needed — §3 shows `Response` and `Channel` perform equivalently
  for a 10 MB payload.
- **Leaving telemetry at 30 Hz** (ADR-003's cautious default pending this measurement): rejected;
  §3 shows 60 Hz costs the same in dropped frames and microseconds of handler time.
- **Trusting `document.hasFocus()`/an assumed 60 Hz display as the sole throttling signal**: the
  spike initially flagged every run as `likelyThrottled` purely because the window was unfocused,
  even though frame delivery was excellent (baseline rAF ≈99 Hz here) — a misleading label the
  ticket explicitly warned against. `frameBench.ts` now judges throttling from measured behavior
  (a `setTimeout` watchdog firing, zero/very-few frames actually delivered, or a slow median) with
  visibility state and focus recorded as context, not as automatic disqualifiers.

## Open questions
- For the owner: complete the manual input checklist (§6), including a focused-window comparison
  run, and confirm the `WEBKIT_DISABLE_DMABUF_RENDERER=1`-by-default packaging decision (§4).
- For M2: re-measure with production-realistic tile sizes/update rates once the real spectrogram
  pipeline exists, to confirm §2's Canvas2D-widens-the-gap argument rather than assume it.

## Amendment 1 — M0 checkpoint (2026-09-12): DMA-BUF workaround on by default
The owner decided: on Linux, `WEBKIT_DISABLE_DMABUF_RENDERER=1` is **on by default** — set by the app
itself at startup (before the WebView initializes, only if the variable is not already set) and by
`just dev`/`just spike` — with an opt-out: `POWERVOICE_WEBKIT_DMABUF=1` keeps the default WebKit DMA-BUF
renderer. Implemented in T-104.
