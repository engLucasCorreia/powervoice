# SPEC-005 — File formats: open/import, save, bit depth & dither, cue markers, resampling

- **Status:** approved (autonomous, T-200)
- **Milestone:** M2 (T-201 WAV read/write + `cue `/`LIST adtl`; T-202 symphonia import, multichannel
  downmix, FLAC encode; open/save wiring in the M2 UI tickets). Items tagged **[M6]** describe export
  behavior. They are fixed here so that one quantizer and one resampler serve everything, and they are
  **refined in SPEC-011/export update** (T-605, T-606). Each AC carries its milestone.
- **Related:** SPEC-000 (glossary, testkit conventions), SPEC-002 (save format of a new recording),
  SPEC-003 (document rate vs device rate), SPEC-004 (§2.6 save and "modified", AC-1 extension, AC-14
  atomic save, OD-3 import cost), SPEC-006 (progressive waveform), SPEC-007 (spectral view after
  import) · ADR-001 §3, §4 (`io`, `dsp::resample`), ADR-003 (`peaks_progress`, `job_progress`,
  `notice`, `IpcError`), ADR-004 §6 (journal `open`/`saved`), §8 (open = import, save), ADR-007
  (symphonia MPL-2.0, AAC patents, LAME) · PROMPT §2 Formats (LOCKED), §3.3 markers, §3.5 export, §4
  performance · MEMORY (hound has no `cue`/`adtl`), D-013, D-015 (OD-3), D-019

## 1. Purpose
A voice-over editor lives between other people's files and delivery specs. The user must be able to:
- open whatever the client or the recorder produced, quickly, even at an hour long;
- never be surprised by what PowerVoice did to a file on the way in, such as mixing stereo to mono;
- save back without silently losing quality, clipping, or the markers they placed;
- trust that a save interrupted by a crash never destroys the original.

This spec defines the user-visible behavior of opening (importing) files, saving them, bit-depth
reduction (dither and clipping), markers in WAV files, and the resampling quality export relies on.

## 2. Behavior / UX

### 2.1 Entry points
- **Open.** File → Open… (**Ctrl+O**), dropping a file onto the window (Tauri's built-in window
  drag-drop event), and Recent files (M3, T-306).
  - Opening replaces the current document after the standard unsaved-changes prompt (Save / Don't
    Save / Cancel, SPEC-004 §2.8).
  - Open is disabled while recording.
  - Opening the path of the document that is already open acts as **Revert**. With unsaved changes it
    first asks "Discard your changes and reopen ‹name› from disk?".
- **Save** (**Ctrl+S**) and **Save As…** (**Ctrl+Shift+S**). These are the common Audition and OS
  bindings; the final map belongs to SPEC-019.
- File dialogs are native OS dialogs. See §4.10 for the dependency this needs.

### 2.2 Supported formats
| Source | Open | Save / Save As (M2) | Export [M6] |
|---|---|---|---|
| WAV PCM 16-bit, 24-bit int; WAV IEEE float 32-bit | yes (bit-exact) | yes | yes |
| WAV tolerated: 8-bit unsigned, 32-bit int, 64-bit float, A-law, µ-law, and any of these inside `WAVE_FORMAT_EXTENSIBLE` | yes, converted (§2.3) with a notice | mapped to a written format (§2.6) | — |
| WAV with a compressed codec (MS/IMA ADPCM, GSM, MPEG-in-WAV, …) | **rejected**: unsupported codec | — | — |
| RF64 / BW64 / Wave64 (> 4 GiB) | only if symphonia's WAV reader decodes it (T-202 records the result); otherwise rejected | never written (§2.7) | — |
| FLAC, 8–24-bit (32-bit if the decoder accepts it) | yes (bit-exact) | FLAC 16 / 24 | yes |
| MP3 (MPEG-1/2/2.5 Layer III) | yes | no (Save becomes Save As, §2.6) | MP3 via LAME [M6] |
| M4A / MP4, AAC-LC | yes | no | — |
| M4A with HE-AAC (SBR/PS) or ALAC | **rejected** if the decoder refuses it ⚠ (implicitly signalled SBR may decode as the LC core at half rate; known limitation, T-202 verifies) | — | — |
| Ogg Vorbis | yes | no | — |
| Ogg Opus, AIFF, CAF, MKV/WebM, WMA, ALAC | **rejected**: unsupported format or codec | — | — |

**Decided (autonomous, T-200):** tolerate the extra WAV PCM variants (8-bit, 32-bit int, 64-bit float,
A-law/µ-law) instead of rejecting them. They cost nothing beyond the `pcm` codec we already need, and
voice-over users receive such files from phone recorders and broadcast tools. Compressed-WAV codecs,
AIFF, Opus and ALAC stay out: they are outside PROMPT §2's LOCKED format list and each would need an
extra codec feature.

### 2.3 Open = import: progress and progressive waveform
1. **Probe.** The user picks a file. PowerVoice probes container, codec, channels, rate and length.
   This takes ≤ 200 ms for a local file, and errors (§2.5) surface here whenever possible.
2. **Channels.** If the file has ≥ 2 channels, the channel choice applies first (§2.4).
3. **Import.** The import job starts and `document_open` returns.
   - The editor immediately shows the document shell: file name, rate, and length (estimated for
     formats without a sample count).
   - The waveform fills in progressively from `peaks_progress`. Pending columns use SPEC-006 §2.3's
     `--wave-pending` token.
   - A progress bar under the time ruler shows "Opening ‹name›… 43 %", with **Cancel**.
   - `peaks_progress` and `job_progress` are emitted at 4–10 Hz (ADR-003: ≤ 10 Hz per event).
4. **While importing:** zoom, scroll and selection work. Playback, editing, markers, Save and the
   spectral view (SPEC-007 §2.1) are disabled, with the tooltip "Available when the file has finished
   opening".
5. **Completion.** The document is ready and unmodified, the imported audio is the undo floor
   (SPEC-004), and the title shows the file name.
6. **Cancel.** The import stops within 200 ms, its session is garbage-collected, and the editor
   returns to the empty "no document" state. No partial document is ever left open.

**Document rate = source rate.** Accepted rates are 8 000–384 000 Hz; anything else is rejected
(§2.5). There is no resampling on open (ADR-004 §8). A mismatch with the device is handled by the
reader (SPEC-003 §2.4).

**Sample conversion to the internal f32** (document samples are always f32, ADR-004):

| Source samples | Conversion | Exact? |
|---|---|---|
| 16/24-bit int | `v / 2^(bits−1)` | bit-exact: every 16- and 24-bit value is an exact f32 |
| 8-bit unsigned | `(u − 128) / 128` | exact |
| 32-bit int | `v / 2^31`, rounded to nearest f32 | lossy below ≈ −144 dBFS, documented |
| 32-bit float | copied | bit-exact |
| 64-bit float | rounded to nearest f32 | lossy, documented |
| A-law / µ-law | ITU-T G.711 expansion to 16-bit, then `/ 2^15` | exact |
| Lossy decoders (MP3/AAC/Vorbis) | decoder f32 output, not clipped | — |

- The scale factor `2^(bits−1)` is the same one testkit's `read_wav` uses, so testkit decodes and our
  import agree bit for bit.
- **Float values above 0 dBFS are kept.** Lossy decoders routinely overshoot, and float WAVs may be
  unnormalized. Nothing is clipped on import (SPEC-006 §2.3 highlights them).
