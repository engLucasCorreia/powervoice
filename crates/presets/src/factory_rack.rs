//! Factory rack presets (T-406, ticket goal: "Podcast voice", "Audiobook (ACX)", "Gentle
//! cleanup"). Built from literal module ids and parameter keys (ADR-005 §2/§3) rather than a
//! `vox-modules` dependency — like `vox_engine::rack_api::NOISE_REDUCTION_MODULE_ID`
//! (MEMORY.md S3-06), a dev-dependency test (`tests/factory_rack.rs`) guards the literals against
//! drift. Every slot state only sets the keys it cares about: [`vox_module_api::prepare_state`]
//! (used by `Registry::resolve` on load, ADR-005 §10) fills every other key at the module's own
//! schema default, so these presets stay correct as long as the module's *other* defaults do.

use std::collections::BTreeMap;

use vox_module_api::{LocalizedText, ModuleRef, ModuleState, Version};
use vox_rack::{RackModel, SlotModel};

const NOISE_GATE_ID: &str = "org.powervoice.noise-gate";
const DYNAMICS_ID: &str = "org.powervoice.dynamics";
const PARAMETRIC_EQ_ID: &str = "org.powervoice.parametric-eq";
const TRUE_PEAK_LIMITER_ID: &str = "org.powervoice.true-peak-limiter";

/// Every built-in module id a factory rack preset below refers to (in-file test fodder; the
/// dev-dependency test in `tests/factory_rack.rs` checks these against the real registry).
#[cfg(test)]
const BUILT_IN_MODULE_IDS: [&str; 4] = [
    NOISE_GATE_ID,
    DYNAMICS_ID,
    PARAMETRIC_EQ_ID,
    TRUE_PEAK_LIMITER_ID,
];

fn slot(module_id: &str, overrides: &[(&str, f64)]) -> SlotModel {
    let params: BTreeMap<String, f64> = overrides
        .iter()
        .map(|(k, v)| ((*k).to_owned(), *v))
        .collect();
    let state = ModuleState {
        format_version: 1,
        params,
        blob: None,
    };
    SlotModel::new(
        &ModuleRef {
            id: module_id.to_owned(),
            version: Version::new(1, 0, 0),
        },
        false,
        &state,
    )
}

/// A built-in, read-only rack preset (Effects → Rack Presets, SPEC-012 "the rack-preset menu").
#[derive(Clone, Debug)]
pub struct FactoryRackPreset {
    /// Stable key (never shown; storage/telemetry only).
    pub key: String,
    /// Display name.
    pub name: LocalizedText,
    /// The chain the preset loads.
    pub model: RackModel,
}

fn rumble_filter() -> SlotModel {
    // A Parametric EQ slot used purely as a high-pass filter (ADR-005 §12: every other key stays
    // at its schema default — every other band is neutral/off).
    slot(PARAMETRIC_EQ_ID, &[("hp_on", 1.0), ("hp_freq_hz", 80.0)])
}

