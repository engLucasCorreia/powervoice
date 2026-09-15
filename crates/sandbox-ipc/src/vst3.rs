//! VST3 vocabulary shared by the sandbox's VST3 backend and the editor's scanner (T-806, ADR-008
//! Amendment 6) — plain facts about bundles and class categories, **no plugin code** (ADR-008
//! §8: the editor links no format host):
//!
//! - [`binary_path`]: where a bundle keeps its module binary for this OS and architecture (the
//!   VST 3 SDK's "plug-in format structure");
//! - [`moduleinfo_path`]: `Contents/Resources/moduleinfo.json` (SDK ≥ 3.7.5), which lets the
//!   editor index a bundle without loading it (ADR-008 §6);
//! - [`AUDIO_MODULE_CLASS`] and [`features`]: which classes are audio processors, and their
//!   `|`-separated sub-categories as the feature strings the registry already uses (CLAP's
//!   vocabulary: `audio-effect`, `instrument`, `equalizer`, …).

use std::path::{Path, PathBuf};

/// `kVstAudioEffectClass`: the category of an audio processor component class (instruments
/// included — their sub-categories tell them apart).
pub const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

/// The architecture folder inside a bundle's `Contents/` for this build (`x86_64-linux`,
/// `aarch64-linux`, `x86_64-win`, `arm64-win`, `MacOS`, …).
pub fn arch_folder() -> &'static str {
    let arch = std::env::consts::ARCH;
    if cfg!(target_os = "macos") {
        "MacOS"
    } else if cfg!(windows) {
        match arch {
            "x86" => "x86-win",
            "aarch64" => "arm64-win",
            _ => "x86_64-win",
        }
    } else {
        match arch {
            "x86" => "i386-linux",
            "aarch64" => "aarch64-linux",
            "arm" => "armv7l-linux",
            _ => "x86_64-linux",
        }
    }
}

/// The module binary's file extension on this OS (`""` on macOS: `Contents/MacOS/<name>`).
fn binary_extension() -> &'static str {
    if cfg!(target_os = "macos") {
        ""
    } else if cfg!(windows) {
        "vst3"
    } else {
        "so"
    }
}

/// The module binary of `path`:
/// - a bundle directory → `Contents/<arch>/<bundle name>.<ext>`, or else (the SDK loader's own
///   fallback) the first file with the platform's extension in that folder;
/// - a regular file → the file itself (Windows' legacy single-file `.vst3`, and how tests load a
///   bare library).
///
/// `None`: a bundle without a binary for this platform and architecture.
pub fn binary_path(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let stem = path.file_stem()?;
    let dir = path.join("Contents").join(arch_folder());
    let ext = binary_extension();
    let mut name = stem.to_os_string();
    if !ext.is_empty() {
        name.push(".");
        name.push(ext);
    }
    let exact = dir.join(&name);
    if exact.is_file() {
        return Some(exact);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && (ext.is_empty() || p.extension().is_some_and(|e| e == ext)))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// `<bundle>/Contents/Resources/moduleinfo.json`.
pub fn moduleinfo_path(bundle: &Path) -> PathBuf {
    bundle
        .join("Contents")
        .join("Resources")
        .join("moduleinfo.json")
}

/// One VST3 sub-category as a registry feature string, when there's an equivalent.
fn mapped(sub: &str) -> Option<&'static str> {
    Some(match sub.to_ascii_lowercase().as_str() {
        "analyzer" => "analyzer",
        "delay" => "delay",
        "distortion" => "distortion",
        "dynamics" => "compressor",
        "eq" => "equalizer",
        "filter" => "filter",
        "mastering" => "mastering",
        "pitch shift" => "pitch-shifter",
        "restoration" => "restoration",
        "reverb" => "reverb",
        "tools" => "utility",
        "spatial" | "surround" => "surround",
        "mono" => "mono",
        "stereo" => "stereo",
        _ => return None,
    })
}

/// A class's `|`-separated sub-categories (`"Fx|EQ|Stereo"`) as registry features:
/// `Instrument` → `instrument`; otherwise `Fx` → `audio-effect` (not for `OnlyARA` classes,
/// which only run inside an ARA host); then every sub-category with an equivalent (`EQ` →
/// `equalizer`, `Tools` → `utility`, `Mono`/`Stereo`, …). Unknown sub-categories are dropped.
pub fn features(sub_categories: &str) -> Vec<String> {
    let subs: Vec<&str> = sub_categories
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let has = |s: &str| subs.iter().any(|x| x.eq_ignore_ascii_case(s));
    let mut out: Vec<String> = Vec::new();
    if has("Instrument") {
        out.push("instrument".to_owned());
    } else if has("Fx") && !has("OnlyARA") {
        out.push("audio-effect".to_owned());
    }
    for f in subs.iter().filter_map(|s| mapped(s)) {
        if !out.iter().any(|x| x == f) {
            out.push(f.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_categories_map_to_registry_features() {
        assert_eq!(
            features("Fx|EQ|Stereo"),
            ["audio-effect", "equalizer", "stereo"]
        );
        assert_eq!(
            features("Fx|Dynamics|Tools|Mono"),
            ["audio-effect", "compressor", "utility", "mono"]
        );
        assert_eq!(features("Instrument|Synth"), ["instrument"]);
        assert!(features("Fx|OnlyARA").is_empty());
        assert_eq!(features(" fx | reverb "), ["audio-effect", "reverb"]);
        assert!(features("").is_empty());
    }

    #[test]
    fn bundles_resolve_their_platform_binary() {
        let root = std::env::temp_dir().join(format!(
            "pv-ipc-vst3-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bundle = root.join("Acme Gain.vst3");
        let arch = bundle.join("Contents").join(arch_folder());
        std::fs::create_dir_all(&arch).unwrap();
        assert_eq!(binary_path(&bundle), None, "no binary yet");
        let ext = binary_extension();
        let named = |n: &str| {
            if ext.is_empty() {
                arch.join(n)
            } else {
                arch.join(format!("{n}.{ext}"))
            }
        };
        // Fallback: the only binary in the architecture folder, whatever its name.
        std::fs::write(named("other"), b"x").unwrap();
        assert_eq!(binary_path(&bundle), Some(named("other")));
        // The bundle's own name wins.
        std::fs::write(named("Acme Gain"), b"x").unwrap();
        assert_eq!(binary_path(&bundle), Some(named("Acme Gain")));
        // A regular file is its own binary; a missing path has none.
        let single = root.join("single.vst3");
        std::fs::write(&single, b"x").unwrap();
        assert_eq!(binary_path(&single), Some(single.clone()));
        assert_eq!(binary_path(&root.join("missing.vst3")), None);
        assert!(moduleinfo_path(&bundle).ends_with("Contents/Resources/moduleinfo.json"));
        std::fs::remove_dir_all(&root).ok();
    }
}
