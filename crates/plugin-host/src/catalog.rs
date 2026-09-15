//! The plugin catalog (T-804, ADR-008 §6 Amendment 4): orchestrates the CLAP scanner so
//! start-up is instant (cached plugins only, no sandbox spawned) and a background rescan finds
//! everything else — new files, changed files, custom folders — hot-adding newly found effects
//! into every [`Registry`] that [`observe`](PluginCatalog::observe)s this catalog, so the
//! Add-module list updates without a restart (ADR-008 §6, T-804 item 1).
//!
//! Formats (T-806, T-807): CLAP, VST3 and LV2. Each format has its own H-29 search tiers (its per-user
//! install folder, its standard paths, then the shared custom folders); both formats' files go
//! through one scan (the format follows the extension), one cache and one blocklist. Module ids
//! are format-prefixed (`clap:`, `vst3:`), so the duplicate-id policy applies within a format.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use vox_module_api::ModuleFactory;
use vox_rack::Registry;

use crate::blocklist::{BlockReason, Blocklist};
use crate::factory::{SandboxFactory, SandboxOptions, SandboxSpec};
use crate::health::HealthStore;
use crate::install::{self, InstallError, UninstallError};
use crate::scan::{
    self, FoundPlugin, PluginFormat, ScanFailure, ScanOptions, ScanOutcome, ScannedPlugin,
    ShadowedPlugin,
};

/// Where the catalog persists its state.
#[derive(Clone, Debug)]
pub struct CatalogPaths {
    /// The scan cache (`plugin-scan.json`, H-34; migrated once from the old `clap-scan.json`).
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
    /// VST3 bundles indexed from their `moduleinfo.json`, without a sandbox (T-806).
    pub indexed: usize,
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

/// What the last scan learned about one effect beyond its descriptor (T-804's richer scan data,
/// shown by T-809's plugin manager). Both channel counts `0`: never instantiated, unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PluginDetails {
    pub param_count: u32,
    pub input_channels: u32,
    pub output_channels: u32,
}

impl From<&ScannedPlugin> for PluginDetails {
    fn from(p: &ScannedPlugin) -> Self {
        Self {
            param_count: p.param_count,
            input_channels: p.main_input_channels,
            output_channels: p.main_output_channels,
        }
    }
}

/// A successful "Install module…" (T-809).
#[derive(Clone, Debug)]
pub struct InstallReport {
    /// The installed file (in the per-user plugin folder).
    pub target: PathBuf,
    /// The audio effects it registered (hot-added into every observed registry).
    pub effects: Vec<SandboxSpec>,
    /// An older file with the same name was replaced (after the user confirmed).
    pub replaced: bool,
}

/// The plugin file a spec loads from (`None` for the test backend).
fn spec_path(spec: &SandboxSpec) -> Option<PathBuf> {
    spec.path()
}

/// Per-effect details for `outcome`, keyed by module id; the first file wins for a duplicate id,
/// like [`scan::effect_specs`].
fn details_of(outcome: &ScanOutcome) -> HashMap<String, PluginDetails> {
    let mut out = HashMap::new();
    for f in outcome
        .plugins
        .iter()
        .filter(|f| scan::is_effect(&f.plugin))
    {
        out.entry(scan::spec_for(&f.path, &f.plugin).descriptor.id)
            .or_insert_with(|| PluginDetails::from(&f.plugin));
    }
    out
}

/// The plugin catalog: current known effect specs, the sandbox/scan options, custom folders, and
/// the registries to hot-add into.
pub struct PluginCatalog {
    sandbox_options: SandboxOptions,
    paths: CatalogPaths,
    custom_folders: Mutex<Vec<PathBuf>>,
    specs: RwLock<Vec<SandboxSpec>>,
    details: RwLock<HashMap<String, PluginDetails>>,
    /// Duplicate ids that lost to another file (H-29, ADR-008 Amendment 5); `plugins_list`'s
    /// "Shadowed by …" rows.
    shadowed: RwLock<Vec<ShadowedPlugin>>,
    observers: Mutex<Vec<Weak<Registry>>>,
    /// Held for a whole rescan or install (T-809): they read and rewrite the same cache file and
    /// spec snapshot, so one never overwrites the other's result.
    scan_lock: Mutex<()>,
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
            details: RwLock::new(HashMap::new()),
            shadowed: RwLock::new(Vec::new()),
            observers: Mutex::new(Vec::new()),
            scan_lock: Mutex::new(()),
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