/// The built-in factory rack presets (ticket goal: at least "Podcast voice", "Audiobook (ACX)",
/// "Gentle cleanup").
pub fn factory_rack_presets() -> Vec<FactoryRackPreset> {
    vec![
        FactoryRackPreset {
            key: "podcast_voice".into(),
            name: LocalizedText::keyed("rack_preset.podcast_voice", "Podcast voice"),
            // HP 80 Hz -> gate -> compressor -> EQ -> TP limiter -1 dBTP (ticket goal).
            model: RackModel {
                slots: vec![
                    rumble_filter(),
                    slot(NOISE_GATE_ID, &[]),
                    slot(
                        DYNAMICS_ID,
                        &[
                            ("compressor_enabled", 1.0),
                            ("compressor_threshold_db", -20.0),
                            ("compressor_ratio", 3.5),
                            ("compressor_attack_ms", 10.0),
                            ("compressor_release_ms", 120.0),
                            ("compressor_makeup_db", 3.0),
                        ],
                    ),
                    slot(
                        PARAMETRIC_EQ_ID,
                        &[
                            ("b2_freq_hz", 500.0),
                            ("b2_gain_db", -1.5),
                            ("b4_freq_hz", 3_000.0),
                            ("b4_gain_db", 2.5),
                            ("hs_freq_hz", 10_000.0),
                            ("hs_gain_db", 1.5),
                        ],
                    ),
                    slot(TRUE_PEAK_LIMITER_ID, &[("ceiling_dbtp", -1.0)]),
                ],
            },
        },
        FactoryRackPreset {
            key: "audiobook_acx".into(),
            name: LocalizedText::keyed("rack_preset.audiobook_acx", "Audiobook (ACX)"),
            // ACX guidance: HPF for rumble, gentle compression (low ratio, high threshold), and
            // peaks kept under ACX's -3 dB requirement via the limiter's ceiling.
            model: RackModel {
                slots: vec![
                    rumble_filter(),
                    slot(
                        DYNAMICS_ID,
                        &[
                            ("compressor_enabled", 1.0),
                            ("compressor_threshold_db", -20.0),
                            ("compressor_ratio", 2.0),
                            ("compressor_attack_ms", 15.0),
                            ("compressor_release_ms", 150.0),
                            ("compressor_makeup_db", 1.0),
                        ],
                    ),
                    slot(
                        TRUE_PEAK_LIMITER_ID,
                        &[("ceiling_dbtp", -3.0), ("release_ms", 150.0)],
                    ),
                ],
            },
        },
        FactoryRackPreset {
            key: "gentle_cleanup".into(),
            name: LocalizedText::keyed("rack_preset.gentle_cleanup", "Gentle cleanup"),
            // Light-touch polish: rumble removal, light gating, mild leveling, a safety ceiling.
            model: RackModel {
                slots: vec![
                    rumble_filter(),
                    slot(
                        NOISE_GATE_ID,
                        &[
                            ("threshold_db", -50.0),
                            ("hysteresis_db", 4.0),
                            ("range_db", -18.0),
                        ],
                    ),
                    slot(
                        DYNAMICS_ID,
                        &[
                            ("compressor_enabled", 1.0),
                            ("compressor_threshold_db", -18.0),
                            ("compressor_ratio", 1.5),
                            ("compressor_release_ms", 150.0),
                        ],
                    ),
                    slot(TRUE_PEAK_LIMITER_ID, &[("ceiling_dbtp", -1.0)]),
                ],
            },
        },
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact literal values
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_unique_key_and_an_i18n_name() {
        let presets = factory_rack_presets();
        assert!(presets.len() >= 3);
        let mut keys: Vec<&str> = presets.iter().map(|p| p.key.as_str()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate factory rack preset key");
        for p in &presets {
            assert!(p.name.key.is_some() && !p.name.text.is_empty(), "{}", p.key);
            assert!(!p.model.slots.is_empty(), "{} has no slots", p.key);
        }
    }

    #[test]
    fn every_slot_only_names_built_in_module_ids() {
        for preset in factory_rack_presets() {
            for s in &preset.model.slots {
                let id = s.module_ref().unwrap().id;
                assert!(
                    BUILT_IN_MODULE_IDS.contains(&id.as_str()),
                    "{}: unknown module id {id}",
                    preset.key
                );
            }
        }
    }

    /// The ticket's ACX example: ceiling stays under ACX's -3 dB peak requirement.
    #[test]
    fn audiobook_acx_limiter_ceiling_is_minus_3_dbtp() {
        let preset = factory_rack_presets()
            .into_iter()
            .find(|p| p.key == "audiobook_acx")
            .unwrap();
        let limiter = preset
            .model
            .slots
            .iter()
            .find(|s| s.module_ref().unwrap().id == TRUE_PEAK_LIMITER_ID)
            .expect("audiobook_acx has a limiter");
        let state: ModuleState = serde_json::from_value(limiter.state.clone()).unwrap();
        assert_eq!(state.params["ceiling_dbtp"], -3.0);
    }

    /// The ticket's "Podcast voice" example: TP limiter ceiling -1 dBTP.
    #[test]
    fn podcast_voice_limiter_ceiling_is_minus_1_dbtp() {
        let preset = factory_rack_presets()
            .into_iter()
            .find(|p| p.key == "podcast_voice")
            .unwrap();
        let limiter = preset
            .model
            .slots
            .iter()
            .find(|s| s.module_ref().unwrap().id == TRUE_PEAK_LIMITER_ID)
            .unwrap();
        let state: ModuleState = serde_json::from_value(limiter.state.clone()).unwrap();
        assert_eq!(state.params["ceiling_dbtp"], -1.0);
    }
}
