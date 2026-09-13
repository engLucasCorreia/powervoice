//! Test modules for rack and harness tests (SPEC-012 §6): the §4.3 calibration modules
//! [`TestHardStep`] and [`TestStair64`], plus [`TestDelay`], [`TestNaN`], [`TestReporter`] and
//! [`TestRestart`]. None of them is a product module.

use std::collections::BTreeMap;

use crate::{
    ActivateConfig, HostRequest, LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor,
    ModuleError, ModuleState, ParamFlags, ParamId, ParamInfo, ProcessContext, ProcessStatus,
    StateError, Tail, Taper, Unit, Version, segments,
};

use super::TestGain;

fn descriptor(id: &str, name: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        id: id.into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain(name),
        vendor: "PowerVoice".into(),
        description: LocalizedText::plain("test module"),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    }
}

fn check_config(config: &ActivateConfig) -> Result<(), ModuleError> {
    if !(config.sample_rate.is_finite() && config.sample_rate > 0.0) || config.max_block == 0 {
        return Err(ModuleError::Unsupported(format!("{config:?}")));
    }
    Ok(())
}

fn state_of(params: &[ParamInfo], values: &[f64]) -> ModuleState {
    ModuleState {
        format_version: 1,
        params: params
            .iter()
            .zip(values)
            .filter(|(p, _)| !p.flags.contains(ParamFlags::READ_ONLY))
            .map(|(p, v)| (p.key.clone(), *v))
            .collect::<BTreeMap<_, _>>(),
        blob: None,
    }
}

// ---------------------------------------------------------------------------------------------
// Calibration gains.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Jumps to the target at the event sample.
    HardStep,
    /// 20 ms linear ramp, but the applied gain is only updated every 64 samples.
    Stair64,
}

/// A gain with a deliberately bad transition shape (shared by the calibration modules).
struct ShapedGain {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    shape: Shape,
    gain_db: f64,
    ramp_len: u32,
    start: f64,
    target: f64,
    ramp_pos: u32,
    applied: f32,
    since_update: u32,
}

impl ShapedGain {
    fn new(id: &str, name: &str, shape: Shape, smoothing_ms: f32) -> Self {
        let mut p = TestGain::gain_param();
        p.smoothing_ms = smoothing_ms;
        let mut m = Self {
            descriptor: descriptor(id, name),
            params: vec![p],
            shape,
            gain_db: 0.0,
            ramp_len: 0,
            start: 1.0,
            target: 1.0,
            ramp_pos: 0,
            applied: 1.0,
            since_update: 0,
        };
        m.snap(0.0);
        m
    }

    fn snap(&mut self, db: f64) {
        self.gain_db = db;
        self.target = f64::from(TestGain::db_to_gain(db));
        self.start = self.target;
        self.ramp_pos = self.ramp_len;
        self.applied = self.target as f32;
    }

    fn current(&self) -> f64 {
        if self.ramp_pos >= self.ramp_len {
            self.target
        } else {
            self.start
                + (self.target - self.start) * f64::from(self.ramp_pos) / f64::from(self.ramp_len)
        }
    }

    fn apply(&mut self, value: f64) {
        self.gain_db = value;
        let target = f64::from(TestGain::db_to_gain(value));
        match self.shape {
            Shape::HardStep => {
                self.target = target;
                self.applied = target as f32;
            }
            Shape::Stair64 => {
                self.start = self.current();
                self.target = target;
                self.ramp_pos = 0;
                self.since_update = 0;
            }
        }
    }

    fn next(&mut self) -> f32 {
        if self.shape == Shape::Stair64 {
            if self.since_update == 0 {
                self.applied = self.current() as f32;
            }
            self.since_update = (self.since_update + 1) % 64;
            if self.ramp_pos < self.ramp_len {
                self.ramp_pos += 1;
            }
        }
        self.applied
    }
}

