# SPEC-010 — Peak normalize favorites

- **Status:** approved (autonomous, T-300)
- **Milestone:** M3 (T-305, Sonnet + Opus review). Depends on T-301 (undo, journal, `ChunkWriter`
  jobs) and uses the SPEC-006 selection.
- **Related:** SPEC-000 (glossary; testkit conventions), SPEC-004 (undoable, all-or-nothing, progress
  and cancel, stop-before-commit, refused while recording; AC-2 and AC-4 use normalize as their
  example), SPEC-008 (the other M3 destructive ops; enablement, busy state, stale-revision check),
  SPEC-012 (rack: non-destructive; offline render), M6 specs (LUFS normalize, true peak, bake, T-601,
  T-602, T-603) · ADR-004 §2–§5 (`ChunkWriter`, chunk commit, per-chunk peaks), §6 (journal) · ADR-001
  §4 (`project` owns "edit ops (incl. normalize gain computation)") · ADR-002 (workers, generation-token
  cancellation) · ADR-003 (`job_progress`, `notice`) · PROMPT §2 (LOCKED: *normalize favorites are
  one-click destructive actions on the selection, or the whole file if there is no selection, fully
  undoable*), §3.3, §3.5, §5 (the "−1 dB → −1.00 dBFS ± 0.01 dB" example)

## 1. Purpose
Before a voice-over goes anywhere, its level must sit at a known ceiling:
- −1 dB is a safe general-purpose peak;
- −3 dB is the ACX peak limit (PROMPT §3.5);
- −0.1 dB is "as loud as possible without clipping".

Audition users do this dozens of times a day from its Favorites menu. PowerVoice makes it **one click**:
- no dialog;
- applied to the selection, or to the whole file when nothing is selected;
- exact to a hundredth of a dB;
- impossible to clip;
- undone with Ctrl+Z.

A **Normalize…** dialog covers any other target.

## 2. Behavior / UX

### 2.1 Commands
| Command | Target (sample peak) | Interaction |
|---|---|---|
| **Normalize to −1 dB** | −1.00 dBFS | one click, no dialog |
| **Normalize to −0.1 dB** | −0.10 dBFS | one click, no dialog |
| **Normalize to −3 dB** | −3.00 dBFS | one click, no dialog |
| **Normalize…** | custom (§2.4) | dialog, then Apply |

- **Scope.** The scope is the **non-empty selection** `[S, E)` if one exists. Otherwise it is **the
  whole file** `[0, L)`. An empty selection `[S, S)` counts as no selection (SPEC-008 §2.2). This is
  LOCKED in PROMPT §2.
- **One click, no confirmation.** A favorite starts at once and ends in one undo entry. There is
  nothing to confirm: the result is undoable, and a normalize cannot clip (§2.3).
- **Enablement.** The four commands are enabled when a document with `L > 0` is open. They are disabled
  (tooltip + command error):
  - while recording (`error.not_while_recording`, SPEC-004 §2.3);
  - while another document job runs (`error.document_busy`).
- **Selection, cursor and markers** are unchanged by a normalize. The length doesn't change, so no
  marker moves (SPEC-008 §2.4).

### 2.2 What "normalize" means here
- **Sample peak.** The target is the **sample peak**: `P = max |x[i]|` over the scope, in dBFS
  (`20·log10 P`), which is exactly what testkit's `peak_dbfs` and `powervoice-cli analyze` report.
- **One gain.** The operation multiplies every sample in the scope by one gain `g = T / P`, where
  `T = 10^(target_dB / 20)`. The scope's new sample peak equals the target, and every level inside the
  scope moves by the same amount, so relative levels are preserved exactly.
- **Up or down.** The gain may be above or below 1. Float content above 0 dBFS (overs), or a quiet
  take, both land on the target.
- **Only the scope changes.** Samples outside the scope are untouched, bit for bit. There is no ramp
  at the scope edges, for the same reasons as SPEC-008 §2.8.
- **No true-peak option.** **Decided (autonomous, T-300): sample peak only in v1.**
  - True-peak (dBTP, 4× oversampled) targeting belongs with LUFS normalize, the loudness meter and the
    true-peak limiter in M6 (PROMPT §3.3, §3.5). Those specs already need a characterised true-peak
    meter (MEMORY follow-up for SPEC-017: `ebur128` reads ~+0.1 dB high near fs/4).
  - Shipping a true-peak normalize in M3 would duplicate that work before it is characterised.
  - Users who need a dBTP ceiling use the rack's true-peak limiter.
