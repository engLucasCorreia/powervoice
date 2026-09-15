//! T-806: VST3 enumeration — the test plugin's bundle is found in a temporary VST3 path and
//! scanned in a sandbox (with ports and parameter counts), a bundle that crashes (or hangs) while
//! being scanned is reported and blocklisted, a bundle with `moduleinfo.json` is indexed without
//! loading any code, the found effects register and load in a rack, "Install module…" and
//! "Uninstall…" of a `.vst3` bundle, and (opt-in, `POWERVOICE_TEST_REAL_VST3=<bundle or
//! directory>`) a real installed plugin loads and processes a block.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, ModuleFactory, OutputEvents, ProcessContext, ProcessMode,
    Transport,
};
use vox_plugin_host::blocklist::BlockReason;
use vox_plugin_host::scan::{
    FailureKind, ScanOptions, effect_specs, factories, find_vst3_files, scan_file,
    scan_plugin_files,
};
use vox_rack::{MAX_BLOCK, RackHost, RackOptions, Registry, SlotStatus};
use vox_test_vst3 as tv;

fn options(cache: Option<PathBuf>, timeout: Duration) -> ScanOptions {
    let mut o = ScanOptions::new(SANDBOX);
    o.cache = cache;
    o.timeout = timeout;
    o
}

/// A `moduleinfo.json` describing the test plugin's classes (what the SDK's `moduleinfotool`
/// writes, trailing commas included).
fn moduleinfo() -> String {
    let class = |cid: &str, name: &str, sub: &str| {
        format!(
            r#"{{ "CID": "{cid}", "Category": "Audio Module Class", "Name": "{name}",
               "Vendor": "PowerVoice", "Version": "1.2.3", "SDKVersion": "VST 3.8.0",
               "Sub Categories": [ {sub} ], "Class Flags": 0, "Cardinality": 2147483647, }},"#
        )
    };
    format!(
        r#"{{ "Name": "vox-test-vst3", "Version": "1.2.3",
  "Factory Info": {{ "Vendor": "PowerVoice", "URL": "https://powervoice.app", "E-Mail": "", }},
  "Classes": [ {} {} {} ], }}"#,
        class(tv::ID_GAIN, "Test Gain", r#""Fx", "Tools", "Mono","#),
        class(
            tv::ID_GAIN_STEREO,
            "Test Gain (stereo)",
            r#""Fx", "Tools", "Stereo","#
        ),
        class(tv::ID_LATENCY, "Test Latency", r#""Fx", "Tools", "Mono","#),
    )
}

fn write_moduleinfo(bundle: &Path) {
    let path = vox_sandbox_ipc::vst3::moduleinfo_path(bundle);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, moduleinfo()).unwrap();
}

