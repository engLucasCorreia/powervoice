//! Built-in **Noise Reduction** (`org.powervoice.noise-reduction@1.0.0`, SPEC-014).
//!
//! Spectral noise reduction from a captured noise print: decision-directed Wiener gain with a
//! "Reduce by" floor and dB-domain frequency/time smoothing, on a √Hann STFT with 75 % overlap
//! (`vox_dsp::nr`).
//!
//! - **Parameters** (§3.1): `reduction_db`, `amount_pct`, `noise_only`, then the collapsed
//!   "Advanced" group `fft_size`, `sensitivity_db`, `smoothing_hz`, `attack_ms`, `release_ms`.
//!   Every parameter takes effect at the first frame starting at or after its event (so no output
//!   sample earlier than event + latency changes); smoothing comes from the overlap-add.
//! - **Latency** = FFT size N (2048 = 42.7 ms at 48 kHz by default). An `fft_size` event keeps
//!   the running size and requests [`HostRequest::Restart`]. **Tail** = 2N (see the module
//!   report: frequency-domain gains smear up to N samples past the delayed input).
//! - **State** = the parameters plus the noise-print blob (v1, 32 824 bytes), kept verbatim. A
//!   missing, unreadable or too-new blob → exact N-sample delay (silence with `noise_only`).
//! - **Extension** [`NoiseProfile`]: capture from an excerpt, minimum length, describe.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use vox_dsp::nr::{self, BlobError, CaptureError, NrParams, ProfileView, SpectralNr};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, GroupId, HostRequest, LocalizedText,
    MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleState,
    NoiseProfile, ParamFlags, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus,
    StateError, Tail, Taper, Unit, Version, features, segments,
};

const PARAM_COUNT: usize = 8;

/// The built-in Noise Reduction module.
pub struct NoiseReduction {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// Current plain values, indexed by parameter id.
    values: [f64; PARAM_COUNT],
    /// Committed blob, verbatim (rule S1).
    blob: Option<Vec<u8>>,
    /// FFT size of the active instance (0 before the first activate).
    active_fft_size: u32,
    engine: Option<SpectralNr>,
    profile: Arc<dyn NoiseProfile>,
}

impl NoiseReduction {
    /// Module id (ADR-005 §2).
    pub const ID: &'static str = "org.powervoice.noise-reduction";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;
    /// `reduction_db` (Reduce by).
    pub const REDUCTION_DB: ParamId = ParamId(0);
    /// `amount_pct` (Noise reduction).
    pub const AMOUNT_PCT: ParamId = ParamId(1);
    /// `noise_only` (Output noise only).
    pub const NOISE_ONLY: ParamId = ParamId(2);
    /// `fft_size` (enum index into [`FFT_SIZES`](Self::FFT_SIZES)).
    pub const FFT_SIZE: ParamId = ParamId(3);
    /// `sensitivity_db`.
    pub const SENSITIVITY_DB: ParamId = ParamId(4);
    /// `smoothing_hz` (Spectral smoothing).
    pub const SMOOTHING_HZ: ParamId = ParamId(5);
    /// `attack_ms`.
    pub const ATTACK_MS: ParamId = ParamId(6);
    /// `release_ms`.
    pub const RELEASE_MS: ParamId = ParamId(7);
    /// The "Advanced" group.
    pub const ADVANCED: GroupId = GroupId(0);
    /// FFT sizes offered by `fft_size` (default index 1 = 2048).
    pub const FFT_SIZES: [u32; 4] = [1024, 2048, 4096, 8192];
    /// Declared smoothing of the continuous parameters (covers N ≤ 8192 at ≥ 44.1 kHz).
    pub const SMOOTHING_MS: f32 = 200.0;

    /// A new, inactive instance with default values and no print.
    pub fn new() -> Self {
        let params = Self::param_infos();
        let mut values = [0.0; PARAM_COUNT];
        for (v, p) in values.iter_mut().zip(&params) {
            *v = p.default;
        }
        Self {
            descriptor: Self::descriptor_value(),
            params,
            groups: Self::param_groups(),
            values,
            blob: None,
            active_fft_size: 0,
            engine: None,
            profile: Arc::new(NrNoiseProfile),
        }
    }

