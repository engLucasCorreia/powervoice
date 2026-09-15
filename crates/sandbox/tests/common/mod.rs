//! Shared helpers for the sandbox process tests (T-802).
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vox_module_api::{ModuleFactory, ModuleRef, ModuleState};
use vox_modules::Gain;
use vox_plugin_host::{SandboxFactory, SandboxOptions, test_spec};
use vox_rack::{RackModel, Registry, SlotModel};
use vox_sandbox_ipc::WaitBudget;

pub const SANDBOX: &str = env!("CARGO_BIN_EXE_powervoice-sandbox");
pub const RATE: f64 = 48_000.0;

/// Realtime waits long enough that a loaded machine never misses (bit-exact comparisons).
pub fn exact_options() -> SandboxOptions {
    let mut o = SandboxOptions::new(SANDBOX);
    o.realtime_wait = WaitBudget::Fixed(Duration::from_secs(5));
    o
}

/// The shipping defaults (25 % of the block period).
pub fn default_options() -> SandboxOptions {
    SandboxOptions::new(SANDBOX)
}

pub fn factory(id: &str, plugin: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    Arc::new(SandboxFactory::new(test_spec(id, plugin, id), options))
}

/// The built-ins plus `extra`.
pub fn registry_with(extra: &[Arc<SandboxFactory>]) -> Arc<Registry> {
    let mut all = vox_modules::builtin_factories();
    all.extend(extra.iter().map(|f| f.clone() as Arc<dyn ModuleFactory>));
    Arc::new(Registry::with_factories(all).unwrap())
}

/// One slot of `module` (`id@version`) at `gain_db`.
pub fn gain_slot(module: &str, gain_db: f64) -> SlotModel {
    let r: ModuleRef = module.parse().unwrap();
    SlotModel::new(&r, false, &Gain::state_with_gain_db(gain_db))
}

pub fn model(slots: Vec<SlotModel>) -> RackModel {
    RackModel { slots }
}

pub fn state_gain_db(state: &ModuleState) -> f64 {
    state.params["gain_db"]
}

/// `/proc/<pid>` exists (a running process or an unreaped zombie).
pub fn pid_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

pub fn kill9(pid: u32) {
    // SAFETY: plain signal to a process id we spawned.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    loop {
        if f() {
            return true;
        }
        if t0.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Named POSIX segments of the sandbox transport (`/dev/shm/pvs.*`).
pub fn pvs_shm_objects() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev/shm")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("pvs."))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").map_or(0, |d| d.count())
}

/// [`fd_count`], but only once it has stopped changing for `settle` — a bounded poll, not a fixed
/// sleep (H-34). A dropped proxy's `live_instances().is_empty()` only means the proxy removed its
/// own bookkeeping; the sandbox's pipes/shm close and the watchdog reaps its process slightly
/// later, on other threads. A baseline taken right after `is_empty()` can still include those
/// about-to-close descriptors, so a later, truly-settled count reads as *fewer* than the
/// baseline ("leaked: 5 → 4" — the count went down, not up) even though nothing actually leaked.
/// Waiting here for the count to hold steady avoids catching it mid-teardown. Gives up and
/// returns the last-seen count after `timeout` (the caller's own leak assertion still catches a
/// real leak either way; this only avoids a baseline taken too early).
pub fn stable_fd_count(timeout: Duration) -> usize {
    const SETTLE: Duration = Duration::from_millis(50);
    let deadline = Instant::now() + timeout;
    let mut last = fd_count();
    let mut unchanged_since = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(5));
        let now = fd_count();
        if now != last {
            last = now;
            unchanged_since = Instant::now();
        } else if unchanged_since.elapsed() >= SETTLE {
            return last;
        }
        if Instant::now() >= deadline {
            return last;
        }
    }
}

/// Deterministic noise in [−0.5, 0.5).
pub fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / 16_777_216.0 - 0.5
        })
        .collect()
}

pub fn sine(n: usize, freq_hz: f64, amp: f64) -> Vec<f32> {
    (0..n)
        .map(|t| (amp * (std::f64::consts::TAU * freq_hz * t as f64 / RATE).sin()) as f32)
        .collect()
}

pub fn max_step(y: &[f32]) -> f32 {
    y.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

pub fn assert_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    if let Some(i) = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!("{what}: sample {i}: got {} want {}", got[i], want[i]);
    }
}

// --- T-803: the test CLAP plugin -------------------------------------------------------------

/// The test CLAP plugin (`vox-test-clap`'s `cdylib`): cargo builds it next to the test binaries
/// because this crate dev-depends on it (`target/<profile>/deps/`), or uplifts it one level up
/// in a workspace build.
pub fn test_clap_library() -> std::path::PathBuf {
    let dir = Path::new(SANDBOX)
        .parent()
        .expect("sandbox binary directory");
    let name = format!(
        "{}vox_test_clap{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [dir.join("deps").join(&name), dir.join(&name)]
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("{name} not found next to {SANDBOX}"))
}

/// A scanned-plugin record for test plugin `id`.
pub fn test_clap_plugin(id: &str) -> vox_plugin_host::scan::ScannedPlugin {
    vox_plugin_host::scan::ScannedPlugin {
        id: id.to_owned(),
        name: format!("Test {id}"),
        vendor: "PowerVoice".into(),
        version: "1.2.3".into(),
        description: String::new(),
        url: None,
        features: vec!["audio-effect".into(), "utility".into()],
        ..vox_plugin_host::scan::ScannedPlugin::default()
    }
}

/// A factory for test CLAP plugin `id` loaded from `path`.
pub fn clap_factory_at(path: &Path, id: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    Arc::new(SandboxFactory::new(
        vox_plugin_host::clap_spec(path, &test_clap_plugin(id)),
        options,
    ))
}

