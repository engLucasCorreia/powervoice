//! Built-in **Parametric EQ** (`org.powervoice.parametric-eq@1.0.0`, SPEC-015), lean slice
//! (S3-03; the graph is S3-07).
//!
//! Signal flow (SPEC-015 §4.1): `x → HP → L → 1 → 2 → 3 → 4 → 5 → H → LP → × master → y`, f64
//! inside, f32 at the edges. Fixed band roles: Butterworth HP/LP 6–48 dB/oct (default off), RBJ
//! low/high shelves (Q form) and five RBJ peaks (all on at 0 dB by default, so the default EQ
//! passes audio bit-exact). Every continuous parameter glides over 20 ms (log Hz, dB, ln Q; the
//! master gain linearly in linear gain) with per-sample coefficient recompute while ramping;
//! band on/off and slope changes crossfade over 20 ms (`vox_dsp::eq::band`). Latency 0; the tail
//! is the worst case over the parameter ranges (§4.8).
//!
//! [`ResponseCurve`] evaluates the **target** response from the same `vox_dsp::eq::coeffs`
//! functions as `process()` (same 0.49·fs clamp), so the graph is exact.

use std::sync::Arc;

use vox_dsp::dynamics::db_to_lin;
use vox_dsp::dynamics::ramp::LinearRamp;
use vox_dsp::eq::band::{GainBand, PassBand};
use vox_dsp::eq::coeffs::{Biquad, GainShape, PassKind, Sections, butterworth};
use vox_dsp::eq::{MAX_SECTIONS, ramp_samples};
use vox_module_api::{
    ActivateConfig, ChannelLayout, CurveHandle, Extension, ExtensionId, LocalizedText,
    MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModulePreset,
    ModuleState, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus, ResponseCurve,
    StateError, Tail, Unit, Version, features, segments,
};

use crate::schema::{self, ParamBuild, param, switch};

const PREFIX: &str = "module.parametric_eq";
const BANDS: usize = 9;
/// Shelf Q default: SPEC-015 §3 fixes the plain value 0.7071 (not 1/√2) so the default state is
/// the documented number.
#[allow(clippy::approx_constant)]
const SHELF_Q_DEFAULT: f64 = 0.7071;
const SLOPE_LABELS: [(&str, &str); 8] = [
    ("6", "6 dB/oct"),
    ("12", "12 dB/oct"),
    ("18", "18 dB/oct"),
    ("24", "24 dB/oct"),
    ("30", "30 dB/oct"),
    ("36", "36 dB/oct"),
    ("42", "42 dB/oct"),
    ("48", "48 dB/oct"),
];

/// Butterworth order for a slope enum value (index 0…7 → N 1…8).
fn order(slope: f64) -> usize {
    slope.round().clamp(0.0, 7.0) as usize + 1
}

/// Shape of gain band `band` (1…7).
fn gain_shape(band: usize) -> GainShape {
    match band {
        1 => GainShape::LowShelf,
        7 => GainShape::HighShelf,
        _ => GainShape::Peak,
    }
}

/// The built-in Parametric EQ.
pub struct ParametricEq {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// Plain target values in `params()` order.
    values: Vec<f64>,
    curve: Arc<EqCurve>,
    tail_samples: u64,
    hp: PassBand,
    /// Low shelf, peaks 1…5, high shelf.
    bands: [GainBand; 7],
    lp: PassBand,
    /// Master gain, linear.
    master: LinearRamp,
}

impl ParametricEq {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.parametric-eq";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;
    /// Smoothing / crossfade time of every parameter.
    pub const SMOOTHING_MS: f32 = 20.0;

