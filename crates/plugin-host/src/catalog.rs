//! The plugin catalog (T-804, ADR-008 §6 Amendment 4): orchestrates the CLAP scanner so
//! start-up is instant (cached plugins only, no sandbox spawned) and a background rescan finds
//! everything else — new files, changed files, custom folders — hot-adding newly found effects
//! into every [`Registry`] that [`observe`](PluginCatalog::observe)s this catalog, so the
//! Add-module list updates without a restart (ADR-008 §6, T-804 item 1).
//!
//! Format-agnostic in spirit (the sandbox already takes `--format` as a parameter, ADR-008 §6):
//! this orchestrates CLAP today because CLAP is the only backend that exists (T-803). T-806/807/
//! 808 add `vst3`/`lv2`/`jsfx` scanning the same way, without changing this module's shape.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use vox_module_api::ModuleFactory;
use vox_rack::Registry;

use crate::blocklist::{BlockReason, Blocklist};
use crate::factory::{SandboxFactory, SandboxOptions, SandboxSpec};
use crate::health::HealthStore;
use crate::scan::{self, ScanOptions, ScanOutcome};

/// Where the catalog persists its state.
#[derive(Clone, Debug)]
pub struct CatalogPaths {
    /// The scan cache (`clap-scan.json`).
    pub cache: Option<PathBuf>,
    /// The blocklist (`plugin-blocklist.json`).
    pub blocklist: Option<PathBuf>,
}

/// One background (or foreground) rescan's result, for the `plugin_scan_progress` final summary
/// (T-804 item 1) and logging.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// Files scanned in a sandbox this time.
    pub scanned: usize,
    /// Files answered from the cache.
    pub cached: usize,
    /// Files that failed (not blocklisted — see `blocklisted_now`).
    pub failed: usize,
    /// Files newly blocklisted by this scan (crash/timeout).
    pub blocklisted_now: usize,
    /// Files skipped because they were already blocklisted.
    pub blocklisted: usize,
    /// Audio effects registered after this scan.
    pub effects: usize,
    /// Module ids that are new since the previous snapshot (hot-added, T-804 item 1).
    pub newly_registered: Vec<String>,
}

/// The plugin catalog: current known effect specs, the sandbox/scan options, custom folders, and
/// the registries to hot-add into.
pub struct PluginCatalog {
    sandbox_options: SandboxOptions,
    paths: CatalogPaths,
    custom_folders: Mutex<Vec<PathBuf>>,
    specs: RwLock<Vec<SandboxSpec>>,
    observers: Mutex<Vec<Weak<Registry>>>,
}

impl PluginCatalog {
    /// A catalog with no known plugins yet — call [`Self::load_cached`] then
    /// [`Self::rescan`]/[`Self::rescan_in_background`].
    pub fn new(sandbox_options: SandboxOptions, paths: CatalogPaths) -> Self {
        Self {
            sandbox_options,
            paths,
            custom_folders: Mutex::new(Vec::new()),
            specs: RwLock::new(Vec::new()),
            observers: Mutex::new(Vec::new()),
        }
    }

    /// The sandbox options this catalog scans and hosts with (binary, health store, …).
    pub fn sandbox_options(&self) -> &SandboxOptions {
        &self.sandbox_options
    }

    /// Sets the extra folders to scan in addition to the standard per-format paths (T-804 item
    /// 5, ADR-008 §6; `Settings.plugins.custom_folders`). Takes effect on the next scan.
    pub fn set_custom_folders(&self, folders: Vec<PathBuf>) {
        *self
            .custom_folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = folders;
    }