- **Non-finite samples** (NaN/±inf in float WAVs, or f64 values that overflow f32) are replaced by
  0.0. The notice "‹name› contained 12 invalid samples; they were replaced with silence" appears. A
  document never contains a non-finite sample.

**Encoder delay and padding (lossy formats).** When the container reports them (symphonia `Track`
`delay` / `padding`: LAME/Xing tag in MP3, iTunSMPB or edit list in M4A), the importer removes the
leading `delay` and trailing `padding` frames. The document then has the original length and timing.
When they are not reported, the decoded output is kept as is. ⚠ T-202 records which readers populate
them in symphonia 0.6.1, because `FormatOptions` no longer has a gapless switch.

**Length 0.** A valid file with 0 samples opens as an empty document at its rate.

### 2.4 Multichannel → mono (PowerVoice edits mono only, PROMPT §2 LOCKED)
**Decided (autonomous, T-200):**
- The default downmix is the **average of the channels**, `(L+R)/2` for stereo. An LFE channel, when
  the WAV channel mask or container identifies one, is excluded.
- The alternative is **picking one channel**.
- The choice is offered at open with a small dialog, unless the user has chosen to remember it.

Rationale: averaging never clips, is exactly transparent for dual-mono files, and is what Audition
and most editors do. Picking a channel rescues the most common voice-over accident, a mono mic
recorded into one side of a stereo file.

- **Dialog** "Open stereo file" (the title names the channel count for > 2 channels). Text: "PowerVoice
  edits mono audio. How should the 2 channels become one?"
  - ◉ **Mix to mono (average of channels)**
  - ○ **Use one channel:** [Left ▾] (labels from the channel mask: Left, Right, Center, LFE, Surround
    L…, or "Channel N")
  - ☐ **Always do this for multichannel files** (sets `multichannel_policy`)
  - **Open** / **Cancel**
- **Silent-channel hint.** The probe scans the first min(30 s, file length) of every channel. If
  exactly one channel peaks above −50 dBFS and every other channel peaks at or below −70 dBFS
  (digital silence included), the dialog preselects that channel. It then shows "The Right channel
  appears silent — using Left only keeps the full level (mixing would lower it by 6 dB)".
- **Identical channels.** If all channels are bit-identical over the probe window, there is no dialog.
  The file opens with the average, which is bit-identical to each channel. When the import completes,
  the notice states what actually happened over the whole file:
  - "Both channels are identical — opened as mono"; or
  - "Channels mixed to mono (average)" if they diverged later.
- **Remembered policy** (`multichannel_policy`): *Ask* (default) / *Always mix to mono* / *Always use
  the first channel*. It is changeable in Settings → Files. A remembered policy applies without the
  dialog; the probe hint then shows only as a notice.
- **Record of the choice.** The choice is recorded in the journal `open` record (§4.12) and shown in
  the document's properties ("Stereo file, mixed to mono (average)").
- **Saving over a multichannel source.** The first Save over it asks: "‹name› is a stereo file.
  Saving replaces it with a mono file." **Save** / **Save As…** / **Cancel**. **Decided (autonomous,
  T-200):** this prevents the one irreversible surprise in this flow, overwriting a stereo original
  with mono.

### 2.5 Unsupported and corrupt files
Every failure returns an `IpcError` with the listed i18n key (ADR-003), panics nowhere, leaves no
session directory behind, and keeps the app responsive.

| Condition | Behavior | Key / message |
|---|---|---|
| Path missing, a directory, or unreadable | refused | `error.open.not_found` / `error.open.permission` |
| Empty file, or a container no enabled reader recognizes | refused | `error.open.unsupported_format` "‹name› isn't a supported audio file" |
| Recognized container, unsupported codec | refused | `error.open.unsupported_codec` "‹name› uses ‹codec›, which PowerVoice can't open" |
| No audio track | refused | `error.open.no_audio_track` |
| Rate outside 8 000–384 000 Hz, or > 32 channels | refused | `error.open.rate_out_of_range` / `error.open.too_many_channels` |
| WAV `data` shorter than declared (truncated file) | imports every complete frame present | notice `notice.open.truncated` "‹name› seems truncated — 12:31 of audio opened" |
| WAV `data` size 0 or 0xFFFFFFFF with audio present (unfinished streaming writer) | uses the extent to the end of the file, or to the next valid chunk | silent |
| Damaged packets in a compressed stream | each damaged packet is replaced by silence of its duration when known (else skipped); timing is preserved | notice `notice.open.damaged_frames` "N damaged frames were replaced with silence (first at 1:02.3)" |
| More than max(10, 1 % of packets) damaged | the import is aborted at the end of the pass | `error.open.damaged` "‹name› is too damaged to open" |
| Malformed `cue ` chunk | the audio opens, with no markers | notice `notice.open.markers_unreadable` |
| Malformed `LIST adtl` | the audio and cue positions open; unreadable names become default names | same notice |
| I/O error during the import (e.g. removable media pulled) | aborted | `error.open.io` |
| Not enough disk space for the session | refused **before** importing. Estimate = frames × 4 B × 1.02 (peaks); refused if free space − estimate < 2 GiB (SPEC-004 floor) | `error.open.disk_space` "Opening ‹name› needs 0.7 GB; keep at least 2 GB free" |
| Chained Ogg stream whose rate or channel count changes | refused | `error.open.unsupported_format` |

### 2.6 Save format of a document
Every document carries a **save format**: container plus sample format. Save uses it; Save As lets the
user change it.

| Document origin | Save format | Notice at open |
|---|---|---|
| WAV 16 / 24 / 32f | same | — |
| WAV 8-bit unsigned, A-law, µ-law | WAV 16 | `notice.open.format_mapped` "This 8-bit WAV will be saved as 16-bit" |
| WAV 32-bit int, WAV 64-bit float | WAV 32f | same key ("will be saved as 32-bit float") |
| FLAC ≤ 16-bit | FLAC 16 | — (8-bit and 12-bit are promoted losslessly) |
| FLAC 17–24-bit | FLAC 24 | — |
| FLAC 25–32-bit | FLAC 24 (dithered) | format-mapped notice |
| MP3, M4A, Ogg Vorbis | **none**: Save acts as **Save As** with WAV 24-bit and `‹name›.wav` preselected | — |
| New recording (SPEC-002) | WAV at the recording dialog's bit depth; untitled, so Save acts as Save As | — |

**Decided (autonomous, T-200):**
- Lossy imports default to **WAV 24-bit**. It matches the recording default (PROMPT §2) and is
  universally readable. The overs check (§2.8) catches decoder overshoot.
- 32-bit int and 64-bit float map to **32f**, the only written format that keeps everything PowerVoice
  holds internally.
- PowerVoice never writes 8-bit, 32-bit int, 64-bit float or A-law/µ-law. The notice at open tells the
  user in advance.

### 2.7 Save and Save As
- **What is written.** Save writes the **current revision**, audio and markers, to the bound path in
  the save format. The revision is taken as one snapshot: edits made while the save runs never leak
  in (SPEC-004 §2.1, AC-1 extension).
- **Atomic.** The file is written to `.<name>.powervoice-tmp-<pid>` in the target folder, then
  `fdatasync`, then renamed over the target, then the directory is fsynced (ADR-004 §8). A crash
  leaves the old or the new file, never a partial one (SPEC-004 §2.6, AC-14).
