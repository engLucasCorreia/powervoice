# SPEC-007 — Spectral display & live output spectrum analyzer

- **Status:** approved (autonomous, T-200)
- **Milestone:** M2 (T-204 STFT tile service, T-207 spectrogram renderer and split view, T-208 live
  output analyzer). **[M4]** items describe the analyzer's reuse by the EQ graph (T-409). Each AC
  carries its milestone.
- **Related:** SPEC-000 (glossary, testkit conventions), SPEC-003 (playhead), SPEC-004 (`audio_rev`,
  memory budget, destructive edits), SPEC-005 (spectral view unlocks when the import completes),
  SPEC-006 (shared viewport, rulers, selection, renderer, theme tokens), SPEC-012 §4.3 (the
  sine-normalized dB convention reused here) · ADR-001 §4 (spectrogram tile service in `engine`, STFT
  in `dsp`), ADR-002 §1, §2, §4 (workers, output callback, rings), ADR-003 (`VXST`, channels,
  `audio_rev`), ADR-004 §4 (tile cache inside the memory budget), ADR-005 §11 (`ResponseCurve`),
  ADR-009 (WebGL2 primary, Canvas2D fallback; 60 Hz telemetry) · PROMPT §2 EQ (LOCKED), §3.2, §3.4,
  §3.8 (spectral *editing* out of scope), §4

## 1. Purpose
Many voice-over problems are invisible in a waveform but obvious in a spectrogram:
- mains hum and its harmonics;
- a steady hiss or fan whine;
- mouth clicks;
- plosive thumps;
- harsh sibilance;
- a band-limited, dull recording.

The **spectral display** lets the user see them over the whole file, aligned with the waveform, while
selecting and editing in the same time axis. The **live output analyzer** shows what the rack is doing
to the sound right now. It is the background the EQ graph draws its curve on (M4), so the user can
shape the EQ against the real spectrum of their voice.

## 2. Behavior / UX