- **No DC bias adjust.** **Decided (autonomous, T-300): out of scope for v1.** Audition's Normalize has
  "DC bias adjust" (Adobe help, via search summary).
  - Modern interfaces rarely produce meaningful DC offset.
  - PowerVoice's non-destructive answer is the EQ's high-pass filter (M4), which removes DC along with
    rumble.
  - Peak normalize stays correct with DC present: `P` is the larger of the two polarities' peaks, so
    the asymmetric waveform still never exceeds the target.
- **Mono only.** Audition's "normalize all channels equally" does not apply (PROMPT §2).

### 2.3 Clipping is impossible by construction
- **Target range.** The target is limited to ≤ 0 dBFS (§2.4), so `T ≤ 1`.
- **Proof.** For every sample, `|x| ≤ P`, so `|x · g| ≤ P · g = T ≤ 1` in exact arithmetic. PowerVoice
  computes the product in f64 and rounds once to f32 (round to nearest). A value ≤ 1.0 can never round
  above 1.0, because 1.0 is representable. The resulting peak is `f32(T)` to within 1 f32 ulp, and no
  sample exceeds `f32(T)` rounded up by 1 ulp.
- **Consequence.** No normalize ever produces a sample with `|y| > 1.0`, whatever the input, including
  float input far above 0 dBFS.

### 2.4 Normalize… dialog
- **Opening.** Effects → Normalize… (also at the bottom of the Favorites menu) opens a small modal
  dialog:
  - **Normalize peak to** [value] [unit: **dB** | **%**]. The unit is a two-way toggle.
  - **Scope** readout: "Selection (0:01.200 – 0:03.400)" or "Whole file".
  - **Apply** and **Cancel**. Enter = Apply, Esc = Cancel.
- **dB mode.** −60.00 … 0.00 dBFS, step 0.01.
- **% mode.** 0.1 … 100.0 %, step 0.1, where 100 % = 0 dBFS and `dB = 20·log10(p / 100)`, so
  50 % ≈ −6.02 dB and 0.1 % = −60 dB.
  - **Decided (autonomous, T-300):** 0 dBFS is the upper bound so that §2.3 holds for every target.
    −60 dBFS (0.1 %) is the lower bound because a lower peak target has no voice-over use.
- **Switching units** converts the shown value (−1.00 dB ↔ 89.1 %), rounded to the new unit's step.
- **Which value is used.** The value is used **in the unit it was entered in**. Typing "−1" in dB mode
  gives exactly `T = 10^(−1/20)`. There is no double rounding through the other unit.
- **Invalid input.** Unparseable or out-of-range text shows an error outline and disables Apply.
  Parsing is locale-neutral and accepts `−` as a minus sign (SPEC-012 §2.6 text rules).
- **Default and memory.** The default is −1.00 dB. The last applied value and unit are remembered in
  settings across restarts.
  - **Decided (autonomous, T-300):** custom targets are usually a personal house standard (e.g.
    −2 dB), so remembering them saves retyping.
- **No preview and no peak readout.** The dialog shows neither a preview button nor the scope's current
  peak.
  - **Decided (autonomous, T-300):** the peak of a 60-min scope is a job in itself. The loudness and
    analysis panel (M6) is the place for measurement, and the result is undoable.

### 2.5 Where it lives in the UI
- **Toolbar.** A **Normalize** group sits in the top toolbar (PROMPT §3.6) with three compact buttons:
  "−1 dB", "−0.1 dB", "−3 dB", in PROMPT §3.3 order. Tooltip: "Normalize peak to −1.0 dBFS (selection,
  or whole file)".
- **Favorites menu** (top level, as in Audition):
  - Normalize to −1 dB;
  - Normalize to −0.1 dB;
  - Normalize to −3 dB;
  - a separator;
  - Normalize….

  M6 adds the LUFS favorites (−16, −19, −23 LUFS, PROMPT §3.3) to this menu. Audition's own default
  Favorites include "Normalize to −0.1 dB" and "Normalize to −3 dB" (single secondary source:
  nobledesktop.com, via search summary).