impl Module for ShapedGain {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        check_config(config)?;
        self.ramp_len = (config.sample_rate * 0.020).round() as u32;
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
        let (input, output) = (inputs[0], &mut *outputs[0]);
        for seg in segments(ctx.frames, ctx.events) {
            for ev in seg.events {
                if ev.id == TestGain::GAIN_DB {
                    self.apply(ev.value);
                }
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            for (o, i) in output[r.clone()].iter_mut().zip(&input[r]) {
                *o = *i * self.next();
            }
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        let db = self.gain_db;
        self.snap(db);
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == TestGain::GAIN_DB).then_some(self.gain_db)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(state_of(&self.params, &[self.gain_db]))
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        let p = &self.params[0];
        let v = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        self.snap(v);
        Ok(())
    }
}

/// §4.3 calibration: a gain (`gain_db`, same schema as [`TestGain`], declaring 5 ms smoothing)
/// that **jumps** to its target at the event sample. Must fail §4.3.
pub struct TestHardStep;

impl TestHardStep {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-hard-step";
    /// A new instance at 0 dB (parameter [`TestGain::GAIN_DB`]).
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Box<dyn Module> {
        Box::new(ShapedGain::new(
            Self::ID,
            "Test Hard Step",
            Shape::HardStep,
            5.0,
        ))
    }
}

/// §4.3 calibration: a 20 ms linear ramp in linear gain whose applied value is updated only
/// every 64 samples (block-rate smoothing). Declares 20 ms. Must fail §4.3.
pub struct TestStair64;

