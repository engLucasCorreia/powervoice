//! T-805 end to end: the packaged Gain's real `.clap`, zipped into a `.voxmod` with its
//! `voxmod.json`, installed through "Install module…" (`PluginCatalog::install_module`: static
//! validation, extraction into `<modules>/<id>/<version>/`, one real sandboxed scan), used in a
//! live rack, then uninstalled:
//! - a document slot that was Missing (the module wasn't installed) recovers live (H-40) and
//!   sounds like the built-in Gain at the slot's stored state;
//! - installing it again asks first (version collision); a built-in id is refused before
//!   anything runs;
//! - "Uninstall…" removes `<modules>/<id>/` and the registry entry; the slot keeps its state.

#![allow(clippy::float_cmp)] // exact values round-tripped through JSON (H-44 preset params)

vox_module_api::install_test_allocator!();

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleRef, OutputEvents, ProcessContext, ProcessMode,
    Transport,
};
use vox_modules::Gain;
use vox_plugin_host::install::{InstallError, installed_versions};
use vox_plugin_host::voxmod::{self, Manifest, VoxmodError, host_platform};
use vox_plugin_host::{CatalogPaths, PluginCatalog};
use vox_rack::{LiveRack, MAX_BLOCK, RackHost, RackNotice, RackOptions, SlotModel, SlotStatus};

const ID: &str = "org.powervoice.gain.packaged";

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

/// `voxmod-gain`'s manifest template (`just voxmod` fills in binaries and checksums).
fn template() -> Manifest {
    serde_json::from_str(include_str!("../../voxmod-gain/voxmod.json")).unwrap()
}

/// Packs `binary` under `manifest` like `just voxmod voxmod-gain` does — including the crate's
/// two example factory presets and its example locale file (H-44, ADR-006 §3/§7 step 5).
fn pack_into(dir: &std::path::Path, manifest: &Manifest, binary: &std::path::Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let license = dir.join("LICENSE-MIT");
    std::fs::write(&license, b"MIT").unwrap();
    let warm_boost = dir.join("warm_boost.vopreset.json");
    std::fs::write(
        &warm_boost,
        include_str!("../../voxmod-gain/presets/warm_boost.vopreset.json"),
    )
    .unwrap();
    let gentle_cut = dir.join("gentle_cut.vopreset.json");
    std::fs::write(
        &gentle_cut,
        include_str!("../../voxmod-gain/presets/gentle_cut.vopreset.json"),
    )
    .unwrap();
    let en_locale = dir.join("en.json");
    std::fs::write(
        &en_locale,
        include_str!("../../voxmod-gain/locales/en.json"),
    )
    .unwrap();
    let out = dir.join(format!("{}-{}.voxmod", manifest.id, manifest.version));
    voxmod::pack(
        manifest,
        &[(host_platform(), binary.to_path_buf())],
        &[
            ("licenses/LICENSE-MIT".into(), license),
            ("presets/warm_boost.vopreset.json".into(), warm_boost),
            ("presets/gentle_cut.vopreset.json".into(), gentle_cut),
            ("locales/en.json".into(), en_locale),
        ],
        &out,
    )
    .unwrap();
    out
}

/// The packaged Gain's real `.clap` packed under `manifest`, built once per test binary and
/// kept in cargo's per-crate scratch folder (`target/tmp`, not `/tmp`: a debug `.clap` is 40 MB
/// and packing it isn't free).
fn package(manifest: &Manifest) -> PathBuf {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let key = format!("{}-{}", manifest.id, manifest.version);
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap();
    built
        .entry(key.clone())
        .or_insert_with(|| {
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join("voxmod-e2e")
                .join(key);
            pack_into(&dir, manifest, &voxmod_gain_library())
        })
        .clone()
}

/// A catalog with its own cache, blocklist and modules folder under `dir`, and the built-ins
/// reserved (as `src-tauri` configures it).
fn catalog(dir: &std::path::Path) -> Arc<PluginCatalog> {
    let c = PluginCatalog::new(
        exact_options(),
        CatalogPaths {
            cache: Some(dir.join("cache").join("plugin-scan.json")),
            blocklist: Some(dir.join("cache").join("plugin-blocklist.json")),
        },
    );
    c.set_modules_dir(Some(dir.join("data").join("modules")));
    c.set_reserved_ids(
        vox_modules::builtin_factories()
            .iter()
            .map(|f| f.descriptor().id.clone())
            .collect(),
    );
    Arc::new(c)
}

