//! Editor side of the plugin sandbox (T-802, ADR-008 + Amendment 2).
//!
//! An out-of-process plugin behaves like any other rack module: [`SandboxFactory`] is a
//! `ModuleFactory` whose instances are [`ProxyModule`]s — each backed by its own
//! `powervoice-sandbox` process:
//!
//! - **Parameters** are mirrored from the plugin's schema (`Load` answer); events travel through
//!   the T-801 event ring with absolute stream positions (sample-accurate); plugin-originated
//!   changes come back as module output events.
//! - **`process()`** is T-801's [`vox_sandbox_ipc::HostEnd`]: no allocation, a bounded wait, a
//!   crossfaded dry substitute on a deadline miss. After a fault it returns
//!   `ProcessStatus::Error`, so the rack bypasses the slot (15 ms crossfade) and applies its
//!   restart policy (rack: restart once with the last good state, then "Failed").
//! - **Latency** = the transport block `B` (pipelined, ADR-008 §2) + the plugin's own latency.
//!   `B` follows the host's callback size ([`SandboxOptions::min_block`], grown by a restart
//!   request when callbacks exceed it).
//! - **State** = the plugin's state wrapped with its format, plugin reference, id and version
//!   ([`state`]), plus the mirrored parameter values (which win on load).
//! - The **watchdog** thread (one per process, [`watchdog`]) spawns the sandboxes, polls every
//!   channel's `Monitor` (crash, hang, clean exit), kills hung processes and reaps retired ones:
//!   no orphan processes, and no named shared-memory object outlives the handshake.

mod factory;
mod proxy;
mod rpc;
mod sandbox;
pub mod state;
pub mod watchdog;

pub use factory::{
    SandboxFactory, SandboxInstance, SandboxOptions, SandboxSpec, TEST_FORMAT, test_factories,
    test_spec,
};
pub use proxy::{MAX_TRANSPORT_BLOCK, ProxyModule};
pub use sandbox::SandboxFault;
