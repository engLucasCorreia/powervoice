//! The control-thread side of the live rack: [`RackHost`] owns the authoritative rack model and
//! the parameter mirror (ADR-005 §7), builds and activates chains off the audio thread, sends
//! commands, drains the RT event ring and the return ring (ADR-002 §1 control tick).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use rtrb::PushError;
use serde_json::Map;
use vox_module_api::{
    ActivateConfig, AdapterHealth, CurveHandle, EditorRequest, Module, ModuleDescriptor,
    ModuleError, ModuleRef, ModuleState, NoiseProfile, ParamEvent, ParamFlags, ParamGroup, ParamId,
    ParamInfo, ParamText, PluginEditor, ResponseCurve, Telemetry, TelemetryInfo, TransferCurve,
    TransferHandle, adapter_health, noise_profile, param_text, plugin_editor, response_curve,
    telemetry, transfer_curve,
};

use crate::chain::PlanEntry;
use crate::delay::DelayLine;
use crate::live::RackLink;
use crate::slot::{Slot, SlotInit};
use crate::{
    Chain, FailReason, LiveRack, MAX_SLOTS, MAX_SWAPS_IN_FLIGHT, RackCommand, RackError, RackEvent,
    RackModel, RackOptions, Registry, Resolved, SlotModel, SlotUid,
};

/// Automatic restarts an out-of-process slot gets after an adapter failure before it stays
/// "Failed" (T-802 restart policy: restart once with the last good state; a manual Restart
/// resets the budget).
pub const AUTO_RESTARTS: u32 = 1;

/// Delay before an automatic restart (the UI shows "Restarting"; a crash right at load doesn't
/// respawn in a tight loop).
pub const AUTO_RESTART_DELAY: Duration = Duration::from_millis(200);

/// What the host tells the UI (T-108 maps these to IPC events).
#[derive(Clone, Debug, PartialEq)]
pub enum RackNotice {
    /// A parameter value changed: sent by the UI (echo), reported by the module (READ_ONLY) or
    /// set by the host. `text` is Rust's formatting (ADR-005 §13). Coalesced per (slot, id)
    /// between two [`RackHost::tick`]s: the last value wins.
    ParamChanged {
        /// Slot.
        slot: SlotUid,
        /// Slot index at the time of the notice.
        index: usize,
        /// Parameter.
        id: ParamId,
        /// Plain value.
        value: f64,
        /// Normalized position.
        normalized: f64,
        /// Display text.
        text: String,
    },
    /// The total rack latency changed (samples).
    LatencyChanged {
        /// New total.
        total_samples: u32,
    },
    /// A slot failed and was bypassed ("‹module› produced invalid audio and was bypassed").
    SlotFailed {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
        /// User-facing message.
        message: String,
    },
    /// A slot's instance was replaced (restart request, Restart of a failed slot, new state).
    SlotRestarted {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
    },
    /// A loading slot (a module created off the control thread, T-803) is ready: its schema,
    /// values and latency are known now — or it resolved to a placeholder. A load that failed
    /// is a [`RackNotice::SlotFailed`] instead.
    SlotLoaded {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
    },
    /// A Missing/too-new placeholder slot came back to life on its own (H-40): the catalog just
    /// registered or upgraded its module (install, a quick/full rescan, unblock, re-enable), so
    /// the rack re-resolved the slot from its kept state and blob, through the same async-load or
    /// instant-swap path and T-103 crossfade as any other replacement. The UI shows a toast
    /// ("‹Plugin› is available again — restored in the rack"); unlike [`Self::SlotRestarted`],
    /// this is never a manual [`RackHost::restart`] — see the ADR-008 amendment log (the design
    /// note this ticket added) for why it doesn't touch undo/dirty state.
    SlotRecovered {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
        /// Display name, for the toast.
        name: String,
    },
    /// A plugin's own editor window opened or closed (T-901): through "Open plugin window",
    /// the user closing it, the slot's removal, or its sandbox dying. The UI re-reads the rack.
    EditorChanged {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
        /// Whether the window is open now.
        open: bool,
    },
    /// A plugin changed its state outside its parameters (a GUI-only change, T-901): the slot's
    /// committed blob was refreshed, so the rack model (and the document's dirty state) changed.
    PluginStateChanged {
        /// Slot.
        slot: SlotUid,
        /// Slot index.
        index: usize,
    },
}

/// State of a slot as the UI shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum SlotStatus {
    /// Running.
    Active,
    /// A placeholder: "Missing module ‹id@version›" or "‹id› requires a newer version".
    Missing {
        /// User-facing message.
        message: String,
        /// The module is installed, the state is newer.
        too_new: bool,
    },
    /// Bypassed by the rack after a failure, or could not start when the rack was loaded
    /// ("Couldn't start ‹module›: ‹reason›"; the slot is then kept verbatim and passes dry).
    Failed {
        /// User-facing message.
        message: String,
    },
    /// An out-of-process module failed and is bypassed; an automatic restart with its last
    /// committed state is pending (T-802, [`AUTO_RESTARTS`]).
    Restarting {
        /// The failure ("‹plugin› crashed and was bypassed").
        message: String,
    },
    /// A module whose factory [`loads_async`](vox_module_api::ModuleFactory::loads_async) (an
    /// out-of-process plugin, T-803) is being created and activated off the control thread:
    /// the slot passes dry (latency 0, no schema yet) until it's ready. Also shown while a
    /// failed slot's replacement loads.
    Loading,
}

/// Whether/how a slot's [`NoiseProfile`] blob loads (SPEC-014 §2.5, §2.8). `None` at the call
/// site (not a variant here) means the module has no `NoiseProfile` extension at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseProfileStatus {
    /// No blob: the module passes audio through unchanged.
    None,
    /// A blob is present and validates.
    Loaded,
    /// A blob is present but fails validation (corrupt/truncated).
    Unreadable,
    /// A blob is present but was written by a newer version than this build supports.
    TooNew,
}

/// Metadata of one slot (copied at insertion, ADR-005 §7).
#[derive(Clone, Debug)]
pub struct SlotInfo {
    /// Identity.
    pub uid: SlotUid,
    /// `"id@version"`.
    pub module: String,
    /// The module id alone (registry key, no `@version`); `None` for a placeholder (T-406: which
    /// preset menu to show — a missing module has no schema and no presets).
    pub module_id: Option<String>,
    /// Display name (the module reference for placeholders).
    pub name: String,
    /// Host bypass flag.
    pub bypass: bool,
    /// Latency in samples (0 for placeholders).
    pub latency_samples: u32,
    /// Status.
    pub status: SlotStatus,
    /// Parameter schema (empty for placeholders).
    pub params: Arc<[ParamInfo]>,
    /// Parameter groups, display order (empty for placeholders, S3-01 generic UI layout).
    pub groups: Arc<[ParamGroup]>,
    /// The slot's noise-print status (S3-06): `None` when the module has no `NoiseProfile`
    /// extension (every module but Noise Reduction, and placeholders), `Some(..)` otherwise —
    /// the NR panel section only renders when this is `Some`.
    pub noise_profile: Option<NoiseProfileStatus>,
    /// The module's [`ResponseCurve`] handles (S3-07, SPEC-015 §2.6.6): `None` when the module
    /// has no `ResponseCurve` extension (every module but the EQ, and placeholders) — the graph
    /// panel only renders when this is `Some`. `Some(handles)` even when `handles` is empty (a
    /// `ResponseCurve` with no draggable nodes is still drawable).
    pub curve_handles: Option<Vec<CurveHandle>>,
    /// The module's [`TransferCurve`] handles (H-63, SPEC-016 §4.11): `None` when the module has
    /// no `TransferCurve` extension (every module but Dynamics and the Noise Gate, and
    /// placeholders) — the transfer graph only renders when this is `Some`. `Some(handles)` even
    /// when `handles` is empty.
    pub transfer_handles: Option<Vec<TransferHandle>>,
    /// The module's [`Telemetry`] channel descriptions (H-03; SPEC-016 §4.12: descriptions
    /// travel once, with the rack state): empty when the module has none, and for placeholders.
    /// The values come from [`RackHost::read_telemetry`], in this order.
    pub telemetry: Arc<[TelemetryInfo]>,
    /// The module runs out of process (it answers the `AdapterHealth` extension: a sandboxed
    /// plugin, T-802). `false` for placeholders.
    pub sandboxed: bool,
    /// The module has an editor window of its own (T-901: a sandboxed plugin with a GUI).
    pub has_editor: bool,
    /// That window is open.
    pub editor_open: bool,
}

/// One slot's current [`Telemetry`] values ([`RackHost::read_telemetry`]), in the order of its
/// [`SlotInfo::telemetry`] channels.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotTelemetry {
    /// The slot.
    pub uid: SlotUid,
    /// One value per channel, in the channel's unit.
    pub values: Vec<f32>,
}

struct Loaded {
    descriptor: ModuleDescriptor,
    params: Arc<[ParamInfo]>,
    groups: Arc<[ParamGroup]>,
    /// The parameter mirror, index-aligned with `params` (READ_ONLY values as reported).
    values: Vec<f64>,
    /// The committed state blob.
    blob: Option<Vec<u8>>,
    latency: u32,
    failed: Option<String>,
    /// An activated instance not yet handed to the audio thread (insert or replacement).
    fresh: Option<Box<dyn Module>>,
    /// The module's [`NoiseProfile`] handle (S3-06), captured once here so it stays available
    /// after `fresh` is taken by the audio thread — `capture`/`describe` are pure functions of
    /// their inputs (module docs), safe to call from any non-audio thread regardless of which
    /// instance is currently live.
    noise_profile: Option<Arc<dyn NoiseProfile>>,
    /// The module's [`ResponseCurve`] handle (S3-07), captured the same way and for the same
    /// reason: `magnitude_db`/`component_magnitude_db` are pure functions of their arguments
    /// (module docs), safe to call from the control thread regardless of which instance is
    /// currently live.
    response_curve: Option<Arc<dyn ResponseCurve>>,
    /// The module's [`TransferCurve`] handle (H-63), captured the same way and for the same
    /// reason: every call is a pure function of its arguments (module docs), safe from the
    /// control thread whichever instance is currently live.
    transfer_curve: Option<Arc<dyn TransferCurve>>,
    /// The [`Telemetry`] handle of the newest instance (H-03). Unlike the pure-function
    /// extensions above, telemetry values belong to one instance, so this is refreshed on every
    /// replacement ([`RackHost::replace_state`]/restart). The host is its single reader
    /// (ADR-005 §11, [`RackHost::read_telemetry`]).
    telemetry: Option<Arc<dyn Telemetry>>,
    /// `telemetry`'s channel descriptions (empty without the extension).
    telemetry_channels: Arc<[TelemetryInfo]>,
    /// The newest instance's [`AdapterHealth`] handle (out-of-process modules only; T-802):
    /// the failure reason, and whether the restart policy applies. Refreshed on replacement.
    health: Option<Arc<dyn AdapterHealth>>,
    /// The newest instance's [`ParamText`] handle (a plugin that formats its own values,
    /// T-803): display text and typed-text parsing. Refreshed on replacement.
    text: Option<Arc<dyn ParamText>>,
    /// The newest instance's [`PluginEditor`] handle, when its plugin has a window of its own
    /// (T-901). Refreshed on replacement.
    editor: Option<Arc<dyn PluginEditor>>,
    /// The window's open state as last reported to the UI ([`RackNotice::EditorChanged`]).
    editor_reported_open: bool,
    /// How the window was last opened (reopened with it after a plugin-requested restart).
    editor_request: Option<EditorRequest>,
    /// The window was opened at least once: the plugin may hold GUI-only state, so a save
    /// captures its state first ([`RackHost::editors_to_capture`]).
    editor_used: bool,
}

