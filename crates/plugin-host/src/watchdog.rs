//! The sandbox watchdog (ADR-008 §5): one thread per editor process that
//! - **spawns** every sandbox (so Linux's `PR_SET_PDEATHSIG`, which fires when the *spawning
//!   thread* exits, is tied to a thread that lives as long as the editor);
//! - **supervises** every live sandbox every [`POLL_INTERVAL`]: crash (process gone), hang
//!   (heartbeat stale ≥ the hang timeout → SIGKILL), clean-but-unrequested exit, reported
//!   failure — each flips the channel to bypass and records the fault the proxy reports;
//! - **retires** sandboxes whose proxy was dropped: `Shutdown`, close the channel, wait up to
//!   [`RETIRE_GRACE`], then SIGKILL, and always reap (no zombies, no orphans).
//!
//! It never runs on the audio thread and never blocks on a sandbox.

use std::io;
use std::process::{Child, Command};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::sandbox::Sandbox;

/// Supervision period (ADR-008 §5: "poll it regularly").
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a retired sandbox may take to exit before it is killed.
pub const RETIRE_GRACE: Duration = Duration::from_millis(500);

enum Cmd {
    Spawn(Box<Command>, Sender<io::Result<Child>>),
    Watch(Weak<Sandbox>),
    Retire(Arc<Sandbox>),
}

fn sender() -> &'static Sender<Cmd> {
    static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("sandbox-watchdog".into())
            .spawn(move || run(rx))
            .expect("couldn't start the sandbox watchdog thread");
        tx
    })
}

fn run(rx: mpsc::Receiver<Cmd>) {
    let mut watched: Vec<Weak<Sandbox>> = Vec::new();
    let mut retiring: Vec<(Arc<Sandbox>, Instant)> = Vec::new();
    let mut last_poll = Instant::now();
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Cmd::Spawn(mut cmd, reply)) => {
                let _ = reply.send(cmd.spawn());
            }
            Ok(Cmd::Watch(w)) => watched.push(w),
            Ok(Cmd::Retire(s)) => {
                s.begin_retire();
                watched.retain(|w| !std::ptr::eq(w.as_ptr(), Arc::as_ptr(&s)));
                retiring.push((s, Instant::now() + RETIRE_GRACE));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if last_poll.elapsed() >= POLL_INTERVAL {
            last_poll = Instant::now();
            watched.retain(|w| match w.upgrade() {
                Some(s) => {
                    s.poll();
                    true
                }
                None => false,
            });
            retiring.retain(|(s, deadline)| !s.reap(*deadline));
        }
    }
}

/// Spawns `cmd` on the watchdog thread.
pub(crate) fn spawn(cmd: Command) -> io::Result<Child> {
    let (tx, rx) = mpsc::channel();
    sender()
        .send(Cmd::Spawn(Box::new(cmd), tx))
        .map_err(|_| io::Error::other("the sandbox watchdog is gone"))?;
    rx.recv()
        .map_err(|_| io::Error::other("the sandbox watchdog is gone"))?
}

/// Starts supervising `s` (until it is retired or dropped).
pub(crate) fn watch(s: &Arc<Sandbox>) {
    let _ = sender().send(Cmd::Watch(Arc::downgrade(s)));
}

/// Hands `s` over for a clean shutdown (never blocks).
pub(crate) fn retire(s: Arc<Sandbox>) {
    if let Err(mpsc::SendError(Cmd::Retire(s))) = sender().send(Cmd::Retire(s)) {
        // No watchdog (can't happen while the process runs): kill and reap here.
        s.begin_retire();
        s.reap(Instant::now());
    }
}
