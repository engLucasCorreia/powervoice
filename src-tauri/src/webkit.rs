//! Linux WebKitGTK DMA-BUF renderer default (ADR-009 Amendment 1, MEMORY D-016).
//!
//! WebKitGTK's DMA-BUF compositing path measured ~1.7x worse frame times on this project's test
//! hardware (ADR-009 §3: AMD Phoenix/Mesa, not just NVIDIA as first assumed). On Linux,
//! PowerVoice now sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` itself before the WebView is created,
//! unless the variable is already set (the user/launcher decided) or the opt-out
//! `POWERVOICE_WEBKIT_DMABUF=1` is set (keep the default WebKit DMA-BUF renderer). This replaces
//! the old `POWERVOICE_WEBKIT_SAFE` opt-in hook, which required the user to know about the
//! workaround at all.

const DMABUF_VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const OPT_OUT_VAR: &str = "POWERVOICE_WEBKIT_DMABUF";

/// Pure decision (unit-testable without touching the environment): should PowerVoice set the
/// DMA-BUF workaround itself?
pub fn should_set_dmabuf_workaround(is_linux: bool, already_set: bool, opt_out: bool) -> bool {
    is_linux && !already_set && !opt_out
}

/// Applies the ADR-009 Amendment 1 default. **Must** run at the very top of `main`, before any
/// threads start and before the Tauri/WebView builder runs — see the call site in `main.rs` and
/// its own comment.
pub fn apply_dmabuf_default() {
    let is_linux = cfg!(target_os = "linux");
    let already_set = std::env::var_os(DMABUF_VAR).is_some();
    let opt_out = std::env::var(OPT_OUT_VAR).ok().as_deref() == Some("1");

    if should_set_dmabuf_workaround(is_linux, already_set, opt_out) {
        // SAFETY: called at the very top of `main()`, before the Tauri builder / WebView starts
        // any threads — the process is still single-threaded at this point, so there is no
        // concurrent reader or writer of the environment. That single-threaded-startup
        // guarantee is exactly what makes `set_var` sound (it's `unsafe` since edition 2024
        // because a concurrent `getenv`/`setenv` from another thread would be a data race).
        unsafe {
            std::env::set_var(DMABUF_VAR, "1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_on_linux_when_unset_and_not_opted_out() {
        assert!(should_set_dmabuf_workaround(true, false, false));
    }

    #[test]
    fn leaves_untouched_when_already_set() {
        assert!(!should_set_dmabuf_workaround(true, true, false));
    }

    #[test]
    fn leaves_untouched_when_opted_out() {
        assert!(!should_set_dmabuf_workaround(true, false, true));
    }

    #[test]
    fn leaves_untouched_on_non_linux() {
        assert!(!should_set_dmabuf_workaround(false, false, false));
    }

    #[test]
    fn opt_out_and_already_set_together_still_leave_it_untouched() {
        assert!(!should_set_dmabuf_workaround(true, true, true));
    }

    #[test]
    fn non_linux_with_opt_out_and_not_set_is_still_untouched() {
        assert!(!should_set_dmabuf_workaround(false, false, true));
    }
}
