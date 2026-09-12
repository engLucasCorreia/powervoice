//! Test utilities (feature `test-util`, ADR-005 §15).
//!
//! - [`ModuleTestHost`]: runs the Module API test obligations against any [`Module`](crate::Module).
//! - [`TestGain`] / [`TestGainFactory`]: the reference module (smoothed gain).
//! - [`no_alloc`] and [`install_test_allocator!`](crate::install_test_allocator): the
//!   allocation checker (`assert_no_alloc` in warn mode, active in debug and release builds).
//! - [`TestRng`]: a tiny deterministic PRNG.
//!
//! Every test binary that runs RT code under the checker must install it once:
//!
//! ```ignore
//! vox_module_api::install_test_allocator!();
//! ```

mod host;
mod rng;
mod test_gain;

pub use host::{EffectProbe, HostFailure, HostReport, ModuleTestHost};
pub use rng::TestRng;
pub use test_gain::{TestGain, TestGainFactory};

/// The checking global allocator. Prefer [`install_test_allocator!`](crate::install_test_allocator).
pub use assert_no_alloc::AllocDisabler;

/// Installs the allocation checker as the `#[global_allocator]` of the calling test binary.
/// Use once per test binary (a binary has exactly one global allocator).
#[macro_export]
macro_rules! install_test_allocator {
    () => {
        #[global_allocator]
        static VOX_TEST_ALLOCATOR: $crate::test_util::AllocDisabler =
            $crate::test_util::AllocDisabler;
    };
}

/// Runs `f` with heap allocation and deallocation forbidden on this thread.
/// `Err(n)`: `n` (de)allocations were attempted (they still succeed, so the test can report).
/// Always `Ok` when the checker is not installed (see [`alloc_checks_active`]).
pub fn no_alloc<T>(f: impl FnOnce() -> T) -> Result<T, u32> {
    let before = assert_no_alloc::violation_count();
    // The one sanctioned call site (clippy.toml disallows the raw function elsewhere).
    #[allow(clippy::disallowed_methods)]
    let out = assert_no_alloc::assert_no_alloc(f);
    match assert_no_alloc::violation_count().wrapping_sub(before) {
        0 => Ok(out),
        n => Err(n),
    }
}

/// True if [`no_alloc`] actually detects allocations, i.e. the checker is installed as the
/// global allocator. Probes with one deliberate allocation.
pub fn alloc_checks_active() -> bool {
    no_alloc(|| drop(std::hint::black_box(Box::new(0u64)))).is_err()
}
