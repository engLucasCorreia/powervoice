//! T-808: JSFX enumeration — the test scripts are found in a temporary effects folder (`.jsfx`
//! files and an extensionless REAPER-style script; imports, text and other files skipped) and
//! each is scanned in its own sandbox (name, author, version, tags, pins and slider count from its
//! header; it's compiled and `@init` run once), a script that doesn't compile is listed without
//! `audio-effect`, a script that hangs while being scanned is stopped and blocklisted, the found
//! effects register and load in a rack, "Install module…" (with the script's relative imports)
//! and "Uninstall…", and (opt-in, `POWERVOICE_TEST_REAL_JSFX=<script or folder>`) a real script
//! loads and processes a block. Every test passes with a note where this build doesn't host JSFX.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, ModuleFactory, OutputEvents, ProcessContext, ProcessMode,
    Transport,
};
use vox_plugin_host::blocklist::{BlockReason, Blocklist};
use vox_plugin_host::scan::{
    FailureKind, ScanOptions, effect_specs, factories, find_jsfx_files, is_effect,
    scan_plugin_files,
};
use vox_rack::{MAX_BLOCK, RackHost, RackOptions, Registry, SlotStatus};

fn options(cache: Option<PathBuf>, timeout: Duration) -> ScanOptions {
    let mut o = ScanOptions::new(SANDBOX);
    o.cache = cache;
    o.timeout = timeout;
    o
}

#[test]
fn enumeration_finds_the_scripts_in_a_temp_path() {
    if !jsfx_available() {
        return;
    }
    let dir = TempDir::new("jsfx-scan");
    let fx = copy_jsfx_scripts(&dir.0);
    // The hanging script has its own test.
    std::fs::remove_file(fx.join("hang.jsfx")).unwrap();
    // REAPER's own scripts have no extension.
    std::fs::copy(fx.join("gain.jsfx"), fx.join("bare")).unwrap();
    std::fs::write(fx.join("notes"), "no header here\n").unwrap();
    std::fs::write(fx.join("readme.txt"), "desc:not a script\n").unwrap();

    let files = find_jsfx_files(std::slice::from_ref(&dir.0));
    let names: Vec<String> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "bare",
            "broken.jsfx",
            "gain.jsfx",
            "hang_processing.jsfx",
            "latency.jsfx",
            "serialize.jsfx",
            "stereo.jsfx"
        ]
    );
    let cache = dir.0.join("cache").join("plugin-scan.json");
    let mut opts = options(Some(cache.clone()), Duration::from_secs(20));
    opts.blocklist = Some(dir.0.join("blocklist.json"));
    let out = scan_plugin_files(&files, &opts);
    assert!(out.failures.is_empty(), "{:?}", out.failures);
    let get = |name: &str| -> &vox_plugin_host::scan::ScannedPlugin {
        &out.plugins
            .iter()
            .find(|p| p.path == fx.join(name))
            .unwrap_or_else(|| panic!("{name} not listed"))
            .plugin
    };
    let gain = get("gain.jsfx");
    assert_eq!(
        (
            gain.id.as_str(),
            gain.name.as_str(),
            gain.vendor.as_str(),
            gain.version.as_str()
        ),
        (
            "PowerVoice/gain.jsfx",
            "PowerVoice Test Gain",
            "PowerVoice",
            "1.2.3"
        )
    );
    assert_eq!(gain.features, ["audio-effect", "utility", "mono"]);
    assert_eq!(
        (
            gain.param_count,
            gain.main_input_channels,
            gain.main_output_channels
        ),
        (4, 1, 1)
    );
    let stereo = get("stereo.jsfx");
    assert_eq!(stereo.features, ["audio-effect", "utility", "stereo"]);
    assert_eq!(
        (stereo.main_input_channels, stereo.main_output_channels),
        (2, 2)
    );
    assert_eq!(
        get("latency.jsfx").features,
        ["audio-effect", "utility", "delay", "mono"]
    );
    assert_eq!(get("bare").id, "PowerVoice/bare");
    assert_eq!(get("bare").name, "PowerVoice Test Gain");
    // Doesn't compile: listed, never an effect.
    assert!(!is_effect(get("broken.jsfx")));
    assert_eq!((out.scanned, out.cached), (7, 0));

    // Registry entries: `jsfx:<path relative to the effects root>`.
    let specs = effect_specs(&out);
    let ids: Vec<&str> = specs.iter().map(|s| s.descriptor.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "jsfx:PowerVoice/bare",
            "jsfx:PowerVoice/gain.jsfx",
            "jsfx:PowerVoice/hang_processing.jsfx",
            "jsfx:PowerVoice/latency.jsfx",
            "jsfx:PowerVoice/serialize.jsfx",
            "jsfx:PowerVoice/stereo.jsfx"
        ]
    );
    assert_eq!(specs[1].format, "jsfx");
    assert_eq!(specs[1].path(), Some(fx.join("gain.jsfx")));

    // Cached.
    let again = scan_plugin_files(&files, &opts);
    assert_eq!((again.scanned, again.cached), (0, 7));
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert!(
        json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["format"] == "jsfx")
    );

    // An edited script is scanned again.
    std::thread::sleep(Duration::from_millis(20));
    let mut text = std::fs::read_to_string(fx.join("gain.jsfx")).unwrap();
    text.push_str("// edited\n");
    std::fs::write(fx.join("gain.jsfx"), text).unwrap();
    let third = scan_plugin_files(&files, &opts);
    assert_eq!((third.scanned, third.cached), (1, 6));
}