    /// The descriptor every instance returns.
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.noise_reduction.name", "Noise Reduction"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.noise_reduction.description",
                "Removes steady background noise using a captured noise print.",
            ),
            url: None,
            features: vec![
                features::AUDIO_EFFECT.into(),
                features::RESTORATION.into(),
                features::MONO.into(),
            ],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    fn name(key: &str, text: &str) -> LocalizedText {
        LocalizedText::keyed(format!("module.noise_reduction.param.{key}"), text)
    }

    fn continuous(
        id: ParamId,
        key: &str,
        text: &str,
        unit: Unit,
        (min, max, default): (f64, f64, f64),
        taper: Taper,
        decimals: u8,
    ) -> ParamInfo {
        ParamInfo {
            id,
            key: key.into(),
            name: Self::name(key, text),
            group: None,
            unit,
            min,
            max,
            default,
            taper,
            step: None,
            enum_labels: Vec::new(),
            decimals,
            smoothing_ms: Self::SMOOTHING_MS,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    /// The parameter schema (SPEC-014 §3.1), in display order.
    pub fn param_infos() -> Vec<ParamInfo> {
        let db = Taper::Db {
            neg_inf_at_min: false,
        };
        let adv = Some(Self::ADVANCED);
        vec![
            Self::continuous(
                Self::REDUCTION_DB,
                "reduction_db",
                "Reduce by",
                Unit::Db,
                (0.0, 40.0, 12.0),
                db,
                1,
            ),
            Self::continuous(
                Self::AMOUNT_PCT,
                "amount_pct",
                "Noise reduction",
                Unit::Percent,
                (0.0, 100.0, 100.0),
                Taper::Linear,
                0,
            ),
            ParamInfo {
                step: Some(1.0),
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE | ParamFlags::STEPPED | ParamFlags::BOOL,
                ..Self::continuous(
                    Self::NOISE_ONLY,
                    "noise_only",
                    "Output noise only",
                    Unit::None,
                    (0.0, 1.0, 0.0),
                    Taper::Linear,
                    0,
                )
            },
            ParamInfo {
                group: adv,
                step: Some(1.0),
                enum_labels: Self::FFT_SIZES
                    .iter()
                    .map(|n| LocalizedText::plain(n.to_string()))
                    .collect(),
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE | ParamFlags::STEPPED,
                ..Self::continuous(
                    Self::FFT_SIZE,
                    "fft_size",
                    "FFT size",
                    Unit::None,
                    (0.0, 3.0, 1.0),
                    Taper::Linear,
                    0,
                )
            },
            ParamInfo {
                group: adv,
                ..Self::continuous(
                    Self::SENSITIVITY_DB,
                    "sensitivity_db",
                    "Sensitivity",
                    Unit::Db,
                    (-6.0, 12.0, 3.0),
                    db,
                    1,
                )
            },
            ParamInfo {
                group: adv,
                ..Self::continuous(
                    Self::SMOOTHING_HZ,
                    "smoothing_hz",
                    "Spectral smoothing",
                    Unit::Hz,
                    (0.0, 1000.0, 100.0),
                    Taper::Linear,
                    0,
                )
            },
            ParamInfo {
                group: adv,
                ..Self::continuous(
                    Self::ATTACK_MS,
                    "attack_ms",
                    "Attack",
                    Unit::Ms,
                    (1.0, 200.0, 5.0),
                    Taper::Log,
                    1,
                )
            },
            ParamInfo {
                group: adv,
                ..Self::continuous(
                    Self::RELEASE_MS,
                    "release_ms",
                    "Release",
                    Unit::Ms,
                    (10.0, 2000.0, 100.0),
                    Taper::Log,
                    0,
                )
            },
        ]
    }

    /// The parameter groups: "Advanced", collapsed by default.
    pub fn param_groups() -> Vec<ParamGroup> {
        vec![ParamGroup {
            id: Self::ADVANCED,
            key: "advanced".into(),
            name: LocalizedText::keyed("module.noise_reduction.group.advanced", "Advanced"),
            parent: None,
            enable_param: None,
            collapsed_by_default: true,
        }]
    }

    /// FFT size for an `fft_size` plain value (enum index, clamped; NaN → default).
    pub fn fft_size_for(index: f64) -> u32 {
        let i = if index.is_finite() {
            index.round().clamp(0.0, 3.0) as usize
        } else {
            1
        };
        Self::FFT_SIZES[i]
    }

    /// A complete state: every parameter at its default, plus `blob`.
    pub fn state_with_blob(blob: Option<Vec<u8>>) -> ModuleState {
        ModuleState {
            format_version: Self::STATE_FORMAT_VERSION,
            params: Self::param_infos()
                .into_iter()
                .map(|p| (p.key, p.default))
                .collect::<BTreeMap<_, _>>(),
            blob,
        }
    }

    /// True while active with a valid print (STFT processing); false for the delay line.
    pub fn has_active_print(&self) -> bool {
        self.engine.as_ref().is_some_and(SpectralNr::has_print)
    }

    /// Diagnostic (tests, SPEC-014 AC-19): any subnormal value in the processing state.
    pub fn state_has_subnormals(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(SpectralNr::state_has_subnormals)
    }

    fn value(&self, id: ParamId) -> f64 {
        let i = id.0 as usize;
        self.params[i].clamp_quantize(self.values[i])
    }

    fn nr_params(&self) -> NrParams {
        NrParams {
            reduction_db: self.value(Self::REDUCTION_DB),
            amount_pct: self.value(Self::AMOUNT_PCT),
            noise_only: self.value(Self::NOISE_ONLY) >= 0.5,
            sensitivity_db: self.value(Self::SENSITIVITY_DB),
            smoothing_hz: self.value(Self::SMOOTHING_HZ),
            attack_ms: self.value(Self::ATTACK_MS),
            release_ms: self.value(Self::RELEASE_MS),
        }
    }

    /// Records an event; true if it asks for a different FFT size than the active one.
    fn apply_event(&mut self, id: ParamId, value: f64) -> bool {
        let Some(v) = self.values.get_mut(id.0 as usize) else {
            return false;
        };
        *v = value;
        id == Self::FFT_SIZE && Self::fft_size_for(value) != self.active_fft_size
    }
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for NoiseReduction {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn groups(&self) -> &[ParamGroup] {
        &self.groups
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
        let n = Self::fft_size_for(self.values[Self::FFT_SIZE.0 as usize]);
        let lambda = self
            .blob
            .as_deref()
            .and_then(|b| ProfileView::parse(b).ok())
            .map(|v| nr::derive_lambda(&v, n as usize, config.sample_rate));
        self.engine = Some(SpectralNr::new(
            n as usize,
            config.sample_rate,
            lambda,
            self.nr_params(),
        ));
        self.active_fft_size = n;
        Ok(())
    }

    fn deactivate(&mut self) {
        self.engine = None;
    }

    fn latency_samples(&self) -> u32 {
        self.active_fft_size
    }

    fn tail(&self) -> Tail {
        Tail::Samples(2 * u64::from(self.active_fft_size))
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let events = ctx.events;
        let mut restart = false;
        let (Some(input), Some(output)) = (inputs.first(), outputs.first_mut()) else {
            return ProcessStatus::Continue;
        };
        for seg in segments(ctx.frames, events) {
            if !seg.events.is_empty() {
                for ev in seg.events {
                    restart |= self.apply_event(ev.id, ev.value);
                }
                let p = self.nr_params();
                if let Some(e) = self.engine.as_mut() {
                    e.set_params(p);
                }
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            let (Some(out), Some(inp)) = (output.get_mut(r.clone()), input.get(r)) else {
                continue;
            };
            match self.engine.as_mut() {
                Some(e) => e.process(inp, out),
                None => out.fill(0.0),
            }
        }
        if restart {
            ctx.request(HostRequest::Restart);
        }
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.reset();
        }
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState {
            format_version: Self::STATE_FORMAT_VERSION,
            params: self
                .params
                .iter()
                .zip(&self.values)
                .map(|(p, &v)| (p.key.clone(), v))
                .collect(),
            blob: self.blob.clone(),
        })
    }

    /// Always succeeds: an unreadable blob is kept verbatim and the module then behaves as
    /// without a print (SPEC-014 §2.5).
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        for (v, p) in self.values.iter_mut().zip(&self.params) {
            *v = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        }
        self.blob = state.blob.clone();
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::NoiseProfile => Some(Extension::NoiseProfile(self.profile.clone())),
            _ => None,
        }
    }
}