- **Job behavior.**
  - Save runs as a job with a progress bar (`job_progress`). Editing and playback continue.
  - Save and Save As are disabled ("Saving…") while a save runs. Quitting waits for it to finish.
- **No rack.** Save does **not** render the rack (ADR-004 §8, D-019). When the rack is non-empty,
  the Save As dialog shows "Effects in the rack are not applied to saved files — use Export to render
  them" (Export is M6).
- **Save As dialog:**
  - native file chooser plus a format row;
  - **Format** WAV / FLAC;
  - **Bit depth**: WAV 16, 24 or 32-bit float; FLAC 16 or 24;
  - **Dither** TPDF / None, shown only for 16/24-bit and remembered as a preference;
  - **Sample rate** shown read-only as the document rate. Conversion is Export's job.
  - The extension is appended or corrected (`.wav` / `.flac`).
- **Pre-flight, in this order, before any byte is written:**
  1. The target folder is writable, and free space ≥ estimated file size + 64 MiB.
  2. WAV size limit. The RIFF size field is u32, so a WAV file must stay ≤ 4 GiB, e.g. ≈ 6 h 12 min
     at 48 kHz 32f. A larger document is refused with `error.save.too_large_for_wav` "Too long for a
     WAV file in this format — choose 16/24-bit or FLAC". RF64 is not written in v1.
  3. The overs check for integer formats (§2.8).
  4. The multichannel-source warning (§2.4).
  5. Notices: markers can't be stored in FLAC (§2.9), and metadata is dropped (§2.10).
- **On success:**
  - `dirty` is cleared (SPEC-004 AC-8) and the journal records `saved {path, seq, format}`;
  - after Save As, the document is bound to the new path and save format;
  - leftover temp files of dead processes in that folder are deleted (SPEC-004 AC-12).
- **On failure,** the target is untouched, the temp file is removed, and the error names the cause:
  - `error.save.permission`;
  - `error.save.disk_full`;
  - `error.save.locked` (target in use, Windows);
  - `error.save.io`;
  - `error.save.verify_failed` (FLAC, §2.11).

### 2.8 Bit depth, dither and clipping
**Dither, default TPDF** (PROMPT §3.5, docs/references.md):
- **When.** Writing an integer format (16/24-bit WAV or FLAC) from the f32 document, TPDF dither is
  applied **per block** of 4096 samples, aligned to document positions `[4096·b, 4096·(b+1))`.
- **Grid-exact blocks.** A block in which *every* sample is already exactly representable at the
  target depth is written **without dither**. Examples: untouched audio from a 16-bit file saved as 16-
  or 24-bit, and digital silence.
- **Decided (autonomous, T-200):** grid-exact passthrough gives three guarantees:
  1. open → save of an unedited 16/24-bit file is **bit-exact**;
  2. cut/paste-only edits (M3 piece splices) never add noise to untouched audio;
  3. digital silence stays digital silence.

  Every block that contains even one off-grid sample is dithered in full, so dither is never applied
  selectively to parts of the signal it protects.
- **Dither signal.** `d = u₁ − u₂`, with u₁ and u₂ independent and uniform in [0, 1) LSB. That is
  triangular on (−1, +1) LSB, variance 1/6 LSB². The written value is
  `q = round_half_away_from_zero(x · 2^(bits−1) + d)`, clamped to `[−2^(bits−1), 2^(bits−1) − 1]`.
- **Resulting error.** Total error variance = 1/12 + 1/6 = 1/4 LSB², i.e. **RMS = LSB/2**:
  - 16-bit: **−96.33 dBFS**;
  - 24-bit: **−144.49 dBFS**;
  - in testkit's `20·log10(rms)` convention, where a full-scale sine reads −3.01 dB.
  - For comparison, undithered rounding noise is −101.10 dBFS at 16-bit, and the dither alone is
    −98.09 dBFS.
- **Deterministic.** The generator is a PCG32 with fixed seed and stream constants in `dsp::dither`.
  It is reset at the start of every save and advanced twice per sample of every dithered block, so two
  saves of the same revision are bit-identical. Save is a pure function of snapshot + format.
- **Dither None** rounds half away from zero, which is exactly testkit's `encode_int_sample`.
- **No noise shaping** in v1.

**Clipping (integer formats only):**
- **Trigger.** If any sample has |x| > 1.0 (strictly above 0 dBFS), Save/Save As asks before writing:
  "12 samples are above 0 dBFS (peak +1.8 dBFS) and will be clipped in a 16-bit file."
  **Clip and save** (primary) / **Save as 32-bit float instead** / **Cancel**.
- **Decided (autonomous, T-200):** the primary button is "Clip and save". The user explicitly chose
  the format, and the dialog already quantifies the damage. Audition clips silently; we inform without
  blocking.
- **Silently clamped.** Samples in (1 − 2^−(bits−1), 1.0] and dither excursions are clamped without a
  prompt. The change is ≤ 0.0003 dB at 16-bit, and it is not an "over".
- **Detection cost.** The document peak pyramid is checked first. It never under-reports (ADR-004
  §5), so a pyramid peak ≤ 1.0 proves there are no overs. Only otherwise does an exact counting pass
  run, as a cancellable job with progress.

**32-bit float** is written bit-exact: no dither, and overs are preserved.

### 2.9 Markers ↔ WAV `cue ` / `LIST adtl`
Markers are `{pos_samples, len_samples, name}` (ADR-004 §3). A **region** is a marker with
`len_samples > 0`. In M2 markers are read on open, drawn by SPEC-006 §2.11, and written back on Save.
Adding and editing them is M3 (T-303).

**Reading (open):**
- **Position.** Each cue point's position is `dwSampleOffset` when `fccChunk = 'data'` and
  `dwChunkStart = dwBlockStart = 0` (the uncompressed case), otherwise `dwPosition`. Common readers use
  `dwSampleOffset`; `dwPosition` is the playlist position in the RIFF spec.
- **Names.** The name comes from the `labl` sub-chunk with the same cue id; else from an `ltxt`
  text; else the default name "Marker N" (`marker.default_name`, N = 1-based order by position).
  Empty or whitespace-only names also get the default name.
- **Regions.** An `ltxt` with `dwSampleLength > 0` makes the marker a region of that length, clamped
  to the document end. Any purpose code is accepted.
- **Ignored.** `note` sub-chunks are ignored, as are `labl`/`ltxt` entries for unknown cue ids.
- **Text decoding.** Text is decoded as UTF-8 when valid, otherwise as Windows-1252, cut at the first
  NUL.
- **Out-of-range and duplicate cues.**
  - Cue points with a position > document length are dropped, with the notice "2 markers outside the
    audio were ignored" (`notice.open.markers_out_of_range`).
  - For a duplicate cue id, the first occurrence is kept.
  - A `cue ` chunk declaring more than 100 000 points counts as malformed (§2.5).
- **Placement.** `cue ` and `LIST adtl` are accepted before or after `data`.
- **Order.** Markers are ordered by position, then by cue order.

**Writing (Save/Save As to WAV)** — **Decided (autonomous, T-200)**, matching common editor practice
and ADR-003's little-endian convention:
- **Chunk order:** `RIFF/WAVE`, `fmt `, `fact` (float only), `data` (+ pad byte), `cue `,
  `LIST adtl`. Markers go after the audio, so the data offset does not depend on the marker count and
  the file streams in one pass.