    /// Band keys in processing order (= `ResponseCurve` component index).
    pub const BAND_KEYS: [&'static str; 9] = ["hp", "ls", "b1", "b2", "b3", "b4", "b5", "hs", "lp"];

    /// Master gain (dB).
    pub const MASTER_GAIN_DB: ParamId = ParamId(0);
    /// Band field: on/off.
    pub const ON: u32 = 0;
    /// Band field: frequency (Hz).
    pub const FREQ: u32 = 1;
    /// Band field: gain (dB), shelves and peaks.
    pub const GAIN: u32 = 2;
    /// Band field: Q, shelves and peaks.
    pub const Q: u32 = 3;
    /// Band field: slope enum, HP/LP.
    pub const SLOPE: u32 = 4;

    /// The id of `field` of band `band` (0 = HP … 8 = LP): `10·(band + 1) + field`.
    pub const fn param_id(band: usize, field: u32) -> ParamId {
        ParamId(10 * (band as u32 + 1) + field)
    }

    /// A new, inactive instance with default values.
    pub fn new() -> Self {
        let params = Self::schema();
        let values = params.iter().map(|p| p.default).collect();
        let hp = PassBand::new(PassKind::HighPass, 80.0, 4, false);
        let lp = PassBand::new(PassKind::LowPass, 12_000.0, 4, false);
        let band = |b: usize| GainBand::new(gain_shape(b), 1_000.0, 0.0, 1.0, true);
        let mut m = Self {
            descriptor: Self::descriptor_value(),
            curve: Arc::new(EqCurve::new(params.clone())),
            params,
            groups: Self::groups_value(),
            values,
            tail_samples: Self::worst_case_tail_samples(48_000.0),
            hp,
            bands: [
                band(1),
                band(2),
                band(3),
                band(4),
                band(5),
                band(6),
                band(7),
            ],
            lp,
            master: LinearRamp::new(1.0),
        };
        m.configure();
        m
    }

    /// The descriptor every instance returns (SPEC-015 §2.1).
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.parametric_eq.name", "Parametric EQ"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.parametric_eq.description",
                "High-pass, low-pass, two shelves and five peaks.",
            ),
            url: None,
            features: vec![features::EQUALIZER.into(), features::MONO.into()],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// Parameter schema (SPEC-015 §3), in table order.
    pub fn schema() -> Vec<ParamInfo> {
        let sm = Self::SMOOTHING_MS;
        let on = |id: u32, key: &str, name: &str, group: u32, default: bool| {
            param(PREFIX, id, key, name)
                .toggle(default)
                .in_group(group)
                .smoothing(sm)
        };
        let freq = |id: u32, key: &str, group: u32, default: f64| {
            param(PREFIX, id, key, "Frequency")
                .log(Unit::Hz, 20.0, 20_000.0, default)
                .decimals(0)
                .in_group(group)
                .smoothing(sm)
        };
        let gain = |id: u32, key: &str, group: u32| {
            param(PREFIX, id, key, "Gain")
                .db(Unit::Db, -24.0, 24.0, 0.0)
                .decimals(1)
                .in_group(group)
                .smoothing(sm)
        };
        let q = |id: u32, key: &str, group: u32, min: f64, max: f64, default: f64| {
            param(PREFIX, id, key, "Q")
                .log(Unit::None, min, max, default)
                .decimals(2)
                .in_group(group)
                .smoothing(sm)
        };
        let slope = |id: u32, key: &str, group: u32| {
            param(PREFIX, id, key, "Slope")
                .choice(&SLOPE_LABELS, 3.0)
                .in_group(group)
                .smoothing(sm)
        };
        let mut v = vec![
            param(PREFIX, 0, "master_gain_db", "Master gain")
                .db(Unit::Db, -24.0, 24.0, 0.0)
                .decimals(1)
                .smoothing(sm),
            on(10, "hp_on", "High-pass on", 1, false),
            freq(11, "hp_freq_hz", 1, 80.0),
            slope(14, "hp_slope", 1),
            on(20, "ls_on", "Low shelf on", 2, true),
            freq(21, "ls_freq_hz", 2, 100.0),
            gain(22, "ls_gain_db", 2),
            q(23, "ls_q", 2, 0.3, 2.0, SHELF_Q_DEFAULT),
        ];
        for (k, default_hz) in [200.0, 500.0, 1_200.0, 3_000.0, 6_000.0]
            .into_iter()
            .enumerate()
        {
            let n = k as u32 + 1;
            let (base, group) = (20 + 10 * n, 2 + n);
            v.extend([
                on(
                    base,
                    &format!("b{n}_on"),
                    &format!("Band {n} on"),
                    group,
                    true,
                ),
                freq(base + 1, &format!("b{n}_freq_hz"), group, default_hz),
                gain(base + 2, &format!("b{n}_gain_db"), group),
                q(base + 3, &format!("b{n}_q"), group, 0.1, 30.0, 1.0),
            ]);
        }
        v.extend([
            on(80, "hs_on", "High shelf on", 8, true),
            freq(81, "hs_freq_hz", 8, 10_000.0),
            gain(82, "hs_gain_db", 8),
            q(83, "hs_q", 8, 0.3, 2.0, SHELF_Q_DEFAULT),
            on(90, "lp_on", "Low-pass on", 9, false),
            freq(91, "lp_freq_hz", 9, 12_000.0),
            slope(94, "lp_slope", 9),
        ]);
        v
    }

