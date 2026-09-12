//! `TestGain`: the reference module proving the API is usable (not a product module; the
//! built-in Gain is `modules`' job).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, Hold, LocalizedText, MODULE_API_VERSION,
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModulePreset, ModuleState, ParamEvent,
    ParamFlags, ParamId, ParamInfo, ProcessContext, ProcessStatus, ResponseCurve, StateError, Tail,
    Taper, TelemetryCells, TelemetryInfo, TelemetryKind, Unit, Version, features, segments,
};

const MIN_DB: f64 = -60.0;
const MAX_DB: f64 = 24.0;

/// Reference module: mono gain with one parameter, `gain_db` (−∞…+24 dB, default 0 dB).
///
/// Smoothing: a change at offset k starts a **linear ramp in linear gain** at sample k that
/// reaches the target exactly after `round(sample_rate × 5 ms)` samples (the last ramp sample is
/// the target). `min` (−60 dB) means −∞ dB (silence). Latency 0, no tail.
///
/// State: `{ "gain_db": v }` plus the blob last given to `load_state`, kept verbatim.
/// Extensions: [`Telemetry`](crate::Telemetry) (channel 0 = applied gain in dB) and a flat
/// [`ResponseCurve`].
pub struct TestGain {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    telemetry: Arc<TelemetryCells>,
    curve: Arc<FlatCurve>,
    gain_db: f64,
    blob: Option<Vec<u8>>,
    ramp_len: u32,
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl TestGain {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-gain";
    /// The `gain_db` parameter.
    pub const GAIN_DB: ParamId = ParamId(0);
    /// Declared smoothing of `gain_db`.
    pub const SMOOTHING_MS: f32 = 5.0;
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;

    /// A new, inactive instance at 0 dB.
    pub fn new() -> Self {
        let telemetry = TelemetryCells::new(
            vec![TelemetryInfo {
                id: 0,
                key: "gain_db".into(),
                name: LocalizedText::keyed("telemetry.test_gain.gain", "Gain"),
                unit: Unit::Db,
                min: MIN_DB,
                max: MAX_DB,
                kind: TelemetryKind::Value,
                group: None,
            }],
            vec![Hold::Latest],
        );
        let mut m = Self {
            descriptor: Self::descriptor_value(),
            params: vec![Self::gain_param()],
            telemetry,
            curve: Arc::new(FlatCurve),
            gain_db: 0.0,
            blob: None,
            ramp_len: 0,
            current: 1.0,
            target: 1.0,
            step: 0.0,
            remaining: 0,
        };
        m.snap_to(0.0);
        m
    }

    /// The descriptor every instance returns.
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Version::new(1, 0, 0),
            name: LocalizedText::keyed("module.test_gain.name", "Test Gain"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.test_gain.description",
                "Reference module for tests: smoothed gain.",
            ),
            url: None,
            features: vec![features::UTILITY.into(), features::MONO.into()],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// The `gain_db` parameter schema.
    pub fn gain_param() -> ParamInfo {
        ParamInfo {
            id: Self::GAIN_DB,
            key: "gain_db".into(),
            name: LocalizedText::keyed("param.gain_db", "Gain"),
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

    /// A state with `gain_db = db` (no blob).
    pub fn state_with_gain_db(db: f64) -> ModuleState {
        ModuleState {
            format_version: Self::STATE_FORMAT_VERSION,
            params: BTreeMap::from([("gain_db".to_owned(), db)]),
            blob: None,
        }
    }

    /// Linear gain for `db` (`<= -60` → 0.0, i.e. −∞ dB).
    pub fn db_to_gain(db: f64) -> f32 {
        if db <= MIN_DB {
            0.0
        } else {
            10f64.powf(db / 20.0) as f32
        }
    }

    /// Ramp length in samples at `sample_rate`.
    pub fn ramp_samples(sample_rate: f64) -> u32 {
        (sample_rate * f64::from(Self::SMOOTHING_MS) / 1000.0).round() as u32
    }

    fn snap_to(&mut self, db: f64) {
        self.gain_db = db;
        self.target = Self::db_to_gain(db);
        self.current = self.target;
        self.remaining = 0;
    }

    fn apply(&mut self, ev: &ParamEvent) {
        if ev.id != Self::GAIN_DB {
            return;
        }
        self.gain_db = ev.value;
        self.target = Self::db_to_gain(ev.value);
        if self.ramp_len == 0 {
            self.current = self.target;
            self.remaining = 0;
        } else {
            self.step = (self.target - self.current) / self.ramp_len as f32;
            self.remaining = self.ramp_len;
        }
    }

    fn next_gain(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
        self.current
    }
}

impl Default for TestGain {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for TestGain {
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
                self.apply(ev);
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            let (Some(out), Some(inp)) = (output.get_mut(r.clone()), input.get(r)) else {
                continue;
            };
            for (o, i) in out.iter_mut().zip(inp) {
                *o = *i * self.next_gain();
            }
        }
        let applied_db = if self.current > 0.0 {
            (20.0 * self.current.log10()).clamp(MIN_DB as f32, MAX_DB as f32)
        } else {
            MIN_DB as f32
        };
        self.telemetry.write(0, applied_db);
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        self.current = self.target;
        self.remaining = 0;
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == Self::GAIN_DB).then_some(self.gain_db)
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState {
            blob: self.blob.clone(),
            ..Self::state_with_gain_db(self.gain_db)
        })
    }

    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        let p = &self.params[0];
        let db = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        self.blob.clone_from(&state.blob);
        self.snap_to(db);
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::Telemetry => Some(Extension::Telemetry(self.telemetry.clone())),
            ExtensionId::ResponseCurve => Some(Extension::ResponseCurve(self.curve.clone())),
            _ => None,
        }
    }
}

/// Flat response at `gain_db` (values\[0\]); −∞ at `min`.
struct FlatCurve;

impl ResponseCurve for FlatCurve {
    fn magnitude_db(
        &self,
        values: &[f64],
        _sample_rate: f64,
        _freqs_hz: &[f64],
        out_db: &mut [f64],
    ) {
        let db = values.first().copied().unwrap_or(0.0);
        out_db.fill(if db <= MIN_DB { f64::NEG_INFINITY } else { db });
    }
}

/// Factory for [`TestGain`], with one preset (`"minus_6_db"`).
pub struct TestGainFactory {
    descriptor: ModuleDescriptor,
}

impl TestGainFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: TestGain::descriptor_value(),
        }
    }
}

impl Default for TestGainFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for TestGainFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TestGain::new()))
    }

    fn presets(&self) -> Vec<ModulePreset> {
        vec![ModulePreset {
            key: "minus_6_db".into(),
            name: LocalizedText::keyed("preset.test_gain.minus_6_db", "\u{2212}6 dB"),
            state: TestGain::state_with_gain_db(-6.0),
        }]
    }
}