- **Cue points.** Ids are 1…n in position order. Each record has `dwPosition = dwSampleOffset =`
  frame index, `fccChunk = 'data'`, and `dwChunkStart = dwBlockStart = 0`.
- **Names.** Every marker gets a `labl`: name in UTF-8 without BOM, NUL-terminated, padded to even
  size. **Decided (autonomous, T-200):** UTF-8, because marker names are free text and UTF-8 is
  lossless; Windows-1252 is only a read fallback.
- **Regions.** Every region also gets an `ltxt` with `dwSampleLength = len_samples`, purpose `'rgn '`,
  and country/language/dialect/code page 0, with no text. ⚠ The `'rgn '` purpose follows the Sound
  Forge/Audition convention but is unverified from a primary source; readers are purpose-agnostic
  anyway.
- **No markers → no `cue ` and no `LIST adtl` chunk.**

**Other containers.** FLAC, and the WAV files other programs will read after a FLAC save, carry no
markers from PowerVoice. When the document has markers, Save/Save As to FLAC shows the notice "FLAC
files can't store markers; 5 markers won't be in the saved file" (`notice.save.markers_not_in_flac`).
From M3 the sidecar keeps them. Lossy imports bring no markers; ID3 chapters are out of scope.

**Sidecar precedence (M3, T-306 refines).** When a sidecar exists and its audio fingerprint matches
the file, its markers are authoritative, because they carry ids and future fields. Otherwise the WAV
cue markers are used.

### 2.10 Other metadata
**Decided (autonomous, T-200):** M2 keeps audio and markers only. Other metadata is **not
preserved**: `LIST INFO`, `bext`, `iXML`, `smpl`, ID3 and Vorbis comments. The first Save over a
source that had such metadata shows "Metadata in the original file (INFO, bext) isn't kept by
PowerVoice" (`notice.save.metadata_dropped`). Rationale: rewriting `bext` time references or coding
history after edits would be wrong, verbatim preservation of the rest is a separate feature, and ACX
delivery needs none of it.

### 2.11 FLAC encoding parameters
- **Encoder.** flacenc (ADR-007) at 16 or 24 bits, mono, document rate.
- **Settings.** Block size **4096** and flacenc's default LPC/residual settings (≈ libFLAC level 5
  class). T-202 records the exact `config::Encoder` values. Encoding may use the worker pool.
- **STREAMINFO.** Total samples = document length exactly; rate and bits correct. MD5 is filled if
  flacenc computes it, otherwise all zeros ("unknown" per the FLAC format).
- **Verify before rename.** **Decided (autonomous, T-200):** after encoding, the save job decodes the
  temp file with symphonia and compares it with the quantized samples it wrote, like `flac -V`. The
  file replaces the target only if they are bit-identical; otherwise `error.save.verify_failed` and
  the target is untouched. The encoder is younger than libFLAC, and a lossless format must be provably
  lossless.
- **Export [M6].** The same encoder is used, and the export dialog may expose a compression level
  (T-606).

### 2.12 Resampling quality (used by export [M6]; the M1 reader and capture-writer share the configuration)
Open and Save never resample. Export does (PROMPT §3.5), and so do the playback reader and the
capture-writer when a device can't run at the document rate (ADR-002 §5, SPEC-002 AC-4, SPEC-003
AC-7). All of them go through `dsp::resample` (rubato `Fft`, synchronous, fixed ratio; ADR-001 rule 3).
The quality targets below are **normative for export** ([M6], checked in T-605):
- **Precision.** Export resamples in **f64**. Realtime paths use f32 with the same filter settings.
- **Passband.** 20 Hz … 0.90 × min(fs_in, fs_out)/2: magnitude flat within **±0.1 dB**; frequency
  exact (ratio exact, not approximate).
- **Stopband / aliasing.** Any input component in [1.02 × min(fs_in, fs_out)/2, fs_in/2) at −1 dBFS
  produces no output component above **−100 dBFS**.
- **Noise and distortion.** For a 997 Hz −1 dBFS sine, the residual after removing the fitted tone is
  ≤ **−110 dB** relative to the tone over 20 Hz–20 kHz.
- **Timing.**
  - Output length = round(len × fs_out / fs_in) ± 1.
  - rubato's `output_delay()` is trimmed, so an impulse at input sample t lands at
    round(t × fs_out / fs_in) ± 1.
  - There is no added delay and no pre/post silence.
- **Tuning.** rubato 5 documents no stopband figures. If `Fft::new` defaults miss these numbers, T-605
  uses `Fft::new_custom` (window, cutoff, sub-chunks). If they are still missed it reports, and it
  **never relaxes the numbers silently**.
- **Order in export:** rack → resample (f64) → dither/quantize last → encode.

### 2.13 Loudness-neutral conversions
PowerVoice changes level only when the user asks (gain, normalize). File conversions must be neutral.

| Conversion | Allowed change |
|---|---|
| Open WAV 16/24/32f, FLAC | none: bit-exact (`ΔLUFS = 0.00`) |
| Save 32f | none: bit-exact |
| Save 16/24-bit with TPDF | adds only the §2.8 noise; `|ΔLUFS| ≤ 0.01 LU` for content louder than −60 LUFS |
| Downmix: pick a channel | bit-exact copy of that channel |
| Downmix: average of dual-mono (L = R) | bit-exact copy of L. BS.1770 loudness of the mono result reads **3.01 LU lower** than the stereo file, because BS.1770 sums channel powers. This is expected, not a bug: PROMPT §3.3 measures mono without dual-mono compensation by default |
| Downmix: average of uncorrelated equal-level channels | −3.01 dB RMS relative to each channel (documented) |
| Resample [M6] | `|ΔLUFS| ≤ 0.05 LU` for band-limited content (1 kHz sine, voice-like fixture) |
| Lossy decode | level within ±0.1 dB of the encoded source for ≥ 128 kbps (AC-12) |

### 2.14 Export [M6] — pointer only
The export dialog, MP3 (LAME runtime-loaded, D-013; CBR/VBR; ACX preset 44.1 kHz 192 kbps CBR), WAV
and FLAC export, and the rendering of the rack (D-019) are refined in SPEC-011/export update.
- Export reuses §2.8 (quantizer and overs check), §2.9 (cue markers in exported WAV; marker positions
  scaled by fs_out/fs_in and rounded), §2.11 (FLAC) and §2.12 (resampler).
- Export never changes the document or its save format.

