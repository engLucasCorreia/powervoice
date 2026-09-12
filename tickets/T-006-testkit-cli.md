# T-006 — `testkit` + `powervoice-cli` gen/analyze + `just fixtures`

- **Milestone / wave:** M0 / W2
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** T-001
- **PROMPT refs:** §3.5, §5 · **References:** `docs/references.md` (loudness, ACX)

## Goal
Objective measurement tooling used by every DSP acceptance test and by the owner at checkpoints.

## Scope
**In:**
- `crates/testkit` (dev-dependency for other crates; also used by `cli`):
  - Generators (deterministic, seeded): sine (freq, level dBFS, duration, sample rate), log sweep, white and pink noise, tone bursts (on/off pattern), silence, impulse, "voice-like" test signal (tone bursts over noise at a set SNR), EBU Tech 3341 cases 1–2 (stereo, for verifying the loudness wrapper).
  - Measurements: sample peak dBFS, RMS dB, crest factor, DC offset; loudness via `ebur128` (feature `precision-true-peak`): integrated, max momentary, max short-term, LRA, true peak dBTP; noise floor = quietest 500 ms unweighted RMS window; null test (max abs difference in dB).
  - Golden helpers: compare buffers with tolerance; read/write f32 WAV via `hound`.
- `powervoice-cli`:
  - `gen <signal> [options] -o <file|->` — writes WAV (default 48 kHz mono 32-bit float; `--bits 16|24|32f`, `--rate`).
  - `analyze <file|-> [--json]` — prints all measurements above plus duration, rate, channels, bit depth.
  - `-` means stdout/stdin so `powervoice-cli gen sine --freq 1000 --level -20 -o - | powervoice-cli analyze -` works.
  - `bench` stays a stub (T-110).
- `just fixtures` → generates into `fixtures/generated/`: a set of short signals, and `long-60min-48k-mono.wav` (pink noise + tone bursts) for performance tests.

**Out:** `render --rack` (T-103), resampling, MP3/FLAC.

## Acceptance tests
- [ ] Mono 1 kHz sine at −20 dBFS, 48 kHz, 20 s → peak −20.00 ± 0.01 dBFS, RMS −23.01 ± 0.01 dB, integrated −23.0 ± 0.1 LUFS.
- [ ] Tech 3341 case 1 (stereo 1 kHz −23 dBFS/ch, 20 s) → I, M-max, S-max = −23.0 ± 0.1 LUFS; case 2 (−33 dBFS) → −33.0 ± 0.1.
- [ ] True peak of a sine at fs/4 with 45° phase offset, sample peak −6 dBFS → TP > sample peak (demonstrates inter-sample detection), value within ± 0.3 dB of analytic.
- [ ] Noise floor of (10 s tone bursts with 1 s gaps of −70 dBFS white noise) → −70 ± 1 dB.
- [ ] Generators are deterministic for a given seed (hash test).
- [ ] CLI pipe example above works in an integration test.
- [ ] `just check` green.

## Definition of Done
- [ ] Above green; Opus review passed; report includes sample `analyze` output.
