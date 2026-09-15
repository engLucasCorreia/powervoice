# SPEC-006 — Waveform view: rendering, zoom/scroll, rulers, playhead, selection

- **Status:** approved (autonomous, T-200)
- **Milestone:** M2 (editor view). Implementing tickets are assigned when the M2 ticket wave is
  written; this spec is written just-in-time per PROMPT §5.
- **Related:** SPEC-000 (architecture overview, glossary), SPEC-003 (transport & playback — playhead
  extrapolation contract, playhead-follow toggle, this spec implements the *view's* consumption of
  both), SPEC-004 (document model & undo — snapshots, `audio_rev`, markers), SPEC-005 (file formats —
  parallel M2 spec; this view is format-agnostic, see §7), SPEC-007 (spectral display & analyzer —
  parallel M2 spec; split/toggle mechanics between waveform and spectral views belong there, see §7) ·
  ADR-003 (IPC data paths — `VXPK` peaks, `VXTM` telemetry, staleness by `audio_rev`) · ADR-004 §5
  (per-chunk peak pyramid) · ADR-009 (renderer choice — WebGL2 primary, Canvas2D fallback, measured
  frame times) · PROMPT §2 (LOCKED), §3.2, §3.6, §4 · MEMORY D-014 (shortcuts), D-018 (Stop semantics)

## 1. Purpose

The waveform view is where a voice-over editor spends nearly all their time: finding a breath to trim,
judging whether a level is hot, checking exactly where a word starts so a cut lands clean. It has to
show accurate peaks at every zoom level from the whole file down to individual samples, scroll and
zoom at a speed that never breaks the editor's sense of "I'm looking at the real audio," and give
pixel-and-sample-exact selections, because every destructive edit in M3 acts on whatever this view
says is selected. This spec is the contract for all of that: what's drawn, from what data, at what
rate, and exactly which sample a selection boundary or ruler label refers to.

## 2. Behavior / UX

### 2.1 Layout

Top to bottom, left to right, inside the editor pane (PROMPT §3.6 app shell — "editor, center"):

```
┌───────────────────────────────────────────────────────────────────┐
│ time ruler (ticks + labels)                                       │
├───┬───────────────────────────────────────────────────────────────┤
│ a │                                                                │
│ m │                    waveform canvas                            │
│ p │           (min/max fill, playhead, selection,                  │
│ l │            marker lines, clip highlight)                       │
│ r │                                                                │
├───┴───────────────────────────────────────────────────────────────┤
│ horizontal scrollbar / overview strip                              │
└───────────────────────────────────────────────────────────────────┘
```

- The **amplitude ruler** is a fixed-width gutter on the left of the waveform canvas.
- The **time ruler** is a fixed-height strip above it, spanning the same horizontal extent as the
  waveform canvas (not the amplitude gutter).
- The **overview strip** below the waveform is a fixed-height, always-whole-file miniature (min/max at
  the coarsest pyramid level, unconditionally), with a draggable "viewport" rectangle showing the
  current horizontal zoom/scroll window. Clicking or dragging inside it moves the main viewport;
  dragging its handles resizes the zoomed window without changing the main selection. It is the same
  role as Audition's zoomed-out overview strip below the waveform.
- Markers (SPEC-004 data model) are drawn as thin vertical lines through the waveform canvas with a
  small flag at the top edge (§2.10); the markers **list panel** (rename, navigate, delete) is M3 —
  out of scope here (§7).

### 2.2 View state

The view holds, per open document:

```ts
interface WaveformViewState {
  startSample: bigint;        // document sample at the left edge of the canvas
  samplesPerPixel: number;    // f64; can be < 1 when zoomed past 1:1 (down to 0.1)
  viewportPx: number;         // canvas CSS width in px
  verticalZoom: number;       // linear amplitude scale factor, see §2.4
  amplitudeRulerMode: 'dbfs' | 'percent';
  timeRulerFormat: 'timecode' | 'samples' | 'seconds';
  selection: { startSample: bigint; endSample: bigint } | null; // endSample exclusive, may be empty
  snapToZeroCrossing: boolean;
}
```

`samplesPerPixel` (spp) is the single source of truth for zoom: it is compared directly against the
VXPK pyramid levels (ADR-003 §2) to pick which level to request, and it is what converts a pixel
coordinate to a document sample and back (§4.1). All positions the view produces or consumes —
selection edges, ruler labels, marker x-positions, the playhead — are **document samples** (SPEC-000
§2.4 "document time"), never pixels, never device time.

### 2.3 Peak rendering

- **Data source.** The view never reads sample data from the source file or format (SPEC-005); it
  only ever reads the session's per-chunk peak pyramid through `peaks_get` → `VXPK` (ADR-003, ADR-004
  §5). This holds regardless of the file's original format, sample rate conversion, or import path.
