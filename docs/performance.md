# Performance

T-704 checks every PROMPT §2 and spec performance target with an automated measurement, on the
owner's machine (AMD Ryzen with a Radeon 780M, "Phoenix"). This page holds the targets matrix, the
fixes T-704 made (with before/after numbers), and the open findings.

## Reproduce

| Command | What it measures | Log |
|---|---|---|
| `just bench` | Every `cargo bench` target: module/rack CPU, spec CPU budgets, playback start, peaks, chunk store, encoders, sandbox IPC | `target/bench/raw.log` |
| `just test-big` | Release checks on 60-min documents: the real open path, import cold/warm, memory, undo/redo, recovery, edit ops, tile latency, sidecar | `target/bench/big.log` |
| `just bench-ui` | Headless UI frame-time sweep (its own Vite on port 5193 + Chromium over CDP), plus the H-43 idle pass (idle CPU and playback frame rate, dev and release builds) | `target/bench/ui.log` |
| `just perf-matrix` | Rewrites the matrix below from the three logs | `docs/performance.md` |

Each measurement prints `BENCH_RESULT` lines, using the T-110 convention from
`vox_testkit::bench_report`. `target/bench/summary.md` merges all three logs, and
`scripts/bench/matrix.py` turns them into the matrix below.

### Method notes

- **Background load.** T-704 ran while other agents were building in parallel (load average 7–15
  on 16 threads). Wall-clock results therefore vary from run to run. For example, the Canvas2D
  waveform at 2126×850 measured p95 13.6 ms in one sweep and 45.7 ms in the next. The open and
  import checks report the best of N runs, which is the intrinsic cost; they report the median,
  the worst run and the CPU time as info. The UI log records the load average of each run.
- **Open path.** `powervoice-app`'s `perf_big` calls `DocumentService::open`, the code behind the
  `document_open` command. It runs headless on a `FakeBackend` engine, on the real disk under
  `target/big-tests`. That disk is btrfs with `compress=zstd:3`, so the one `fdatasync` after an
  import also pays for compression.
- **UI stand-in.** The UI sweep uses headless Chromium in place of WebKitGTK, the owner's reference
  renderer (ADR-009).
  - It runs on the machine's GPU: `--enable-gpu` makes ANGLE use Mesa radeonsi, the same driver
    stack WebKitGTK uses. The default SwiftShader is CPU-only.
  - rAF is uncapped, so a frame's delta is its real cost rather than the 16.7 ms vsync cadence.
  - The App runs on mocked IPC (`?preview&doc=60min`) with a 60-min document. Its peaks come from
    a precomputed pyramid and its spectrogram tiles from memoized templates, so the sweep measures
    the renderer, not the mock's synthesis on the UI thread.
  - Each case runs a 3 s warm-up sweep, then a 10 s zoom-then-scroll sweep through the real wheel
    handler. It runs with both the `canvas2d` renderer Setting (the fallback) and `auto` (WebGL2,
    the default).
- **Playback start.** The engine runs on the `FakeBackend` simulated clock, with the output stream
  already open and idle. The bench times Play → the first non-silent frame written to the device
  buffer (SPEC-003 AC-1), worst of 8 Play phases, at 0.1 ms resolution.

<!-- BEGIN TARGETS MATRIX -->

_Generated 2026-09-16 05:36 UTC by `just perf-matrix` on AMD Ryzen 7 PRO 7840U w/ Radeon 780M Graphics, 16 threads, Linux 7.2.5-3-omarchy, `target/` on btrfs. Logs:
`just bench` 2026-09-16 05:27 UTC, `just test-big` 2026-09-16 05:30 UTC, `just bench-ui` 2026-09-16 05:36 UTC._
**102 pass, 9 tight (< 30 % margin), 1 fail, 0 not run.**


### PROMPT §2 — open a 60-min 48 kHz mono WAV in < 3 s, waveform shown (SPEC-006 AC-19: first draw ≤ 3 s after open)

How: `powervoice-app` `perf_big`: `DocumentService::open` (the `document_open` path: probe, import into the chunk store with peak pyramids, sidecar, engine hand-over), worst of 3, headless; then the UI's first zoom-to-fit `peaks_get`. `vox-project` `big.rs`: `import_file` alone, cold (`posix_fadvise(DONTNEED)`) and warm page cache. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `open_60min_wav_ms` | 1421 ms | ≤ 3000 | +53 % | pass |
| `open_60min_to_first_overview_ms` | 1428 ms | ≤ 3000 | +52 % | pass |
| `open_60min_first_overview_1280px_ms` | 7.051 ms | ≤ 3000 | +100 % | pass |
| `open_60min_first_overview_2126px_ms` | 6.179 ms | ≤ 3000 | +100 % | pass |
| `import_60min_wav_cold_cache_ms` | 1069 ms | ≤ 3000 | +64 % | pass |
| `import_60min_wav_warm_cache_ms` | 1692 ms | ≤ 3000 | +44 % | pass |