    /// Groups 1…9 in processing order, collapsed, each enabled by its `*_on`.
    pub fn groups_value() -> Vec<ParamGroup> {
        let groups = [
            ("hp", "High-pass"),
            ("low_shelf", "Low shelf"),
            ("band_1", "Band 1"),
            ("band_2", "Band 2"),
            ("band_3", "Band 3"),
            ("band_4", "Band 4"),
            ("band_5", "Band 5"),
            ("high_shelf", "High shelf"),
            ("lp", "Low-pass"),
        ];
        groups
            .iter()
            .enumerate()
            .map(|(i, (key, name))| {
                let id = i as u32 + 1;
                schema::group(PREFIX, id, key, name, Some(10 * id), true)
            })
            .collect()
    }

    /// `tail()` at `sample_rate` (SPEC-015 §4.8): `ceil(1.1 × 144 dB / d_min)` with d_min the
    /// slowest decay (dB/sample) reachable: the peak at 20 Hz, Q 30, +24 dB.
    pub fn worst_case_tail_samples(sample_rate: f64) -> u64 {
        let r = Biquad::peaking(20.0, 30.0, 24.0, sample_rate).pole_radius();
        let d_min = -20.0 * r.log10();
        let n = (1.1 * 144.0 / d_min).ceil();
        if n.is_finite() && n > 0.0 {
            n as u64
        } else {
            0
        }
    }