- **Level choice and reduction.** Exactly ADR-003 §2's rule: the largest pyramid level `spp ∈ {64, 256,
  1024, 4096, 16384, 65536}` with `level ≤ samplesPerPixel` is requested, and because levels step by
  ×4, combining the returned buckets into one pixel column never needs more than **4 buckets per
  pixel**. Below `samplesPerPixel = 64` the view requests `RAW` (flag bit0) instead of buckets.
- **Column fill.** For a level-based column, the pixel is filled from `(min, max)` — vertically from
  `y(min)` to `y(max)` in the current vertical-zoom mapping (§2.4). This is a conservative union
  (ADR-004 §5): it never under-reports amplitude and is exact to ±1 bucket in time, which is ≤ 1 px by
  construction.
- **Raw rendering (samplesPerPixel < 64).** Samples are drawn as a connected line (a polyline through
  consecutive sample values, vertically mapped per §2.4).
  - **Decided (autonomous, T-200):** once the on-screen spacing between consecutive samples reaches
    **≥ 3 px** (i.e. `1 / samplesPerPixel ≥ 3`), a small filled dot is drawn at each sample's exact
    point in addition to the connecting line — enough separation to read individual sample dots
    without them merging into a smear, matching the convention in other sample-accurate editors
    (Audition, Reaper) of showing dots only once samples are visually distinct.
- **Clipped-sample highlight.** Any pixel column (level or raw) whose bucket touches or exceeds full
  scale is marked: if `max ≥ 1.0`, the sliver from `y(1.0)` to `y(max)` (minimum 2 px so it's visible
  even exactly at 1.0) is drawn in the theme's clip-red token instead of the normal waveform fill;
  symmetric for `min ≤ −1.0`. In raw/dot mode, an individual sample with `|x| ≥ 1.0` gets a red dot
  instead of the normal color.
  - **Decided (autonomous, T-200):** the threshold is exactly `|x| ≥ 1.0` (0 dBFS), not a
    near-clipping margin, because our internal format is f32 with headroom above 1.0 for legitimate
    unnormalized float content — flagging only samples that actually reach the digital ceiling is
    the correct "this needs attention before dither/export" signal, matching Audition's clip
    indication.
- **Progressive display.** While a chunk's peaks haven't been computed yet (`peaks_progress` event,
  ADR-003), the corresponding `VXPK` buckets carry `PARTIAL` (bit1) and their `(min, max)` values are
  `(NaN, NaN)`.
  - **Decided (autonomous, T-200):** `NaN` buckets are drawn as a flat mid-tone placeholder column
    (a distinct `--wave-pending` theme token, not silence's color and not the normal waveform color),
    so a partially-loaded 60-min file reads as "still filling in" rather than "silent" or "solid
    audio." A thin progress indicator under the time ruler shows overall completion from
    `peaks_progress`. The view re-requests and redraws the affected range as `peaks_progress` reports
    completion, without the user needing to scroll or zoom to trigger a refresh.
- **Staleness / out-of-order rejection.** Every `VXPK` response is dropped if its `audio_rev` isn't the
  document's current `audio_rev` (ADR-003 rule, applies here directly). In addition:
  - **Decided (autonomous, T-200):** the view also tracks the highest `request_id` it has issued per
    `(level, viewport range)` key and ignores a response whose `request_id` is lower than what's
    already been applied for that key, even when `audio_rev` matches — a rapid zoom/scroll gesture
    fires several requests before the first reply lands, and without this, an older, cheaper reply
    can arrive after a newer one and visibly flicker the view back a frame.

### 2.4 Vertical (amplitude) zoom and ruler

- Vertical zoom is a **linear scale factor** applied to sample/bucket amplitude before mapping to y
  pixels: `y = centerY − amplitude × verticalZoom × halfHeightPx`, independent of the ruler display
  mode below. `verticalZoom` ranges **1× to 256×** (power-of-two steps by keyboard/menu, continuous by
  drag on the ruler gutter), default **1×** (±1.0 spans the full waveform canvas height).
  - **Decided (autonomous, T-200):** the 256× ceiling is chosen so the quietest inspectable full-height
    range is ±1/256 ≈ −48 dBFS — enough to eyeball a noise floor or a quiet room-tone selection
    without an impractically large scale factor; finer forensic inspection is what the spectral view
    (SPEC-007) and the loudness meters are for.
- **Amplitude ruler modes:**
  - **dBFS** (nonlinear): major ticks at a fixed set (0, −6, −12, −18, −24, −36, −48, −60, ... dBFS,
    thinned to avoid label collision at low `verticalZoom`), each placed at `y` for its linear
    amplitude `10^(dB/20)` under the current `verticalZoom`. −∞ is never labeled as a tick; the
    centerline (0 amplitude) is unlabeled (it isn't a dB value).
  - **Percentage** (linear): evenly spaced ticks at ..., −100 %, −50 %, 0 %, 50 %, 100 %, ... of full
    scale, thinned the same way.
  - **Decided (autonomous, T-200): default = dBFS.** PowerVoice's other level-facing surfaces (meters,
    normalize favorites, ACX check, LUFS) are all dB-based (PROMPT §3.5); keeping the waveform ruler in
    the same units avoids a unit conversion in the user's head while judging a level. Percentage is a
    one-click toggle for users who think in it.
- Vertical zoom and ruler mode are per-view UI state (SPEC-004 §2.2 "Selection, zoom, scroll, view
  state, playhead" — none of it is undoable, all of it is out of scope for the document journal).

### 2.5 Time ruler

- **Formats:** `timecode` (`[hh:]mm:ss.fff`, the `hh:` group only shown once the document is ≥ 1 hour),
  `samples` (integer document sample number), `seconds` (decimal seconds, precision increasing with
  zoom — see §4.2).
  - **Decided (autonomous, T-200): default = `timecode`.** It's immediately readable without doing
    arithmetic, unlike raw samples, and it doesn't silently lose precision the way a fixed-decimal
    seconds display would when zoomed to the sample; the format toggle (a settings/view control) is
    for users who want raw sample numbers for scripting/precision work or plain seconds for talking
    with someone about a cue point.
- Ticks are spaced so labels never collide (major/minor tick spacing recomputed on every zoom change
  from a fixed "nice number" ladder per format: 1/2/5×10ⁿ seconds for `seconds`/`timecode`,
  1/2/5×10ⁿ samples for `samples`).
- **Label exactness** (AC-6): every label's underlying value is the exact document sample at that
  tick's pixel position under the current `startSample`/`samplesPerPixel` mapping (§4.1) — never a
  value rounded independently of that mapping.

### 2.6 Horizontal zoom

- **Range.** From "whole file" (`samplesPerPixel = len_samples / viewportPx`, i.e. **zoom full** — no
  padding, the first and last sample exactly at the canvas edges) down to **1 sample per 10 px**
  (`samplesPerPixel = 0.1`), per this ticket. `verticalZoom` and `samplesPerPixel` are independent
  axes — zooming horizontally never changes the amplitude scale.
- **Zoom in/out (keyboard):** `=` zooms in, `-` zooms out, by a fixed step factor (√2 per press,
  giving a doubling every two presses) — **Verified** against Audition's default bindings by three
  independent sources: [tutorialtactic.com](https://tutorialtactic.com/blog/adobe-audition-shortcuts/)
  ("Zoom in horizontally: =", "Zoom out horizontally: −"), [pie-menu.com](https://www.pie-menu.com/shortcuts/adobe-audition)
  (same two bindings), and a web search summary corroborating both. The official
  `helpx.adobe.com/audition/desktop/keyboard-shortcuts/default-keyboard-shortcuts.html` page returned
  HTTP 403 to automated fetch, consistent with `docs/references.md`'s and SPEC-003's existing note
  that this page 403s.
  - **Decided (autonomous, T-200):** the zoom is centered on the **playhead** if it's currently visible
    in the viewport, otherwise on the **viewport's horizontal center** — neither Audition source
    documents a centering rule, and centering on the playhead matches the common "I'm zooming in on
    what I'm listening to/editing near" intent.
- **Vertical zoom in/out (keyboard):** `Alt+=` zooms in, `Alt+-` zooms out (Option on macOS) —
  **Verified**, same two sources as above, both agreeing on the modifier.
  - **Decided (autonomous, H-35):** no source documents a reset-to-1×-vertical-zoom binding.
    `Alt+0` is chosen — conservative and non-conflicting (unused by any other default binding,
    `registry.ts`'s `findDuplicateBindings` enforces this), and `0` reads naturally as "reset" next
    to `Alt+=`/`Alt+-`'s in/out, the same digit-as-endpoint shape numeric zoom controls commonly
    use elsewhere (e.g. browsers' `Ctrl+0` = reset zoom).
- **Mouse wheel:** plain wheel scrolls the view horizontally (§2.7); **Ctrl+wheel zooms horizontally**,
  centered on the pointer's document-sample position; **Alt+wheel zooms vertically** (§2.4), centered
  on the ruler's current center line.
  - The Ctrl-modifier-to-zoom convention is **verified**: an Adobe Community thread
    ([community.adobe.com](https://community.adobe.com/t5/audition-discussions/zooming-in-out-with-wheel/td-p/14357347))
    confirms current Audition requires Ctrl+wheel to zoom, with plain wheel doing the pan/scroll a
    user would otherwise expect from the wheel alone — matching what we adopt here.
  - **Decided (autonomous, T-200):** cursor-centered zoom (the sample under the pointer stays under the
    pointer after the zoom) and the Alt-for-vertical convention are our own choices — the community
    thread doesn't document cursor-centering or a vertical-zoom wheel modifier, and this is standard,
    predictable behavior in comparable editors.
- **Zoom to selection** and **zoom full** (fit the whole document): both commands exist as toolbar
  buttons/menu items unconditionally (available regardless of shortcut). Their default *keyboard*
  shortcuts are **provisional — deferred to SPEC-019**, because the two sources found disagree and
  neither could be checked against the (403'd) official page:
  - [tutorialtactic.com](https://tutorialtactic.com/blog/adobe-audition-shortcuts/): "Zoom to
    selection: Ctrl+Alt+=", "Zoom out full: Ctrl+Alt+−".
  - [pie-menu.com](https://www.pie-menu.com/shortcuts/adobe-audition): "Zoom in on selection:
    Shift+S", "Zoom out to entire waveform: ⌘+\\" (this source also mixes Option/Command modifier
    glyphs inconsistently across its own list, which lowers confidence in it further).
  - ⚠ **Contradiction, not silently resolved:** these two sources disagree on both bindings and don't
    even agree on modifier *style* internally. Per this ticket's instruction, both commands are
    implemented and reachable by menu/toolbar now (H-35; PowerVoice has no command palette — that
    was this bullet's own speculative wording, corrected here); **no keyboard shortcut is bound
    for either** pending SPEC-019's owner-reviewed shortcut audit (M7). Do not guess a binding
    that might collide with something SPEC-019 assigns later.
  - **Zoom to selection** sets `startSample = selection.startSample`, `samplesPerPixel =
    (selection.endSample − selection.startSample) / viewportPx`, clamped to the §2.6 range; a no-op
    (shows nothing, no error) when there's no selection.
  - **Zoom full** sets `samplesPerPixel = len_samples / viewportPx`, `startSample = 0`.

### 2.7 Horizontal scrolling and overview

- **Scrollbar.** A standard horizontal scrollbar tracks `startSample`/`samplesPerPixel` against
  `len_samples`; dragging it pans without changing zoom.
- **Keyboard paging:** Page Up scrolls left by one viewport width, Page Down scrolls right by one
  viewport width — **single-source, unverified**: only tutorialtactic documents these; no second
  source corroborates, so (like SPEC-003's treatment of unconfirmed bindings) this is implemented but
  flagged unverified pending SPEC-019.
- **Overview strip** (§2.1): always shows the whole file at the coarsest pyramid level (65 536 spp,
  reduced further if the strip is narrower than `len_samples / 65536` pixels), independent of the main
  viewport's zoom.

### 2.8 Playhead display and follow

This view is a **consumer**, not an owner, of the playhead contract — SPEC-003 §2.2 defines
extrapolation, slew-vs-jump, and the telemetry source (`VXTM` over the 60 Hz `telemetry` channel,
ADR-003). This spec covers only how the waveform view draws and follows it:

- **Display.** Every animation frame, the view computes `displayedPosition` exactly per SPEC-003 §2.2's
  formula from the latest `VXTM` anchor and the UI's clock-sync offset, converts it to a pixel x via
  §4.1, and draws a 1-device-pixel-wide vertical line at that x in the theme's playhead-accent token.
  No separate polling or re-derivation of position happens in this view.
- **Follow scrolling.** When `playhead_follow` (SPEC-003 §2.1/§3, view toggle, default on) is on and
  the displayed position would leave a central **follow band**, the view scrolls to keep it inside the
  band, in the same animation frame as the playhead redraw (SPEC-003 AC-5's "no tearing" requirement,
  which this spec inherits verbatim for the waveform view).
  - **Decided (autonomous, T-200): continuous ("smooth") scroll within a follow band spanning the
    middle 80% of the viewport width (10%–90%)**, not page-at-a-time jumps. When the playhead would
    move past the band's trailing edge, `startSample` shifts by exactly the overflow that frame, so the
    playhead visually holds at the band edge while the waveform continues to scroll under it smoothly.
    Page-jump scrolling (common in some DAWs) causes a visible full-viewport jump-cut mid-listen,
    which is disorienting for the kind of close, repeated listen-back editing this app is built for;
    continuous scrolling never re-frames the whole view at once.
  - A user-initiated scroll or zoom during playback temporarily suspends follow until the playhead next
    re-enters the band from a subsequent telemetry anchor, so manually panning to inspect something
    while audio keeps playing doesn't fight the user (same spirit as SPEC-003 §2.1's "out of scope to
    fully specify the toggle UI, but default is on").

### 2.9 Time selection

- **Click** (no drag) on the waveform: clears any existing selection and moves the edit cursor
  (playhead, when stopped) to the clicked document sample, snapped per §2.11 if enabled.
- **Click-drag:** creates a selection from the mousedown sample to the current pointer sample,
  live-updating; releasing sets the final selection. The anchor is the mousedown point — dragging past
  it in either direction is allowed (the selection is always normalized to `startSample < endSample`
  regardless of drag direction).
- **Shift+click:** extends the existing selection's far edge (the edge farther from the click point) to
  the clicked sample; with no existing selection, behaves like a plain click-drag anchored at the
  previous cursor position.
- **Double-click:**
  - **Decided (autonomous, T-200): selects the entire document** (equivalent to Ctrl+A). Audition's
    own double-click behavior in a single-file waveform editor isn't independently documented by any
    source found for this ticket (community discussions describe multitrack clip double-click, a
    different view); "select everything, fast" is a safe, common, low-risk convention that matches
    Ctrl+A's existing semantics and serves the common VO workflow of normalizing/exporting the whole
    file.
- **Ctrl+A:** selects the entire document (`[0, len_samples)`).
- **Esc:** clears the selection (the edit cursor stays where the selection's near edge was, or where it
  already was if there was no selection).
- **Selection handles:** once a non-empty selection exists, its two boundary pixels are grab handles —
  hovering within **6 px** of a boundary shows a resize cursor; dragging a handle moves that boundary
  (snapped per §2.11 if enabled), and dragging one handle past the other swaps which edge is "start."
- Selection state is view/UI state, not document state (SPEC-004 §2.2), so it is never in the undo
  history and survives no crash; it's read fresh from the current document on open.

### 2.10 Snap to zero-crossing (M2 scope)

- A settings toggle, `snapToZeroCrossing`, default **off**.
- **When on:** placing or dragging a **selection boundary** (drag-create or handle-drag) snaps to the
  nearest zero crossing (a sign change between two consecutive document samples, i.e. `sample[i] ≥ 0 >
  sample[i+1]` or `sample[i] < 0 ≤ sample[i+1]`) within a search window, instead of landing exactly on
  the pointer's raw sample.
  - **Decided (autonomous, T-200):** the search window is **±512 samples** (~10.7 ms at 48 kHz) from
    the raw pointer sample. If no sign change exists in that window (e.g. inside true digital silence,
    which never crosses because it's already zero, or a long DC-offset run), the boundary is **not**
    snapped and lands on the raw sample with no error/notice — silently doing nothing is the least
    surprising behavior for an edge case that is rare in real recordings.
  - **Decided (autonomous, T-200):** snapping applies only to **selection boundaries**, not to a plain
    click's cursor placement or the playhead — snapping cursor placement during ordinary
    scrubbing/listening would make the cursor visibly jump away from where the user actually clicked,
    which is only welcome when the point is about to become a destructive edit boundary.
  - A sample exactly at zero is its own zero crossing (distance 0).

### 2.11 Markers on the waveform (M2 scope)

- Markers (SPEC-004 §2.2 data model, `Marker { id, pos_samples, len_samples, name }`) are drawn as a
  thin vertical line in the theme's marker-accent token at `x(marker.pos_samples)` (§4.1), with a small
  triangular flag at the top of the waveform canvas.
- The marker's name is shown as a label near the flag when there's room (label width < distance to the
  next marker's flag at the current zoom); otherwise the flag alone is drawn and the full name is
  available on hover (a plain browser tooltip is enough for M2).
- Region markers (`len_samples > 0`) are drawn as a shaded band between their start and end lines
  instead of a single line.
- The markers **list panel** (add/rename/navigate/delete UI, PROMPT §3.6 "markers + properties, left/
  bottom") is **M3** — out of scope here; this spec only covers drawing markers that already exist.

### 2.12 Theming

All colors are theme tokens (dark palette, PROMPT §2 "UI — Audition-like dark layout"), not literals,
so a future theme/accent change (PROMPT §3.6 Settings) never touches this view's code:
`--wave-bg`, `--wave-fill`, `--wave-outline`, `--wave-pending` (§2.3 PARTIAL placeholder),
`--wave-clip` (§2.3 clip highlight), `--wave-playhead`, `--wave-selection-fill`,
`--wave-selection-handle`, `--wave-marker`, `--wave-marker-region`, `--wave-ruler-text`,
`--wave-ruler-grid`, `--wave-zero-line`. Exact hex values are the implementing ticket's concern (a
design-tokens file, if one doesn't exist yet by M2, is created there); this spec fixes only the token
*names* and *roles* so tests can assert "the clip color token is used," not a literal color.

### 2.13 Renderer

- **WebGL2 primary, Canvas2D fallback** (ADR-009 §2, §4) — this spec does not re-decide the renderer,
  it inherits ADR-009's choice and detection strategy verbatim:
  - Feature-detect `canvas.getContext("webgl2")` at view creation; `null`/throw → Canvas2D for this
    view's lifetime, with a `notice` event so the user can see which path is active (ADR-009 §4).
  - On `webglcontextlost`, fall back to Canvas2D for the remainder of the view's lifetime rather than
    attempting to recreate the WebGL2 context repeatedly (ADR-009 §4).
- **HiDPI.** The canvas's backing store is sized at `cssWidth × devicePixelRatio` ×
  `cssHeight × devicePixelRatio` device pixels; all pixel-space math in §4.1/§4.2 operates in device
  pixels, and CSS size (and therefore layout) is unaffected. The view re-sizes its backing store and
  redraws (not just rescales) when `devicePixelRatio` changes (e.g. the window moves to a differently
  scaled monitor), so lines stay crisp rather than blurred by browser upscaling.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` is the packaged Linux default (ADR-009 Amendment 1) and is out of
  this spec's scope to re-decide — it's an environment variable, not a renderer behavior.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `samples_per_pixel` | Horizontal zoom | doc samples / px | `0.1 … len_samples / viewportPx` | fit whole file (zoom full) at open | continuous (drag/wheel), √2 per keyboard press | §2.6 |
