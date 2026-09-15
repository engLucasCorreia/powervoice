//! T-803: CLAP enumeration in sandbox processes — the test plugin is found in a temporary CLAP
//! path, a bundle that crashes (or hangs) while being scanned is reported and the scan goes on,
//! results are cached by path + size + mtime, the found effects register and load in a rack,
//! and (opt-in, `POWERVOICE_TEST_REAL_CLAP=<file or directory>`) a real installed plugin loads
//! and processes a block.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, ModuleFactory, OutputEvents, ProcessContext, ProcessMode,
    ProcessStatus, Transport,
};
use vox_plugin_host::blocklist::{BlockReason, Blocklist};
use vox_plugin_host::scan::{
    FailureKind, ScanOptions, effect_specs, factories, find_clap_files, scan_clap_files,
    scan_clap_files_with, scan_file,
};
use vox_rack::{MAX_BLOCK, RackHost, RackOptions, Registry, SlotStatus};
use vox_test_clap as tc;

fn copy_plugin(to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(test_clap_library(), to).unwrap();
}

fn options(cache: Option<PathBuf>, timeout: Duration) -> ScanOptions {
    let mut o = ScanOptions::new(SANDBOX);
    o.cache = cache;
    o.timeout = timeout;
    o
}

#[test]
fn enumeration_finds_the_test_plugin_and_reports_a_crashing_bundle() {
    let dir = TempDir::new("clap-scan");
    let good = dir.0.join("vendor").join("good.clap");
    copy_plugin(&good);
    copy_plugin(&dir.0.join(format!("{}.clap", tc::CRASH_ON_SCAN_MARKER)));
    std::fs::write(dir.0.join("garbage.clap"), b"not a shared library").unwrap();
    std::fs::write(dir.0.join("readme.txt"), b"ignored").unwrap();

    let files = find_clap_files(std::slice::from_ref(&dir.0));
    assert_eq!(files.len(), 3, "{files:?}");
    let cache = dir.0.join("cache").join("clap-scan.json");
    let opts = options(Some(cache.clone()), Duration::from_secs(20));
    let out = scan_clap_files(&files, &opts);
    let ids: Vec<&str> = out.plugins.iter().map(|p| p.plugin.id.as_str()).collect();
    assert_eq!(ids, tc::IDS.to_vec());
    assert!(out.plugins.iter().all(|p| p.path == good));
    let p = &out.plugins[0].plugin;
    assert_eq!(
        (p.name.as_str(), p.vendor.as_str(), p.version.as_str()),
        ("Test Gain", "PowerVoice", "1.2.3")
    );
    assert!(p.features.iter().any(|f| f == "audio-effect"));
    assert_eq!((out.scanned, out.cached), (3, 0));
    assert_eq!(out.failures.len(), 2, "{:?}", out.failures);
    let crash = out
        .failures
        .iter()
        .find(|f| f.path.to_string_lossy().contains(tc::CRASH_ON_SCAN_MARKER))
        .unwrap();
    assert!(crash.message.contains("crashed"), "{}", crash.message);
    let garbage = out
        .failures
        .iter()
        .find(|f| f.path.ends_with("garbage.clap"))
        .unwrap();
    assert!(
        garbage.message.contains("couldn't load"),
        "{}",
        garbage.message
    );

    // Registry entries: `clap:<id>`, loaded from the file that offered them.
    let specs = effect_specs(&out);
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].descriptor.id, "clap:org.powervoice.test.gain");
    assert!(specs[0].plugin.contains("good.clap"));

    // Cached: unchanged files aren't loaded again; failures are retried.
    let again = scan_clap_files(&files, &opts);
    assert_eq!((again.scanned, again.cached), (2, 1));
    assert_eq!(again.plugins, out.plugins);
    // A changed file (new mtime) is scanned again.
    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&good)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let changed = scan_clap_files(&files, &opts);
    assert_eq!((changed.scanned, changed.cached), (3, 0));
    assert_eq!(changed.plugins.len(), 3);
}