### PROMPT §2 — 60 fps scroll/zoom; SPEC-006 AC-18: p50 ≤ 16.7 ms, p99 ≤ 50 ms, ≤ 1 frame > 50 ms (T-704 adds p95 ≤ 16.7 ms)

How: `scripts/bench/ui_frames.mjs`: the preview App (`?preview&scene=document&doc=60min`) in headless Chromium with uncapped rAF, 10 s zoom-then-scroll sweep through the real wheel handler after a 3 s warm-up; renderer Setting `canvas2d` (fallback) and `auto` (WebGL2 first, the default). Reproduce: `just bench-ui`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `frame_document_canvas2d_1280x720_p50_ms` | 1.7 ms | ≤ 16.7 | +90 % | pass |
| `frame_document_canvas2d_1280x720_p95_ms` | 2.9 ms | ≤ 16.7 | +83 % | pass |
| `frame_document_canvas2d_1280x720_p99_ms` | 6.1 ms | ≤ 50 | +88 % | pass |
| `frame_document_canvas2d_1280x720_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_document_canvas2d_2126x850_p50_ms` | 2.2 ms | ≤ 16.7 | +87 % | pass |
| `frame_document_canvas2d_2126x850_p95_ms` | 4.4 ms | ≤ 16.7 | +74 % | pass |
| `frame_document_canvas2d_2126x850_p99_ms` | 13.3 ms | ≤ 50 | +73 % | pass |
| `frame_document_canvas2d_2126x850_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_document_auto_1280x720_p50_ms` | 1.8 ms | ≤ 16.7 | +89 % | pass |
| `frame_document_auto_1280x720_p95_ms` | 3.3 ms | ≤ 16.7 | +80 % | pass |
| `frame_document_auto_1280x720_p99_ms` | 6.8 ms | ≤ 50 | +86 % | pass |
| `frame_document_auto_1280x720_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_document_auto_2126x850_p50_ms` | 1.9 ms | ≤ 16.7 | +89 % | pass |
| `frame_document_auto_2126x850_p95_ms` | 4.6 ms | ≤ 16.7 | +72 % | pass |
| `frame_document_auto_2126x850_p99_ms` | 11.1 ms | ≤ 50 | +78 % | pass |
| `frame_document_auto_2126x850_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |

### SPEC-007 AC-10 — split view (waveform + spectral) frame time, tiles cached: same tolerance as SPEC-006 AC-18

How: Same sweep with `scene=spectral` (split view). Reproduce: `just bench-ui`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `frame_spectral_canvas2d_1280x720_p50_ms` | 3.9 ms | ≤ 16.7 | +77 % | pass |
| `frame_spectral_canvas2d_1280x720_p95_ms` | 11.5 ms | ≤ 16.7 | +31 % | pass |
| `frame_spectral_canvas2d_1280x720_p99_ms` | 13.5 ms | ≤ 50 | +73 % | pass |
| `frame_spectral_canvas2d_1280x720_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_spectral_canvas2d_2126x850_p50_ms` | 8.9 ms | ≤ 16.7 | +47 % | pass |
| `frame_spectral_canvas2d_2126x850_p95_ms` | 21.2 ms | ≤ 16.7 | -27 % | **FAIL** |
| `frame_spectral_canvas2d_2126x850_p99_ms` | 24.7 ms | ≤ 50 | +51 % | pass |
| `frame_spectral_canvas2d_2126x850_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_spectral_auto_1280x720_p50_ms` | 3.2 ms | ≤ 16.7 | +81 % | pass |
| `frame_spectral_auto_1280x720_p95_ms` | 9.9 ms | ≤ 16.7 | +41 % | pass |
| `frame_spectral_auto_1280x720_p99_ms` | 15.1 ms | ≤ 50 | +70 % | pass |
| `frame_spectral_auto_1280x720_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_spectral_auto_2126x850_p50_ms` | 3.1 ms | ≤ 16.7 | +81 % | pass |
| `frame_spectral_auto_2126x850_p95_ms` | 6.1 ms | ≤ 16.7 | +63 % | pass |
| `frame_spectral_auto_2126x850_p99_ms` | 14 ms | ≤ 50 | +72 % | pass |
| `frame_spectral_auto_2126x850_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |

### H-43 — idle main-thread CPU ≤ 2 % of one core in a release build (≤ 10 % debug); playback still draws at the display rate (60 fps)