| `vertical_zoom` | Amplitude zoom | × | `1 … 256` | 1 | power-of-2 (keyboard/menu), continuous (drag) | §2.4 |
| `amplitude_ruler_mode` | Amplitude ruler | enum | `dbfs`, `percent` | `dbfs` | — | §2.4 |
| `time_ruler_format` | Time ruler | enum | `timecode`, `samples`, `seconds` | `timecode` | — | §2.5 |
| `playhead_follow` | Playhead follow (owned by SPEC-003 §3) | bool | on/off | on | n/a | consumed here, §2.8 |
| `follow_band` | Follow-scroll keep-visible band | % of viewport | fixed `10–90` | `10–90` | fixed | §2.8 |
| `snap_to_zero_crossing` | Snap selection boundaries to zero crossings | bool | on/off | off | n/a | §2.10 |
| `snap_search_window_samples` | Zero-crossing search window | doc samples | fixed | `512` | fixed | §2.10 |
| `clip_threshold_abs` | Clip highlight threshold | linear amplitude | fixed | `1.0` | fixed | §2.3 |
| `dot_threshold_px_per_sample` | Raw-sample dot display threshold | px | fixed | `3` | fixed | §2.3 |
| `max_buckets_per_pixel` | Peak reduction ceiling | buckets | fixed | `4` | fixed | ADR-003 §2, inherited |
| `selection_handle_hit_px` | Selection handle grab width | px | fixed | `6` | fixed | §2.9 |
| `zoom_step_factor` | Keyboard zoom step | × | fixed | `√2` | fixed | §2.6 |