fn status(host: &RackHost) -> SlotStatus {
    host.slot_info(0).unwrap().status
}

/// Ticks until nothing is loading (at most 10 s), collecting notices.
fn settle(host: &mut RackHost) -> Vec<RackNotice> {
    let mut notices = host.tick();
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(t0.elapsed() < Duration::from_secs(10), "never loaded");
        thread::sleep(Duration::from_millis(2));
        notices.extend(host.tick());
    }
    notices.extend(host.tick());
    notices
}

/// Drives `live` over `x` in 256-frame calls (allocation-checked), ticking every 3 blocks.
fn drive(host: &mut RackHost, live: &mut LiveRack, x: &[f32]) -> Vec<f32> {
    let mut y = vec![0.0f32; x.len()];
    for (k, (i, o)) in x.chunks(256).zip(y.chunks_mut(256)).enumerate() {
        let t = Transport {
            playing: true,
            position_samples: Some((k * 256) as u64),
        };
        no_alloc(|| live.process(t, i, o)).expect("LiveRack allocated");
        if k % 3 == 0 {
            host.tick();
        }
    }
    y
}

fn reference(x: &[f32], gain_db: f64) -> Vec<f32> {
    let mut g = Gain::new();
    g.load_state(&Gain::state_with_gain_db(gain_db)).unwrap();
    g.activate(&rt()).unwrap();
    let mut y = vec![0.0f32; x.len()];
    let mut oe = OutputEvents::with_capacity(4);
    for (i, o) in x.chunks(512).zip(y.chunks_mut(512)) {
        let mut ctx = ProcessContext::new(i.len() as u32, 0, Transport::default(), &[], &mut oe);
        let mut outs = [o];
        g.process(&mut ctx, &[i], &mut outs);
    }
    y
}