## 3. Parameters
| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `multichannel_policy` | Multichannel files | enum | ask / always mix / always first channel | ask | list | Settings → Files; §2.4 |
| `downmix_choice` | Downmix at open | enum | average / channel n | average (or the probe hint's channel) | list | per open; journaled |
| `probe_window_s` | Channel probe window | s | — | 30 | fixed | min(30 s, length) |
| `silent_channel_dbfs` | Silent / active channel thresholds | dBFS peak | — | ≤ −70 silent / ≥ −50 active | fixed | §2.4 |
| `doc_rate_range_hz` | Accepted document rates | Hz | 8 000 … 384 000 | — | fixed | |
| `max_channels` | Channels accepted at open | — | 1 … 32 | — | fixed | |
| `save_dither` | Dither when writing 16/24-bit | enum | TPDF / None | TPDF | list | Save As dialog; remembered |
| `dither_block_samples` | Grid-exact decision block | samples | — | 4096 | fixed | aligned to document positions |
| `dither_seed` | Dither PRNG seed/stream | — | — | fixed constants in `dsp::dither` | fixed | deterministic saves |
| `clip_warn_abs` | Overs threshold for integer saves | linear | — | > 1.0 | fixed | §2.8 |
| `damaged_packet_limit` | Abort threshold | packets | — | > max(10, 1 % of packets) | fixed | §2.5 |
| `progress_rate_hz` | `peaks_progress` / `job_progress` rate | Hz | 4 … 10 | — | fixed | ADR-003 |
| `open_target_s` | Open 60-min 48 kHz WAV (SSD/NVMe) | s | — | ≤ 3.0 | fixed | PROMPT §4, OD-3 |
| `first_peaks_ms` | First completed chunk visible | ms | — | ≤ 300 | fixed | after the import starts |
| `wav_max_bytes` | Largest WAV written | bytes | — | 2³² − 1 total RIFF | fixed | RF64 not written |
| `flac_bits` | FLAC bit depth | bits | {16, 24} | from the save format | list | |
| `flac_block_size` | FLAC block size | samples | — | 4096 | fixed | |
| `cue_max_points` | Cue points accepted | — | — | 100 000 | fixed | more = malformed |
| `resample_precision` [M6] | Export resampler precision | — | f32 / f64 | f64 | fixed | realtime paths use f32 |
| `resample_passband` [M6] | Flat passband edge | × min Nyquist | — | 0.90 (±0.1 dB) | fixed | |
| `resample_stopband_dbfs` [M6] | Alias rejection from 1.02 × min Nyquist | dBFS | — | ≤ −100 | fixed | |

## 4. Algorithm / implementation notes

### 4.1 Crates and features
- **`io`** owns decode, encode, the RIFF chunk walker/writer, downmix and sample-format conversion
  (ADR-001 §4). `dsp` owns `dither` and `resample`.
- **symphonia 0.6.1** with `default-features = false` and features `wav`, `pcm`, `flac`, `ogg`,
  `vorbis`, `mp3`, `isomp4`, `aac`. `mkv` and `adpcm` are left off (§2.2). `aac` carries the patent
  caveat of ADR-007 open question 4.
  - ⚠ T-202 verifies that `pcm` covers u8, s32, f64, A-law and µ-law in WAV and `WAVE_FORMAT_EXTENSIBLE`
    (the docs say "all raw PCM and log-PCM codecs"). It also runs ADR-007's MPL "Incompatible With
    Secondary Licenses" check.
- **WAV reading.** Samples are decoded through symphonia. A small RIFF **chunk walker** in `io` reads
  `cue `, `LIST adtl` and the metadata-presence flags, because symphonia skips unknown chunks and
  hound has no `cue`/`adtl` support (MEMORY). The walker handles odd-sized chunks with pad bytes.
- **WAV writing.** A streaming RIFF writer of our own, so hound is not needed in `io`.
  1. Write the header with placeholder sizes.
  2. Stream `data` and add the pad byte if the byte count is odd (e.g. 24-bit mono with an odd sample
     count).
  3. Append `cue ` and `LIST adtl`.
  4. Patch the RIFF/`data`/`fact` sizes, then `fdatasync` and rename (§2.7).

  `fmt ` details:
  - 16/24-bit: `WAVE_FORMAT_PCM` (tag 1, 16-byte `fmt `), **not** `EXTENSIBLE`, for the broadest
    reader compatibility. **Decided (autonomous, T-200)**; `EXTENSIBLE` is accepted on read.
  - 32f: tag 3, 18-byte `fmt ` with `cbSize = 0`, plus a `fact` chunk (`dwSampleLength`).
- **`hound`** stays in testkit, as the independent measuring stick (ADR-001 rule 3). If `io` no
  longer needs it, T-201 drops it from `io` and reports that.

### 4.2 Import pipeline
1. **Probe.** Open the reader, select the first audio track, and read the codec parameters. For
   WAV, run the chunk walker.
2. **Pre-checks.** Rate and channel limits, the disk estimate, and the multichannel decision (§2.4).
3. **Decode loop** on a worker (ADR-002):
   - decode each packet;
   - convert to f32 per §2.3;
   - replace non-finite samples;
   - downmix (§4.3);
   - trim delay/padding;
   - push into a `ChunkWriter` (ADR-004 §2). Each committed chunk computes its peaks and drives
     `peaks_progress`.
   - Cancellation is checked at least every 50 ms.
4. **Commit.** Commit the final chunk, build the snapshot including markers, journal `open` +
   `chunks`, emit `document_changed`, and unlock playback and editing.
5. **Throughput.** The loop is one sequential read → decode → write pass. The 60-min WAV target
   (ADR-004 §8: ≈ 518 MB read + 691 MB write ≈ 1–2 s on NVMe) leaves headroom only if conversion is
   vectorizable and chunk writes are large and sequential. T-202 benchmarks with `just fixtures`'
   60-min file.

### 4.3 Downmix arithmetic (normative for AC-9)
- **Average.** For n channels, `y = (Σ x_c) · (1/n)` in f32. The sum runs in channel order and
  excludes LFE; the factor is `0.5` exactly for n = 2.
  - Hence identical channels give y = x bit-exactly.
  - L = −R gives y = 0.0 exactly.
- **Pick.** Copies the channel.
- **Probe.** The probe for §2.4 decodes the first min(30 s, length) and keeps a per-channel peak.
  Its decode is reused for the import when possible.

### 4.4 Quantizer (`dsp::dither`, shared with export)
```
for each block b of 4096 document samples (last block may be shorter):
    exact = all(s * 2^(B-1) is an integer within [-2^(B-1), 2^(B-1)-1])   // power-of-two scale: no rounding
    for s in block:
        if exact:  q = s * 2^(B-1)
        else:      q = clamp(round_half_away_from_zero(s * 2^(B-1) + (u1 - u2)), -2^(B-1), 2^(B-1)-1)
```
- u1 and u2 come from the PCG32 (§2.8) and are drawn only in non-exact blocks.
- Overs are counted as `|s| > 1.0` before quantization (§2.8).
- The quantizer is allocation-free per block. Export calls it after resampling.

### 4.5 Cue reading rules
Parse `cue ` records (24 bytes each) into `{id, position}` per §2.9. Then parse `LIST adtl`
sub-chunks, each size-checked: a sub-chunk overrunning its parent ends parsing, and the notice
applies. Map `labl` names and `ltxt` lengths onto the cue ids. Drop out-of-range cues, sort, and
assign default names.

### 4.6 Text decoding
UTF-8 is tried first. Any invalid sequence decodes the whole string as Windows-1252. Control
characters other than tab are stripped, and names longer than 1024 bytes are cut at a character
boundary. Writing uses UTF-8.

### 4.7 Save pipeline
A worker streams the snapshot through the reader-style chunk iterator (never holding the document in
RAM) → quantizer or f32 → WAV writer or flacenc → temp file → FLAC verify → `fdatasync` → rename →
directory fsync. It reports `job_progress`. The overs pre-check (§2.8) runs before the temp file is
created.

### 4.8 Resampler configuration [M6]
- **Setup.** `rubato::Fft::<f64>` with `FixedSync::Input` and an input chunk of `OFFLINE_BLOCK`
  (4096) frames. `sub_chunks` and window come from `Fft::new` unless §2.12 requires `new_custom`.
- **Priming and trimming.** Prime and trim `output_delay()`; after the last input, flush with zeros
  and cut to the exact output length.
- **Shared configuration.** The realtime reader's f32 instance uses the same configuration (ADR-002
  §5). SPEC-003 AC-7 and SPEC-002 AC-4 are its existing checks.

