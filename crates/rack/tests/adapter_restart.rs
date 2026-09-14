//! T-802 restart policy (rack side): an out-of-process module — one answering the
//! `AdapterHealth` extension — that fails with `ProcessStatus::Error` is bypassed, shown as
//! "Restarting", and restarted once with its last committed state; a second failure leaves it
//! "Failed" until a manual Restart (which resets the budget). Invalid audio and in-process
//! modules never restart automatically. In-process stand-in for the sandbox proxy (the real
//! proxy is exercised by `powervoice-sandbox`'s process tests).

vox_module_api::install_test_allocator!();

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Driver, SR};
use vox_module_api::{
    ActivateConfig, AdapterHealth, Extension, ExtensionId, LocalizedText, MODULE_API_VERSION,
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, ParamFlags,
    ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail, Taper, Unit, Version,
};
use vox_rack::{AUTO_RESTART_DELAY, RackModel, RackNotice, Registry, SlotModel, SlotStatus};

const LEVEL: ParamId = ParamId(0);

/// Which instance generation fails (0 = none), and whether it fails with invalid audio.
#[derive(Default)]
struct Switch {
    fail_generation: AtomicU64,
    nan: std::sync::atomic::AtomicBool,
    created: AtomicU64,
}

struct Health(Arc<Mutex<Option<&'static str>>>);

impl AdapterHealth for Health {
    fn fault(&self) -> Option<String> {
        self.0.lock().unwrap().map(str::to_owned)
    }
}

struct Adapter {
    desc: ModuleDescriptor,
    params: Vec<ParamInfo>,
    level: f64,
    generation: u64,
    switch: Arc<Switch>,
    fault: Arc<Mutex<Option<&'static str>>>,
    with_health: bool,
}

impl Module for Adapter {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, _: &ActivateConfig) -> Result<(), ModuleError> {
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
        for e in ctx.events {
            if e.id == LEVEL {
                self.level = e.value;
            }
        }
        let failing = self.switch.fail_generation.load(Ordering::Relaxed) == self.generation;
        let nan = failing && self.switch.nan.load(Ordering::Relaxed);
        for (o, i) in outputs[0].iter_mut().zip(inputs[0]) {
            *o = if nan {
                f32::NAN
            } else {
                *i * self.level as f32
            };
        }
        if failing && !nan {
            // The proxy records the fault before returning Error (no lock in a real proxy:
            // this stand-in's mutex is uncontended and allocation-free).
            if let Ok(mut f) = self.fault.try_lock()
                && f.is_none()
            {
                *f = Some("crashed");
            }
            return ProcessStatus::Error;
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == LEVEL).then_some(self.level)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        let mut s = ModuleState::new(1);
        s.params.insert("level".into(), self.level);
        Ok(s)
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.level = state.params.get("level").copied().unwrap_or(1.0);
        Ok(())
    }
    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::AdapterHealth if self.with_health => {
                let fault = self.fault.clone();
                Some(Extension::AdapterHealth(Arc::new(Health(fault))))
            }
            _ => None,
        }
    }
}

struct AdapterFactory {
    desc: ModuleDescriptor,
    switch: Arc<Switch>,
    with_health: bool,
}

fn descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        id: id.into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain("Faulty"),
        vendor: "test".into(),
        description: LocalizedText::plain(""),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    }
}

impl ModuleFactory for AdapterFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        let generation = self.switch.created.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(Box::new(Adapter {
            desc: self.desc.clone(),
            params: vec![ParamInfo {
                id: LEVEL,
                key: "level".into(),
                name: LocalizedText::plain("Level"),
                group: None,
                unit: Unit::None,
                min: 0.0,
                max: 2.0,
                default: 1.0,
                taper: Taper::Linear,
                step: None,
                enum_labels: Vec::new(),
                decimals: 2,
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE,
            }],
            level: 1.0,
            generation,
            switch: self.switch.clone(),
            fault: Arc::new(Mutex::new(None)),
            with_health: self.with_health,
        }))
    }
}

fn rig(with_health: bool) -> (Driver, Arc<Switch>, Vec<f32>) {
    let switch = Arc::new(Switch::default());
    let registry = Registry::with_factories([Arc::new(AdapterFactory {
        desc: descriptor("test:faulty"),
        switch: switch.clone(),
        with_health,
    }) as Arc<dyn ModuleFactory>])
    .unwrap();
    let mut state = ModuleState::new(1);
    state.params.insert("level".into(), 0.5);
    let r: ModuleRef = "test:faulty@1.0.0".parse().unwrap();
    let model = RackModel {
        slots: vec![SlotModel::new(&r, false, &state)],
    };
    let len = (SR * 4.0) as usize;
    let input: Vec<f32> = (0..len).map(|i| ((i % 97) as f32 / 97.0) - 0.5).collect();
    (
        Driver::with_registry(Arc::new(registry), &model, 7, len),
        switch,
        input,
    )
}

