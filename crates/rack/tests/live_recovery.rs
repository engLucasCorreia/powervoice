//! H-40: a Missing/too-new placeholder slot recovers live once its module is registered again —
//! install, a quick/full rescan, unblock, re-enable (T-804/H-29's `Registry::upsert`, now polled
//! by [`RackHost::tick`] through [`Registry::generation`]). No reopen needed. Uses [`Driver`]
//! throughout, so every `LiveRack::process` in these tests already runs under the `no_alloc`
//! allocation checker (`crates/rack/tests/common/mod.rs`) — proving the swap itself never
//! allocates on the audio thread.

#![allow(clippy::float_cmp)]
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use vox_module_api::{
    ActivateConfig, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleState, ParamId,
    StateError, Tail,
};
use vox_modules::{Gain, GainFactory};
use vox_rack::{RackNotice, Registry, SlotStatus};

vox_module_api::install_test_allocator!();

/// A module that always fails to *activate* (not create) — like `model.rs`'s private `Failing`,
/// repeated here since test binaries don't share `#[cfg(test)]` items across files.
struct AlwaysFailsToActivate;

impl Module for AlwaysFailsToActivate {
    fn descriptor(&self) -> &ModuleDescriptor {
        static D: std::sync::OnceLock<ModuleDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| descriptor("org.powervoice.test-always-fails", "AlwaysFails"))
    }
    fn params(&self) -> &[vox_module_api::ParamInfo] {
        &[]
    }
    fn activate(&mut self, _c: &ActivateConfig) -> Result<(), ModuleError> {
        Err(ModuleError::Resource("no licence".into()))
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
        _: &mut vox_module_api::ProcessContext<'_>,
        _: &[&[f32]],
        _: &mut [&mut [f32]],
    ) -> vox_module_api::ProcessStatus {
        vox_module_api::ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, _: ParamId) -> Option<f64> {
        None
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}

/// A `loads_async` factory around `Gain` (T-803's out-of-process shape, without a real sandbox):
/// `fail` makes `create()` error instead, exercising the async re-resolve failure path.
struct AsyncGain {
    descriptor: ModuleDescriptor,
    fail: AtomicBool,
}

impl ModuleFactory for AsyncGain {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(ModuleError::External("the plugin refused to load".into()));
        }
        Ok(Box::new(Gain::new()))
    }
    fn loads_async(&self) -> bool {
        true
    }
}

fn async_gain(fail: bool) -> Arc<AsyncGain> {
    Arc::new(AsyncGain {
        descriptor: Gain::new().descriptor().clone(),
        fail: AtomicBool::new(fail),
    })
}

/// Install/rescan (a synchronous factory, e.g. a built-in module or a quick VST3 index): the
/// Missing slot becomes Running with the same state, a `SlotRecovered` notice is emitted, and the
/// tail of the audio reflects the recovered module — with every intervening `LiveRack::process`
/// (the fade-out of the dry placeholder into the newly resolved instance included) running under
/// `no_alloc`.
#[test]
fn installing_the_missing_module_recovers_the_slot_live() {
    let registry = Arc::new(Registry::new());
    let m = model(vec![gain_slot(-12.5)]);
    let x = white(1, 24_000);
    let mut d = Driver::with_registry(registry.clone(), &m, 7, x.len());

    // Not registered yet: a verbatim Missing placeholder, dry (passthrough).
    d.run_until(&x, 6_000);
    assert!(
        matches!(
            d.host.slot_info(0).unwrap().status,
            SlotStatus::Missing { .. }
        ),
        "{:?}",
        d.host.slot_info(0).unwrap().status
    );
    assert!(bit_identical(&d.out[..6_000], &x[..6_000]));

    // "Install"/"rescan" hot-adds the module into the SAME registry the live rack already
    // shares (T-804/H-29's `Registry::upsert`) — no reopen.
    registry.register(Arc::new(GainFactory::new())).unwrap();

    d.run_to_end(&x);

    assert_eq!(d.host.slot_info(0).unwrap().status, SlotStatus::Active);
    assert_eq!(
        d.host.param_value(0, Gain::GAIN_DB),
        Some(-12.5),
        "the kept state survived the recovery"
    );
    assert!(
        d.notices.iter().any(|(_, n)| matches!(
            n,
            RackNotice::SlotRecovered { index: 0, name, .. } if name == "Gain"
        )),
        "no SlotRecovered notice: {:?}",
        d.notices
    );
    assert!(
        !d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRestarted { .. })),
        "a live recovery is not a manual restart: {:?}",
        d.notices
    );

    // The gain is audible at the tail, once the swap's crossfade has finished.
    let want = Gain::db_to_gain(-12.5) as f32;
    let tail = &d.out[d.out.len() - 200..];
    let expect: Vec<f32> = x[x.len() - 200..].iter().map(|&s| s * want).collect();
    assert!(
        max_abs_diff(tail, &expect) < 1e-4,
        "tail doesn't reflect the recovered gain"
    );
}

/// The same recovery for a module whose factory `loads_async` (T-803's out-of-process shape):
/// the slot passes through "Loading" (dry) and resolves once the background instantiation
/// finishes, still without a manual restart.
#[test]
fn an_async_plugin_recovers_through_the_loading_state() {
    let registry = Arc::new(Registry::new());
    let m = model(vec![gain_slot(3.0)]);
    let x = white(2, 24_000);
    let mut d = Driver::with_registry(registry.clone(), &m, 8, x.len());

    d.run_until(&x, 4_000);
    assert!(matches!(
        d.host.slot_info(0).unwrap().status,
        SlotStatus::Missing { .. }
    ));

    registry.upsert(async_gain(false));

    d.run_to_end(&x);
    // The background instantiation (T-803) may still be in flight when the input runs out —
    // give it a moment, ticking (never `process`ing, nothing left to feed) until it lands.
    let t0 = std::time::Instant::now();
    while d.host.is_loading() {
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(5),
            "load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        d.tick();
    }

    assert_eq!(d.host.slot_info(0).unwrap().status, SlotStatus::Active);
    assert_eq!(d.host.param_value(0, Gain::GAIN_DB), Some(3.0));
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRecovered { index: 0, .. })),
        "{:?}",
        d.notices
    );
}