/// A module's [`Telemetry`] handle and its channel descriptions.
fn telemetry_of(module: &dyn Module) -> (Option<Arc<dyn Telemetry>>, Arc<[TelemetryInfo]>) {
    let handle = telemetry(module);
    let channels = handle
        .as_deref()
        .map_or_else(|| Arc::from(Vec::new()), |t| Arc::from(t.channels()));
    (handle, channels)
}

enum Kind {
    Loaded(Box<Loaded>),
    /// Being created and activated on a worker thread (T-803, a `loads_async` factory): dry,
    /// latency 0, written back verbatim until the instance arrives.
    Loading {
        /// The stored slot being loaded.
        model: SlotModel,
        /// Display name (the factory's).
        name: String,
    },
    /// Dry, latency 0, written back verbatim: a missing module, a too-new state, or (`failed`)
    /// a module that could not start when the rack was loaded.
    Placeholder {
        model: SlotModel,
        message: String,
        too_new: bool,
        failed: bool,
    },
}

struct HostSlot {
    uid: SlotUid,
    bypass: bool,
    extra: Map<String, serde_json::Value>,
    kind: Kind,
    /// Automatic restarts since the last manual one (T-802 restart policy).
    auto_restarts: u32,
    /// An automatic restart is due at this time (the slot shows "Restarting").
    restart_due: Option<Instant>,
    /// The background instantiation in flight for this slot (T-803): its first instance
    /// ([`Kind::Loading`]) or a replacement of a loaded one.
    job: Option<PendingJob>,
    /// T-901: the slot's editor window was open when the slot was moved; reopen it once the new
    /// instance is loaded.
    reopen_editor: Option<EditorRequest>,
}

impl HostSlot {
    fn new(uid: SlotUid, bypass: bool, extra: Map<String, serde_json::Value>, kind: Kind) -> Self {
        Self {
            uid,
            bypass,
            extra,
            kind,
            auto_restarts: 0,
            restart_due: None,
            job: None,
            reopen_editor: None,
        }
    }
}

/// Reopens a plugin's editor window on a short-lived thread (T-901): opening waits for the
/// plugin's GUI, which must never stall the rack's control thread.
fn reopen_editor(editor: Arc<dyn PluginEditor>, request: EditorRequest) {
    let spawned = std::thread::Builder::new()
        .name("rack-editor-open".into())
        .spawn(move || {
            let _ = editor.open(&request);
        });
    drop(spawned);
}

/// A background instantiation a slot waits for (T-803).
struct PendingJob {
    id: u64,
    /// Replacements: the mirror when it was requested — a value the user changes meanwhile
    /// wins over the new instance's.
    values: Vec<f64>,
    /// Replacements: the requested state's blob (the committed blob if the instance has none).
    blob: Option<Vec<u8>>,
    /// H-40: this job is a live recovery of a Missing/too-new placeholder (registry generation
    /// bump), not an initial load or a restart/preset replacement — its success is
    /// [`RackNotice::SlotRecovered`], not [`RackNotice::SlotLoaded`].
    recovering: bool,
}

/// A finished background instantiation (T-803), sent by the loader thread.
struct LoadDone {
    job: u64,
    uid: SlotUid,
    result: Result<Resolved, RackError>,
}

/// The display name of `s`'s module when its factory loads asynchronously (T-803), else
/// `None` (create it here, synchronously).
fn async_name(registry: &Registry, s: &SlotModel) -> Option<String> {
    if s.is_malformed() {
        return None;
    }
    let r = s.module_ref().ok()?;
    let f = registry.get(&r.id)?;
    f.loads_async().then(|| f.descriptor().name.text.clone())
}

/// "Couldn't start ‹module›: ‹reason›" for a slot that failed to start.
fn start_failure(s: &SlotModel, e: RackError) -> String {
    match e {
        RackError::Activate { .. } | RackError::Create { .. } => e.to_string(),
        other => format!("Couldn't start {}: {other}", s.module),
    }
}

/// A slot of the chain the audio thread has (or will have after the next flush).
#[derive(Clone, Copy, Debug)]
struct LayoutEntry {
    uid: SlotUid,
    /// The audio thread has (or has been sent) this slot.
    sent: bool,
    /// A replacement instance waits in `Loaded::fresh`.
    replace: bool,
    /// A replacement was sent and its crossfade has not finished: a further replacement waits
    /// for [`RackEvent::ReplaceDone`].
    aux_pending: bool,
    /// Removed from the model, fading out.
    dying: bool,
    /// Fade-out finished: leave it out of the next chain.
    dead: bool,
    /// The replacement crossfade finished: drop the outgoing instance at the next chain.
    aux_done: bool,
    /// Latency of the instance the audio thread runs.
    live_latency: u32,
}

impl LayoutEntry {
    fn new(uid: SlotUid) -> Self {
        Self {
            uid,
            sent: false,
            replace: false,
            aux_pending: false,
            dying: false,
            dead: false,
            aux_done: false,
            live_latency: 0,
        }
    }
}

/// [`SlotInfo::noise_profile`] for a loaded slot: `None` (outer) when the module has no
/// `NoiseProfile` extension; otherwise the status of `blob` per SPEC-014 §2.5/§2.8. `describe`'s
/// scratch buffer is not otherwise used here (the profile graph is [H]).
fn noise_profile_status(
    ext: Option<&dyn NoiseProfile>,
    blob: Option<&[u8]>,
) -> Option<NoiseProfileStatus> {
    let ext = ext?;
    Some(match blob {
        None => NoiseProfileStatus::None,
        Some(b) => {
            let mut scratch = Vec::new();
            match ext.describe(b, &mut scratch) {
                Ok(()) => NoiseProfileStatus::Loaded,
                Err(ModuleError::Unsupported(_)) => NoiseProfileStatus::TooNew,
                Err(_) => NoiseProfileStatus::Unreadable,
            }
        }
    })
}

fn committed_state(l: &Loaded) -> ModuleState {
    ModuleState {
        format_version: l.descriptor.state_format_version,
        params: l
            .params
            .iter()
            .zip(&l.values)
            .filter(|(p, _)| !p.flags.contains(ParamFlags::READ_ONLY))
            .map(|(p, v)| (p.key.clone(), *v))
            .collect(),
        blob: l.blob.clone(),
    }
}

fn loaded_from(module: Box<dyn Module>) -> Box<Loaded> {
    let params: Arc<[ParamInfo]> = module.params().into();
    let groups: Arc<[ParamGroup]> = module.groups().into();
    let values = params
        .iter()
        .map(|p| module.param_value(p.id).unwrap_or(p.default))
        .collect();
    let blob = module.save_state().ok().and_then(|s| s.blob);
    let profile = noise_profile(module.as_ref());
    let curve = response_curve(module.as_ref());
    let transfer = transfer_curve(module.as_ref());
    let (telemetry, telemetry_channels) = telemetry_of(module.as_ref());
    let health = adapter_health(module.as_ref());
    let text = param_text(module.as_ref());
    let editor = plugin_editor(module.as_ref()).filter(|e| e.available());
    Box::new(Loaded {
        descriptor: module.descriptor().clone(),
        params,
        groups,
        values,
        blob,
        latency: module.latency_samples(),
        failed: None,
        fresh: Some(module),
        noise_profile: profile,
        response_curve: curve,
        transfer_curve: transfer,
        telemetry,
        telemetry_channels,
        health,
        text,
        editor,
        editor_reported_open: false,
        editor_request: None,
        editor_used: false,
    })
}

/// The host slot kind for a stored slot, never failing: missing modules and too-new states
/// become placeholders, modules that cannot start become failed placeholders (kept verbatim),
/// modules that load asynchronously start [`Kind::Loading`] (the caller starts the job).
fn resolve_lenient(
    registry: &Registry,
    config: &ActivateConfig,
    s: &SlotModel,
    index: usize,
) -> Kind {
    if let Some(name) = async_name(registry, s) {
        return Kind::Loading {
            model: s.clone(),
            name,
        };
    }
    match registry.instantiate(s, config, index) {
        Ok(Resolved::Module(m)) => Kind::Loaded(loaded_from(m)),
        Ok(Resolved::Placeholder { message, too_new }) => Kind::Placeholder {
            model: s.clone(),
            message,
            too_new,
            failed: false,
        },
        Err(e) => Kind::Placeholder {
            model: s.clone(),
            message: start_failure(s, e),
            too_new: false,
            failed: true,
        },
    }
}