/// A factory for test CLAP plugin `id` (module id `clap:<id>`).
pub fn clap_factory(id: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    clap_factory_at(&test_clap_library(), id, options)
}

/// A state setting the CLAP gain parameter (key `p0`).
pub fn clap_gain_state(gain_db: f64) -> ModuleState {
    ModuleState {
        format_version: 1,
        params: std::collections::BTreeMap::from([("p0".to_owned(), gain_db)]),
        blob: None,
    }
}

/// A temporary directory removed on drop (the `/tmp` quota: never leak test dirs).
pub struct TempDir(pub std::path::PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "pv-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// --- T-806: the test VST3 plugin -------------------------------------------------------------

/// The test VST3 plugin's library (`vox-test-vst3`'s `cdylib`, built next to the tests like the
/// CLAP one). A regular file is loaded as the module binary itself.
pub fn test_vst3_library() -> std::path::PathBuf {
    let dir = Path::new(SANDBOX)
        .parent()
        .expect("sandbox binary directory");
    let name = format!(
        "{}vox_test_vst3{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [dir.join("deps").join(&name), dir.join(&name)]
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("{name} not found next to {SANDBOX}"))
}

/// A real bundle `<dir>/<name>.vst3` (the platform layout) holding a copy of the test plugin.
pub fn make_vst3_bundle(dir: &Path, name: &str) -> std::path::PathBuf {
    let bundle = dir.join(format!("{name}.vst3"));
    let arch = bundle
        .join("Contents")
        .join(vox_sandbox_ipc::vst3::arch_folder());
    std::fs::create_dir_all(&arch).unwrap();
    let file = if cfg!(target_os = "macos") {
        name.to_owned()
    } else if cfg!(windows) {
        format!("{name}.vst3")
    } else {
        format!("{name}.so")
    };
    std::fs::copy(test_vst3_library(), arch.join(file)).unwrap();
    bundle
}

/// A scanned-plugin record for test VST3 class `cid`.
pub fn test_vst3_plugin(cid: &str) -> vox_plugin_host::scan::ScannedPlugin {
    vox_plugin_host::scan::ScannedPlugin {
        id: cid.to_owned(),
        name: format!("Test {cid}"),
        vendor: "PowerVoice".into(),
        version: "1.2.3".into(),
        description: String::new(),
        url: None,
        features: vec!["audio-effect".into(), "utility".into()],
        ..vox_plugin_host::scan::ScannedPlugin::default()
    }
}

/// A factory for test VST3 class `cid` loaded from `path`.
pub fn vst3_factory_at(path: &Path, cid: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    Arc::new(SandboxFactory::new(
        vox_plugin_host::vst3_spec(path, &test_vst3_plugin(cid)),
        options,
    ))
}

/// A factory for test VST3 class `cid` (module id `vst3:<cid>`).
pub fn vst3_factory(cid: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    vst3_factory_at(&test_vst3_library(), cid, options)
}

/// A state setting the VST3 gain parameter (key `p0`, normalized) to `gain_db`.
pub fn vst3_gain_state(gain_db: f64) -> ModuleState {
    ModuleState {
        format_version: 1,
        params: std::collections::BTreeMap::from([(
            "p0".to_owned(),
            vox_test_vst3::gain_db_to_normalized(gain_db),
        )]),
        blob: None,
    }
}

// --- T-807: the test LV2 plugin --------------------------------------------------------------

/// Whether lilv loads here (the LV2 tests pass with a note when it doesn't: nothing to host
/// through).
pub fn lv2_available() -> bool {
    match powervoice_sandbox::lv2::available() {
        Ok(()) => true,
        Err(e) => {
            eprintln!("LV2 test skipped: {e}");
            false
        }
    }
}

/// The test LV2 plugin's library (`vox-test-lv2`'s `cdylib`, built next to the tests).
pub fn test_lv2_library() -> std::path::PathBuf {
    let dir = Path::new(SANDBOX)
        .parent()
        .expect("sandbox binary directory");
    let name = format!(
        "{}vox_test_lv2{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [dir.join("deps").join(&name), dir.join(&name)]
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("{name} not found next to {SANDBOX}"))
}

/// A real bundle `<dir>/<name>.lv2` (TTL + a copy of the test plugin).
pub fn make_lv2_bundle(dir: &Path, name: &str) -> std::path::PathBuf {
    vox_test_lv2::write_bundle(dir, name, &test_lv2_library()).unwrap()
}

/// A scanned-plugin record for test LV2 plugin `uri`.
pub fn test_lv2_plugin(uri: &str) -> vox_plugin_host::scan::ScannedPlugin {
    vox_plugin_host::scan::ScannedPlugin {
        id: uri.to_owned(),
        name: format!("Test {uri}"),
        vendor: "PowerVoice".into(),
        version: "2.3".into(),
        description: String::new(),
        url: None,
        features: vec!["audio-effect".into(), "utility".into()],
        ..vox_plugin_host::scan::ScannedPlugin::default()
    }
}

/// A factory for test LV2 plugin `uri` in `bundle` (module id `lv2:<uri>`).
pub fn lv2_factory(bundle: &Path, uri: &str, options: SandboxOptions) -> Arc<SandboxFactory> {
    Arc::new(SandboxFactory::new(
        vox_plugin_host::lv2_spec(bundle, &test_lv2_plugin(uri)),
        options,
    ))
}

/// A state setting the LV2 gain port (key `gain`, dB).
pub fn lv2_gain_state(gain_db: f64) -> ModuleState {
    ModuleState {
        format_version: 1,
        params: std::collections::BTreeMap::from([("gain".to_owned(), gain_db)]),
        blob: None,
    }
}
