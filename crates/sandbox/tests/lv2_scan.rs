//! T-807: LV2 enumeration — the test plugin's bundle is found in a temporary LV2 path and scanned
//! in a sandbox through lilv (names, features, ports and parameter counts from its data; each
//! effect instantiated once), a bundle that crashes (or hangs) while being scanned is reported and
//! blocklisted, a bundle whose binary doesn't load is listed without the `audio-effect` feature,
//! the found effects register and load in a rack, "Install module…" and "Uninstall…" of an `.lv2`
//! bundle, and (opt-in, `POWERVOICE_TEST_REAL_LV2=<bundle or directory>`) a real installed plugin
//! loads and processes a block. Every test passes with a note when lilv isn't installed.

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
    FailureKind, ScanOptions, effect_specs, factories, find_lv2_files, is_effect, scan_file,
    scan_plugin_files,
};
use vox_rack::{MAX_BLOCK, RackHost, RackOptions, Registry, SlotStatus};
use vox_test_lv2 as tl;

fn options(cache: Option<PathBuf>, timeout: Duration) -> ScanOptions {
    let mut o = ScanOptions::new(SANDBOX);
    o.cache = cache;
    o.timeout = timeout;
    o
}

#[test]
fn enumeration_finds_the_test_plugin_and_reports_a_crashing_bundle() {
    if !lv2_available() {
        return;
    }
    let dir = TempDir::new("lv2-scan");
    let good = make_lv2_bundle(&dir.0.join("vendor"), "Good");
    let crash = make_lv2_bundle(&dir.0, tl::CRASH_ON_SCAN_MARKER);
    let broken = make_lv2_bundle(&dir.0, "broken");
    std::fs::write(
        broken.join(tl::binary_name("broken")),
        b"not a shared library",
    )
    .unwrap();
    // Not plugin bundles: a specification-like bundle and a folder without a manifest.
    let spec = dir.0.join("spec.lv2");
    std::fs::create_dir_all(&spec).unwrap();
    std::fs::write(
        spec.join("manifest.ttl"),
        "<http://example.org/ns> a <http://lv2plug.in/ns/lv2core#Specification> .\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.0.join("empty.lv2")).unwrap();
    std::fs::write(dir.0.join("readme.txt"), b"ignored").unwrap();

    let files = find_lv2_files(std::slice::from_ref(&dir.0));
    assert_eq!(files, vec![broken.clone(), crash.clone(), good.clone()]);
    let cache = dir.0.join("cache").join("plugin-scan.json");
    let mut opts = options(Some(cache.clone()), Duration::from_secs(20));
    opts.blocklist = Some(dir.0.join("blocklist.json"));
    let out = scan_plugin_files(&files, &opts);
    let from = |path: &Path| -> Vec<&vox_plugin_host::scan::ScannedPlugin> {
        out.plugins
            .iter()
            .filter(|p| p.path == path)
            .map(|p| &p.plugin)
            .collect()
    };
    let ids: Vec<&str> = from(&good).iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, tl::URIS.to_vec());
    let mono = from(&good)[0];
    assert_eq!(
        (
            mono.name.as_str(),
            mono.vendor.as_str(),
            mono.version.as_str()
        ),
        ("Test Gain", "PowerVoice", "2.3")
    );
    assert_eq!(mono.features, ["audio-effect", "utility", "mono"]);
    assert_eq!(mono.url.as_deref(), Some("https://powervoice.app"));
    assert_eq!(
        (
            mono.param_count,
            mono.main_input_channels,
            mono.main_output_channels
        ),
        (5, 1, 1)
    );
    let stereo = from(&good)[1];
    assert_eq!(stereo.features, ["audio-effect", "utility", "stereo"]);
    assert_eq!(
        (stereo.main_input_channels, stereo.main_output_channels),
        (2, 2)
    );
    assert_eq!(from(&good)[2].param_count, 2);
    // The broken binary: listed from its data, but never an effect.
    assert_eq!(from(&broken).len(), 3);
    assert!(from(&broken).iter().all(|p| !is_effect(p)));
    assert_eq!((out.scanned, out.cached), (3, 0));
    assert_eq!(out.failures.len(), 1, "{:?}", out.failures);
    let crashed = &out.failures[0];
    assert_eq!(crashed.path, crash);
    assert!(crashed.message.contains("crashed"), "{}", crashed.message);
    assert_eq!(crashed.kind, FailureKind::Crashed);

    // Registry entries: `lv2:<uri>`, loaded from the bundle that offered them.
    let specs = effect_specs(&out);
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].descriptor.id, format!("lv2:{}", tl::URI_GAIN));
    assert_eq!(specs[0].format, "lv2");
    assert_eq!(specs[0].path(), Some(good.clone()));

    // Cached; the crashing bundle is blocklisted (not scanned again).
    let again = scan_plugin_files(&files, &opts);
    assert_eq!((again.scanned, again.cached), (0, 2));
    assert_eq!(again.blocklisted, vec![crash.clone()]);
    let blocked = vox_plugin_host::blocklist::Blocklist::load(opts.blocklist.clone()).list();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].1.reason, BlockReason::Crashed);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(json["entries"][0]["format"], "lv2");

    // Replacing the bundle's binary changes its stamp: scanned again.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::copy(test_lv2_library(), good.join(tl::binary_name("Good"))).unwrap();
    let third = scan_plugin_files(&files, &opts);
    assert_eq!((third.scanned, third.cached), (1, 1));
}

