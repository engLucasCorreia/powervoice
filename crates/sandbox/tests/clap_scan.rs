//! T-803: CLAP enumeration in sandbox processes — the test plugin is found in a temporary CLAP
//! path, a bundle that crashes (or hangs) while being scanned is reported and the scan goes on,
//! results are cached by path + size + mtime, the found effects register and load in a rack,
//! and (opt-in, `POWERVOICE_TEST_REAL_CLAP=<file or directory>`) a real installed plugin loads
//! and processes a block.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use common::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, ModuleFactory, OutputEvents, ProcessContext, ProcessMode,
    Transport,
};
use vox_plugin_host::scan::{
    ScanOptions, effect_specs, factories, find_clap_files, scan_clap_files, scan_file,
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