- **Effects → Normalize…** opens the dialog, mirroring Audition's Effects → Amplitude → Normalize.
- **Shortcuts: none.** **Decided (autonomous, T-300): no keyboard shortcut for any normalize command
  in v1.**
  - No source for Audition's defaults (tutorialtactic.com, killerkeys.com, pie-menu.com) lists a
    binding for Normalize or for Favorites, so there is nothing to be compatible with.
  - Inventing keys now risks colliding with SPEC-019's (M7) audit.
  - SPEC-019 may assign some. The keymap registry must contain no normalize binding in M3.

### 2.6 Rack interplay
- **Source only.** Normalize changes the **document's source audio**. The effects rack is **not**
  applied and not changed.
- **Consequence.** After a normalize to −1 dB, the processed output (playback, export) is the rack
  applied to the normalized source. Its peak is **not** −1 dBFS whenever the rack changes level (Gain,
  EQ boosts, compression makeup).
- **Notice.** The first time per app session that a normalize runs while the rack is **active** (at
  least one slot not bypassed; whole-rack A/B ignored, since offline renders ignore it, SPEC-012 §2.3),
  an info notice appears:
  > "Normalize changed the source audio. The effects rack still processes it on playback and export,
  > so the output peak may differ."

  From M6 (T-602) it adds: "To normalize the processed sound, bake the rack first (Effects → Bake
  Rack), then normalize."
- **Normalizing the processed sound** is **Bake rack → Normalize** (M6), or the true-peak limiter in
  the rack. SPEC-010 never renders the rack.

### 2.7 Silent, near-silent, invalid and already-normalized input
The peak scan (§4, pass 1) runs first. Then:

| Scope's sample peak `P` | Result |
|---|---|
| `P = 0` (digital silence, `-inf` dBFS; includes all-`Silence` pieces) | **no-op**; notice `notice.normalize_silent`: "Nothing to normalize: the selection is silent" (or "the file is silent") |
| `0 < P < −120 dBFS` | **no-op**, same notice |
| any non-finite sample in scope | **refused**; notice/error `error.normalize_non_finite`: "The selection contains invalid samples" |
| `|20·log10 g| < 0.001 dB` | **no-op**; notice `notice.normalize_already`: "Already at −1.0 dBFS" |
| otherwise | normalize |

- **No-op means nothing changes:** same `rev`, no undo entry, no journal record, no chunks kept.
- **Decided (autonomous, T-300): −120 dBFS is the near-silence floor.** Below it the content is
  numerical residue, below 24-bit quantization and the project's own −120 dBFS "equal" tolerance
  (`RT_OFFLINE_TOL`, SPEC-000). Amplifying it by more than 119 dB would only manufacture noise.
- **Between −120 and −60 dBFS,** normalize proceeds with no extra warning. The user asked for it, and
  Ctrl+Z undoes it.
- **Non-finite samples** should never exist in a document. The check is defensive, so that an
  `inf · 0` or `NaN` never reaches the store.
- **Decided (autonomous, T-300): the 0.001 dB "already there" tolerance** avoids rewriting up to
  691 MB and adding an undo step for an inaudible change. 0.001 dB is 10× tighter than the AC
  tolerance, so the AC still holds.

### 2.8 Progress, cancel, playback, recording, undo
- **Stop at command start.** **Decided (autonomous, T-300):** playback stops, with the ~5 ms fade and
  acknowledgement, **when the command starts**, before the peak scan, not only before the commit.
  - The click is a destructive edit.
  - The reader's disk I/O would compete with the job's.
  - The modal progress dialog blocks the transport anyway.

  This is stricter than, and compatible with, SPEC-004's "stops before commit". It applies to no-ops
  too; playback does not resume by itself. It is an engine-initiated stop, so the playhead stays at
  the heard position (SPEC-003 AC-9), which is also where it stays after the normalize.
- **Progress dialog.** The normalize runs as a job on a worker (ADR-002). If it is still running after
  **250 ms**, a modal progress dialog "Normalizing…" appears with a determinate bar and **Cancel**
  (Esc):
  - progress arrives through `job_progress` at ≤ 10 Hz (ADR-003) and is monotonic;
  - pass 1 (peak scan) covers 0–30 % and pass 2 (write) 30–100 %;
  - it reaches 100 % only at the commit.

  Short selections finish before the dialog would appear, so a favorite on a phrase feels instant.
