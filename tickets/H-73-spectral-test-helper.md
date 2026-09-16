# H-73 — One shared spectral test helper

- **Tier:** Sonnet (test infrastructure)
- **From:** H-60. Every crate that needs a spectrum in a test hand-rolls its own DFT — `crates/io/tests/codec_decode.rs::dominant_frequency_hz`, `crates/dsp/src/dither.rs::magnitude_spectrum_db`, and others.
- **Read first:** CLAUDE.md, MEMORY.md (T-006's `vox-testkit` — generators, measurements, golden helpers; H-60's note; T-110's benches for anything performance-sensitive), `crates/testkit/src`, and every current hand-rolled analysis (grep for `dft`, `magnitude`, `spectrum`, `goertzel`, `dominant_frequency`).

## Scope (in)
1. One analysis helper in `vox-testkit`: a windowed magnitude spectrum (Blackman-Harris by default, window selectable), plus the small conveniences the call sites actually need — dominant frequency, the level at a frequency, harmonic levels relative to a fundamental, and a noise-floor estimate.
2. Adopt it at the existing call sites, deleting their private copies. Values asserted by existing tests must not change; if a test's numbers shift because the shared window differs, say so explicitly in your report and justify the new expectation rather than loosening the tolerance.
3. Document it briefly in `docs/contributing.md`'s testing section.

## Tests
- The helper against known signals (a pure tone at several bin offsets, a harmonic series, white noise).
- The adopting crates' suites stay green.

`just check` must pass.