    /// Routes one plain value to the DSP (starting a ramp or fade).
    fn apply(&mut self, id: ParamId, v: f64) {
        if id == Self::MASTER_GAIN_DB {
            self.master.set_target(db_to_lin(v));
            return;
        }
        let (band, field) = ((id.0 / 10) as usize, id.0 % 10);
        let on = switch(v) >= 0.5;
        match band {
            1 | 9 => {
                let b = if band == 1 {
                    &mut self.hp
                } else {
                    &mut self.lp
                };
                match field {
                    Self::ON => b.set_on(on),
                    Self::FREQ => b.set_freq_hz(v),
                    Self::SLOPE => b.set_order(order(v)),
                    _ => {}
                }
            }
            2..=8 => {
                let b = &mut self.bands[band - 2];
                match field {
                    Self::ON => b.set_on(on),
                    Self::FREQ => b.set_freq_hz(v),
                    Self::GAIN => b.set_gain_db(v),
                    Self::Q => b.set_q(v),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// A parameter event: store the clamp-quantized value, route it. RT-safe.
    fn set_param(&mut self, id: ParamId, value: f64) {
        if let Some(i) = schema::index_of(&self.params, id) {
            let v = self.params[i].clamp_quantize(value);
            self.values[i] = v;
            self.apply(id, v);
        }
    }

    /// Every value routed, ramps and fades at their targets.
    fn configure(&mut self) {
        for i in 0..self.params.len() {
            let (id, v) = (self.params[i].id, self.values[i]);
            self.apply(id, v);
        }
        self.hp.snap();
        for b in &mut self.bands {
            b.snap();
        }
        self.lp.snap();
        self.master.snap();
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let mut s = self.hp.tick(f64::from(x));
        for b in &mut self.bands {
            s = b.tick(s);
        }
        s = self.lp.tick(s);
        // × 1.0 is exact, so neutral settings stay bit-exact.
        (s * self.master.advance()) as f32
    }

    fn flush_denormals(&mut self) {
        self.hp.flush_denormals();
        for b in &mut self.bands {
            b.flush_denormals();
        }
        self.lp.flush_denormals();
    }
}

impl Default for ParametricEq {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for ParametricEq {
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
        let fs = config.sample_rate;
        self.hp.set_sample_rate(fs);
        for b in &mut self.bands {
            b.set_sample_rate(fs);
        }
        self.lp.set_sample_rate(fs);
        self.master.set_len(ramp_samples(fs));
        self.tail_samples = Self::worst_case_tail_samples(fs);
        self.configure();
        self.reset();
        Ok(())
    }

    fn deactivate(&mut self) {}

    fn latency_samples(&self) -> u32 {
        0
    }

    fn tail(&self) -> Tail {
        Tail::Samples(self.tail_samples)
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
                self.set_param(ev.id, ev.value);
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            let (Some(out), Some(inp)) = (output.get_mut(r.clone()), input.get(r)) else {
                continue;
            };
            for (o, &x) in out.iter_mut().zip(inp) {
                *o = self.tick(x);
            }
        }
        if ctx.frames > 0 {
            self.flush_denormals();
        }
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        self.hp.reset();
        for b in &mut self.bands {
            b.reset();
        }
        self.lp.reset();
        self.master.snap();
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        schema::index_of(&self.params, id).map(|i| self.values[i])
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(schema::save_values(
            &self.params,
            &self.values,
            Self::STATE_FORMAT_VERSION,
        ))
    }

    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        schema::load_values(&self.params, state, &mut self.values);
        self.configure();
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::ResponseCurve => {
                let c: Arc<dyn ResponseCurve> = self.curve.clone();
                Some(Extension::ResponseCurve(c))
            }
            _ => None,
        }
    }
}

/// The EQ's [`ResponseCurve`]: stateless, evaluates any values at any rate.
struct EqCurve {
    params: Vec<ParamInfo>,
    handles: Vec<CurveHandle>,
}

impl EqCurve {
    fn new(params: Vec<ParamInfo>) -> Self {
        let handles = (0..BANDS)
            .map(|b| {
                let gain_band = (1..=7).contains(&b);
                CurveHandle {
                    component: b,
                    freq: ParametricEq::param_id(b, ParametricEq::FREQ),
                    gain: gain_band.then_some(ParametricEq::param_id(b, ParametricEq::GAIN)),
                    q: gain_band.then_some(ParametricEq::param_id(b, ParametricEq::Q)),
                    enable: Some(ParametricEq::param_id(b, ParametricEq::ON)),
                }
            })
            .collect();
        Self { params, handles }
    }

    /// Clamp-quantized value of `id` (missing → default), as the host would set it.
    fn value(&self, values: &[f64], id: ParamId) -> f64 {
        schema::index_of(&self.params, id).map_or(0.0, |i| {
            let p = &self.params[i];
            p.clamp_quantize(values.get(i).copied().unwrap_or(p.default))
        })
    }

    fn field(&self, values: &[f64], band: usize, field: u32) -> f64 {
        self.value(values, ParametricEq::param_id(band, field))
    }

    /// Band `band`'s sections for `values`; `None` = exactly 0 dB (a shelf or peak at 0 dB).
    fn sections(&self, band: usize, values: &[f64], sample_rate: f64) -> Option<Sections> {
        let f = self.field(values, band, ParametricEq::FREQ);
        match band {
            0 | 8 => {
                let kind = if band == 0 {
                    PassKind::HighPass
                } else {
                    PassKind::LowPass
                };
                let n = order(self.field(values, band, ParametricEq::SLOPE));
                Some(butterworth(kind, n, f, sample_rate))
            }
            _ => {
                let g = self.field(values, band, ParametricEq::GAIN);
                if g.abs() <= 0.0 {
                    return None;
                }
                let q = self.field(values, band, ParametricEq::Q);
                let mut coeffs = [Biquad::IDENTITY; MAX_SECTIONS];
                coeffs[0] = gain_shape(band).coeffs(f, q, g, sample_rate);
                Some(Sections { coeffs, len: 1 })
            }
        }
    }