#[test]
fn install_scan_insert_then_uninstall_a_voxmod() {
    let dir = TempDir::new("voxmod-e2e");
    let pkg = package(&template());
    let cat = catalog(&dir.0);
    let modules = cat.modules_dir().unwrap();
    let registry = registry_with(&[]);
    cat.observe(&registry);

    // A document saved with the packaged Gain at −6 dB, opened before the module is installed.
    let r: ModuleRef = format!("{ID}@1.0.0").parse().unwrap();
    let slot = SlotModel::new(&r, false, &Gain::state_with_gain_db(-6.0));
    let (mut host, mut live) = RackHost::new(
        registry.clone(),
        rt(),
        RackOptions::default(),
        &model(vec![slot]),
    )
    .unwrap();
    assert!(matches!(status(&host), SlotStatus::Missing { .. }));

    // Install module…
    let report = cat.install_module(&pkg, false).unwrap();
    let target = modules.join(ID).join("1.0.0").join(format!("{ID}.clap"));
    assert_eq!(report.target, target);
    assert!(!report.replaced);
    assert_eq!(report.effects.len(), 1);
    assert_eq!(
        report.effects[0].descriptor.id, ID,
        "registered under its bare id"
    );
    assert!(target.is_file());
    assert!(
        target
            .with_file_name("licenses")
            .join("LICENSE-MIT")
            .is_file()
    );
    assert!(
        registry.get(ID).is_some(),
        "hot-added into the live registry"
    );
    assert!(
        registry.get(Gain::ID).is_some(),
        "the built-in is untouched (different id)"
    );

    // H-44: the package's two example factory presets are indexed, and its locale file's
    // strings — namespaced under this module's own id — are merged.
    let presets = cat.module_presets(ID);
    let mut keys: Vec<&str> = presets.iter().map(|p| p.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["gentle_cut", "warm_boost"]);
    let warm = presets.iter().find(|p| p.key == "warm_boost").unwrap();
    assert_eq!(warm.state.params["gain_db"], 6.0);
    let locale = cat.module_locale(ID, "en").unwrap().unwrap();
    assert_eq!(
        locale[&format!("modules.{ID}.presets.warm_boost.name")],
        "Warm boost"
    );
    assert_eq!(
        cat.module_locale(ID, "fr"),
        Ok(None),
        "no French file shipped"
    );

    // The Missing slot recovers live (H-40) with its stored state, sandboxed.
    let notices = settle(&mut host);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotRecovered { .. })),
        "{notices:?}"
    );
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Active);
    assert!(info.sandboxed);
    assert_eq!(info.params[0].key, "gain_db", "the module's own schema");

    // …and sounds like the built-in at −6 dB once the crossfade is over, latency-compensated.
    let latency = host.total_latency_samples() as usize;
    let x = noise(RATE as usize, 5);
    let y = drive(&mut host, &mut live, &x);
    let want = reference(&x, -6.0);
    let s = latency + (RATE * 0.015).round() as usize + 256;
    assert_bits(
        &y[s..],
        &want[s - latency..x.len() - latency],
        "installed module",
    );

    // H-44: applying the package's "Warm boost" factory preset (a parameter-only state, no blob)
    // takes over the sandboxed instance's `gain_db`, smoothed, exactly like a built-in factory
    // preset would.
    host.apply_module_preset(0, &warm.state).unwrap();
    let y = drive(&mut host, &mut live, &x);
    let want = reference(&x, 6.0);
    let s = latency + (RATE * 0.02).round() as usize + 256;
    assert_bits(
        &y[s..],
        &want[s - latency..x.len() - latency],
        "the applied factory preset",
    );
    host.teardown(live);

    // Installing it again asks first; a package claiming a built-in id is refused up front.
    match cat.install_module(&pkg, false) {
        Err(InstallError::VersionCollision { installed, new, .. }) => {
            assert_eq!((installed.as_str(), new.as_str()), ("1.0.0", "1.0.0"))
        }
        other => panic!("{other:?}"),
    }
    // (Its binary is never looked at: the manifest is refused first — a stand-in will do.)
    let mut builtin = template();
    builtin.id = Gain::ID.into();
    let stand_in = dir.0.join("stand-in.clap");
    std::fs::write(&stand_in, b"never loaded").unwrap();
    let hostile = pack_into(&dir.0.join("hostile"), &builtin, &stand_in);
    assert!(matches!(
        cat.install_module(&hostile, false),
        Err(InstallError::Package(VoxmodError::BuiltinId(_)))
    ));
    assert!(!modules.join(Gain::ID).exists());

    // The next start knows it from the cache (the modules folder is a scan root).
    let fresh = catalog(&dir.0);
    fresh.load_cached();
    assert!(fresh.specs().iter().any(|s| s.descriptor.id == ID));

    // Uninstall…
    cat.uninstall_module(&target, &modules).unwrap();
    assert!(!modules.join(ID).exists());
    assert!(installed_versions(&modules.join(ID)).is_empty());
    assert!(registry.get(ID).is_none());
    assert!(!cat.specs().iter().any(|s| s.descriptor.id == ID));
    // H-44: both the factory presets and the locale strings are gone with it — read fresh from
    // the folder `uninstall_module` just removed, nothing to invalidate.
    assert!(cat.module_presets(ID).is_empty());
    assert_eq!(cat.module_locale(ID, "en"), Ok(None));
}

#[test]
fn a_replacement_version_takes_over_and_the_old_one_is_gone() {
    let dir = TempDir::new("voxmod-e2e-replace");
    let cat = catalog(&dir.0);
    let modules = cat.modules_dir().unwrap();
    let registry = registry_with(&[]);
    cat.observe(&registry);
    cat.install_module(&package(&template()), false).unwrap();

    // The same binary under a manifest that claims another version: the scan reports the
    // plugin's own version (1.0.0), so the package is refused and 1.0.0 stays installed.
    let mut lying = template();
    lying.version = "1.1.0".into();
    let err = cat.install_module(&package(&lying), true).unwrap_err();
    assert!(matches!(err, InstallError::ModuleMismatch(_)), "{err:?}");
    assert_eq!(installed_versions(&modules.join(ID)), vec!["1.0.0"]);
    assert!(registry.get(ID).is_some());
    assert!(
        modules
            .join(ID)
            .join("1.0.0")
            .join(format!("{ID}.clap"))
            .is_file()
    );

    // Re-installing the same version after confirming replaces it in place.
    let report = cat.install_module(&package(&template()), true).unwrap();
    assert!(report.replaced);
    assert_eq!(installed_versions(&modules.join(ID)), vec!["1.0.0"]);
    assert_eq!(
        cat.specs().iter().filter(|s| s.descriptor.id == ID).count(),
        1
    );
}