    /// The standard CLAP and VST3 paths plus the configured custom folders (flat, without
    /// duplicates; for callers that don't care about search priority). Scanning itself uses
    /// [`Self::search_tiers`], which keeps them apart.
    pub fn search_dirs(&self) -> Vec<PathBuf> {
        let mut seen = std::collections::HashSet::new();
        [PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Lv2]
            .into_iter()
            .flat_map(|f| self.search_tiers(f))
            .flatten()
            .filter(|d| seen.insert(d.clone()))
            .collect()
    }

    /// Every plugin file of every format, each format in its H-29 priority order.
    fn plugin_files(&self) -> Vec<PathBuf> {
        let mut files = scan::find_clap_files_ranked(&self.search_tiers(PluginFormat::Clap));
        files.extend(scan::find_vst3_files_ranked(
            &self.search_tiers(PluginFormat::Vst3),
        ));
        files.extend(scan::find_lv2_files_ranked(
            &self.search_tiers(PluginFormat::Lv2),
        ));
        files
    }

    /// [`Self::search_dirs`], grouped into H-29's duplicate-id priority tiers (ADR-008 Amendment
    /// 5): the user's own install folder (if the environment names one), then the standard CLAP
    /// paths (`$CLAP_PATH` first — unchanged since T-803/T-804: it's one tier, not split
    /// further), then the configured custom folders. [`scan::find_clap_files_ranked`] turns this
    /// straight into "first occurrence wins" priority.
    fn search_tiers(&self, format: PluginFormat) -> Vec<Vec<PathBuf>> {
        let mut tiers = Vec::new();
        let (install_dir, standard) = match format {
            PluginFormat::Clap => (install::user_clap_dir(), scan::clap_search_paths()),
            PluginFormat::Vst3 => (install::user_vst3_dir(), scan::vst3_search_paths()),
            PluginFormat::Lv2 => (install::user_lv2_dir(), scan::lv2_search_paths()),
        };
        if let Some(install_dir) = install_dir {
            tiers.push(vec![install_dir]);
        }
        tiers.push(standard);
        tiers.push(
            self.custom_folders
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        );
        tiers
    }

    /// The audio effects known right now (a registry snapshot).
    pub fn specs(&self) -> Vec<SandboxSpec> {
        self.specs
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What the last scan learned about `module_id` (ports, parameter count), if anything.
    pub fn details(&self, module_id: &str) -> Option<PluginDetails> {
        self.details
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module_id)
            .copied()
    }