#[test]
fn a_script_that_hangs_while_being_scanned_is_stopped_and_blocklisted() {
    if !jsfx_available() {
        return;
    }
    let dir = TempDir::new("jsfx-hang");
    let hang = copy_jsfx_scripts(&dir.0).join("hang.jsfx");
    let mut opts = options(None, Duration::from_millis(1500));
    opts.blocklist = Some(dir.0.join("blocklist.json"));
    let t0 = Instant::now();
    let out = scan_plugin_files(std::slice::from_ref(&hang), &opts);
    assert!(t0.elapsed() < Duration::from_secs(8), "{:?}", t0.elapsed());
    assert_eq!(out.failures.len(), 1, "{:?}", out.failures);
    assert_eq!(out.failures[0].kind, FailureKind::TimedOut);
    assert!(
        out.failures[0].message.contains("didn't finish scanning"),
        "{}",
        out.failures[0].message
    );
    let again = scan_plugin_files(std::slice::from_ref(&hang), &opts);
    assert_eq!(again.blocklisted, vec![hang.clone()]);
    let blocked = Blocklist::load(opts.blocklist.clone()).list();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].1.reason, BlockReason::TimedOut);
}

#[test]
fn scanned_scripts_register_and_load_in_the_rack() {
    if !jsfx_available() {
        return;
    }
    let dir = TempDir::new("jsfx-rack");
    let fx = copy_jsfx_scripts(&dir.0);
    let out = scan_plugin_files(
        &[fx.join("stereo.jsfx")],
        &options(None, Duration::from_secs(20)),
    );
    let mut all = vox_modules::builtin_factories();
    all.extend(factories(&effect_specs(&out), &exact_options()));
    let registry = Arc::new(Registry::with_factories(all).unwrap());
    let config = ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    };
    let (mut host, live) =
        RackHost::new(registry, config, RackOptions::default(), &model(Vec::new())).unwrap();
    host.insert_module(0, "jsfx:PowerVoice/stereo.jsfx")
        .unwrap();
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(t0.elapsed() < Duration::from_secs(10));
        host.tick();
        std::thread::sleep(Duration::from_millis(2));
    }
    host.tick();
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Active);
    assert_eq!(info.name, "PowerVoice Test Gain (stereo)");
    host.teardown(live);
}