/// Builds the slot for a layout entry that brings a new instance (or a placeholder).
fn build_fresh_slot(
    hs: &mut HostSlot,
    config: &ActivateConfig,
    options: &RackOptions,
    fade_len: u32,
    fade_in: bool,
    extra_tap: u32,
) -> Slot {
    let init = SlotInit {
        bypassed: hs.bypass,
        fade_in,
    };
    match &mut hs.kind {
        Kind::Loaded(l) => {
            let module = l.fresh.take().expect("layout entry without an instance");
            // Values changed since the instance was created reach it as events at the first
            // sample.
            let diffs: Vec<ParamEvent> = l
                .params
                .iter()
                .zip(&l.values)
                .filter(|(p, v)| {
                    !p.flags.contains(ParamFlags::READ_ONLY)
                        && module
                            .param_value(p.id)
                            .is_none_or(|cur| cur.to_bits() != v.to_bits())
                })
                .map(|(p, v)| ParamEvent {
                    offset: 0,
                    id: p.id,
                    value: *v,
                })
                .collect();
            let name = l.descriptor.name.text.clone();
            let mut slot = Slot::new(hs.uid, Some(module), name, options);
            slot.attach(config, fade_len, init, extra_tap);
            for e in diffs {
                let _ = slot.push_event(e);
            }
            slot
        }
        Kind::Placeholder { model, .. } => {
            let mut slot = Slot::new(hs.uid, None, model.module.clone(), options);
            slot.attach(config, fade_len, init, 0);
            slot
        }
        Kind::Loading { name, .. } => {
            let mut slot = Slot::new(hs.uid, None, name.clone(), options);
            slot.attach(config, fade_len, init, 0);
            slot
        }
    }
}

/// Builds the next chain from the layout (fresh, kept and replacing slots). Returns whether
/// it differs from the chain the audio thread has.
fn build_chain(
    slots: &mut [HostSlot],
    layout: &mut [LayoutEntry],
    config: &ActivateConfig,
    options: RackOptions,
    fade_len: u32,
    fade_in: bool,
) -> (Chain, bool) {
    let mut chain = Chain::assembled(config, options);
    let mut changed = false;
    for e in layout.iter_mut() {
        let index = slots.iter().position(|s| s.uid == e.uid);
        match (e.sent, e.replace && !e.aux_pending, index) {
            (false, _, Some(k)) => {
                let slot = build_fresh_slot(&mut slots[k], config, &options, fade_len, fade_in, 0);
                e.sent = true;
                e.replace = false;
                e.live_latency = slot.latency;
                chain.slots.push(slot);
                chain.plan.push(PlanEntry::Fresh);
                changed = true;
            }
            (true, true, Some(k)) => {
                let slot = build_fresh_slot(
                    &mut slots[k],
                    config,
                    &options,
                    fade_len,
                    false,
                    e.live_latency,
                );
                e.replace = false;
                e.aux_done = false;
                e.aux_pending = true;
                e.live_latency = slot.latency;
                chain.slots.push(slot);
                chain.plan.push(PlanEntry::Replace { uid: e.uid });
                changed = true;
            }
            _ => {
                chain.slots.push(Slot::shell(e.uid));
                chain.plan.push(PlanEntry::Keep {
                    uid: e.uid,
                    drop_aux: e.aux_done,
                });
                changed |= e.aux_done;
                e.aux_done = false;
            }
        }
    }
    chain.expected_latency = Some(
        layout
            .iter()
            .filter(|e| !e.dying)
            .map(|e| e.live_latency)
            .fold(0, u32::saturating_add),
    );
    (chain, changed)
}

fn total_latency(slots: &[HostSlot]) -> u32 {
    slots
        .iter()
        .map(|s| match &s.kind {
            Kind::Loaded(l) => l.latency,
            Kind::Placeholder { .. } | Kind::Loading { .. } => 0,
        })
        .fold(0u32, u32::saturating_add)
}

/// The live rack's control side. Every method runs on the control thread; call
/// [`tick`](Self::tick) regularly (ADR-002: every 16 ms) to drain the RT event and return rings
/// and to send deferred swaps.
pub struct RackHost {
    registry: Arc<Registry>,
    config: ActivateConfig,
    options: RackOptions,
    fade_len: u32,
    slots: Vec<HostSlot>,
    layout: Vec<LayoutEntry>,
    link: RackLink,
    backlog: VecDeque<RackCommand>,
    ab: bool,
    in_flight: usize,
    dirty: bool,
    ab_capacity: usize,
    next_uid: u64,
    notices: Vec<RackNotice>,
    reported_latency: u32,
    /// Background instantiations report here (T-803; drained by [`tick`](Self::tick)).
    loads_tx: Sender<LoadDone>,
    loads_rx: Receiver<LoadDone>,
    next_job: u64,
    /// [`Registry::generation`] as of the last check (H-40): [`tick`](Self::tick) rechecks
    /// placeholders only when this is stale, instead of walking every slot every 16 ms.
    registry_generation: u64,
}

impl RackHost {
    /// Creates the live rack for `model` at `config` (realtime, `max_block` = [`MAX_BLOCK`]).
    /// Returns the control side and the [`LiveRack`] to hand to the audio thread. Slots whose
    /// module is missing or has a too-new state become placeholders; modules that fail to start
    /// become failed slots ("Couldn't start …") that pass dry and are kept verbatim. The
    /// whole-rack A/B starts off.
    ///
    /// [`MAX_BLOCK`]: crate::MAX_BLOCK
    pub fn new(
        registry: Arc<Registry>,
        config: ActivateConfig,
        options: RackOptions,
        model: &RackModel,
    ) -> Result<(Self, LiveRack), RackError> {
        if model.slots.len() > MAX_SLOTS {
            return Err(RackError::TooManySlots);
        }
        if config.max_block == 0 || !(config.sample_rate.is_finite() && config.sample_rate > 0.0) {
            return Err(RackError::InvalidConfig("invalid sample rate or max_block"));
        }
        let fade_len = crate::xfade_samples(config.sample_rate);
        let mut slots = Vec::with_capacity(model.slots.len());
        let mut layout = Vec::with_capacity(model.slots.len());
        let mut next_uid = 1;
        for (i, s) in model.slots.iter().enumerate() {
            let uid = SlotUid(next_uid);
            next_uid += 1;
            slots.push(HostSlot::new(
                uid,
                s.bypass,
                s.extra.clone(),
                resolve_lenient(&registry, &config, s, i),
            ));
            layout.push(LayoutEntry::new(uid));
        }
        let (chain, _) = build_chain(&mut slots, &mut layout, &config, options, fade_len, false);
        let total = total_latency(&slots);
        let ab_capacity = (total as usize).max((config.sample_rate * 0.1).round() as usize);
        let (live, link) = LiveRack::new(Box::new(chain), ab_capacity);
        let (loads_tx, loads_rx) = mpsc::channel();
        let registry_generation = registry.generation();
        let mut host = Self {
            registry,
            config,
            options,
            fade_len,
            slots,
            layout,
            link,
            backlog: VecDeque::new(),
            ab: false,
            in_flight: 0,
            dirty: false,
            ab_capacity,
            next_uid,
            notices: Vec::new(),
            reported_latency: total,
            loads_tx,
            loads_rx,
            next_job: 1,
            registry_generation,
        };
        host.start_pending_loads();
        Ok((host, live))
    }

    /// Starts a background instantiation of `model` for slot `uid` (T-803): the loader thread
    /// creates and activates the instance and reports through the load ring; a result for a
    /// slot that is gone (or has a newer job) is deactivated and dropped by the next tick.
    fn spawn_load(&mut self, uid: SlotUid, model: SlotModel, index: usize) -> u64 {
        let job = self.next_job;
        self.next_job += 1;
        let (registry, config, tx) = (self.registry.clone(), self.config, self.loads_tx.clone());
        let id = model.module.clone();
        let spawned = std::thread::Builder::new()
            .name("rack-loader".into())
            .spawn(move || {
                let result = registry.instantiate(&model, &config, index);
                if let Err(mpsc::SendError(done)) = tx.send(LoadDone { job, uid, result })
                    && let Ok(Resolved::Module(mut m)) = done.result
                {
                    // The rack is gone: nobody will take the instance.
                    m.deactivate();
                }
            });
        if let Err(e) = spawned {
            let _ = self.loads_tx.send(LoadDone {
                job,
                uid,
                result: Err(RackError::Create {
                    id,
                    source: ModuleError::External(format!("couldn't start a loader thread: {e}")),
                }),
            });
        }
        job
    }

    /// Starts the job of every [`Kind::Loading`] slot that has none yet.
    fn start_pending_loads(&mut self) {
        for k in 0..self.slots.len() {
            let hs = &self.slots[k];
            if hs.job.is_some() {
                continue;
            }
            let Kind::Loading { model, .. } = &hs.kind else {
                continue;
            };
            let (uid, model) = (hs.uid, model.clone());
            let id = self.spawn_load(uid, model, k);
            self.slots[k].job = Some(PendingJob {
                id,
                values: Vec::new(),
                blob: None,
                recovering: false,
            });
        }
    }

    /// True while a background instantiation is in flight (a slot loading, or a replacement
    /// on its way).
    pub fn is_loading(&self) -> bool {
        self.slots.iter().any(|s| s.job.is_some())
    }

    /// Takes the finished background instantiations (T-803).
    fn drain_loads(&mut self) {
        while let Ok(done) = self.loads_rx.try_recv() {
            self.on_load_done(done);
        }
    }