#[test]
fn a_bundle_that_hangs_while_being_scanned_is_stopped() {
    let dir = TempDir::new("clap-hang");
    let hang = dir.0.join(format!("{}.clap", tc::HANG_ON_SCAN_MARKER));
    copy_plugin(&hang);
    let t0 = Instant::now();
    let err = scan_file(Path::new(SANDBOX), &hang, Duration::from_millis(700)).unwrap_err();
    assert!(err.contains("didn't finish scanning"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(5));
}

#[test]
fn scanned_effects_register_and_load_in_the_rack() {
    let dir = TempDir::new("clap-rack");
    copy_plugin(&dir.0.join("test.clap"));
    let files = find_clap_files(std::slice::from_ref(&dir.0));
    let out = scan_clap_files(&files, &options(None, Duration::from_secs(20)));
    let mut all = vox_modules::builtin_factories();
    all.extend(factories(&effect_specs(&out), &exact_options()));
    let registry = std::sync::Arc::new(Registry::with_factories(all).unwrap());
    let config = ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    };
    let (mut host, live) =
        RackHost::new(registry, config, RackOptions::default(), &model(Vec::new())).unwrap();
    host.insert_module(0, "clap:org.powervoice.test.gain-stereo")
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

/// `POWERVOICE_TEST_REAL_CLAP=<.clap file or directory>`: loads the first audio effect found
/// there in a sandbox and processes a few blocks. Skipped (passes) when unset.
#[test]
fn real_clap_plugin_smoke() {
    let Some(target) = std::env::var_os("POWERVOICE_TEST_REAL_CLAP") else {
        eprintln!("POWERVOICE_TEST_REAL_CLAP not set: real-plugin smoke test skipped");
        return;
    };
    let target = PathBuf::from(target);
    let files = if target.is_dir() && target.extension().is_none_or(|e| e != "clap") {
        find_clap_files(&[target])
    } else {
        vec![target]
    };
    let out = scan_clap_files(&files, &options(None, Duration::from_secs(30)));
    for f in &out.failures {
        eprintln!("skipped {}: {}", f.path.display(), f.message);
    }
    let specs = effect_specs(&out);
    let spec = specs.first().expect("no CLAP audio effect found");
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

// --- T-804: scanner orchestration, cache, blocklist ------------------------------------------

/// Several copies of the test CLAP, scanned in parallel: all found, and `plugin_scan_progress`
/// (T-804 item 1) events arrive with a strictly increasing `done` count.
#[test]
fn parallel_scan_finds_every_copy_with_progress_in_order() {
    let dir = TempDir::new("clap-parallel");
    const N: usize = 8;
    let mut files = Vec::new();
    for i in 0..N {
        let f = dir.0.join(format!("copy{i}.clap"));
        copy_plugin(&f);
        files.push(f);
    }
    let opts = options(None, Duration::from_secs(20));
    let seen: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let out = scan_clap_files_with(&files, &opts, move |done, total, _path| {
        seen2.lock().unwrap().push((done, total));
    });
    assert_eq!(out.plugins.len(), N * tc::IDS.len());
    assert_eq!(out.scanned, N);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), N, "one progress event per file");
    let dones: Vec<usize> = seen.iter().map(|&(d, _)| d).collect();
    assert_eq!(dones, (1..=N).collect::<Vec<_>>(), "strictly in order");
    assert!(seen.iter().all(|&(_, total)| total == N));
}

/// `crash-on-scan` and `hang-on-scan` are auto-blocklisted (ADR-008 §5, T-804 item 4) and are
/// not scanned again.
#[test]
fn crashing_and_hanging_bundles_are_blocklisted_and_not_rescanned() {
    let dir = TempDir::new("clap-blocklist");
    let crash = dir.0.join(format!("{}.clap", tc::CRASH_ON_SCAN_MARKER));
    let hang = dir.0.join(format!("{}.clap", tc::HANG_ON_SCAN_MARKER));
    copy_plugin(&crash);
    copy_plugin(&hang);
    let files = vec![crash.clone(), hang.clone()];
    let mut opts = options(None, Duration::from_millis(700));
    opts.blocklist = Some(dir.0.join("blocklist.json"));

    let first = scan_clap_files(&files, &opts);
    assert_eq!(first.failures.len(), 2, "{:?}", first.failures);
    assert!(
        first.blocklisted.is_empty(),
        "not blocklisted yet, just failed"
    );
    let crash_failure = first.failures.iter().find(|f| f.path == crash).unwrap();
    assert_eq!(crash_failure.kind, FailureKind::Crashed);
    let hang_failure = first.failures.iter().find(|f| f.path == hang).unwrap();
    assert_eq!(hang_failure.kind, FailureKind::TimedOut);

    // Second scan: both are now blocklisted, so neither is scanned (or fails) again.
    let second = scan_clap_files(&files, &opts);
    assert_eq!(second.scanned, 0, "no sandbox should run");
    assert!(second.failures.is_empty());
    let mut blocklisted = second.blocklisted.clone();
    blocklisted.sort();
    let mut expected = vec![crash, hang];
    expected.sort();
    assert_eq!(blocklisted, expected);
}

