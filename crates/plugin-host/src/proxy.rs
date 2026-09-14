//! [`ProxyModule`]: an ADR-005 `Module` whose processing happens in a sandbox process.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use vox_module_api::{
    ActivateConfig, AdapterHealth, ChannelLayout, Extension, ExtensionId, HostRequest, Module,
    ModuleDescriptor, ModuleError, ModuleState, ParamEvent, ParamFlags, ParamGroup, ParamId,
    ParamInfo, ParamText, ProcessContext, ProcessMode, ProcessStatus, StateError, Tail,
    validate_schema,
};
use vox_sandbox_ipc::layout::Layout;
use vox_sandbox_ipc::protocol::{
    PROTOCOL_VERSION, ParamValue, PluginInfo, RequestBody, ResponseBody,
};
use vox_sandbox_ipc::{
    BlockOutcome, Channel, ChannelConfig, EventKind, HostEnd, HostOptions, SharedRegion, WireEvent,
};

use crate::factory::{SandboxOptions, SandboxSpec};
use crate::sandbox::{FaultCell, Sandbox};
use crate::state::{self, StateHeader};

/// Largest transport block `B` (= the rack's offline block): the segment is sized for it once,
/// at spawn, and re-initialised for the actual `B` at every activation.
pub const MAX_TRANSPORT_BLOCK: u32 = 4096;

/// `Deactivate` answers quickly (the audio thread idles in ≤ 20 ms); a longer wait means the
/// plugin is stuck.
const DEACTIVATE_TIMEOUT: Duration = Duration::from_secs(1);

/// The proxy's [`AdapterHealth`] handle (outlives the process).
struct Health(Arc<FaultCell>);

impl AdapterHealth for Health {
    fn fault(&self) -> Option<String> {
        self.0.get().map(|f| f.reason().to_owned())
    }
}

/// Parameter text requests answer within this, or the host falls back to its own formatting.
const TEXT_TIMEOUT: Duration = Duration::from_millis(500);

/// The proxy's [`ParamText`] handle (T-803): the plugin's own text, one control round trip per
/// call (a batch for `values_to_text`). Holds the sandbox weakly: after the proxy is dropped it
/// answers `None`.
struct Texts(Weak<Sandbox>);

impl Texts {
    fn call(&self, body: RequestBody) -> Option<ResponseBody> {
        let s = self.0.upgrade().filter(|s| s.is_usable())?;
        s.call_soft(body, TEXT_TIMEOUT).ok().map(|(b, _)| b)
    }
}

impl ParamText for Texts {
    fn values_to_text(&self, values: &[(ParamId, f64)]) -> Vec<Option<String>> {
        let body = RequestBody::ParamTexts {
            values: values
                .iter()
                .map(|&(id, value)| ParamValue { id, value })
                .collect(),
        };
        match self.call(body) {
            Some(ResponseBody::Texts { texts }) if texts.len() == values.len() => texts,
            _ => vec![None; values.len()],
        }
    }

    fn text_to_value(&self, id: ParamId, text: &str) -> Option<f64> {
        match self.call(RequestBody::TextToParam {
            id,
            text: text.to_owned(),
        })? {
            ResponseBody::Value { value } => value.filter(|v| v.is_finite()),
            _ => None,
        }
    }
}

struct Active {
    host: HostEnd,
    mode: ProcessMode,
    block: u32,
    plugin_latency: u32,
    tail: Tail,
    restart_requested: bool,
}

/// A sandboxed plugin instance as a rack module (ADR-008 `ProxyModule`).
///
/// - `params()`/`groups()`: the plugin's schema, mirrored at load; `param_value` is the mirror
///   (updated by `load_state`, by the events `process` forwards and by plugin reports).
/// - `process()`: forwards the block's events (absolute positions) and audio through the
///   T-801 [`HostEnd`] — no allocation, no lock, at most one wake and one bounded wait; the
///   output is delayed by `B`. Returns `ProcessStatus::Error` once the channel is bypassed after
///   a fault (and, offline, when a block missed its deadline).
/// - `latency_samples()` = `B` + the plugin's latency.
/// - State: parameter values plus the plugin's state wrapped with its identity
///   ([`crate::state`]); on load the blob goes first, then the values.
/// - Dropping it retires its sandbox process through the watchdog (never blocks).
pub struct ProxyModule {
    descriptor: ModuleDescriptor,
    spec: Arc<SandboxSpec>,
    options: Arc<SandboxOptions>,
    block_hint: Arc<AtomicU32>,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    values: Vec<f64>,
    plugin_version: String,
    sandbox: Option<Arc<Sandbox>>,
    fault: Arc<FaultCell>,
    health: Arc<dyn AdapterHealth>,
    /// The plugin's own parameter text, when it has one (T-803).
    text: Option<Arc<dyn ParamText>>,
    active: Option<Active>,
    last_state: Mutex<Option<Vec<u8>>>,
}