## 4. Algorithm / implementation notes

### 4.1 Pixel ↔ document sample mapping

```
sample(px) = startSample + round(px * samplesPerPixel)
px(sample) = round((sample - startSample) / samplesPerPixel)
```

Both directions use the **same** `samplesPerPixel` snapshot for a given frame — the view never mixes
a stale zoom level with a fresh pan or vice versa within one redraw, which is what makes selection
boundaries, ruler labels and marker positions all agree with each other and with the playhead in the
same frame. All three call sites (mouse hit-testing, ruler tick generation, marker/playhead draw)
share one implementation (`ui/src/lib/waveform/coords.ts` or equivalent), tested once (§6).

### 4.2 Ruler tick generation

- **Time ruler:** given the visible sample range `[startSample, startSample + viewportPx *
  samplesPerPixel)`, pick the largest step from the ladder `{1, 2, 5} × 10ⁿ` (in the current format's
  unit — seconds for `timecode`/`seconds`, samples for `samples`) such that consecutive labels are
  ≥ some minimum pixel gap (enough for the longest label string at the current font). Tick sample
  positions are then `k × step` converted back through the format (e.g. seconds → samples via
  `round(seconds × sample_rate_hz)`), so a label's sample position is always an exact document sample,
  never a rounded-then-reconverted approximation.
