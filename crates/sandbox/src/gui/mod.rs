//! **Plugin editor windows** (T-901, ADR-008 §7 and Amendment 13): the plugin's own GUI runs on
//! the sandbox's main thread, as a floating top-level window of the sandbox process. If the
//! plugin crashes, the window goes with the process; the editor never embeds foreign GUIs.
//!
//! - [`WindowSystem`]: the platform's windows, on the main thread only —
//!   - Linux and the other unixes: X11 through the system's libX11, **loaded at run time**
//!     ([`x11`]); under Wayland that is XWayland, since the plugin GUI APIs are X11-based;
//!   - Windows: a top-level `HWND` whose owner is the editor's window ([`win32`]);
//!   - macOS: not yet ("plugin windows aren't supported on macOS yet");
//!   - `POWERVOICE_SANDBOX_GUI=headless[:close-after-ms=N]`: no window at all (tests — never a
//!     window on a developer's screen); `close-after-ms` pretends the user closed it.
//! - [`EditorHost`]: what a backend gets to open its plugin's editor with — the window API and a
//!   top-level window to embed into, or the editor's own window for a floating plugin.
//! - [`runloop`]: timers and file descriptors plugin GUIs register.

pub(crate) mod runloop;

mod headless;
#[cfg(windows)]
mod win32;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

use std::ffi::CStr;
use std::time::Duration;

/// The environment variable selecting a headless window system (tests).
pub const GUI_ENV: &str = "POWERVOICE_SANDBOX_GUI";

/// A window API as the plugin formats name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    /// X11 window ids.
    X11,
    /// Win32 `HWND`s.
    Win32,
    /// Cocoa `NSView*`s.
    Cocoa,
}

impl Api {
    /// This platform's API.
    pub const fn native() -> Self {
        if cfg!(windows) {
            Self::Win32
        } else if cfg!(target_os = "macos") {
            Self::Cocoa
        } else {
            Self::X11
        }
    }

    /// The CLAP window API string (`CLAP_WINDOW_API_*`).
    pub const fn clap_name(self) -> &'static CStr {
        match self {
            Self::X11 => c"x11",
            Self::Win32 => c"win32",
            Self::Cocoa => c"cocoa",
        }
    }
}

/// A native window handle: an X11 window id, an `HWND`, an `NSView*` (as an integer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWindow {
    /// Which API the handle belongs to.
    pub api: Api,
    /// The handle.
    pub raw: u64,
}

/// What happened to the host window since the last pump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    /// The user asked to close it (or it was destroyed).
    CloseRequested,
    /// The user resized it (client size in pixels).
    Resized(u32, u32),
}

/// A top-level window to create.
#[derive(Clone, Debug)]
pub struct WindowSpec<'a> {
    /// Title.
    pub title: &'a str,
    /// Client width in pixels.
    pub width: u32,
    /// Client height in pixels.
    pub height: u32,
    /// The user may resize it.
    pub resizable: bool,
    /// The editor's own window, to stay above (X11 transient-for, Win32 owner).
    pub transient_for: Option<u64>,
}

/// One window backend: at most one top-level window at a time (one editor per sandbox).
trait Backend {
    fn create(&mut self, spec: &WindowSpec<'_>) -> Result<u64, String>;
    fn show(&mut self);
    fn raise(&mut self);
    fn resize(&mut self, width: u32, height: u32);
    fn destroy(&mut self);
    fn fd(&self) -> Option<i32>;
    fn pump(&mut self, out: &mut Vec<WindowEvent>);
}

enum Mode {
    Native,
    Headless { close_after: Option<Duration> },
}

/// The platform's windows as the sandbox uses them (main thread only). The native system is
/// connected lazily, the first time an editor opens.
pub struct WindowSystem {
    mode: Mode,
    backend: Option<Box<dyn Backend>>,
    has_window: bool,
}

/// Parses [`GUI_ENV`]: `headless` or `headless:close-after-ms=<n>` (anything else: native).
fn parse_mode(value: Option<&str>) -> Mode {
    match value {
        Some(v) if v == "headless" || v.starts_with("headless:") => Mode::Headless {
            close_after: v
                .strip_prefix("headless:close-after-ms=")
                .and_then(|n| n.parse().ok())
                .map(Duration::from_millis),
        },
        _ => Mode::Native,
    }
}

impl WindowSystem {
    /// The window system [`GUI_ENV`] selects (native by default).
    pub fn from_env() -> Self {
        Self {
            mode: parse_mode(std::env::var(GUI_ENV).ok().as_deref()),
            backend: None,
            has_window: false,
        }
    }

    fn backend(&mut self) -> Result<&mut Box<dyn Backend>, String> {
        if self.backend.is_none() {
            let b: Box<dyn Backend> = match &self.mode {
                Mode::Headless { close_after } => Box::new(headless::Headless::new(*close_after)),
                Mode::Native => native_backend()?,
            };
            self.backend = Some(b);
        }
        Ok(self.backend.as_mut().expect("just set"))
    }

    /// The API plugin GUIs use here, or why there are no plugin windows.
    pub fn api(&mut self) -> Result<Api, String> {
        self.backend()?;
        Ok(Api::native())
    }

    /// Creates the top-level window (not shown yet); replaces any previous one.
    pub fn create(&mut self, spec: &WindowSpec<'_>) -> Result<NativeWindow, String> {
        self.destroy();
        let raw = self.backend()?.create(spec)?;
        self.has_window = true;
        Ok(NativeWindow {
            api: Api::native(),
            raw,
        })
    }