    fn on_load_done(&mut self, done: LoadDone) {
        let current = self
            .index_of(done.uid)
            .filter(|&k| self.slots[k].job.as_ref().is_some_and(|j| j.id == done.job));
        let Some(k) = current else {
            // The slot was removed, or a newer job superseded this one.
            if let Ok(Resolved::Module(mut m)) = done.result {
                m.deactivate();
            }
            return;
        };
        let Some(job) = self.slots[k].job.take() else {
            return;
        };
        let uid = done.uid;
        match &self.slots[k].kind {
            Kind::Loading { model, name } => {
                let model = model.clone();
                let name = name.clone();
                match done.result {
                    Ok(Resolved::Module(m)) => {
                        self.slots[k].kind = Kind::Loaded(loaded_from(m));
                        if let Some(request) = self.slots[k].reopen_editor.take() {
                            self.reopen_when_loaded(k, request);
                        }
                        // The audio thread runs the dry stand-in: replace it (the instance
                        // fades in from the undelayed input once its output is valid).
                        if let Some(li) = self.layout_pos(uid)
                            && self.layout[li].sent
                        {
                            self.layout[li].replace = true;
                        }
                        self.dirty = true;
                        // H-40: a live recovery gets its own notice (a toast, and the document
                        // layer rebases its dirty baseline) instead of the silent `SlotLoaded`
                        // every other async load reports.
                        self.notices.push(if job.recovering {
                            RackNotice::SlotRecovered {
                                slot: uid,
                                index: k,
                                name,
                            }
                        } else {
                            RackNotice::SlotLoaded {
                                slot: uid,
                                index: k,
                            }
                        });
                    }
                    Ok(Resolved::Placeholder { message, too_new }) => {
                        self.slots[k].kind = Kind::Placeholder {
                            model,
                            message,
                            too_new,
                            failed: false,
                        };
                        self.notices.push(RackNotice::SlotLoaded {
                            slot: uid,
                            index: k,
                        });
                    }
                    Err(e) => {
                        let message = start_failure(&model, e);
                        self.slots[k].kind = Kind::Placeholder {
                            model,
                            message: message.clone(),
                            too_new: false,
                            failed: true,
                        };
                        self.notices.push(RackNotice::SlotFailed {
                            slot: uid,
                            index: k,
                            message,
                        });
                    }
                }
            }
            Kind::Loaded(l) => {
                let name = l.descriptor.name.text.clone();
                let err = match done.result {
                    Ok(Resolved::Module(m)) => {
                        self.install_replacement(k, m, job.blob, Some(&job.values));
                        None
                    }
                    Ok(Resolved::Placeholder { message, .. }) => Some(message),
                    Err(e) => Some(e.to_string()),
                };
                if let Some(err) = err
                    && let Kind::Loaded(l) = &mut self.slots[k].kind
                {
                    let message = format!("Couldn't restart {name}: {err}");
                    l.failed = Some(message.clone());
                    self.notices.push(RackNotice::SlotFailed {
                        slot: uid,
                        index: k,
                        message,
                    });
                }
            }
            Kind::Placeholder { .. } => {
                if let Ok(Resolved::Module(mut m)) = done.result {
                    m.deactivate();
                }
            }
        }
        self.check_latency();
    }

    fn alloc_uid(&mut self) -> SlotUid {
        let uid = SlotUid(self.next_uid);
        self.next_uid += 1;
        uid
    }