- **Amplitude ruler:** dBFS ticks come from the fixed set in §2.4, thinned by the same
  minimum-pixel-gap rule against the current `verticalZoom`; percentage ticks are `{100, 50, 0, −50,
  −100}%` and finer subdivisions thinned the same way.

### 4.3 Peak request pipeline

1. On any view-state change affecting the visible sample range or `samplesPerPixel`, compute the
   target pyramid level (ADR-003 §2) and issue `peaks_get(PeaksRequest { request_id, audio_rev, spp,
   start_sample, count })` for the visible range (± a small prefetch margin, e.g. one viewport width
   each side, to make small pans not re-request).
2. On each `VXPK` response: reject if `audio_rev` ≠ current (ADR-003) or `request_id` is superseded for
   its `(level, range)` key (§2.3); otherwise merge into the view's bucket cache and redraw.
3. `PARTIAL` buckets redraw as `--wave-pending` (§2.3) and are re-requested when `peaks_progress`
   reports the covering chunk(s) done.
4. Below `spp = 64`, step 1 requests `RAW` (`flags bit0`) instead of a pyramid level; the request/limit
   rules (65 536 buckets or 1 Mi samples, ADR-003 §2) still apply, so an extreme raw-mode viewport
   wider than that is served as several requests tiled across the visible range.

