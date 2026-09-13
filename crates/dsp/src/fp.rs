//! Flush-to-zero / denormals-are-zero guard (ADR-002 §2): entered at the top of every audio
//! callback and every offline render. DSP must still be denormal-safe on its own; the guard only
//! removes the CPU penalty of denormal arithmetic.

use std::marker::PhantomData;

/// Enables FTZ/DAZ on the current thread for its lifetime (x86_64: MXCSR FTZ|DAZ; aarch64:
/// FPCR.FZ; no-op elsewhere) and restores the previous mode on drop.
///
/// RT-safe: two register accesses, no allocation, no syscall. Not `Send`: the mode is per-thread
/// register state, so the guard must be dropped on the thread that created it.
pub struct DenormalGuard {
    saved: u64,
    _not_send: PhantomData<*const ()>,
}

impl DenormalGuard {
    /// Enters FTZ/DAZ mode.
    #[inline]
    #[must_use = "the mode is restored when the guard is dropped"]
    pub fn new() -> Self {
        Self {
            saved: imp::enable(),
            _not_send: PhantomData,
        }
    }
}

impl Default for DenormalGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DenormalGuard {
    #[inline]
    fn drop(&mut self) {
        imp::restore(self.saved);
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use std::arch::asm;

    const FTZ: u32 = 1 << 15;
    const DAZ: u32 = 1 << 6;

    #[inline]
    pub(super) fn enable() -> u64 {
        let mut csr: u32 = 0;
        // SAFETY: `stmxcsr` stores the 32-bit MXCSR register into `csr`, a valid, aligned,
        // writable u32 on our stack. SSE is part of the x86_64 baseline.
        unsafe {
            asm!("stmxcsr [{}]", in(reg) std::ptr::addr_of_mut!(csr), options(nostack, preserves_flags));
        }
        let new = csr | FTZ | DAZ;
        // SAFETY: `ldmxcsr` loads MXCSR from `new`, a valid u32. Only the FTZ and DAZ mode bits
        // differ from the current value (rounding mode and exception masks are kept), which Rust
        // code tolerates: it only changes how denormals are produced and read.
        unsafe {
            asm!("ldmxcsr [{}]", in(reg) std::ptr::addr_of!(new), options(nostack, readonly, preserves_flags));
        }
        u64::from(csr)
    }

    #[inline]
    pub(super) fn restore(saved: u64) {
        let csr = saved as u32;
        // SAFETY: restores the MXCSR value read by `enable` on this thread (the guard is !Send).
        unsafe {
            asm!("ldmxcsr [{}]", in(reg) std::ptr::addr_of!(csr), options(nostack, readonly, preserves_flags));
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use std::arch::asm;

    const FZ: u64 = 1 << 24;

    #[inline]
    pub(super) fn enable() -> u64 {
        let fpcr: u64;
        // SAFETY: reading FPCR has no side effects.
        unsafe { asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags)) };
        // SAFETY: sets only FPCR.FZ (flush-to-zero); rounding mode and trap bits are kept.
        unsafe {
            asm!("msr fpcr, {}", in(reg) fpcr | FZ, options(nomem, nostack, preserves_flags))
        };
        fpcr
    }

    #[inline]
    pub(super) fn restore(saved: u64) {
        // SAFETY: restores the FPCR value read by `enable` on this thread (the guard is !Send).
        unsafe { asm!("msr fpcr, {}", in(reg) saved, options(nomem, nostack, preserves_flags)) };
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod imp {
    #[inline]
    pub(super) fn enable() -> u64 {
        0
    }

    #[inline]
    pub(super) fn restore(_saved: u64) {}
}

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::DenormalGuard;
    use std::hint::black_box;

    fn half_of_min_positive() -> f32 {
        black_box(f32::MIN_POSITIVE) / black_box(2.0f32)
    }

    #[test]
    fn flushes_denormals_inside_and_restores_after() {
        assert_ne!(
            half_of_min_positive().to_bits(),
            0,
            "denormals without the guard"
        );
        {
            let _g = DenormalGuard::new();
            assert_eq!(half_of_min_positive().to_bits(), 0, "flushed to zero");
            {
                let _nested = DenormalGuard::new();
            }
            assert_eq!(
                half_of_min_positive().to_bits(),
                0,
                "nested guard restores FTZ"
            );
        }
        assert_ne!(half_of_min_positive().to_bits(), 0, "mode restored");
    }
}
