//! `powervoice-sandbox --scan <bundle> --format lv2` (ADR-008 §6, T-807): load one `.lv2`
//! bundle's data (`manifest.ttl` and the files it points to) through lilv in this disposable
//! process and list its plugins. A bundle that crashes or hangs takes only this process down.
//!
//! LV2 describes ports in its data, so the names, features, channel counts and parameter counts
//! come from the TTL; each audio effect is then also **instantiated** (never activated) with
//! PowerVoice's features — which loads its binary — so a plugin that can't be hosted here (a
//! required feature PowerVoice doesn't provide, a port type it can't connect, a binary that
//! doesn't load) is reported without the `audio-effect` feature and never reaches Add Module.

use std::path::Path;

use vox_sandbox_ipc::protocol::ScanReport;
#[cfg(unix)]
use vox_sandbox_ipc::protocol::ScannedPlugin;

/// LV2 plugin classes (`lv2:<Class>`) → registry features.
#[cfg(unix)]
fn class_feature(class: &str) -> Option<&'static str> {
    Some(match class {
        "EQPlugin" | "ParaEQPlugin" | "MultiEQPlugin" => "equalizer",
        "FilterPlugin" | "LowpassPlugin" | "HighpassPlugin" | "BandpassPlugin"
        | "AllpassPlugin" | "CombPlugin" => "filter",
        "DynamicsPlugin" | "CompressorPlugin" => "compressor",
        "ExpanderPlugin" => "expander",
        "GatePlugin" => "gate",
        "LimiterPlugin" => "limiter",
        "ReverbPlugin" => "reverb",
        "DelayPlugin" => "delay",
        "DistortionPlugin" | "WaveshaperPlugin" => "distortion",
        "AnalyserPlugin" => "analyzer",
        "UtilityPlugin" | "MixerPlugin" | "ConverterPlugin" | "AmplifierPlugin"
        | "FunctionPlugin" => "utility",
        "PitchPlugin" => "pitch-shifter",
        "SpatialPlugin" => "surround",
        "ChorusPlugin" => "chorus",
        "FlangerPlugin" => "flanger",
        "PhaserPlugin" => "phaser",
        _ => return None,
    })
}

/// The registry features of a plugin with `classes` (`rdf:type` URIs) and `main_in`/`main_out`
/// main audio ports: `instrument`, or `audio-effect` (audio in and out), then the class
/// features, then `mono`/`stereo` from the inputs.
#[cfg(unix)]
pub(crate) fn features(classes: &[String], main_in: usize, main_out: usize) -> Vec<String> {
    let names: Vec<&str> = classes
        .iter()
        .filter_map(|c| c.strip_prefix(vox_lv2_abi::uri::LV2_CORE_PREFIX))
        .collect();
    let mut f: Vec<String> = Vec::new();
    let generator = names.iter().any(|n| {
        matches!(
            *n,
            "GeneratorPlugin" | "OscillatorPlugin" | "ConstantPlugin"
        )
    });
    if names.contains(&"InstrumentPlugin") || (main_in == 0 && generator) {
        f.push("instrument".into());
    } else if main_in > 0 && main_out > 0 {
        f.push(vox_module_api::features::AUDIO_EFFECT.into());
    }
    for n in &names {
        if let Some(x) = class_feature(n)
            && !f.iter().any(|y| y == x)
        {
            f.push(x.into());
        }
    }
    match main_in {
        1 => f.push(vox_module_api::features::MONO.into()),
        2 => f.push("stereo".into()),
        _ => {}
    }
    f
}

/// The plugins of the bundle `path`.
#[cfg(unix)]
pub fn scan(path: &Path) -> Result<ScanReport, String> {
    use super::Lv2Instance;
    use super::instance::describe;
    use super::lilv::{self, World};
    use crate::clap::scan::looks_like_effect;

    let world = World::new(lilv::api()?)?;
    world.load_bundle(path)?;
    let mut plugins = Vec::new();
    for plugin in world.plugins() {
        let d = describe(&world, plugin);
        if d.uri.is_empty() {
            continue;
        }
        let (ins, outs) = d.main_audio();
        let mut features = features(&d.classes, ins.len(), outs.len());
        let mut scanned = ScannedPlugin {
            id: d.uri.clone(),
            name: d.name.clone(),
            vendor: d.vendor.clone(),
            version: d.version.clone(),
            description: d.description.clone(),
            url: d.url.clone(),
            features: Vec::new(),
            param_count: 0,
            main_input_channels: 0,
            main_output_channels: 0,
            module_info: None,
        };
        if looks_like_effect(&features) {
            match Lv2Instance::load(path, &d.uri) {
                Ok(inst) => {
                    scanned.param_count = inst.param_count();
                    (scanned.main_input_channels, scanned.main_output_channels) = inst.main_ports();
                }
                Err(e) => {
                    eprintln!("powervoice-sandbox: {} [scan] not usable: {e}", d.name);
                    features.retain(|f| f != vox_module_api::features::AUDIO_EFFECT);
                }
            }
        }
        scanned.features = features;
        plugins.push(scanned);
    }
    Ok(ScanReport {
        path: path.to_string_lossy().into_owned(),
        plugins,
    })
}

/// LV2 hosting isn't built for this platform.
#[cfg(not(unix))]
pub fn scan(_path: &Path) -> Result<ScanReport, String> {
    Err(super::UNSUPPORTED.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn class(c: &str) -> String {
        format!("{}{c}", vox_lv2_abi::uri::LV2_CORE_PREFIX)
    }

    #[test]
    fn classes_become_features() {
        assert_eq!(
            features(&[class("Plugin"), class("UtilityPlugin")], 1, 1),
            ["audio-effect", "utility", "mono"]
        );
        assert_eq!(
            features(&[class("CompressorPlugin"), class("DynamicsPlugin")], 2, 2),
            ["audio-effect", "compressor", "stereo"]
        );
        assert_eq!(features(&[class("InstrumentPlugin")], 0, 2), ["instrument"]);
        assert_eq!(features(&[class("OscillatorPlugin")], 0, 1), ["instrument"]);
        assert_eq!(features(&[class("EQPlugin")], 1, 0), ["equalizer", "mono"]);
    }
}
