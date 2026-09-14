//! One sandbox process as the host sees it: the child, its control channel, the channel monitor
//! and the sticky fault. Shared between the proxy (control-thread calls) and the watchdog.

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use vox_sandbox_ipc::protocol::{RequestBody, ResponseBody};
use vox_sandbox_ipc::{Fault, Monitor, PeerStatus, SharedRegion};

use crate::rpc::{Rpc, RpcError};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a sandbox stopped serving (sticky: the first one wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxFault {
    /// The process died (killed, crashed) without being asked to stop.
    Crashed,
    /// No heartbeat progress for the hang timeout while fed, or a control request timed out;
    /// the process was killed.
    Hung,
    /// The plugin stopped cleanly without being asked to.
    Exited,
    /// The plugin reported a fatal error.
    Failed,
    /// Offline: a block wasn't ready within the offline timeout (the render aborts).
    OfflineDeadline,
}

impl SandboxFault {
    /// Completes "‹plugin› … and was bypassed" (`AdapterHealth::fault`).
    pub fn reason(self) -> &'static str {
        match self {
            Self::Crashed => "crashed",
            Self::Hung => "stopped responding",
            Self::Exited => "stopped unexpectedly",
            Self::Failed => "reported a fatal error",
            Self::OfflineDeadline => "was too slow for the render",
        }
    }
}

impl From<Fault> for SandboxFault {
    fn from(f: Fault) -> Self {
        match f {
            Fault::Crashed => Self::Crashed,
            Fault::Exited => Self::Exited,
            Fault::Failed => Self::Failed,
            Fault::Hung { .. } => Self::Hung,
        }
    }
}

/// The fault cell shared with the proxy's `AdapterHealth` handle (outlives the process).
#[derive(Default)]
pub(crate) struct FaultCell {
    fault: Mutex<Option<SandboxFault>>,
    /// Set from `process` (no lock there) when an offline block missed its deadline.
    pub(crate) offline_miss: AtomicBool,
}

impl FaultCell {
    pub(crate) fn get(&self) -> Option<SandboxFault> {
        let f = *lock(&self.fault);
        f.or_else(|| {
            self.offline_miss
                .load(Ordering::Acquire)
                .then_some(SandboxFault::OfflineDeadline)
        })
    }

    /// Records `f` unless a fault is already recorded; `true` if it was the first.
    fn set(&self, f: SandboxFault) -> bool {
        let mut g = lock(&self.fault);
        if g.is_some() {
            return false;
        }
        *g = Some(f);
        true
    }
}

pub(crate) struct Sandbox {
    pub(crate) pid: u32,
    /// The host's own mapping of the segment (duplicated for each activation's `Channel`).
    pub(crate) region: SharedRegion,
    child: Mutex<Child>,
    rpc: Mutex<Rpc>,
    monitor: Mutex<Option<Monitor>>,
    pub(crate) fault: Arc<FaultCell>,
    /// The process has exited (reaped).
    exited: AtomicBool,
    /// Being shut down on purpose: an exit is not a fault.
    retiring: AtomicBool,
}

