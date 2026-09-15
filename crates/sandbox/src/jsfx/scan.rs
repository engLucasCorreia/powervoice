//! `powervoice-sandbox --scan <script> --format jsfx` (ADR-008 §6, T-808): load one JSFX script
//! through ysfx in this disposable process and describe it. A script that crashes or hangs while
//! being loaded, compiled or initialised takes only this process down (the editor blocklists it).
//!
//! The name, author, tags, pins and sliders come from the header. The script is also compiled
//! and its `@init` run once (48 kHz), so one that can't be hosted (it doesn't compile, an import
//! is missing, it has no audio pins) is listed **without** the `audio-effect` feature — the
//! reason on the sandbox's stderr — and never reaches Add Module, while one whose `@init` never
//! returns is caught here rather than in the rack.

use std::path::Path;

use vox_sandbox_ipc::protocol::ScanReport;

/// JSFX `tags:` words → registry features.
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
fn tag_feature(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "eq" | "equalizer" | "equaliser" => "equalizer",
        "filter" => "filter",
        "dynamics" | "compressor" => "compressor",
        "expander" => "expander",
        "gate" => "gate",
        "limiter" => "limiter",
        "reverb" => "reverb",
        "delay" | "echo" => "delay",
        "distortion" | "saturation" | "waveshaper" => "distortion",
        "analysis" | "analyzer" | "analyser" | "meter" | "metering" => "analyzer",
        "utility" => "utility",
        "pitch" => "pitch-shifter",
        "chorus" => "chorus",
        "flanger" => "flanger",
        "phaser" => "phaser",
        _ => return None,
    })
}

/// The registry features of a script with `tags` and `num_in`/`num_out` audio pins:
/// `audio-effect` (pins both ways) or `instrument` (outputs only), then the tag features, then
/// `mono`/`stereo` from the inputs.
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) fn features(tags: &[String], num_in: u32, num_out: u32) -> Vec<String> {
    let mut f: Vec<String> = Vec::new();
    if num_in > 0 && num_out > 0 {
        f.push(vox_module_api::features::AUDIO_EFFECT.into());
    } else if num_out > 0 {
        f.push("instrument".into());
    }
    for word in tags
        .iter()
        .flat_map(|t| t.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|w| !w.is_empty())
    {
        if let Some(x) = tag_feature(&word.to_ascii_lowercase())
            && !f.iter().any(|y| y == x)
        {
            f.push(x.into());
        }
    }
    match num_in {
        1 => f.push(vox_module_api::features::MONO.into()),
        2 => f.push("stereo".into()),
        _ => {}
    }
    f
}

/// The script at `path`.
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn scan(path: &Path) -> Result<ScanReport, String> {
    use super::fx::Fx;
    use super::params;
    use vox_sandbox_ipc::jsfx;
    use vox_sandbox_ipc::protocol::ScannedPlugin;

    if !jsfx::is_script(path) {
        return Err(format!("{} is not a JSFX script", path.display()));
    }
    let text = std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut scanned = ScannedPlugin {
        id: jsfx::plugin_id(path),
        name: file_name.clone(),
        version: jsfx::version(&text).unwrap_or_default(),
        ..ScannedPlugin::default()
    };
    match Fx::load(path) {
        Ok(mut fx) => {
            let name = fx.name();
            if !name.trim().is_empty() {
                scanned.name = name;
            }
            scanned.vendor = fx.author();
            let (ins, outs) = (fx.num_inputs(), fx.num_outputs());
            scanned.features = features(&fx.tags(), ins, outs);
            scanned.param_count = params::map(&fx.sliders()).len() as u32;
            scanned.main_input_channels = ins;
            scanned.main_output_channels = outs;
            // A script whose `@init` hangs or crashes is caught here (and blocklisted).
            fx.prepare(48_000.0, 4096);
        }
        Err(e) => eprintln!("powervoice-sandbox: {file_name} [scan] not usable: {e}"),
    }
    Ok(ScanReport {
        path: path.to_string_lossy().into_owned(),
        plugins: vec![scanned],
    })
}

/// JSFX hosting isn't built for this platform.
#[cfg(not(all(unix, any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub fn scan(_path: &Path) -> Result<ScanReport, String> {
    Err(super::UNSUPPORTED.into())
}

#[cfg(all(test, unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    #[test]
    fn tags_and_pins_become_features() {
        let t = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            features(&t(&["utility"]), 1, 1),
            ["audio-effect", "utility", "mono"]
        );
        assert_eq!(
            features(&t(&["Dynamics Compressor", "limiter"]), 2, 2),
            ["audio-effect", "compressor", "limiter", "stereo"]
        );
        assert_eq!(features(&t(&["synth"]), 0, 2), ["instrument"]);
        assert_eq!(features(&[], 0, 0), Vec::<String>::new());
        assert_eq!(
            features(&t(&["eq,filter"]), 4, 2),
            ["audio-effect", "equalizer", "filter"]
        );
    }
}