### 4.9 Timing budget of open (60 min, 48 kHz, NVMe, warm cache)
| Step | Budget |
|---|---|
| Probe + pre-checks | ≤ 0.2 s |
| Read + convert + chunk writes | ≤ 2.2 s |
| Peaks per chunk | ≤ 0.3 s, overlapped with the writes |
| Final commit + journal + snapshot | ≤ 0.1 s |
| **Total** | **≤ 3.0 s** |

**Cold cache and HDDs.** OD-3 accepts that HDDs may miss the target; the waveform is still
progressive there.

### 4.10 IPC surface (commands in `src-tauri`, logic in `engine`/`project`/`io`)
- **Opening.**
  - `document_open(path, channel_choice?) -> OpenStarted { job_id } | NeedsChannelChoice { channels,
    labels, probe_peaks_dbfs, suggestion, identical }`.
  - The UI shows the dialog, then calls `document_open` again with the choice.
- **Saving.** `document_save()` and `document_save_as(path, format, bits, dither)`. Both return
  `SaveStarted { job_id } | NeedsConfirmation { kind: overs { count, peak_dbfs } |
  multichannel_source | metadata_dropped | markers_not_in_flac }`. The UI re-issues with `confirm`
  flags.
- **Cancel.** `job_cancel(job_id)`.
- **Events** (ADR-003): `job_progress`, `peaks_progress`, `document_changed`, `notice`.
- **⚠ Dependency flag.** Native Open/Save dialogs need **`tauri-plugin-dialog`** (MIT/Apache-2.0,
  Tauri team), which is not in ADR-007's table. It must be approved (ADR-007 amendment) before the M2
  UI ticket. Drag-and-drop uses Tauri's built-in window drag-drop event and needs nothing new.

### 4.11 CLI additions (acceptance tooling; `cli → io` is an allowed edge, ADR-001)
- **`powervoice-cli convert <in> <out.wav|out.flac>`**, with `--bits 16|24|32f`,
  `--dither tpdf|none` and `--downmix average|ch:N`. It imports through `io` exactly like the app
  (conversion, downmix, delay trim) and writes through `io` exactly like Save, quantizer included.
  `analyze` can then measure any importer's output.
- **`powervoice-cli markers <file.wav> [--json]`** lists the markers `io` reads: position, length,
  name.
- Both follow SPEC-000's conventions: non-zero exit on error, JSON `null` for −inf/NaN.

### 4.12 Journal (ADR-004 §6)
The `open` record gains additive JSON fields `{container, codec, bits, channels, downmix,
save_format}`. `saved` already carries `format`. No structural change: new fields are additive.

## 5. Acceptance criteria
- **AC-1 [M2] (WAV open is exact).**
  - Given seeded white noise at −10 dBFS RMS, 1 s at 48 kHz, written by testkit as WAV 16, 24 and
    32f, when each is opened, then the document samples equal testkit `read_wav` of the same file
    bit-exactly (FNV-1a equal) and the document rate is 48 000.
  - Given hand-built WAV files (8-bit unsigned, 32-bit int, 64-bit float, A-law, µ-law, and 5.1
    `WAVE_FORMAT_EXTENSIBLE` 24-bit), the samples equal the §2.3 conversion computed independently in
    the test, bit-exactly.
- **AC-2 [M2] (round trip is bit-exact).**
  - Given a 16-bit WAV (60 s of seeded pink noise at −18 dBFS RMS) and a 24-bit one, when opened and
    saved with no edits and TPDF dither on, then the new `data` chunk bytes are identical to the
    source's. The same holds for 32f.
  - [M3 variant] After cut/paste-only edits (piece splices), every 4096-sample block not overlapping
    an edit boundary is bit-identical to the corresponding source samples, and an inserted silence
    piece saves as exact zeros.
- **AC-3 [M2] (TPDF level and whiteness).** Given a 32f document with a 997 Hz sine at −40 dBFS,
  10 s at 48 kHz, when saved as 16-bit with TPDF, then the residual e = saved − source (computed in
  f64):
  - has an RMS of **−96.33 ± 0.15 dB**, with |mean(e)| ≤ 1e-7;
  - is white: Welch PSD, 8192-point Blackman-Harris 4-term, 50 % overlap; every 1/3-octave band mean
    from 100 Hz to 20 kHz is within ±1.0 dB of the overall mean.
  Saved as 24-bit, the residual RMS is **−144.49 ± 0.15 dB**.
- **AC-4 [M2] (dither removes distortion; the harness discriminates).** Given a 1000 Hz sine at
  −80 dBFS peak (≈ 3.3 LSB at 16-bit), 10 s at 48 kHz, when saved as 16-bit:
  - with **Dither None**, the 3 kHz component is ≥ 20 dB above the median bin level of 3.1–4.0 kHz;
  - with **TPDF**, the components at 2, 3 and 5 kHz are each within +3 dB of the median of their
    neighbouring ±200 Hz bins.
  - Dither None output equals testkit's 16-bit `encode_wav_bytes` data bit-exactly.
  Analysis: 8192-point BH4, averaged over 10 s.
- **AC-5 [M2] (grid-exact passthrough and determinism).**
  - Two saves of the same revision are bit-identical files (FNV-1a).
  - A 16-bit save of a document whose first 4096-sample block is digital silence has exact zeros in
    that block.
  - A block that contains one off-grid sample is dithered in full: all its samples show dither noise,
    and the PRNG draw count equals 2 × block length.
- **AC-6 [M2] (overs).** Given a 32f document with a 997 Hz −6 dBFS sine and 3 samples of +1.5
  (+3.52 dBFS), when Save As 16-bit is requested:
  - the confirmation reports **3** samples and a peak of **+3.5 dBFS** (±0.05 dB);
  - **Clip and save** writes 32767 at those 3 positions;
  - **Save as 32-bit float** writes 1.5 exactly;
  - **Cancel** writes nothing: no file and no temp.
  Given a document whose peak is exactly 1.0, no confirmation appears and 1.0 is written as 32767.
- **AC-7 [M2] (cue markers round-trip).** Given a 24-bit document with an odd sample count and
  markers:
  - point markers at 0, 1, 48 000 and len − 1, and at len;
  - regions [96 000, +4 800) and [200 000, +1);
  - names "Intro", "Café — take 2" (UTF-8) and "" (empty);
  when saved and reopened, then:
  - the marker count, positions and lengths are exactly equal;
  - non-empty names are equal, and the empty name becomes "Marker N";
  - the file is structurally valid: RIFF size = file size − 8, every chunk even-padded, `data`
    followed by its pad byte, `cue ` count 7, `dwPosition = dwSampleOffset` and `fccChunk = 'data'`
    for every point, one `ltxt` with purpose `'rgn '` per region.
  A document with no markers produces no `cue ` and no `LIST adtl` chunk.
- **AC-8 [M2] (cue reading robustness).** Given hand-built files, then:
  - `dwSampleOffset = N`, `dwPosition = 0` → marker at N;
  - `fccChunk = 'slnt'` → `dwPosition` is used;
  - `cue ` placed before `data` → read;
  - a position > length → dropped, with the out-of-range notice showing the count;
  - a `labl` with Windows-1252 byte 0xE9 → "é";
  - a cue without a `labl` → "Marker 1";
  - an `ltxt` with length 480 → a region of 480;
  - an `LIST adtl` truncated mid-sub-chunk → audio and positions open, names default, notice;
  - a `cue ` declaring 10⁹ points → no markers, notice, no allocation beyond the file size, no panic.