/// Touching a blocklisted file's mtime (or its contents — covered at the `vox_plugin_host`
/// blocklist unit level) clears the entry, and the next scan picks it up again.
#[test]
fn touching_a_blocklisted_files_mtime_clears_it() {
    let dir = TempDir::new("clap-unblock-touch");
    let good = dir.0.join("good.clap");
    copy_plugin(&good);
    let blocklist_path = dir.0.join("blocklist.json");
    // Simulate a prior crash without actually crashing the good plugin.
    let mut b = Blocklist::load(Some(blocklist_path.clone()));
    b.block(&good, BlockReason::Crashed);
    drop(b);

    let mut opts = options(None, Duration::from_secs(20));
    opts.blocklist = Some(blocklist_path);
    let blocked = scan_clap_files(std::slice::from_ref(&good), &opts);
    assert_eq!(blocked.blocklisted, vec![good.clone()]);
    assert!(blocked.plugins.is_empty());

    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&good)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let rescanned = scan_clap_files(std::slice::from_ref(&good), &opts);
    assert!(rescanned.blocklisted.is_empty());
    assert_eq!(rescanned.plugins.len(), tc::IDS.len());
}

/// Unblocking a manually blocked plugin, then rescanning, registers it again.
#[test]
fn unblock_then_rescan_registers_the_plugin() {
    let dir = TempDir::new("clap-unblock");
    let good = dir.0.join("good.clap");
    copy_plugin(&good);
    let blocklist_path = dir.0.join("blocklist.json");
    let mut b = Blocklist::load(Some(blocklist_path.clone()));
    b.block(&good, BlockReason::Manual);

    let mut opts = options(None, Duration::from_secs(20));
    opts.blocklist = Some(blocklist_path.clone());
    let blocked = scan_clap_files(std::slice::from_ref(&good), &opts);
    assert!(blocked.plugins.is_empty());

    assert!(b.unblock(&good));
    let unblocked = scan_clap_files(std::slice::from_ref(&good), &opts);
    assert_eq!(unblocked.plugins.len(), tc::IDS.len());
}

/// Richer scan data (T-804 item 3, ADR-008 §6): the mono gain reports 1×1 channels and one
/// parameter; the stereo-only gain (hosted through the mono shim) reports 2×2; the latency
/// variant reports two parameters.
#[test]
fn richer_scan_data_reports_ports_and_parameter_count() {
    let dir = TempDir::new("clap-richer");
    copy_plugin(&dir.0.join("test.clap"));
    let files = find_clap_files(std::slice::from_ref(&dir.0));
    let out = scan_clap_files(&files, &options(None, Duration::from_secs(20)));
    let by_id = |id: &str| {
        out.plugins
            .iter()
            .find(|p| p.plugin.id == id)
            .unwrap_or_else(|| panic!("{id} not found"))
    };
    let mono = &by_id(tc::ID_GAIN).plugin;
    assert_eq!(
        (mono.main_input_channels, mono.main_output_channels),
        (1, 1)
    );
    assert_eq!(mono.param_count, 1);
    let stereo = &by_id(tc::ID_GAIN_STEREO).plugin;
    assert_eq!(
        (stereo.main_input_channels, stereo.main_output_channels),
        (2, 2)
    );
    let latency = &by_id(tc::ID_LATENCY).plugin;
    assert_eq!(latency.param_count, 2);
}

/// A plugin found by a rescan (after the live rack already started) registers and loads without
/// a restart (T-804 item 1: "join the module registry without a restart").
#[test]
fn a_plugin_found_after_a_rescan_is_insertable_without_a_restart() {
    let dir = TempDir::new("clap-hotadd");
    // Start with an empty registry (as if the plugin hadn't been found yet).
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let config = ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    };
    let (mut host, live) = RackHost::new(
        registry.clone(),
        config,
        RackOptions::default(),
        &model(Vec::new()),
    )
    .unwrap();
    assert!(registry.get("clap:org.powervoice.test.gain").is_none());

    // The background rescan finds it and hot-adds it into the *same* registry the live rack
    // already holds.
    copy_plugin(&dir.0.join("test.clap"));
    let files = find_clap_files(std::slice::from_ref(&dir.0));
    let out = scan_clap_files(&files, &options(None, Duration::from_secs(20)));
    for spec in effect_specs(&out) {
        registry.upsert(Arc::new(vox_plugin_host::SandboxFactory::new(
            spec,
            exact_options(),
        )));
    }
    assert!(registry.get("clap:org.powervoice.test.gain").is_some());

    host.insert_module(0, "clap:org.powervoice.test.gain")
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
    host.teardown(live);
}

