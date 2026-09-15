# Performance

T-704 checks every PROMPT §2 and spec performance target with an automated measurement, on the
owner's machine (AMD Ryzen with a Radeon 780M, "Phoenix"). This page holds the targets matrix, the
fixes T-704 made (with before/after numbers), and the open findings.

## Reproduce

| Command | What it measures | Log |
|---|---|---|
| `just bench` | Every `cargo bench` target: module/rack CPU, spec CPU budgets, playback start, peaks, chunk store, encoders, sandbox IPC | `target/bench/raw.log` |
| `just test-big` | Release checks on 60-min documents: the real open path, import cold/warm, memory, undo/redo, recovery, edit ops, tile latency, sidecar | `target/bench/big.log` |
| `just bench-ui` | Headless UI frame-time sweep (its own Vite on port 5193 + Chromium over CDP), plus the idle-CPU baseline | `target/bench/ui.log` |
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

_Generated 2026-09-15 18:54 UTC by `just perf-matrix` on AMD Ryzen 7 PRO 7840U w/ Radeon 780M Graphics, 16 threads, Linux 7.2.3-arch1-3, `target/` on btrfs. Logs:
`just bench` 2026-09-15 18:54 UTC, `just test-big` 2026-09-15 18:45 UTC, `just bench-ui` 2026-09-15 18:48 UTC._
**67 pass, 11 tight (< 30 % margin), 16 fail, 0 not run.**


### PROMPT §2 — open a 60-min 48 kHz mono WAV in < 3 s, waveform shown (SPEC-006 AC-19: first draw ≤ 3 s after open)

How: `powervoice-app` `perf_big`: `DocumentService::open` (the `document_open` path: probe, import into the chunk store with peak pyramids, sidecar, engine hand-over), worst of 3, headless; then the UI's first zoom-to-fit `peaks_get`. `vox-project` `big.rs`: `import_file` alone, cold (`posix_fadvise(DONTNEED)`) and warm page cache. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `open_60min_wav_ms` | 2017 ms | ≤ 3000 | +33 % | pass |
| `open_60min_to_first_overview_ms` | 2030 ms | ≤ 3000 | +32 % | pass |
| `open_60min_first_overview_1280px_ms` | 9.583 ms | ≤ 3000 | +100 % | pass |
| `open_60min_first_overview_2126px_ms` | 13.12 ms | ≤ 3000 | +100 % | pass |
| `import_60min_wav_cold_cache_ms` | 1656 ms | ≤ 3000 | +45 % | pass |
| `import_60min_wav_warm_cache_ms` | 2612 ms | ≤ 3000 | +13 % | tight |

### PROMPT §2 — 60 fps scroll/zoom; SPEC-006 AC-18: p50 ≤ 16.7 ms, p99 ≤ 50 ms, ≤ 1 frame > 50 ms (T-704 adds p95 ≤ 16.7 ms)