fn external(what: &str, e: impl std::fmt::Display) -> ModuleError {
    ModuleError::External(format!("{what}: {e}"))
}

impl ProxyModule {
    /// Spawns a sandbox for `spec`, handshakes and loads the plugin.
    pub(crate) fn spawn(
        spec: Arc<SandboxSpec>,
        options: Arc<SandboxOptions>,
        block_hint: Arc<AtomicU32>,
    ) -> Result<Self, ModuleError> {
        let layout = Layout::new(MAX_TRANSPORT_BLOCK, ChannelConfig::DEFAULT_EVENT_CAPACITY)
            .map_err(|e| external("sandbox segment", e))?;
        let region =
            SharedRegion::create(layout.total_size).map_err(|e| external("sandbox segment", e))?;
        let mut cmd = Command::new(&options.binary);
        let handle = region.share_with(&mut cmd);
        cmd.arg("--shm")
            .arg(handle)
            .arg("--host-pid")
            .arg(std::process::id().to_string());
        let sandbox = Sandbox::spawn(cmd, region, &spec.descriptor.name.text)
            .map_err(ModuleError::External)?;
        let info = match Self::handshake(&sandbox, &spec, &options) {
            Ok(info) => info,
            Err(e) => {
                crate::watchdog::retire(sandbox);
                return Err(ModuleError::External(e));
            }
        };
        let values = info
            .params
            .iter()
            .map(|p| {
                info.values
                    .iter()
                    .find(|v| v.id == p.id)
                    .map_or(p.default, |v| v.value)
            })
            .collect();
        let fault = sandbox.fault.clone();
        let text = info
            .param_text
            .then(|| Arc::new(Texts(Arc::downgrade(&sandbox))) as Arc<dyn ParamText>);
        Ok(Self {
            descriptor: spec.descriptor.clone(),
            spec,
            options,
            block_hint,
            params: info.params,
            groups: info.groups,
            values,
            plugin_version: info.version,
            sandbox: Some(sandbox),
            health: Arc::new(Health(fault.clone())),
            fault,
            text,
            active: None,
            last_state: Mutex::new(None),
        })
    }

    fn handshake(
        sandbox: &Sandbox,
        spec: &SandboxSpec,
        options: &SandboxOptions,
    ) -> Result<PluginInfo, String> {
        let t = options.request_timeout;
        match sandbox.call(
            RequestBody::Hello {
                protocol: PROTOCOL_VERSION,
            },
            Vec::new(),
            t,
        ) {
            Ok((ResponseBody::Hello { protocol, .. }, _)) if protocol == PROTOCOL_VERSION => {}
            Ok(_) => return Err("the plugin sandbox speaks another protocol version".into()),
            Err(e) => return Err(format!("the plugin sandbox didn't answer: {e}")),
        }
        // Named segments (macOS): both sides have it open now.
        sandbox.region.unlink();
        let info = match sandbox.call(
            RequestBody::Load {
                backend: spec.format.clone(),
                plugin: spec.plugin.clone(),
            },
            Vec::new(),
            t,
        ) {
            Ok((ResponseBody::Loaded(info), _)) => info,
            Ok(_) => return Err("unexpected answer to Load".into()),
            Err(e) => return Err(format!("couldn't load the plugin: {e}")),
        };
        validate_schema(&info.params, &info.groups)
            .map_err(|e| format!("invalid parameter schema: {e}"))?;
        Ok(info)
    }

    /// The sandbox (the factory tracks instances; tests find the process id through it).
    pub(crate) fn sandbox(&self) -> Option<&Arc<Sandbox>> {
        self.sandbox.as_ref()
    }

    fn usable_sandbox(&self) -> Option<&Arc<Sandbox>> {
        self.sandbox.as_ref().filter(|s| s.is_usable())
    }

    fn not_running(&self) -> String {
        match self.fault.get() {
            Some(f) => format!("the plugin sandbox {}", f.reason()),
            None => "the plugin sandbox is not running".to_owned(),
        }
    }