### 2.1 Showing it: toggle and split (Audition-style)
- **Default: waveform only.**
- **Toggle.** View → **Show Spectral Frequency Display**, shortcut **Shift+D**, shows or hides the
  spectral pane. Audition's own binding is **verified** in two sources:
  - Adobe's how-to page ([helpx.adobe.com, "How to use Spectral Frequency Display"](https://helpx.adobe.com/ph_fil/audition/how-to/audition-spectral-frequency-display-cc.html));
  - an Adobe Community thread ([community.adobe.com](https://community.adobe.com/t5/audition-discussions/lost-the-waveform-and-can-t-find-the-frequency/td-p/10496753)).

  Both state View > Show Spectral Frequency Display / Shift+D. The final map is SPEC-019.
- **Split layout** (like Audition's Waveform editor with the spectral display shown), top to bottom:
  1. the shared time ruler (SPEC-006 §2.1);
  2. the **waveform pane**;
  3. a 6 px **divider**;
  4. the **spectral pane**;
  5. the shared scrollbar and overview strip (SPEC-006 §2.7).
- **Left gutter.** It holds the amplitude ruler for the waveform pane (SPEC-006 §2.4) and the
  **frequency ruler** for the spectral pane (§2.4), at the same width.
- **Divider.** The default split is 50 / 50. Dragging moves it anywhere from 0 % to 100 %; a pane
  dragged to 0 collapses to a 6 px grab strip, so "spectral only" and "waveform only" are both
  reachable. Double-clicking the divider resets it to 50 %.
- **Hiding.** Shift+D again hides the spectral pane and remembers the ratio. Visibility, ratio and
  the §2.4–§2.6 display settings are saved as app settings; from M3 they go in the sidecar view state
  (T-306).
- **Disabled states:**
  - **During import** the pane shows "Available when the file has finished opening" (SPEC-005 §2.3)
    and requests no tiles.
  - **During recording** the pane keeps its last image with the overlay "Spectral view updates after
    recording" and requests no tiles. After Stop, the take's `audio_rev` triggers a refresh.
    **Decided (autonomous, T-200):** recording safety and CPU headroom matter more than a live
    spectrogram of the take.
- **Rationale for the default** (**Decided (autonomous, T-200)**): the pane starts hidden, as in
  Audition, because most editing is done on the waveform. The split is the only multi-pane layout;
  there is no separate tabbed spectral mode.

### 2.2 What the spectral pane draws
- **Content.** The STFT magnitude of the document, per bin, in sine-normalized dB (§4.2). It is
  colored through the colormap between the display floor and ceiling (§2.5).
- **Overlays.** The spectral pane draws the same overlays as the waveform pane, at the same x
  positions, from the same shared state and theme tokens (SPEC-006 §2.12):
  - playhead;
  - time selection (full height);
  - markers and region bands;
  - the playhead-follow scrolling.
- **No editing.** There is **no spectral (frequency-range) selection and no spectral editing**
  (PROMPT §3.8). Clicks and drags act on time exactly as in the waveform pane (SPEC-006 §2.9),
  including zero-crossing snap (SPEC-006 §2.10).

### 2.3 Time axis and zoom sync with the waveform (SPEC-006)
- **One viewport.** Both panes share SPEC-006's view state: `startSample`, `samplesPerPixel`,
  `selection`. A zoom or scroll gesture over either pane updates both **in the same animation
  frame**. Plain wheel scrolls, Ctrl+wheel zooms around the pointer, `=`/`-` zoom, and the overview and
  scrollbar behave as in SPEC-006 §2.6/§2.7. Alt+wheel over the spectral *pane* does nothing; over the
  frequency ruler it zooms frequency (§2.4).
- **Frame placement.** STFT frame *i* is centred at document sample `c_i` and drawn at x = `px(c_i)`
  (SPEC-006 §4.1). A transient therefore lines up in both panes to within half a hop (AC-4).
- **Time resolution follows zoom.** The hop is chosen from the zoom (§4.3). There are always 1–2
  frames per device pixel column when zoomed out, and never fewer than 16 frames per window length
  when zoomed in.
  - When a column covers several frames, it shows their **maximum**.
  - When zoomed in past one frame per column, adjacent frames are **linearly interpolated** in dB.

### 2.4 Frequency axis and ruler
- **Scale.** **Log** (default) or **Linear**, switched from the ruler's right-click menu or the
  spectral settings popover.
  - Log spans **20 Hz → Nyquist**; Linear spans **0 Hz → Nyquist**.
  - **Decided (autonomous, T-200): default Log.** At 48 kHz a linear axis spends half its height
    above 12 kHz, where a voice has little energy. On a log axis, 100 Hz–4 kHz (hum, plosives,
    formants, most of the intelligibility) fills about half of the pane.
- **Frequency zoom** (**Decided (autonomous, T-200)**, Audition offers the same on its ruler):
  - the wheel over the ruler zooms the visible frequency range around the pointer's frequency (√2 per
    notch);
  - dragging the ruler pans;
  - double-clicking resets to the full range;
  - the minimum span is 1 octave (Log) or 500 Hz (Linear);
  - the frequency range is per document and is not persisted.
- **Ruler labels.**
  - Log: `20 50 100 200 500 1k 2k 5k 10k 20k`, from a 1-2-5 ladder thinned so labels never overlap.
  - Linear: 1/2/5 × 10ⁿ Hz steps.
  - Format: "Hz" values below 1000, then "1k", "2.5k", "12k". The unit "Hz" appears once at the top of
    the ruler.
- **Pixel rows.** A pixel row that spans several bins shows the **maximum** of those bins, so a
  narrowband whine never disappears at small pane heights (AC-11). A row narrower than one bin
  interpolates linearly in dB between the two nearest bin centres.

### 2.5 Level mapping and colormap
- **Display floor and ceiling** are sliders in the spectral settings popover:
  - floor default **−120 dB**, range −150 … −30;
  - ceiling default **0 dB**, range −60 … +6;
  - ceiling − floor ≥ 20 dB.
  - Levels ≤ floor get the colormap's first color; levels ≥ ceiling get its last.
  - Changing them, the colormap or the frequency scale is **shader-only**: no IPC and no refetch
    (ADR-003 §2), and the result is visible in the next animation frame (AC-8).
  - **Decided (autonomous, T-200):** −120 dB puts the per-bin floor of a quiet booth (≈ −90 dB per
    bin for −65 dBFS RMS room noise at FFT 2048, §4.2) about a quarter of the way up the colormap:
    visible, but not glaring. A 16-bit dither floor (≈ −122 dB per bin) is black.
- **Colormaps.** **Decided (autonomous, T-200):**
  - **Inferno** (default: black → purple → orange → pale yellow, close to Audition's look and
    perceptually uniform);
  - **Viridis**;
  - **Grayscale** (black → white).

  Each is a 256-entry RGB LUT. Inferno and Viridis come from matplotlib's CC0-licensed colormap data
  (van der Walt & Smith). The popover shows a vertical color bar with the dB scale.
- **Window.** Audition's own spectral preferences default to a Blackman-Harris-family window. We keep
  **Hann**, as ADR-003's `VXST` specifies (`window = 0`). The field allows adding windows later without
  a format change.

### 2.6 FFT size
- **Choices.** **Auto** (default) or an explicit 256, 512, 1024, 2048, 4096, 8192 or 16384.
- **Auto rule.** The power of two nearest to `2048 × fs / 48 000`, clamped to 256 … 16384:
  - 44.1/48 kHz → **2048** (the task default at 48 kHz: 42.7 ms window, 23.4 Hz bins);
  - 88.2/96 kHz → 4096;
  - 176.4/192 kHz → 8192;
  - 22.05/24 kHz → 1024;
  - 8 kHz → 256.

  **Decided (autonomous, T-200):** Auto keeps time and frequency resolution the same across rates.
  An explicit choice is stored as given.
- **Texture limit.** Sizes whose bin count `N/2 + 1` exceeds the GPU's `MAX_TEXTURE_SIZE` are
  disabled, with the tooltip "Not supported by this graphics driver". Canvas2D has no such limit.
- **Changing the size** requests new tiles. The old image stays on screen until the replacement
  tiles arrive, so the pane never flashes blank.

### 2.7 Hover readout
- **Where.** Over the spectral pane, a small floating readout follows the pointer (12 px offset,
  flipping at the edges).
- **What it shows:**
  - **Time** in the current time-ruler format (SPEC-006 §2.5), for `sample(px)`.
  - **Frequency** at the pointer row, from the axis mapping: "1 007.8 Hz" below 10 kHz, "12.35 kHz"
    above.
  - **Level** of the nearest bin in the nearest frame, dequantized from the tile (§4.2) and shown with
    one decimal ("−20.3 dB"). Code 0 shows "≤ −150 dB" and code 255 "≥ +6 dB". It shows "—" where no
    tile has arrived yet.
- **Cost.** It updates every animation frame from tile bytes already in the UI. There is no IPC.
- **Accuracy.** The level is within ±0.31 dB (half a quantization step) of the frame's true bin
  level (AC-13).

### 2.8 Loading, caching and invalidation (user view)
- **Order.** The visible tiles are requested first, then one viewport's width on each side.
- **Zoomed out.** At overview hops (§4.4), a fast **preview** tile appears first and is replaced by
  the refined tile moments later.
- **While a tile loads,** the pane shows any cached tile of the same span at another hop or FFT size,
  stretched. Otherwise it shows the `--spec-pending` token (the spectral analogue of SPEC-006's
  `--wave-pending`).
- **After an audio edit** (`audio_rev` changes), stale tiles are dropped (ADR-003) and the visible
  set is requested again. Spans whose audio did not change are served from the content-addressed
  cache almost at once (§4.5).
- **Marker-only edits** change nothing here.
- **Memory.** The Rust tile cache counts against the memory budget (ADR-004 §4) and is capped at 25 %
  of it. The UI keeps at most 192 MiB of tile textures (LRU). Both evict silently; evicted tiles are
  recomputed on demand.

### 2.9 Live output spectrum analyzer
- **Where.** An **Analyzer** panel in the bottom dock, to the right of the meter bridge (PROMPT
  §3.6). It is shown by default and can be hidden with View → Analyzer. Its minimum width is 240 px.
  Visibility and response settings are persisted.
- **What it measures.** The **mono output signal** as it leaves the rack path. This is the ADR-002 §4
  `out` signal: rack output plus Dry monitoring, before sample-format conversion and channel
  duplication. It is the same tap as the output level meter. It therefore shows playback, "through
  rack" monitoring, and Dry monitoring: exactly what the user hears. When the output is idle
  (silence), the curve falls to the floor.
- **Display:**
  - **Averaged spectrum**, a filled curve on a **log frequency axis** from 20 Hz to min(Nyquist,
    24 kHz), in 1/24-octave bands (§4.8).
  - **dB axis**: default floor **−120 dB**, ceiling **0 dB**. The floor can be −150, −120, −100, −80 or
    −60; the ceiling 0 or +6.
  - Grid lines at 1-2-5 frequencies and every 12 dB.
  - The same sine-normalized dB as the spectrogram: a full-scale sine's band reads 0 dB.
- **Response.** **Fast / Medium / Slow** = exponential averaging in the power domain with
  τ = 50 / 150 / 500 ms. Default **Medium**.
  - **Decided (autonomous, T-200):** Medium follows speech syllables yet is steady enough to read an
    EQ change. Slow approximates a long-term voice spectrum. Fast is for catching clicks and plosives.
- **Peak hold.** On by default. A line holds each band's maximum for **2 s**, then falls at
  **12 dB/s**. Clicking the panel resets it; the panel menu toggles it.
- **Hover readout.** Frequency and averaged level at the pointer.
- **States:**
  - no output device → "No output device";
  - panel hidden and no subscriber → the analyzer is completely off (§4.8), at zero cost.

### 2.10 Reuse by the EQ graph [M4]
- **Same stream.** The Parametric EQ graph (T-409, PROMPT §2 "draggable graph with live spectrum")
  subscribes to the **same analyzer stream** (§4.9). It draws the spectrum behind the module's
  `ResponseCurve` (ADR-005 §11).
- **Fixed contract** (M4 may extend it but not change it):
  - one frequency-axis module `ui/src/lib/spectrum/freqAxis.ts`, parameterized by (f_lo, f_hi, scale),
    used by the analyzer panel, the spectral ruler and the EQ graph;
  - the band-centre list f_k (§4.8) can be passed as `freqs_hz` to `ResponseCurve::magnitude_db`, so
    curve and analyzer can share points;
  - the analyzer's levels are absolute dBFS. The EQ curve's gain axis (dB relative) is a separate
    vertical scale on the same x axis.
- **Tap point.** Where the analyzer taps when the EQ graph is open is an M4 decision. It is the rack
  output unless the M4 EQ spec requires a per-slot tap. A per-slot tap would need an ADR-002/ADR-003
  amendment and is not assumed here.

## 3. Parameters
| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `spectral_visible` | Show spectral pane | bool | on/off | off | toggle | Shift+D (verified Audition binding) |
| `split_ratio` | Waveform share of the editor height | % | 0 … 100 | 50 | continuous drag; double-click = 50 | 0/100 = collapsed strip |
| `fft_size` | FFT size | samples | Auto, 256 … 16384 (powers of 2) | Auto | list | Auto = pow2 nearest 2048·fs/48 000 |
| `window` | Analysis window | enum | Hann | Hann (periodic) | fixed | ADR-003 `window = 0` |
| `hop` | Frame hop | samples | powers of 2 ≥ fft_size/16 | from zoom (§4.3) | derived | not user-set |
| `freq_scale` | Frequency axis | enum | log / linear | log | toggle | log from 20 Hz |
| `freq_range` | Visible frequency range | Hz | ≥ 1 octave (log) / ≥ 500 Hz (linear) | full | wheel/drag on ruler | per document |
| `display_floor_db` | Display floor | dB | −150 … −30 | −120 | 1 dB | shader-only |
| `display_ceil_db` | Display ceiling | dB | −60 … +6 | 0 | 1 dB | ceil − floor ≥ 20 |
| `colormap` | Colormap | enum | inferno / viridis / gray | inferno | list | 256-entry LUTs |
| `q_floor_db`, `q_ceil_db` | Tile quantization range | dB | — | −150 / +6 | fixed | ADR-003; step 0.6118 dB |
| `tile_frames` | Frames per tile | frames | — | 256 | fixed | ADR-003 |
| `tile_cache_share` | Rust tile cache cap | % of memory budget | — | 25 | fixed | ADR-004 §4 |
| `ui_texture_cache_mib` | UI tile-texture cap | MiB | — | 192 | fixed | LRU |
| `spec_latency_ms` | Visible tiles after zoom/scroll (FFT ≤ 4096) | ms | — | ≤ 200 | fixed | 60-min document, cold cache |
| `an_fft_size` | Analyzer FFT size | samples | 4096 … 32768 | pow2 nearest 8192·fs/48 000 | derived | 8192 at 44.1/48 kHz |
| `an_window` | Analyzer window | enum | — | Blackman-Harris 4-term | fixed | SPEC-012 §4.3 coefficients |
| `an_bands_per_octave` | Analyzer resolution | bands/oct | — | 24 | fixed | f_k = 20·2^(k/24) Hz |
| `an_rate_hz` | Analyzer frame rate | Hz | {30, 60} | = `TELEMETRY_RATE` (60) | follows the setting | ADR-009 |
| `an_response` | Averaging | enum | fast / medium / slow (τ 50 / 150 / 500 ms) | medium | list | power-domain EMA |
| `an_floor_db` / `an_ceil_db` | Analyzer dB range | dB | floor {−150, −120, −100, −80, −60}; ceiling {0, +6} | −120 / 0 | list | |
| `an_peak_hold` | Peak hold | bool, s, dB/s | on/off; hold 2 s; fall 12 dB/s | on | toggle | UI ballistics |
| `an_ring_samples` | RT tap ring | samples | — | 32 768 | fixed | ≈ 0.68 s at 48 kHz |

## 4. Algorithm / implementation notes

### 4.1 Where things run
- **Tiles.** The **spectrogram tile service** is in `engine` (ADR-001 §4). It runs on the worker pool
  (ADR-002 §1) using `dsp`'s STFT (realfft, f32).
  - It reads samples through snapshot readers, never on the audio thread.
  - While a save, export or bake job runs, tile work uses at most half of the workers, so it never
    starves those jobs.
- **Analyzer.** The analyzer tap is in the output callback. Its consumer is an `engine` thread
  (§4.8).
- **UI.** The UI renders tiles with WebGL2, falling back to Canvas2D (ADR-009 §4, as in SPEC-006
  §2.13). The analyzer panel is small (≤ 246 points at 60 Hz) and uses Canvas2D.

### 4.2 Normalization and quantization (normative)
- **Frame spectrum.** For a frame centred at c, the input is `x[c − N/2 … c + N/2)`, with zeros
  outside `[0, len)`. The window is periodic Hann, `w[n] = 0.5 − 0.5·cos(2πn/N)`, so Σw = N/2.
  `X[k] = Σ w[n]·x[n]·e^{−j2πkn/N}` for k = 0 … N/2.
- **Level.** `L[k] = 20·log10(|X[k]| / (Σw/2))` dB: a full-scale sine centred on a bin reads **0 dB**.
  This is the same convention as SPEC-012 §4.3 (there with BH4).
  - Scalloping: a tone δ bins off-centre reads lower by Hann's scalloping loss. That is **−0.63 dB at
    δ = 1/3**, e.g. 1 kHz at 48 kHz with N = 2048 (bin 42.67); the worst case, δ = ½, is **−1.42 dB**.
  - DC and Nyquist bins of a constant offset A read `20·log10(2A)`.
- **Noise.** White noise of RMS σ has an expected per-bin power level of
  `20·log10(σ) + 10·log10(6/N)`, i.e. **−25.33 dB** relative at N = 2048. The spectrogram shows
  per-bin density, not band power.
- **Quantization** (ADR-003): `v = round(255 · clamp((L + 150)/156, 0, 1))`. The UI dequantizes with
  `L̂ = −150 + v·156/255` (step 0.6118 dB). −∞ (digital silence) → v = 0.

### 4.3 Frame grid, hop rule, tile geometry
- **Device-pixel zoom.** `spp_dev` = document samples per **device** pixel = SPEC-006's
  `samplesPerPixel` converted to device pixels (SPEC-006 §2.13 does its pixel math in device pixels).
- **Hop.** `hop = max(pow2_floor(spp_dev), N/16)`, a power of two. This gives 1–2 frames per device
  column when zoomed out, and a minimum hop of N/16 (93.75 % overlap) when zoomed in. Frames per
  visible request ≤ 2 × viewport device width + prefetch.
- **Frame grid.** It is anchored at document sample 0: frame i is centred at `c_i = i·hop`, for
  `0 ≤ c_i < len`. Tile k holds frames `[256k, 256k + 256)`, so
  `first_frame_center_sample = 256·k·hop`. Tiles are **frame-major, bin 0 = DC**, with
  `bins = N/2 + 1` (ADR-003 `VXST`).
- **Stable grid.** Anchoring at 0 with power-of-two hops keeps tile identity stable across scrolls
  and small zoom changes. The same tile is reused until the zoom crosses a power-of-two boundary.

### 4.4 Overview frames (hop > N): mean power, with a fast preview
- **Refined frame.** When `hop > N`, one plain FFT per hop would analyse only N of every `hop`
  samples. Instead, frame i's value is the **mean power** of `2·hop/N` Hann sub-frames. Their centres
  are `c_i − hop/2 + (j + ½)·N/2`, for j = 0 … 2·hop/N − 1: stride N/2, which is COLA for Hann, so
  every sample is covered evenly.
  `L[k] = 10·log10(mean_j |X_j[k]|² / (Σw/2)²)`.
- **Decided (autonomous, T-200): mean power, not maximum.** Stationary sounds (hum, hiss, room tone)
  then read the **same dB at every zoom level** (AC-5), and hover readouts stay comparable. Short
  transients are best inspected zoomed in, where frames are plain STFT.
- **Cost.** A refined whole-file overview of 60 min at N = 2048 is 168 750 FFTs, ≈ 1.7 s of one core.
- **Preview.** To meet the 200 ms target, the service first sends **preview** tiles for overview
  hops, computed from the single frame at `c_i`. It then sends the refined tiles for the same
  `tile_index`.
- **⚠ ADR-003 amendment (additive).** `VXST` gains flag **bit1 `PREVIEW`**: "this tile will be
  replaced by a refined tile of the same request and `tile_index`". The `LAST` flag marks the final
  tile of the request, after refinement. Readers that ignore bit1 still work.

### 4.5 Cache keys and invalidation
- **Key.** Per ADR-003, the key is `hash(runs, N, hop, window, preview, ALGO_VERSION)`. `runs` = the
  `(chunk_id, offset, len)` and silence runs covering the tile's input span `[first − N/2 (or hop/2),
  last + N/2 (or hop/2))`, clipped to `[0, len)`.
- **Position-free payload.** The payload does not depend on the tile's document position, only the
  header does. A cache hit therefore returns the payload with a fresh header (request_id,
  `audio_rev`, tile_index).
- **In-place edits** (normalize a range, silence) recompute only the tiles whose input span
  intersects the edited range (AC-8).
- **⚠ Note on ADR-003.** ADR-003 says "an edit invalidates only the tiles it touches". That holds for
  in-place edits only. A **length-changing** edit (cut, paste, insert silence) shifts every later
  frame relative to the grid anchored at 0. All later tiles then miss the cache, unless the shift is a
  multiple of 256·hop. This is acceptable because only visible tiles are recomputed first (AC-9 still
  holds). ADR-003's wording should be amended; the design does not change.
- **Service counters.** The service exposes counters for tests (`spectro_stats`: computed, cache
  hits, cancelled, bytes cached).

### 4.6 Requests and scheduling
- **Commands** (ADR-003): `spectro_attach(view_id, channel)` once per view, then
  `spectro_request(view_id, { request_id, audio_rev, fft_size, hop, window, tiles })`, with visible
  tiles first and at most 64 tiles per request.
- **Cancellation.** A newer `request_id` cancels unsent and uncomputed tiles of older requests.
  Workers check cancellation between frames.
- **Budget for the 200 ms target** (60-min document, 1920 device px, N = 2048, cold cache; owner
  machine, 8 workers):

  | Step | Cost |
  |---|---|
  | Frames to compute | ≤ 2 × 1920 = 3 840 ≈ 15 tiles, ≈ 40 ms of FFT work ÷ workers |
  | Channel transfer | 15 × 262 KB ≈ 3.9 MB at ≈ 80 MB/s (ADR-009 §3) ≈ 50 ms |
  | Texture uploads | ≤ 2 ms each |

  Larger FFT sizes send proportionally more bytes per tile, hence the relaxed targets in AC-9.

### 4.7 Renderer
- **Upload.** Each tile is an `R8` texture, width = frames, height = bins.
- **Fragment shader**, per pixel:
  1. Map the pixel's x to a frame (SPEC-006 §4.1) and its y to a frequency (below).
  2. Sample the tile(s), applying the §2.3 max/interpolate rule in time and the §2.4 rule in
     frequency. It may loop over up to 16 bins or frames per pixel, or use a max-reduced mip chain
     built at upload; T-207 chooses.
  3. Dequantize, normalize `t = clamp((L̂ − floor)/(ceil − floor), 0, 1)`, and look up the colormap
     LUT (a 256×1 texture).
- **Frequency mapping.** For a row at normalized height u ∈ [0, 1] (0 = bottom) within the visible
  range [f_a, f_b]:
  - Log: `f = f_a · (f_b/f_a)^u`, with f_a ≥ 20 Hz in log mode.
  - Linear: `f = f_a + u·(f_b − f_a)`.
  - Bin = f·N/fs. The inverse is used for ruler ticks and hover.
- **Canvas2D fallback.** It computes the same per-pixel values on the CPU into an `ImageData` for the
  visible area only. It is slower but visually identical (ADR-009 §2).
- **Overlays** use the SPEC-006 §2.12 tokens. New tokens: `--spec-bg`, `--spec-pending`,
  `--spec-ruler-text`, `--spec-ruler-grid`, `--analyzer-fill`, `--analyzer-peak`, `--analyzer-grid`.

### 4.8 Live analyzer pipeline
1. **RT tap (output callback).** After step 3.4 of ADR-002 §4, and only when the atomic
   `ANALYZER_ON` is set, each sub-block's mono `out` samples are pushed into an `rtrb` SPSC ring of
   32 768 f32.
   - If the ring lacks space, the samples that fit are written and an atomic drop counter is bumped.
   - The tap never blocks, allocates, locks or logs (ADR-002 §2).
   - With `ANALYZER_ON` clear, the tap does nothing.
2. **Consumer.** A dedicated `engine` thread runs at `TELEMETRY_RATE`, woken by the control tick or
   its own timer. It never blocks the control tick. Per frame, it:
   1. drains the ring into a history of the last N samples;
   2. applies BH4 (SPEC-012 §4.3 coefficients) and computes the realfft (f32);
   3. computes bin powers `|X[k]|²/(Σw/2)²`;
   4. reduces them to bands;
   5. applies the EMA per band;
   6. converts to dB and sends a `VXSA` frame to every subscriber.
3. **Bands.** `f_k = 20·2^(k/24)` Hz for k = 0 … K−1, with `f_{K−1} ≤ min(fs/2, 24 000)`: K = 246 at
   48 kHz and 243 at 44.1 kHz. Edges are `f_k·2^(±1/48)`.
   - Band power = the **maximum** bin power with centre in `[lo, hi)`, so a sine reads its bin level.
   - When no bin centre falls inside (low bands at small N), the value is the dB-linear interpolation
     of the two bins bracketing f_k.
4. **Averaging.** `P ← P + α·(p − P)`, with `α = 1 − exp(−(1/rate)/τ)`. `L = 10·log10(P)`, and 0 →
   −∞. NaN is impossible, because the rack's non-finite guard upstream is SPEC-012 §2.9.
5. **Accuracy.** BH4 scalloping is ≤ 0.83 dB (worst at δ = ½). 1 kHz at 48 kHz / N = 8192
   (δ = 1/3 bin) reads **−0.37 dB** below the tone level. White noise per-bin level =
   σ_dB − 30.09 dB at N = 8192.
6. **Reset.** An output-device reopen or rate change clears the history and EMA and sets `RESET` on
   the next frame. The UI then clears its peak hold.
7. **Peak hold** is UI ballistics, computed per animation frame from the received frames (like the
   meters, ADR-003 §1).
8. **Calibration** (simulation for this spec, BH4 N = 8192 at 48 kHz, 60 Hz frames; the tone step
   covers window fill + EMA):

   | Response | Rise to −1 dB of final (s) | Fall by 20 dB after stop (s) |
   |---|---|---|
   | Fast | 0.18 | 0.30 |
   | Medium | 0.33 | 0.77 |
   | Slow | 0.88 | 2.37 |

   AC-16 adds margin for frame phase.

### 4.9 `VXSA` — analyzer frame (new magic; ADR-003 §2 header convention)
| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXSA"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 48 |
| 8 | u32 | seq |
| 12 | u32 | flags: bit0 `RESET` (history/averaging restarted), bit1 `DROPPED` (tap samples dropped since the previous frame), bit2 `SILENT` (the window is digital silence) |
| 16 | u64 | frame_time_ns (app clock at computation) |
| 24 | u32 | sample_rate_hz (device rate) |
| 28 | u32 | fft_size |
| 32 | f32 | f0_hz = 20.0 |
| 36 | u32 | bands_per_octave = 24 |
| 40 | u32 | band_count K |
| 44 | u32 | response (0 fast, 1 medium, 2 slow) |
| 48 | f32[K] | averaged band level, dB (−∞ allowed, never NaN) |

- **Commands:**
  - `analyzer_subscribe(channel, response)`, where each subscriber (analyzer panel, EQ graph) gets its
    own channel from one computation;
  - `analyzer_set_response(response)`;
  - `analyzer_unsubscribe(channel)`.

  `ANALYZER_ON` is set while at least one subscriber exists.
- **Rate.** `TELEMETRY_RATE` (60 Hz default, 30 selectable; ADR-009). ADR-003 §1's "30 Hz" row for
  the analyzer predates ADR-009 (see §5 flag).
- **Contract fixture.** `gen_ipc_fixtures` gains a golden `VXSA` frame, making it part of SPEC-000
  AC-7's Rust↔TS contract.

### 4.10 Real-time and resource notes
- The only RT-side cost is one bounded `memcpy` into a ring per sub-block while subscribed.
- Everything else runs on non-RT threads.
- Tile computation never touches the audio thread and never holds an `Arc<DocSnapshot>` on it
  (ADR-002 §2).
- The tile cache is LRU-evicted inside the memory budget (SPEC-004 §2.4, AC-5 accounting includes it).

## 5. Acceptance criteria
- **AC-1 [M2] (level and frequency accuracy).** Given a 48 kHz document with 10 s of sine at
  −20 dBFS, at FFT 2048 (Hann), for every frame not touching the document edges:
  - at **1 007.8125 Hz** (bin 43): the peak bin index is exactly 43 and the dequantized level is
    **−20.00 ± 0.35 dB**;
  - at **1 000 Hz**: the peak bin is **43** (round(42.67)) and the level is **−20.63 ± 0.35 dB**
    (Hann scalloping −0.627 dB at δ = 1/3);
  - for every FFT size 256 … 16384, a bin-centred tone at bin round(1000·N/48 000) reads
    **−20.00 ± 0.35 dB** in that bin;
  - a full-scale bin-centred sine reads **0.00 ± 0.35 dB**.
- **AC-2 [M2] (noise convention).** Given seeded white noise at −20 dBFS RMS (10 s, 48 kHz, FFT 2048),
  the mean **power** of the dequantized bins between 100 Hz and 20 kHz, over all interior frames, is
  **−45.33 ± 0.5 dB** (= −20 + 10·log10(6/2048)).
- **AC-3 [M2] (dynamic range and extremes).**
  - Digital silence gives code 0 in every bin, and the hover shows "≤ −150 dB".
  - A bin-centred tone at −120 dBFS reads −120 ± 1.0 dB.
  - A float document with a bin-centred +3 dBFS sine reads +3.00 ± 0.35 dB.
  - A +10 dBFS sine gives code 255 ("≥ +6 dB").
  - No tile ever carries a value derived from a non-finite sample.
- **AC-4 [M2] (time alignment with the waveform).** Given an impulse at sample 240 000 of a 10 s
  document:
  - for every hop from N/16 to 65 536, the frame with the highest mean level is centred within
    ±hop/2 of 240 000;
  - in the UI (Vitest, shared coordinate module), that frame's column is drawn at `px(c_i)`, which is
    within ±max(1, hop/spp_dev/2) device px of the waveform's `px(240 000)`.
- **AC-5 [M2] (zoom-consistent levels).**
  - Given 60 s of a bin-centred 1 007.8125 Hz −20 dBFS sine, the level of bin 43 is −20.0 ± 0.5 dB at
    every hop from N/16 to 65 536, for preview and refined tiles alike.
  - Given 60 s of seeded pink noise, the mean power level over 100 Hz–10 kHz differs by ≤ 0.5 dB
    between hop 512 and refined hop 65 536.
- **AC-6 [M2] (hop rule and tile geometry).**
  - For a sweep of `spp_dev` from 0.1 to 2 × 10⁵ at each FFT size: hop = max(pow2_floor(spp_dev),
    N/16); frames per viewport ≤ 2 × device width; tile k has `first_frame_center_sample = 256·k·hop`,
    `frames ≤ 256`, `bins = N/2 + 1`, `window = 0`, `q_floor/q_ceil = −150/+6`.
  - Golden `VXST` frames, including one with `PREVIEW` and one with `LAST`, decode in Vitest with every
    field equal to its known value (SPEC-000 AC-7).
- **AC-7 [M2] (determinism and tile hash stability).**
  - The same tile requested twice, with 1 and with 8 workers, before and after cache eviction, has an
    identical payload FNV-1a hash.
  - A fixed synthetic tile (bin-centred sine + seeded noise) matches its checked-in golden hash on
    x86_64. On other architectures, codes may differ by at most 1 step (±0.61 dB) in ≤ 0.1 % of cells
    (SIMD rounding), and the golden test allows exactly that.
- **AC-8 [M2] (caching and invalidation).** Given a 60-min document with the whole-file view loaded:
  - a marker add/rename/delete sends **0** `spectro_request`s and computes 0 tiles;
  - silencing [30:00, 30:01) recomputes exactly the tiles whose input span intersects that range, and
    every other tile of the re-request is a cache hit (`spectro_stats`);
  - after a cut at 30:00, every tile whose span ends before 30:00 − N/2 is a cache hit;
  - a response carrying a stale `audio_rev` is dropped by the UI;
  - changing floor, ceiling, colormap or frequency scale issues **0** IPC calls and is visible in the
    next animation frame.
- **AC-9 [M2] (latency).** Given the 60-min 48 kHz fixture, a 1920-device-px spectral pane and a cold
  cache, on the owner's machine, the time from `spectro_request` until every visible tile (preview
  allowed at overview hops) has been received in the UI is:
  - **≤ 200 ms** for FFT ≤ 4096 at every zoom level, including whole-file;
  - ≤ 600 ms at 8192 and ≤ 1 000 ms at 16384;
  - refined whole-file tiles complete ≤ 2.0 s after the request;
  - with a warm cache, ≤ 50 ms.

  The engine-side part (compute + channel send into a sink) is an integration test. The end-to-end
  figure is a bench on the owner's machine.
- **AC-10 [M2] (frame time, split view).** Given the split view (waveform + spectral, 50/50) on the
  60-min fixture with tiles cached, during a scripted 10 s zoom/scroll sweep (ADR-009 methodology,
  `WEBKIT_DISABLE_DMABUF_RENDERER=1`): p50 ≤ 16.7 ms, p99 ≤ 50 ms, and at most 1 frame > 50 ms. This is
  the same tolerance as SPEC-006 AC-18, for the same reason.
- **AC-11 [M2] (axis mapping and row maximum)** (Vitest, pure functions):
  - log and linear `f(u)` are monotonic; y → f → y round-trips within 0.01 px; 20 Hz maps to the
    bottom and Nyquist to the top in log mode;
  - ruler labels never overlap at pane heights 80–2000 px;
  - given a tile with a single hot bin at 15 kHz (N = 2048, 48 kHz) and a 300 px pane in log mode, some
    row shows that bin's value exactly (row max), and in linear mode likewise.
- **AC-12 [M2] (toggle, split, sync)** (Vitest + manual):
  - Shift+D shows and hides the spectral pane;
  - the default ratio is 50 %; the divider clamps to 0–100 %; double-click resets to 50 %;
  - visibility, ratio and display settings survive a restart;
  - a Ctrl+wheel zoom or scroll over either pane changes the shared `startSample`/`samplesPerPixel`
    and both panes redraw in the same animation frame;
  - the selection and markers appear in both panes at identical x;
  - during import and during recording, the pane shows its disabled or frozen message and issues no
    requests.
- **AC-13 [M2] (hover readout)** (Vitest over a tile fixture):
  - with the pointer on the row of 1 007.8 Hz, the frequency text matches the axis mapping to within
    one pixel row's frequency span;
  - the level text equals the dequantized code of the nearest bin in the nearest frame, exactly, to
    0.1 dB;
  - the time text equals SPEC-006's formatting of `sample(px)`;
  - codes 0 and 255 show "≤ −150 dB" and "≥ +6 dB".
- **AC-14 [M2] (FFT size defaults and limits).**
  - Auto resolves to 2048 at 44.1 and 48 kHz, 4096 at 96 kHz, 1024 at 22.05 kHz and 256 at 8 kHz.
  - With `MAX_TEXTURE_SIZE` stubbed to 8192, the 16384 option is disabled with its tooltip.
  - Changing the FFT size keeps the previous image on screen until the new tiles arrive (no frame
    painted entirely `--spec-pending` for an area previously shown).
- **AC-15 [M2] (analyzer level and frequency).** Given the fake backend at 48 kHz playing a 1 kHz
  −20 dBFS sine through an empty rack, Medium response, after 2 s:
  - the band containing 1 kHz is the maximum band and reads **−20.37 ± 0.2 dB** (BH4 at δ = 1/3 bin);
  - a bin-centred tone (1 001.953 Hz, bin 171) reads **−20.00 ± 0.2 dB**;
  - with a +6 dB Gain in the rack, the reading rises by 6.00 ± 0.05 dB, which proves the tap is
    post-rack;
  - with Dry monitoring of a −20 dBFS input, the monitored tone appears as well.
- **AC-16 [M2] (analyzer response time).** Given a step from digital silence to the −20 dBFS sine
  (fake backend; time measured from the first tone sample entering the tap), the 1 kHz band reaches
  within 1 dB of its final value within **0.25 s (Fast)**, **0.40 s (Medium)** and **1.0 s (Slow)**.
  After the tone stops, it falls by ≥ 20 dB within **0.40 s**, **0.85 s** and **2.6 s** respectively.
  This holds at 60 Hz and, with `TELEMETRY_RATE` = 30 Hz, within +34 ms.
- **AC-17 [M2] (peak hold)** (Vitest, ballistics over synthetic frames):
  - a band peak is held for 2.0 s ± 1 frame, then falls at 12.0 ± 0.1 dB/s;
  - a click or a `RESET` frame clears the hold;
  - with peak hold off, no line is drawn.
- **AC-18 [M2] (real-time safety and cost).**
  - With the analyzer subscribed, SPEC-000 AC-2's scripted session reports 0 allocations and 0
    deallocations in the callbacks, and the tap's drop counter stays 0 at buffer sizes 64–2048.
  - The analyzer consumer uses ≤ 2 % of one core at 60 Hz, N = 8192 (bench).
  - Unsubscribed, the tap writes nothing and no `VXSA` frames are sent.
  - An output-device reopen produces one frame with `RESET`.
- **AC-19 [M2] (analyzer IPC contract).**
  - The golden `VXSA` fixture decodes in Vitest with every header field and band value bit-identical.
  - Live frames arrive at `TELEMETRY_RATE` ± 5 % over 10 s.
  - `band_count` = 246 at 48 kHz and 243 at 44.1 kHz, and band k's centre is 20·2^(k/24) Hz.
  - Silence frames carry −∞ (not NaN) and set `SILENT`.
- **AC-20 [M4] (EQ graph reuse).** Given the EQ graph (T-409) open with an EQ `ResponseCurve`, then:
  - it subscribes to the same `VXSA` stream through `analyzer_subscribe`;
  - the analyzer band k and the curve point evaluated at f_k are drawn at the same x within 0.5 device
    px, over the full graph width, through the shared `freqAxis` module;
  - closing the graph unsubscribes. If the analyzer panel is also hidden, the tap turns off
    (`ANALYZER_ON` clear).

## 6. Test plan
| AC | Unit (Rust) | Integration | Vitest | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | `dsp` STFT + quantizer on synthetic tones, all FFT sizes | tile service over a synthetic snapshot | — | view `sine-1khz-minus20dbfs-20s.wav`, hover the peak |
| AC-2 | per-bin noise power over seeded white noise | — | — | — |
| AC-3 | quantizer extremes, silence, float overs | — | readout strings for codes 0/255 | — |
| AC-4 | frame-centre math, impulse fixture | — | column x vs waveform `px()` (shared coords) | zoom on a click in a real take, both panes |
| AC-5 | overview mean-power vs plain frames; preview vs refined | whole-file request on a 60 s fixture | — | zoom from whole file to 1 s on a hum-y file |
| AC-6 | hop rule sweep; tile header builder | — | golden `VXST` decode incl. `PREVIEW`/`LAST` | — |
| AC-7 | payload hash across worker counts/eviction | golden tile hash | — | — |
| AC-8 | cache key over piece-table edits | `spectro_stats` after marker/in-place/cut edits | stale `audio_rev` drop; 0 IPC on display changes | edit while the spectral pane is open |
| AC-9 | — | engine timing into a channel sink (60-min fixture) | — | bench on the owner's machine (`just bench` / spike-style harness) |
| AC-10 | — | — | frame-time harness (ADR-009 watchdog pattern) | `just spike`-style sweep with the split view |
| AC-11 | — | — | `freqAxis` mapping, tick thinning, row-max function | resize the pane small; check a 15 kHz whine stays visible |
| AC-12 | — | — | Shift+D, divider, shared viewport, disabled states (mockIPC) | toggle, drag, zoom both panes; record with the pane open |
| AC-13 | — | — | hover readout over a tile fixture | hover known tones |
| AC-14 | — | — | Auto table, texture-limit stub, no-blank FFT change | switch FFT sizes on a long file |
| AC-15 | band reduction + scalloping | fake backend playback, Gain in rack, Dry monitoring | — | play a test tone; compare with a known meter |
| AC-16 | EMA + window step simulation | fake backend step on/off, both telemetry rates | — | play speech; watch it follow syllables |
| AC-17 | — | — | peak-hold ballistics, reset, toggle | click the analyzer to reset |
| AC-18 | tap ring under `assert_no_alloc` | SPEC-000 AC-2 script with the analyzer on; bench CPU | — | watch `top` with the analyzer visible vs hidden |
| AC-19 | `VXSA` builder, band table | frame-rate check over 10 s | golden `VXSA` decode | — |
| AC-20 [M4] | — | subscribe/unsubscribe lifecycle (T-409) | curve/analyzer x agreement | open the EQ, drag a band over live speech |

**Fixtures.** testkit signals generated in test code: bin-centred and 1 kHz sines, seeded white and
pink noise, impulses, 60 s sines. The long fixture is `just fixtures`' 60-min WAV. Golden `VXST`/`VXSA`
frames come from `gen_ipc_fixtures` (ADR-003 §4). No audio files are committed.

## 7. Out of scope
- **Spectral editing** and frequency-range selection (PROMPT §3.8); spectral repair.
- **Display options:** windows other than Hann for tiles; reassigned or multitaper spectrograms; mel
  or other scales beyond log/linear; 3-D or waterfall views; pitch display.
- **Recording:** a live spectrogram while recording (frozen by design, §2.1).
- **Analyzer extras:** a separate analysis of the recording input (the input meter covers pre-record
  levels, SPEC-002); analyzer freeze, snapshot or reference-curve comparison; phase or correlation
  meters.
- **M4 items:** the EQ graph's layout, handles and per-slot tap (M4 EQ spec, T-409); an ADR amendment
  if a per-slot tap is needed.

## 8. Amendment H-42 (2026-09-15) — analyzer diagnostics
- **Status:** approved (owner request H-42, "be creative"). Where the ticket is silent, the values
  below are engineering defaults chosen for voice-over (marked *default*), not standards.
- **Supersedes** §7's exclusion of "analyzer freeze, snapshot or reference-curve comparison".
- **Look:** §2.9's analyzer is unchanged. Everything added here is an overlay or a panel that can be
  turned off (peak labels default on, diagnostics panel default off).

### 8.1 Where it lives
- **Analyzer header** (dock): a **Live / Average / Compare** segmented control (default Live, not
  persisted); a **Peaks** toggle (labels, markers, note in the crosshair; default on); a
  **Diagnostics** toggle (the voice-statistics panel; default off); a button that opens the Spectrum
  Inspector. Fast/Medium/Slow and Peak hold are hidden in Average mode. Both toggles and the
  Inspector settings persist in `Settings.analyzer_diagnostics` (additive, settings version 1).
- **Mode bar** under the header: Average — signal (Processed *default* / Source), Analyze / Analyze
  again, progress + Cancel, "Selection · 12.3 s"; Compare — Freeze A, Freeze B, Source vs Processed,
  Clear. A legend (A / B / room tone) sits in the band above the plot.
- **Diagnostics panel:** to the right of the plot inside the analyzer panel (288 px, min 220 px),
  scrolling; also in the Inspector.

### 8.2 Peak labels, markers, crosshair
- Picked on the displayed curve, power-smoothed with τ = 0.4 s (*default*) so labels stay readable,
  re-picked at most every 100 ms.
- A peak is a local maximum ≥ display floor + 6 dB, within 20 Hz … the visible upper edge, with a
  **prominence** ≥ 6 dB (its level minus the higher of the lowest points within ±1/3 octave on either
  side), at least **1/6 octave** from every louder peak kept. The **5** loudest are labelled
  (*defaults*). Frequency and level come from parabolic interpolation of the three points around the
  maximum (geometric in frequency).
- Label: two lines — frequency ("220 Hz", "6.31 kHz"), then the **note with cents** and the level
  ("A3 +12¢ · −18.4 dB"). Notes: equal temperament, A4 = 440 Hz, scientific pitch notation (C4 =
  261.63 Hz), sharps, cents in −50 … +50.
- **Placement:** above, above-right, above-left, right, left, then below the marker — the first spot
  inside the plot that is ≥ 3 px clear of every placed label and of the hover readout. Labels are
  placed loudest first; one that fits nowhere is dropped (its marker still shows).
- **Markers:** a dot on each labelled peak, and a tick that holds each peak's level for 1.5 s, then
  falls at 6 dB/s (*defaults*; independent of §2.9's per-band peak hold).
- **Crosshair:** vertical and horizontal lines at the pointer; the readout adds the note
  ("1 007.8 Hz · −23.4 dB · B5 −12¢"), and in Compare "B − A +2.1 dB".
- **Resolution:** Live draws the 1/24-octave bands, so its peak frequencies are band estimates
  (≈ ±15 cents). The Average curve and the Inspector are FFT bins (bin-accurate after interpolation).
- **Keyboard:** the plot is focusable (`role="application"`): ←/→ pan 10 %, +/− zoom by √2, 0/Home
  reset. Its accessible name lists the labelled peaks.

### 8.3 Spectrum Inspector
- **Open:** View → Spectrum Inspector (checkbox), or the analyzer's button. A non-modal floating
  window (`role="dialog"`, `aria-modal="false"`: app shortcuts keep working): drag by the title bar,
  resize from the corner (min 560 × 360 px), Escape closes. First opened centred, ≤ 980 × 600 px.
- **Sources:** Live, Average, Snapshot A, Snapshot B. Other snapshots and the room tone draw over the
  chosen source.
- **Controls** (persisted): FFT size 1 024 … 32 768 (default 16 384); window Hann (default),
  Blackman-Harris 4-term (SPEC-012 §4.3), flat-top (SR785 / MATLAB `flattopwin`), rectangular;
  **smoothing** none (default), 1/3, 1/6, 1/12 octave — the power mean over a rectangular 1/n-octave
  window; **axis** log (default) or linear; response Fast/Medium/Slow (Live); Peaks. Zoom: wheel,
  drag to pan, **Shift-drag to zoom to a range**, double-click or "Reset zoom". dB axis −140 … 0.
  The title bar shows the resolution ("2.93 Hz per bin · 341 ms window").
- **Peak table:** the labelled peaks (frequency to 0.1 Hz / 1 Hz, note, level).
- **Freeze A / Freeze B** from whatever is shown.
- **Export CSV…:** the displayed curve (after smoothing) inside the visible range; header
  `frequency_hz,level_db`, one row per point, "." decimals, ASCII "-", an empty level cell for −∞;
  written through `spectrum_export_csv` (atomic, `.csv` added when missing, ≤ 64 MiB).
- **Live stream:** its own engine subscription (`VXIS`, §8.9) at the chosen FFT size / window /
  response, only while the Inspector is open on Live. Frames every 2nd publish (30 Hz at 60 Hz),
  power EMA with the response's τ. After a silent window's average falls below −150 dB the stream
  sends one last frame and then nothing until sound returns.

### 8.4 Spectrum conventions (normative)
- Every H-42 spectrum is **sine-normalized**: `P[k] = |X[k]|² / (Σw/2)²`, so a bin-centred full-scale
  sine reads 0 dB with every window.
- **Band power** = `Σ P[k] / ENBW` over the bins whose centre lies in `[lo, hi)`: a sine of amplitude
  `A` reads `A²`, noise of RMS `σ` reads `2σ²` (its RMS dBFS + 3.01 dB). ENBW (periodic windows):
  Hann 1.5, Blackman-Harris 2.004, flat-top 3.770, rectangular 1.0 bins. Every diagnostic is a ratio
  of band powers, hence independent of FFT size and window.
- White noise per bin reads `σ_dB + 10·log10(4·ENBW/N)` (generalizes §4.8.5).

### 8.5 Long-term average (Average mode, the Inspector's Average source)
- **Scope:** the selection, else the whole file. **Signal:** processed (the live rack, rendered like the
  loudness job, S4-01) or source. Compare's **Source vs Processed** analyses both in one job and
  freezes them as A (source) and B (processed).
- **Job:** `spectrum_analyze_start` → a Rust thread, off the audio thread; `job_progress`
  (`spectrum_analyze`, ≤ 10 Hz), `spectrum_analyze_cancel`; the error notice precedes a terminal
  `failed` (H-30); a `spectrum_report` event carries each signal's diagnostics; curves are then
  fetched as binary `VXLT` frames (`spectrum_analyze_curve(job_id, index)`; the last 4 jobs kept).
- **LTAS** = the mean sine-normalized power over Welch frames (the Inspector's FFT size and window,
  hop N/2). A selection shorter than N is analysed as one zero-padded frame. The **room-tone curve** is
  the same mean over the quiet frames (§8.7 gating), drawn dashed.

### 8.6 Fundamental frequency (F0)
- **YIN** (de Cheveigné & Kawahara 2002) with the difference function from an FFT cross-correlation:
  50 … 1 000 Hz, integration window W = τ_max = fs / 50, absolute threshold 0.15, parabolic
  interpolation of d′. Unvoiced when the frame RMS < −60 dBFS or the aperiodicity d′(τ*) > 0.35.
- Live: every 10 ms; offline: every 20 ms. Statistics over the voiced frames: **median**, **10th–90th
  percentile** (the "range"), **voiced fraction** (voiced / frames above the energy gate). Live
  statistics cover the last **10 s**; "now" is the median of the voiced frames of the last 0.3 s
  (shown only while voiced). Shown as note and Hz; descriptive only — no voice-type labels.

### 8.7 Spectral diagnostics
- **Diagnostics spectrum** (independent of the display settings): Hann, FFT = the power of two
  nearest 0.34 s, clamped to 8 192 … 32 768 (16 384 at 44.1/48 kHz: 2.9 Hz bins, enough to tell 50
  from 60 Hz hum). Live: one frame per 100 ms of new audio; offline: hop N/2.
- **Gating** by the frame's RMS: ≥ noise floor + 15.9 dB → the **voice** spectrum; ≤ noise floor +
  6 dB → the **room-tone** spectrum; digital silence → neither. Live averages have τ = 3 s (a running
  mean until then); the room-tone average restarts when the noise floor drops by > 3 dB (before the
  first pause the "floor" is the quietest stretch of the voice). Offline: plain means.
- **Tone balance** (voice spectrum): per-octave density relative to the 1 kHz octave (707–1 414 Hz),
  pink noise = 0 dB — **mud/boxiness 200–500 Hz**, **presence 2–5 kHz**, **air 10–16 kHz** (a band
  needs at least half an octave below Nyquist).
- **Sibilance** (voice spectrum): **4–10 kHz** relative to the overall level (20 Hz … min(20 kHz,
  Nyquist)). **De-esser target** = the geometric centre of the −3 dB span around the maximum of the
  1/3-octave-smoothed spectrum within 4–10 kHz (1/48-octave grid).
- **Rumble** (voice spectrum): **20–80 Hz** relative to the overall level.
- **Hum** (room-tone spectrum; needs bins ≤ 4 Hz): harmonics 1–8 of **50 and 60 Hz**. A line's level
  is the band power within ±max(2 bins, 0.6 %); its **prominence** is its peak bin over the median of
  the bins 0.2–0.5 × mains away on either side. Prominent at ≥ 10 dB; hum when ≥ 2 harmonics are
  prominent, or one of the first three is ≥ 20 dB. The mains frequency with the larger summed
  prominence wins; the "strongest" line is the most prominent one (where the notch goes).
- Relative values are clamped to ±60 dB; below −90 dB overall nothing is diagnosed.

### 8.8 Levels, noise floor, SNR
- 10 ms blocks of mean square; blocks of exact digital silence are ignored.
- **Noise floor** = the ACX definition (the RMS of the quietest 500 ms window, unweighted), evaluated
  on the 10 ms grid; live over the last **30 s**.
- **Active level** = the power mean of the blocks ≥ noise floor + 15.9 dB (ITU-T P.56's margin,
  simplified: no hangover). **SNR** = active level − noise floor.
- `span_s` = seconds of non-silent audio covered (live: within the last 10 s) — constant during
  silence, so an idle report doesn't change.

### 8.9 Wire formats, commands, events
- `VXIS` (Inspector frame, little-endian): `"VXIS"`, u16 version 1, u16 header_len 40, u32 seq, u32
  flags (bit0 RESET, bit1 SILENT), u32 sample_rate_hz, u32 fft_size, u32 window (0 Hann,
  1 Blackman-Harris, 2 flat-top, 3 rectangular), u32 bin_count, u32 response (0/1/2), u32 reserved,
  then `f32[bin_count]` averaged levels (dB, −∞ allowed, never NaN). Bin k at k·fs/fft_size.
- `VXLT` (long-term average curve): `"VXLT"`, u16 version 1, u16 header_len 36, u32 job_id, u32
  index, u32 sample_rate_hz, u32 fft_size, u32 window, u32 bin_count, u32 flags (bit0 HAS_NOISE),
  then `f32[bin_count]` levels and, with HAS_NOISE, `f32[bin_count]` room-tone levels.
- Golden fixtures `vxis_fixture.ts` / `vxlt_fixture.ts` are generated by `just gen-types` (SPEC-000
  AC-7 contract).
- Commands: `analyzer_voice_subscribe(channel)` → id (JSON `VoiceReportDto` ≈ 10 Hz, only when
  changed); `analyzer_inspector_subscribe(channel, config)` → id; `analyzer_inspector_configure(id,
  config)`; `analyzer_unsubscribe(id)` removes a subscriber of any kind (`ANALYZER_ON` stays set
  while any exists); `spectrum_analyze_start(request)` → `{job_id}`; `spectrum_analyze_cancel`;
  `spectrum_analyze_curve(job_id, index)` → `VXLT`; `spectrum_export_csv(path, contents)` → path.
- Event `spectrum_report` (`SpectrumReportDto`); `JobKind::SpectrumAnalyze`.

### 8.10 Findings and "Add EQ band here"
- Hints (plain language, i18n) and severities (ok / info / warn, each with a labelled status dot —
  never colour alone):

  | Finding | Measure | Threshold (*default*) | EQ move offered |
  |---|---|---|---|
  | Mud | tone mud | > +9 dB warn "a bit boomy", > +6 info, < −6 info "thin" | peak cut 300 Hz, −3 dB, Q 1.4 |
  | Presence | tone presence | < −14 warn "dull", > −2 warn "forward/harsh" | boost 3.5 kHz +2.5 dB Q 1 / cut 3.5 kHz −3 dB Q 2 |
  | Air | tone air | < −35 info "little air", > −12 info "bright" | boost 12 kHz +3 dB Q 0.7 (low air) |
  | Sibilance | 4–10 kHz vs overall | > −12 warn, > −22 info | copies the de-esser target (no de-esser module yet) |
  | Hum | §8.7 | detected → warn | notch at the strongest line, −20 dB, Q 20 |
  | Rumble | 20–80 Hz vs overall | > −25 dB warn | high-pass 80 Hz |
  | Noise floor | §8.8 | > −60 dBFS warn (ACX) | — |
  | SNR | §8.8 | < 30 warn, < 40 info | — |

  A typical voice reads about +3 (mud), −7 (presence), −22 dB (air), −15 … −25 dB sibilance.
- **Add EQ band here** goes through the normal rack commands (rack changes are outside the undo
  history, SPEC-004 OD-1 — the user undoes it in the rack): the first `org.powervoice.parametric-eq`
  slot, or a new one appended with `rack_add`; then `param_set_plain` per parameter (SPEC-015 §3
  ids) in the order frequency, Q, gain, on. High-pass → the HP band (on, frequency). Notch/cut/boost →
  a **free** peak band (gain within 0.05 dB of 0), the one whose frequency is nearest the target; with
  no free band nothing changes and a notice says so. A notice confirms the band used.
- The de-esser action copies the rounded frequency in Hz to the clipboard.

### 8.11 Performance and idle behaviour
- Live diagnostics run on the engine's control tick beside the analyzer, only while a voice subscriber
  exists; during silence YIN stops at its energy gate and silent spectrum frames skip the FFT; reports
  go out only when they changed. Bench (`crates/dsp/benches/diagnostics.rs`, T-110 convention):
  `diagnostics_live_per_frame` ≤ 333 µs per 60 Hz frame (AC-18's 2 % of a core), plus informational
  `inspector_frame_fft{N}` and `ltas_offline_realtime_factor`.
- The plot draws **on demand** (one animation frame per changed input: new curve, hover, zoom, toggle,
  resize, theme), keeps animating only while the peak hold or markers are falling, and skips silent
  curves that are already off the axis. `SpectrumPlot.requestDraw()` is the hook for H-43's scheduler.

### 8.12 Acceptance criteria (H-42)
- **AC-H42-1** Peak picking finds synthetic multi-sine peaks within 0.05 Hz / 0.01 dB (interpolated),
  loudest first, respects the 1/6-octave spacing and the count (`peaks.test.ts`).
- **AC-H42-2** Note naming: 440 Hz = A4 ±0¢, 220·2^(12/1200) = A3 +12¢, cents stay in ±50
  (`notes.test.ts`).
- **AC-H42-3** F0 within 2 cents on sines (44.1/48/96 kHz), 3 cents on harmonics 2–8 of 200 Hz
  (missing fundamental), 25 cents median on a formant-shaped voice with vibrato, no octave errors
  (`pitch.rs`).
- **AC-H42-4** Sibilance centre within 1/6 octave of 6.32 kHz on 5–8 kHz band-limited noise and
  1/12 octave of 7 kHz on a narrow band (`features.rs`).
- **AC-H42-5** Hum at 50 and 60 Hz with harmonics is detected with the right mains frequency,
  harmonic mask and line level (±1 dB); noise and a 440 Hz tone are not hum; 5.9 Hz bins refuse
  (`features.rs`); from a voice session, hum is found in the pauses, live and offline (`voice.rs`).
- **AC-H42-6** LTAS equals the average of its frames bit-for-bit, reads −3.01 dB for a tone present
  half the time, and white noise sits at σ + 10·log10(4·ENBW/N) ± 0.2 dB (`spectrum.rs`).
- **AC-H42-7** "Add EQ band here" issues `rack_add` (when needed) then `param_set_plain` with the
  SPEC-015 ids and values (`eqSuggest.test.ts`, `DiagnosticsPanel.test.ts`).
- **AC-H42-8** Toggles, keyboard (plot pan/zoom/reset, Inspector Escape) and label collision
  (`SpectrumPlot.test.ts`, `peakLabels.test.ts`, `AnalyzerPanel.test.ts`, `SpectrumInspector.test.ts`).
- **AC-H42-9** Engine: voice reports follow the output (F0 of a 200 Hz tone ±1 Hz), unchanged
  reports are not resent, Inspector frames read a bin-centred flat-top tone within 0.1 dB, reconfigure
  and unsubscribe work, silence idles the stream, and the output callback never allocates
  (`crates/engine/tests/analyzer.rs`).

## Amendment 1 — H-79 selection colour (2026-09-16, owner-reported, autonomous)

§4.7's overlays share SPEC-006 §2.12's tokens, so SPEC-006's Amendment 2 (selection/wave colour
collision) applies here too: `--wave-selection-fill`/`--wave-selection-handle` move to a
pink/magenta hue, and the selection now draws a boundary line at each edge (previously fill-only).
Unlike the waveform pane, this pane's draw order is **unchanged** — the wash still draws after the
tile content, after the boundary lines are added alongside it — because the spectrogram's content
is an opaque heatmap image, not a thin line on an empty background; a translucent wash tints it
rather than hiding it, which was never the reported failure mode here. Both panes pick up the new
hue automatically since they read the same shared tokens.

## Amendment 2 — H-99 diagnostics/Explain register (2026-09-21, autonomous)

H-91–H-94 added *Explain My Voice*, a second reading of the same measurements §8.10 already
covers, and gave it firmer principles than §8.10's original table: a voice's high end is not
required to be flat, and "harsh"/"hard"/"boomy" language (with the correction that goes with it)
is reserved for a reading that clears its threshold by more than a microphone swap would explain.
§8.10's table predates those principles and disagreed with them on both counts. This amendment
brings §8.10 into line so the panel and the modal never offer two different verdicts over the
same number:

- **Air's "EQ move offered" cell is withdrawn.** §8.10 listed "boost 12 kHz +3 dB Q 0.7 (low
  air)"; no such move is offered any more, at either end of the air zone. A voice's spectrum
  rolls off up here, same as most microphones — the finding stays informational and boosting to
  flatten it would mostly raise hiss and sibilance, not presence.
- **Mud's and Presence's warn thresholds gain a 1 dB grace margin**, shared with Explain My
  Voice's `NEAR_THRESHOLD_DB` (`explain/thresholds.ts`, re-exported from
  `analyzer/diagnosticsHints.ts`, which now owns the constant). A reading within that margin of
  the warn line (mud's +9 dB, presence's −2 dB) is phrased the same mild, no-EQ-move way as the
  band below the threshold (mud: "slightly full"; presence: "slightly forward"); "a bit boomy" /
  "forward or harsh", and the EQ move that comes with them, apply only once the margin is cleared.
  The thresholds and severities themselves (mud info at +6, warn at +9; presence warn at −14/−2)
  are unchanged — only the last 1 dB below each warn line reads more conservatively.

No wire format, command or acceptance criterion changes; this is a copy/verdict correction only.
