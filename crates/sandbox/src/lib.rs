//! The plugin sandbox process (T-802, ADR-008 + Amendment 2): one `powervoice-sandbox` process
//! per plugin instance, so a crashing or hanging plugin never takes the editor down.
//!
//! `powervoice-sandbox --shm <handle> --host-pid <pid>`:
//! - opens the shared-memory segment the host created for this instance (T-801 layout);
//! - answers the host's control requests ([`vox_sandbox_ipc::protocol`]) read from stdin, writing
//!   the responses to a private duplicate of stdout (fd 1 is pointed at stderr, so a plugin that
//!   prints can't corrupt the stream);
//! - loads the plugin through a [`PluginBackend`] (T-802: the built-in [`test_backend`]; T-803+
//!   add real formats here, and only here — ADR-008 §8);
//! - on `Activate`, attaches a [`vox_sandbox_ipc::PluginEnd`] and runs the **sandbox audio
//!   thread** (real-time priority when the OS grants it), which serves the host's chunks;
//! - exits on `Shutdown`, when stdin closes (the host is gone), when the host process
//!   disappears, and (Linux) when the host thread that spawned it dies (`PR_SET_PDEATHSIG`).

mod backend;
mod rt;
mod server;
pub mod test_backend;

pub use backend::{ActiveInfo, OUT_EVENT_CAPACITY, PluginBackend, PluginInstance, backends};
pub use server::main;
