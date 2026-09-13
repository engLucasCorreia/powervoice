//! S3-06 (SPEC-014 §2.3, §2.9): the `RackHost` building blocks Capture Noise Print is built on —
//! target-slot resolution, the upstream-slots model for the offline pre-render, the noise-profile
//! status the panel reads, and the live replacement that installs a captured print.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::*;
use vox_module_api::{ModuleRef, ModuleState};
use vox_modules::{Gain, NoiseReduction, NoiseReductionFactory};
use vox_rack::{
    NoiseProfileStatus, RackHost, RackModel, RackNotice, RackOptions, Registry, SlotModel,
};

vox_module_api::install_test_allocator!();

/// A registry with the shared test factories (Gain, TestRestart, ...) plus the real Noise
/// Reduction module.
fn nr_registry() -> Arc<Registry> {
    let mut factories = test_factories();
    factories.push(Arc::new(NoiseReductionFactory::new()));
    Arc::new(Registry::with_factories(factories).unwrap())
}

fn nr_slot(state: ModuleState) -> SlotModel {
    SlotModel::new(
        &ModuleRef {
            id: NoiseReduction::ID.into(),
            version: NoiseReduction::VERSION,
        },
        false,
        &state,
    )
}

fn default_nr_slot() -> SlotModel {
    nr_slot(NoiseReduction::state_with_blob(None))
}

fn host_with(registry: Arc<Registry>, model: &RackModel) -> RackHost {
    let (host, _live) =
        RackHost::new(registry, rt_config(), RackOptions::default(), model).unwrap();
    host
}

/// A print of `len` samples of white noise at `SR`, captured directly (no rack in the way).
fn print_of(seed: u64, len: usize) -> Vec<u8> {
    let x = white(seed, len);
    vox_dsp::nr::capture_profile(&x, SR, &AtomicBool::new(false)).unwrap()
}

// --- Target slot resolution (SPEC-014 §2.3 "target slot") -----------------------------------

#[test]
fn find_noise_profile_slot_prefers_the_hint_then_the_first_match_then_none() {
    let registry = nr_registry();
    let three = model(vec![gain_slot(0.0), default_nr_slot(), default_nr_slot()]);
    let host = host_with(registry.clone(), &three);

    // No NR slot at all: nothing to find.
    let empty = host_with(registry.clone(), &model(vec![gain_slot(0.0)]));
    assert_eq!(empty.find_noise_profile_slot(None), None);
    assert_eq!(empty.find_noise_profile_slot(Some(0)), None);

    // A valid hint wins even though it isn't the first NR slot.
    assert_eq!(host.find_noise_profile_slot(Some(2)), Some(2));
    // An invalid hint (not an NR slot, or out of range) falls back to the first NR slot.
    assert_eq!(host.find_noise_profile_slot(Some(0)), Some(1));
    assert_eq!(host.find_noise_profile_slot(Some(99)), Some(1));
    assert_eq!(host.find_noise_profile_slot(None), Some(1));
}

#[test]
fn resolve_or_insert_inserts_a_default_slot_at_the_top_only_when_none_exists() {
    let registry = nr_registry();
    let mut host = host_with(registry, &model(vec![gain_slot(-6.0)]));

    let (index, inserted) = host
        .resolve_or_insert_noise_profile_slot(None, NoiseReduction::ID)
        .unwrap();
    assert_eq!((index, inserted), (0, true));
    assert_eq!(host.len(), 2);
    assert!(
        host.slot_info(0)
            .unwrap()
            .module
            .starts_with(NoiseReduction::ID)
    );
    // Gain, which was slot 0, is now slot 1.
    assert!(host.slot_info(1).unwrap().module.starts_with(Gain::ID));

    // A second call finds the slot just inserted rather than inserting another.
    let (index, inserted) = host
        .resolve_or_insert_noise_profile_slot(None, NoiseReduction::ID)
        .unwrap();
    assert_eq!((index, inserted), (0, false));
    assert_eq!(host.len(), 2);
}

// --- Upstream model (offline pre-render source, SPEC-014 §2.3) ------------------------------

#[test]
fn upstream_model_is_the_committed_state_of_earlier_slots_omitting_placeholders() {
    let registry = nr_registry();
    let model = model(vec![
        gain_slot(-6.0),
        slot("com.acme.unknown-effect", &[]), // becomes a placeholder: omitted
        default_nr_slot(),
    ]);
    let host = host_with(registry, &model);
    assert!(matches!(
        host.slot_info(1).unwrap().status,
        vox_rack::SlotStatus::Missing { .. }
    ));

    let upstream = host.upstream_model(2);
    assert_eq!(upstream.slots.len(), 1, "the placeholder is omitted");
    assert_eq!(
        upstream.slots[0].module,
        format!("{}@{}", Gain::ID, Gain::VERSION)
    );
    assert_eq!(
        upstream.slots[0].state["params"]["gain_db"],
        serde_json::json!(-6.0)
    );

    // Nothing before the first slot.
    assert!(host.upstream_model(0).slots.is_empty());
}

