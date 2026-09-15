//! The host side of a sandboxed plugin's **editor window** (T-901, ADR-008 §7 and its T-901
//! amendment).
//!
//! The window lives in the sandbox process. The editor only:
//! - asks for it (`OpenEditor`/`CloseEditor` on the control channel);
//! - collects what the sandbox reports unsolicited ([`Notifications`], fed by the control
//!   channel's reader thread): the window closed, the plugin's state changed outside its
//!   parameters, parameters the window changed while the plugin was inactive, and a main-thread
//!   heartbeat that lets the watchdog kill a hung GUI;
//! - answers the host-internal [`PluginEditor`] extension ([`EditorHandle`]) for the rack.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use vox_module_api::{EditorRequest, EditorUpdate, PluginEditor};
use vox_sandbox_ipc::protocol::{Notification, ParamValue, RequestBody, ResponseBody};

use crate::rpc::RpcError;
use crate::sandbox::Sandbox;
use crate::state::{self, StateHeader};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Parameter changes kept at most between two polls (a runaway plugin can't grow the queue
/// without bound; the oldest are dropped — the newest value of a parameter always wins).
const MAX_PENDING_PARAMS: usize = 4096;

/// What one sandbox reported unsolicited (written by its control reader thread, read by the
/// rack's control thread and the watchdog).
pub(crate) struct Notifications {
    epoch: Instant,
    /// The editor window is open (set by an `EditorOpened` reply, cleared by `EditorClosed` and
    /// by the host's own close).
    open: AtomicBool,
    /// Milliseconds since `epoch` at the sandbox's last sign of life (any message).
    alive_ms: AtomicU64,
    params: Mutex<Vec<ParamValue>>,
    state: Mutex<Option<Vec<u8>>>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            open: AtomicBool::new(false),
            alive_ms: AtomicU64::new(0),
            params: Mutex::new(Vec::new()),
            state: Mutex::new(None),
        }
    }
}

impl Notifications {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// \[reader\] Any message from the sandbox: its main thread answered.
    pub(crate) fn note_alive(&self) {
        self.alive_ms.store(self.now_ms(), Ordering::Release);
    }

    /// How long since the sandbox's main thread last said anything.
    pub(crate) fn silent_for(&self) -> Duration {
        Duration::from_millis(
            self.now_ms()
                .saturating_sub(self.alive_ms.load(Ordering::Acquire)),
        )
    }

    /// \[reader\] One unsolicited message (`payload`: the state of `StateChanged`).
    pub(crate) fn handle(&self, n: Notification, payload: Vec<u8>) {
        match n {
            Notification::EditorClosed => self.open.store(false, Ordering::Release),
            Notification::StateChanged => *lock(&self.state) = Some(payload),
            Notification::Params { values } => {
                let mut q = lock(&self.params);
                q.extend(values);
                if q.len() > MAX_PENDING_PARAMS {
                    let excess = q.len() - MAX_PENDING_PARAMS;
                    q.drain(..excess);
                }
            }
            Notification::Alive => {}
        }
    }

    pub(crate) fn set_open(&self, open: bool) {
        if open {
            self.note_alive();
        }
        self.open.store(open, Ordering::Release);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Drains the pending parameter changes and the newest state.
    fn take(&self) -> (Vec<ParamValue>, Option<Vec<u8>>) {
        (
            std::mem::take(&mut *lock(&self.params)),
            lock(&self.state).take(),
        )
    }
}

/// The proxy's [`PluginEditor`] handle. Holds the sandbox weakly: once the proxy is gone the
/// window is gone too (the sandbox is retired), `is_open` is `false` and `open` fails.
pub(crate) struct EditorHandle {
    pub(crate) sandbox: Weak<Sandbox>,
    pub(crate) notes: Arc<Notifications>,
    /// The plugin reported an editor (`PluginInfo::editor`).
    pub(crate) available: bool,
    /// Wraps a reported state like the proxy's own `save_state` does.
    pub(crate) header: StateHeader,
    pub(crate) timeout: Duration,
}

impl EditorHandle {
    fn usable(&self) -> Option<Arc<Sandbox>> {
        self.sandbox.upgrade().filter(|s| s.is_usable())
    }
}

impl PluginEditor for EditorHandle {
    fn available(&self) -> bool {
        self.available
    }