    fn add_band(
        &self,
        band: usize,
        values: &[f64],
        sample_rate: f64,
        freqs_hz: &[f64],
        out_db: &mut [f64],
    ) {
        if let Some(s) = self.sections(band, values, sample_rate) {
            for (o, &f) in out_db.iter_mut().zip(freqs_hz) {
                *o += s.magnitude_db(f, sample_rate);
            }
        }
    }
}

impl ResponseCurve for EqCurve {
    fn magnitude_db(&self, values: &[f64], sample_rate: f64, freqs_hz: &[f64], out_db: &mut [f64]) {
        out_db.fill(self.value(values, ParametricEq::MASTER_GAIN_DB));
        for band in 0..BANDS {
            if switch(self.field(values, band, ParametricEq::ON)) >= 0.5 {
                self.add_band(band, values, sample_rate, freqs_hz, out_db);
            }
        }
    }

    fn component_count(&self, _values: &[f64]) -> usize {
        BANDS
    }

    fn component_magnitude_db(
        &self,
        component: usize,
        values: &[f64],
        sample_rate: f64,
        freqs_hz: &[f64],
        out_db: &mut [f64],
    ) {
        out_db.fill(0.0);
        if component < BANDS {
            self.add_band(component, values, sample_rate, freqs_hz, out_db);
        }
    }

    fn handles(&self) -> &[CurveHandle] {
        &self.handles
    }
}

/// Factory for [`ParametricEq`].
pub struct ParametricEqFactory {
    descriptor: ModuleDescriptor,
}

impl ParametricEqFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: ParametricEq::descriptor_value(),
        }
    }
}

impl Default for ParametricEqFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for ParametricEqFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(ParametricEq::new()))
    }

    /// Factory presets (T-406; SPEC-015 doesn't suggest values, so these cover the module's two
    /// common jobs: a plain rumble filter, and gentle voice tone shaping).
    fn presets(&self) -> Vec<ModulePreset> {
        let params = ParametricEq::schema();
        let state = |overrides: &[(&str, f64)]| {
            schema::preset_state(&params, ParametricEq::STATE_FORMAT_VERSION, overrides)
        };
        vec![
            ModulePreset {
                key: "rumble_filter_hp80".into(),
                name: LocalizedText::keyed(
                    "module.parametric_eq.preset.rumble_filter_hp80",
                    "Rumble filter (HP 80 Hz)",
                ),
                state: state(&[("hp_on", 1.0), ("hp_freq_hz", 80.0)]),
            },
            ModulePreset {
                key: "podcast_presence".into(),
                name: LocalizedText::keyed(
                    "module.parametric_eq.preset.podcast_presence",
                    "Podcast presence",
                ),
                state: state(&[
                    ("hp_on", 1.0),
                    ("hp_freq_hz", 80.0),
                    ("b2_freq_hz", 500.0),
                    ("b2_gain_db", -1.5),
                    ("b4_freq_hz", 3_000.0),
                    ("b4_gain_db", 2.5),
                    ("hs_freq_hz", 10_000.0),
                    ("hs_gain_db", 1.5),
                ]),
            },
            ModulePreset {
                key: "de_mud".into(),
                name: LocalizedText::keyed(
                    "module.parametric_eq.preset.de_mud",
                    "De-mud (cut 300\u{2013}500 Hz)",
                ),
                state: state(&[
                    ("hp_on", 1.0),
                    ("hp_freq_hz", 100.0),
                    ("b1_freq_hz", 300.0),
                    ("b1_gain_db", -1.5),
                    ("b2_freq_hz", 500.0),
                    ("b2_gain_db", -2.0),
                ]),
            },
        ]
    }
}
