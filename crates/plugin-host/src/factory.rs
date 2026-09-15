//! [`SandboxFactory`]: registers a sandboxed plugin in the rack's module registry.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use vox_module_api::{
    LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory,
    Version, features,
};
use vox_sandbox_ipc::WaitBudget;
use vox_sandbox_ipc::protocol::{ClapPluginRef, ScannedPlugin, Vst3PluginRef};

use crate::health::HealthStore;
use crate::proxy::ProxyModule;
use crate::sandbox::{Sandbox, SandboxFault};

/// The built-in test backend's format name.
pub const TEST_FORMAT: &str = "test";
/// The CLAP backend's format name (T-803); module ids are `clap:<plugin id>`.
pub const CLAP_FORMAT: &str = "clap";
/// The VST3 backend's format name (T-806); module ids are `vst3:<class id>` (the processor's
/// 32 hex digits, ADR-005 §2).
pub const VST3_FORMAT: &str = "vst3";

/// What a sandboxed module is: its registry descriptor and how the sandbox loads it.
#[derive(Clone, Debug)]
pub struct SandboxSpec {
    /// Registry identity (`"<format>:<id>"`, ADR-005 §2), name, features.
    pub descriptor: ModuleDescriptor,
    /// The sandbox backend (`"test"`; T-803+: `"clap"`, …).
    pub format: String,
    /// The backend's plugin reference.
    pub plugin: String,
}

impl SandboxSpec {
    /// The plugin file or bundle this spec loads from (`None` for the test backend or an
    /// unreadable reference).
    pub fn path(&self) -> Option<PathBuf> {
        match self.format.as_str() {
            CLAP_FORMAT => ClapPluginRef::parse(&self.plugin)
                .ok()
                .map(|r| PathBuf::from(r.path)),
            VST3_FORMAT => Vst3PluginRef::parse(&self.plugin)
                .ok()
                .map(|r| PathBuf::from(r.path)),
            _ => None,
        }
    }
}

/// Editor-side knobs of the sandbox.
#[derive(Clone, Debug)]
pub struct SandboxOptions {
    /// The `powervoice-sandbox` executable.
    pub binary: PathBuf,
    /// Smallest realtime transport block `B` (ADR-008 §2); grown to the observed callback size.
    pub min_block: u32,
    /// Realtime wait budget per block (T-801 default: 25 % of the block period).
    pub realtime_wait: WaitBudget,
    /// No heartbeat progress for this long while fed = hang (ADR-008 §5: 250 ms).
    pub hang_timeout: Duration,
    /// Offline: how long one block may take before the render aborts.
    pub offline_timeout: Duration,
    /// Control requests (load, activate, state; ADR-008 §4: 5 s).
    pub request_timeout: Duration,
    /// Runtime-crash flag counters (T-804, ADR-008 §5): incremented once per fault, the first
    /// time a sandboxed instance of a module faults. `None` (the default) records nothing —
    /// tests and the CLI that don't care about the plugin manager's flags.
    pub health: Option<Arc<HealthStore>>,
}

impl SandboxOptions {
    /// Defaults for `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            min_block: 256,
            realtime_wait: WaitBudget::FractionOfBlock(0.25),
            hang_timeout: Duration::from_millis(250),
            offline_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
            health: None,
        }
    }

    /// The sandbox next to the running executable (`POWERVOICE_SANDBOX_BIN` overrides).
    pub fn beside_current_exe() -> Self {
        let binary = std::env::var_os("POWERVOICE_SANDBOX_BIN")
            .map(PathBuf::from)
            .or_else(|| {
                let exe = std::env::current_exe().ok()?;
                Some(exe.parent()?.join(BINARY_NAME))
            })
            .unwrap_or_else(|| PathBuf::from(BINARY_NAME));
        Self::new(binary)
    }
}

/// The sandbox executable's file name.
pub const BINARY_NAME: &str = if cfg!(windows) {
    "powervoice-sandbox.exe"
} else {
    "powervoice-sandbox"
};

/// A live sandbox of a factory (diagnostics, tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SandboxInstance {
    /// Process id.
    pub pid: u32,
    /// Its fault, if any.
    pub fault: Option<SandboxFault>,
}

/// A `ModuleFactory` whose instances run in sandbox processes ([`ProxyModule`]).
pub struct SandboxFactory {
    spec: Arc<SandboxSpec>,
    options: Arc<SandboxOptions>,
    /// Largest realtime callback any instance saw (the next instance's `B`).
    block_hint: Arc<AtomicU32>,
    instances: Mutex<Vec<Weak<Sandbox>>>,
}

