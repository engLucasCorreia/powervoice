# ADR-003 — IPC data paths & shared types
- Status: accepted (owner, M0 checkpoint 2026-09-12)
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
The UI needs three kinds of data from the Rust core:
- small request/response traffic;
- low-rate state changes;
- bulk or streamed binary: peaks for a 60-minute document, spectrogram tiles, live recording peaks,
  and 30–60 Hz playhead and meters.

Tauri 2.11 offers the following (https://v2.tauri.app/develop/calling-rust/,
https://v2.tauri.app/develop/calling-frontend/):
- **Commands**, with JSON by default.
- **`ipc::Response`**, which returns raw bytes without JSON encoding.
- **Events.** Payloads are always JSON strings, and "the event system is not designed for low latency
  or high throughput situations".
- **Channels.** They "are designed to be fast and deliver ordered data". A
  `Channel<InvokeResponseBody>` can send `InvokeResponseBody::Raw(Vec<u8>)`
  (https://docs.rs/tauri/latest/tauri/ipc/enum.InvokeResponseBody.html).

CLAUDE.md forbids JSON float arrays for bulk data and hand-copied TS types.

## Decision

### 1. Which mechanism for what

| Data | Mechanism | Encoding | Rate |
|---|---|---|---|
| Requests & mutations (open, edit, transport, rack edits, settings, `clock_now_ns`) | command | JSON args → `Result<T, IpcError>` | on demand |
| Parameter changes | commands `set_param_normalized` / `set_param_text` (ADR-005) | JSON | while dragging, the UI throttles to one per animation frame, latest value wins |
| Low-rate state: `document_changed {rev, audio_rev, len, rate, dirty}`, `transport_state`, `devices_changed`, `rack_changed`, `param_changed` (ADR-005), `job_progress`, `peaks_progress`, `notice` | event | JSON (generated types) | ≤ 10 Hz each |
| Peaks or raw samples for a view | command → `ipc::Response` | binary `VXPK` | on demand |
| Spectrogram tiles | `Channel` per spectral view | binary `VXST`, one message per tile | streamed |
| Playhead + meters | one `Channel` (`telemetry_subscribe`) | binary `VXTM` | **30 Hz** |
| Live recording peaks | `Channel` (`recording_subscribe`) | binary `VXRP` | 30 Hz |
| Later: module telemetry (gain reduction), output analyzer | `Channel` | same header convention, new magic | 30 Hz |

**Telemetry.** Playhead and meters travel in one channel frame instead of events. One ordered binary
message at 30 Hz is cheaper than two JSON events, and the ticket permits it. Meter values are
**max-hold since the previous frame**, so no peak is lost at 30 Hz. Ballistics and decay are drawn per
animation frame. T-007 measures WebKitGTK channel cost. If 60 Hz is free, the rate becomes a
setting.

### 2. Binary conventions
- All multi-byte fields are little-endian. Every supported target is LE, which is asserted at
  compile time.
- **Common prefix** (8 bytes): `magic [u8;4]`, `version u16`, `header_len u16`.
- The payload starts at `header_len`, which is always a multiple of 8. Readers must use `header_len`,
  not a constant. New fields may therefore be appended to a header without a version bump.
  Incompatible changes bump `version`.
- TS decoders (`ui/src/lib/ipc/binary.ts`) read headers with `DataView` and view payloads with typed
  arrays, with no copy.
- Every document-derived payload carries **`audio_rev`** (ADR-004). The UI drops any response whose
  `audio_rev` is not current. Marker-only edits do not change `audio_rev`.

**`VXPK` — peaks / raw samples** (response to `peaks_get`)

| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXPK"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 48 |
| 8 | u32 | request_id |
| 12 | u32 | flags: bit0 `RAW` (payload = f32 samples, spp = 1); bit1 `PARTIAL` (some buckets not computed yet) |
| 16 | u64 | audio_rev |
| 24 | u64 | start_sample |
| 32 | u32 | samples_per_bucket (spp) |
| 36 | u32 | count (buckets, or samples if `RAW`) |
| 40 | u32 | sample_rate_hz |
| 44 | u32 | reserved = 0 |
| 48 | f32[] | `RAW`: `count` samples. Otherwise `count × (min, max)`. Buckets not yet ready are `(NaN, NaN)`. |

- **Levels.** spp ∈ {64, 256, 1024, 4096, 16384, 65536}, matching the per-chunk pyramid in ADR-004.
  Coarser levels (×4 steps) are derived per request.
- **UI choice.** The UI picks the largest level with spp ≤ its pixel spp and reduces at most 4 buckets
  per pixel. Below 64 spp it requests `RAW`.
- **Limits.** The server caps a request at 65 536 buckets or 1 Mi samples (4 MiB).
- **Precision.** Values are f32, not i16, so float documents above 0 dBFS display correctly. A
  4000-px view is 32 KB.
- **Request:**
  `peaks_get(PeaksRequest { request_id, audio_rev, spp, start_sample, count }) -> Result<Response, IpcError>`.

**`VXST` — spectrogram tile** (streamed on the view's channel)

| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXST"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 64 |
| 8 | u32 | request_id |
| 12 | u32 | flags: bit0 `LAST` (final tile of this request) |
| 16 | u64 | audio_rev |
| 24 | u64 | first_frame_center_sample (frame *i* is centred at this + *i*·hop) |
| 32 | u32 | hop_samples |
| 36 | u32 | fft_size |
| 40 | u32 | frames (tile width, ≤ 256) |
| 44 | u32 | bins (= fft_size/2 + 1) |
| 48 | f32 | q_floor_db = −150 |
| 52 | f32 | q_ceil_db = +6 |
| 56 | u32 | tile_index |
| 60 | u32 | window (0 = Hann) |
| 64 | u8[] | `frames × bins`, frame-major, bin 0 = DC |

- **Quantization.** `v = round(255 · clamp((dB − q_floor)/(q_ceil − q_floor), 0, 1))`, with dB
  normalized so that a full-scale sine reads 0 dB. That is a fixed 0.61 dB step.
- **GPU-side work.** The UI uploads each tile as an R8 texture. The display floor/ceiling, log/linear
  frequency mapping and colormap are all shader work, so changing any of them never refetches.
- **Requests.**
  - `spectro_attach(view_id, channel)` once per view.
  - `spectro_request(view_id, SpectroRequest { request_id, audio_rev, fft_size, hop, window, tiles })`,
    with visible tiles first.
  - A newer `request_id` cancels unsent tiles from older requests.
- **Caching.** The tile cache key is content-based: a hash of the `(chunk_id, offset, len)` runs
  covering the tile's input span, plus the parameters. An edit invalidates only the tiles it touches.

**`VXTM` — telemetry frame** (header only, no payload yet)

| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXTM"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 72 |
| 8 | u32 | seq |
| 12 | u32 | flags: bit0 `PLAYING`, bit1 `RECORDING`, bit2 `MONITORING`, bit3 `LOOPING`, bit4 `XRUN` (since last frame), bit5 `OUT_CLIP`, bit6 `IN_CLIP` |
| 16 | u64 | playhead_sample (heard position, doc samples; ADR-002 §8) |
| 24 | u64 | playhead_time_ns (app monotonic clock) |
| 32 | f64 | rate (doc samples/s; the document rate while playing, 0 when stopped) |
| 40 | f32 | out_peak_dbfs (post-rack, max since last frame; −∞ allowed) |
| 44 | f32 | out_rms_dbfs (window per meter spec) |
| 48 | f32 | in_peak_dbfs |
| 52 | f32 | in_rms_dbfs |
| 56 | u64 | audio_rev |
| 64 | u32 | dropped_rt_events |
| 68 | u32 | reserved = 0 |

**`VXRP` — live recording peaks**

| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXRP"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 40 |
| 8 | u32 | take_id |
| 12 | u32 | flags: bit0 `FINAL` |
| 16 | u64 | start_sample (doc position of the first bucket) |
| 24 | u32 | samples_per_bucket = 64 |
| 28 | u32 | count |
| 32 | u64 | take_len_samples |
| 40 | f32[] | `count × (min, max)`, appended since the previous message |

### 3. Playhead extrapolation & clock sync
- **Rust side.** Rust timestamps are ns since the process `APP_EPOCH` (ADR-002). JS reads u64 fields
  as `Number`, which is exact for up to 104 days of uptime.
- **Clock sync.** At start-up and then every 30 s, the UI calls `clock_now_ns` five times:
  - it records `t0 = performance.now()` before the call and `t1` after;
  - it keeps the sample with the smallest RTT;
  - `offset = server_ns − (t0 + t1)/2`.
- **Extrapolation.** Each animation frame computes
  `pos = playhead_sample + (now_app_ns − playhead_time_ns) · rate / 1e9` while `PLAYING`:
  - clamped to the document length;
  - wrapped inside the loop range when `LOOPING`.
- **New anchors.** When a new anchor differs from the prediction by < 20 ms of audio, the UI slews
  over 100 ms. Otherwise it jumps.

### 4. Shared Rust→TS types: **`ts-rs`**
Check performed on 2026-09-12:

| Check | Finding | Source |
|---|---|---|
| Latest tauri-specta | `2.0.0-rc.25` (2026-05-08), pre-release. The latest stable is 1.0.2 (2023, Tauri 1). | https://crates.io/crates/tauri-specta/versions |
| Its requirements | `tauri ^2` with feature `specta`; `specta =2.0.0-rc.25` (exact pin); `specta-typescript ^0.0.12` | https://crates.io/api/v1/crates/tauri-specta/2.0.0-rc.25/dependencies |
| Tauri 2.11.5 | Has a `specta` feature requiring `specta ^2.0.0-rc.16`, which on paper resolves to rc.25 | https://docs.rs/crate/tauri/latest/features |
| Stability | Still RC; the release notes list breaking changes between RCs (rc.23, rc.24) | https://github.com/specta-rs/tauri-specta/releases |
| ts-rs | `12.0.1`, stable. Several types may share one `export_to` file (since 10.0.0). serde-compat on by default. | https://crates.io/crates/ts-rs · https://github.com/Aleph-Alpha/ts-rs/blob/main/CHANGELOG.md |

Conclusion: tauri-specta *probably* resolves against Tauri 2.11, but no build can confirm it here (no
toolchain yet). It is also a pre-release with exact pins that break between RCs. Per the ticket rule
("if unsure, choose `ts-rs`"), **we use `ts-rs 12`**. Binary payloads, which are our heaviest traffic,
fall outside what either tool types anyway.

Mechanics:
- **DTO location.** DTOs live in `src-tauri/src/ipc/` and derive `Serialize`/`Deserialize` +
  `ts_rs::TS` with `#[ts(export, export_to = "bindings.ts")]`. Domain crates do not depend on
  `ts-rs`. `src-tauri` maps domain → DTO with `From` impls, which the compiler checks.
- **Command names.** A single `ipc_commands!(app_info, peaks_get, …)` macro expands to
  `tauri::generate_handler![…]` and a `COMMAND_NAMES` list. A unit test asserts that the list equals
  the variants of the generated `CommandName` string union. Event names follow the same pattern
  (`EventName`). Commands and events are both snake_case.
- **Wrappers.** `ui/src/lib/ipc/commands.ts` holds typed wrappers written by hand
  (`invoke<AppInfo>('app_info' satisfies CommandName)`). They use generated types only.
- **Errors.** `IpcError { code: IpcErrorCode, key: string, params: Record<string, string> }`. The key
  is an i18n key; T-104 fills in the details.
- **Integers.** Doc positions are `u64` in Rust and `number` in TS (`TS_RS_LARGE_INT=number`). They
  stay < 2^53, which is 5.9 years at 48 kHz.
- **`just gen-types`.** Runs
  `TS_RS_EXPORT_DIR=ui/src/lib/ipc TS_RS_LARGE_INT=number cargo test -p powervoice-app export_bindings`.
  The `gen_ipc_fixtures` test also writes golden `VXPK`/`VXST`/`VXTM`/`VXRP` frames to
  `ui/src/lib/ipc/__fixtures__/`.
- **Staleness check.** `just check` generates into a temporary directory and runs `diff -r` against the
  checked-in files. Any difference fails with "run `just gen-types`". Vitest decodes the golden
  fixtures with `binary.ts` and asserts the known field values, which makes it a Rust↔TS layout
  contract test.
- **Formatting.** `bindings.ts` starts with a "generated — do not edit" banner and is excluded from
  formatters.

## Consequences
**Positive**
- There is no JSON on any bulk path.
- One binary header convention serves every stream and can be extended.
- Stale data is rejected by `audio_rev`.
- The types toolchain is stable and not coupled to Tauri's release cadence.
- Rust↔TS binary layouts are verified by tests on both sides.

**Negative**
- ts-rs does not type command signatures, so we keep hand-written wrappers plus the command-name test.
- Domain → DTO mapping is boilerplate.

**Follow-ups**
- T-004 sets up ts-rs and `just gen-types`.
- T-007 must confirm on WebKitGTK that raw channel and `Response` payloads arrive as `ArrayBuffer`,
  and measure the cost of 30 vs 60 Hz channels and of 4 MiB responses.
- T-203/T-204/T-108 implement the layouts above.

## Alternatives considered
- **tauri-specta**: typed commands would be nice. Rejected for now because it is pre-release with
  exact pins (see the table). Revisit with a superseding ADR when 2.0 is stable.
- **Events for playhead and meters**: JSON-only and explicitly not meant for throughput. Rejected in
  favour of one binary channel.
- **A custom URI-scheme protocol** (`register_uri_scheme_protocol`) serving peaks over `fetch`: it
  would work and could use HTTP caching, but it adds a second request path. Kept as a fallback if
  T-007 finds `Response` slow.
- **i16 peaks**: they halve the size but clip the display of f32 content above 0 dBFS. Rejected.
- **Types in domain crates behind a `ts` feature**: fewer DTOs, but it spreads serde/ts-rs into
  real-time crates. Rejected.

## Open questions
- None for the owner.
- ~~For T-007: the 30 Hz vs 60 Hz telemetry default~~ — **resolved by ADR-009 §3: telemetry default is 60 Hz** (measured free on the owner's machine), with a settings toggle to 30 Hz. ADR-009 also confirmed raw `Response`/`Channel` payloads arrive as `ArrayBuffer`.

## Amendment 1 — T-200 (SPEC-005/006/007), 2026-09-13
- **Rates:** telemetry-cadence channels (VXTM, VXRP, module telemetry, analyzer) run at the telemetry
  rate, **60 Hz by default** (ADR-009), 30 Hz selectable.
- **`VXST` flags:** bit1 `PREVIEW` — a reduced-resolution tile sent first for responsiveness; the
  full tile for the same `tile_index` replaces it when ready.
- **Tile invalidation wording:** an edit invalidates the tiles whose input content changes.
  Length-changing edits (cut, paste, insert) re-key every later tile; cached tiles are still reused
  when their content key (chunk runs + parameters) matches.
- **New frame `VXSA` — live output analyzer** (channel `analyzer_subscribe`, telemetry rate):

  | Off | Type | Field |
  |---|---|---|
  | 0 | `[u8;4]` | `"VXSA"` |
  | 4 | u16 | version = 1 |
  | 6 | u16 | header_len = 32 |
  | 8 | u32 | seq |
  | 12 | u32 | flags: bit0 `HAS_PEAK_HOLD` |
  | 16 | u32 | bands (number of 1/24-octave bands) |
  | 20 | u32 | fft_size |
  | 24 | u32 | sample_rate_hz |
  | 28 | u32 | reserved = 0 |
  | 32 | f32[] | `bands` levels in dBFS (−inf allowed), then, if `HAS_PEAK_HOLD`, `bands` peak-hold levels |

  Band centre frequencies are derived deterministically from `bands`, `fft_size` and the rate
  (SPEC-007 §4); they are not transmitted.
- **New event** `clipboard_changed { has_audio, len_samples, rate_hz }` (SPEC-008).

## Amendment 2 — T-300 (SPEC-022), 2026-09-13
New events: `record_phase { take, phase: PreRoll | Recording | PostRoll | Finished | Cancelled, at_sample }`
and `record_finished { take, outcome, len_samples, offset_samples }`. Calibration commands
(`calibration_start`, `calibration_verify`, `calibration_cancel`) report progress through `job_progress`.

## Amendment 3 — slices + hardening (S3-07, S4-01, S4-04, H-03, H-09), 2026-09-14
Records what the vertical slices built, as implemented.
- **New frame `VXMT` — module telemetry** (SPEC-016 §4.12; the table's reserved "module telemetry"
  row). Channel `module_telemetry_subscribe`, one frame per control tick at the telemetry rate, sent
  only while a subscriber exists and at least one slot has a `Telemetry` extension. The engine's meter
  publisher is the single reader of every handle; channel descriptions (`TelemetryInfo`) travel once
  with the rack-state DTOs (`RackSlotDto.telemetry`).

  | Off | Type | Field |
  |---|---|---|
  | 0 | `[u8;4]` | `"VXMT"` |
  | 4 | u16 | version = 1 |
  | 6 | u16 | header_len = 32 |
  | 8 | u32 | seq |
  | 12 | u32 | flags = 0 (reserved) |
  | 16 | u64 | frame_time_ns (app clock) |
  | 24 | u32 | record count R |
  | 28 | u32 | reserved = 0 |
  | 32 | R × {u32 slot_uid, u16 count, u16 reserved, f32[count]} | values in `channels()` order |

  Golden fixture: `ui/src/lib/ipc/vxmt_fixture.ts` (Rust-generated).
- **Interim JSON:** `rack_response_curve(slot, freqs ≤ 512)` returns `{ freqs_hz, total_db,
  components_db }` as JSON (S3-07). The binary `VXRC` frame stays the hardening target; this is the
  one sanctioned exception to "bulk data is binary" until then (≤ 512 × (C + 1) floats).
- **Jobs:** `job_progress { job_id, kind, state, fraction }` is the single progress channel. Kinds so
  far: `Export`, `NrCapture`, `LoudnessAnalyze`, `NormalizePeak`, `NormalizeLufs`. A job whose result
  is more than a fraction carries it in its own event, tagged with `job_id`: `loudness_report`
  (S4-01), `normalize_result` (H-09).
- **Write-back jobs** (H-09 normalize) refuse concurrent audio edits with `IpcErrorCode::Busy`
  (`error.document_busy`) and commit only if the document snapshot and session are unchanged.

## Amendment 4 — T-208 (live analyzer, as implemented), 2026-09-14
Amendment 1's `VXSA` table (header_len 32, `HAS_PEAK_HOLD` flag, peak-hold payload) predates
SPEC-007 §4.9's fuller, later version and lacks the fields AC-18/AC-19 need (`RESET`/`DROPPED`/
`SILENT`, `frame_time_ns`). **SPEC-007 §4.9 is the layout actually implemented; this amendment
supersedes Amendment 1's `VXSA` table.**

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
| 48 | f32[K] | averaged band level, dB (`-inf` allowed, never NaN) |

- **Peak hold is not transmitted.** SPEC-007 §4.8 step 7 says peak hold is UI ballistics, computed
  per animation frame from the received levels (`ui/src/lib/analyzer/peakHold.ts`), the same as the
  meters. `HAS_PEAK_HOLD`-shaped payload extensions are therefore unnecessary; if a future ticket
  needs server-computed peak hold, add a new flag bit and trailing `f32[K]` rather than reusing
  Amendment 1's bit0 (repurposed here as `RESET`).
- **Multi-subscriber, one computation.** `analyzer_subscribe(channel, response) -> id`,
  `analyzer_set_response(id, response)`, `analyzer_unsubscribe(id)`. One BH4 FFT + band reduction
  runs per telemetry tick, feeding all three Fast/Medium/Slow EMA states at once; each subscriber
  reads the state matching its own `response` from that one shared computation (unlike `VXMT`,
  which has exactly one sink). `ANALYZER_ON` (an `Arc<AtomicBool>` gate the RT tap checks first)
  follows subscriber count: clear whenever none remain.
- **Tap point.** The RT tap pushes the exact `out` signal the output meter's peak/RMS are built
  from (`crates/engine/src/output.rs`, ADR-002 §4 step 3.4/4: rack output + Dry monitor, before
  format conversion and channel duplication), one bounded `rtrb` push per sub-block; a full ring
  writes what fits and bumps an atomic drop counter (surfaces as `DROPPED` on the next frame).
- **Reset.** Every successful output (re)open attaches a fresh ring to the analyzer publisher and
  sets `pending_reset`, so a device reopen or sample-rate change automatically clears the FFT
  history and EMA state and marks the next frame `RESET` — no separate detection was needed beyond
  "a new ring was attached."
- **`TELEMETRY_RATE` gap (pre-existing, not introduced here).** The 60 Hz cadence is the fixed
  control-tick period (`crates/engine/src/control.rs::TICK`); `Settings.telemetry_rate_hz`'s 30 Hz
  option is stored but not wired to the tick loop for any channel (`VXTM`/`VXMT`/`VXSA` alike).
  Wiring it is a separate ticket.
- **Golden fixture:** `ui/src/lib/ipc/vxsa_fixture.ts` (Rust-generated, `vox_engine::AnalyzerFrame`).
