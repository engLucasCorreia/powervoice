//! `ModuleTestHost` self-tests: each deliberately broken module must fail the right check.

use std::sync::atomic::{AtomicU32, Ordering};

use vox_module_api::test_util::{HostFailure, ModuleTestHost, TestGain, alloc_checks_active};
use vox_module_api::{
    ActivateConfig, Extension, ExtensionId, LocalizedText, Module, ModuleDescriptor, ModuleError,
    ModuleState, ParamFlags, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail,
    Taper, Unit, telemetry,
};

vox_module_api::install_test_allocator!();

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    None,
    AllocInProcess,
    AllocInReset,
    NanOutput,
    IgnoreFlush,
    EventsAtBlockStart,
    LossyState,
    Nondeterministic,
    StaleParamValue,
    BadSchema,
    AmbiguousEnumLabels,
    WrongExtension,
}

static INSTANCES: AtomicU32 = AtomicU32::new(0);

/// `TestGain` with one injected fault.
struct Faulty {
    inner: TestGain,
    fault: Fault,
    params: Vec<ParamInfo>,
    instance: u32,
}

impl Faulty {
    fn boxed(fault: Fault) -> Box<dyn Module> {
        let inner = TestGain::new();
        let mut params = inner.params().to_vec();
        match fault {
            Fault::BadSchema => params[0].default = 100.0,
            Fault::AmbiguousEnumLabels => {
                // Valid schema, but "low" parses back to index 0 ("Low").
                let p = &mut params[0];
                p.unit = Unit::None;
                p.taper = Taper::Linear;
                (p.min, p.max, p.default, p.step) = (0.0, 1.0, 0.0, Some(1.0));
                p.flags |= ParamFlags::STEPPED;
                p.enum_labels = vec![LocalizedText::plain("Low"), LocalizedText::plain("low")];
            }
            _ => {}
        }
        Box::new(Self {
            inner,
            fault,
            params,
            instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
        })
    }
}

impl Module for Faulty {
    fn descriptor(&self) -> &ModuleDescriptor {
        self.inner.descriptor()
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        self.inner.activate(config)
    }
    fn deactivate(&mut self) {
        self.inner.deactivate();
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
        match self.fault {
            Fault::AllocInProcess => drop(std::hint::black_box(vec![0u8; 16])),
            Fault::IgnoreFlush if ctx.frames == 0 => return ProcessStatus::Continue,
            Fault::EventsAtBlockStart => {
                // Apply every event before the block (as if all offsets were 0).
                let mut flush = ProcessContext::new(
                    0,
                    ctx.steady_time,
                    ctx.transport,
                    ctx.events,
                    ctx.out_events,
                );
                self.inner.process(&mut flush, &[&[]], &mut [&mut []]);
                let mut rest = ProcessContext::new(
                    ctx.frames,
                    ctx.steady_time,
                    ctx.transport,
                    &[],
                    ctx.out_events,
                );
                return self.inner.process(&mut rest, inputs, outputs);
            }
            _ => {}
        }
        let status = self.inner.process(ctx, inputs, outputs);
        let out = &mut *outputs[0];
        match self.fault {
            Fault::NanOutput if out.len() > 2 => out[out.len() / 2] = f32::NAN,
            Fault::Nondeterministic => {
                for x in out.iter_mut() {
                    *x += self.instance as f32 * 1e-3;
                }
            }
            _ => {}
        }
        status
    }
    fn reset(&mut self) {
        if self.fault == Fault::AllocInReset {
            drop(std::hint::black_box(Box::new(1u64)));
        }
        self.inner.reset();
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        if self.fault == Fault::StaleParamValue {
            return (id == TestGain::GAIN_DB).then_some(0.0);
        }
        self.inner.param_value(id)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        if self.fault == Fault::LossyState {
            let mut s = self.inner.save_state()?;
            s.params.insert("gain_db".into(), 1.0);
            return Ok(s);
        }
        self.inner.save_state()
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.inner.load_state(state)
    }
    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        if self.fault == Fault::WrongExtension && id == ExtensionId::ResponseCurve {
            return telemetry(&self.inner).map(Extension::Telemetry);
        }
        self.inner.extension(id)
    }
}

fn host(fault: Fault) -> ModuleTestHost {
    ModuleTestHost::new(move || Faulty::boxed(fault)).blocks(64)
}

/// `TestGain` followed by a delay line of `actual` samples, reporting `reported` latency.
struct Delayed {
    inner: TestGain,
    reported: u32,
    actual: usize,
    line: Vec<f32>,
    pos: usize,
}

impl Delayed {
    fn boxed(actual: usize, reported: u32) -> Box<dyn Module> {
        Box::new(Self {
            inner: TestGain::new(),
            reported,
            actual,
            line: Vec::new(),
            pos: 0,
        })
    }
}