/// The [`NoiseProfile`] handle: pure functions of their inputs, safe from any non-audio thread.
struct NrNoiseProfile;

impl NoiseProfile for NrNoiseProfile {
    /// The whole new state blob (v1). `values` are not needed: the analysis size is fixed.
    fn capture(
        &self,
        noise: &[f32],
        sample_rate: f64,
        _values: &[f64],
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, ModuleError> {
        nr::capture_profile(noise, sample_rate, cancel).map_err(|e| match e {
            CaptureError::UnsupportedRate(_) => ModuleError::Unsupported(e.to_string()),
            _ => ModuleError::Resource(e.to_string()),
        })
    }

    fn min_capture_samples(&self, sample_rate: f64, _values: &[f64]) -> usize {
        nr::min_capture_samples(sample_rate)
    }

    /// `Unsupported("newer")` for a blob from a newer build; `Resource("invalid state blob: …")`
    /// for a missing or unreadable one.
    fn describe(&self, blob: &[u8], out: &mut Vec<(f32, f32)>) -> Result<(), ModuleError> {
        match ProfileView::parse(blob) {
            Ok(v) => {
                nr::describe_points(&v, out);
                Ok(())
            }
            Err(BlobError::TooNew(_)) => Err(ModuleError::Unsupported("newer".into())),
            Err(e) => Err(ModuleError::Resource(
                StateError::InvalidBlob(e.to_string()).to_string(),
            )),
        }
    }
}

/// Factory for [`NoiseReduction`].
pub struct NoiseReductionFactory {
    descriptor: ModuleDescriptor,
}

impl NoiseReductionFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: NoiseReduction::descriptor_value(),
        }
    }
}

impl Default for NoiseReductionFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for NoiseReductionFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(NoiseReduction::new()))
    }
}
