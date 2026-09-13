//! Built-in **Gain** (`org.powervoice.gain@1.0.0`, SPEC-012 §3).
//!
//! One parameter, `gain_db`: −60 dB (= −∞, silence) … +24 dB, default 0 dB, `Db` taper with
//! `neg_inf_at_min`, continuous, 1 decimal. A change at offset k starts a **linear ramp in linear
//! gain** at sample k that reaches the target exactly `round(20 ms × rate)` samples later (the
//! last ramp sample is the target). A change during a ramp restarts the ramp from the current
//! gain. Latency 0, no tail. State: `{ "gain_db": v }` (rule S1: no blob).

use std::collections::BTreeMap;

use vox_module_api::{
    ActivateConfig, ChannelLayout, LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor,
    ModuleError, ModuleFactory, ModuleState, ParamFlags, ParamId, ParamInfo, ProcessContext,
    ProcessStatus, StateError, Tail, Taper, Unit, Version, features, segments,
};

const MIN_DB: f64 = -60.0;
const MAX_DB: f64 = 24.0;

/// The built-in Gain module.
pub struct Gain {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    gain_db: f64,
    /// Ramp length in samples (set at activate).
    ramp_len: u32,
    /// Gain at the start of the current ramp.
    start: f64,
    /// Target linear gain.
    target: f64,
    /// Samples of the current ramp already rendered (`>= ramp_len`: at target).
    pos: u32,
}

impl Gain {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.gain";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// The `gain_db` parameter.
    pub const GAIN_DB: ParamId = ParamId(0);
    /// Declared smoothing of `gain_db`.
    pub const SMOOTHING_MS: f32 = 20.0;
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;

    /// A new, inactive instance at 0 dB.
    pub fn new() -> Self {
        let mut m = Self {
            descriptor: Self::descriptor_value(),
            params: vec![Self::gain_param()],
            gain_db: 0.0,
            ramp_len: 0,
            start: 1.0,
            target: 1.0,
            pos: 0,
        };
        m.snap_to(0.0);
        m
    }

    /// The descriptor every instance returns.
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.gain.name", "Gain"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.gain.description",
                "Changes the level, smoothly.",
            ),
            url: None,
            features: vec![
                features::AUDIO_EFFECT.into(),
                features::UTILITY.into(),
                features::MONO.into(),
            ],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// The `gain_db` parameter schema.
    pub fn gain_param() -> ParamInfo {
        ParamInfo {
            id: Self::GAIN_DB,
            key: "gain_db".into(),
            name: LocalizedText::keyed("param.gain.gain_db", "Gain"),
            group: None,
            unit: Unit::Db,
            min: MIN_DB,
            max: MAX_DB,
            default: 0.0,
            taper: Taper::Db {
                neg_inf_at_min: true,
            },
            step: None,
            enum_labels: Vec::new(),
            decimals: 1,
            smoothing_ms: Self::SMOOTHING_MS,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    /// A state with `gain_db = db`.
    pub fn state_with_gain_db(db: f64) -> ModuleState {
        ModuleState {
            format_version: Self::STATE_FORMAT_VERSION,
            params: BTreeMap::from([("gain_db".to_owned(), db)]),
            blob: None,
        }
    }

    /// Linear gain for `db` (`<= −60` → 0.0, i.e. −∞ dB).
    pub fn db_to_gain(db: f64) -> f64 {
        if db <= MIN_DB {
            0.0
        } else {
            10f64.powf(db / 20.0)
        }
    }

    /// Ramp length in samples at `sample_rate`.
    pub fn ramp_samples(sample_rate: f64) -> u32 {
        (sample_rate * f64::from(Self::SMOOTHING_MS) / 1000.0).round() as u32
    }

    fn snap_to(&mut self, db: f64) {
        self.gain_db = db;
        self.target = Self::db_to_gain(db);
        self.start = self.target;
        self.pos = u32::MAX;
    }

    /// Gain of the last rendered sample.
    fn current(&self) -> f64 {
        if self.pos >= self.ramp_len {
            self.target
        } else {
            self.start + (self.target - self.start) * f64::from(self.pos) / f64::from(self.ramp_len)
        }
    }

    fn apply(&mut self, value: f64) {
        let start = self.current();
        self.gain_db = value;
        self.target = Self::db_to_gain(value);
        self.start = start;
        self.pos = 0;
    }

    fn next_gain(&mut self) -> f32 {
        if self.pos < self.ramp_len {
            self.pos += 1;
        }
        self.current() as f32
    }
}

impl Default for Gain {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for Gain {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        if config.layout != ChannelLayout::MONO {
            return Err(ModuleError::Unsupported(format!(
                "layout {:?}",
                config.layout
            )));
        }
        if !(config.sample_rate.is_finite() && config.sample_rate > 0.0) || config.max_block == 0 {
            return Err(ModuleError::Unsupported(format!("{config:?}")));
        }
        self.ramp_len = Self::ramp_samples(config.sample_rate);
        self.reset();
        Ok(())
    }

    fn deactivate(&mut self) {}

    fn latency_samples(&self) -> u32 {
        0
    }

    fn tail(&self) -> Tail {
        Tail::Samples(0)
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let (Some(input), Some(output)) = (inputs.first(), outputs.first_mut()) else {
            return ProcessStatus::Continue;
        };
        for seg in segments(ctx.frames, ctx.events) {
            for ev in seg.events {
                if ev.id == Self::GAIN_DB {
                    self.apply(ev.value);
                }
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            let (Some(out), Some(inp)) = (output.get_mut(r.clone()), input.get(r)) else {
                continue;
            };
            if self.pos >= self.ramp_len {
                let g = self.target as f32;
                for (o, i) in out.iter_mut().zip(inp) {
                    *o = *i * g;
                }
            } else {
                for (o, i) in out.iter_mut().zip(inp) {
                    *o = *i * self.next_gain();
                }
            }
        }
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        let db = self.gain_db;
        self.snap_to(db);
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == Self::GAIN_DB).then_some(self.gain_db)
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(Self::state_with_gain_db(self.gain_db))
    }

    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        let p = &self.params[0];
        let db = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        self.snap_to(db);
        Ok(())
    }
}

/// Factory for [`Gain`].
pub struct GainFactory {
    descriptor: ModuleDescriptor,
}

impl GainFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: Gain::descriptor_value(),
        }
    }
}

impl Default for GainFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for GainFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Gain::new()))
    }
}