- **Cancel or failure** (SPEC-004 §2.1, AC-2):
  - the document is exactly as before: `rev`, `audio_rev`, audio hash, undo and redo depths;
  - there is no undo entry;
  - chunks already written become unreachable and are reclaimed by compaction;
  - Cancel takes effect within **100 ms**;
  - an I/O failure also posts a notice.
- **Refused while recording** (§2.1).
- **Undo.** One undo entry, `label_key` = `history.normalize` (en: "Normalize"). Undo restores the
  exact previous samples by snapshot (SPEC-004), and redo restores the normalized ones.
  - The label carries no target value, because ADR-004 §6 journal records have a `label_key` without
    parameters. See §7.
- **Memory.** Resident memory stays within the SPEC-004 budget: a whole-file normalize holds a few
  chunks in RAM (ADR-004 §3), and 100 of them remain undoable (SPEC-004 AC-4).

### 2.9 Performance
- **Whole file.** A **whole-file normalize of a 60-min, 48 kHz document** (172.8 M samples,
  ≈ 691 MB f32) completes in **≤ 10 s**, from the click to the committed snapshot. That includes both
  passes, chunk commits with CRC and peaks, and the journal `fdatasync`.
- **Measured on** the owner's reference machine (the one used for SPEC-006 AC-19), with the session
  store on its NVMe SSD and the fixture freshly imported from `just fixtures`, without dropping the page
  cache. That is the realistic state right after opening a file.
- SATA SSDs and HDDs are not gated (SPEC-004 OD-3: open/edit targets are SSD/NVMe-only).
- **Selection normalizes** are proportional to the selection's length. A 10 s phrase completes in
  < 250 ms, so no dialog appears.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `favorite_targets` | Favorite targets | dBFS (sample peak) | — | −1.0, −0.1, −3.0 | fixed | PROMPT §2/§3.3; order as listed |
| `target_db` | Normalize… target (dB mode) | dBFS | −60.00 … 0.00 | −1.00 | 0.01 | §2.4; remembered in settings |
| `target_pct` | Normalize… target (% mode) | % of full scale | 0.1 … 100.0 | 89.1 (= −1 dB) | 0.1 | 100 % = 0 dBFS |
| `target_unit` | Dialog unit | enum | dB, % | dB | — | remembered in settings |
| `min_peak_dbfs` | Near-silence floor | dBFS | — | −120 | fixed | below → no-op (§2.7) |
| `already_tol_db` | "Already normalized" tolerance | dB | — | 0.001 | fixed | §2.7 |
| `job_dialog_delay_ms` | Progress dialog appears after | ms | — | 250 | fixed | same as SPEC-008 |
| `job_cancel_latency_ms` | Cancel → job stopped | ms | — | ≤ 100 | fixed | |
| `normalize_60min_budget_s` | 60-min whole-file normalize | s | — | ≤ 10 | fixed | reference NVMe (§2.9) |

## 4. Algorithm / implementation notes

**Where the code lives.** Gain computation and the op live in `project` (ADR-001 §4). The job runs on
the engine's worker pool with a generation token (ADR-002). There is no `rack` involvement (ADR-001
rule 4), and no `dsp` dependency beyond what `project` already has.

1. **Validate and stop.**
   - Check `base_rev`, the scope, recording and busy state (errors as in SPEC-008 §4.3).
   - Stop the transport and wait for the acknowledgement (§2.8).
2. **Pass 1: peak scan** over the scope's pieces, on the current `Arc<DocSnapshot>`:
   - `Silence` pieces contribute 0 and cost no I/O.
   - Chunk pieces are streamed through the snapshot sample reader. `P = max(P, |x| as f64)`, and any
     non-finite sample aborts with `error.normalize_non_finite`.
   - **Optional acceleration:** for spans covering whole pyramid buckets, the per-chunk pyramid's
     `max(|min|, |max|)` is an *exact* sample value (ADR-004 §5 stores true min/max, not an estimate).
     Only partial-bucket edges need raw reads. This is allowed **only** if T-305 proves bit-equality
     with the brute-force scan (AC-15) and falls back to raw reads for any chunk whose pyramid is
     missing or being recomputed.
   - The non-finite check needs raw reads, so with acceleration the guard relies on the invariant that
     the store never holds non-finite samples. T-305 decides: either keep raw reads, or assert the
     invariant at chunk commit.