/// A runtime crash (not a scan-time crash) flags the plugin and increments its crash counter
/// (ADR-008 §5, T-804 item 4) without touching the blocklist. `crash?after=…` is the built-in
/// test backend's crash plugin (T-802), independent of the CLAP scan-time crash test doubles
/// used above — this crash happens while *processing*, not while scanning.
#[test]
fn a_runtime_crash_flags_the_plugin_without_blocklisting_it() {
    let dir = TempDir::new("clap-runtime-crash");
    let blocklist_path = dir.0.join("blocklist.json");
    let health = Arc::new(vox_plugin_host::health::HealthStore::load(Some(
        dir.0.join("health.json"),
    )));
    let mut opts = exact_options();
    opts.health = Some(health.clone());
    let f = factory("crash", "crash?after=20", opts);
    let module_id = f.descriptor().id.clone();
    assert_eq!(health.get(&module_id).crash_count, 0);

    let mut m = f.create().unwrap();
    m.activate(&ActivateConfig {
        sample_rate: RATE,
        max_block: 256,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    })
    .unwrap();
    let x = noise(RATE as usize * 3, 11);
    let block = 256;
    let period = Duration::from_secs_f64(block as f64 / RATE);
    let mut y = vec![0.0f32; block];
    let mut oe = OutputEvents::with_capacity(8);
    let start = Instant::now();
    for (k, i) in x.chunks(block).enumerate() {
        oe.clear();
        let mut ctx = ProcessContext::new(i.len() as u32, 0, Transport::default(), &[], &mut oe);
        let status = m.process(&mut ctx, &[i], &mut [&mut y[..i.len()]]);
        if status == ProcessStatus::Error {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "never crashed");
        if let Some(d) = (start + period * (k as u32 + 1)).checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    let t0 = Instant::now();
    while health.get(&module_id).crash_count == 0 {
        assert!(t0.elapsed() < Duration::from_secs(5), "never flagged");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(health.get(&module_id).crash_count, 1);

    // Never touched the blocklist (only scan-time faults do).
    assert!(!blocklist_path.exists());
}

/// T-809: "Install module…" end to end — the real test plugin is copied into a temporary
/// per-user CLAP folder (never the real home), scanned there alone in a sandbox, and its effects
/// hot-add into a live registry; installing it again is a name collision; a copy that crashes
/// while being scanned is rolled back and blocklisted.
#[test]
fn install_module_copies_scans_and_registers_the_test_plugin() {
    use vox_plugin_host::install::{InstallError, user_clap_dir_from};
    use vox_plugin_host::{CatalogPaths, PluginCatalog, SandboxOptions};

    let dir = TempDir::new("clap-install");
    let downloads = dir.0.join("downloads");
    let picked = downloads.join("vox-test.clap");
    copy_plugin(&picked);
    let home = dir.0.join("home");
    let dest = user_clap_dir_from(Some(home.as_os_str()), Some(home.as_os_str())).unwrap();
    assert!(dest.starts_with(&home));
    let catalog = PluginCatalog::new(
        SandboxOptions::new(SANDBOX),
        CatalogPaths {
            cache: Some(dir.0.join("cache").join("clap-scan.json")),
            blocklist: Some(dir.0.join("cache").join("plugin-blocklist.json")),
        },
    );
    let registry = Arc::new(Registry::new());
    catalog.observe(&registry);

    let report = catalog.install(&picked, &dest, false).unwrap();
    assert_eq!(report.target, dest.join("vox-test.clap"));
    assert!(!report.replaced);
    let ids: Vec<String> = report
        .effects
        .iter()
        .map(|s| s.descriptor.id.clone())
        .collect();
    let expected: Vec<String> = tc::IDS.iter().map(|id| format!("clap:{id}")).collect();
    assert_eq!(ids, expected);
    for id in &ids {
        assert!(registry.get(id).is_some(), "{id} hot-added");
    }
    let details = catalog.details(&expected[0]).unwrap();
    assert_eq!((details.input_channels, details.output_channels), (1, 1));

    assert!(matches!(
        catalog.install(&picked, &dest, false),
        Err(InstallError::Collision { .. })
    ));
    assert!(catalog.install(&picked, &dest, true).unwrap().replaced);

    let crash = downloads.join(format!("{}.clap", tc::CRASH_ON_SCAN_MARKER));
    copy_plugin(&crash);
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
            .join(format!("{}.clap", tc::CRASH_ON_SCAN_MARKER))
            .exists(),
        "rolled back"
    );
    let blocked = catalog.blocklist_snapshot();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0, crash);
    assert_eq!(blocked[0].1.reason, BlockReason::Crashed);
}