### 4.4 Zero-crossing snap

Given a raw pointer sample `p` and a chunk-backed sample accessor (reusing the same RAW request path
as §4.3, not a separate IPC round trip when the samples are already cached for display), scan
outward from `p` alternating `+1, −1, +2, −2, …` up to `±512` samples; return the first index where
`sample[i]` and `sample[i+1]` have a sign change (§2.10) or `sample[i] == 0`. Alternating outward scan
picks the *nearest* crossing, breaking ties toward the earlier (lower) index. Return "no snap" if the
window is exhausted with no crossing.

### 4.5 Renderer mechanics

Per ADR-009 §2: WebGL2 draws the min/max fill as a vertex buffer of one (min, max) quad per pixel
column, uploaded via `bufferSubData` on each peak update rather than rebuilt from scratch, so a small
scroll/zoom delta only touches the changed columns; playhead, selection and marker lines are drawn as
thin quads in the same draw call batch to avoid extra passes. The Canvas2D fallback rebuilds a `Path2D`
per redraw (the per-frame cost ADR-009 measured as statistically indistinguishable from WebGL2 at this
content size, §3 there) and manually composites clip/marker/playhead overlays. Both paths share the
same coordinate math (§4.1) and the same theme tokens (§2.12), so there is exactly one visual output
regardless of which renderer is active — a Vitest snapshot-equivalence test (§6) checks this indirectly
by asserting both paths compute the same pixel-space geometry from the same view state, since a true
pixel-diff test would need a real GPU.

### 4.6 Playhead reuse

The waveform view imports SPEC-003 §2.2's extrapolation function as-is (same module, same clock-sync
offset) rather than re-deriving position from `VXTM` locally — this is what guarantees this view's
playhead agrees with the transport bar's playhead display to the pixel, since both read the same
computed `displayedPosition` each frame.

## 5. Acceptance criteria

- **AC-1 (level selection & reduction).** Given a synthetic 60-min document's peak pyramid and a
  viewport width, when `samplesPerPixel` sweeps from `0.1` to `len_samples / viewportPx`, then the
  level chosen at every step satisfies `level ≤ samplesPerPixel` and is the *largest* such level from
  `{64, 256, 1024, 4096, 16384, 65536}` (or RAW below 64), and reducing that level's buckets into pixel
  columns never combines more than **4** buckets per column (± 0, exact).
- **AC-2 (peak union exactness).** Given a chunk's known min/max pyramid and a piece boundary that
  splits a chunk mid-range, when the document-level bucket for a pixel spanning the boundary is
  computed, then it equals the conservative union (min of mins, max of maxes) of the overlapping chunk
  buckets, exact, and is never narrower than the true sample range's min/max (verified against a
  brute-force scan of the same synthetic range).
- **AC-3 (raw + dot rendering).** Given `samplesPerPixel < 64` down to `0.1`, when the view renders,
  then: below 64 spp the request is `RAW` (not a pyramid level); the polyline connects consecutive raw
  samples exactly (no interpolation/smoothing); dots appear on individual samples if and only if
  `1 / samplesPerPixel ≥ 3` px (± 0, a pure threshold check).
- **AC-4 (horizontal zoom bounds).** Given a document of `len_samples` and a viewport of `viewportPx`,
  when "zoom full" is invoked, then `samplesPerPixel = len_samples / viewportPx` exactly and sample 0
  and sample `len_samples − 1` are within 1 px of the canvas edges; when zoomed to the maximum, then
  `samplesPerPixel = 0.1` exactly and attempting to zoom in further is a no-op (clamped, no error).
- **AC-5 (zoom to selection).** Given a selection `[S, E)` and a viewport of `viewportPx`, when "zoom to
  selection" is invoked, then `startSample = S` exactly and `samplesPerPixel = (E − S) / viewportPx`
  exactly (clamped into the AC-4 range if the selection is shorter than 0.1 samples/px would allow).
- **AC-6 (ruler label exactness).** Given any `(startSample, samplesPerPixel, timeRulerFormat)` combo
  from a matrix covering all three formats and zoom levels from whole-file to maximum, when tick labels
  are generated, then every label's underlying document sample equals `sample(px)` (§4.1) at that
  label's exact pixel position, with **zero** sample error — not "close," exact, because §4.2's ladder
  always converts through the same rounding as §4.1.
- **AC-7 (selection exactness across zoom).** Given a click-drag selection made at zoom level Z1
  yielding boundaries `(S, E)` in document samples, when the view is zoomed to Z2 (any value in the
  AC-4 range) and back to Z1, then the stored selection is unchanged (`S`, `E` bit-identical `u64`s) —
  selection state is never re-derived from pixels after creation, only converted *to* pixels for
  display.
- **AC-8 (selection interactions).** Given an empty document view: a click-drag from px `a` to px `b`
  (`a > b` allowed) produces a normalized selection with `start < end`; Shift+click extends the far
  edge; double-click and Ctrl+A both select `[0, len_samples)` exactly; Esc clears the selection
  (`null`); dragging a selection handle past the opposite handle swaps which edge is "start" without
  ever producing `start > end`; a handle's hit target is exactly `selection_handle_hit_px` (6 px) wide
  centered on the boundary pixel.
- **AC-9 (zero-crossing snap).** Given a synthetic sample array with a known, unique zero crossing at
  index `k` within 512 samples of pointer sample `p`, when snap is enabled and a selection boundary is
  placed at `p`, then the boundary lands at `k` exactly. Given an array with **no** sign change within
  ±512 samples of `p` (e.g. constant positive DC), then the boundary lands at `p` unchanged (no snap,
  no error). Given snap disabled, the boundary always lands at the raw pointer sample regardless of
  nearby crossings.
