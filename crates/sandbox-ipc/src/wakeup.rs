//! The wakeup shim (ADR-008 §3, Amendment 1 §2): a [`Doorbell`] (sequence word + waiter count
//! in shared memory) driven by a platform [`Wakeup`] primitive.
//!
//! - **Linux: [`FutexWakeup`]**: `FUTEX_WAKE` / `FUTEX_WAIT_BITSET` (absolute `CLOCK_MONOTONIC`
//!   deadline) on the doorbell's 32-bit sequence word, *without* `FUTEX_PRIVATE_FLAG`, since the
//!   word is in a `MAP_SHARED` segment used by two processes. No fd passing.
//! - **Portable fallback: [`SpinYieldWakeup`]**: spins briefly, then yields, then sleeps in
//!   100 µs steps until the word changes or the deadline passes; `wake` is a no-op. It compiles
//!   everywhere and is the [`PlatformWakeup`] on macOS and Windows for now. The real primitives
//!   there (ADR-008 §3: POSIX named semaphores on macOS, auto-reset named events on Windows)
//!   need a handle per doorbell that the segment can't hold, so they come with the sandbox
//!   process lifecycle (T-802) behind this same trait.
//!
//! `ring` costs one atomic RMW plus one load, and a wake syscall **only if the other side is
//! parked** (waiter count ≠ 0). `wait_until` is bounded by its deadline and by
//! [`MAX_WAIT_ROUNDS`]: it never waits unboundedly.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Result of one [`Wakeup::wait`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitStatus {
    /// Woken, value changed, interrupted or spurious: re-check the condition.
    Woken,
    /// The deadline passed.
    TimedOut,
}

/// A point on the monotonic clock (`CLOCK_MONOTONIC` on unix, the futex clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline {
    at_ns: u64,
}

impl Deadline {
    /// `timeout` from now. One clock read (vDSO on Linux, no syscall).
    pub fn after(timeout: Duration) -> Self {
        Self::after_ns(u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX))
    }

    /// `ns` nanoseconds from now.
    pub fn after_ns(ns: u64) -> Self {
        Self {
            at_ns: now_ns().saturating_add(ns),
        }
    }

    /// Whether the deadline has passed (one clock read).
    pub fn expired(self) -> bool {
        now_ns() >= self.at_ns
    }

    /// Nanoseconds on the monotonic clock.
    pub fn as_ns(self) -> u64 {
        self.at_ns
    }
}

/// Monotonic nanoseconds.
#[cfg(unix)]
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec; CLOCK_MONOTONIC is always supported. On
    // Linux this is a vDSO read (no syscall).
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64)
}

/// Monotonic nanoseconds (since the first call in this process).
#[cfg(not(unix))]
pub fn now_ns() -> u64 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(std::time::Instant::now);
    u64::try_from(base.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// A cross-process wait/wake primitive on a 32-bit word in shared memory.
pub trait Wakeup: 'static {
    /// Name, for benchmarks and logs.
    const NAME: &'static str;

    /// Wakes the waiter(s) parked on `word` (after the caller changed it). Non-blocking; RT-safe
    /// (at most one syscall).
    fn wake(word: &AtomicU32);

    /// Parks while `word == expected`, until woken or `deadline`. May return spuriously.
    fn wait(word: &AtomicU32, expected: u32, deadline: Deadline) -> WaitStatus;
}

/// Linux futex on a shared (non-private) word.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Default)]
pub struct FutexWakeup;

#[cfg(target_os = "linux")]
impl Wakeup for FutexWakeup {
    const NAME: &'static str = "futex";

    fn wake(word: &AtomicU32) {
        // SAFETY: FUTEX_WAKE only reads the address's identity; `word` is a valid, aligned u32
        // that outlives the call. Waking with nobody parked is harmless.
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word.as_ptr(),
                libc::FUTEX_WAKE,
                i32::MAX,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0u32,
            );
        }
    }

    fn wait(word: &AtomicU32, expected: u32, deadline: Deadline) -> WaitStatus {
        const MATCH_ANY: u32 = u32::MAX;
        let ns = deadline.as_ns();
        let ts = libc::timespec {
            tv_sec: (ns / 1_000_000_000) as libc::time_t,
            tv_nsec: (ns % 1_000_000_000) as libc::c_long,
        };
        // SAFETY: `word` is a valid, aligned u32 in memory that outlives the call; `ts` is a
        // valid absolute CLOCK_MONOTONIC timespec (FUTEX_WAIT_BITSET semantics). The kernel
        // compares `*word` with `expected` atomically before sleeping.
        let r = unsafe {
            libc::syscall(
                libc::SYS_futex,
                word.as_ptr(),
                libc::FUTEX_WAIT_BITSET,
                expected,
                &ts as *const libc::timespec,
                std::ptr::null::<u32>(),
                MATCH_ANY,
            )
        };
        if r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ETIMEDOUT) {
            WaitStatus::TimedOut
        } else {
            WaitStatus::Woken
        }
    }
}