fn status(d: &Driver) -> SlotStatus {
    d.host.slot_info(0).unwrap().status
}

fn failed_notices(d: &Driver) -> Vec<String> {
    d.notices
        .iter()
        .filter_map(|(_, n)| match n {
            RackNotice::SlotFailed { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

/// Runs audio for `samples` more samples, then waits out the restart delay and ticks.
fn advance(d: &mut Driver, input: &[f32], samples: usize) {
    let until = (d.pos + samples).min(input.len());
    d.run_until(input, until);
    d.tick();
}

fn wait_restart_delay(d: &mut Driver) {
    std::thread::sleep(AUTO_RESTART_DELAY + Duration::from_millis(50));
    d.tick();
}

#[test]
fn an_adapter_failure_restarts_once_with_state_then_stays_failed() {
    let (mut d, switch, input) = rig(true);
    assert!(d.host.slot_info(0).unwrap().sandboxed);
    advance(&mut d, &input, 4800);
    assert_eq!(status(&d), SlotStatus::Active);

    // First failure → bypassed, "Restarting", restarted with the committed state.
    switch.fail_generation.store(1, Ordering::Relaxed);
    advance(&mut d, &input, 4800);
    match status(&d) {
        SlotStatus::Restarting { message } => {
            assert_eq!(message, "Faulty crashed and was bypassed");
        }
        other => panic!("expected Restarting, got {other:?}"),
    }
    wait_restart_delay(&mut d);
    advance(&mut d, &input, 4800);
    assert_eq!(status(&d), SlotStatus::Active);
    assert_eq!(switch.created.load(Ordering::Relaxed), 2, "one restart");
    assert_eq!(d.host.param_value(0, LEVEL), Some(0.5), "state restored");
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRestarted { index: 0, .. }))
    );
    // The restarted instance really runs with the state: output = 0.5 · input once faded in.
    let p = d.pos;
    advance(&mut d, &input, 4800);
    let from = p + 2400;
    for (i, (o, x)) in d.out[from..d.pos].iter().zip(&input[from..]).enumerate() {
        assert_eq!(o.to_bits(), (x * 0.5).to_bits(), "sample {}", from + i);
    }

    // Second failure → "Failed", no further restart.
    switch.fail_generation.store(2, Ordering::Relaxed);
    advance(&mut d, &input, 4800);
    wait_restart_delay(&mut d);
    advance(&mut d, &input, 4800);
    assert!(
        matches!(status(&d), SlotStatus::Failed { .. }),
        "{:?}",
        status(&d)
    );
    assert_eq!(
        switch.created.load(Ordering::Relaxed),
        2,
        "no second restart"
    );
    assert_eq!(failed_notices(&d).len(), 2);
    // Bypassed: exactly the dry input (latency 0).
    let p = d.pos;
    advance(&mut d, &input, 4800);
    for (i, (o, x)) in d.out[p..d.pos].iter().zip(&input[p..]).enumerate() {
        assert_eq!(o.to_bits(), x.to_bits(), "dry sample {}", p + i);
    }

    // A manual Restart (Retry) resets the budget: the next failure restarts again.
    d.host.restart(0).unwrap();
    advance(&mut d, &input, 4800);
    assert_eq!(status(&d), SlotStatus::Active);
    switch.fail_generation.store(3, Ordering::Relaxed);
    advance(&mut d, &input, 4800);
    assert!(
        matches!(status(&d), SlotStatus::Restarting { .. }),
        "{:?}",
        status(&d)
    );
    wait_restart_delay(&mut d);
    assert_eq!(switch.created.load(Ordering::Relaxed), 4);
    assert_eq!(status(&d), SlotStatus::Active);
}

#[test]
fn invalid_audio_and_in_process_failures_never_restart() {
    // Invalid audio from an adapter: Failed at once.
    let (mut d, switch, input) = rig(true);
    switch.nan.store(true, Ordering::Relaxed);
    switch.fail_generation.store(1, Ordering::Relaxed);
    advance(&mut d, &input, 4800);
    wait_restart_delay(&mut d);
    assert!(
        matches!(status(&d), SlotStatus::Failed { .. }),
        "{:?}",
        status(&d)
    );
    assert_eq!(switch.created.load(Ordering::Relaxed), 1);

    // An in-process module (no AdapterHealth) returning Error: Failed at once.
    let (mut d, switch, input) = rig(false);
    assert!(!d.host.slot_info(0).unwrap().sandboxed);
    switch.fail_generation.store(1, Ordering::Relaxed);
    advance(&mut d, &input, 4800);
    wait_restart_delay(&mut d);
    match status(&d) {
        SlotStatus::Failed { message } => {
            assert_eq!(message, "Faulty stopped working and was bypassed");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(switch.created.load(Ordering::Relaxed), 1);
}