- **AC-10 (playhead display agreement).** Given a simulated `VXTM` telemetry stream driving both the
  transport bar's playhead readout and this view's playhead line (SPEC-003 AC-6's harness reused),
  when sampled at an arbitrary instant between two telemetry frames, then both compute the same
  `displayedPosition` (bit-identical `f64`) from the same anchor and clock offset, and the view's drawn
  playhead x equals `px(displayedPosition)` (§4.1) to the device pixel.
- **AC-11 (playhead follow, no tearing).** Given `playhead_follow` on and a document longer than the
  viewport, when simulated playback telemetry advances the playhead toward the follow band's trailing
  edge (§2.8), then the view scrolls to keep it within `[10%, 90%]` of the viewport width at every
  animation frame, and the scroll update and the playhead redraw happen within the same animation
  frame (no frame where the playhead is drawn outside the band, no frame where the waveform has
  scrolled but the playhead hasn't been redrawn to match).
- **AC-12 (clipped-sample highlight).** Given a synthetic bucket/sample stream containing values at
  exactly `1.0`, `−1.0`, `0.999999`, and `1.5` (float headroom), when rendered, then the `1.0`, `−1.0`
  and `1.5`-containing columns/samples are drawn in `--wave-clip` and the `0.999999` one is not — the
  threshold is exactly `|x| ≥ 1.0`, not a margin.
- **AC-13 (progressive display & refresh).** Given a `VXPK` response with some buckets flagged
  `PARTIAL`/`NaN` and a subsequent `peaks_progress` event reporting the covering chunk done, when the
  view processes both, then the `PARTIAL` buckets render as `--wave-pending` immediately, and after the
  progress event the view re-requests and redraws that exact range with real `(min, max)` values,
  without requiring a manual scroll/zoom/resize to trigger it.
- **AC-14 (stale/out-of-order rejection).** Given two in-flight `peaks_get` requests for the same
  `(level, range)` key where the response for the later `request_id` arrives first, when the earlier
  request's response then arrives, then it is discarded and the view's displayed buckets remain the
  ones from the later response. Given a response whose `audio_rev` doesn't match the document's current
  `audio_rev` (an edit committed after the request was sent), then it is discarded outright regardless
  of `request_id` ordering.
- **AC-15 (renderer fallback).** Given a stubbed `getContext('webgl2')` returning `null`, when the view
  initializes, then it renders via Canvas2D and emits the ADR-009 `notice` with no thrown error. Given
  a `webglcontextlost` event fired mid-session on an active WebGL2 view, then it falls back to Canvas2D
  for the rest of that view's lifetime and does not attempt to recreate the WebGL2 context on
  subsequent redraws.
- **AC-16 (HiDPI correctness).** Given a view at CSS size `W×H` and `devicePixelRatio = 2`, when it
  renders, then the canvas backing store is `2W × 2H` device pixels, and `px(sample)`/`sample(px)`
  (§4.1) operate in device pixels consistently, so a vertical marker/playhead line lands on the same
  physical position regardless of `devicePixelRatio`. When `devicePixelRatio` changes at runtime
  (simulated), the view resizes its backing store and redraws (not CSS-rescales) within one animation
  frame.
- **AC-17 (marker line exactness).** Given a document snapshot with markers at known `pos_samples`,
  when the view renders at any zoom in the AC-4 range, then each marker's drawn line x-position equals
  `px(marker.pos_samples)` (§4.1) exactly, and a region marker's shaded band spans exactly
  `[px(pos_samples), px(pos_samples + len_samples)]`.
- **AC-18 (performance — 60 fps zoom/scroll on a 60-min document).** Given a synthetic 60-min 48 kHz
  mono document's full peak pyramid loaded and a scripted 10 s zoom-then-scroll sweep (matching ADR-009
  §3's spike methodology), when run on the owner's reference hardware (AMD Phoenix/Mesa, WebKitGTK
  4.1, `WEBKIT_DISABLE_DMABUF_RENDERER=1` on), then the median (p50) frame time is ≤ 16.7 ms (60 fps)
  and **p99 ≤ 50 ms**, with **at most 1** frame exceeding 50 ms in the 10 s sweep.
  - ⚠ **Contradiction flagged, not silently resolved:** this ticket's brief states the AC as "no frame
    > 50 ms during a 10 s zoom sweep" (zero tolerance). ADR-009 §3's own measured spike data (waveform
    WebGL2/Canvas2D, `dmabuf-off` runs) shows occasional single-frame outliers up to **87 ms** and
    **28–31 ms** across otherwise-clean 10 s runs (1–2 frames per run flagged `dropped(>25ms)`), on the
    same reference hardware this spec targets. A strict zero-outlier reading of the brief's AC would
    therefore already be known-failing against the platform's own measured baseline before any
    waveform-specific code exists. This spec adopts the **p99 ≤ 50 ms, ≤ 1 outlier frame** tolerance
    above as its Decided-autonomous default (rationale: matches ADR-009's own observed noise floor
    rather than pretending it doesn't exist) and reports the contradiction rather than quietly
    rewriting the brief's literal wording.
- **AC-19 (performance — first draw < 3 s).** Given a 60-min 48 kHz mono WAV opened on the owner's
  reference SSD/NVMe machine (ADR-004 §8's ~1–2 s import estimate), when the document open command
  reports success, then the waveform view's first painted frame (even if some columns are
  `--wave-pending`/`PARTIAL`, §2.3) appears within **3 s** of that success, measured from the `open`
  command's completion timestamp to the first `requestAnimationFrame` that actually draws a non-empty
  canvas.

## 6. Test plan

| AC | Unit (TS pure fn) | Vitest component (mockIPC) | Rust (peaks) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | level-selection function swept over a table of `samplesPerPixel` values | — | pyramid levels match `{64,256,...,65536}` spacing (golden) | — |
| AC-2 | — | render a synthetic multi-chunk snapshot with a mid-chunk piece split, assert displayed bucket vs. brute-force scan | `project` unit test: document-level union over a piece boundary vs. brute-force min/max scan of the same synthetic samples | — |
| AC-3 | polyline/dot-threshold pure functions | render at `samplesPerPixel` sweep incl. `<1`, assert dot presence matches `1/spp ≥ 3` | RAW request path returns exact samples (bit-identical) for a synthetic chunk | zoom to max, eyeball dots appearing on individual samples |
| AC-4 | zoom-full / zoom-max clamp arithmetic | mockIPC-backed view: invoke zoom full/max, assert `startSample`/`samplesPerPixel` | — | Ctrl+wheel to the zoom limits, confirm no error/further zoom |
| AC-5 | zoom-to-selection arithmetic | mockIPC-backed view: select then zoom-to-selection, assert exact viewport | — | select a region, zoom to selection, confirm boundaries match |
| AC-6 | ruler tick-ladder + label-to-sample function, matrix over all 3 formats × zoom levels | render ruler at each combo, assert label DOM/canvas text matches `sample(px)` | — | switch ruler format, confirm labels look sane at several zooms |
| AC-7 | — | mockIPC-backed view: create selection, zoom away and back, assert `u64` selection unchanged | — | select, zoom in/out repeatedly, confirm selection markers didn't move |
| AC-8 | selection normalization / handle hit-test functions | simulated pointer events: click-drag, Shift+click, double-click, Ctrl+A, Esc, handle drag-past-opposite | — | do each interaction by hand, confirm behavior |
| AC-9 | zero-crossing scan function (synthetic arrays incl. no-crossing case) | mockIPC-backed view: drag a handle near a known crossing with snap on/off | `project`/RAW-serving test: sample fetch for the scan window is exact | toggle snap, drag near a transient, confirm it grabs the crossing |
| AC-10 | — | shared harness with SPEC-003 AC-6: assert this view's playhead x equals transport bar's derived position | — | watch playback with the waveform and transport bar both visible |
| AC-11 | follow-band math (pure fn: given anchor + band, next `startSample`) | simulated telemetry stream drives the view (SPEC-003 AC-5 harness reused), assert band invariant + same-frame redraw every frame | — | play a long file zoomed in, confirm smooth continuous follow, no jump-cuts |
| AC-12 | clip-threshold pure function | render synthetic buckets/samples at the four test values, assert token used per column/sample | Rust-side: verify `VXPK` payload for a stress fixture contains the expected extreme values (data correctness, not color) | play/view a clipped stress file, confirm red peaks visible |
| AC-13 | — | mockIPC: respond with `PARTIAL` buckets then a `peaks_progress` event, assert redraw sequence | `project`/IPC: `PARTIAL` flag set correctly while a chunk's peaks aren't ready (golden `VXPK` fixture, ADR-003 §4-style) | open a large file, watch it fill in progressively |
| AC-14 | request/response ordering-key function | mockIPC: deliver responses out of order and with stale `audio_rev`, assert only the correct one is applied | — | rapid zoom/scroll on a real file, confirm no visible flicker-back |
| AC-15 | — | stub `getContext` to return `null`; fire synthetic `webglcontextlost`; assert fallback + no re-creation attempts + `notice` emitted | — | force Canvas2D via a debug flag, confirm it still looks correct |
| AC-16 | `px`/`sample` device-pixel math at `dPR ∈ {1, 1.5, 2}` | render at each `devicePixelRatio`, assert backing-store size and line position agreement | — | check crispness on the owner's actual display scaling |
| AC-17 | marker x-position pure function | render a synthetic marker set at a zoom sweep, assert drawn x matches `px(pos_samples)` | — | scrub to a marker, confirm the line lines up with what's heard |
| AC-18 | — | headless frame-time harness (reusing `ui/src/spike/frameBench.ts`'s watchdog pattern, ADR-009) driving a real 60-min synthetic pyramid through a 10 s zoom/scroll script | — | `just spike`-style run on the owner's machine at M2, compare against ADR-009 §3 baseline numbers |
| AC-19 | — | timestamp-instrumented open→first-paint harness against a generated 60-min fixture (`just fixtures`) | — | open a real 60-min WAV from cold, time it by eye/stopwatch |

Fixtures: `just fixtures`' 60-min generated WAVs (MEMORY T-006), synthetic multi-chunk snapshots and
peak pyramids built directly in Rust/TS test code (no need to round-trip a real file for most unit/
component tests), golden `VXPK` fixtures extended with `PARTIAL`/stale-`audio_rev`/out-of-order cases
(ADR-003 §4's `gen_ipc_fixtures` pattern).

## 7. Out of scope

- Spectral/STFT display, colormap, log/linear frequency toggle, analyzer, and the split/toggle
  mechanics between waveform and spectral panes — SPEC-007.
- Decoding/importing any source format (WAV/FLAC/MP3/AAC/Vorbis) — SPEC-005; this view only ever reads
  the session's peak pyramid (§2.3), never the source file.
- Destructive editing operations themselves (cut/copy/paste/trim/silence/normalize) that *act on* a
  selection this view produces — M3 editing spec. This spec only defines what the selection *is*, in
  document samples.
- The markers list panel (add/rename/navigate/delete UI) — M3; this spec only draws markers that
  already exist (§2.11).
- Level meters (peak/RMS) and the output spectrum analyzer — separate M2/M4 specs (PROMPT §3.2).
- The full Audition shortcut map beyond the zoom/selection commands named here — SPEC-019 (M7),
  including the two provisional bindings flagged in §2.6.
- Multi-document/tabs, remappable shortcuts (PROMPT §2, locked out of v1).
- Exact theme token color values (§2.12) — a design-tokens artifact belonging to the implementing
  ticket, not this behavior spec.