How: `scripts/bench/ui_frames.mjs` idle pass: the preview App (renderer `auto`) in vsync-paced (60 Hz) headless Chromium — not the uncapped sweep above — 10 s idle per scene after a 4 s settle; busy = CDP `TaskDuration` / wall time. `dev` is the Vite dev server (debug JS), `release` a `vite build` bundle with the bench preview (`VITE_PV_BENCH_PREVIEW=1`). Then Play for 3 s: animation frames the app ran per second. Reproduce: `just bench-ui`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `idle_dev_empty_main_thread_pct` | 0.028 pct_core | ≤ 10 | +100 % | pass |
| `idle_dev_document_main_thread_pct` | 0.007 pct_core | ≤ 10 | +100 % | pass |
| `idle_dev_document_rack_main_thread_pct` | 0.008 pct_core | ≤ 10 | +100 % | pass |
| `idle_dev_spectral_main_thread_pct` | 0.009 pct_core | ≤ 10 | +100 % | pass |
| `idle_release_empty_main_thread_pct` | 0.016 pct_core | ≤ 2 | +99 % | pass |
| `idle_release_document_main_thread_pct` | 0.006 pct_core | ≤ 2 | +100 % | pass |
| `idle_release_document_rack_main_thread_pct` | 0.006 pct_core | ≤ 2 | +100 % | pass |
| `idle_release_spectral_main_thread_pct` | 0.008 pct_core | ≤ 2 | +100 % | pass |
| `playback_dev_document_fps` | 59.98 fps | ≥ 55 | +9 % | tight |
| `playback_dev_document_rack_fps` | 60.23 fps | ≥ 55 | +10 % | tight |
| `playback_dev_spectral_fps` | 59.96 fps | ≥ 55 | +9 % | tight |
| `playback_release_document_fps` | 59.96 fps | ≥ 55 | +9 % | tight |
| `playback_release_document_rack_fps` | 59.96 fps | ≥ 55 | +9 % | tight |
| `playback_release_spectral_fps` | 59.96 fps | ≥ 55 | +9 % | tight |

### PROMPT §2 / SPEC-003 AC-1 — playback start < 50 ms (Play → first non-silent frame written to the device buffer)

How: `vox-engine` `playback_start`: `ManualEngine` on the `FakeBackend` clock, stream open and idle, worst of 8 Play phases, 0.1 ms resolution; empty rack and the default voice rack. H-46: the rack is pre-rolled, so the voice rack's overhead over the empty rack is asserted too (≤ 5 ms). Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `playback_start_empty_rack_64f_written_max_ms` | 1.4 ms | ≤ 50 | +97 % | pass |
| `playback_start_empty_rack_256f_written_max_ms` | 1.4 ms | ≤ 50 | +97 % | pass |
| `playback_start_empty_rack_1024f_written_max_ms` | 6.7 ms | ≤ 50 | +87 % | pass |
| `playback_start_voice_rack_64f_written_max_ms` | 2.7 ms | ≤ 50 | +95 % | pass |
| `playback_start_voice_rack_256f_written_max_ms` | 1.4 ms | ≤ 50 | +97 % | pass |
| `playback_start_voice_rack_1024f_written_max_ms` | 6.7 ms | ≤ 50 | +87 % | pass |
| `playback_start_voice_rack_64f_overhead_max_ms` | 1.3 ms | ≤ 5 | +74 % | pass |
| `playback_start_voice_rack_256f_overhead_max_ms` | 0 ms | ≤ 5 | +100 % | pass |
| `playback_start_voice_rack_1024f_overhead_max_ms` | 0 ms | ≤ 5 | +100 % | pass |

### PROMPT §2 — full rack at 48 kHz < 20 % of one core

How: `vox-rack` `full_rack` (T-110): the typical voice rack's `process()` at each realtime block size; `vox-engine` `callback_histogram`: the whole output callback. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `full_rack_process_64f_pct_core` | 0.6393 pct_core | ≤ 20 | +97 % | pass |
| `full_rack_process_128f_pct_core` | 0.6325 pct_core | ≤ 20 | +97 % | pass |
| `full_rack_process_256f_pct_core` | 0.6268 pct_core | ≤ 20 | +97 % | pass |
| `full_rack_process_512f_pct_core` | 0.6257 pct_core | ≤ 20 | +97 % | pass |
| `full_rack_process_1024f_pct_core` | 0.6238 pct_core | ≤ 20 | +97 % | pass |

### SPEC-007 AC-18 / H-42 §8.11 — live analyzer diagnostics ≤ 2 % of one core at 60 Hz (≤ 333 µs per analyzer frame)