3. **Decide** per §2.7. Otherwise compute `T = 10^(target_db / 20)` in f64 (in % mode, `T = p / 100`)
   and `g = T / P` in f64.
4. **Pass 2: write.** For each piece in scope, in order:
   - `Silence` → copy the piece as is (0 × g = 0).
   - Chunk → stream its samples, compute `y = ((x as f64) * g) as f32` (one multiplication, one
     rounding), and push to a `ChunkWriter`. Chunks commit with CRC and per-chunk peaks (ADR-004 §2,
     §5).
   - No dither: the domain stays f32, and the store holds f32.
   - Worker threads don't enable FTZ/DAZ. Results are plain IEEE, and subnormal outputs are allowed and
     harmless off the RT thread.
   - Cancellation is checked at least once per chunk (≤ 65 536 samples), which gives ≪ 100 ms cancel
     latency.
5. **Commit.**
   - New pieces = pieces before `S` ‖ [new chunk pieces and preserved `Silence` pieces] ‖ pieces from
     `E`. This is one `Replace{S, E−S, new_pieces}` with **identity marker mapping** (SPEC-008 §4.1).
   - Journal `chunks` + `edit {label_key: "history.normalize"}` with `fdatasync`, then swap the `Arc`
     (`rev` and `audio_rev` bump) and hand the reader the new snapshot.
   - Pieces outside the scope keep their chunk references, which is why AC-4's bit-identity holds by
     construction.
6. **Rack notice** (§2.6). The engine knows the `RackModel`, so the engine posts the notice, not
   `project`.

**Exactness notes.**
- `f32(T)` for the favorites: −1 dB → 0.89125094, −0.1 dB → 0.98855309, −3 dB → 0.70794578. These are
  well inside ±0.01 dB of target. The quantization error of f32 near 1.0 is ~6e-8 relative, or
  ≈ 5e-7 dB.
- **After saving.** A save to 16/24-bit integer WAV (SPEC-005) adds quantization or dither of at most a
  few LSB. At 16-bit that is ≤ 0.001 dB at a −0.1 dBFS peak. ACs are measured on the f32 document or on
  a 32-bit float save, and a 24-bit save stays within ± 0.01 dB (AC-1).

**i18n keys.**
- Menu, toolbar and tooltip: `favorites.menu`, `favorites.normalize_peak` ("Normalize to {target} dB"),
  `toolbar.normalize.tooltip`, `effects.normalize_dialog`.
- Dialog: `dialog.normalize.*`.
- Job, history and notices: `job.normalize`, `history.normalize`, `notice.normalize_silent`,
  `notice.normalize_already`, `notice.normalize_rack_active`.
- Errors: `error.normalize_non_finite`, `error.not_while_recording`, `error.document_busy`.

**IPC.**
- Command `edit_normalize_peak { base_rev, target: cursor | range, target_db?, target_pct? }`, with
  exactly one of the two targets.
- It returns `EditResult` (SPEC-008 §4.3) or a `job_id` with `job_progress`, `job_cancel(job_id)`, and
  `document_changed` on commit.

## 5. Acceptance criteria

Measurements use testkit `peak_dbfs` / `powervoice-cli analyze`: `-inf` is digital silence and `NaN` is
non-finite (SPEC-000 §2.4). Fixtures are seeded testkit signals at 48 kHz.
- **F1:** `voice_like`, 30 s, sample peak −12.34 dBFS.
- **F2:** pink noise, 30 s, scaled to a sample peak of **+6.0 dBFS** (float overs).
- **F3:** a 1 kHz sine, 10 s, at −20 dBFS.

- **AC-1 (favorites hit their targets).** For each fixture F1–F3 and each favorite, with no selection
  (whole file), the resulting sample peak is:
  - **−1.00 ± 0.01**, **−0.10 ± 0.01** and **−3.00 ± 0.01** dBFS;
  - also, as an exactness check, within 1 f32 ulp of `f32(10^(target/20))`.

  The same results hold when the document is saved as 32-bit float WAV and measured with
  `powervoice-cli analyze`, and within ± 0.01 dB after a 24-bit save.
- **AC-2 (custom targets).** Via the Normalize… command on F1:
  - dB targets −0.01, −6.02, −23.50 and −60.00 each give a peak within ± 0.01 dB of the target;
  - % targets 50.0 %, 100.0 % and 0.1 % give −6.02 ± 0.01, 0.00 ± 0.01 and −60.00 ± 0.01 dBFS;
  - at 100 %, no sample has `|y| > 1.0`.