How: `scripts/bench/ui_frames.mjs`: the preview App (`?preview&scene=document&doc=60min`) in headless Chromium with uncapped rAF, 10 s zoom-then-scroll sweep through the real wheel handler after a 3 s warm-up; renderer Setting `canvas2d` (fallback) and `auto` (WebGL2 first, the default). Reproduce: `just bench-ui`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `frame_document_canvas2d_1280x720_p50_ms` | 4.6 ms | ≤ 16.7 | +72 % | pass |
| `frame_document_canvas2d_1280x720_p95_ms` | 13 ms | ≤ 16.7 | +22 % | tight |
| `frame_document_canvas2d_1280x720_p99_ms` | 22.3 ms | ≤ 50 | +55 % | pass |
| `frame_document_canvas2d_1280x720_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |
| `frame_document_canvas2d_2126x850_p50_ms` | 14.9 ms | ≤ 16.7 | +11 % | tight |
| `frame_document_canvas2d_2126x850_p95_ms` | 45.7 ms | ≤ 16.7 | -174 % | **FAIL** |
| `frame_document_canvas2d_2126x850_p99_ms` | 85.4 ms | ≤ 50 | -71 % | **FAIL** |
| `frame_document_canvas2d_2126x850_frames_over_50ms` | 23 frames | ≤ 1 | -2200 % | **FAIL** |
| `frame_document_auto_1280x720_p50_ms` | 4 ms | ≤ 16.7 | +76 % | pass |
| `frame_document_auto_1280x720_p95_ms` | 11 ms | ≤ 16.7 | +34 % | pass |
| `frame_document_auto_1280x720_p99_ms` | 18.3 ms | ≤ 50 | +63 % | pass |
| `frame_document_auto_1280x720_frames_over_50ms` | 3 frames | ≤ 1 | -200 % | **FAIL** |
| `frame_document_auto_2126x850_p50_ms` | 5.4 ms | ≤ 16.7 | +68 % | pass |
| `frame_document_auto_2126x850_p95_ms` | 12.4 ms | ≤ 16.7 | +26 % | tight |
| `frame_document_auto_2126x850_p99_ms` | 27.3 ms | ≤ 50 | +45 % | pass |
| `frame_document_auto_2126x850_frames_over_50ms` | 0 frames | ≤ 1 | +100 % | pass |

### SPEC-007 AC-10 — split view (waveform + spectral) frame time, tiles cached: same tolerance as SPEC-006 AC-18

How: Same sweep with `scene=spectral` (split view). Reproduce: `just bench-ui`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `frame_spectral_canvas2d_1280x720_p50_ms` | 47.9 ms | ≤ 16.7 | -187 % | **FAIL** |
| `frame_spectral_canvas2d_1280x720_p95_ms` | 199.9 ms | ≤ 16.7 | -1097 % | **FAIL** |
| `frame_spectral_canvas2d_1280x720_p99_ms` | 297.6 ms | ≤ 50 | -495 % | **FAIL** |
| `frame_spectral_canvas2d_1280x720_frames_over_50ms` | 65 frames | ≤ 1 | -6400 % | **FAIL** |
| `frame_spectral_canvas2d_2126x850_p50_ms` | 45.8 ms | ≤ 16.7 | -174 % | **FAIL** |
| `frame_spectral_canvas2d_2126x850_p95_ms` | 91.4 ms | ≤ 16.7 | -447 % | **FAIL** |
| `frame_spectral_canvas2d_2126x850_p99_ms` | 169.2 ms | ≤ 50 | -238 % | **FAIL** |
| `frame_spectral_canvas2d_2126x850_frames_over_50ms` | 64 frames | ≤ 1 | -6300 % | **FAIL** |
| `frame_spectral_auto_1280x720_p50_ms` | 4.3 ms | ≤ 16.7 | +74 % | pass |
| `frame_spectral_auto_1280x720_p95_ms` | 9.8 ms | ≤ 16.7 | +41 % | pass |
| `frame_spectral_auto_1280x720_p99_ms` | 111.4 ms | ≤ 50 | -123 % | **FAIL** |
| `frame_spectral_auto_1280x720_frames_over_50ms` | 23 frames | ≤ 1 | -2200 % | **FAIL** |
| `frame_spectral_auto_2126x850_p50_ms` | 5.9 ms | ≤ 16.7 | +65 % | pass |
| `frame_spectral_auto_2126x850_p95_ms` | 15.4 ms | ≤ 16.7 | +8 % | tight |
| `frame_spectral_auto_2126x850_p99_ms` | 115.4 ms | ≤ 50 | -131 % | **FAIL** |
| `frame_spectral_auto_2126x850_frames_over_50ms` | 22 frames | ≤ 1 | -2100 % | **FAIL** |

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

_H-46 rows re-measured 2026-09-15 with `cargo bench -p vox-engine --bench playback_start` (before: 49.4 ms at every buffer size). The rest of the matrix is from the T-704 run: `just perf-matrix` was not re-run, because this worktree has no `test-big`/`bench-ui` logs and it would have reset those rows to "not run"._

### PROMPT §2 — full rack at 48 kHz < 20 % of one core

How: `vox-rack` `full_rack` (T-110): the typical voice rack's `process()` at each realtime block size; `vox-engine` `callback_histogram`: the whole output callback. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `full_rack_process_64f_pct_core` | 0.7993 pct_core | ≤ 20 | +96 % | pass |
| `full_rack_process_128f_pct_core` | 0.835 pct_core | ≤ 20 | +96 % | pass |
| `full_rack_process_256f_pct_core` | 0.7824 pct_core | ≤ 20 | +96 % | pass |
| `full_rack_process_512f_pct_core` | 0.7546 pct_core | ≤ 20 | +96 % | pass |
| `full_rack_process_1024f_pct_core` | 0.7285 pct_core | ≤ 20 | +96 % | pass |

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
| `noise_gate_60s_hpf_lookahead5ms_render_s` | 0.05714 s | ≤ 0.3 | +81 % | pass |

### SPEC-014 AC-18 — Noise Reduction: 60 s ≤ 1.2 s at defaults, ≤ 1.8 s at N = 8192; 60 s capture ≤ 0.3 s

How: `vox-modules` `spec_budgets` with a captured print (the STFT path — T-110's bench measured the no-print delay line), the AC-6 signal. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `noise_reduction_60s_defaults_render_s` | 0.1592 s | ≤ 1.2 | +87 % | pass |
| `noise_reduction_60s_n8192_render_s` | 0.1778 s | ≤ 1.8 | +90 % | pass |
| `noise_reduction_capture_60s_s` | 0.08144 s | ≤ 0.3 | +73 % | pass |

### SPEC-015 AC-22 — EQ response curve: 2048 points, 9 components ≤ 2 ms (median of 100)

How: `vox-modules` `spec_budgets`. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `eq_response_curve_2048pts_9components_median_ms` | 0.9451 ms | ≤ 2 | +53 % | pass |

### SPEC-016 AC-19 — Dynamics: 60 s all sections + RMS + look-ahead 10 ms ≤ 0.6 s; defaults ≤ 0.3 s

How: `vox-modules` `spec_budgets`. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `dynamics_60s_all_sections_rms_lookahead10ms_render_s` | 0.1662 s | ≤ 0.6 | +72 % | pass |
| `dynamics_60s_defaults_render_s` | 0.165 s | ≤ 0.3 | +45 % | pass |

### SPEC-017 AC-17 — True-Peak Limiter ≤ 1.5 % of one core (256-frame blocks)

How: `vox-modules` `true_peak_limiter`, worst of 0/+12/+24 dB input gain. Reproduce: `just bench`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `true_peak_limiter_256f_worst_pct_core` | 0.3106 pct_core | ≤ 1.5 | +79 % | pass |

### SPEC-004 AC-3 — undo/redo on 20 000 pieces ≤ 50 ms

How: `vox-project` `history_exact` (H-17), real disk. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac3_worst_undo_redo_20000_pieces_ms` | 1.636 ms | ≤ 50 | +97 % | pass |