- **AC-9 [M2] (downmix arithmetic).** Given 48 kHz 32f multichannel files:
  - (a) stereo L = sine(997 Hz, −6 dBFS), R = −L, average → the document is digital silence: testkit
    peak `-inf`, and `analyze --json` prints `null`;
  - (b) L = R → the document is bit-identical to L, opened without a dialog, with the "identical"
    notice;
  - (c) L = sine −20 dBFS, R = seeded white −30 dBFS, average → bit-identical to `(L + R) * 0.5f32`;
  - (d) the same file with "use Right" → bit-identical to R;
  - (e) 5.1 `EXTENSIBLE` with mask 0x3F, average → bit-identical to `(FL+FR+FC+BL+BR) * (1/5 as f32)`
    summed in that order (LFE excluded).
- **AC-10 [M2] (channel dialog and policy).**
  - Given a stereo file whose Right channel is digital silence for the first 30 s, the dialog
    preselects Left and shows the silent-channel hint. With Right at −75 dBFS peak and Left at
    −20 dBFS, the same. With both at −40 dBFS, average is preselected.
  - With `multichannel_policy` = always mix, no dialog appears.
  - The journal `open` record contains the choice.
  - The first Save over a multichannel source shows the mono warning exactly once per document.
  (Vitest dialog logic plus `project` integration.)
- **AC-11 [M2] (open performance, progressive).** Given `just fixtures`' 60-min 48 kHz mono WAV (32f,
  691 MB) and a 24-bit variant (518 MB) in the page cache on the owner's NVMe:
  - from the `document_open` command to `document_changed` with playback enabled is **≤ 3.0 s**;
  - the first `peaks_progress` reporting a completed chunk arrives **≤ 300 ms** after the command;
  - `peaks_progress` events arrive at 4–10 Hz;
  - no Tauri command handler blocks the main thread for > 50 ms.
  Cold-cache opening is a manual smoke check against the same 3 s. First paint is SPEC-006 AC-19.
- **AC-12 [M2] (lossy and FLAC decode accuracy).** Given 10 s of 997 Hz at −20 dBFS, at 44 100 and at
  48 000 Hz, encoded as MP3 192 kbps CBR (LAME, with tag), M4A AAC-LC 192 kbps, Ogg Vorbis q6, and
  FLAC 16 and 24, then:
  - the document rate = the file rate;
  - the RMS is **−23.01 ± 0.1 dB** (testkit), and the dominant frequency is within one bin
    (8192-point BH4);
  - when the container reports delay/padding: the length equals the original exactly, and the
    cross-correlation lag against the original is 0 ± 1 sample. Otherwise the length is within the
    original + 2 codec frames, and the case is listed in the T-202 report;
  - FLAC decodes bit-exactly to the source PCM.
- **AC-13 [M2] (compressed import throughput).** Given 10-min mono fixtures of the voice-like signal,
  the decode + import wall time on the owner's machine is ≤ 3.0 s for FLAC, MP3 and Vorbis (≥ 200×
  realtime) and ≤ 6.0 s for AAC (≥ 100×). This is a bench; a miss is reported to T-704 with the
  profile, not silently accepted.
- **AC-14 [M2] (errors and corruption).** Given the §2.5 cases each return the listed key or notice
  and leave no session directory:
  - zero-byte file;
  - 64 KiB of seeded random bytes;
  - MS-ADPCM WAV;
  - ALAC M4A;
  - Ogg Opus;
  - 4 000 Hz WAV;
  - 40-channel WAV;
  - a missing path.
  Also:
  - a 24-bit WAV truncated at 1 000 seeded byte offsets inside `data` opens with exactly
    floor((size − data_offset) / 3) samples and the truncation notice;
  - an MP3 with 100 seeded byte flips in the audio opens with the damaged-frames notice and a length
    within ±1 frame of the original;
  - 10 000 seeded header mutations of each fixture never panic (fuzz-style unit test in `io`).
- **AC-15 [M2] (save atomicity and failures).**
  - SPEC-004 AC-14 holds for WAV and for FLAC (kill at 20 random points).
  - Saving into a read-only folder returns `error.save.permission`, leaves the target bit-identical
    and leaves no temp file.
  - Injected disk-full returns `error.save.disk_full` with the same guarantees.
  - A second Save while one runs is refused with `error.save.in_progress`.
  - After Save As, `dirty` is false and the document is bound to the new path and format.
  - Saving revision r while 50 edits commit produces a file whose audio hash equals r's (SPEC-004 AC-1
    extension).
- **AC-16 [M2] (FLAC save).**
  - FLAC 16 and 24 saves of seeded noise and of a sine decode (symphonia) bit-identical to the
    quantized samples.
  - STREAMINFO total samples, rate and bits are exact.
  - With the encoder output corrupted by fault injection, verify fails with `error.save.verify_failed`
    and the target is untouched.
  - A 16-bit FLAC of 60 s of 997 Hz at −20 dBFS (TPDF) is ≤ 50 % of the 16-bit WAV's size.
  - A document with markers shows `notice.save.markers_not_in_flac`.
- **AC-17 [M2] (format mapping and Save-as-Save-As).**
  - Opening an 8-bit WAV shows the format-mapped notice. Save writes 16-bit data equal to
    `(u − 128) · 256` exactly.
  - A 64-bit float WAV maps to 32f.
  - For an opened MP3, Save opens the Save As dialog with WAV 24-bit and `‹name›.wav` preselected.
  - A WAV with `LIST INFO` shows `notice.save.metadata_dropped` at the first Save, and the saved file
    contains no `LIST INFO`.
- **AC-18 [M2] (WAV size limit; save speed).**
  - Given a document of 6.3 h of silence pieces at 48 kHz (no storage needed), Save As WAV 32f is
    refused before writing (`error.save.too_large_for_wav`), while WAV 16-bit is accepted.
  - Given the 60-min fixture, Save As WAV 24-bit completes in ≤ 5.0 s on the owner's NVMe (warm), with
    `job_progress` at 4–10 Hz and the UI responsive.
- **AC-19 [M2] (loudness neutrality).**
  - Open of WAV 16/24/32f and FLAC: ΔLUFS = 0.00 (bit-exact).
  - 24-bit TPDF save of the voice-like fixture: |ΔLUFS| ≤ 0.01 LU.
  - A stereo dual-mono file of 1 kHz at −20 dBFS per channel (BS.1770 stereo ≈ −20.0 LUFS) opened
    with average measures **−23.01 ± 0.1 LUFS** mono (the documented −3.01 LU), with a sample peak
    identical to L.
- **AC-20 [M6] (resampler quality, f64).** For the pairs 48 000 → 44 100, 44 100 → 48 000,
  96 000 → 48 000 and 48 000 → 96 000:
  - tones at 20 Hz, 100 Hz, 1 kHz, 10 kHz and 0.90 × min Nyquist keep their level within **±0.1 dB**
    and their frequency within ±0.5 bin (8192-point BH4 at fs_out);
  - −1 dBFS tones every 250 Hz in [1.02 × min Nyquist, 0.99 × fs_in/2) leave no output component
    above **−100 dBFS**;
  - a 997 Hz −1 dBFS sine has a residual ≤ **−110 dB** relative (20 Hz–20 kHz);
  - output length = round(len × r) ± 1, and an impulse at sample 10 007 lands at round(10 007 × r) ± 1;
  - 1 kHz −20 dBFS and the voice-like fixture change integrated loudness by ≤ 0.05 LU.