impl TestStair64 {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-stair64";
    /// A new instance at 0 dB (parameter [`TestGain::GAIN_DB`]).
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Box<dyn Module> {
        Box::new(ShapedGain::new(
            Self::ID,
            "Test Stair 64",
            Shape::Stair64,
            20.0,
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// Delay lines.

/// Exact delay line of `n` samples (no parameters inside the ring buffer: `line[pos]` is the
/// oldest sample).
#[derive(Default)]
struct Line {
    buf: Vec<f32>,
    pos: usize,
}

impl Line {
    fn alloc(&mut self, n: usize) {
        self.buf = vec![0.0; n];
        self.pos = 0;
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
    fn run(&mut self, input: &[f32], output: &mut [f32]) {
        if self.buf.is_empty() {
            output.copy_from_slice(input);
            return;
        }
        for (o, &i) in output.iter_mut().zip(input) {
            *o = self.buf[self.pos];
            self.buf[self.pos] = i;
            self.pos += 1;
            if self.pos == self.buf.len() {
                self.pos = 0;
            }
        }
    }
}

/// `TestDelay(n)`: output = input delayed by exactly `n` samples, `latency_samples() == n`,
/// tail `n`. No parameters.
pub struct TestDelay {
    descriptor: ModuleDescriptor,
    samples: u32,
    line: Line,
}

impl TestDelay {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-delay";

    /// A delay of `samples`.
    pub fn new(samples: u32) -> Self {
        Self {
            descriptor: descriptor(Self::ID, "Test Delay"),
            samples,
            line: Line::default(),
        }
    }
}

impl Module for TestDelay {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        check_config(config)?;
        self.line.alloc(self.samples as usize);
        Ok(())
    }
    fn deactivate(&mut self) {
        self.line = Line::default();
    }
    fn latency_samples(&self) -> u32 {
        self.samples
    }
    fn tail(&self) -> Tail {
        Tail::Samples(u64::from(self.samples))
    }
    fn process(
        &mut self,
        _ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        self.line.run(inputs[0], outputs[0]);
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        self.line.clear();
    }
    fn param_value(&self, _id: ParamId) -> Option<f64> {
        None
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _state: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}

/// `TestRestart`: an exact delay whose length is the stepped parameter `latency_samples`
/// (0…48 000, step 1). The delay (and `latency_samples()`) is fixed at `activate`; an event
/// changing the value keeps the old delay and requests [`HostRequest::Restart`] (ADR-005 §8),
/// so the host's replacement instance runs with the new latency.
pub struct TestRestart {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    value: f64,
    active_latency: u32,
    line: Line,
}

impl TestRestart {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-restart";
    /// The `latency_samples` parameter.
    pub const LATENCY: ParamId = ParamId(0);

    /// An instance with `latency_samples` = `latency`.
    pub fn new(latency: u32) -> Self {
        Self {
            descriptor: descriptor(Self::ID, "Test Restart"),
            params: vec![ParamInfo {
                id: Self::LATENCY,
                key: "latency_samples".into(),
                name: LocalizedText::plain("Latency"),
                group: None,
                unit: Unit::Samples,
                min: 0.0,
                max: 48_000.0,
                default: 0.0,
                taper: Taper::Linear,
                step: Some(1.0),
                enum_labels: Vec::new(),
                decimals: 0,
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE | ParamFlags::STEPPED,
            }],
            value: f64::from(latency),
            active_latency: 0,
            line: Line::default(),
        }
    }
}

impl Module for TestRestart {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        check_config(config)?;
        self.active_latency = self.value as u32;
        self.line.alloc(self.active_latency as usize);
        Ok(())
    }
    fn deactivate(&mut self) {
        self.line = Line::default();
    }
    fn latency_samples(&self) -> u32 {
        self.active_latency
    }
    fn tail(&self) -> Tail {
        Tail::Samples(u64::from(self.active_latency))
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        for ev in ctx.events {
            if ev.id == Self::LATENCY {
                self.value = ev.value;
            }
        }
        if self.value as u32 != self.active_latency {
            ctx.request(HostRequest::Restart);
        }
        self.line.run(inputs[0], outputs[0]);
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        self.line.clear();
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == Self::LATENCY).then_some(self.value)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(state_of(&self.params, &[self.value]))
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        let p = &self.params[0];
        self.value = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------

/// `TestNaN(from)`: passes audio through, but every output sample at or after absolute sample
/// `from` (counted by `steady_time`) is NaN. For the non-finite guard (SPEC-012 §2.9).
pub struct TestNaN {
    descriptor: ModuleDescriptor,
    from: u64,
}

impl TestNaN {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-nan";

    /// NaN from absolute sample `from`.
    pub fn new(from: u64) -> Self {
        Self {
            descriptor: descriptor(Self::ID, "Test NaN"),
            from,
        }
    }
}

impl Module for TestNaN {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        check_config(config)
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
        for (i, (o, &x)) in outputs[0].iter_mut().zip(inputs[0]).enumerate() {
            *o = if ctx.steady_time + i as u64 >= self.from {
                f32::NAN
            } else {
                x
            };
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, _id: ParamId) -> Option<f64> {
        None
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _state: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}

/// `TestReporter(period)`: passes audio through and reports the READ_ONLY parameter `count`
/// through `out_events`: at every absolute sample `k × period` (k ≥ 1) it reports the value `k`.
pub struct TestReporter {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    period: u64,
    count: f64,
}

impl TestReporter {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.test-reporter";
    /// The READ_ONLY `count` parameter.
    pub const COUNT: ParamId = ParamId(0);

    /// Reports every `period` samples (≥ 1).
    pub fn new(period: u64) -> Self {
        Self {
            descriptor: descriptor(Self::ID, "Test Reporter"),
            params: vec![ParamInfo {
                id: Self::COUNT,
                key: "count".into(),
                name: LocalizedText::plain("Count"),
                group: None,
                unit: Unit::None,
                min: 0.0,
                max: 1e9,
                default: 0.0,
                taper: Taper::Linear,
                step: None,
                enum_labels: Vec::new(),
                decimals: 0,
                smoothing_ms: 0.0,
                flags: ParamFlags::READ_ONLY,
            }],
            period: period.max(1),
            count: 0.0,
        }
    }
}

impl Module for TestReporter {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        check_config(config)
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
        outputs[0].copy_from_slice(inputs[0]);
        let start = ctx.steady_time;
        let end = start + u64::from(ctx.frames);
        // First multiple of `period` (>= period) in [start, end).
        let mut k = start.div_ceil(self.period).max(1);
        while k * self.period < end {
            self.count = k as f64;
            let _ = ctx.out_events.try_push(crate::ParamEvent {
                offset: (k * self.period - start) as u32,
                id: Self::COUNT,
                value: self.count,
            });
            k += 1;
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == Self::COUNT).then_some(self.count)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _state: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}