    fn open(&self, request: &EditorRequest) -> Result<(), String> {
        if !self.available {
            return Err("the plugin has no window of its own".into());
        }
        let s = self
            .usable()
            .ok_or_else(|| "the plugin isn't running".to_owned())?;
        let body = RequestBody::OpenEditor {
            title: request.title.clone(),
            parent: request.parent,
        };
        match s.call(body, Vec::new(), self.timeout) {
            Ok((ResponseBody::EditorOpened { .. }, _)) => {
                self.notes.set_open(true);
                Ok(())
            }
            Ok(_) => Err("unexpected answer to OpenEditor".into()),
            Err(RpcError::Remote(message)) => Err(message),
            Err(e) => Err(format!("the plugin {e}")),
        }
    }

    fn close(&self) {
        if self.notes.is_open()
            && let Some(s) = self.usable()
        {
            s.notify(RequestBody::CloseEditor);
        }
        self.notes.set_open(false);
    }

    fn is_open(&self) -> bool {
        self.notes.is_open() && self.usable().is_some()
    }

    fn poll(&self) -> EditorUpdate {
        let (params, state) = self.notes.take();
        EditorUpdate {
            open: self.is_open(),
            params: params.into_iter().map(|v| (v.id, v.value)).collect(),
            state: state.map(|data| state::wrap(&self.header, &data)),
        }
    }

    fn capture_state(&self) -> Option<Vec<u8>> {
        let s = self.usable()?;
        match s.call(RequestBody::SaveState, Vec::new(), self.timeout) {
            Ok((ResponseBody::State, data)) => Some(state::wrap(&self.header, &data)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::ParamId;

    fn header() -> StateHeader {
        StateHeader {
            format: "clap".into(),
            plugin: "{}".into(),
            id: "clap:x".into(),
            version: "1".into(),
        }
    }

    #[test]
    fn notifications_track_the_window_params_and_state() {
        let n = Notifications::default();
        assert!(!n.is_open());
        n.set_open(true);
        assert!(n.is_open());
        n.handle(
            Notification::Params {
                values: vec![
                    ParamValue {
                        id: ParamId(0),
                        value: -12.0,
                    },
                    ParamValue {
                        id: ParamId(1),
                        value: 3.0,
                    },
                ],
            },
            Vec::new(),
        );
        n.handle(Notification::StateChanged, b"gui".to_vec());
        n.handle(Notification::StateChanged, b"gui2".to_vec());
        let (params, state) = n.take();
        assert_eq!(params.len(), 2);
        assert_eq!(
            state.as_deref(),
            Some(&b"gui2"[..]),
            "the newest state wins"
        );
        let (params, state) = n.take();
        assert!(params.is_empty() && state.is_none(), "drained");
        n.handle(Notification::EditorClosed, Vec::new());
        assert!(!n.is_open());
    }

    #[test]
    fn a_flood_of_parameter_changes_is_bounded() {
        let n = Notifications::default();
        let values: Vec<ParamValue> = (0..MAX_PENDING_PARAMS + 10)
            .map(|i| ParamValue {
                id: ParamId(0),
                value: i as f64,
            })
            .collect();
        n.handle(Notification::Params { values }, Vec::new());
        let (params, _) = n.take();
        assert_eq!(params.len(), MAX_PENDING_PARAMS);
        assert_eq!(
            params.last().map(|v| v.value),
            Some((MAX_PENDING_PARAMS + 9) as f64),
            "the newest values are kept"
        );
    }

    #[test]
    fn silence_is_measured_from_the_last_message() {
        let n = Notifications::default();
        n.note_alive();
        assert!(n.silent_for() < Duration::from_secs(1));
    }

    #[test]
    fn a_handle_without_its_sandbox_is_closed_and_inert() {
        let h = EditorHandle {
            sandbox: Weak::new(),
            notes: Arc::new(Notifications::default()),
            available: true,
            header: header(),
            timeout: Duration::from_millis(10),
        };
        h.notes.set_open(true);
        assert!(!h.is_open(), "no sandbox: no window");
        assert!(h.open(&EditorRequest::default()).is_err());
        assert!(h.capture_state().is_none());
        h.notes.handle(Notification::StateChanged, b"s".to_vec());
        let u = h.poll();
        let (hdr, data) = state::unwrap(u.state.as_deref().unwrap()).unwrap();
        assert_eq!((hdr, data), (header(), &b"s"[..]), "states arrive wrapped");
        h.close();
        assert!(!h.notes.is_open());
        let unavailable = EditorHandle {
            available: false,
            ..h
        };
        assert!(
            unavailable
                .open(&EditorRequest::default())
                .unwrap_err()
                .contains("no window")
        );
    }
}
