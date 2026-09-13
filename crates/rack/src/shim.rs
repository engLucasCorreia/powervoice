//! The dual-mono shim (ADR-005 §8): runs a module that has no MONO layout as a mono module.
//!
//! With `STEREO`, the mono input feeds both inputs and the left output is taken; with
//! `MONO_TO_STEREO`, the left output is taken. Latency, tail, parameters, state and extensions
//! pass through unchanged.

use vox_module_api::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, Module, ModuleDescriptor, ModuleError,
    ModuleState, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail,
};

/// Wraps a stereo-only module so the rack can drive it as MONO.
pub struct DualMonoShim {
    inner: Box<dyn Module>,
    layout: ChannelLayout,
    right: Vec<f32>,
}

impl DualMonoShim {
    /// Returns `module` unchanged if it supports MONO, wrapped if it supports STEREO or
    /// MONO_TO_STEREO, and gives it back as `Err` otherwise ("unsupported channel layout").
    #[allow(clippy::result_large_err)]
    pub fn adapt(module: Box<dyn Module>) -> Result<Box<dyn Module>, Box<dyn Module>> {
        let layouts = module.supported_layouts();
        if layouts.contains(&ChannelLayout::MONO) {
            return Ok(module);
        }
        let layout = if layouts.contains(&ChannelLayout::STEREO) {
            ChannelLayout::STEREO
        } else if layouts.contains(&ChannelLayout::MONO_TO_STEREO) {
            ChannelLayout::MONO_TO_STEREO
        } else {
            return Err(module);
        };
        Ok(Box::new(Self {
            inner: module,
            layout,
            right: Vec::new(),
        }))
    }

    /// The layout the wrapped module runs with.
    pub fn inner_layout(&self) -> ChannelLayout {
        self.layout
    }
}

impl Module for DualMonoShim {
    fn descriptor(&self) -> &ModuleDescriptor {
        self.inner.descriptor()
    }
    fn params(&self) -> &[ParamInfo] {
        self.inner.params()
    }
    fn groups(&self) -> &[ParamGroup] {
        self.inner.groups()
    }
    fn supported_layouts(&self) -> &[ChannelLayout] {
        &[ChannelLayout::MONO]
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        self.inner.activate(&ActivateConfig {
            layout: self.layout,
            ..*config
        })?;
        self.right = vec![0.0; config.max_block as usize];
        Ok(())
    }
    fn deactivate(&mut self) {
        self.inner.deactivate();
        self.right = Vec::new();
    }
    fn latency_samples(&self) -> u32 {
        self.inner.latency_samples()
    }
    fn tail(&self) -> Tail {
        self.inner.tail()
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let n = ctx.frames as usize;
        let x = inputs[0];
        let right = &mut self.right[..n];
        if self.layout == ChannelLayout::STEREO {
            self.inner
                .process(ctx, &[x, x], &mut [&mut *outputs[0], right])
        } else {
            self.inner
                .process(ctx, &[x], &mut [&mut *outputs[0], right])
        }
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        self.inner.param_value(id)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        self.inner.save_state()
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.inner.load_state(state)
    }
    fn migrate_state(&self, state: ModuleState) -> Result<ModuleState, StateError> {
        self.inner.migrate_state(state)
    }
    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        self.inner.extension(id)
    }
}