#[test]
fn enumeration_finds_the_test_plugin_and_reports_a_crashing_bundle() {
    let dir = TempDir::new("vst3-scan");
    let good = make_vst3_bundle(&dir.0.join("vendor"), "Good");
    let crash = make_vst3_bundle(&dir.0, tv::CRASH_ON_SCAN_MARKER);
    let garbage = make_vst3_bundle(&dir.0, "garbage");
    let garbage_binary = vox_sandbox_ipc::vst3::binary_path(&garbage).unwrap();
    std::fs::write(garbage_binary, b"not a shared library").unwrap();
    std::fs::write(dir.0.join("readme.txt"), b"ignored").unwrap();

    let files = find_vst3_files(std::slice::from_ref(&dir.0));
    assert_eq!(files.len(), 3, "{files:?}");
    let cache = dir.0.join("cache").join("plugin-scan.json");
    let mut opts = options(Some(cache.clone()), Duration::from_secs(20));
    opts.blocklist = Some(dir.0.join("blocklist.json"));
    let out = scan_plugin_files(&files, &opts);
    let ids: Vec<&str> = out.plugins.iter().map(|p| p.plugin.id.as_str()).collect();
    assert_eq!(ids, tv::IDS.to_vec());
    assert!(out.plugins.iter().all(|p| p.path == good));
    let p = &out.plugins[0].plugin;
    assert_eq!(
        (p.name.as_str(), p.vendor.as_str(), p.version.as_str()),
        ("Test Gain", "PowerVoice", "1.2.3")
    );
    assert_eq!(p.features, ["audio-effect", "utility", "mono"]);
    assert_eq!(p.url.as_deref(), Some("https://powervoice.app"));
    // Richer data: the scan instantiated each effect (never activated).
    let by_id = |id: &str| {
        &out.plugins
            .iter()
            .find(|p| p.plugin.id == id)
            .unwrap()
            .plugin
    };
    let mono = by_id(tv::ID_GAIN);
    assert_eq!(
        (
            mono.param_count,
            mono.main_input_channels,
            mono.main_output_channels
        ),
        (1, 1, 1)
    );
    let stereo = by_id(tv::ID_GAIN_STEREO);
    assert_eq!(
        (stereo.main_input_channels, stereo.main_output_channels),
        (2, 2)
    );
    assert_eq!(by_id(tv::ID_LATENCY).param_count, 2);
    assert_eq!((out.scanned, out.cached, out.indexed), (3, 0, 0));
    assert_eq!(out.failures.len(), 2, "{:?}", out.failures);
    let crashed = out.failures.iter().find(|f| f.path == crash).unwrap();
    assert!(crashed.message.contains("crashed"), "{}", crashed.message);
    assert_eq!(crashed.kind, FailureKind::Crashed);
    let bad = out.failures.iter().find(|f| f.path == garbage).unwrap();
    assert!(bad.message.contains("couldn't load"), "{}", bad.message);

    // Registry entries: `vst3:<class id>`, loaded from the bundle that offered them.
    let specs = effect_specs(&out);
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].descriptor.id, format!("vst3:{}", tv::ID_GAIN));
    assert_eq!(specs[0].format, "vst3");
    assert_eq!(specs[0].path(), Some(good.clone()));

    // Cached; the crashing bundle is blocklisted (not scanned again); the garbage one retried.
    let again = scan_plugin_files(&files, &opts);
    assert_eq!((again.scanned, again.cached), (1, 1));
    assert_eq!(again.blocklisted, vec![crash.clone()]);
    assert_eq!(again.plugins, out.plugins);
    let blocked = vox_plugin_host::blocklist::Blocklist::load(opts.blocklist.clone()).list();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].1.reason, BlockReason::Crashed);
    // The cache records the format.
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(json["entries"][0]["format"], "vst3");
}

#[test]
fn a_bundle_with_moduleinfo_is_indexed_without_loading_any_code() {
    let dir = TempDir::new("vst3-moduleinfo");
    // Its binary would crash if loaded: indexing it must not load it.
    let crash = make_vst3_bundle(&dir.0, tv::CRASH_ON_SCAN_MARKER);
    write_moduleinfo(&crash);
    let indexed = make_vst3_bundle(&dir.0, "Indexed");
    write_moduleinfo(&indexed);
    let mut opts = options(None, Duration::from_secs(20));
    opts.blocklist = Some(dir.0.join("blocklist.json"));
    let files = find_vst3_files(std::slice::from_ref(&dir.0));
    let out = scan_plugin_files(&files, &opts);
    assert_eq!((out.indexed, out.scanned), (2, 0), "no sandbox ran");
    assert!(out.failures.is_empty(), "{:?}", out.failures);
    assert!(
        !dir.0.join("blocklist.json").exists(),
        "nothing blocklisted"
    );
    assert_eq!(out.plugins.len(), 6);
    let p = &out
        .plugins
        .iter()
        .find(|p| p.path == indexed)
        .unwrap()
        .plugin;
    assert_eq!(p.param_count, 0, "unknown without instantiating");
    assert_eq!(p.features, ["audio-effect", "utility", "mono"]);
    // The indexed spec loads and runs like a scanned one.
    let spec = effect_specs(&out)
        .into_iter()
        .find(|s| s.path() == Some(indexed.clone()) && s.descriptor.id.ends_with(tv::ID_GAIN))
        .unwrap();
    let f = vox_plugin_host::SandboxFactory::new(spec, exact_options());
    let m = f.create().expect("the indexed plugin loads");
    assert_eq!(m.params().len(), 1);
    // `scan_file` (the install path's single-file scan) takes the same shortcut.
    let plugins = scan_file(Path::new(SANDBOX), &crash, Duration::from_secs(5)).unwrap();
    assert_eq!(plugins.len(), 3);
}