How: `vox-dsp` `diagnostics`: the live voice tracker fed one 60 Hz control tick (800 samples at 48 kHz) per frame, a report every 6th frame, 20 s of speech-like signal, best of 3; one Spectrum Inspector frame per FFT size (30 Hz); the long-term average job (FFT 16 384 + diagnostics) as a realtime factor. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `diagnostics_live_per_frame` | 41.89 us | ≤ 333 | +87 % | pass |
| `inspector_frame_fft1024` | 1.243 us | — | — | info |
| `inspector_frame_fft4096` | 5.176 us | — | — | info |
| `inspector_frame_fft16384` | 22.54 us | — | — | info |
| `inspector_frame_fft32768` | 47.83 us | — | — | info |
| `ltas_offline_realtime_factor` | 1023 realtime_x | — | — | info |

### PROMPT §2 — noise-reduction latency ≤ 50 ms

How: `vox-modules` `spec_budgets`: the default instance's `latency_samples()` (N = 2048). Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `noise_reduction_latency_48000hz_ms` | 42.67 ms | ≤ 50 | +15 % | tight |
| `noise_reduction_latency_44100hz_ms` | 46.44 ms | ≤ 50 | +7 % | tight |

### SPEC-013 AC-17 — Noise Gate: 60 s pink noise, HPF on, look-ahead 5 ms ≤ 0.3 s

How: `vox-modules` `spec_budgets`, offline 4096-frame blocks, best of 3. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `noise_gate_60s_hpf_lookahead5ms_render_s` | 0.04926 s | ≤ 0.3 | +84 % | pass |

### SPEC-014 AC-18 — Noise Reduction: 60 s ≤ 1.2 s at defaults, ≤ 1.8 s at N = 8192; 60 s capture ≤ 0.3 s