    /// `B` for `config`: offline = the render's block; realtime = the host's observed callback
    /// size (at least `min_block`), rounded up to a power of two, capped by `max_block`.
    fn choose_block(&self, config: &ActivateConfig) -> u32 {
        let cap = config.max_block.clamp(1, MAX_TRANSPORT_BLOCK);
        match config.mode {
            ProcessMode::Offline => cap,
            ProcessMode::Realtime => self
                .block_hint
                .load(Ordering::Relaxed)
                .max(self.options.min_block)
                .max(1)
                .next_power_of_two()
                .min(cap),
        }
    }

    fn header(&self) -> StateHeader {
        StateHeader {
            format: self.spec.format.clone(),
            plugin: self.spec.plugin.clone(),
            id: self.descriptor.id.clone(),
            version: self.plugin_version.clone(),
        }
    }

    /// The plugin's current state bytes (the last good copy if the sandbox can't answer).
    fn fetch_state(&self) -> Option<Vec<u8>> {
        let mut last = self
            .last_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = self.usable_sandbox()
            && let Ok((ResponseBody::State, data)) = s.call(
                RequestBody::SaveState,
                Vec::new(),
                self.options.request_timeout,
            )
        {
            *last = Some(data);
        }
        last.clone()
    }
}

impl Module for ProxyModule {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn groups(&self) -> &[ParamGroup] {
        &self.groups
    }

    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        if config.layout != ChannelLayout::MONO {
            return Err(ModuleError::Unsupported(format!(
                "layout {:?}",
                config.layout
            )));
        }
        self.deactivate();
        let block = self.choose_block(config);
        let Some(sandbox) = self.usable_sandbox().cloned() else {
            return Err(ModuleError::External(self.not_running()));
        };
        let region = sandbox
            .region
            .duplicate()
            .map_err(|e| external("sandbox segment", e))?;
        let channel =
            Channel::create_in(region, ChannelConfig::pipelined(config.sample_rate, block))
                .map_err(|e| ModuleError::Unsupported(e.to_string()))?;
        let host_options = match config.mode {
            ProcessMode::Realtime => HostOptions {
                wait: self.options.realtime_wait,
                hang_timeout: self.options.hang_timeout,
                ..HostOptions::default()
            },
            ProcessMode::Offline => HostOptions::offline(self.options.offline_timeout),
        };
        let (host, monitor) = channel.into_ends(host_options);
        sandbox.set_monitor(Some(monitor));
        let answer = sandbox.call(
            RequestBody::Activate {
                sample_rate: config.sample_rate,
                max_block: block,
                mode: config.mode,
            },
            Vec::new(),
            self.options.request_timeout,
        );
        let (plugin_latency, tail) = match answer {
            Ok((
                ResponseBody::Activated {
                    latency_samples,
                    tail_samples,
                },
                _,
            )) => (
                latency_samples,
                tail_samples.map_or(Tail::Infinite, Tail::Samples),
            ),
            other => {
                sandbox.set_monitor(None);
                return Err(match other {
                    Err(e) => external("couldn't activate the plugin", e),
                    Ok(_) => ModuleError::External("unexpected answer to Activate".into()),
                });
            }
        };
        self.active = Some(Active {
            host,
            mode: config.mode,
            block,
            plugin_latency,
            tail,
            restart_requested: false,
        });
        Ok(())
    }

    fn deactivate(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        if let Some(s) = &self.sandbox {
            s.request_channel_shutdown();
            if s.is_usable() {
                let _ = s.call(RequestBody::Deactivate, Vec::new(), DEACTIVATE_TIMEOUT);
            }
            s.set_monitor(None);
        }
        drop(active);
    }

    fn latency_samples(&self) -> u32 {
        self.active
            .as_ref()
            .map_or(0, |a| a.block.saturating_add(a.plugin_latency))
    }

    fn tail(&self) -> Tail {
        self.active.as_ref().map_or(Tail::Samples(0), |a| a.tail)
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let (Some(input), Some(output)) = (inputs.first(), outputs.first_mut()) else {
            return ProcessStatus::Continue;
        };
        let frames = ctx.frames as usize;
        let (input, output) = (&input[..frames], &mut output[..frames]);
        let Self {
            active,
            params,
            values,
            block_hint,
            fault,
            ..
        } = self;
        let Some(a) = active.as_mut() else {
            output.copy_from_slice(input);
            return ProcessStatus::Error;
        };
        let base = a.host.position();
        for e in ctx.events {
            let _ = a.host.push_event(WireEvent::param(
                base + u64::from(e.offset),
                e.id.0,
                e.value,
            ));
            if let Some(i) = params.iter().position(|p| p.id == e.id) {
                values[i] = e.value;
            }
        }
        let outcome = a.host.process(input, output);
        while let Some(ev) = a.host.pop_output_event() {
            // T-803: the plugin asked for a restart (CLAP `request_restart`: its latency
            // changed, …) — ADR-008 §2: replace the instance through the rack.
            if ev.kind == EventKind::RESTART_REQUEST {
                if !a.restart_requested {
                    a.restart_requested = true;
                    ctx.request(HostRequest::Restart);
                }
                continue;
            }
            if ev.kind != EventKind::PARAM_VALUE {
                continue;
            }
            let id = ParamId(ev.id);
            if let Some(i) = params.iter().position(|p| p.id == id) {
                values[i] = ev.value;
                let _ = ctx.out_events.try_push(ParamEvent {
                    offset: 0,
                    id,
                    value: ev.value,
                });
            }
        }
        // ADR-008 §2: callbacks larger than `B` would miss; ask for a replacement at the
        // observed size (the next instance's `B`).
        if a.mode == ProcessMode::Realtime && ctx.frames > a.block && !a.restart_requested {
            block_hint.fetch_max(ctx.frames.next_power_of_two(), Ordering::Relaxed);
            a.restart_requested = true;
            ctx.request(HostRequest::Restart);
        }
        match outcome {
            BlockOutcome::Bypassed => ProcessStatus::Error,
            BlockOutcome::Missed if a.mode == ProcessMode::Offline => {
                fault.offline_miss.store(true, Ordering::Release);
                ProcessStatus::Error
            }
            _ => ProcessStatus::Continue,
        }
    }

    fn reset(&mut self) {
        if let Some(a) = self.active.as_mut() {
            let pos = a.host.position();
            let _ = a.host.push_event(WireEvent {
                pos,
                kind: EventKind::RESET,
                id: 0,
                value: 0.0,
            });
        }
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        let i = self.params.iter().position(|p| p.id == id)?;
        Some(self.values[i])
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        let data = self.fetch_state();
        Ok(ModuleState {
            format_version: self.descriptor.state_format_version,
            params: self
                .params
                .iter()
                .zip(&self.values)
                .filter(|(p, _)| !p.flags.contains(ParamFlags::READ_ONLY))
                .map(|(p, v)| (p.key.clone(), *v))
                .collect(),
            blob: data.map(|d| state::wrap(&self.header(), &d)),
        })
    }

    fn load_state(&mut self, s: &ModuleState) -> Result<(), StateError> {
        let Some(sandbox) = self.usable_sandbox().cloned() else {
            return Err(StateError::InvalidBlob(self.not_running()));
        };
        let t = self.options.request_timeout;
        if let Some(blob) = &s.blob {
            let (h, data) =
                state::unwrap(blob).map_err(|e| StateError::InvalidBlob(e.to_string()))?;
            if h.format != self.spec.format || h.id != self.descriptor.id {
                return Err(StateError::InvalidBlob(format!(
                    "the state belongs to {} ({})",
                    h.id, h.format
                )));
            }
            let answer = sandbox
                .call(RequestBody::LoadState, data.to_vec(), t)
                .map_err(|e| StateError::InvalidBlob(e.to_string()))?;
            if let (ResponseBody::Params { values }, _) = answer {
                for v in values {
                    if let Some(i) = self.params.iter().position(|p| p.id == v.id) {
                        self.values[i] = v.value;
                    }
                }
            }
            *self
                .last_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(data.to_vec());
        }
        let updates: Vec<(usize, ParamValue)> = self
            .params
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.flags.contains(ParamFlags::READ_ONLY))
            .filter_map(|(i, p)| {
                s.params.get(&p.key).map(|&v| {
                    (
                        i,
                        ParamValue {
                            id: p.id,
                            value: p.clamp_quantize(v),
                        },
                    )
                })
            })
            .collect();
        if !updates.is_empty() {
            sandbox
                .call(
                    RequestBody::SetParams {
                        values: updates.iter().map(|(_, v)| *v).collect(),
                    },
                    Vec::new(),
                    t,
                )
                .map_err(|e| StateError::InvalidBlob(e.to_string()))?;
            for (i, v) in updates {
                self.values[i] = v.value;
            }
        }
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::AdapterHealth => Some(Extension::AdapterHealth(self.health.clone())),
            ExtensionId::ParamText => self.text.clone().map(Extension::ParamText),
            _ => None,
        }
    }
}

impl Drop for ProxyModule {
    fn drop(&mut self) {
        // No control round trip here: the watchdog's retire path stops the audio thread
        // (host_command + `Shutdown`), closes the channel and reaps the process.
        drop(self.active.take());
        if let Some(s) = self.sandbox.take() {
            crate::watchdog::retire(s);
        }
    }
}
