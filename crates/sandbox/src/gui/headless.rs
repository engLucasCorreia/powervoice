//! A window system with no windows (T-901 tests, `POWERVOICE_SANDBOX_GUI=headless`): plugin
//! editors open against a pretend window (handle 0), so the whole editor path — the plugin's
//! GUI object, its timers, parameter changes, state — runs without a display and never puts a
//! window on anyone's screen. `close-after-ms` makes the pretend window report that the user
//! closed it.

use std::time::{Duration, Instant};

use super::{Backend, WindowEvent, WindowSpec};

pub(super) struct Headless {
    close_after: Option<Duration>,
    /// The pretend window's creation time.
    window: Option<Instant>,
    closed: bool,
    size: (u32, u32),
}

impl Headless {
    pub(super) fn new(close_after: Option<Duration>) -> Self {
        Self {
            close_after,
            window: None,
            closed: false,
            size: (0, 0),
        }
    }
}

impl Backend for Headless {
    fn create(&mut self, spec: &WindowSpec<'_>) -> Result<u64, String> {
        self.window = Some(Instant::now());
        self.closed = false;
        self.size = (spec.width, spec.height);
        Ok(0)
    }

    fn show(&mut self) {}

    fn raise(&mut self) {}

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }

    fn destroy(&mut self) {
        self.window = None;
    }

    fn fd(&self) -> Option<i32> {
        None
    }

    fn pump(&mut self, out: &mut Vec<WindowEvent>) {
        if let (Some(created), Some(after)) = (self.window, self.close_after)
            && !self.closed
            && created.elapsed() >= after
        {
            self.closed = true;
            out.push(WindowEvent::CloseRequested);
        }
    }
}