- **AC-3 (relative levels preserved).** For F1 and F2 at each favorite:
  - `null_test_db(result, reference) ≤ −120 dB`, where the reference is computed by the test
    independently as `x · g_ref` in f64, with `g_ref = 10^(target/20) / max|x|` from testkit's own peak
    measurement;
  - RMS changes by exactly the applied gain in dB (± 0.001);
  - crest factor is unchanged (± 0.001 dB).
- **AC-4 (selection-only).** Given F1 with the selection [10 s, 12 s), when Normalize to −1 dB is
  applied:
  - the peak of [10 s, 12 s) is −1.00 ± 0.01 dBFS;
  - `[0, 10 s)` and `[12 s, 30 s)` are **bit-identical** to before (FNV-1a hashes equal), and their
    pieces reference the same chunks;
  - the peak used was measured only inside the selection: a louder transient placed outside it doesn't
    change the gain;
  - the selection, the playhead and every marker are unchanged.
- **AC-5 (clipping impossible).** For 10 000 seeded cases:
  - random signals (white, pink, sines, impulses; peaks from −119 to +24 dBFS; random scopes including
    partial chunks and Silence pieces);
  - random legal targets (−60 … 0 dB and 0.1 … 100 %);

  no output sample satisfies `|y| > f32(T) + 1 ulp`, and none has `|y| > 1.0`.
- **AC-6 (silent and near-silent → no-op).** Given, in turn:
  - a scope of all `+0.0` samples;
  - a scope made only of `Silence` pieces (SPEC-008 Insert silence);
  - a scope whose peak is −130 dBFS;

  each favorite and Normalize… leaves `rev`, `audio_rev`, the audio hash and the undo depth unchanged,
  writes no journal record, and posts `notice.normalize_silent`. A scope peaking at −100 dBFS *is*
  normalized to within ± 0.01 dB.
- **AC-7 (already normalized, non-finite).**
  - F1 already normalized to −1 dB: a second Normalize to −1 dB is a no-op with
    `notice.normalize_already` and no undo entry.
  - A test-injected snapshot containing one NaN sample in scope: the command returns
    `error.normalize_non_finite`, and nothing changes.
- **AC-8 (cancellation and failure are all-or-nothing; SPEC-004 AC-2).** Given a whole-file normalize
  of the 60-min fixture:
  - when cancelled during pass 1, and separately at ≈ 50 % of pass 2, the job stops within 100 ms;
  - in both cases `rev`, `audio_rev`, the audio hash and the undo/redo depths are unchanged, and there
    is no undo entry;
  - with an injected `ChunkWriter` I/O error, the same holds, plus a notice.
- **AC-9 (performance).** On the reference machine (§2.9), a whole-file Normalize to −1 dB of the
  60-min, 48 kHz fixture:
  - completes in **≤ 10 s** from the command to the committed snapshot;
  - keeps the store's resident-memory counter ≤ the memory budget + 128 MiB throughout;
  - computes pyramids only for the newly written chunks (2 637 of them, none for untouched ones);
  - emits `job_progress` events at ≤ 10 Hz with monotonically non-decreasing values ending at 1.0.
- **AC-10 (undo/redo).**
  - A normalize adds exactly one undo entry labelled `history.normalize` (en "Undo Normalize").
  - Undo restores the original audio hash, and redo restores the normalized hash.
  - A seeded sequence of 20 normalizes with random scopes and targets, all undone, returns the original
    hash exactly.
- **AC-11 (playback and recording).**
  - Given fake-backend playback, when a favorite is clicked, the output fades to digital silence within
    6 ms of the stop command, before the peak scan starts. No output sample after the commit comes from
    the old revision, playback does not resume, and the playhead equals the heard position at the stop
    (± 1 sample).
  - Given a recording in progress, all four commands return `error.not_while_recording` and change
    nothing.
- **AC-12 (silence pieces preserved).** Given a scope of 60 s of audio containing 20 s of inserted
  `Silence` pieces, after normalizing:
  - those 20 s are still `Silence` pieces;
  - the `ChunkWriter` wrote exactly `40 s × 48 000 × 4` bytes of sample data (± one partial chunk's
    alignment padding, not counted as sample data);
  - the silent samples read back as `+0.0`.
