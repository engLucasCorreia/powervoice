//! Real-time priority for the sandbox audio thread (ADR-008 §3: the sandbox requests it itself).
//!
//! Linux: `SCHED_FIFO` at [`RT_PRIORITY`] (or the `RLIMIT_RTPRIO` ceiling if lower) when the
//! user may (the `audio` group usually has an rtprio limit); otherwise the thread stays
//! `SCHED_OTHER` — correct, just less protected against scheduling latency. rtkit (D-Bus) and
//! MMCSS / macOS time-constraint policies are follow-ups. `POWERVOICE_SANDBOX_NO_RT=1` disables
//! the attempt.

/// Below PipeWire/JACK's own RT threads (≈ 88), above ordinary threads.
#[cfg(target_os = "linux")]
const RT_PRIORITY: libc::c_int = 60;

/// Tries to give the calling thread real-time priority; `true` on success.
#[cfg(target_os = "linux")]
pub(crate) fn promote_current_thread() -> bool {
    if std::env::var_os("POWERVOICE_SANDBOX_NO_RT").is_some_and(|v| v == "1") {
        return false;
    }
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes into the struct we own.
    if unsafe { libc::getrlimit(libc::RLIMIT_RTPRIO, &mut limit) } != 0 {
        return false;
    }
    let ceiling = if limit.rlim_cur == libc::RLIM_INFINITY {
        RT_PRIORITY
    } else {
        libc::c_int::try_from(limit.rlim_cur)
            .unwrap_or(0)
            .min(RT_PRIORITY)
    };
    if ceiling < 1 {
        return false;
    }
    let param = libc::sched_param {
        sched_priority: ceiling,
    };
    // SAFETY: plain scheduler call on the current thread with a valid param struct.
    unsafe { libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param) == 0 }
}

/// Tries to give the calling thread real-time priority (not implemented on this platform yet).
#[cfg(not(target_os = "linux"))]
pub(crate) fn promote_current_thread() -> bool {
    false
}