#[test]
fn a_bundle_that_hangs_while_being_scanned_is_stopped() {
    if !lv2_available() {
        return;
    }
    let dir = TempDir::new("lv2-hang");
    let hang = make_lv2_bundle(&dir.0, tl::HANG_ON_SCAN_MARKER);
    let t0 = Instant::now();
    let err = scan_file(Path::new(SANDBOX), &hang, Duration::from_millis(700)).unwrap_err();
    assert!(err.contains("didn't finish scanning"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(5));
}

#[test]
fn scanned_effects_register_and_load_in_the_rack() {
    if !lv2_available() {
        return;
    }
    let dir = TempDir::new("lv2-rack");
    make_lv2_bundle(&dir.0, "Test");
    let files = find_lv2_files(std::slice::from_ref(&dir.0));
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
    host.insert_module(0, &format!("lv2:{}", tl::URI_GAIN_STEREO))
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

/// "Install module…" and "Uninstall…" of an `.lv2` bundle end to end: picking the bundle's
/// `manifest.ttl` stands for the bundle, which is copied into a temporary per-user LV2 folder
/// (never the real home), scanned there alone in a sandbox, hot-added; a name collision; a bundle
/// that crashes while being scanned is rolled back and blocklisted; uninstalling removes the
/// bundle and the registrations.
#[test]
fn install_and_uninstall_an_lv2_bundle() {
    use vox_plugin_host::install::{InstallError, user_lv2_dir_from};
    use vox_plugin_host::{CatalogPaths, PluginCatalog, SandboxOptions};

    if !lv2_available() {
        return;
    }
    let dir = TempDir::new("lv2-install");
    let downloads = dir.0.join("downloads");
    let picked = make_lv2_bundle(&downloads, "vox-test");
    let home = dir.0.join("home");
    let dest = user_lv2_dir_from(Some(home.as_os_str())).unwrap();
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
        .install(&picked.join("manifest.ttl"), &dest, false)
        .unwrap();
    let target = dest.join("vox-test.lv2");
    assert_eq!(report.target, target);
    assert!(target.join("manifest.ttl").is_file() && target.join("plugins.ttl").is_file());
    let ids: Vec<String> = report
        .effects
        .iter()
        .map(|s| s.descriptor.id.clone())
        .collect();
    let expected: Vec<String> = tl::URIS.iter().map(|u| format!("lv2:{u}")).collect();
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

    let crash = make_lv2_bundle(&downloads, tl::CRASH_ON_SCAN_MARKER);
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
            .join(format!("{}.lv2", tl::CRASH_ON_SCAN_MARKER))
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

/// `POWERVOICE_TEST_REAL_LV2=<.lv2 bundle or directory>`: loads the first audio effect found
/// there in a sandbox and processes a few blocks. Skipped (passes) when unset.
#[test]
fn real_lv2_plugin_smoke() {
    let Some(target) = std::env::var_os("POWERVOICE_TEST_REAL_LV2") else {
        eprintln!("POWERVOICE_TEST_REAL_LV2 not set: real-plugin smoke test skipped");
        return;
    };
    let target = PathBuf::from(target);
    let files = if target.extension().is_some_and(|e| e == "lv2") {
        vec![target]
    } else {
        find_lv2_files(&[target])
    };
    let out = scan_plugin_files(&files, &options(None, Duration::from_secs(30)));
    for f in &out.failures {
        eprintln!("skipped {}: {}", f.path.display(), f.message);
    }
    let specs = effect_specs(&out);
    eprintln!("{} LV2 audio effects found", specs.len());
    let spec = specs.first().expect("no LV2 audio effect found");
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