How: `vox-modules` `spec_budgets` with a captured print (the STFT path — T-110's bench measured the no-print delay line), the AC-6 signal. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `noise_reduction_60s_defaults_render_s` | 0.1288 s | ≤ 1.2 | +89 % | pass |
| `noise_reduction_60s_n8192_render_s` | 0.1339 s | ≤ 1.8 | +93 % | pass |
| `noise_reduction_capture_60s_s` | 0.06304 s | ≤ 0.3 | +79 % | pass |

### SPEC-015 AC-22 — EQ response curve: 2048 points, 9 components ≤ 2 ms (median of 100)

How: `vox-modules` `spec_budgets`. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `eq_response_curve_2048pts_9components_median_ms` | 0.8391 ms | ≤ 2 | +58 % | pass |

### SPEC-016 AC-19 — Dynamics: 60 s all sections + RMS + look-ahead 10 ms ≤ 0.6 s; defaults ≤ 0.3 s

How: `vox-modules` `spec_budgets`. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `dynamics_60s_all_sections_rms_lookahead10ms_render_s` | 0.1418 s | ≤ 0.6 | +76 % | pass |
| `dynamics_60s_defaults_render_s` | 0.1418 s | ≤ 0.3 | +53 % | pass |

### SPEC-017 AC-17 — True-Peak Limiter ≤ 1.5 % of one core (256-frame blocks)

How: `vox-modules` `true_peak_limiter`, worst of 0/+12/+24 dB input gain. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `true_peak_limiter_256f_worst_pct_core` | 0.2699 pct_core | ≤ 1.5 | +82 % | pass |

### SPEC-004 AC-3 — undo/redo on 20 000 pieces ≤ 50 ms

How: `vox-project` `history_exact` (H-17), real disk. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac3_worst_undo_redo_20000_pieces_ms` | 1.318 ms | ≤ 50 | +97 % | pass |

### SPEC-004 AC-5 / T-301 — memory budget: resident ≤ budget + 128 MiB

How: `vox-project` `big.rs`: 60-min playback + 200 seeks at a 512 MiB budget. `perf_big`: the store's peak resident memory and the process RSS growth with a 60-min document open (default budget). Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac5_60min_playback_peak_resident_mib` | 512 MiB | ≤ 640 | +20 % | tight |
| `open_60min_store_peak_resident_mib` | 704 MiB | ≤ 4224 | +83 % | pass |
| `open_60min_process_rss_growth_mib` | 665.7 MiB | ≤ 4224 | +84 % | pass |

### SPEC-004 AC-11 — recovering a 60-min session with 1000 records < 5 s

How: `vox-project` `recovery` (H-17). Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac11_recover_60min_1000_records_ms` | 64.51 ms | ≤ 5000 | +99 % | pass |

### SPEC-007 AC-9 — spectrogram tiles on a 60-min document: visible ≤ 200 ms, refined ≤ 2 s, warm ≤ 50 ms

How: `vox-engine` `tests/spectro.rs` (engine side: compute + channel send), 1920 px, FFT 2048. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec007_ac9_visible_3600s_view_ms` | 8.035 ms | ≤ 200 | +96 % | pass |
| `spec007_ac9_refined_3600s_view_ms` | 107 ms | ≤ 2000 | +95 % | pass |
| `spec007_ac9_warm_3600s_view_ms` | 1.671 ms | ≤ 50 | +97 % | pass |
| `spec007_ac9_visible_600s_view_ms` | 12.34 ms | ≤ 200 | +94 % | pass |
| `spec007_ac9_refined_600s_view_ms` | 31.02 ms | ≤ 2000 | +98 % | pass |
| `spec007_ac9_warm_600s_view_ms` | 1.491 ms | ≤ 50 | +97 % | pass |
| `spec007_ac9_visible_60s_view_ms` | 7.515 ms | ≤ 200 | +96 % | pass |
| `spec007_ac9_refined_60s_view_ms` | 7.515 ms | ≤ 2000 | +100 % | pass |
| `spec007_ac9_warm_60s_view_ms` | 0.4835 ms | ≤ 50 | +99 % | pass |
| `spec007_ac9_visible_10s_view_ms` | 10.22 ms | ≤ 200 | +95 % | pass |
| `spec007_ac9_refined_10s_view_ms` | 10.22 ms | ≤ 2000 | +99 % | pass |
| `spec007_ac9_warm_10s_view_ms` | 0.9427 ms | ≤ 50 | +98 % | pass |
| `spec007_ac9_visible_1s_view_ms` | 3.664 ms | ≤ 200 | +98 % | pass |
| `spec007_ac9_refined_1s_view_ms` | 3.664 ms | ≤ 2000 | +100 % | pass |
| `spec007_ac9_warm_1s_view_ms` | 0.0548 ms | ≤ 50 | +100 % | pass |

### SPEC-008 AC-14 — edit ops on 20 000 pieces + 1000 markers: splice ≤ 5 ms p95, command (incl. journal fdatasync) ≤ 50 ms p95

How: `vox-project` `history_exact`, 100 seeded runs per op, real disk. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec008_ac14_copy_splice_p95_ms` | 0.001834 ms | ≤ 5 | +100 % | pass |
| `spec008_ac14_cut_splice_p95_ms` | 0.3966 ms | ≤ 5 | +92 % | pass |
| `spec008_ac14_cut_command_p95_ms` | 26.09 ms | ≤ 50 | +48 % | pass |
| `spec008_ac14_paste_splice_p95_ms` | 0.3826 ms | ≤ 5 | +92 % | pass |
| `spec008_ac14_paste_command_p95_ms` | 12.36 ms | ≤ 50 | +75 % | pass |
| `spec008_ac14_delete_splice_p95_ms` | 0.3466 ms | ≤ 5 | +93 % | pass |
| `spec008_ac14_delete_command_p95_ms` | 14.27 ms | ≤ 50 | +71 % | pass |
| `spec008_ac14_trim_splice_p95_ms` | 0.2595 ms | ≤ 5 | +95 % | pass |
| `spec008_ac14_trim_command_p95_ms` | 1.527 ms | ≤ 50 | +97 % | pass |
| `spec008_ac14_silence_splice_p95_ms` | 0.1853 ms | ≤ 5 | +96 % | pass |
| `spec008_ac14_silence_command_p95_ms` | 1.505 ms | ≤ 50 | +97 % | pass |
| `spec008_ac14_insert_silence_splice_p95_ms` | 0.2966 ms | ≤ 5 | +94 % | pass |
| `spec008_ac14_insert_silence_command_p95_ms` | 1.437 ms | ≤ 50 | +97 % | pass |

### SPEC-018 AC-18 — sidecar write/read ≤ 150 ms p95

How: `vox-project` `sidecar_perf`. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec018_ac18_sidecar_write_p95_ms` | 6.938 ms | ≤ 150 | +95 % | pass |
| `spec018_ac18_sidecar_read_p95_ms` | 7.311 ms | ≤ 150 | +95 % | pass |

<!-- END TARGETS MATRIX -->

## T-704 fixes

| Area | Change | Before | After |
|---|---|---|---|
| NR CPU bench (measurement) | `process_cost` built Noise Reduction with no print, so it timed the N-sample delay line. It now captures a print (the STFT path, reduction on) and asserts it. | 0.0007 % of a core (256-frame blocks) | 0.23 % (a 60 s render takes 0.14 s against a 1.2 s budget) |
| Peaks query (`vox_project::peaks`) | Keeps the last chunk's pyramid across the output buckets of one query. It used to re-read, CRC-check and decode the ~11 KB record for every bucket. | 1000 buckets: 1.12 ms (spp 64), 1.17 ms (1024), 1.19 ms (4096); 10 240 record reads for 10 chunks | 17 µs, 40 µs, 119 µs; 10 reads. A zoomed 2126-px view on a 60-min document: p95 0.31 ms |
| Peak pyramid (`ChunkPeaks::compute`) | 8-lane compare-and-select min/max instead of a serial fold, and a branch-free non-finite scan | 165.7 µs per 64 Ki-sample chunk; 482–602 ms for a 60-min import | 18.8 µs (8.8×); 88–154 ms |
| Import (`import_file`) | Decoding and downmixing run on their own thread, overlapping chunk commits (bounded channel, recycled buffers) | Open 60 min: best 2241 ms, worst 2654 ms (3 opens, load ~7) | Best 1671 ms, median 1734 ms (5 opens, load ~10) |
| Spectrogram, Canvas2D path | `columnDb`: the time rule is computed once per column and bin, and its max is taken on raw codes; the renderer keeps a per-draw tile memo. Both are exactly `pixelDb`'s values (tested). | Split view 1280×720: 2 fps (p50 491 ms); 2126×850: 0.35 fps (p50 2809 ms) | 15–34 fps (p50 22–48 ms); 16–20 fps (p50 44–46 ms). Finished by H-47 below |
| WebGL2 quads (`QuadBatch`) | Writes into a growable `Float32Array` and hands back a view. It used to push 36 numbers per quad into a JS array and copy that into a new `Float32Array` every frame. | Waveform, WebGL2, 2126×850: p95 19.3 ms, p99 42.3 ms, 5 frames > 50 ms | p95 11.8–12.4 ms, p99 27–31 ms, 0–2 frames > 50 ms |

Each code change has a regression test:
- `peaks_query::a_query_reads_each_chunk_pyramid_record_once`
- `store::peaks::compute_matches_the_reference_fold`
- `samplerColumn.test.ts`
- `quadsTyped.test.ts`
- the existing `import` tests (cancel, damage, downmix, markers)

## H-47 fixes — split-view frame spikes

T-704 reported the split view spiking to p99 110–130 ms on WebGL2 (22–29 frames over 50 ms,
clustered on tile arrivals) and the Canvas2D fallback stuck at 15–34 fps. H-47 attributed those
frames with a CDP CPU profile aligned to the page's own `requestAnimationFrame` windows, plus
every GL call timed from inside the page (headless Chromium, 60-min document, `scene=spectral`,
2126×850).

**The spikes were the measurement harness, not the renderer.** 96.7 % of the self time inside
every WebGL2 frame over 50 ms, and 77.4 % of the Canvas2D one, was `vxst()` in
`ui/src/dev/previewIpc.ts` — the preview's *own* tile synthesis, on the UI thread. GL work inside
those same frames was 3.7 ms out of 2769 ms, and the whole 10 s sweep spent 122 ms inside GL calls
for 212 MiB of texture upload. T-704's memo keyed a template on (FFT size, **hop**, tile % 16), and
every Ctrl+wheel step of the sweep picks a new hop, so each zoom step re-synthesized 16 fresh
~10 ms tiles in `setTimeout(0)` tasks that landed inside a frame. The real app never pays this:
tiles come from Rust tile workers over IPC.

| Area | Change | Before | After |
|---|---|---|---|
| Preview tile mock (measurement) | The long document's templates are synthesized at a canonical hop (`fft/4`) whatever hop was requested — the header still carries the requested hop, so geometry is unchanged — so the whole sweep shares 16 templates per FFT size. | Split view, WebGL2: p99 34.5 ms, max 152 ms, 25 frames > 50 ms; Canvas2D 2126×850: 38 fps, p95 98.4 ms, 30 frames > 50 ms | WebGL2: p99 11.9 ms, max 17.7 ms, 0 frames > 50 ms; Canvas2D: 51 fps, p95 39.1 ms, 3 frames > 50 ms |
| Spectrogram tile textures (`spectrogram/webglRenderer.ts`) | A 64 MiB least-recently-drawn texture cache, re-used textures and `texSubImage2D` when the tile size is unchanged. The old rule deleted every texture the latest draw hadn't used, so a scroll re-uploaded what it had just thrown away. | 847 `createTexture` + 847 `deleteTexture` for 847 uploads, 212 MiB in 10 s | No texture create or delete after the first draw of a tile; a tile that scrolled off screen and back costs no upload |
| Per-frame upload budget (`render/uploadBudget.ts`) | At most 2 tiles / 2 MiB uploaded per frame (visible before off-screen margin, newest first); the rest are uploaded on the following frames, which the renderer asks for by staying "animating" (the H-43 scheduler contract). | A 64-tile request could upload in one frame | Bounded per frame at every FFT size, backlog drained at the display rate |
| Spectrogram, Canvas2D path (`sampler.ts::columnDb`, `SpectralView.svelte`) | `dequantizeDb` per bin (a multiply and a **division**, ~1.5 M of each per 2126×850 frame) becomes a bit-exact 256-entry table; "unknown" is `-Infinity` instead of `NaN`, so both hot loops compare instead of calling `Number.isNaN`; the one- and two-frame-row cases read through hoisted locals; the pixel loop indexes the colormap LUT directly instead of `normalizeDb` + `clamp01` + `colorForT` (which allocated a tuple per pixel). | Split view 1280×720: 81 fps, p50 12.4 ms, p95 21.9 ms; 2126×850: 38 fps, p50 19.4 ms, p95 98.4 ms | 167 fps, p50 4.1 ms, p95 12.2 ms; 82 fps, p50 9.7 ms, p95 22.7 ms |

Every rendered value is unchanged: `samplerColumn.test.ts` still asserts `columnDb` ≡ `pixelDb`
exactly (SPEC-007 AC-13), and the colormap index is the same expression `colorForT` computes.

Regression tests: `render/uploadBudget.test.ts` (priority order, the per-frame cap, a backlog
draining over frames) and `spectrogram/webglRenderer.test.ts` (no re-upload of an unchanged tile,
`texSubImage2D` re-use, off-screen textures kept, LRU eviction, the budget across frames, only
uploaded tiles drawn) against a recording fake WebGL2 context.

## Findings and open items

- **Playback start with the default voice rack: fixed by H-46 (was 49.4 ms, a 1.2 % margin).** The
  rack's latency (NR 2048 samples = 42.7 ms, plus 257 for the limiter) used to be added to every
  start. The rack is now pre-rolled (SPEC-003 Amendment 2, A-026): L samples of warm-up before
  the play position, then L of look-ahead, fed faster than real time and never heard. The voice
  rack starts in 1.4–6.7 ms, the same as the empty rack except +1.3 ms at 64 frames, where the
  pre-roll takes three callbacks (at most 32× real time per callback). The slowest start callback
  takes 243 µs of a 1333 µs deadline at 64 frames, 297 µs of 5333 µs at 256 frames, and 439 µs of
  21 333 µs at 1024 frames (`playback_start_*_callback_max_us`, informational).
- **Split view frame time (SPEC-007 AC-10) still misses.**
  - WebGL2, the default renderer, has p50 4–7 ms and p95 10–20 ms. Its p99 is 110–130 ms, with
    22–29 frames over 50 ms per 10 s sweep. The spikes cluster on tile arrivals; texture uploads
    and tile scheduling are the next suspects.
  - The Canvas2D fallback still computes every pixel in JS: 15–34 fps.
- **Idle CPU (H-43, fixed).** The owner measured the web view's main thread at about one full core
  while the debug app sat idle. A CDP trace of the preview App (10 s idle, before the fix) showed
  three causes:
  - **Perpetual draw loops.** Every canvas renderer redrew at the display rate from its own
    `requestAnimationFrame` loop (the H-32 rule): waveform, spectral view and EQ graph, plus the
    transport's playhead loop. That is 120–180 animation frames a second with nothing on screen
    changing. The top JS stacks were `WaveformView loop → draw → drawWebgl2 → buildColumnQuads /
    QuadBatch.rect`, `EqGraph loop → drawInner → totalCurveToScreen / nodeGainDb`, `SpectralView
    loop → drawSpectrogramWebgl2` and the transport's `onFrame`.
  - **Telemetry at 60 Hz while stopped.** The engine sent `VXTM`, `VXMT` and `VXSA` every control
    tick whether or not anything changed, and each frame was decoded and written into a Svelte
    store (the analyzer's `frame`, the rack's slot meters, the input meter).
  - **Meter transitions.** The output meter animates `height`/`bottom` with a 100 ms CSS
    transition, so any moving level keeps a style recalc + layout running every frame. The
    preview's fake 20 Hz meter signal did this even while "stopped" (about 60 layouts a second in
    the empty state). In the real app it only happens while a level moves.

  The fixes:
  - `ui/src/lib/render/frameScheduler.ts` is one shared on-demand scheduler. A renderer requests a
    frame when an input changes, and keeps frames coming only while something animates (playback,
    recording, a meter or peak marker falling). It keeps H-32's robustness: a thrown draw is
    retried on the next frames and never blocks later redraws.
  - The engine gates idle telemetry (`IdleTelemetryGate`, `crates/engine/src/telemetry.rs`).
    While stopped with nothing monitored or armed, `VXTM` goes out only on a change, with a 4 Hz
    heartbeat for a steady non-silent level, and falls silent at the floor. `VXMT` skips repeats.
    `VXSA` sends one at-rest frame, then nothing until the signal returns.
  - The UI stores skip unchanged values.
  - The output meter's decay runs on scheduler frames once telemetry stops, and ends at the floor.

  Numbers from `just bench-ui`'s idle pass on a quiet machine (load average < 1): vsync-paced
  headless Chromium, 1600×900, renderer `auto`. They are main-thread busy % of one core, with
  animation frames per second in brackets. "Before" is the ticket's base commit, with the preview
  mock patched to stream what the pre-H-43 engine sent while idle: 60 Hz silent `VXTM` and `VXSA`.
  `VXMT` wasn't emulated, so the before rack row is a lower bound.

  | Scene | Before, dev | Before, release | After, dev | After, release |
  |---|---|---|---|---|
  | Empty app | 4.0 % (120/s) | 4.1 % (120/s) | 0.03 % (0/s) | 0.02 % (0/s) |
  | Document | 7.8 % (120/s) | 6.8 % (120/s) | 0.007 % (0/s) | 0.006 % (0/s) |
  | Document + rack (EQ graph) | 13.4 % (180/s) | 12.1 % (180/s) | 0.006 % (0/s) | 0.005 % (0/s) |
  | Split view (waveform + spectral) | 7.8 % (180/s) | 7.4 % (180/s) | 0.009 % (0/s) | 0.007 % (0/s) |

  Playback after the fix still runs at 60.0 frames/s in every scene, with the main thread 18–23 %
  busy. Headless Chromium on the GPU is much cheaper per frame than WebKitGTK, the owner's
  renderer (more so with debug JS). The owner's one core is the same per-frame work at WebKitGTK's
  cost. The "after" numbers are about zero because nothing runs at all while idle: no animation
  frame, and no IPC message. The rows come from `idle_*` and `playback_*` in `target/bench/ui.log`;
  the H-43 group in the matrix above holds them.