    /// The registry this rack resolves modules with.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The realtime configuration.
    pub fn config(&self) -> ActivateConfig {
        self.config
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True if the rack has no slots.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Identity of slot `index`.
    pub fn slot_uid(&self, index: usize) -> Option<SlotUid> {
        self.slots.get(index).map(|s| s.uid)
    }

    /// Index of slot `uid`.
    pub fn index_of(&self, uid: SlotUid) -> Option<usize> {
        self.slots.iter().position(|s| s.uid == uid)
    }

    fn layout_pos(&self, uid: SlotUid) -> Option<usize> {
        self.layout.iter().position(|e| e.uid == uid)
    }

    /// Where a slot inserted at model index `index` goes in the chain layout.
    fn layout_pos_for(&self, index: usize) -> usize {
        self.slots
            .get(index)
            .and_then(|s| self.layout_pos(s.uid))
            .unwrap_or(self.layout.len())
    }

    /// Metadata and status of slot `index`.
    pub fn slot_info(&self, index: usize) -> Option<SlotInfo> {
        let hs = self.slots.get(index)?;
        Some(match &hs.kind {
            Kind::Loaded(l) => SlotInfo {
                uid: hs.uid,
                module: ModuleRef::of(&l.descriptor).to_string(),
                module_id: Some(l.descriptor.id.clone()),
                name: l.descriptor.name.text.clone(),
                bypass: hs.bypass,
                latency_samples: l.latency,
                status: match (&l.failed, hs.restart_due, &hs.job) {
                    (Some(_), _, Some(_)) => SlotStatus::Loading,
                    (Some(message), Some(_), None) => SlotStatus::Restarting {
                        message: message.clone(),
                    },
                    (Some(message), None, None) => SlotStatus::Failed {
                        message: message.clone(),
                    },
                    (None, _, _) => SlotStatus::Active,
                },
                params: l.params.clone(),
                groups: l.groups.clone(),
                noise_profile: noise_profile_status(l.noise_profile.as_deref(), l.blob.as_deref()),
                curve_handles: l.response_curve.as_deref().map(|c| c.handles().to_vec()),
                transfer_handles: l.transfer_curve.as_deref().map(|c| c.handles().to_vec()),
                telemetry: l.telemetry_channels.clone(),
                sandboxed: l.health.is_some(),
                has_editor: l.editor.is_some(),
                editor_open: l.editor.as_ref().is_some_and(|e| e.is_open()),
            },
            Kind::Placeholder {
                model,
                message,
                too_new,
                failed,
            } => SlotInfo {
                uid: hs.uid,
                module: model.module.clone(),
                module_id: None,
                name: model.module.clone(),
                bypass: hs.bypass,
                latency_samples: 0,
                status: if *failed {
                    SlotStatus::Failed {
                        message: message.clone(),
                    }
                } else {
                    SlotStatus::Missing {
                        message: message.clone(),
                        too_new: *too_new,
                    }
                },
                params: Arc::from(Vec::new()),
                groups: Arc::from(Vec::new()),
                noise_profile: None,
                curve_handles: None,
                transfer_handles: None,
                telemetry: Arc::from(Vec::new()),
                sandboxed: false,
                has_editor: false,
                editor_open: false,
            },
            Kind::Loading { model, name } => SlotInfo {
                uid: hs.uid,
                module: model.module.clone(),
                module_id: None,
                name: name.clone(),
                bypass: hs.bypass,
                latency_samples: 0,
                status: SlotStatus::Loading,
                params: Arc::from(Vec::new()),
                groups: Arc::from(Vec::new()),
                noise_profile: None,
                curve_handles: None,
                transfer_handles: None,
                telemetry: Arc::from(Vec::new()),
                sandboxed: false,
                has_editor: false,
                editor_open: false,
            },
        })
    }

    /// The display text of every parameter value of slot `index` (index-aligned with its
    /// schema; empty for a slot without one): the plugin's own text when it has
    /// [`ParamText`] (one batched round trip, T-803), else the module API's text rules.
    pub fn param_texts(&self, index: usize) -> Vec<String> {
        let Some(Kind::Loaded(l)) = self.slots.get(index).map(|s| &s.kind) else {
            return Vec::new();
        };
        let own = l.text.as_ref().map(|t| {
            let values: Vec<(ParamId, f64)> = l
                .params
                .iter()
                .zip(&l.values)
                .map(|(p, &v)| (p.id, v))
                .collect();
            t.values_to_text(&values)
        });
        l.params
            .iter()
            .zip(&l.values)
            .enumerate()
            .map(|(i, (p, &v))| {
                own.as_ref()
                    .and_then(|o| o.get(i).cloned().flatten())
                    .unwrap_or_else(|| p.value_to_text(v))
            })
            .collect()
    }

    /// Total latency: the sum of slot latencies, bypassed slots included, placeholders 0
    /// (SPEC-012 §2.5).
    pub fn total_latency_samples(&self) -> u32 {
        total_latency(&self.slots)
    }

    /// The mirrored value of parameter `id` of slot `index` (the last value sent or reported).
    pub fn param_value(&self, index: usize, id: ParamId) -> Option<f64> {
        let Kind::Loaded(l) = &self.slots.get(index)?.kind else {
            return None;
        };
        let pi = l.params.iter().position(|p| p.id == id)?;
        Some(l.values[pi])
    }

    /// True while the whole-rack A/B is on.
    pub fn ab(&self) -> bool {
        self.ab
    }

    /// Chain swaps sent whose retired chain has not come back yet.
    pub fn swaps_in_flight(&self) -> usize {
        self.in_flight
    }

    /// RT events the audio thread dropped (module reports beyond the ring's lifecycle reserve,
    /// or a full outbox), plus queued parameter events a replacement instance's full queue
    /// could not take over.
    pub fn dropped_rt_events(&self) -> u64 {
        self.link.dropped.load(Ordering::Relaxed)
    }

    /// The rack as plain data (sidecar slot schema): committed states from the mirror,
    /// placeholders verbatim. The A/B flag is not part of it.
    pub fn model(&self) -> RackModel {
        RackModel {
            slots: self
                .slots
                .iter()
                .map(|hs| match &hs.kind {
                    Kind::Loaded(l) => {
                        let mut m = SlotModel::new(
                            &ModuleRef::of(&l.descriptor),
                            hs.bypass,
                            &committed_state(l),
                        );
                        m.extra = hs.extra.clone();
                        m
                    }
                    Kind::Placeholder { model, .. } | Kind::Loading { model, .. } => {
                        let mut m = model.clone();
                        m.bypass = hs.bypass;
                        m
                    }
                })
                .collect(),
        }
    }

    fn send(&mut self, cmd: RackCommand) {
        if self.backlog.is_empty() {
            if let Err(PushError::Full(cmd)) = self.link.commands.push(cmd) {
                self.backlog.push_back(cmd);
            }
        } else {
            self.backlog.push_back(cmd);
        }
    }

    fn pump_backlog(&mut self) {
        while let Some(cmd) = self.backlog.pop_front() {
            if let Err(PushError::Full(cmd)) = self.link.commands.push(cmd) {
                self.backlog.push_front(cmd);
                break;
            }
        }
    }

    fn check_latency(&mut self) {
        let total = self.total_latency_samples();
        if total != self.reported_latency {
            self.reported_latency = total;
            self.notices.push(RackNotice::LatencyChanged {
                total_samples: total,
            });
        }
    }

    /// Queues a `ParamChanged` for slot `index`, parameter `pi`, replacing an earlier one for
    /// the same (slot, id) since the last tick.
    fn notice_param(&mut self, index: usize, pi: usize) {
        let hs = &self.slots[index];
        let Kind::Loaded(l) = &hs.kind else {
            return;
        };
        let p = &l.params[pi];
        let v = l.values[pi];
        let (uid, pid) = (hs.uid, p.id);
        let text = l
            .text
            .as_ref()
            .and_then(|t| t.values_to_text(&[(pid, v)]).into_iter().next().flatten())
            .unwrap_or_else(|| p.value_to_text(v));
        let notice = RackNotice::ParamChanged {
            slot: uid,
            index,
            id: pid,
            value: v,
            normalized: p.to_normalized(v),
            text,
        };
        self.notices.retain(
            |n| !matches!(n, RackNotice::ParamChanged { slot, id, .. } if *slot == uid && *id == pid),
        );
        self.notices.push(notice);
    }

    /// Sends the next chain if the layout changed and the in-flight limit allows it.
    fn flush(&mut self) {
        if !self.dirty || self.in_flight >= MAX_SWAPS_IN_FLIGHT {
            return;
        }
        self.dirty = false;
        let before = self.layout.len();
        self.layout.retain(|e| !e.dead);
        let compacted = self.layout.len() != before;
        let (mut chain, changed) = build_chain(
            &mut self.slots,
            &mut self.layout,
            &self.config,
            self.options,
            self.fade_len,
            true,
        );
        if !(changed || compacted) {
            // Only shells: nothing to send (a held-back replacement waits for ReplaceDone).
            return;
        }
        let total = chain.expected_latency.unwrap_or(0);
        if total as usize > self.ab_capacity {
            let cap = (total as usize).max(2 * self.ab_capacity);
            chain.spare_ab = Some(DelayLine::new(cap, self.config.max_block as usize));
            self.ab_capacity = cap;
        }
        self.send(RackCommand::Swap(Box::new(chain)));
        self.in_flight += 1;
    }

    /// Inserts `slot` (a stored slot, e.g. from a preset) at `index` (`0..=len`), live: the
    /// instance is created and activated here; it is heard once its output is valid (after its
    /// reported latency) and fades in over 15 ms while the other slots keep running untouched.
    /// A missing module becomes a placeholder. A module that fails to activate is not inserted
    /// ("Couldn't start ‹module›: ‹reason›").
    ///
    /// A module whose factory loads asynchronously (T-803) is inserted at once as a loading
    /// slot (dry) and created on a worker thread; it becomes active — or a failed slot with
    /// the reason — at a later [`tick`](Self::tick).
    pub fn insert(&mut self, index: usize, slot: SlotModel) -> Result<SlotUid, RackError> {
        if index > self.slots.len() {
            return Err(RackError::IndexOutOfRange {
                index,
                len: self.slots.len(),
            });
        }
        if self.slots.len() >= MAX_SLOTS {
            return Err(RackError::TooManySlots);
        }
        let kind = match async_name(&self.registry, &slot) {
            Some(name) => Kind::Loading {
                model: slot.clone(),
                name,
            },
            None => match self.registry.instantiate(&slot, &self.config, index)? {
                Resolved::Module(m) => Kind::Loaded(loaded_from(m)),
                Resolved::Placeholder { message, too_new } => Kind::Placeholder {
                    model: slot.clone(),
                    message,
                    too_new,
                    failed: false,
                },
            },
        };
        let uid = self.alloc_uid();
        let pos = self.layout_pos_for(index);
        self.layout.insert(pos, LayoutEntry::new(uid));
        self.slots
            .insert(index, HostSlot::new(uid, slot.bypass, slot.extra, kind));
        self.start_pending_loads();
        self.dirty = true;
        self.flush();
        self.check_latency();
        Ok(uid)
    }

    /// Inserts module `id` (registered) with its default state at `index`.
    pub fn insert_module(&mut self, index: usize, id: &str) -> Result<SlotUid, RackError> {
        let f = self
            .registry
            .get(id)
            .ok_or_else(|| RackError::UnknownModule(id.to_owned()))?;
        let r = ModuleRef::of(f.descriptor());
        self.insert(
            index,
            SlotModel {
                module: r.to_string(),
                bypass: false,
                state: serde_json::Value::Null,
                extra: Map::new(),
                raw: None,
            },
        )
    }

    /// Removes slot `index`, live: it fades out to its undelayed input over 15 ms, then the
    /// audio thread drops it from the chain (the instance returns through the return ring).
    pub fn remove(&mut self, index: usize) -> Result<(), RackError> {
        self.remove_with_hold(index, 0)
    }

    fn remove_with_hold(&mut self, index: usize, hold: u32) -> Result<(), RackError> {
        if index >= self.slots.len() {
            return Err(RackError::IndexOutOfRange {
                index,
                len: self.slots.len(),
            });
        }
        let mut hs = self.slots.remove(index);
        if let Kind::Loaded(l) = &mut hs.kind {
            // T-901: the window goes with its slot at once (not when the fading instance is
            // finally dropped).
            if let Some(e) = &l.editor {
                e.close();
            }
            if let Some(mut m) = l.fresh.take() {
                m.deactivate();
            }
        }
        if let Some(li) = self.layout_pos(hs.uid) {
            if self.layout[li].sent {
                self.layout[li].dying = true;
                self.layout[li].replace = false;
                self.send(RackCommand::Remove { slot: hs.uid, hold });
            } else {
                self.layout.remove(li);
            }
        }
        self.check_latency();
        Ok(())
    }

    /// Moves slot `from` to position `to` (both `< len`), live. The moved module restarts from
    /// its committed state (SPEC-012 §2.2): a fresh instance fades in at the new position once
    /// its output is valid (its latency), and the old one fades out at the same moment; the
    /// other slots keep running untouched.
    pub fn move_slot(&mut self, from: usize, to: usize) -> Result<(), RackError> {
        let len = self.slots.len();
        for index in [from, to] {
            if index >= len {
                return Err(RackError::IndexOutOfRange { index, len });
            }
        }
        if from == to {
            return Ok(());
        }
        let hs = &self.slots[from];
        let (bypass, extra) = (hs.bypass, hs.extra.clone());
        // T-901: a window open on the moved slot reopens on its new instance.
        let reopen = match &hs.kind {
            Kind::Loaded(l) if l.editor.as_ref().is_some_and(|e| e.is_open()) => {
                l.editor_request.clone()
            }
            _ => None,
        };
        let kind = match &hs.kind {
            Kind::Loaded(l) => {
                let model =
                    SlotModel::new(&ModuleRef::of(&l.descriptor), bypass, &committed_state(l));
                match async_name(&self.registry, &model) {
                    Some(name) => Kind::Loading { model, name },
                    None => match self.registry.instantiate(&model, &self.config, to)? {
                        Resolved::Module(m) => Kind::Loaded(loaded_from(m)),
                        Resolved::Placeholder { message, too_new } => Kind::Placeholder {
                            model,
                            message,
                            too_new,
                            failed: false,
                        },
                    },
                }
            }
            Kind::Loading { model, name } => Kind::Loading {
                model: model.clone(),
                name: name.clone(),
            },
            Kind::Placeholder {
                model,
                message,
                too_new,
                failed,
            } => Kind::Placeholder {
                model: model.clone(),
                message: message.clone(),
                too_new: *too_new,
                failed: *failed,
            },
        };
        let hold = match &kind {
            Kind::Loaded(l) => l.latency,
            Kind::Placeholder { .. } | Kind::Loading { .. } => 0,
        };
        self.remove_with_hold(from, hold)?;
        let uid = self.alloc_uid();
        let pos = self.layout_pos_for(to);
        self.layout.insert(pos, LayoutEntry::new(uid));
        self.slots
            .insert(to, HostSlot::new(uid, bypass, extra, kind));
        if let Some(request) = reopen {
            self.reopen_when_loaded(to, request);
        }
        self.start_pending_loads();
        self.dirty = true;
        self.flush();
        self.check_latency();
        Ok(())
    }

    /// Per-slot host bypass (15 ms crossfade to the latency-matched dry path; honoured by
    /// offline renders through the model).
    pub fn set_bypass(&mut self, index: usize, bypassed: bool) -> Result<(), RackError> {
        let len = self.slots.len();
        let hs = self
            .slots
            .get_mut(index)
            .ok_or(RackError::IndexOutOfRange { index, len })?;
        hs.bypass = bypassed;
        let uid = hs.uid;
        if self.layout_pos(uid).is_some_and(|li| self.layout[li].sent) {
            self.send(RackCommand::SetBypass {
                slot: uid,
                bypassed,
            });
        }
        Ok(())
    }

    /// Whole-rack A/B (listening only; not part of the model, so offline renders ignore it).
    pub fn set_ab(&mut self, on: bool) {
        self.ab = on;
        self.send(RackCommand::SetAb(on));
    }

    /// Sets parameter `id` of slot `index` (plain value, `clamp_quantize`d here): updates the
    /// mirror, sends the event (offset 0 of the next block) and echoes
    /// [`RackNotice::ParamChanged`]. Returns the applied value.
    pub fn set_param(&mut self, index: usize, id: ParamId, value: f64) -> Result<f64, RackError> {
        let len = self.slots.len();
        let hs = self
            .slots
            .get_mut(index)
            .ok_or(RackError::IndexOutOfRange { index, len })?;
        let uid = hs.uid;
        let Kind::Loaded(l) = &mut hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        let pi = l
            .params
            .iter()
            .position(|p| p.id == id && !p.flags.contains(ParamFlags::READ_ONLY))
            .ok_or(RackError::UnknownParam { index, id })?;
        let v = l.params[pi].clamp_quantize(value);
        l.values[pi] = v;
        if self
            .layout_pos(uid)
            .is_some_and(|li| self.layout[li].sent && !self.layout[li].dying)
        {
            self.send(RackCommand::Param {
                slot: uid,
                id,
                value: v,
            });
        }
        self.notice_param(index, pi);
        Ok(v)
    }

    fn param_info(&self, index: usize, id: ParamId) -> Result<ParamInfo, RackError> {
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        l.params
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or(RackError::UnknownParam { index, id })
    }

    /// [`set_param`](Self::set_param) from a normalized position (sliders).
    pub fn set_param_normalized(
        &mut self,
        index: usize,
        id: ParamId,
        t: f64,
    ) -> Result<f64, RackError> {
        let p = self.param_info(index, id)?;
        self.set_param(index, id, p.from_normalized(t))
    }

    /// [`set_param`](Self::set_param) from typed text; unparseable text sends nothing.
    pub fn set_param_text(
        &mut self,
        index: usize,
        id: ParamId,
        text: &str,
    ) -> Result<f64, RackError> {
        let p = self.param_info(index, id)?;
        // A plugin that parses its own text (T-803) first, then the module API's rules.
        let own = match self.slots.get(index).map(|s| &s.kind) {
            Some(Kind::Loaded(l)) => l.text.clone(),
            _ => None,
        };
        let v = own
            .and_then(|t| t.text_to_value(id, text))
            .or_else(|| p.text_to_value(text))
            .ok_or_else(|| RackError::InvalidText(text.to_owned()))?;
        self.set_param(index, id, v)
    }

    /// Replaces slot `index`'s instance with one built from its committed state (mirror values
    /// and blob), ADR-005 §12. Used for restart requests (latency changes) and for "Restart" of a
    /// failed slot. The new instance is activated here and crossfades in over 15 ms once its
    /// output is valid (its latency); until then the old one keeps playing.
    ///
    /// For a slot that could not start when the rack was loaded ("Couldn't start …"), Restart
    /// resolves the stored slot again (SPEC-012 §2.9): on success the instance fades in from
    /// the dry path once its output is valid; on failure the error is returned and the slot
    /// stays as it was. Missing-module placeholders return [`RackError::NotLoaded`].
    ///
    /// A manual restart also resets the slot's automatic-restart budget ([`AUTO_RESTARTS`]).
    pub fn restart(&mut self, index: usize) -> Result<(), RackError> {
        if let Some(hs) = self.slots.get_mut(index) {
            hs.auto_restarts = 0;
            hs.restart_due = None;
        }
        self.restart_now(index)
    }

    /// [`restart`](Self::restart) without touching the automatic-restart budget.
    fn restart_now(&mut self, index: usize) -> Result<(), RackError> {
        if let Some(HostSlot {
            kind: Kind::Placeholder { failed: true, .. },
            ..
        }) = self.slots.get(index)
        {
            return self.retry_start(index);
        }
        self.replace_with(index, None)
    }

    /// [`restart`](Self::restart) of a slot that failed to start at load (a failed
    /// placeholder, index checked by the caller).
    fn retry_start(&mut self, index: usize) -> Result<(), RackError> {
        let hs = &self.slots[index];
        let uid = hs.uid;
        let Kind::Placeholder { model, .. } = &hs.kind else {
            unreachable!("checked by restart");
        };
        let mut model = model.clone();
        model.bypass = hs.bypass;
        if let Some(name) = async_name(&self.registry, &model) {
            // T-803: created off the control thread; the stand-in keeps passing dry until then.
            self.slots[index].kind = Kind::Loading { model, name };
            self.start_pending_loads();
            return Ok(());
        }
        let m = match self.registry.instantiate(&model, &self.config, index)? {
            Resolved::Module(m) => m,
            Resolved::Placeholder { message, .. } => {
                return Err(RackError::InvalidState {
                    id: model.module,
                    message,
                });
            }
        };
        self.slots[index].kind = Kind::Loaded(loaded_from(m));
        // The audio thread runs the placeholder: replace it (the old slot has no instance, so
        // the new one fades in from the undelayed input after its latency).
        if let Some(li) = self.layout_pos(uid)
            && self.layout[li].sent
        {
            self.layout[li].replace = true;
        }
        self.dirty = true;
        self.notices
            .push(RackNotice::SlotRestarted { slot: uid, index });
        self.flush();
        self.check_latency();
        Ok(())
    }

    /// H-40: rechecks every Missing/too-new placeholder (`failed: false` — a slot that couldn't
    /// *start* is a separate, manual [`Self::restart`] story) against the registry, which just
    /// registered or upgraded some module ids (install, rescan, unblock, re-enable). Called from
    /// [`tick`](Self::tick) only when [`Registry::generation`] moved since the last check.
    fn recover_missing(&mut self) {
        for k in 0..self.slots.len() {
            let hs = &self.slots[k];
            if hs.job.is_some() {
                continue;
            }
            let Kind::Placeholder {
                model,
                failed: false,
                ..
            } = &hs.kind
            else {
                continue;
            };
            if !model
                .module_ref()
                .is_ok_and(|r| self.registry.get(&r.id).is_some())
            {
                continue;
            }
            self.begin_recovery(k);
        }
    }

    /// Re-resolves placeholder slot `index` from its kept state and blob (the caller has already
    /// checked its module id now resolves): off the audio thread, through the normal async-load
    /// or instant-swap path and the usual T-103 crossfade. Success is
    /// [`RackNotice::SlotRecovered`] — never [`RackNotice::SlotRestarted`], which is reserved for
    /// a manual [`Self::restart`] (SPEC-004 OD-1 / the ADR-008 amendment: this is not an undoable
    /// edit, and the caller — `src-tauri`'s `DocumentService` — must not mark the document dirty
    /// for it either). A re-resolve that still can't produce a module (still not installed, still
    /// too new, or fails to start) leaves the slot a placeholder with its blob untouched; a start
    /// failure is shown as [`RackNotice::SlotFailed`], same as any other failed start.
    fn begin_recovery(&mut self, index: usize) {
        let hs = &self.slots[index];
        let uid = hs.uid;
        let Kind::Placeholder { model, .. } = &hs.kind else {
            return;
        };
        let mut model = model.clone();
        model.bypass = hs.bypass;
        if let Some(name) = async_name(&self.registry, &model) {
            self.slots[index].kind = Kind::Loading {
                model: model.clone(),
                name,
            };
            let id = self.spawn_load(uid, model, index);
            self.slots[index].job = Some(PendingJob {
                id,
                values: Vec::new(),
                blob: None,
                recovering: true,
            });
            return;
        }
        match self.registry.instantiate(&model, &self.config, index) {
            Ok(Resolved::Module(m)) => {
                let name = m.descriptor().name.text.clone();
                self.slots[index].kind = Kind::Loaded(loaded_from(m));
                // The audio thread runs the placeholder: replace it (no instance to crossfade
                // from, so the new one fades in from the undelayed input after its latency).
                if let Some(li) = self.layout_pos(uid)
                    && self.layout[li].sent
                {
                    self.layout[li].replace = true;
                }
                self.dirty = true;
                self.notices.push(RackNotice::SlotRecovered {
                    slot: uid,
                    index,
                    name,
                });
                self.flush();
                self.check_latency();
            }
            Ok(Resolved::Placeholder { message, too_new }) => {
                // Still not resolvable (e.g. still too new) — refresh the message, no notice.
                self.slots[index].kind = Kind::Placeholder {
                    model,
                    message,
                    too_new,
                    failed: false,
                };
            }
            Err(e) => {
                let message = start_failure(&model, e);
                self.slots[index].kind = Kind::Placeholder {
                    model,
                    message: message.clone(),
                    too_new: false,
                    failed: true,
                };
                self.notices.push(RackNotice::SlotFailed {
                    slot: uid,
                    index,
                    message,
                });
            }
        }
    }

    /// Replaces slot `index`'s instance with one loaded from `state` (a preset with a blob, a
    /// captured noise print, ADR-005 §12), through the same replacement crossfade as
    /// [`restart`](Self::restart). The mirror and the committed blob take the new state's values.
    pub fn replace_state(&mut self, index: usize, state: ModuleState) -> Result<(), RackError> {
        self.replace_with(index, Some(state))
    }

    fn replace_with(&mut self, index: usize, state: Option<ModuleState>) -> Result<(), RackError> {
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let uid = hs.uid;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        let state = state.unwrap_or_else(|| committed_state(l));
        let model = SlotModel::new(&ModuleRef::of(&l.descriptor), hs.bypass, &state);
        if async_name(&self.registry, &model).is_some() {
            // T-803: the replacement is created off the control thread; the current instance
            // keeps playing (or stays bypassed) until it arrives (`install_replacement`).
            let values = l.values.clone();
            let id = self.spawn_load(uid, model, index);
            self.slots[index].job = Some(PendingJob {
                id,
                values,
                blob: state.blob,
                recovering: false,
            });
            return Ok(());
        }
        let m = match self.registry.instantiate(&model, &self.config, index)? {
            Resolved::Module(m) => m,
            Resolved::Placeholder { message, .. } => {
                return Err(RackError::InvalidState {
                    id: l.descriptor.id.clone(),
                    message,
                });
            }
        };
        self.install_replacement(index, m, state.blob, None);
        Ok(())
    }

    /// Makes `m` (activated, built from the slot's committed or requested state) slot
    /// `index`'s next instance: mirror, blob and extension handles follow it, and it
    /// crossfades in once its output is valid. `requested`: the mirror when an asynchronous
    /// replacement was requested — values changed since then keep the mirror's value (they reach
    /// the new instance as events at its first sample).
    fn install_replacement(
        &mut self,
        index: usize,
        mut m: Box<dyn Module>,
        blob: Option<Vec<u8>>,
        requested: Option<&[f64]>,
    ) {
        let uid = self.slots[index].uid;
        let latency = m.latency_samples();
        let Kind::Loaded(l) = &mut self.slots[index].kind else {
            m.deactivate();
            return;
        };
        let mut changed = Vec::new();
        for (pi, p) in l.params.iter().enumerate() {
            if p.flags.contains(ParamFlags::READ_ONLY) {
                continue;
            }
            let edited = requested
                .and_then(|r| r.get(pi))
                .is_some_and(|v| v.to_bits() != l.values[pi].to_bits());
            if edited {
                continue;
            }
            let v = m.param_value(p.id).unwrap_or(p.default);
            if v.to_bits() != l.values[pi].to_bits() {
                l.values[pi] = v;
                changed.push(pi);
            }
        }
        l.blob = m.save_state().ok().and_then(|s| s.blob).or(blob);
        (l.telemetry, l.telemetry_channels) = telemetry_of(m.as_ref());
        l.health = adapter_health(m.as_ref());
        l.text = param_text(m.as_ref());
        // T-901: a window still open on the outgoing instance (a plugin-requested restart, a
        // preset or state load — not a crash, whose window died with its process) closes and
        // reopens on the new instance.
        let reopen = l
            .editor
            .as_ref()
            .filter(|e| e.is_open())
            .and(l.editor_request.clone());
        if let Some(old) = &l.editor {
            old.close();
        }
        l.editor = plugin_editor(m.as_ref()).filter(|e| e.available());
        if let (Some(e), Some(request)) = (&l.editor, reopen) {
            reopen_editor(e.clone(), request);
        }
        if let Some(mut old) = l.fresh.replace(m) {
            old.deactivate();
        }
        l.latency = latency;
        l.failed = None;
        if let Some(li) = self.layout_pos(uid)
            && self.layout[li].sent
        {
            self.layout[li].replace = true;
        }
        for pi in changed {
            self.notice_param(index, pi);
        }
        self.dirty = true;
        self.notices
            .push(RackNotice::SlotRestarted { slot: uid, index });
        self.flush();
        self.check_latency();
    }

    // --- Plugin editor windows (T-901, ADR-008 §7) ------------------------------------------

    /// Slot `index`'s editor handle, to open its window with `request` (the caller opens it off
    /// the control thread: opening waits for the plugin's GUI). Remembers `request` (a restart
    /// reopens the window with it) and that the window was used (saves capture the plugin's
    /// state first). [`RackError::NoEditor`] for a slot without a window of its own.
    pub fn editor_for_open(
        &mut self,
        index: usize,
        request: EditorRequest,
    ) -> Result<Arc<dyn PluginEditor>, RackError> {
        let len = self.slots.len();
        let hs = self
            .slots
            .get_mut(index)
            .ok_or(RackError::IndexOutOfRange { index, len })?;
        let name = match &hs.kind {
            Kind::Loaded(l) => l.descriptor.name.text.clone(),
            Kind::Loading { name, .. } => name.clone(),
            Kind::Placeholder { model, .. } => model.module.clone(),
        };
        match &mut hs.kind {
            Kind::Loaded(l) if l.failed.is_none() && hs.job.is_none() => {
                let editor = l.editor.clone().ok_or(RackError::NoEditor { name })?;
                l.editor_request = Some(request);
                l.editor_used = true;
                Ok(editor)
            }
            _ => Err(RackError::NoEditor { name }),
        }
    }

    /// Closes slot `index`'s editor window (no-op without one; never blocks).
    pub fn close_editor(&mut self, index: usize) -> Result<(), RackError> {
        let len = self.slots.len();
        let hs = self
            .slots
            .get(index)
            .ok_or(RackError::IndexOutOfRange { index, len })?;
        if let Kind::Loaded(l) = &hs.kind
            && let Some(e) = &l.editor
        {
            e.close();
        }
        self.poll_editors();
        Ok(())
    }

    /// Closes every plugin window (document close, app quit; never blocks).
    pub fn close_all_editors(&mut self) {
        for hs in &self.slots {
            if let Kind::Loaded(l) = &hs.kind
                && let Some(e) = &l.editor
            {
                e.close();
            }
        }
        self.poll_editors();
    }

    /// The editors whose plugin state a save should capture first ([`PluginEditor::
    /// capture_state`], one blocking round trip each — call it off the control thread and hand
    /// the results to [`Self::apply_plugin_state`]): slots whose window was opened at least once,
    /// so they may hold GUI-only state the committed blob doesn't have yet.
    pub fn editors_to_capture(&self) -> Vec<(SlotUid, Arc<dyn PluginEditor>)> {
        self.slots
            .iter()
            .filter_map(|hs| match &hs.kind {
                Kind::Loaded(l) if l.editor_used => l.editor.clone().map(|e| (hs.uid, e)),
                _ => None,
            })
            .collect()
    }

    /// Makes `blob` (the plugin's state, wrapped like its `save_state` blob) slot `uid`'s
    /// committed blob — a no-op for a removed slot or an unchanged blob;
    /// [`RackNotice::PluginStateChanged`] otherwise.
    pub fn apply_plugin_state(&mut self, uid: SlotUid, blob: Vec<u8>) {
        let Some(k) = self.index_of(uid) else {
            return;
        };
        if let Kind::Loaded(l) = &mut self.slots[k].kind
            && l.blob.as_ref() != Some(&blob)
        {
            l.blob = Some(blob);
            self.notices.push(RackNotice::PluginStateChanged {
                slot: uid,
                index: k,
            });
        }
    }

    /// Opens slot `k`'s window with `request` once it is loaded (now, if it is).
    fn reopen_when_loaded(&mut self, k: usize, request: EditorRequest) {
        let hs = &mut self.slots[k];
        match &mut hs.kind {
            Kind::Loaded(l) => {
                if let Some(e) = &l.editor {
                    l.editor_request = Some(request.clone());
                    l.editor_used = true;
                    reopen_editor(e.clone(), request);
                }
            }
            Kind::Loading { .. } => hs.reopen_editor = Some(request),
            Kind::Placeholder { .. } => {}
        }
    }

    /// Takes what every plugin window did since the last tick (T-901): parameters it changed
    /// while its plugin was inactive go to the mirror (like a module's output events), a
    /// GUI-only state change refreshes the committed blob, and an opened or closed window is
    /// reported.
    fn poll_editors(&mut self) {
        for k in 0..self.slots.len() {
            let uid = self.slots[k].uid;
            let Kind::Loaded(l) = &mut self.slots[k].kind else {
                continue;
            };
            let Some(editor) = l.editor.clone() else {
                continue;
            };
            let update = editor.poll();
            let mut changed = Vec::new();
            for (id, value) in update.params {
                if let Some(pi) = l.params.iter().position(|p| p.id == id) {
                    let p = &l.params[pi];
                    let v = if p.flags.contains(ParamFlags::READ_ONLY) {
                        value
                    } else {
                        p.clamp_quantize(value)
                    };
                    if v.is_finite() {
                        l.values[pi] = v;
                        changed.push(pi);
                    }
                }
            }
            let state_changed = match update.state {
                Some(blob) if l.blob.as_ref() != Some(&blob) => {
                    l.blob = Some(blob);
                    true
                }
                _ => false,
            };
            let open_changed = update.open != l.editor_reported_open;
            l.editor_reported_open = update.open;
            for pi in changed {
                self.notice_param(k, pi);
            }
            if state_changed {
                self.notices.push(RackNotice::PluginStateChanged {
                    slot: uid,
                    index: k,
                });
            }
            if open_changed {
                self.notices.push(RackNotice::EditorChanged {
                    slot: uid,
                    index: k,
                    open: update.open,
                });
            }
        }
    }

    // --- Presets (T-406, ADR-005 §10/§12, SPEC-012 §2.7) ------------------------------------

    /// The committed state of slot `index` (mirror values, `READ_ONLY` excluded, plus the
    /// committed blob) — what saving a module preset from a live slot captures. `None` blob
    /// means "no noise print"; callers that implement an "include noise print" checkbox clear
    /// [`ModuleState::blob`] themselves before storing it.
    pub fn slot_state(&self, index: usize) -> Result<ModuleState, RackError> {
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        Ok(committed_state(l))
    }

    /// The module id of slot `index` (registry key, no `@version`); `None` for a placeholder.
    pub fn slot_module_id(&self, index: usize) -> Option<String> {
        let Kind::Loaded(l) = &self.slots.get(index)?.kind else {
            return None;
        };
        Some(l.descriptor.id.clone())
    }

    /// Applies a module preset to slot `index` (SPEC-012 §2.7): a state with a blob replaces the
    /// instance behind the 15 ms crossfade, exactly like [`replace_state`](Self::replace_state)
    /// (a noise-reduction preset "includes the print", SPEC-014 §2.4). A state with no blob is
    /// **parameter-only** (ADR-005 §12): every parameter the preset defines is sent like a typed
    /// value — smoothed, with no restart — and the slot's own committed blob (if any) is left
    /// untouched. Keys the preset doesn't mention keep their current value.
    pub fn apply_module_preset(
        &mut self,
        index: usize,
        preset: &ModuleState,
    ) -> Result<(), RackError> {
        if preset.blob.is_some() {
            return self.replace_state(index, preset.clone());
        }
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        let targets: Vec<(ParamId, f64)> = l
            .params
            .iter()
            .filter(|p| !p.flags.contains(ParamFlags::READ_ONLY))
            .filter_map(|p| preset.params.get(&p.key).map(|&v| (p.id, v)))
            .collect();
        for (id, value) in targets {
            self.set_param(index, id, value)?;
        }
        Ok(())
    }

    /// Resets every writable parameter of slot `index` to its schema default, through the same
    /// parameter-only path as [`apply_module_preset`](Self::apply_module_preset) (smoothed, no
    /// restart, committed blob untouched — e.g. a Noise Reduction slot keeps its print).
    pub fn reset_to_default(&mut self, index: usize) -> Result<(), RackError> {
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        let params = l
            .params
            .iter()
            .filter(|p| !p.flags.contains(ParamFlags::READ_ONLY))
            .map(|p| (p.key.clone(), p.default))
            .collect();
        let defaults = ModuleState {
            format_version: l.descriptor.state_format_version,
            params,
            blob: None,
        };
        self.apply_module_preset(index, &defaults)
    }

    // --- Noise print capture (S3-06, SPEC-014 §2.3, §2.9) -----------------------------------

    /// The module's [`NoiseProfile`] handle, if any (`None` for a placeholder or a module without
    /// the extension). Safe to call `capture`/`describe` on from any thread (module docs).
    pub fn noise_profile_extension(&self, index: usize) -> Option<Arc<dyn NoiseProfile>> {
        match &self.slots.get(index)?.kind {
            Kind::Loaded(l) => l.noise_profile.clone(),
            Kind::Placeholder { .. } | Kind::Loading { .. } => None,
        }
    }

    // --- Response curve (S3-07, SPEC-015 §2.6.6) ---------------------------------------------

    /// The module's [`ResponseCurve`] handle, if any (`None` for a placeholder or a module
    /// without the extension). Safe to call `magnitude_db`/`component_magnitude_db` on from any
    /// thread (module docs) — it is a pure function of its arguments, not of which instance is
    /// currently live.
    pub fn response_curve_extension(&self, index: usize) -> Option<Arc<dyn ResponseCurve>> {
        match &self.slots.get(index)?.kind {
            Kind::Loaded(l) => l.response_curve.clone(),
            Kind::Placeholder { .. } | Kind::Loading { .. } => None,
        }
    }

    // --- Transfer curve (H-63, SPEC-016 §4.11) ------------------------------------------------

    /// The module's [`TransferCurve`] handle, if any (`None` for a placeholder or a module
    /// without the extension). Like [`Self::response_curve_extension`], every call on it is a
    /// pure function of its arguments, not of which instance is currently live.
    pub fn transfer_curve_extension(&self, index: usize) -> Option<Arc<dyn TransferCurve>> {
        match &self.slots.get(index)?.kind {
            Kind::Loaded(l) => l.transfer_curve.clone(),
            Kind::Placeholder { .. } | Kind::Loading { .. } => None,
        }
    }

    // --- Telemetry (H-03, SPEC-016 §4.12 module telemetry) ----------------------------------

    /// Reads every running slot's [`Telemetry`] values, in rack order: the control thread's meter
    /// publisher calls this at the telemetry rate. The host is the single reader of each handle
    /// (ADR-005 §11), so a read ends the channels' Min/Max hold period. Slots without the
    /// extension, failed slots and placeholders are left out. Wait-free on the audio side (the
    /// module writes atomics once per block).
    pub fn read_telemetry(&self) -> Vec<SlotTelemetry> {
        self.slots
            .iter()
            .filter_map(|hs| match &hs.kind {
                Kind::Loaded(l) if l.failed.is_none() => {
                    let t = l.telemetry.as_ref()?;
                    Some(SlotTelemetry {
                        uid: hs.uid,
                        values: (0..l.telemetry_channels.len()).map(|i| t.read(i)).collect(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// The first slot exposing [`NoiseProfile`] that matches `hint` (Capture Noise Print's
    /// "last-focused NR slot"), else the first such slot in rack order, else `None`.
    pub fn find_noise_profile_slot(&self, hint: Option<usize>) -> Option<usize> {
        let has_profile =
            |hs: &HostSlot| matches!(&hs.kind, Kind::Loaded(l) if l.noise_profile.is_some());
        if let Some(h) = hint
            && self.slots.get(h).is_some_and(has_profile)
        {
            return Some(h);
        }
        self.slots.iter().position(has_profile)
    }

    /// [`find_noise_profile_slot`](Self::find_noise_profile_slot), inserting a default instance
    /// of `module_id` as the first slot when none exists (SPEC-014 §2.3 "target slot"). Returns
    /// the resolved index and whether a slot was inserted (the caller shows "‹Module› added to
    /// the rack" only then).
    pub fn resolve_or_insert_noise_profile_slot(
        &mut self,
        hint: Option<usize>,
        module_id: &str,
    ) -> Result<(usize, bool), RackError> {
        if let Some(i) = self.find_noise_profile_slot(hint) {
            return Ok((i, false));
        }
        self.insert_module(0, module_id)?;
        Ok((0, true))
    }

    /// The committed state of every slot strictly before `index` (bypass flags included),
    /// **omitting placeholders** — they pass dry at latency 0 live, so omitting them from an
    /// offline pre-render is equivalent (SPEC-014 §2.3). Used to render the audio a target NR
    /// slot receives, for noise-print capture.
    pub fn upstream_model(&self, index: usize) -> RackModel {
        let end = index.min(self.slots.len());
        RackModel {
            slots: self.slots[..end]
                .iter()
                .filter_map(|hs| match &hs.kind {
                    Kind::Loaded(l) => {
                        let mut m = SlotModel::new(
                            &ModuleRef::of(&l.descriptor),
                            hs.bypass,
                            &committed_state(l),
                        );
                        m.extra = hs.extra.clone();
                        Some(m)
                    }
                    Kind::Placeholder { .. } | Kind::Loading { .. } => None,
                })
                .collect(),
        }
    }

    /// Replaces slot `index`'s instance with one holding `blob` as its noise print, keeping every
    /// other committed value (SPEC-014 §2.3 "Result"): a live replacement through the same
    /// crossfade as [`restart`](Self::restart)/[`replace_state`](Self::replace_state).
    pub fn replace_noise_print(&mut self, index: usize, blob: Vec<u8>) -> Result<(), RackError> {
        let hs = self.slots.get(index).ok_or(RackError::IndexOutOfRange {
            index,
            len: self.slots.len(),
        })?;
        let Kind::Loaded(l) = &hs.kind else {
            return Err(RackError::NotLoaded { index });
        };
        let mut state = committed_state(l);
        state.blob = Some(blob);
        self.replace_with(index, Some(state))
    }

    /// Replaces the whole rack (document open): every current slot fades out, the new slots
    /// fade in; placeholders for missing modules; the A/B flag is reset to off.
    pub fn load_model(&mut self, model: &RackModel) -> Result<(), RackError> {
        if model.slots.len() > MAX_SLOTS {
            return Err(RackError::TooManySlots);
        }
        while !self.slots.is_empty() {
            self.remove(0)?;
        }
        for (i, s) in model.slots.iter().enumerate() {
            let uid = self.alloc_uid();
            let kind = resolve_lenient(&self.registry, &self.config, s, i);
            self.slots
                .push(HostSlot::new(uid, s.bypass, s.extra.clone(), kind));
            self.layout.push(LayoutEntry::new(uid));
        }
        self.start_pending_loads();
        self.set_ab(false);
        self.dirty = true;
        // H-40: every slot was just resolved fresh against the current registry — nothing to
        // recover until it changes again.
        self.registry_generation = self.registry.generation();
        self.flush();
        self.check_latency();
        Ok(())
    }

    fn on_event(&mut self, e: RackEvent) {
        match e {
            RackEvent::ParamReport { slot, id, value } => {
                let Some(k) = self.index_of(slot) else {
                    return;
                };
                let Kind::Loaded(l) = &mut self.slots[k].kind else {
                    return;
                };
                if let Some(pi) = l.params.iter().position(|p| p.id == id) {
                    l.values[pi] = value;
                    self.notice_param(k, pi);
                }
            }
            RackEvent::RestartRequested { slot } => {
                let Some(k) = self.index_of(slot) else {
                    return;
                };
                if self.slots[k].job.is_some()
                    || self
                        .layout_pos(slot)
                        .is_some_and(|li| self.layout[li].replace)
                {
                    // A replacement is already on its way.
                    return;
                }
                if let Err(err) = self.restart_now(k) {
                    self.notices.push(RackNotice::SlotFailed {
                        slot,
                        index: k,
                        message: err.to_string(),
                    });
                }
            }
            RackEvent::SlotFailed { slot, reason } => {
                let Some(k) = self.index_of(slot) else {
                    return;
                };
                let HostSlot {
                    kind,
                    auto_restarts,
                    restart_due,
                    ..
                } = &mut self.slots[k];
                let Kind::Loaded(l) = kind else {
                    return;
                };
                let name = &l.descriptor.name.text;
                let message = match reason {
                    FailReason::NonFinite => {
                        format!("{name} produced invalid audio and was bypassed")
                    }
                    FailReason::ModuleError => match l.health.as_ref().and_then(|h| h.fault()) {
                        Some(why) => format!("{name} {why} and was bypassed"),
                        None => format!("{name} stopped working and was bypassed"),
                    },
                };
                l.failed = Some(message.clone());
                // T-802 restart policy: an out-of-process module that failed on its own
                // (crash, hang — not invalid audio) restarts once with its last committed state.
                if reason == FailReason::ModuleError
                    && l.health.is_some()
                    && *auto_restarts < AUTO_RESTARTS
                {
                    *restart_due = Some(Instant::now() + AUTO_RESTART_DELAY);
                }
                self.notices.push(RackNotice::SlotFailed {
                    slot,
                    index: k,
                    message,
                });
            }
            RackEvent::SlotRemoved { slot } => {
                if let Some(li) = self.layout_pos(slot) {
                    self.layout[li].dead = true;
                    self.dirty = true;
                }
            }
            RackEvent::ReplaceDone { slot } => {
                if let Some(li) = self.layout_pos(slot) {
                    self.layout[li].aux_pending = false;
                    self.layout[li].aux_done = true;
                    self.dirty = true;
                }
            }
        }
    }

    /// The control tick (ADR-002 §1, every ~16 ms): sends queued commands, deactivates and drops
    /// retired chains, drains the RT events into the mirror (restart requests trigger a
    /// replacement), sends a pending chain, and returns the notices since the last call.
    pub fn tick(&mut self) -> Vec<RackNotice> {
        self.pump_backlog();
        while let Ok(mut chain) = self.link.retired.pop() {
            chain.deactivate();
            drop(chain);
            self.in_flight = self.in_flight.saturating_sub(1);
        }
        while let Ok(e) = self.link.events.pop() {
            self.on_event(e);
        }
        self.drain_loads();
        self.poll_editors();
        self.run_due_restarts(Instant::now());
        // H-40: cheap poll — only worth walking the slots when the registry actually changed
        // since the last tick (install, rescan, unblock, re-enable all bump its generation).
        let generation = self.registry.generation();
        if generation != self.registry_generation {
            self.registry_generation = generation;
            self.recover_missing();
        }
        self.flush();
        self.pump_backlog();
        self.check_latency();
        std::mem::take(&mut self.notices)
    }

    /// Performs the automatic restarts that are due (T-802 restart policy). A restart that fails
    /// leaves the slot "Failed" with the reason.
    fn run_due_restarts(&mut self, now: Instant) {
        for k in 0..self.slots.len() {
            let hs = &mut self.slots[k];
            if !hs.restart_due.is_some_and(|t| t <= now) {
                continue;
            }
            hs.restart_due = None;
            hs.auto_restarts += 1;
            let uid = hs.uid;
            if let Err(err) = self.replace_with(k, None)
                && let Kind::Loaded(l) = &mut self.slots[k].kind
            {
                let message = format!("Couldn't restart {}: {err}", l.descriptor.name.text);
                l.failed = Some(message.clone());
                self.notices.push(RackNotice::SlotFailed {
                    slot: uid,
                    index: k,
                    message,
                });
            }
        }
    }

    /// Notices produced since the last [`tick`](Self::tick) (without ticking).
    pub fn take_notices(&mut self) -> Vec<RackNotice> {
        std::mem::take(&mut self.notices)
    }

    /// Shuts the live rack down on the control thread once the audio thread has let go of
    /// `live` (stream stopped): deactivates and drops every chain — installed, retired, queued —
    /// and every instance not yet handed over.
    pub fn teardown(mut self, live: LiveRack) {
        // T-901: no plugin window outlives the rack.
        self.close_all_editors();
        for mut chain in live.into_chains() {
            chain.deactivate();
        }
        while let Ok(mut chain) = self.link.retired.pop() {
            chain.deactivate();
        }
        for cmd in self.backlog.drain(..) {
            if let RackCommand::Swap(mut chain) = cmd {
                chain.deactivate();
            }
        }
        for hs in &mut self.slots {
            if let Kind::Loaded(l) = &mut hs.kind
                && let Some(mut m) = l.fresh.take()
            {
                m.deactivate();
            }
        }
    }
}