// --- Noise-profile status (SPEC-014 §2.5, §2.8) ----------------------------------------------

#[test]
fn noise_profile_status_reflects_the_blob_and_only_exists_for_modules_with_the_extension() {
    let registry = nr_registry();
    let model = model(vec![gain_slot(0.0), default_nr_slot()]);
    let host = host_with(registry, &model);

    // Gain has no `NoiseProfile` extension at all.
    assert_eq!(host.slot_info(0).unwrap().noise_profile, None);
    assert!(host.noise_profile_extension(0).is_none());

    // NR with no blob: `None` status (not to be confused with the outer `Option`).
    assert_eq!(
        host.slot_info(1).unwrap().noise_profile,
        Some(NoiseProfileStatus::None)
    );
    assert!(host.noise_profile_extension(1).is_some());
}

#[test]
fn noise_profile_status_distinguishes_loaded_unreadable_and_too_new() {
    let registry = nr_registry();
    let good = print_of(1, 24_000);
    let mut too_new = good.clone();
    too_new[4..6].copy_from_slice(&2u16.to_le_bytes()); // blob_version = 2
    let mut corrupt = good.clone();
    corrupt[0] = b'X'; // wrong magic

    for (blob, want) in [
        (good, NoiseProfileStatus::Loaded),
        (too_new, NoiseProfileStatus::TooNew),
        (corrupt, NoiseProfileStatus::Unreadable),
    ] {
        let model = model(vec![nr_slot(NoiseReduction::state_with_blob(Some(blob)))]);
        let host = host_with(registry.clone(), &model);
        assert_eq!(host.slot_info(0).unwrap().noise_profile, Some(want));
    }
}

// --- Live replacement (SPEC-014 §2.3 "Result") -----------------------------------------------

#[test]
fn replace_noise_print_installs_the_blob_live_and_keeps_the_parameters() {
    let registry = nr_registry();
    let x = white(11, 4 * SR as usize);
    let mut d = Driver::with_registry(registry, &model(vec![default_nr_slot()]), 3, x.len());
    d.host
        .set_param(0, NoiseReduction::REDUCTION_DB, 20.0)
        .unwrap();
    d.run_until(&x, (2.0 * SR) as usize);

    let before = rms_db(&d.out[(1.0 * SR) as usize..(2.0 * SR) as usize]);

    let blob = print_of(9, 24_000);
    d.host.replace_noise_print(0, blob).unwrap();
    d.run_to_end(&x);

    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::SlotRestarted { index: 0, .. })),
        "a replacement swaps in a fresh instance"
    );
    // The parameter value survives the replacement (only the blob changed).
    assert_eq!(
        d.host.param_value(0, NoiseReduction::REDUCTION_DB),
        Some(20.0)
    );
    assert_eq!(
        d.host.slot_info(0).unwrap().noise_profile,
        Some(NoiseProfileStatus::Loaded)
    );

    let after = rms_db(&d.out[x.len() - (1.0 * SR) as usize..]);
    assert!(
        after < before - 5.0,
        "the noise floor should drop once the print is loaded: before {before} dB, after {after} dB"
    );
}

// --- AC-14-style capture source: upstream slots are applied before the print is taken --------

#[test]
fn capture_source_is_the_audio_as_the_nr_slot_receives_it() {
    let registry = nr_registry();
    let model = model(vec![gain_slot(-6.0), default_nr_slot()]);
    let host = host_with(registry.clone(), &model);

    let raw = white(21, 4 * 24_000);
    let upstream = host.upstream_model(1);
    let rendered = vox_rack::offline::render(&registry, &upstream, SR, &raw).unwrap();

    let extension = host.noise_profile_extension(1).unwrap();
    let values = vec![0.0; host.slot_info(1).unwrap().params.len()];
    let blob = extension
        .capture(&rendered, SR, &values, &AtomicBool::new(false))
        .unwrap();
    let got = vox_dsp::nr::ProfileView::parse(&blob).unwrap();

    let raw_blob = vox_dsp::nr::capture_profile(&raw, SR, &AtomicBool::new(false)).unwrap();
    let want = vox_dsp::nr::ProfileView::parse(&raw_blob).unwrap();
    let scale = 10f64.powf(-6.0 / 10.0); // -6 dB gain -> 1/4 power
    for k in 100..2000 {
        let g = f64::from(got.density(k));
        let w = f64::from(want.density(k)) * scale;
        assert!(
            ((g - w) / w).abs() < 1e-3,
            "bin {k}: {g} vs {w} (raw x -6 dB Gain)"
        );
    }
}