    /// The standard CLAP paths plus the configured custom folders.
    pub fn search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = scan::clap_search_paths();
        dirs.extend(
            self.custom_folders
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned(),
        );
        dirs
    }

    /// The audio effects known right now (a registry snapshot).
    pub fn specs(&self) -> Vec<SandboxSpec> {
        self.specs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Registers `registry` to receive hot-adds from future rescans (T-804 item 1). Held
    /// weakly — a dropped registry is simply skipped, never kept alive by the catalog.
    pub fn observe(&self, registry: &Arc<Registry>) {
        self.observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(registry));
    }

    fn scan_options(&self, force: bool) -> ScanOptions {
        let mut o = ScanOptions::new(self.sandbox_options.binary.clone());
        o.cache = self.paths.cache.clone();
        o.blocklist = self.paths.blocklist.clone();
        o.force = force;
        o
    }

    /// Fast, synchronous, start-up path (T-804 item 1): registers whatever the cache already
    /// knows, without spawning a single sandbox process. Never blocks on I/O beyond a directory
    /// walk and reading two small JSON files.
    pub fn load_cached(&self) {
        let files = scan::find_clap_files(&self.search_dirs());
        let specs = scan::cached_effect_specs(
            &files,
            self.paths.cache.as_deref(),
            self.paths.blocklist.as_deref(),
        );
        *self.specs.write().unwrap_or_else(PoisonError::into_inner) = specs;
    }

    fn factory_for(&self, spec: &SandboxSpec) -> Arc<dyn ModuleFactory> {
        Arc::new(SandboxFactory::new(
            spec.clone(),
            self.sandbox_options.clone(),
        ))
    }

    /// The full authoritative scan (blocklist-aware, cache-refreshing; `force`: ignore the
    /// cache, `plugins_rescan`'s "full" flag). Newly found effects (ids not in the previous
    /// snapshot) hot-add into every still-live [`Self::observe`]d registry. `on_progress` is
    /// [`scan::scan_clap_files_with`]'s callback (`plugin_scan_progress`, T-804 item 1).
    pub fn rescan(
        &self,
        force: bool,
        on_progress: impl Fn(usize, usize, &Path) + Sync,
    ) -> ScanSummary {
        let files = scan::find_clap_files(&self.search_dirs());
        let options = self.scan_options(force);
        let outcome: ScanOutcome = scan::scan_clap_files_with(&files, &options, on_progress);
        let new_specs = scan::effect_specs(&outcome);

        let previous_ids: std::collections::HashSet<String> =
            self.specs().into_iter().map(|s| s.descriptor.id).collect();
        let newly_registered: Vec<String> = new_specs
            .iter()
            .map(|s| s.descriptor.id.clone())
            .filter(|id| !previous_ids.contains(id))
            .collect();

        // Hot-add every current effect (not just the new ones) into observers: a replaced
        // module (same id, e.g. an updated plugin file) should also refresh in a live registry,
        // and `Registry::upsert` is a no-op-ish cheap replace for ids that are unchanged.
        let mut observers = self
            .observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        observers.retain(|w| w.strong_count() > 0);
        for spec in &new_specs {
            let factory = self.factory_for(spec);
            for w in observers.iter() {
                if let Some(registry) = w.upgrade() {
                    registry.upsert(factory.clone());
                }
            }
        }
        drop(observers);

        *self.specs.write().unwrap_or_else(PoisonError::into_inner) = new_specs.clone();
        ScanSummary {
            scanned: outcome.scanned,
            cached: outcome.cached,
            failed: outcome.failures.len(),
            blocklisted_now: outcome
                .failures
                .iter()
                .filter(|f| f.kind != scan::FailureKind::Other)
                .count(),
            blocklisted: outcome.blocklisted.len(),
            effects: new_specs.len(),
            newly_registered,
        }
    }

    /// [`Self::rescan`] on a new thread; `on_done` runs there too, after the scan finishes.
    pub fn rescan_in_background(
        self: &Arc<Self>,
        force: bool,
        on_progress: impl Fn(usize, usize, &Path) + Send + Sync + 'static,
        on_done: impl FnOnce(ScanSummary) + Send + 'static,
    ) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("plugin-scan".into())
            .spawn(move || {
                let summary = me.rescan(force, on_progress);
                on_done(summary);
            })
            .expect("couldn't start the plugin scan thread");
    }

    /// Every blocked file and why (T-804 item 6, `plugins_list`).
    pub fn blocklist_snapshot(&self) -> Vec<(PathBuf, crate::blocklist::BlockInfo)> {
        Blocklist::load(self.paths.blocklist.clone()).list()
    }

    /// Blocks `path` manually (`plugins_block`).
    pub fn block(&self, path: &Path) {
        Blocklist::load(self.paths.blocklist.clone()).block(path, BlockReason::Manual);
    }

    /// Unblocks `path` (`plugins_unblock`); `true` if it was blocked.
    pub fn unblock(&self, path: &Path) -> bool {
        Blocklist::load(self.paths.blocklist.clone()).unblock(path)
    }

    /// The runtime crash-flag store (T-804 item 4; shared with every sandboxed instance this
    /// catalog's [`Self::sandbox_options`] spawns).
    pub fn health(&self) -> Option<&Arc<HealthStore>> {
        self.sandbox_options.health.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::ModuleDescriptor;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "pv-catalog-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn catalog(dir: &Path) -> PluginCatalog {
        let options = SandboxOptions::new(PathBuf::from("/nonexistent/powervoice-sandbox"));
        PluginCatalog::new(
            options,
            CatalogPaths {
                cache: Some(dir.join("cache.json")),
                blocklist: Some(dir.join("blocklist.json")),
            },
        )
    }

    #[test]
    fn search_dirs_include_custom_folders() {
        let dir = temp_dir("dirs");
        let c = catalog(&dir);
        let custom = dir.join("my-plugins");
        c.set_custom_folders(vec![custom.clone()]);
        assert!(c.search_dirs().contains(&custom));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A custom folder is included in the search (T-804 item 5): `rescan` walks it and attempts
    /// to scan whatever it finds there, even a bogus file — proving the directory itself is
    /// searched, not silently skipped. (Scanning a *real* plugin found this way needs the real
    /// `powervoice-sandbox` binary; that end-to-end path is covered by
    /// `crates/sandbox/tests/clap_scan.rs`.)
    #[test]
    fn a_custom_folder_is_scanned() {
        let dir = temp_dir("custom-scan");
        let custom = dir.join("extra");
        std::fs::create_dir_all(&custom).unwrap();
        let plugin = custom.join("test.clap");
        std::fs::write(&plugin, b"not a real library").unwrap();

        let c = catalog(&dir);
        c.set_custom_folders(vec![custom.clone()]);
        let seen: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        let summary = c.rescan(false, |_, _, path| {
            seen.lock().unwrap().push(path.to_path_buf());
        });
        assert!(
            seen.lock().unwrap().contains(&plugin),
            "the custom folder wasn't searched"
        );
        assert_eq!(summary.scanned, 1);
        assert_eq!(
            summary.failed, 1,
            "a bogus binary is not a scannable plugin"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Manually hot-adding (as `rescan` would, once a real scan finds something) reaches every
    /// live observer and never panics over one that was dropped.
    #[test]
    fn observers_are_weak_and_a_dropped_one_is_never_touched() {
        let dir = temp_dir("hotadd");
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);
        drop(registry);
        // The dropped registry above must not be upgraded/touched; a second, live one still is.
        let registry2 = Arc::new(Registry::new());
        c.observe(&registry2);

        let spec = SandboxSpec {
            descriptor: ModuleDescriptor {
                id: "clap:com.x.y".into(),
                version: vox_module_api::Version::new(1, 0, 0),
                name: vox_module_api::LocalizedText::plain("Y"),
                vendor: "V".into(),
                description: vox_module_api::LocalizedText::plain(""),
                url: None,
                features: vec!["audio-effect".into()],
                state_format_version: 1,
                api_version: vox_module_api::MODULE_API_VERSION,
            },
            format: "clap".into(),
            plugin: "{}".into(),
        };
        let factory = c.factory_for(&spec);
        for w in c.observers.lock().unwrap().iter() {
            if let Some(r) = w.upgrade() {
                r.upsert(factory.clone());
            }
        }
        assert!(registry2.get("clap:com.x.y").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
