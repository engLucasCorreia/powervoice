//! T-406 (SPEC-012 §2.7, ADR-005 §2 `ModuleFactory::presets`): every built-in module's factory
//! presets are well-formed and, loaded into a fresh instance, pass the whole `ModuleTestHost`
//! obligation set — not just the schema defaults every other test file already covers.

use std::sync::Arc;

use vox_module_api::prepare_state;
use vox_module_api::test_util::ModuleTestHost;
use vox_modules::builtin_factories;

vox_module_api::install_test_allocator!();

/// The ticket asks for "2–3 per module where the specs suggest values" — every built-in has at
/// least two.
#[test]
fn every_built_in_declares_at_least_two_presets() {
    for f in builtin_factories() {
        let presets = f.presets();
        assert!(
            presets.len() >= 2,
            "{} declares only {} preset(s)",
            f.descriptor().id,
            presets.len()
        );
    }
}

#[test]
fn preset_keys_are_unique_snake_case_and_have_an_i18n_name() {
    for f in builtin_factories() {
        let presets = f.presets();
        let mut keys: Vec<&str> = presets.iter().map(|p| p.key.as_str()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys.len(),
            before,
            "{}: duplicate preset key",
            f.descriptor().id
        );
        for p in &presets {
            assert!(
                !p.key.is_empty()
                    && p.key
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}: preset key `{}` isn't snake_case",
                f.descriptor().id,
                p.key
            );
            assert!(
                p.name.key.is_some() && !p.name.text.is_empty(),
                "{}: preset `{}` needs an i18n key and English text",
                f.descriptor().id,
                p.key
            );
        }
    }
}

/// SPEC-014 §2.4: Noise Reduction's factory presets are parameter-only (no blob), so loading one
/// keeps whatever print the slot already has.
#[test]
fn noise_reduction_factory_presets_are_parameter_only() {
    let f = builtin_factories()
        .into_iter()
        .find(|f| f.descriptor().id == "org.powervoice.noise-reduction")
        .expect("noise reduction is a built-in");
    for p in f.presets() {
        assert_eq!(p.state.blob, None, "preset `{}` carries a blob", p.key);
    }
}

/// Every factory preset, loaded into a fresh instance (through the same [`prepare_state`] /
/// `load_state` pipeline the rack uses, ADR-005 §10), passes the full ADR-005 §15 obligation set:
/// schema, activation at 44.1/48/96 kHz, `process` under the allocation checker with finite
/// output, `reset`, the parameter mirror, state round-trip, determinism, text round-trips,
/// extensions. Exact event-timing is not this test's concern (each module's own test file checks
/// it tightly against the schema defaults), so every parameter opts out of it here.
#[test]
fn every_factory_preset_passes_module_test_host() {
    for factory in builtin_factories() {
        let module_id = factory.descriptor().id.clone();
        let params = factory
            .create()
            .unwrap_or_else(|e| panic!("{module_id}: create failed: {e}"))
            .params()
            .to_vec();
        for preset in factory.presets() {
            let key = preset.key.clone();
            let state = preset.state.clone();
            let factory = Arc::clone(&factory);
            let module_id_for_panic = module_id.clone();
            let mut host = ModuleTestHost::new(move || {
                let mut m = factory.create().unwrap_or_else(|e| {
                    panic!("{module_id_for_panic}: preset `{key}`: create failed: {e}")
                });
                let prepared = prepare_state(&*m, state.clone()).unwrap_or_else(|e| {
                    panic!("{module_id_for_panic}: preset `{key}`: prepare_state failed: {e}")
                });
                m.load_state(&prepared).unwrap_or_else(|e| {
                    panic!("{module_id_for_panic}: preset `{key}`: load_state failed: {e}")
                });
                m
            });
            for p in &params {
                host = host.allow_delayed_effect(p.id);
            }
            host.assert_passes();
        }
    }
}