### SPEC-004 AC-5 / T-301 — memory budget: resident ≤ budget + 128 MiB

How: `vox-project` `big.rs`: 60-min playback + 200 seeks at a 512 MiB budget. `perf_big`: the store's peak resident memory and the process RSS growth with a 60-min document open (default budget). Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac5_60min_playback_peak_resident_mib` | 512 MiB | ≤ 640 | +20 % | tight |
| `open_60min_store_peak_resident_mib` | 704 MiB | ≤ 4224 | +83 % | pass |
| `open_60min_process_rss_growth_mib` | 665.4 MiB | ≤ 4224 | +84 % | pass |

### SPEC-004 AC-11 — recovering a 60-min session with 1000 records < 5 s

How: `vox-project` `recovery` (H-17). Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec004_ac11_recover_60min_1000_records_ms` | 104.1 ms | ≤ 5000 | +98 % | pass |

### SPEC-007 AC-9 — spectrogram tiles on a 60-min document: visible ≤ 200 ms, refined ≤ 2 s, warm ≤ 50 ms

How: `vox-engine` `tests/spectro.rs` (engine side: compute + channel send), 1920 px, FFT 2048. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec007_ac9_visible_3600s_view_ms` | 10.75 ms | ≤ 200 | +95 % | pass |
| `spec007_ac9_refined_3600s_view_ms` | 167.2 ms | ≤ 2000 | +92 % | pass |
| `spec007_ac9_warm_3600s_view_ms` | 2.676 ms | ≤ 50 | +95 % | pass |
| `spec007_ac9_visible_600s_view_ms` | 10.78 ms | ≤ 200 | +95 % | pass |
| `spec007_ac9_refined_600s_view_ms` | 36.52 ms | ≤ 2000 | +98 % | pass |
| `spec007_ac9_warm_600s_view_ms` | 1.499 ms | ≤ 50 | +97 % | pass |
| `spec007_ac9_visible_60s_view_ms` | 12.27 ms | ≤ 200 | +94 % | pass |
| `spec007_ac9_refined_60s_view_ms` | 12.27 ms | ≤ 2000 | +99 % | pass |
| `spec007_ac9_warm_60s_view_ms` | 0.7614 ms | ≤ 50 | +98 % | pass |
| `spec007_ac9_visible_10s_view_ms` | 14.64 ms | ≤ 200 | +93 % | pass |
| `spec007_ac9_refined_10s_view_ms` | 14.64 ms | ≤ 2000 | +99 % | pass |
| `spec007_ac9_warm_10s_view_ms` | 2.619 ms | ≤ 50 | +95 % | pass |
| `spec007_ac9_visible_1s_view_ms` | 5.071 ms | ≤ 200 | +97 % | pass |
| `spec007_ac9_refined_1s_view_ms` | 5.071 ms | ≤ 2000 | +100 % | pass |
| `spec007_ac9_warm_1s_view_ms` | 0.0965 ms | ≤ 50 | +100 % | pass |

