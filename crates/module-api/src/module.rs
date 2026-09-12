//! The `Module` trait (ADR-005 §6–§7).

use crate::descriptor::ModuleDescriptor;
use crate::extension::{Extension, ExtensionId};
use crate::param::{ParamGroup, ParamId, ParamInfo};
use crate::process::{ActivateConfig, ChannelLayout, ProcessContext, ProcessStatus, Tail};
use crate::state::{ModuleState, StateError};

/// A rack processor: built-in module, packaged module or external-plugin adapter.
///
/// A module is `Send`, not `Sync`, and has exactly one owner at a time. It is **live** while it
/// belongs to a chain installed on the audio thread; **a live instance only receives
/// [`process`](Self::process) and [`reset`](Self::reset)**. Every other call happens before it
/// is inserted or after it is removed.
///
/// | Call | Thread | Instance state | May allocate/block |
/// |---|---|---|---|
/// | `descriptor`, `params`, `groups`, `supported_layouts` | any non-audio | any | no |
/// | `activate` / `deactivate` | control | inactive / active, not live | yes |
/// | `latency_samples`, `tail` | control | active, not live | no |
/// | `process`, `reset` | audio (or offline render) | active | **no (RT)** |
/// | `param_value`, `save_state`, `extension` | control | not processing | yes |
/// | `load_state` | control | inactive | yes |
/// | `migrate_state` | control | any, not processing | yes |
///
/// **State rule S1:** a built-in module's persistent state is exactly its parameter values plus
/// the blob last given to `load_state`; `process` never mutates persistent state.
///
/// **Determinism:** given the same state, config, input and events, two instances produce
/// bit-identical output.
pub trait Module: Send {
    // --- Metadata: any non-audio thread, any state. Constant for the instance's lifetime. ---

    /// The module type's descriptor.
    fn descriptor(&self) -> &ModuleDescriptor;
    /// Parameter schema. Display order = slice order. Constant for the instance's lifetime.
    fn params(&self) -> &[ParamInfo];
    /// Parameter groups (display order).
    fn groups(&self) -> &[ParamGroup] {
        &[]
    }
    /// Channel layouts the module can run with.
    fn supported_layouts(&self) -> &[ChannelLayout] {
        &[ChannelLayout::MONO]
    }

    // --- Lifecycle: control thread, never concurrent with process(). ---

    /// \[inactive → active\] May allocate and block (buffers, FFT plans).
    /// On `Ok` the module is in reset state.
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError>;
    /// \[active → inactive\] May deallocate.
    fn deactivate(&mut self);
    /// \[active\] Valid after activate, constant until deactivate.
    /// Output sample n corresponds to input sample n − latency.
    fn latency_samples(&self) -> u32;
    /// \[active\] Valid after activate, constant until deactivate.
    fn tail(&self) -> Tail;

    // --- Audio: the audio thread (or an offline render thread), active only, RT-safe. ---

    /// Renders one block. `inputs.len() == layout.inputs`, `outputs.len() == layout.outputs`,
    /// all slices `ctx.frames` long. No allocation, locks, I/O, logging, syscalls, unbounded
    /// loops; no panic on valid input; output finite for finite input; denormal-safe.
    ///
    /// Events apply sample-accurately: a change at offset k starts affecting output at sample k
    /// (then ramps over the parameter's `smoothing_ms`). A block with `frames == 0` is a legal
    /// parameter flush and must apply its events. See [`segments`](crate::segments).
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus;
    /// \[active\] Seek / loop wrap / transport start: clear delay lines, envelopes, overlap
    /// buffers; smoothers snap to their targets. Parameter values unchanged. RT-safe.
    fn reset(&mut self);

    // --- Values & state: control thread, instance not live. ---

    /// Current target plain value (last event or `load_state`). `None` for unknown ids.
    fn param_value(&self, id: ParamId) -> Option<f64>;
    /// \[active or inactive, not processing\] Parameter values (every non-READ_ONLY key) plus
    /// the committed blob, at `descriptor().state_format_version`.
    fn save_state(&self) -> Result<ModuleState, StateError>;
    /// \[inactive\] `state` is already at `descriptor().state_format_version` (the host calls
    /// [`prepare_state`](crate::prepare_state) first). Unknown keys ignored, missing keys →
    /// default, values clamp_quantized.
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError>;
    /// Upgrades an older state. Default: formats are additive, so the result is `state` with its
    /// `format_version` bumped.
    fn migrate_state(&self, state: ModuleState) -> Result<ModuleState, StateError> {
        Ok(ModuleState {
            format_version: self.descriptor().state_format_version,
            ..state
        })
    }

    // --- Extensions: control thread, not processing. ---

    /// Returns the extension matching `id` (the variant for that id) or `None`.
    /// The host queries every known id once after `activate`, before insertion.
    fn extension(&self, _id: ExtensionId) -> Option<Extension> {
        None
    }
}

/// Error creating or activating a module.
#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    /// The configuration (rate, block size, layout) is not supported.
    #[error("unsupported configuration: {0}")]
    Unsupported(String),
    /// A resource could not be allocated or loaded.
    #[error("resource error: {0}")]
    Resource(String),
    /// An external module (adapter, sandbox) failed.
    #[error("external module failure: {0}")]
    External(String),
}
