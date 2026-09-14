//! The control-thread side: fault detection without blocking (ADR-008 §5 building block; the
//! watchdog thread, kill and user notice are T-802).

use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::channel::Mapped;
use crate::host::{HostCounters, HostStats};
use crate::layout::{CMD_SHUTDOWN, ControlBlock, PeerState, TELEMETRY_CELLS};
use crate::wakeup::PlatformWakeup;

/// Whether the peer process still exists (the process owner knows; the segment can't tell).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    /// Running (or unknown).
    Alive,
    /// Exited or killed.
    Exited,
}

impl PeerStatus {
    /// Non-blocking check of a child process (`try_wait`, i.e. `waitpid(WNOHANG)`; a crashed
    /// child that hasn't been reaped is a zombie, which a `kill(pid, 0)` probe would miss).
    pub fn of_child(child: &mut Child) -> Self {
        match child.try_wait() {
            Ok(None) => Self::Alive,
            Ok(Some(_)) | Err(_) => Self::Exited,
        }
    }
}

/// A channel fault. The first one flips the host end to permanent bypass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The process is gone without having stopped cleanly (peer gone).
    Crashed,
    /// The plugin stopped cleanly without being asked to.
    Exited,
    /// The plugin reported a fatal error ([`PeerState::Failed`]).
    Failed,
    /// No heartbeat progress for `stale` (≥ the hang timeout).
    Hung {
        /// How long the heartbeat has been stale.
        stale: Duration,
        /// Stuck inside the plugin's processing callback (blame the plugin, not the sandbox).
        in_call: bool,
    },
}

/// One [`Monitor::poll`] result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Health {
    /// The channel's fault, if any (sticky).
    pub fault: Option<Fault>,
    /// The fault detected by *this* poll (report it once).
    pub new_fault: Option<Fault>,
    /// The plugin's lifecycle word.
    pub state: PeerState,
    /// The plugin's heartbeat counter.
    pub heartbeat: u64,
    /// Host-side counters.
    pub counters: HostCounters,
    /// Misses since the previous poll.
    pub new_misses: u64,
    /// Input overruns reported by the plugin.
    pub overruns: u64,
    /// The plugin's process id (0 until attached).
    pub plugin_pid: u32,
}

/// The control thread's view of a channel. Poll it regularly (e.g. every 10–50 ms).
pub struct Monitor {
    map: Mapped,
    stats: Arc<HostStats>,
    hang_timeout: Duration,
    last_heartbeat: u64,
    last_change: Instant,
    last_missed: u64,
    shutdown_requested: bool,
    fault: Option<Fault>,
}

impl Monitor {
    pub(crate) fn new(
        map: Mapped,
        stats: Arc<HostStats>,
        hang_timeout: Duration,
        now: Instant,
    ) -> Self {
        Self {
            map,
            stats,
            hang_timeout,
            last_heartbeat: 0,
            last_change: now,
            last_missed: 0,
            shutdown_requested: false,
            fault: None,
        }
    }

    /// Checks heartbeat, lifecycle word and `peer`; never blocks. On the first fault it sets
    /// the host end's bypass flag (its next `process` outputs dry without waking or waiting).
    pub fn poll(&mut self, peer: PeerStatus) -> Health {
        self.poll_at(peer, Instant::now())
    }

    /// [`poll`](Self::poll) with an explicit clock (tests).
    pub fn poll_at(&mut self, peer: PeerStatus, now: Instant) -> Health {
        let cb = self.map.control();
        let heartbeat = cb.heartbeat.load(Ordering::Acquire);
        let state = PeerState::from_u32(cb.state.load(Ordering::Acquire));
        // The hang clock starts at attach (startup is the control channel's timeout, T-802).
        if heartbeat != self.last_heartbeat || state == PeerState::Created {
            self.last_heartbeat = heartbeat;
            self.last_change = now;
        }
        if peer == PeerStatus::Exited {
            self.bypass_now();
        }
        let mut new_fault = None;
        if self.fault.is_none() {
            let detected = if peer == PeerStatus::Exited || state.is_done() {
                match state {
                    PeerState::Failed => Some(Fault::Failed),
                    _ if self.shutdown_requested => None,
                    PeerState::Stopped => Some(Fault::Exited),
                    _ => Some(Fault::Crashed),
                }
            } else if state == PeerState::Running && !self.shutdown_requested {
                let stale = now.saturating_duration_since(self.last_change);
                (stale >= self.hang_timeout).then(|| Fault::Hung {
                    stale,
                    in_call: cb.in_call.load(Ordering::Acquire) != 0,
                })
            } else {
                None
            };
            if let Some(fault) = detected {
                self.bypass_now();
                self.fault = Some(fault);
                new_fault = Some(fault);
            }
        }
        let counters = self.stats.snapshot();
        let new_misses = counters.missed.saturating_sub(self.last_missed);
        self.last_missed = counters.missed;
        Health {
            fault: self.fault,
            new_fault,
            state,
            heartbeat,
            counters,
            new_misses,
            overruns: cb.overruns.load(Ordering::Relaxed),
            plugin_pid: cb.plugin_pid.load(Ordering::Relaxed),
        }
    }

    /// The sticky fault.
    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }

    /// Host-side counters.
    pub fn counters(&self) -> HostCounters {
        self.stats.snapshot()
    }

    /// The plugin's lifecycle word.
    pub fn state(&self) -> PeerState {
        PeerState::from_u32(self.map.control().state.load(Ordering::Acquire))
    }

    /// Telemetry cell `index` (plugin-written), if in range.
    pub fn telemetry(&self, index: usize) -> Option<f32> {
        (index < TELEMETRY_CELLS)
            .then(|| f32::from_bits(self.map.control().telemetry[index].load(Ordering::Relaxed)))
    }

    /// Asks the plugin to stop (it answers [`crate::Serviced::Shutdown`]); a later clean stop
    /// or exit is then not a fault.
    pub fn request_shutdown(&mut self) {
        self.shutdown_requested = true;
        let cb = self.map.control();
        cb.host_command.store(CMD_SHUTDOWN, Ordering::Release);
        cb.to_plugin.ring::<PlatformWakeup>();
    }

    /// Bypasses the channel permanently without a fault (e.g. the slot is being removed).
    pub fn force_bypass(&self) {
        self.bypass_now();
    }

    /// Sets the bypass flag and rings `to_host`, so a host end waiting for output stops waiting.
    fn bypass_now(&self) {
        if !self.stats.bypass.swap(true, Ordering::AcqRel) {
            self.map.control().to_host.ring::<PlatformWakeup>();
        }
    }

    /// The segment's control block (diagnostics and tests).
    pub fn control(&self) -> &ControlBlock {
        self.map.control()
    }
}
