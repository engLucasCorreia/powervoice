//! Registry, rack model (sidecar slot schema), placeholders (SPEC-012 AC-13), the dual-mono
//! shim, insertion failures and the parameter mirror API.

#![allow(clippy::float_cmp, clippy::needless_range_loop)] // exact values, indexed by sample time
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::*;
use serde_json::json;
use vox_module_api::{
    ActivateConfig, ChannelLayout, LocalizedText, Module, ModuleDescriptor, ModuleError,
    ModuleState, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail,
};
use vox_modules::Gain;
use vox_rack::{
    RackError, RackHost, RackModel, RackNotice, RackOptions, Registry, RegistryError, Resolved,
    SlotModel, SlotStatus,
};

vox_module_api::install_test_allocator!();

#[test]
fn registry_holds_one_version_per_id() {
    let mut r = Registry::with_factories(test_factories()).unwrap();
    assert_eq!(
        r.register(Arc::new(vox_modules::GainFactory::new())),
        Err(RegistryError::Duplicate(Gain::ID.into()))
    );
    let ids = r.ids();
    assert!(ids.contains(&Gain::ID));
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "sorted: {ids:?}");
    assert_eq!(r.descriptors().count(), r.len());
    let m = model(vec![
        gain_slot(0.0),
        slot("com.acme.x", &[]),
        SlotModel {
            module: "not a ref".into(),
            ..gain_slot(0.0)
        },
        slot("com.acme.x", &[]),
    ]);
    assert_eq!(r.missing_ids(&m), vec!["com.acme.x@1.0.0", "not a ref"]);
}

#[test]
fn resolution_is_by_id_the_version_is_informational() {
    let r = registry();
    let mut s = gain_slot(-6.0);
    s.module = "org.powervoice.gain@0.9.3".into();
    let Resolved::Module(m) = r.resolve(&s).unwrap() else {
        panic!("expected a module");
    };
    assert_eq!(m.param_value(Gain::GAIN_DB), Some(-6.0));
}

