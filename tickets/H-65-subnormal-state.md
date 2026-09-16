# H-65 — Close SPEC-016 AC-18 with a state accessor

- **Tier:** Sonnet (no review loop)
- **From:** H-58. The 120 s silence test asserts the *output* has no subnormals; AC-18 is about the module's internal state (envelope followers, filter memories), which nothing can see.
- **Read first:** CLAUDE.md, MEMORY.md (H-58's Dynamics sections; the flush-to-zero/denormal rules in CLAUDE.md), specs/SPEC-016 AC-18, `crates/modules/src/dynamics/`, `crates/modules/src/noise_gate.rs`.

## Scope (in)
1. A `#[doc(hidden)]` (or `cfg(test)`/`cfg(feature = "testing")`) accessor exposing the internal state values of Dynamics and Noise Gate as `f32`/`f64` — keep it out of the public API surface and out of the RT path.
2. Rewrite the AC-18 test to feed 120 s of silence after signal and assert every state value is zero or normal (`is_subnormal()` false), not just the output.
3. If the test finds real subnormals, fix them (flush in the follower/filter update) and say so in your report.

## Tests
As above; the existing Dynamics and Noise Gate suites stay green.

`just check` must pass.