/// Portable fallback: spin, then yield, then short sleeps, polling the word until the deadline.
/// `wake` is a no-op (the waiter polls).
#[derive(Clone, Copy, Debug, Default)]
pub struct SpinYieldWakeup;

impl SpinYieldWakeup {
    const SPINS: u32 = 256;
    const YIELD_NS: u64 = 1_000_000;
    const SLEEP: Duration = Duration::from_micros(100);
}

impl Wakeup for SpinYieldWakeup {
    const NAME: &'static str = "spin-yield";

    fn wake(_word: &AtomicU32) {}

    fn wait(word: &AtomicU32, expected: u32, deadline: Deadline) -> WaitStatus {
        for _ in 0..Self::SPINS {
            if word.load(Ordering::Acquire) != expected {
                return WaitStatus::Woken;
            }
            std::hint::spin_loop();
        }
        let yield_until = now_ns().saturating_add(Self::YIELD_NS);
        loop {
            if word.load(Ordering::Acquire) != expected {
                return WaitStatus::Woken;
            }
            let now = now_ns();
            if now >= deadline.as_ns() {
                return WaitStatus::TimedOut;
            }
            if now < yield_until {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Self::SLEEP);
            }
        }
    }
}

/// The primitive used by default on this platform.
#[cfg(target_os = "linux")]
pub type PlatformWakeup = FutexWakeup;
/// The primitive used by default on this platform.
#[cfg(not(target_os = "linux"))]
pub type PlatformWakeup = SpinYieldWakeup;

/// Upper bound of park/re-check rounds in one [`Doorbell::wait_until`] (spurious wake-ups and
/// rings that don't satisfy the condition), on top of the deadline.
pub const MAX_WAIT_ROUNDS: u32 = 64;

/// A doorbell in shared memory (one cache line): the ringing side bumps `seq`; the waiting
/// side parks on `seq` after announcing itself in `waiters`.
#[repr(C, align(64))]
pub struct Doorbell {
    /// Bumped by every ring.
    pub seq: AtomicU32,
    /// Number of parked (or about to park) waiters.
    pub waiters: AtomicU32,
    reserved: [AtomicU32; 14],
}

const _: () = assert!(std::mem::size_of::<Doorbell>() == 64);

impl Doorbell {
    /// Rings after the caller published its data (with at least `Release`). RT-safe: one RMW,
    /// one load, and one `W::wake` only when the other side is parked.
    pub fn ring<W: Wakeup>(&self) {
        self.seq.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) != 0 {
            W::wake(&self.seq);
        }
    }

    /// Waits until `ready()` holds, `deadline` passes, or [`MAX_WAIT_ROUNDS`] rounds elapse;
    /// returns the last `ready()`. `ready` must only read (acquire) state the ringing side
    /// publishes before it rings. No lost wake-ups: `seq` is sampled before `ready()`, and the
    /// primitive re-checks it atomically before parking.
    pub fn wait_until<W: Wakeup>(
        &self,
        deadline: Deadline,
        mut ready: impl FnMut() -> bool,
    ) -> bool {
        for _ in 0..MAX_WAIT_ROUNDS {
            let seq = self.seq.load(Ordering::SeqCst);
            if ready() {
                return true;
            }
            self.waiters.fetch_add(1, Ordering::SeqCst);
            let status = if self.seq.load(Ordering::SeqCst) == seq {
                W::wait(&self.seq, seq, deadline)
            } else {
                WaitStatus::Woken
            };
            self.waiters.fetch_sub(1, Ordering::SeqCst);
            if status == WaitStatus::TimedOut {
                break;
            }
        }
        ready()
    }
}