/// A re-resolve that still can't produce a running module (the newly registered factory fails to
/// activate) leaves the slot a placeholder with its state and blob untouched, and shows the
/// failure — it does not silently drop the slot's data.
#[test]
fn a_failing_reresolve_keeps_the_placeholder_and_its_blob() {
    let registry = Arc::new(Registry::new());
    let mut state = ModuleState::new(1);
    state.blob = Some(vec![1, 2, 3, 4]);
    let s = vox_rack::SlotModel::new(
        &vox_module_api::ModuleRef {
            id: "org.powervoice.test-always-fails".into(),
            version: vox_module_api::Version::new(1, 0, 0),
        },
        false,
        &state,
    );
    let m = model(vec![s.clone()]);
    let x = white(3, 12_000);
    let mut d = Driver::with_registry(registry.clone(), &m, 9, x.len());
    d.run_until(&x, 3_000);

    registry
        .register(common::factory(|| Box::new(AlwaysFailsToActivate)))
        .unwrap();

    d.run_to_end(&x);

    let SlotStatus::Failed { message } = d.host.slot_info(0).unwrap().status else {
        panic!("expected a failed placeholder: {:?}", d.host.slot_info(0));
    };
    assert!(message.contains("no licence"), "{message}");
    assert_eq!(
        d.host.model().slots[0],
        s,
        "the slot's state and blob are kept verbatim"
    );
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotFailed { index: 0, .. })),
        "{:?}",
        d.notices
    );
    assert!(
        !d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRecovered { .. })),
        "{:?}",
        d.notices
    );
}

/// A synchronous factory standing in for a concurrent install (H-125): the first time the rack
/// instantiates it, it registers `GainFactory` into the registry the rack shares — so the
/// registry changes *while the rack is still resolving its other slots*, the window a real
/// background scan (T-804) can hit when it hot-adds a plugin during a document open or an
/// output (re)open. Deterministic: no threads, the register happens at a fixed point.
struct InstallsGainOnCreate {
    descriptor: ModuleDescriptor,
    registry: std::sync::Mutex<std::sync::Weak<Registry>>,
    installed: AtomicBool,
}

impl ModuleFactory for InstallsGainOnCreate {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        if !self.installed.swap(true, Ordering::SeqCst)
            && let Some(registry) = self.registry.lock().unwrap().upgrade()
        {
            registry.register(Arc::new(GainFactory::new())).unwrap();
        }
        Ok(Box::new(Sentinel::new()))
    }
}

/// A registry holding only [`InstallsGainOnCreate`] (under `Sentinel`'s id).
fn registry_that_installs_gain_mid_resolve() -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    registry
        .register(Arc::new(InstallsGainOnCreate {
            descriptor: Sentinel::new().descriptor().clone(),
            registry: std::sync::Mutex::new(Arc::downgrade(&registry)),
            installed: AtomicBool::new(false),
        }))
        .unwrap();
    registry
}

fn assert_recovered_after_ticks(d: &mut Driver, x: &[f32]) {
    d.run_to_end(x);
    assert_eq!(
        d.host.slot_info(0).unwrap().status,
        SlotStatus::Active,
        "a module registered while the rack was resolving must still be picked up"
    );
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRecovered { index: 0, .. })),
        "{:?}",
        d.notices
    );
}

/// H-125: a registry change that lands *during* `RackHost::new` (after slot 0 resolved as
/// Missing, before the rack recorded the registry generation) must not be swallowed — the rack
/// has to record the generation it resolved against, not the one after, or the slot stays
/// Missing until the next unrelated registry change.
#[test]
fn a_module_registered_while_the_rack_is_being_built_still_recovers() {
    let registry = registry_that_installs_gain_mid_resolve();
    let m = model(vec![gain_slot(-6.0), slot(Sentinel::ID, &[])]);
    let x = white(4, 12_000);
    let mut d = Driver::with_registry(registry.clone(), &m, 10, x.len());
    assert!(
        registry.get(Gain::ID).is_some(),
        "setup: resolving slot 1 installed Gain"
    );
    assert!(
        matches!(
            d.host.slot_info(0).unwrap().status,
            SlotStatus::Missing { .. }
        ),
        "setup: slot 0 resolved before the install"
    );
    assert_recovered_after_ticks(&mut d, &x);
}

/// H-125: the same window in `RackHost::load_model` (document open into a live rack).
#[test]
fn a_module_registered_while_a_model_is_being_loaded_still_recovers() {
    let registry = registry_that_installs_gain_mid_resolve();
    let x = white(5, 12_000);
    let mut d = Driver::with_registry(registry.clone(), &model(Vec::new()), 11, x.len());
    d.run_until(&x, 2_000);
    d.host
        .load_model(&model(vec![gain_slot(-6.0), slot(Sentinel::ID, &[])]))
        .unwrap();
    assert!(
        registry.get(Gain::ID).is_some(),
        "setup: resolving slot 1 installed Gain"
    );
    assert!(
        matches!(
            d.host.slot_info(0).unwrap().status,
            SlotStatus::Missing { .. }
        ),
        "setup: slot 0 resolved before the install"
    );
    assert_recovered_after_ticks(&mut d, &x);
}