    /// A window of the editor (`transient_for` of a floating plugin), in this platform's API.
    pub fn foreign(&self, raw: u64) -> NativeWindow {
        NativeWindow {
            api: Api::native(),
            raw,
        }
    }

    /// Whether a host window exists.
    pub fn has_window(&self) -> bool {
        self.has_window
    }

    /// Maps the window.
    pub fn show(&mut self) {
        if let (true, Some(b)) = (self.has_window, self.backend.as_mut()) {
            b.show();
        }
    }

    /// Brings it to the front.
    pub fn raise(&mut self) {
        if let (true, Some(b)) = (self.has_window, self.backend.as_mut()) {
            b.raise();
        }
    }

    /// Resizes its client area.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let (true, Some(b)) = (self.has_window, self.backend.as_mut()) {
            b.resize(width.max(1), height.max(1));
        }
    }

    /// Destroys it (no-op without one).
    pub fn destroy(&mut self) {
        if self.has_window {
            self.has_window = false;
            if let Some(b) = self.backend.as_mut() {
                b.destroy();
            }
        }
    }

    /// The connection's descriptor to wait on (X11), if any.
    pub fn fd(&self) -> Option<i32> {
        self.backend.as_ref().and_then(|b| b.fd())
    }

    /// Takes the window's events.
    pub fn pump(&mut self, out: &mut Vec<WindowEvent>) {
        if let Some(b) = self.backend.as_mut() {
            b.pump(out);
        }
    }
}

impl Drop for WindowSystem {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native_backend() -> Result<Box<dyn Backend>, String> {
    Ok(Box::new(x11::X11::connect()?))
}

#[cfg(windows)]
fn native_backend() -> Result<Box<dyn Backend>, String> {
    Ok(Box::new(win32::Win32::new()))
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn native_backend() -> Result<Box<dyn Backend>, String> {
    Err("plugin windows aren't supported on this platform yet".into())
}

/// What a backend opens its plugin's editor with ([`crate::PluginInstance::open_editor`]).
pub struct EditorHost<'a> {
    windows: &'a mut WindowSystem,
    title: &'a str,
    parent: Option<u64>,
    created: bool,
}

impl<'a> EditorHost<'a> {
    /// For `OpenEditor { title, parent }`.
    pub fn new(windows: &'a mut WindowSystem, title: &'a str, parent: Option<u64>) -> Self {
        Self {
            windows,
            title,
            parent,
            created: false,
        }
    }

    /// The window title ("‹Plugin› — PowerVoice").
    pub fn title(&self) -> &str {
        self.title
    }

    /// The window API plugin GUIs use here, or why there are no plugin windows.
    pub fn api(&mut self) -> Result<Api, String> {
        self.windows.api()
    }

    /// The editor's own window, for a floating plugin window to stay above.
    pub fn transient_for(&self) -> Option<NativeWindow> {
        self.parent.map(|raw| self.windows.foreign(raw))
    }

    /// Creates the top-level window an embedded plugin GUI goes into (the sandbox shows it once
    /// the backend returns).
    pub fn create_window(
        &mut self,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> Result<NativeWindow, String> {
        let w = self.windows.create(&WindowSpec {
            title: self.title,
            width: width.max(1),
            height: height.max(1),
            resizable,
            transient_for: self.parent,
        })?;
        self.created = true;
        Ok(w)
    }

    /// Resizes the window [`Self::create_window`] made (a GUI that asked for another size while
    /// it was being created).
    pub fn resize_window(&mut self, width: u32, height: u32) {
        if self.created {
            self.windows.resize(width, height);
        }
    }

    /// Whether [`Self::create_window`] was used (embedded) — else the plugin floats on its own.
    pub fn created(&self) -> bool {
        self.created
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_selects_headless_windows() {
        assert!(matches!(parse_mode(None), Mode::Native));
        assert!(matches!(parse_mode(Some("x11")), Mode::Native));
        assert!(matches!(
            parse_mode(Some("headless")),
            Mode::Headless { close_after: None }
        ));
        match parse_mode(Some("headless:close-after-ms=250")) {
            Mode::Headless { close_after } => {
                assert_eq!(close_after, Some(Duration::from_millis(250)));
            }
            Mode::Native => panic!("expected headless"),
        }
    }

    #[test]
    fn a_headless_host_creates_one_window_at_a_time() {
        let mut ws = WindowSystem {
            mode: Mode::Headless {
                close_after: Some(Duration::ZERO),
            },
            backend: None,
            has_window: false,
        };
        assert_eq!(ws.api().unwrap(), Api::native());
        let mut host = EditorHost::new(&mut ws, "Gain — PowerVoice", Some(0x4a0_0003));
        assert_eq!(host.title(), "Gain — PowerVoice");
        assert_eq!(host.transient_for().map(|w| w.raw), Some(0x4a0_0003));
        assert!(!host.created());
        let w = host.create_window(320, 200, false).unwrap();
        assert!(host.created());
        assert_eq!(w.api, Api::native());
        assert!(ws.has_window());
        ws.show();
        let mut events = Vec::new();
        ws.pump(&mut events);
        assert_eq!(events, vec![WindowEvent::CloseRequested]);
        ws.destroy();
        assert!(!ws.has_window());
        events.clear();
        ws.pump(&mut events);
        assert!(events.is_empty(), "no window, no events");
    }
}