impl Module for Delayed {
    fn descriptor(&self) -> &ModuleDescriptor {
        self.inner.descriptor()
    }
    fn params(&self) -> &[ParamInfo] {
        self.inner.params()
    }
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        self.inner.activate(config)?;
        self.line = vec![0.0; self.actual];
        self.pos = 0;
        Ok(())
    }
    fn deactivate(&mut self) {
        self.inner.deactivate();
    }
    fn latency_samples(&self) -> u32 {
        self.reported
    }
    fn tail(&self) -> Tail {
        Tail::Samples(u64::from(self.reported))
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let status = self.inner.process(ctx, inputs, outputs);
        if !self.line.is_empty() {
            for x in outputs[0].iter_mut() {
                std::mem::swap(x, &mut self.line[self.pos]);
                self.pos = (self.pos + 1) % self.line.len();
            }
        }
        status
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.line.fill(0.0);
        self.pos = 0;
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
    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        self.inner.extension(id)
    }
}

#[test]
fn event_timing_is_latency_aware() {
    for latency in [64usize, 512] {
        let report = ModuleTestHost::new(move || Delayed::boxed(latency, latency as u32))
            .blocks(64)
            .assert_passes();
        assert!(
            report
                .first_effect
                .iter()
                .all(|p| p.first_difference == Some(p.offset + latency as u32)),
            "latency {latency}: {:?}",
            report.first_effect
        );
    }
}

#[test]
fn detects_under_reported_latency_unless_declared_delayed() {
    let lying = || ModuleTestHost::new(|| Delayed::boxed(64, 0)).blocks(16);
    let e = expect_fail(lying().check_event_timing(), "event_timing");
    assert!(
        e.detail.contains("expected the first change exactly"),
        "{e}"
    );
    // Opt-out: a declared delayed parameter only has to not change output early.
    lying()
        .allow_delayed_effect(TestGain::GAIN_DB)
        .check_event_timing()
        .unwrap();
    // Over-reported latency: the change comes before offset + latency.
    let e = expect_fail(
        ModuleTestHost::new(|| Delayed::boxed(0, 64)).check_event_timing(),
        "event_timing",
    );
    assert!(e.detail.contains("before offset"), "{e}");
}

fn expect_fail(result: Result<(), HostFailure>, check: &str) -> HostFailure {
    let err = result.expect_err("the broken module must fail");
    assert_eq!(err.check, check, "{err}");
    assert_eq!(err.module, TestGain::ID);
    err
}

#[test]
fn delegating_wrapper_without_fault_passes() {
    host(Fault::None).assert_passes();
}

#[test]
fn allocation_checker_is_active() {
    assert!(alloc_checks_active());
}

#[test]
fn detects_allocation_in_process() {
    let e = expect_fail(host(Fault::AllocInProcess).check_process(), "process");
    assert!(e.detail.contains("allocation"), "{e}");
    assert!(host(Fault::AllocInProcess).run().is_err());
}

#[test]
fn detects_allocation_in_reset() {
    let e = expect_fail(host(Fault::AllocInReset).check_reset(), "reset");
    assert!(e.detail.contains("reset"), "{e}");
}

#[test]
fn detects_non_finite_output() {
    let e = expect_fail(host(Fault::NanOutput).check_process(), "process");
    assert!(e.detail.contains("non-finite"), "{e}");
}

#[test]
fn detects_ignored_parameter_flush() {
    expect_fail(host(Fault::IgnoreFlush).check_param_flush(), "param_flush");
}

#[test]
fn detects_events_applied_before_their_offset() {
    let e = expect_fail(
        host(Fault::EventsAtBlockStart).check_event_timing(),
        "event_timing",
    );
    assert!(e.detail.contains("before offset"), "{e}");
}

#[test]
fn detects_lossy_state() {
    expect_fail(
        host(Fault::LossyState).check_state_roundtrip(),
        "state_roundtrip",
    );
}

#[test]
fn detects_nondeterminism() {
    expect_fail(
        host(Fault::Nondeterministic).check_determinism(),
        "determinism",
    );
}

#[test]
fn detects_stale_param_mirror() {
    let e = expect_fail(host(Fault::StaleParamValue).check_process(), "process");
    assert!(e.detail.contains("param_value"), "{e}");
}

#[test]
fn detects_invalid_schema() {
    expect_fail(host(Fault::BadSchema).check_schema(), "schema");
}

#[test]
fn detects_text_that_does_not_round_trip() {
    expect_fail(
        host(Fault::AmbiguousEnumLabels).check_text_roundtrip(),
        "text_roundtrip",
    );
}

#[test]
fn detects_wrong_extension_variant() {
    expect_fail(host(Fault::WrongExtension).check_extensions(), "extensions");
}

#[test]
fn failure_message_names_module_check_and_seed() {
    let e = host(Fault::BadSchema)
        .seed(0xABC)
        .check_schema()
        .unwrap_err();
    let msg = e.to_string();
    assert!(
        msg.contains(TestGain::ID) && msg.contains("schema") && msg.contains("0xabc"),
        "{msg}"
    );
}