    /// Duplicate-id losers from the last scan or cached load (H-29; `plugins_list`'s "Shadowed
    /// by …" rows).
    pub fn shadowed(&self) -> Vec<ShadowedPlugin> {
        self.shadowed
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
        let files = self.plugin_files();
        let outcome = scan::cached_outcome(
            &files,
            self.paths.cache.as_deref(),
            self.paths.blocklist.as_deref(),
        );
        let (specs, shadowed) = scan::effect_specs_with_shadows(&outcome);
        *self.specs.write().unwrap_or_else(PoisonError::into_inner) = specs;
        *self.details.write().unwrap_or_else(PoisonError::into_inner) = details_of(&outcome);
        *self
            .shadowed
            .write()
            .unwrap_or_else(PoisonError::into_inner) = shadowed;
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
        let _scanning = self
            .scan_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let files = self.plugin_files();
        let options = self.scan_options(force);
        let outcome: ScanOutcome = scan::scan_plugin_files_with(&files, &options, on_progress);
        let (new_specs, shadowed) = scan::effect_specs_with_shadows(&outcome);

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
        *self.details.write().unwrap_or_else(PoisonError::into_inner) = details_of(&outcome);
        *self
            .shadowed
            .write()
            .unwrap_or_else(PoisonError::into_inner) = shadowed;
        ScanSummary {
            scanned: outcome.scanned,
            cached: outcome.cached,
            indexed: outcome.indexed,
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

    /// "Install module…" (T-809): copies `source` into `dest_dir` (the per-user plugin folder —
    /// [`install::user_clap_dir`]; never a system folder), scans **only that file** in a sandbox,
    /// and on success records it in the scan cache and hot-adds its effects into every observed
    /// registry, so it's in Add Module at once. See [`install::install_file`] for collisions
    /// (`replace`), rollback and blocklisting.
    pub fn install(
        &self,
        source: &Path,
        dest_dir: &Path,
        replace: bool,
    ) -> Result<InstallReport, InstallError> {
        let binary = self.sandbox_options.binary.clone();
        self.install_with(source, dest_dir, replace, |path| {
            scan::scan_one(&binary, path, scan::SCAN_TIMEOUT)
        })
    }

    /// [`Self::install`] with the single-file scanner supplied (tests; `install` passes the
    /// sandboxed scan).
    pub fn install_with(
        &self,
        source: &Path,
        dest_dir: &Path,
        replace: bool,
        scan_file: impl FnOnce(&Path) -> Result<Vec<ScannedPlugin>, ScanFailure>,
    ) -> Result<InstallReport, InstallError> {
        let _scanning = self
            .scan_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut blocklist = Blocklist::load(self.paths.blocklist.clone());
        let installed =
            install::install_file(source, dest_dir, replace, &mut blocklist, scan_file)?;
        if let Some(cache) = &self.paths.cache {
            scan::cache_upsert(cache, &installed.target, &installed.plugins);
        }
        let outcome = ScanOutcome {
            plugins: installed
                .plugins
                .iter()
                .map(|plugin| FoundPlugin {
                    path: installed.target.clone(),
                    plugin: plugin.clone(),
                })
                .collect(),
            ..ScanOutcome::default()
        };
        let effects = scan::effect_specs(&outcome);
        {
            // The snapshot drops what this file offered before (a replaced version) and any
            // other file's effect with an id this one now provides (the newest install wins).
            let mut specs = self.specs.write().unwrap_or_else(PoisonError::into_inner);
            specs.retain(|s| {
                spec_path(s).as_deref() != Some(installed.target.as_path())
                    && !effects.iter().any(|e| e.descriptor.id == s.descriptor.id)
            });
            specs.extend(effects.iter().cloned());
            self.details
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(details_of(&outcome));
        }
        let mut observers = self
            .observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        observers.retain(|w| w.strong_count() > 0);
        for spec in &effects {
            let factory = self.factory_for(spec);
            for w in observers.iter() {
                if let Some(registry) = w.upgrade() {
                    registry.upsert(factory.clone());
                }
            }
        }
        Ok(InstallReport {
            target: installed.target,
            effects,
            replaced: installed.replaced,
        })
    }

    /// "Uninstall…" (H-29): removes `path` from `install_dir` (the per-user plugin folder —
    /// [`install::user_clap_dir`]; never a system folder — the caller resolves it, same as
    /// [`Self::install`]'s `dest_dir`) — refusing anything outside it
    /// ([`install::uninstall_file`]) — then drops whatever it provided from the catalog's
    /// snapshot, the scan cache, and every observed registry. An open document that already uses
    /// one of those modules keeps its slot: the registry's next `resolve` falls back to the
    /// "Missing module" placeholder (`vox_rack::Registry::resolve`), and the slot's own stored
    /// state is never touched by any of this. Held under the same [`Self::scan_lock`] as
    /// [`Self::install`]/[`Self::rescan`], so an uninstall never races either.
    pub fn uninstall(&self, path: &Path, install_dir: &Path) -> Result<(), UninstallError> {
        let _scanning = self
            .scan_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        install::uninstall_file(path, install_dir)?;

        let removed_ids: Vec<String> = {
            let mut specs = self.specs.write().unwrap_or_else(PoisonError::into_inner);
            let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut *specs)
                .into_iter()
                .partition(|s| spec_path(s).as_deref() != Some(path));
            *specs = kept;
            removed.into_iter().map(|s| s.descriptor.id).collect()
        };
        {
            let mut details = self.details.write().unwrap_or_else(PoisonError::into_inner);
            for id in &removed_ids {
                details.remove(id);
            }
        }
        if let Some(cache) = &self.paths.cache {
            scan::cache_remove(cache, path);
        }
        let mut observers = self
            .observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        observers.retain(|w| w.strong_count() > 0);
        for id in &removed_ids {
            for w in observers.iter() {
                if let Some(registry) = w.upgrade() {
                    registry.remove(id);
                }
            }
        }
        Ok(())
    }

    /// Clears `module_id`'s runtime crash flag (the plugin manager's "Clear crash warning").
    pub fn clear_flag(&self, module_id: &str) {
        if let Some(health) = self.health() {
            health.clear(module_id);
        }
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

    /// T-809: "Install module…" scans only the installed file, registers its effects in the
    /// catalog (with ports/params), records it in the cache, and hot-adds into live registries.
    #[test]
    fn install_registers_the_new_file_only() {
        let dir = temp_dir("install");
        let dest = dir.join("home").join(".clap");
        let source = dir.join("downloads").join("acme.clap");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"plugin").unwrap();
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);

        let mut calls = Vec::new();
        let report = c
            .install_with(&source, &dest, false, |p| {
                calls.push(p.to_path_buf());
                Ok(vec![crate::install::tests::effect("com.acme.deesser")])
            })
            .unwrap();
        assert_eq!(calls, vec![dest.join("acme.clap")]);
        assert_eq!(report.target, dest.join("acme.clap"));
        assert_eq!(report.effects.len(), 1);
        let id = "clap:com.acme.deesser";
        assert!(
            registry.get(id).is_some(),
            "hot-added into the live registry"
        );
        assert!(c.specs().iter().any(|s| s.descriptor.id == id));
        assert_eq!(
            c.details(id),
            Some(PluginDetails {
                param_count: 7,
                input_channels: 1,
                output_channels: 2
            })
        );
        // Known from the cache at the next start, without a sandbox.
        let cached = scan::cached_effect_specs(
            &[dest.join("acme.clap")],
            Some(&dir.join("cache.json")),
            None,
        );
        assert_eq!(cached.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// H-29's duplicate-id policy ("a user-installed file wins") applies at install time too, not
    /// just at the next rescan: installing a plugin whose id was already registered from some
    /// other file immediately supersedes it — the older spec, from a different path, is dropped
    /// the moment the new one lands, not left to double up until the next scan.
    #[test]
    fn installing_a_duplicate_id_supersedes_the_previously_registered_file() {
        let dir = temp_dir("install-dup");
        let dest = dir.join("home").join(".clap");
        let source = dir.join("downloads").join("acme.clap");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"plugin").unwrap();
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);

        // A same-id plugin is already registered, as if a previous scan had found it in a
        // standard or custom folder.
        let old_path = dir.join("usr-lib-clap").join("acme-old.clap");
        let old_spec = crate::factory::clap_spec(
            &old_path,
            &crate::install::tests::effect("com.acme.deesser"),
        );
        registry.upsert(c.factory_for(&old_spec));
        *c.specs.write().unwrap() = vec![old_spec];

        let report = c
            .install_with(&source, &dest, false, |_| {
                Ok(vec![crate::install::tests::effect("com.acme.deesser")])
            })
            .unwrap();
        assert_eq!(report.effects.len(), 1);

        let id = "clap:com.acme.deesser";
        let specs = c.specs();
        assert_eq!(
            specs.len(),
            1,
            "the old file's spec is gone, not doubled up"
        );
        assert_eq!(specs[0].descriptor.id, id);
        assert_eq!(
            spec_path(&specs[0]).as_deref(),
            Some(dest.join("acme.clap").as_path())
        );
        // The registry now resolves to the newly installed file too (hot-added over the old one).
        assert!(registry.get(id).is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// H-29 item 1: "Uninstall…" removes the file, the scan cache entry and the registry entry —
    /// and nothing else changes (the file's own directory is left otherwise intact).
    #[test]
    fn uninstall_removes_the_file_the_cache_entry_and_the_registry_entry() {
        let dir = temp_dir("uninstall");
        let dest = dir.join("home").join(".clap");
        let source = dir.join("downloads").join("acme.clap");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"plugin").unwrap();
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);
        c.install_with(&source, &dest, false, |_| {
            Ok(vec![crate::install::tests::effect("com.acme.deesser")])
        })
        .unwrap();
        let target = dest.join("acme.clap");
        let id = "clap:com.acme.deesser";
        assert!(target.exists());
        assert!(registry.get(id).is_some());

        c.uninstall(&target, &dest).unwrap();

        assert!(!target.exists(), "the file is removed");
        assert!(registry.get(id).is_none(), "dropped from the registry");
        assert!(
            !c.specs().iter().any(|s| s.descriptor.id == id),
            "dropped from the catalog snapshot"
        );
        assert_eq!(c.details(id), None, "dropped from the details cache");
        let cached = scan::cached_effect_specs(&[target], Some(&dir.join("cache.json")), None);
        assert!(cached.is_empty(), "dropped from the scan cache");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file outside the install folder is refused, up front — nothing about it changes: not
    /// the file, not the catalog snapshot, not the registry.
    #[test]
    fn uninstall_refuses_a_file_outside_the_install_folder() {
        let dir = temp_dir("uninstall-outside");
        let dest = dir.join("home").join(".clap");
        let outside = dir.join("usr-lib-clap").join("acme.clap");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"plugin").unwrap();
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);
        let spec =
            crate::factory::clap_spec(&outside, &crate::install::tests::effect("com.acme.deesser"));
        registry.upsert(c.factory_for(&spec));
        *c.specs.write().unwrap() = vec![spec];