- **Output-callback worst case (`just bench-callback`, ADR-002 §2) is load-sensitive — re-checked
  independently in both H-47 and H-50, and it was the load.** T-704 saw the 128-frame row's max
  callback reach 4–13 ms, over its 2.7 ms deadline, at load average 10–16, while p99 stayed within
  the deadline at every block size, and left "re-check on an idle machine" open. On a quiet
  machine (H-47's run) the whole histogram sits far inside its deadlines:

  | block | deadline | p50 | p99 | max | max as % of the deadline |
  |---|---|---|---|---|---|
  | 128 | 2666.7 µs | 32.8 µs | 32.8 µs | 264.3 µs | 9.9 % |
  | 256 | 5333.3 µs | 65.5 µs | 65.5 µs | 287.9 µs | 5.4 % |
  | 512 | 10 666.7 µs | 131.1 µs | 524.3 µs | 679.3 µs | 6.4 % |
  | 1024 | 21 333.3 µs | 262.1 µs | 262.1 µs | 396.3 µs | 1.9 % |

  A second H-47 run taken while the machine was still busy from a build gave 595 µs / 1730 µs /
  1800 µs / 2869 µs — still inside every deadline, and an order of magnitude better than T-704's
  worst. H-50 re-ran the same bench again on its own idle window (load average 1.2; `ps` still
  showed a `cargo check` that had started in the same second, too recent to move the 1-minute
  average): 269.6 / 284.0 / 328.9 / 389.8 µs max (10.1 %, 5.3 %, 3.1 %, 1.8 % of deadline, 128→1024
  frames), then 4 more runs at load average 4.2–13.2, ranging 266.7–948.4 µs (128 frames) up to
  389.8–2192.3 µs (1024 frames) — the 1024-frame row is the noisiest under load in absolute terms,
  but its worst observed max there is still only ~10 % of its deadline. Across both tickets' runs,
  idle or loaded, no max has exceeded its deadline since T-704's original measurement, so no
  follow-up ticket is opened for this item. The bench thread isn't realtime-scheduled, so what
  T-704 measured was preemption, not the callback path; the caveat both tickets agree on is that
  this bench only means something on a quiet machine, and it should still be re-checked
  periodically rather than declared fixed for good from one or two idle sessions.
- **The UI frame-time rows vary with background load.** The matrix shows the latest sweep. In it,
  the Canvas2D waveform at 2126×850 failed (p95 45.7 ms); in the sweep before, it passed (p95
  13.6 ms, 0 frames over 50 ms). The split-view failures reproduce in every run.
- **Progressive waveform while importing** (PROMPT §2's "shown progressively", SPEC-006 §2.3
  `PARTIAL`) is not implemented. The whole waveform is ready about 1.7 s after the open command
  starts (best of 5, under load), well within the 3 s target, so it was not needed to meet it.
- **SPEC-009 AC-15 (10 000-marker list) is not measurable yet.** The marker list's virtualization
  is deferred (`MarkersProperties.svelte`).
- **Not specced.** FLAC and MP3 import times have no PROMPT or spec target. The decode costs are in
  `vox-io`'s encode/decode benches only.
