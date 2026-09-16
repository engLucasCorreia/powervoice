//! H-62: `HostEnd::push_event`/the sandbox proxy drop host → plugin automation events when the
//! ring is full and only count them (`AdapterHealth::events_dropped`) — this used to reach no
//! one. The rack now polls that counter every tick ([`RackHost::poll_events_dropped`]) and
//! surfaces a run of drops as `SlotInfo::automation_dropped` plus a `RackNotice::AutomationDropped`
//! on the rising/falling edge, with a user-facing notice only the first time a slot ever raises it
//! this session. In-process stand-in for the sandbox proxy (`AdapterHealth`, driven directly by
//! the test) — the real ring-fill path is `vox_sandbox_ipc`'s own `transport.rs` tests.

vox_module_api::install_test_allocator!();

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::Driver;
use vox_module_api::{
    ActivateConfig, AdapterHealth, Extension, ExtensionId, LocalizedText, MODULE_API_VERSION,
    Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, ParamFlags,
    ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail, Taper, Unit, Version,
};
use vox_rack::{AUTOMATION_DROPPED_CLEAR, RackModel, RackNotice, Registry, SlotModel};

const LEVEL: ParamId = ParamId(0);
const SR: f64 = 48_000.0;

/// A healthy adapter (never faults) whose `events_dropped()` the test drives directly.
struct Health(Arc<AtomicU64>);

impl AdapterHealth for Health {
    fn fault(&self) -> Option<String> {
        None
    }
    fn events_dropped(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

struct Adapter {
    desc: ModuleDescriptor,
    params: Vec<ParamInfo>,
    level: f64,
    dropped: Arc<AtomicU64>,
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
        for (o, i) in outputs[0].iter_mut().zip(inputs[0]) {
            *o = *i * self.level as f32;
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
            ExtensionId::AdapterHealth => Some(Extension::AdapterHealth(Arc::new(Health(
                self.dropped.clone(),
            )))),
            _ => None,
        }
    }
}

struct AdapterFactory {
    desc: ModuleDescriptor,
    dropped: Arc<AtomicU64>,
}

impl ModuleFactory for AdapterFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
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
            dropped: self.dropped.clone(),
        }))
    }
}

fn rig() -> (Driver, Arc<AtomicU64>) {
    let dropped = Arc::new(AtomicU64::new(0));
    let desc = ModuleDescriptor {
        id: "test:sandboxed".into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain("Breath Control"),
        vendor: "test".into(),
        description: LocalizedText::plain(""),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    };
    let registry = Registry::with_factories([Arc::new(AdapterFactory {
        desc,
        dropped: dropped.clone(),
    }) as Arc<dyn ModuleFactory>])
    .unwrap();
    let r: ModuleRef = "test:sandboxed@1.0.0".parse().unwrap();
    let model = RackModel {
        slots: vec![SlotModel::new(&r, false, &ModuleState::new(1))],
    };
    let len = (SR * 2.0) as usize;
    (
        Driver::with_registry(Arc::new(registry), &model, 11, len),
        dropped,
    )
}

fn automation_dropped(d: &Driver) -> bool {
    d.host.slot_info(0).unwrap().automation_dropped
}

fn edges(d: &Driver) -> Vec<(bool, bool)> {
    d.notices
        .iter()
        .filter_map(|(_, n)| match n {
            RackNotice::AutomationDropped { active, notify, .. } => Some((*active, *notify)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_dropped_event_flags_the_slot_and_notifies_once_per_session() {
    let (mut d, dropped) = rig();
    let input: Vec<f32> = (0..d.out.len())
        .map(|i| ((i % 97) as f32 / 97.0) - 0.5)
        .collect();

    // Healthy: no flag, no notice.
    d.run_until(&input, 4800);
    d.tick();
    assert!(!automation_dropped(&d));
    assert!(edges(&d).is_empty());

    // First drop: flags the slot and notifies (the first time ever this session).
    dropped.fetch_add(1, Ordering::Relaxed);
    d.tick();
    assert!(automation_dropped(&d), "flagged after the first drop");
    assert_eq!(edges(&d), vec![(true, true)], "one rising edge, notified");

    // A second drop while still active: the flag stays up, but no new notice (never spammed).
    dropped.fetch_add(1, Ordering::Relaxed);
    d.tick();
    assert!(automation_dropped(&d));
    assert_eq!(
        edges(&d),
        vec![(true, true)],
        "no repeat notice while already active"
    );

    // No further drop for the clear window: the flag clears itself (a silent falling edge).
    std::thread::sleep(AUTOMATION_DROPPED_CLEAR + Duration::from_millis(150));
    d.tick();
    assert!(!automation_dropped(&d), "cleared after the window");
    assert_eq!(
        edges(&d),
        vec![(true, true), (false, false)],
        "falling edge, never a toast"
    );

    // A new episode later this session: flags again, but never notifies twice.
    dropped.fetch_add(1, Ordering::Relaxed);
    d.tick();
    assert!(automation_dropped(&d));
    assert_eq!(
        edges(&d),
        vec![(true, true), (false, false), (true, false)],
        "second episode, no second toast"
    );
}

#[test]
fn a_healthy_slot_never_flags() {
    let (mut d, _dropped) = rig();
    let input: Vec<f32> = (0..d.out.len())
        .map(|i| ((i % 97) as f32 / 97.0) - 0.5)
        .collect();
    for _ in 0..5 {
        d.run_until(&input, d.pos + 4800);
        d.tick();
    }
    assert!(!automation_dropped(&d));
    assert!(edges(&d).is_empty());
}