- **AC-21 [M6] (export order).** Given a 32f 48 kHz −40 dBFS 997 Hz sine exported as 16-bit
  44.1 kHz, the residual against the f64 resampled reference is **−96.33 ± 0.2 dB** RMS, which proves
  that dither is applied once, after resampling. Cue markers in the exported WAV sit at
  round(pos × 44 100 / 48 000).

## 6. Test plan
| AC | Unit | Integration | Vitest | Manual smoke (owner, Arch/PipeWire) |
|---|---|---|---|---|
| AC-1 | `io`: conversion table, hand-built headers | `powervoice-cli convert` → `analyze` for each variant | — | open a phone-recorder 8-bit/µ-law WAV if available |
| AC-2 | `dsp::dither` grid check | open → save hash compare (`project` + `io`) | — | open/save a real 16-bit take, `cmp` the data |
| AC-3 | `dsp::dither` residual statistics (seeded) | `convert --bits 16/24` → residual vs source | — | — |
| AC-4 | `dsp::dither` PSD harmonics (TPDF vs None) | — | — | — |
| AC-5 | block decision + PRNG draw count | save twice, hash compare | — | — |
| AC-6 | overs count (pyramid pre-check + exact pass) | save-as with confirmation flags | confirmation dialog buttons | save a hot float file as 16-bit |
| AC-7 | RIFF writer layout (golden bytes), cue/adtl writer | save → reopen round trip; `powervoice-cli markers` | — | open the saved file in REAPER, Audacity and ocenaudio (and Audition if available): names, positions, regions |
| AC-8 | chunk walker on hand-built variants, mutation fuzz | — | — | open WAVs with markers made by REAPER/Audacity |
| AC-9 | downmix arithmetic | `convert --downmix` → `analyze` (a: peak `null`) | — | open a stereo interview file |
| AC-10 | probe classifier | open flow with the policy variants; journal inspection | channel dialog (mockIPC) | open a stereo file with one silent side |
| AC-11 | — | timed open of the 60-min fixtures (warm cache) | — | cold open of a real 60-min WAV |
| AC-12 | — | codec vectors → document; `analyze`; cross-correlation | — | open client MP3/M4A files |
| AC-13 | — | bench (`just bench`) on 10-min generated compressed fixtures | — | — |
| AC-14 | fuzz and truncation loops in `io` | error cases through `document_open` | error toast keys | open a corrupt file |
| AC-15 | temp/rename sequence with fault injection | child-process kill at 20 points; read-only dir; disk-full provider | "Saving…" disabled state | save over a file on a nearly full USB stick |
| AC-16 | flacenc wrapper + verify | save FLAC → symphonia decode compare | — | `flac -t` on the saved file, if installed |
| AC-17 | format mapping table | open → save flows | Save → Save As redirect | save an imported MP3 |
| AC-18 | size-limit arithmetic | silence-piece document; timed 60-min save | — | — |
| AC-19 | — | `analyze` before/after conversions | — | — |
| AC-20 [M6] | `dsp::resample` tone/alias/impulse suite | `convert`-style export path (T-605) | — | — |
| AC-21 [M6] | — | export job residual + cue scaling (T-605) | — | — |

**Fixtures**
- **WAV 16/24/32f and 8-bit:** testkit (hound) at test time.
- **Hand-built RIFF headers:** in `io` test code (64-bit float, A-law, µ-law, `EXTENSIBLE` 5.1,
  cue/adtl variants, truncations).
- **FLAC:** encoded at test time with `io` and verified against symphonia plus testkit sample hashes.
- **MP3, M4A (AAC-LC and ALAC), Ogg Vorbis, Ogg Opus:** tiny vectors, ≤ 64 KiB each and ≤ 512 KiB in
  total, 1–10 s tones. **Decided (autonomous, T-200):** they are committed under
  `crates/io/tests/data/`, because no in-tree encoder exists for these codecs. A `just fixtures-codec`
  recipe regenerates them with system `ffmpeg`/`lame`/`oggenc` and prints the exact commands. This is
  a deliberate exception to the "no committed audio" convention (reported for MEMORY).
- **10-min compressed performance fixtures:** generated by `just fixtures-codec --long` into
  `fixtures/generated/` (not committed). The bench skips with a message when the tools are missing.

## 7. Out of scope
- **Export** (the dialog, MP3/LAME, rack rendering, export sample-rate choice): M6, SPEC-011/export
  update. The sidecar and marker editing: M3.
- **Formats:** RF64/BW64 writing; AIFF, CAF, ALAC, Opus, WMA, MKV; ADPCM WAV; 8-bit, 32-bit int or
  64-bit float writing.
- **Metadata:** preserving `LIST INFO`/`bext`/`iXML`/ID3/Vorbis comments; FLAC cuesheets or markers
  in FLAC; ID3 chapters; CD cue sheets.
- **Other:** noise-shaped dither; resampling on open or save; stereo or multichannel editing (PROMPT
  §3.8); batch conversion.

## Amendment 1 — H-82 editing during an import: which document, and why (2026-09-17, autonomous)

H-76 found that §2.3 item 4 ("editing... are disabled" while an import runs) was never actually
enforced for Cut/Copy/Paste/Delete/Trim/Silence/Insert Silence, and — because the import job never
touches the *previous* document until it commits — asked whether that gap even mattered. It does,
but not for data-integrity reasons, so the wording is sharpened here to say why.

- **The previous document is safe either way.** `document_open` only swaps the previous document
  out for the newly-imported one at commit (`document_changed`); nothing an edit command does
  while an import job is merely running can reach the file being imported, and nothing the import
  does can reach the previous document.
- **The *view* is not safe.** While importing, the waveform shows the *importing* file's
  progressive shell (§2.3 step 3, H-71), and — per item 4's own "zoom, scroll and selection work"
  — every selection, cursor position and zoom/scroll interaction is bounded by that file's own
  probed length, not the previous document's (H-76's `interactionLenSamples`). A selection or
  cursor position the user sets during an import therefore describes a location in the *importing*
  file, not in the document an edit command would actually act on. Running Cut, Copy, Paste,
  Delete, Trim, Silence or Insert Silence during that window would silently apply those
  importing-file coordinates to the previous document's audio — wrong, even though nothing is
  corrupted in the sense of writing to the wrong file on disk.
- **Item 4 is read literally, not narrowed.** "Editing... are disabled" means exactly that: all
  seven ops named above, in every place they can be triggered — the Edit menu, the waveform's
  right-click menu (SPEC-008 §2.11) and their keyboard shortcuts alike — for as long as an import
  job is `running`. This matches the existing treatment of `peaks_get`, zero-crossing snap and
  marker hit-testing (H-76: "they would read the wrong document"). Selection, zoom and scroll stay
  exactly as item 4 already describes them: live, bounded by the importing file.
- **Undo/Redo are unaffected.** They replay the previous document's own history stack directly, by
  index — never through a selection or cursor position read off the current view — so they carry
  none of the ambiguity above and are out of this ticket's scope.