#[test]
fn a_bundle_that_hangs_while_being_scanned_is_stopped() {
    let dir = TempDir::new("vst3-hang");
    let hang = make_vst3_bundle(&dir.0, tv::HANG_ON_SCAN_MARKER);
    let t0 = Instant::now();
    let err = scan_file(Path::new(SANDBOX), &hang, Duration::from_millis(700)).unwrap_err();
    assert!(err.contains("didn't finish scanning"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(5));
}

#[test]
fn scanned_effects_register_and_load_in_the_rack() {
    let dir = TempDir::new("vst3-rack");
    make_vst3_bundle(&dir.0, "Test");
    let files = find_vst3_files(std::slice::from_ref(&dir.0));
    let out = scan_plugin_files(&files, &options(None, Duration::from_secs(20)));
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
    host.insert_module(0, &format!("vst3:{}", tv::ID_GAIN_STEREO))
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
    assert_eq!(info.name, "Test Gain (stereo)");
    host.teardown(live);
}

/// "Install module…" and "Uninstall…" of a `.vst3` bundle end to end: copied into a temporary
/// per-user VST3 folder (never the real home), scanned there alone in a sandbox, hot-added; a
/// name collision; a bundle that crashes while being scanned is rolled back and blocklisted;
/// uninstalling removes the bundle and the registrations.
#[test]
fn install_and_uninstall_a_vst3_bundle() {
    use vox_plugin_host::install::{InstallError, user_vst3_dir_from};
    use vox_plugin_host::{CatalogPaths, PluginCatalog, SandboxOptions};

    let dir = TempDir::new("vst3-install");
    let downloads = dir.0.join("downloads");
    let picked = make_vst3_bundle(&downloads, "vox-test");
    let home = dir.0.join("home");
    let dest = user_vst3_dir_from(Some(home.as_os_str()), Some(home.as_os_str())).unwrap();
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

    let report = catalog.install(&picked, &dest, false).unwrap();
    let target = dest.join("vox-test.vst3");
    assert_eq!(report.target, target);
    let ids: Vec<String> = report
        .effects
        .iter()
        .map(|s| s.descriptor.id.clone())
        .collect();
    let expected: Vec<String> = tv::IDS.iter().map(|id| format!("vst3:{id}")).collect();
    assert_eq!(ids, expected);
    for id in &ids {
        assert!(registry.get(id).is_some(), "{id} hot-added");
    }
    assert_eq!(
        catalog
            .details(&expected[1])
            .map(|d| (d.input_channels, d.output_channels)),
        Some((2, 2))
    );
    assert!(matches!(
        catalog.install(&picked, &dest, false),
        Err(InstallError::Collision { .. })
    ));
    assert!(catalog.install(&picked, &dest, true).unwrap().replaced);

    let crash = make_vst3_bundle(&downloads, tv::CRASH_ON_SCAN_MARKER);
    let err = catalog.install(&crash, &dest, false).unwrap_err();
    assert!(
        matches!(
            err,
            InstallError::ScanFailed {
                blocklisted: true,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        !dest
            .join(format!("{}.vst3", tv::CRASH_ON_SCAN_MARKER))
            .exists(),
        "rolled back"
    );

    catalog.uninstall(&target, &dest).unwrap();
    assert!(!target.exists());
    for id in &ids {
        assert!(registry.get(id).is_none(), "{id} unregistered");
    }
    assert!(picked.exists(), "the picked bundle is untouched");
}

/// `POWERVOICE_TEST_REAL_VST3=<.vst3 bundle or directory>`: loads the first audio effect found
/// there in a sandbox and processes a few blocks. Skipped (passes) when unset.
#[test]
fn real_vst3_plugin_smoke() {
    let Some(target) = std::env::var_os("POWERVOICE_TEST_REAL_VST3") else {
        eprintln!("POWERVOICE_TEST_REAL_VST3 not set: real-plugin smoke test skipped");
        return;
    };
    let target = PathBuf::from(target);
    let files = if target.extension().is_some_and(|e| e == "vst3") {
        vec![target]
    } else {
        find_vst3_files(&[target])
    };
    let out = scan_plugin_files(&files, &options(None, Duration::from_secs(30)));
    for f in &out.failures {
        eprintln!("skipped {}: {}", f.path.display(), f.message);
    }
    let specs = effect_specs(&out);
    let spec = specs.first().expect("no VST3 audio effect found");
    eprintln!(
        "loading {} ({})",
        spec.descriptor.name.text, spec.descriptor.id
    );
    let f = vox_plugin_host::SandboxFactory::new(spec.clone(), exact_options());
    let mut m = f.create().expect("the plugin loads");
    eprintln!("{} parameters", m.params().len());
    m.activate(&ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    })
    .expect("the plugin activates");
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