impl Sandbox {
    /// Spawns `cmd` (stdin/stdout piped) through the watchdog thread and starts its channel.
    pub(crate) fn spawn(
        mut cmd: Command,
        region: SharedRegion,
        name: &str,
    ) -> Result<Arc<Self>, String> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = crate::watchdog::spawn(cmd)
            .map_err(|e| format!("couldn't start the plugin sandbox: {e}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("couldn't open the plugin sandbox's control channel".into());
        };
        let rpc = match Rpc::start(stdin, stdout, name) {
            Ok(r) => r,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("couldn't start the control channel: {e}"));
            }
        };
        let sandbox = Arc::new(Self {
            pid: child.id(),
            region,
            child: Mutex::new(child),
            rpc: Mutex::new(rpc),
            monitor: Mutex::new(None),
            fault: Arc::new(FaultCell::default()),
            exited: AtomicBool::new(false),
            retiring: AtomicBool::new(false),
        });
        crate::watchdog::watch(&sandbox);
        Ok(sandbox)
    }

    /// The sticky fault.
    pub(crate) fn fault(&self) -> Option<SandboxFault> {
        self.fault.get()
    }

    /// Whether the process is still usable (no fault, not exited).
    pub(crate) fn is_usable(&self) -> bool {
        self.fault().is_none() && !self.exited.load(Ordering::Acquire)
    }

    /// Records a fault, bypasses the live channel and — for a hang — kills the process.
    pub(crate) fn record(&self, f: SandboxFault) {
        if self.retiring.load(Ordering::Acquire) {
            return;
        }
        self.fault.set(f);
        if let Some(m) = lock(&self.monitor).as_ref() {
            m.force_bypass();
        }
        if f == SandboxFault::Hung {
            self.kill();
        }
    }

    pub(crate) fn kill(&self) {
        let _ = lock(&self.child).kill();
    }

    /// A control request; a timeout is a hang (the process is killed), a closed channel a crash.
    pub(crate) fn call(
        &self,
        body: RequestBody,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<(ResponseBody, Vec<u8>), RpcError> {
        let r = lock(&self.rpc).call(body, payload, timeout);
        match &r {
            Err(RpcError::Timeout) => self.record(SandboxFault::Hung),
            Err(RpcError::Disconnected) => self.record(SandboxFault::Crashed),
            _ => {}
        }
        r
    }

    /// A best-effort request (T-803: parameter text): a timeout is not a hang (the plugin's
    /// main thread may just be busy); a closed channel is still a crash.
    pub(crate) fn call_soft(
        &self,
        body: RequestBody,
        timeout: Duration,
    ) -> Result<(ResponseBody, Vec<u8>), RpcError> {
        let r = lock(&self.rpc).call(body, Vec::new(), timeout);
        if let Err(RpcError::Disconnected) = &r {
            self.record(SandboxFault::Crashed);
        }
        r
    }

    /// Installs (or removes) the monitor of the current activation's channel.
    pub(crate) fn set_monitor(&self, m: Option<Monitor>) {
        *lock(&self.monitor) = m;
    }

    /// Asks the plugin's audio thread to stop (`host_command`), before `Deactivate`.
    pub(crate) fn request_channel_shutdown(&self) {
        if let Some(m) = lock(&self.monitor).as_mut() {
            m.request_shutdown();
        }
    }

    fn peer_status(&self) -> PeerStatus {
        if self.exited.load(Ordering::Acquire) {
            return PeerStatus::Exited;
        }
        let status = PeerStatus::of_child(&mut lock(&self.child));
        if status == PeerStatus::Exited {
            self.exited.store(true, Ordering::Release);
        }
        status
    }

    /// \[watchdog\] One supervision round.
    pub(crate) fn poll(&self) {
        let status = self.peer_status();
        let new_fault = lock(&self.monitor)
            .as_mut()
            .and_then(|m| m.poll(status).new_fault);
        if let Some(f) = new_fault {
            self.record(f.into());
        } else if status == PeerStatus::Exited {
            // Died while inactive (no monitor) or right after the monitor was removed.
            self.record(SandboxFault::Crashed);
        }
    }

    /// \[watchdog\] Starts a deliberate shutdown: `Shutdown`, then close the channel.
    pub(crate) fn begin_retire(&self) {
        self.retiring.store(true, Ordering::Release);
        self.request_channel_shutdown();
        self.set_monitor(None);
        let mut rpc = lock(&self.rpc);
        rpc.notify(RequestBody::Shutdown);
        rpc.close();
    }

    /// \[watchdog\] `true` once the process is gone (reaped); kills it after `deadline`.
    pub(crate) fn reap(&self, deadline: Instant) -> bool {
        if self.peer_status() == PeerStatus::Exited {
            return true;
        }
        if Instant::now() >= deadline {
            let mut c = lock(&self.child);
            let _ = c.kill();
            let _ = c.wait();
            self.exited.store(true, Ordering::Release);
            return true;
        }
        false
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Normally reaped by the watchdog's retire path; this is the last line of defence
        // against an orphan (SIGKILL + wait never blocks for long).
        if !self.exited.load(Ordering::Acquire) {
            let c = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