- **AC-13 (rack interplay).** Given the rack [Gain +6 dB] (not bypassed) and F1, when Normalize to
  −1 dB is applied:
  - the **source** peak is −1.00 ± 0.01 dBFS;
  - the `RackModel` is unchanged (JSON-equal);
  - an offline render of the result through the rack peaks at +5.00 ± 0.01 dBFS, which confirms the
    rack is not applied by normalize;
  - `notice.normalize_rack_active` is posted on the first such normalize of the app session and not on
    the second;
  - with every slot bypassed, no notice is posted.
- **AC-14 (one click; UI placement; no shortcut).** In Vitest with mockIPC:
  - each toolbar button and each Favorites menu item sends exactly one `edit_normalize_peak` with its
    target, and scope = the selection if non-empty, otherwise the whole file; no dialog opens;
  - the Normalize… dialog rejects −60.01 dB, +0.01 dB, 0.0 %, 100.1 % and "abc" (Apply disabled);
  - it converts −1.00 dB ↔ 89.1 %, sends the value in the unit it was entered in, and reopens with the
    last applied value and unit;
  - the keymap registry contains no binding for any normalize command;
  - the progress dialog appears only for jobs that are still running after 250 ms.
- **AC-15 (exact peak detection).** If the pyramid-accelerated peak scan (§4 step 2) is implemented,
  then for 1 000 seeded piece tables (random sub-chunk ranges, silence pieces, chunk edges) its `P`
  equals the brute-force `max |x|` over the same samples, **bit-exact**. Otherwise this AC is N/A and
  pass 1 reads raw samples.

## 6. Test plan

| AC | Unit (`project`) | Integration (engine, fake backend, CLI) | Vitest (UI, mockIPC) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | gain computation + write pass on F1–F3 | normalize → save 32f/24-bit → `powervoice-cli analyze` | — | click each favorite on a real take; check the meter / `analyze` |
| AC-2 | dB and % target conversion | — | dialog sends the right target | type −2 dB, apply |
| AC-3 | null test vs independent testkit reference | — | — | listen: only the level changed |
| AC-4 | selection-scope splice; outside hashes and chunk refs | — | scope from selection | normalize one phrase |
| AC-5 | seeded property test (10 000 cases) | — | — | — |
| AC-6 | no-op decision table | notices reach the UI | notice text | normalize a room-tone-free silent region |
| AC-7 | already-normalized tolerance; NaN guard | — | notice text | click −1 dB twice |
| AC-8 | `ChunkWriter` cancel/fail leaves state untouched | job cancel in pass 1 / pass 2; injected I/O error | Cancel/Esc wiring | cancel a whole-file normalize |
| AC-9 | — | bench on the 60-min fixture (`just fixtures`, `just bench`) | — | time a 60-min normalize |
| AC-10 | one entry, label, seeded undo sequence | undo/redo through the engine | "Undo Normalize" | Ctrl+Z / Ctrl+Shift+Z |
| AC-11 | — | stop-before-scan ordering; refusal while recording | disabled controls while recording | click a favorite while playing |
| AC-12 | silence pieces preserved; writer byte count | — | — | — |
| AC-13 | — | engine: notice once per session; offline render peak | notice shown | normalize with a Gain in the rack |
| AC-14 | — | — | toolbar, menu, dialog, keymap tests | toolbar and Favorites menu by hand |
| AC-15 | pyramid vs brute-force property test | — | — | — |

Fixtures are seeded testkit signals (`voice_like`, `pink_noise`, `sine`, `white_noise`, `impulse`,
scaled with `dbfs_to_linear`) and the 60-min fixture from `just fixtures`. Nothing is committed.

## 7. Out of scope
- True-peak (dBTP) normalize, LUFS normalize and its favorites, and the loudness meter (M6: T-601,
  T-603).
- Bake rack (M6, T-602). Normalizing the processed output is Bake → Normalize.
- DC bias adjust (§2.2) and RMS/average normalize.
- Preview, current-peak readout in the dialog (§2.4), and any per-favorite target editing.
- User-defined favorites, and effects other than Normalize in the Favorites menu (candidates for later).
- Keyboard shortcuts for normalize (SPEC-019 may add them).
- Undo labels carrying the target value ("Undo Normalize to −1 dB"). This needs label parameters in
  the journal `edit` record, an ADR-004 §6 change, proposed for T-301.