#[test]
fn sidecar_slot_shape_and_rack_file_forms() {
    let (host, _live) = RackHost::new(
        registry(),
        rt_config(),
        RackOptions::default(),
        &model(vec![gain_slot(-6.0)]),
    )
    .unwrap();
    let v = serde_json::to_value(host.model()).unwrap();
    assert_eq!(
        v,
        json!({ "slots": [ { "module": "org.powervoice.gain@1.0.0", "bypass": false,
            "state": { "format_version": 1, "params": { "gain_db": -6.0 } } } ] })
    );
    let arr = r#"[ { "module": "org.powervoice.gain@1.0.0", "bypass": true,
        "state": { "format_version": 1, "params": { "gain_db": -3.0 } } } ]"#;
    let a = RackModel::from_json(arr).unwrap();
    let b = RackModel::from_json(&format!(r#"{{ "slots": {arr} }}"#)).unwrap();
    assert_eq!(a, b);
    assert!(a.slots[0].bypass);
    assert_eq!(RackModel::from_json(&a.to_json()).unwrap(), a);
}

/// AC-13: an unknown module becomes a placeholder: bit-exact dry, latency 0, message, and the
/// slot's JSON is written back unchanged.
#[test]
fn ac13_missing_module_is_a_verbatim_placeholder() {
    let slot_json = json!({
        "module": "com.acme.x@2.0.0",
        "bypass": false,
        "state": { "format_version": 7, "params": { "q": 1.5, "mix": 0.1 },
                   "blob": "AAEC", "future": { "a": [1, 2, 3.25] } },
        "note": "kept"
    });
    let m: RackModel = serde_json::from_value(json!({ "slots": [slot_json.clone()] })).unwrap();
    let x = white(1, 20_000);
    let mut d = Driver::new(&m, 1, x.len());
    d.run_to_end(&x);
    assert!(bit_identical(&d.out, &x));
    assert_eq!(d.host.total_latency_samples(), 0);
    assert_eq!(d.live.latency_samples(), 0);
    let info = d.host.slot_info(0).unwrap();
    assert_eq!(
        info.status,
        SlotStatus::Missing {
            message: "Missing module com.acme.x@2.0.0".into(),
            too_new: false
        }
    );
    let back = serde_json::to_value(&d.host.model().slots[0]).unwrap();
    assert_eq!(back, slot_json);
}

/// H-15 (SPEC-018 §2.6.4): a slot object that isn't even well-formed (no `module` string) never
/// invalidates the rest of the rack or the sidecar — it becomes an "Unreadable module" placeholder
/// on its own, written back verbatim, while sibling slots keep working normally.
#[test]
fn a_malformed_slot_is_an_unreadable_placeholder_and_does_not_invalidate_the_rest_of_the_rack() {
    let malformed = json!({ "bypass": true, "state": { "k": 1 }, "note": "no module field" });
    let m: RackModel = serde_json::from_value(json!({
        "slots": [
            { "module": "org.powervoice.gain@1.0.0", "bypass": false,
              "state": { "format_version": 1, "params": { "gain_db": 0.0 } } },
            malformed.clone(),
            { "module": "org.powervoice.gain@1.0.0", "bypass": false,
              "state": { "format_version": 1, "params": { "gain_db": -6.0 } } },
        ]
    }))
    .unwrap();
    assert!(m.slots[1].is_malformed());
    assert_eq!(m.slots[1].module, "");

    let (host, _live) = RackHost::new(registry(), rt_config(), RackOptions::default(), &m).unwrap();
    assert_eq!(
        host.slot_info(1).unwrap().status,
        SlotStatus::Missing {
            message: "Unreadable module".into(),
            too_new: false
        }
    );
    // Siblings resolved normally, untouched by the malformed slot between them.
    assert_eq!(host.slot_info(0).unwrap().status, SlotStatus::Active);
    assert_eq!(host.slot_info(2).unwrap().status, SlotStatus::Active);
    assert_eq!(
        serde_json::to_value(&host.model().slots[1]).unwrap(),
        malformed
    );
    // Also excluded from the "missing modules" install prompt — there's no id to install.
    assert!(!registry().missing_ids(&m).contains(&String::new()));
}

#[test]
fn too_new_state_is_a_placeholder_too() {
    let mut s = gain_slot(-6.0);
    s.state["format_version"] = json!(99);
    let original = serde_json::to_value(&s).unwrap();
    let (host, _live) = RackHost::new(
        registry(),
        rt_config(),
        RackOptions::default(),
        &model(vec![s]),
    )
    .unwrap();
    assert_eq!(
        host.slot_info(0).unwrap().status,
        SlotStatus::Missing {
            message: "org.powervoice.gain requires a newer version".into(),
            too_new: true
        }
    );
    assert_eq!(
        serde_json::to_value(&host.model().slots[0]).unwrap(),
        original
    );
}

struct Failing;

impl Module for Failing {
    fn descriptor(&self) -> &ModuleDescriptor {
        static D: std::sync::OnceLock<ModuleDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| descriptor("org.powervoice.test-failing", "Failing"))
    }
    fn params(&self) -> &[ParamInfo] {
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
        _: &mut ProcessContext<'_>,
        _: &[&[f32]],
        _: &mut [&mut [f32]],
    ) -> ProcessStatus {
        ProcessStatus::Continue
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

/// Used by `restart_retries_a_slot_that_failed_to_start_at_load` only.
static FLAKY_FAILS: AtomicBool = AtomicBool::new(true);
static FLAKY_DEACTIVATIONS: AtomicUsize = AtomicUsize::new(0);

/// ×0.5, but `activate` fails while [`FLAKY_FAILS`] is set (a licence that shows up later).
struct Flaky;

impl Flaky {
    const ID: &'static str = "org.powervoice.test-flaky";
}

impl Module for Flaky {
    fn descriptor(&self) -> &ModuleDescriptor {
        static D: std::sync::OnceLock<ModuleDescriptor> = std::sync::OnceLock::new();
        D.get_or_init(|| descriptor(Flaky::ID, "Flaky"))
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn activate(&mut self, _c: &ActivateConfig) -> Result<(), ModuleError> {
        if FLAKY_FAILS.load(Ordering::Relaxed) {
            return Err(ModuleError::Resource("no licence".into()));
        }
        Ok(())
    }
    fn deactivate(&mut self) {
        FLAKY_DEACTIVATIONS.fetch_add(1, Ordering::Relaxed);
    }
    fn latency_samples(&self) -> u32 {
        0
    }
    fn tail(&self) -> Tail {
        Tail::Samples(0)
    }
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        for (o, i) in outputs[0].iter_mut().zip(inputs[0]) {
            *o = 0.5 * i;
        }
        ProcessStatus::Continue
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

/// Stereo-only: out L = 0.5·L + 0.25·R, out R = garbage. Through the shim: 0.75·x.
struct StereoOnly {
    desc: ModuleDescriptor,
}

impl Module for StereoOnly {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn supported_layouts(&self) -> &[ChannelLayout] {
        &[ChannelLayout::STEREO]
    }
    fn activate(&mut self, c: &ActivateConfig) -> Result<(), ModuleError> {
        if c.layout == ChannelLayout::STEREO {
            Ok(())
        } else {
            Err(ModuleError::Unsupported("stereo only".into()))
        }
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
        _ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let (l, r) = (inputs[0], inputs[1]);
        for i in 0..l.len() {
            outputs[0][i] = 0.5 * l[i] + 0.25 * r[i];
            outputs[1][i] = 7.0;
        }
        ProcessStatus::Continue
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

fn odd_registry() -> Arc<Registry> {
    let mut r = Registry::with_factories(test_factories()).unwrap();
    r.register(factory(|| Box::new(Failing))).unwrap();
    r.register(factory(|| {
        Box::new(StereoOnly {
            desc: descriptor("org.powervoice.test-stereo", "Stereo Only"),
        })
    }))
    .unwrap();
    r.register(factory(|| {
        let mut p = Probe::new(0);
        p.layouts = vec![ChannelLayout {
            inputs: 3,
            outputs: 3,
        }];
        Box::new(p)
    }))
    .unwrap();
    Arc::new(r)
}

#[test]
fn insertion_failures_and_the_dual_mono_shim() {
    let x = white(2, 30_000);
    let mut d = Driver::with_registry(odd_registry(), &model(vec![gain_slot(0.0)]), 3, x.len());
    d.run_until(&x, 1000);
    let e = d
        .host
        .insert(0, slot("org.powervoice.test-failing", &[]))
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Couldn't start Failing: resource error: no licence"
    );
    let e = d
        .host
        .insert(0, slot("org.powervoice.test-probe", &[]))
        .unwrap_err();
    assert!(matches!(e, RackError::UnsupportedLayout { .. }));
    assert_eq!(e.to_string(), "Probe has an unsupported channel layout");
    assert_eq!(d.host.len(), 1);
    d.host
        .insert(1, slot("org.powervoice.test-stereo", &[]))
        .unwrap();
    d.run_to_end(&x);
    for n in 5000..x.len() {
        assert!((d.out[n] - 0.75 * x[n]).abs() <= 1e-6, "sample {n}");
    }
    for _ in d.host.len()..vox_rack::MAX_SLOTS {
        d.host.insert_module(0, Gain::ID).unwrap();
    }
    assert!(matches!(
        d.host.insert_module(0, Gain::ID),
        Err(RackError::TooManySlots)
    ));
    assert!(matches!(
        d.host.insert_module(0, "com.acme.nope"),
        Err(RackError::UnknownModule(_))
    ));
}

#[test]
fn parameter_mirror_normalized_and_text() {
    let (mut host, _live) = RackHost::new(
        registry(),
        rt_config(),
        RackOptions::default(),
        &model(vec![gain_slot(0.0), slot("com.acme.x", &[])]),
    )
    .unwrap();
    assert_eq!(
        host.set_param_normalized(0, Gain::GAIN_DB, 0.75).unwrap(),
        3.0
    );
    assert_eq!(
        host.set_param_text(0, Gain::GAIN_DB, "−6 dB").unwrap(),
        -6.0
    );
    assert_eq!(host.set_param(0, Gain::GAIN_DB, 100.0).unwrap(), 24.0);
    let n = host.take_notices();
    assert_eq!(n.len(), 1, "coalesced per (slot, id), last value");
    assert!(matches!(&n[0], RackNotice::ParamChanged { value, .. } if *value == 24.0));
    assert!(matches!(
        host.set_param_text(0, Gain::GAIN_DB, "loud"),
        Err(RackError::InvalidText(_))
    ));
    assert!(host.take_notices().is_empty(), "invalid text sends nothing");
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(24.0));
    assert!(matches!(
        host.set_param(1, Gain::GAIN_DB, 0.0),
        Err(RackError::NotLoaded { index: 1 })
    ));
    assert!(matches!(
        host.set_param(0, ParamId(9), 0.0),
        Err(RackError::UnknownParam { .. })
    ));
    // The model carries the mirror.
    assert_eq!(
        host.model().slots[0].state["params"]["gain_db"],
        json!(24.0)
    );
    let _ = LocalizedText::plain("unused");
}

#[test]
fn load_time_failures_are_failed_slots_kept_verbatim() {
    let s = slot("org.powervoice.test-failing", &[("x", 1.0)]);
    let original = serde_json::to_value(&s).unwrap();
    let x = white(9, 10_000);
    let mut d = Driver::with_registry(odd_registry(), &model(vec![s]), 4, x.len());
    d.run_to_end(&x);
    assert!(bit_identical(&d.out, &x));
    assert_eq!(
        d.host.slot_info(0).unwrap().status,
        SlotStatus::Failed {
            message: "Couldn't start Failing: resource error: no licence".into()
        }
    );
    assert_eq!(
        serde_json::to_value(&d.host.model().slots[0]).unwrap(),
        original
    );
}

#[test]
fn restart_retries_a_slot_that_failed_to_start_at_load() {
    let mut r = Registry::with_factories(test_factories()).unwrap();
    r.register(factory(|| Box::new(Flaky))).unwrap();
    let x = white(12, 48_000);
    let m = model(vec![gain_slot(0.0), slot(Flaky::ID, &[])]);
    let mut d = Driver::with_registry(Arc::new(r), &m, 7, x.len());
    d.run_until(&x, 10_000);
    let failed = SlotStatus::Failed {
        message: "Couldn't start Flaky: resource error: no licence".into(),
    };
    assert_eq!(d.host.slot_info(1).unwrap().status, failed);
    // Still failing: the error comes back and the slot stays as it was.
    let e = d.host.restart(1).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Couldn't start Flaky: resource error: no licence"
    );
    assert_eq!(d.host.slot_info(1).unwrap().status, failed);
    FLAKY_FAILS.store(false, Ordering::Relaxed);
    d.host.restart(1).unwrap();
    d.run_until(&x, 20_000);
    assert_eq!(d.host.slot_info(1).unwrap().status, SlotStatus::Active);
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRestarted { index: 1, .. }))
    );
    assert!(bit_identical(&d.out[..10_000], &x[..10_000]));
    // Fades in from the dry path over 15 ms.
    for n in 10_000 + XF..20_000 {
        assert!((d.out[n] - 0.5 * x[n]).abs() <= 1e-6, "sample {n}");
    }
    // A further replacement is not held back: the first instance gets retired.
    d.host.restart(1).unwrap();
    d.run_to_end(&x);
    for _ in 0..3 {
        d.tick();
    }
    assert_eq!(FLAKY_DEACTIVATIONS.load(Ordering::Relaxed), 1);
    assert_eq!(d.host.swaps_in_flight(), 0);
    assert_eq!(d.live.chain().len(), 2);
    assert_eq!(
        d.host.model().slots[1].module,
        "org.powervoice.test-flaky@1.0.0"
    );
    // A missing module still cannot be restarted.
    let (mut host, _live) = RackHost::new(
        registry(),
        rt_config(),
        RackOptions::default(),
        &model(vec![slot("com.acme.x", &[])]),
    )
    .unwrap();
    assert!(matches!(
        host.restart(0),
        Err(RackError::NotLoaded { index: 0 })
    ));
}

#[test]
fn replace_state_swaps_in_a_new_instance() {
    let x = white(10, 96_000);
    let mut d = Driver::new(&model(vec![gain_slot(0.0)]), 5, x.len());
    d.run_until(&x, 20_000);
    d.host
        .replace_state(0, Gain::state_with_gain_db(-12.0))
        .unwrap();
    d.run_to_end(&x);
    assert_eq!(d.host.param_value(0, Gain::GAIN_DB), Some(-12.0));
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::ParamChanged { value, .. } if *value == -12.0))
    );
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRestarted { index: 0, .. }))
    );
    let g = 10f32.powf(-12.0 / 20.0);
    for n in 20_000 + 2 * XF..x.len() {
        assert!((d.out[n] - x[n] * g).abs() <= 1e-6, "sample {n}");
    }
    assert_eq!(
        d.host.model().slots[0].state["params"]["gain_db"],
        json!(-12.0)
    );
}
