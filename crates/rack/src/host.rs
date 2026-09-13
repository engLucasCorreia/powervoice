//! The control-thread side of the live rack: [`RackHost`] owns the authoritative rack model and
//! the parameter mirror (ADR-005 §7), builds and activates chains off the audio thread, sends
//! commands, drains the RT event ring and the return ring (ADR-002 §1 control tick).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rtrb::PushError;
use serde_json::Map;
use vox_module_api::{
    ActivateConfig, Module, ModuleDescriptor, ModuleRef, ModuleState, ParamEvent, ParamFlags,
    ParamId, ParamInfo,
};

use crate::chain::PlanEntry;
use crate::delay::DelayLine;
use crate::live::RackLink;
use crate::slot::{Slot, SlotInit};
use crate::{
    Chain, FailReason, LiveRack, MAX_SLOTS, MAX_SWAPS_IN_FLIGHT, RackCommand, RackError, RackEvent,
    RackModel, RackOptions, Registry, Resolved, SlotModel, SlotUid,
};

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
}

/// Metadata of one slot (copied at insertion, ADR-005 §7).
#[derive(Clone, Debug)]
pub struct SlotInfo {
    /// Identity.
    pub uid: SlotUid,
    /// `"id@version"`.
    pub module: String,
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
}

struct Loaded {
    descriptor: ModuleDescriptor,
    params: Arc<[ParamInfo]>,
    /// The parameter mirror, index-aligned with `params` (READ_ONLY values as reported).
    values: Vec<f64>,
    /// The committed state blob.
    blob: Option<Vec<u8>>,
    latency: u32,
    failed: Option<String>,
    /// An activated instance not yet handed to the audio thread (insert or replacement).
    fresh: Option<Box<dyn Module>>,
}

enum Kind {
    Loaded(Box<Loaded>),
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
    let values = params
        .iter()
        .map(|p| module.param_value(p.id).unwrap_or(p.default))
        .collect();
    let blob = module.save_state().ok().and_then(|s| s.blob);
    Box::new(Loaded {
        descriptor: module.descriptor().clone(),
        params,
        values,
        blob,
        latency: module.latency_samples(),
        failed: None,
        fresh: Some(module),
    })
}