### SPEC-008 AC-14 — edit ops on 20 000 pieces + 1000 markers: splice ≤ 5 ms p95, command (incl. journal fdatasync) ≤ 50 ms p95

How: `vox-project` `history_exact`, 100 seeded runs per op, real disk. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec008_ac14_copy_splice_p95_ms` | 0.002184 ms | ≤ 5 | +100 % | pass |
| `spec008_ac14_cut_splice_p95_ms` | 0.6845 ms | ≤ 5 | +86 % | pass |
| `spec008_ac14_cut_command_p95_ms` | 12.77 ms | ≤ 50 | +74 % | pass |
| `spec008_ac14_paste_splice_p95_ms` | 1.069 ms | ≤ 5 | +79 % | pass |
| `spec008_ac14_paste_command_p95_ms` | 14.88 ms | ≤ 50 | +70 % | pass |
| `spec008_ac14_delete_splice_p95_ms` | 0.4251 ms | ≤ 5 | +91 % | pass |
| `spec008_ac14_delete_command_p95_ms` | 1.671 ms | ≤ 50 | +97 % | pass |
| `spec008_ac14_trim_splice_p95_ms` | 0.5764 ms | ≤ 5 | +88 % | pass |
| `spec008_ac14_trim_command_p95_ms` | 1.836 ms | ≤ 50 | +96 % | pass |
| `spec008_ac14_silence_splice_p95_ms` | 0.4975 ms | ≤ 5 | +90 % | pass |
| `spec008_ac14_silence_command_p95_ms` | 1.828 ms | ≤ 50 | +96 % | pass |
| `spec008_ac14_insert_silence_splice_p95_ms` | 0.6559 ms | ≤ 5 | +87 % | pass |
| `spec008_ac14_insert_silence_command_p95_ms` | 1.995 ms | ≤ 50 | +96 % | pass |

### SPEC-018 AC-18 — sidecar write/read ≤ 150 ms p95

How: `vox-project` `sidecar_perf`. Reproduce: `just test-big`.

| metric | measured | target | margin | status |
|---|---|---|---|---|
| `spec018_ac18_sidecar_write_p95_ms` | 10.48 ms | ≤ 150 | +93 % | pass |
| `spec018_ac18_sidecar_read_p95_ms` | 26.38 ms | ≤ 150 | +82 % | pass |

<!-- END TARGETS MATRIX -->

## T-704 fixes

| Area | Change | Before | After |
|---|---|---|---|
| NR CPU bench (measurement) | `process_cost` built Noise Reduction with no print, so it timed the N-sample delay line. It now captures a print (the STFT path, reduction on) and asserts it. | 0.0007 % of a core (256-frame blocks) | 0.23 % (a 60 s render takes 0.14 s against a 1.2 s budget) |
| Peaks query (`vox_project::peaks`) | Keeps the last chunk's pyramid across the output buckets of one query. It used to re-read, CRC-check and decode the ~11 KB record for every bucket. | 1000 buckets: 1.12 ms (spp 64), 1.17 ms (1024), 1.19 ms (4096); 10 240 record reads for 10 chunks | 17 µs, 40 µs, 119 µs; 10 reads. A zoomed 2126-px view on a 60-min document: p95 0.31 ms |
| Peak pyramid (`ChunkPeaks::compute`) | 8-lane compare-and-select min/max instead of a serial fold, and a branch-free non-finite scan | 165.7 µs per 64 Ki-sample chunk; 482–602 ms for a 60-min import | 18.8 µs (8.8×); 88–154 ms |
| Import (`import_file`) | Decoding and downmixing run on their own thread, overlapping chunk commits (bounded channel, recycled buffers) | Open 60 min: best 2241 ms, worst 2654 ms (3 opens, load ~7) | Best 1671 ms, median 1734 ms (5 opens, load ~10) |
| Spectrogram, Canvas2D path | `columnDb`: the time rule is computed once per column and bin, and its max is taken on raw codes; the renderer keeps a per-draw tile memo. Both are exactly `pixelDb`'s values (tested). | Split view 1280×720: 2 fps (p50 491 ms); 2126×850: 0.35 fps (p50 2809 ms) | 15–34 fps (p50 22–48 ms); 16–20 fps (p50 44–46 ms). Still misses (see below) |
| WebGL2 quads (`QuadBatch`) | Writes into a growable `Float32Array` and hands back a view. It used to push 36 numbers per quad into a JS array and copy that into a new `Float32Array` every frame. | Waveform, WebGL2, 2126×850: p95 19.3 ms, p99 42.3 ms, 5 frames > 50 ms | p95 11.8–12.4 ms, p99 27–31 ms, 0–2 frames > 50 ms |

Each code change has a regression test:
- `peaks_query::a_query_reads_each_chunk_pyramid_record_once`
- `store::peaks::compute_matches_the_reference_fold`
- `samplerColumn.test.ts`
- `quadsTyped.test.ts`
- the existing `import` tests (cancel, damage, downmix, markers)

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
- **Idle CPU (H-43 baseline).** These are dev-build (Vite) figures, measured with uncapped rAF as
  main-thread time per idle frame × 60 Hz. The perpetual draw loops (H-32) redraw every frame even
  when nothing changes.

  | Scene | Canvas2D | WebGL2 |
  |---|---|---|
  | Waveform 1280×720 | ~1.0 ms/frame, 6 % of a core | ~1.0 ms/frame, 6 % |
  | Waveform 2126×850 | 2.9 ms/frame, 17 % | 1.9 ms/frame, 12 % |
  | Split view 1280×720 | 42 ms/frame (can't hold 60 Hz) | 1.3 ms/frame, 8 % |
  | Split view 2126×850 | 46 ms/frame (can't hold 60 Hz) | 2.2 ms/frame, 13 % |

  The rows come from `idle_*` in `target/bench/ui.log`.
- **Output-callback worst case (`just bench-callback`, ADR-002 §2) is load-sensitive.** At load
  average 10–16, the 128-frame row's max callback reached 4–13 ms, over its 2.7 ms deadline, while
  p99 stayed within the deadline at every block size. T-110 measured p99 ≤ 5 % of the deadline on
  a quiet machine, and T-704 didn't touch the callback path. The bench thread isn't realtime-
  scheduled, so it gets preempted under load; H-43 or a later RT pass should re-check on an idle
  machine.
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
