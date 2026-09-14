//! Plugin sandbox transport (T-801, ADR-008 + Amendment 1): the host's audio thread hands audio
//! to a sandbox process through shared memory and gets the processed audio back within a
//! bounded budget — or substitutes the latency-matched dry signal with a short crossfade —
//! without ever blocking unboundedly, allocating, locking or logging.
//!
//! # Pieces
//! - [`SharedRegion`] (`shm`): one shared-memory segment per plugin instance (Linux memfd,
//!   POSIX `shm_open`, Windows named file mapping).
//! - [`layout`]: the versioned segment layout — [`layout::ControlBlock`] (header, cursors,
//!   doorbells, heartbeat/state, telemetry), the input/output **sample rings** indexed by
//!   absolute stream position, and one event ring per direction.
//! - [`wakeup`]: the [`Wakeup`] shim (Linux futex; portable spin-then-yield fallback) and the
//!   [`wakeup::Doorbell`] built on it.
//! - [`Channel`]: creates and initialises a segment for a [`ChannelConfig`] and splits into the
//!   host's two ends: [`HostEnd`] (audio thread) and [`Monitor`] (control thread).
//! - [`PluginEnd`]: the sandbox side (attach, serve chunks, heartbeat).
//! - [`test_plugins`]: the four test plugins behind `src/bin/vox-sbx-test-*.rs`.
//!
//! # Protocol (per host callback of `n ≤ max_block` frames at input position `p`)
//! 1. The host writes the input to `in[p, p+n)`, publishes `in_write_pos = p+n` and rings
//!    `to_plugin` (a wake syscall only if the plugin is parked — the ADR-008 exception).
//! 2. It needs output for `[p−L, p−L+n)` (`L` = [`ChannelConfig::latency_samples`], ADR-008's
//!    `B` in the pipelined default). If `out_write_pos` doesn't cover it yet, it waits on
//!    `to_host` at most [`HostOptions::wait`] (a fraction of the block period by default).
//! 3. Available → wet (crossfading back in after a miss); not available → a **miss**: the
//!    dry signal (from a host-local copy of the input) with a splice crossfade, counted.
//! 4. The plugin wakes, processes every published chunk, publishes `out_write_pos`, bumps the
//!    heartbeat and rings `to_host`.
//!
//! Faults — crash (peer process gone), hang (heartbeat stale ≥ [`HostOptions::hang_timeout`]),
//! a failed/stopped plugin — are detected by [`Monitor::poll`] on the control thread without
//! blocking; it flips the host end to permanent bypass (no more wakes or waits).

mod channel;
mod host;
pub mod layout;
mod monitor;
mod plugin;
pub mod shm;
pub mod test_plugins;
pub mod wakeup;

pub use channel::{Channel, ChannelConfig, ChannelError};
pub use host::{BlockOutcome, HostCounters, HostEnd, HostOptions, WaitBudget};
pub use layout::{EventKind, Layout, PeerState, WireEvent};
pub use monitor::{Fault, Health, Monitor, PeerStatus};
pub use plugin::{Chunk, MAX_CHUNKS_PER_SERVICE, PluginEnd, Serviced};
pub use shm::SharedRegion;
#[cfg(target_os = "linux")]
pub use wakeup::FutexWakeup;
pub use wakeup::{Deadline, PlatformWakeup, SpinYieldWakeup, Wakeup};