        let err = c.uninstall(&outside, &dest).unwrap_err();
        assert_eq!(err, UninstallError::OutsideInstallFolder);
        assert!(outside.exists());
        assert!(registry.get("clap:com.acme.deesser").is_some());
        assert_eq!(c.specs().len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// T-806: "Install module…" of a `.vst3` bundle registers `vst3:<class id>` effects (with
    /// their details) and "Uninstall…" removes the bundle directory and the registrations.
    #[test]
    fn a_vst3_bundle_installs_and_uninstalls() {
        let dir = temp_dir("vst3-install");
        let dest = dir.join("home").join(".vst3");
        let bundle = crate::scan::tests::vst3_bundle(&dir.join("downloads"), "Acme", None);
        let c = catalog(&dir);
        let registry = Arc::new(Registry::new());
        c.observe(&registry);
        let cid = "84E8DE5F92554F5396FAE4133C935A18";
        let report = c
            .install_with(&bundle, &dest, false, |_| {
                Ok(vec![crate::install::tests::effect(cid)])
            })
            .unwrap();
        let target = dest.join("Acme.vst3");
        assert_eq!(report.target, target);
        let id = format!("vst3:{cid}");
        assert_eq!(report.effects[0].descriptor.id, id);
        assert_eq!(report.effects[0].format, "vst3");
        assert!(registry.get(&id).is_some());
        assert_eq!(c.details(&id).map(|d| d.param_count), Some(7));
        assert_eq!(spec_path(&c.specs()[0]).as_deref(), Some(target.as_path()));

        c.uninstall(&target, &dest).unwrap();
        assert!(!target.exists());
        assert!(registry.get(&id).is_none());
        assert!(c.specs().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_dirs_cover_both_formats() {
        let dir = temp_dir("dirs-formats");
        let c = catalog(&dir);
        let dirs = c.search_dirs();
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert!(dirs.contains(&PathBuf::from("/usr/lib/clap")));
            assert!(dirs.contains(&PathBuf::from("/usr/lib/vst3")));
            assert!(dirs.contains(&PathBuf::from("/usr/lib/lv2")));
        }
        let mut unique = dirs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), dirs.len(), "no duplicates");
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