impl SandboxFactory {
    /// A factory for `spec`.
    pub fn new(spec: SandboxSpec, options: SandboxOptions) -> Self {
        Self {
            block_hint: Arc::new(AtomicU32::new(options.min_block)),
            spec: Arc::new(spec),
            options: Arc::new(options),
            instances: Mutex::new(Vec::new()),
        }
    }

    /// The sandboxes of instances that still exist (their proxy hasn't been dropped).
    pub fn live_instances(&self) -> Vec<SandboxInstance> {
        let mut list = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        list.retain(|w| w.strong_count() > 0);
        list.iter()
            .filter_map(Weak::upgrade)
            .map(|s| SandboxInstance {
                pid: s.pid,
                fault: s.fault(),
            })
            .collect()
    }
}

impl ModuleFactory for SandboxFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.spec.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        let proxy = ProxyModule::spawn(
            self.spec.clone(),
            self.options.clone(),
            self.block_hint.clone(),
        )?;
        if let Some(s) = proxy.sandbox() {
            self.instances
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Arc::downgrade(s));
        }
        Ok(Box::new(proxy))
    }

    /// A sandbox spawn plus a plugin load: the live rack creates these off its control thread
    /// and shows the slot as loading (T-803).
    fn loads_async(&self) -> bool {
        true
    }
}

/// A CLAP plugin's spec (T-803): module id `clap:<plugin id>`, the plugin's name, vendor,
/// description, URL and CLAP features, version parsed leniently; loaded from `path`.
pub fn clap_spec(path: &Path, plugin: &ScannedPlugin) -> SandboxSpec {
    SandboxSpec {
        descriptor: ModuleDescriptor {
            id: format!("{CLAP_FORMAT}:{}", plugin.id),
            version: Version::parse_lenient(&plugin.version),
            name: LocalizedText::plain(&plugin.name),
            vendor: plugin.vendor.clone(),
            description: LocalizedText::plain(&plugin.description),
            url: plugin.url.clone(),
            features: plugin.features.clone(),
            state_format_version: 1,
            api_version: MODULE_API_VERSION,
        },
        format: CLAP_FORMAT.into(),
        plugin: ClapPluginRef {
            path: path.to_string_lossy().into_owned(),
            id: plugin.id.clone(),
        }
        .to_reference(),
    }
}

/// A VST3 plugin's spec (T-806): module id `vst3:<class id>`, the class's name, vendor, version
/// (parsed leniently) and features (from its sub-categories); loaded from the bundle `path`.
pub fn vst3_spec(path: &Path, plugin: &ScannedPlugin) -> SandboxSpec {
    SandboxSpec {
        descriptor: ModuleDescriptor {
            id: format!("{VST3_FORMAT}:{}", plugin.id),
            version: Version::parse_lenient(&plugin.version),
            name: LocalizedText::plain(&plugin.name),
            vendor: plugin.vendor.clone(),
            description: LocalizedText::plain(&plugin.description),
            url: plugin.url.clone(),
            features: plugin.features.clone(),
            state_format_version: 1,
            api_version: MODULE_API_VERSION,
        },
        format: VST3_FORMAT.into(),
        plugin: Vst3PluginRef {
            path: path.to_string_lossy().into_owned(),
            cid: plugin.id.clone(),
        }
        .to_reference(),
    }
}

/// A test-backend spec: module id `test:<id>`, backend plugin reference `plugin`.
pub fn test_spec(id: &str, plugin: &str, name: &str) -> SandboxSpec {
    SandboxSpec {
        descriptor: ModuleDescriptor {
            id: format!("{TEST_FORMAT}:{id}"),
            version: Version::new(1, 0, 0),
            name: LocalizedText::plain(name),
            vendor: "PowerVoice".into(),
            description: LocalizedText::plain(
                "Built-in test plugin, hosted out of process (developer builds)",
            ),
            url: None,
            features: vec![features::AUDIO_EFFECT.into(), features::MONO.into()],
            state_format_version: 1,
            api_version: MODULE_API_VERSION,
        },
        format: TEST_FORMAT.into(),
        plugin: plugin.into(),
    }
}

/// The developer Add-module entries (T-802, behind a dev flag until T-803+ ship real formats):
/// a sandboxed Gain, and crash / hang plugins that fail after ~8 s of audio at 48 kHz.
pub fn test_factories(options: &SandboxOptions) -> Vec<Arc<dyn ModuleFactory>> {
    [
        ("gain", "gain", "Gain (sandboxed test)"),
        ("crash", "crash?after=1500", "Crash test (sandboxed)"),
        ("hang", "hang?after=1500", "Hang test (sandboxed)"),
    ]
    .into_iter()
    .map(|(id, plugin, name)| {
        Arc::new(SandboxFactory::new(
            test_spec(id, plugin, name),
            options.clone(),
        )) as Arc<dyn ModuleFactory>
    })
    .collect()
}