/// The host slot kind for a stored slot, never failing: missing modules and too-new states
/// become placeholders, modules that cannot start become failed placeholders (kept verbatim).
fn resolve_lenient(
    registry: &Registry,
    config: &ActivateConfig,
    s: &SlotModel,
    index: usize,
) -> Kind {
    match registry.instantiate(s, config, index) {
        Ok(Resolved::Module(m)) => Kind::Loaded(loaded_from(m)),
        Ok(Resolved::Placeholder { message, too_new }) => Kind::Placeholder {
            model: s.clone(),
            message,
            too_new,
            failed: false,
        },
        Err(e) => {
            let message = match e {
                RackError::Activate { .. } | RackError::Create { .. } => e.to_string(),
                other => format!("Couldn't start {}: {other}", s.module),
            };
            Kind::Placeholder {
                model: s.clone(),
                message,
                too_new: false,
                failed: true,
            }
        }
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
            Kind::Placeholder { .. } => 0,
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
            slots.push(HostSlot {
                uid,
                bypass: s.bypass,
                extra: s.extra.clone(),
                kind: resolve_lenient(&registry, &config, s, i),
            });
            layout.push(LayoutEntry::new(uid));
        }
        let (chain, _) = build_chain(&mut slots, &mut layout, &config, options, fade_len, false);
        let total = total_latency(&slots);
        let ab_capacity = (total as usize).max((config.sample_rate * 0.1).round() as usize);
        let (live, link) = LiveRack::new(Box::new(chain), ab_capacity);
        let host = Self {
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
        };
        Ok((host, live))
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
                name: l.descriptor.name.text.clone(),
                bypass: hs.bypass,
                latency_samples: l.latency,
                status: match &l.failed {
                    Some(message) => SlotStatus::Failed {
                        message: message.clone(),
                    },
                    None => SlotStatus::Active,
                },
                params: l.params.clone(),
            },
            Kind::Placeholder {
                model,
                message,
                too_new,
                failed,
            } => SlotInfo {
                uid: hs.uid,
                module: model.module.clone(),
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
            },
        })
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
    /// or a full outbox).
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
                    Kind::Placeholder { model, .. } => {
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
        let notice = RackNotice::ParamChanged {
            slot: uid,
            index,
            id: pid,
            value: v,
            normalized: p.to_normalized(v),
            text: p.value_to_text(v),
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
        let kind = match self.registry.instantiate(&slot, &self.config, index)? {
            Resolved::Module(m) => Kind::Loaded(loaded_from(m)),
            Resolved::Placeholder { message, too_new } => Kind::Placeholder {
                model: slot.clone(),
                message,
                too_new,
                failed: false,
            },
        };
        let uid = self.alloc_uid();
        let pos = self.layout_pos_for(index);
        self.layout.insert(pos, LayoutEntry::new(uid));
        self.slots.insert(
            index,
            HostSlot {
                uid,
                bypass: slot.bypass,
                extra: slot.extra,
                kind,
            },
        );
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
        if let Kind::Loaded(l) = &mut hs.kind
            && let Some(mut m) = l.fresh.take()
        {
            m.deactivate();
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
        let kind = match &hs.kind {
            Kind::Loaded(l) => {
                let model =
                    SlotModel::new(&ModuleRef::of(&l.descriptor), bypass, &committed_state(l));
                match self.registry.instantiate(&model, &self.config, to)? {
                    Resolved::Module(m) => Kind::Loaded(loaded_from(m)),
                    Resolved::Placeholder { message, too_new } => Kind::Placeholder {
                        model,
                        message,
                        too_new,
                        failed: false,
                    },
                }
            }
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
            Kind::Placeholder { .. } => 0,
        };
        self.remove_with_hold(from, hold)?;
        let uid = self.alloc_uid();
        let pos = self.layout_pos_for(to);
        self.layout.insert(pos, LayoutEntry::new(uid));
        self.slots.insert(
            to,
            HostSlot {
                uid,
                bypass,
                extra,
                kind,
            },
        );
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
        let v = p
            .text_to_value(text)
            .ok_or_else(|| RackError::InvalidText(text.to_owned()))?;
        self.set_param(index, id, v)
    }

    /// Replaces slot `index`'s instance with one built from its committed state (mirror values
    /// and blob), ADR-005 §12. Used for restart requests (latency changes) and for "Restart" of a
    /// failed slot. The new instance is activated here and crossfades in over 15 ms once its
    /// output is valid (its latency); until then the old one keeps playing.
    pub fn restart(&mut self, index: usize) -> Result<(), RackError> {
        self.replace_with(index, None)
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
        let m = match self.registry.instantiate(&model, &self.config, index)? {
            Resolved::Module(m) => m,
            Resolved::Placeholder { message, .. } => {
                return Err(RackError::InvalidState {
                    id: l.descriptor.id.clone(),
                    message,
                });
            }
        };
        let latency = m.latency_samples();
        let Kind::Loaded(l) = &mut self.slots[index].kind else {
            unreachable!("checked above");
        };
        let mut changed = Vec::new();
        for (pi, p) in l.params.iter().enumerate() {
            if p.flags.contains(ParamFlags::READ_ONLY) {
                continue;
            }
            let v = m.param_value(p.id).unwrap_or(p.default);
            if v.to_bits() != l.values[pi].to_bits() {
                l.values[pi] = v;
                changed.push(pi);
            }
        }
        l.blob = m.save_state().ok().and_then(|s| s.blob).or(state.blob);
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
        Ok(())
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
            self.slots.push(HostSlot {
                uid,
                bypass: s.bypass,
                extra: s.extra.clone(),
                kind,
            });
            self.layout.push(LayoutEntry::new(uid));
        }
        self.set_ab(false);
        self.dirty = true;
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
                if self
                    .layout_pos(slot)
                    .is_some_and(|li| self.layout[li].replace)
                {
                    return;
                }
                if let Err(err) = self.restart(k) {
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
                let Kind::Loaded(l) = &mut self.slots[k].kind else {
                    return;
                };
                let name = &l.descriptor.name.text;
                let message = match reason {
                    FailReason::NonFinite => {
                        format!("{name} produced invalid audio and was bypassed")
                    }
                    FailReason::ModuleError => format!("{name} stopped working and was bypassed"),
                };
                l.failed = Some(message.clone());
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
        self.flush();
        self.pump_backlog();
        self.check_latency();
        std::mem::take(&mut self.notices)
    }

    /// Notices produced since the last [`tick`](Self::tick) (without ticking).
    pub fn take_notices(&mut self) -> Vec<RackNotice> {
        std::mem::take(&mut self.notices)
    }

    /// Shuts the live rack down on the control thread once the audio thread has let go of
    /// `live` (stream stopped): deactivates and drops every chain — installed, retired, queued —
    /// and every instance not yet handed over.
    pub fn teardown(mut self, live: LiveRack) {
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