/// "Install module…" and "Uninstall…" of a JSFX script end to end: the script is copied into a
/// temporary PowerVoice JSFX folder (never the real home) with the file it imports relatively,
/// scanned there alone in a sandbox, hot-added; a name collision; a script that hangs while being
/// scanned is rolled back and blocklisted; uninstalling removes the script and its registration.
#[test]
fn install_and_uninstall_a_jsfx_script() {
    use vox_plugin_host::install::{InstallError, user_jsfx_dir_from};
    use vox_plugin_host::{CatalogPaths, PluginCatalog, SandboxOptions};

    if !jsfx_available() {
        return;
    }
    let dir = TempDir::new("jsfx-install");
    let picked = copy_jsfx_scripts(&dir.0.join("downloads"));
    let home = dir.0.join("home");
    let dest = user_jsfx_dir_from(Some(home.as_os_str()), None).unwrap();
    assert!(dest.starts_with(&home));
    let catalog = PluginCatalog::new(
        SandboxOptions::new(SANDBOX),
        CatalogPaths {
            cache: Some(dir.0.join("cache").join("plugin-scan.json")),
            blocklist: Some(dir.0.join("cache").join("plugin-blocklist.json")),
        },
    );
    let registry = Arc::new(Registry::new());
    catalog.observe(&registry);

    let report = catalog
        .install(&picked.join("gain.jsfx"), &dest, false)
        .unwrap();
    let target = dest.join("gain.jsfx");
    assert_eq!(report.target, target);
    assert!(
        dest.join("lib").join("gain_core.jsfx-inc").is_file(),
        "its import came along"
    );
    let ids: Vec<String> = report
        .effects
        .iter()
        .map(|s| s.descriptor.id.clone())
        .collect();
    assert_eq!(ids, ["jsfx:gain.jsfx"]);
    assert!(registry.get("jsfx:gain.jsfx").is_some(), "hot-added");
    assert_eq!(
        catalog
            .details("jsfx:gain.jsfx")
            .map(|d| (d.input_channels, d.output_channels)),
        Some((1, 1))
    );
    assert!(matches!(
        catalog.install(&picked.join("gain.jsfx"), &dest, false),
        Err(InstallError::Collision { .. })
    ));
    assert!(
        catalog
            .install(&picked.join("gain.jsfx"), &dest, true)
            .unwrap()
            .replaced
    );

    // A script that hangs while being scanned (a short timeout instead of the catalog's 30 s).
    let hang = picked.join("hang.jsfx");
    let err = catalog
        .install_with(&hang, &dest, false, |p| {
            vox_plugin_host::scan::scan_one(Path::new(SANDBOX), p, Duration::from_millis(1500))
        })
        .unwrap_err();
    assert!(
        matches!(
            err,
            InstallError::ScanFailed {
                blocklisted: true,
                kind: FailureKind::TimedOut,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(!dest.join("hang.jsfx").exists(), "rolled back");

    catalog.uninstall(&target, &dest).unwrap();
    assert!(!target.exists());
    assert!(registry.get("jsfx:gain.jsfx").is_none(), "unregistered");
    assert!(
        picked.join("gain.jsfx").exists(),
        "the picked script is untouched"
    );
}

/// `POWERVOICE_TEST_REAL_JSFX=<script or folder>`: loads the first audio effect found there in a
/// sandbox and processes a few blocks. Skipped (passes) when unset.
#[test]
fn real_jsfx_smoke() {
    let Some(target) = std::env::var_os("POWERVOICE_TEST_REAL_JSFX") else {
        eprintln!("POWERVOICE_TEST_REAL_JSFX not set: real-script smoke test skipped");
        return;
    };
    let target = PathBuf::from(target);
    let files = if target.is_file() {
        vec![target]
    } else {
        find_jsfx_files(&[target])
    };
    let out = scan_plugin_files(&files, &options(None, Duration::from_secs(30)));
    for f in &out.failures {
        eprintln!("skipped {}: {}", f.path.display(), f.message);
    }
    let specs = effect_specs(&out);
    eprintln!("{} JSFX audio effects found", specs.len());
    let spec = specs.first().expect("no JSFX audio effect found");
    eprintln!(
        "loading {} ({})",
        spec.descriptor.name.text, spec.descriptor.id
    );
    let f = vox_plugin_host::SandboxFactory::new(spec.clone(), exact_options());
    let mut m = f.create().expect("the script loads");
    eprintln!("{} parameters", m.params().len());
    m.activate(&ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    })
    .expect("the script activates");
    let x = noise(256 * 40, 9);
    let mut y = vec![0.0f32; x.len()];
    let mut oe = OutputEvents::with_capacity(64);
    for (i, o) in x.chunks(256).zip(y.chunks_mut(256)) {
        oe.clear();
        let mut ctx = ProcessContext::new(256, 0, Transport::default(), &[], &mut oe);
        let mut outs = [o];
        m.process(&mut ctx, &[i], &mut outs);
    }
    assert!(y.iter().all(|v| v.is_finite()), "non-finite output");
    eprintln!(
        "latency {} samples, output peak {}",
        m.latency_samples(),
        y.iter().fold(0.0f32, |a, v| a.max(v.abs()))
    );
    m.deactivate();
    let state = m.save_state().expect("state");
    eprintln!(
        "state: {} bytes",
        state.blob.as_ref().map_or(0, std::vec::Vec::len)
    );
}
