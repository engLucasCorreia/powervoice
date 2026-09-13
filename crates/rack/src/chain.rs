//! [`Chain`]: an ordered list of slots processed in series (ADR-002's `SwapChain(Box<Chain>)`,
//! ADR-001 §5's `rack::Chain`). The same code runs realtime (inside a [`LiveRack`]) and offline
//! ([`offline::render`]).
//!
//! [`LiveRack`]: crate::LiveRack
//! [`offline::render`]: crate::offline::render

use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ParamEvent, ProcessStatus, Tail, Transport,
    validate_schema,
};

use crate::delay::DelayLine;
use crate::shim::DualMonoShim;
use crate::slot::{Outbox, Slot, SlotInit};
use crate::{
    FailReason, MAX_SLOTS, PushEventError, RackError, RackEvent, RackOptions, SlotUid,
    xfade_samples,
};

/// Capacity of a chain's RT event outbox (drained after every `process` call).
const OUTBOX_CAPACITY: usize = 1024;

/// How the live rack fills a slot of a newly built chain at the swap (built off-thread by the
/// rack host, executed on the audio thread with pointer moves only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanEntry {
    /// The slot was built with a fresh instance (a live insert).
    Fresh,
    /// Move the live slot `uid` (instance, queue, dry line, fade state) into this position;
    /// `drop_aux` leaves its finished outgoing instance behind in the retired chain.
    Keep { uid: SlotUid, drop_aux: bool },
    /// The slot holds a replacement instance; the live slot `uid`'s instance crossfades out.
    Replace { uid: SlotUid },
}

/// An ordered chain of modules.
///
/// Lifecycle: build it on the control thread ([`insert`](Self::insert) /
/// [`remove`](Self::remove) / [`move_slot`](Self::move_slot)), [`activate`](Self::activate) it,
/// then hand it to the audio thread (or an offline render), which only calls the RT-safe
/// [`process`](Self::process), [`reset`](Self::reset), [`push_event`](Self::push_event) and
/// [`set_bypass`](Self::set_bypass). Deactivate it after it comes back.
///
/// Every slot has a host-owned, latency-matched dry path and a 15 ms linear equal-gain
/// crossfade (bypass, live insert/remove, replacement), a non-finite guard, and an event queue
/// with same-`(offset, id)` coalescing and carry-over.
pub struct Chain {
    pub(crate) slots: Vec<Slot>,
    options: RackOptions,
    config: Option<ActivateConfig>,
    fade_len: u32,
    ping: Vec<f32>,
    pong: Vec<f32>,
    pub(crate) outbox: Outbox,
    next_uid: u64,
    pub(crate) plan: Vec<PlanEntry>,
    pub(crate) spare_ab: Option<DelayLine>,
    /// Total latency once the plan is executed (set by the rack host).
    pub(crate) expected_latency: Option<u32>,
    failure: Option<(usize, FailReason)>,
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain {
    /// Empty, inactive chain with default capacities.
    pub fn new() -> Self {
        Self::with_options(RackOptions::default())
    }

    /// Empty, inactive chain.
    pub fn with_options(options: RackOptions) -> Self {
        Self {
            slots: Vec::new(),
            options,
            config: None,
            fade_len: 0,
            ping: Vec::new(),
            pong: Vec::new(),
            outbox: Outbox::with_capacity(OUTBOX_CAPACITY),
            next_uid: 1,
            plan: Vec::new(),
            spare_ab: None,
            expected_latency: None,
            failure: None,
        }
    }

    /// \[control thread\] An empty, **active** chain (buffers allocated) that the rack host
    /// fills with already-active slots.
    pub(crate) fn assembled(config: &ActivateConfig, options: RackOptions) -> Self {
        let mut c = Self::with_options(options);
        c.fade_len = xfade_samples(config.sample_rate);
        c.ping = vec![0.0; config.max_block as usize];
        c.pong = vec![0.0; config.max_block as usize];
        c.config = Some(*config);
        c
    }

    pub(crate) fn fade_len(&self) -> u32 {
        self.fade_len
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True if there are no slots.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The configuration while active.
    pub fn config(&self) -> Option<ActivateConfig> {
        self.config
    }

    /// True between `activate` and `deactivate`.
    pub fn is_active(&self) -> bool {
        self.config.is_some()
    }

    /// The module in slot `index` (control thread, chain not live). `None` for a placeholder.
    pub fn module(&self, index: usize) -> Option<&dyn Module> {
        self.slots.get(index).and_then(|s| s.module.as_deref())
    }

    /// Identity of slot `index`.
    pub fn slot_uid(&self, index: usize) -> Option<SlotUid> {
        self.slots.get(index).map(|s| s.uid)
    }

    /// Display name of slot `index`'s module.
    pub fn slot_name(&self, index: usize) -> Option<&str> {
        self.slots.get(index).map(|s| s.name.as_str())
    }

    pub(crate) fn find(&self, uid: SlotUid) -> Option<usize> {
        self.slots.iter().position(|s| s.uid == uid)
    }

    /// \[control thread\] Inserts `module` (inactive) at `index` (`0..=len`). Modules without a
    /// MONO layout are wrapped in the dual-mono shim (ADR-005 §8); anything else is refused.
    /// The schema must be valid. If the chain is active, the module is activated with its
    /// configuration.
    pub fn insert(&mut self, index: usize, module: Box<dyn Module>) -> Result<(), RackError> {
        if index > self.slots.len() {
            return Err(RackError::IndexOutOfRange {
                index,
                len: self.slots.len(),
            });
        }
        if self.slots.len() >= MAX_SLOTS {
            return Err(RackError::TooManySlots);
        }
        let module = DualMonoShim::adapt(module).map_err(|m| RackError::UnsupportedLayout {
            id: m.descriptor().id.clone(),
            name: m.descriptor().name.text.clone(),
        })?;
        let id = module.descriptor().id.clone();
        let name = module.descriptor().name.text.clone();
        validate_schema(module.params(), module.groups()).map_err(|source| RackError::Schema {
            id: id.clone(),
            source,
        })?;
        let uid = SlotUid(self.next_uid);
        self.next_uid += 1;
        let mut slot = Slot::new(uid, Some(module), name.clone(), &self.options);
        if let Some(cfg) = self.config {
            slot.activate(&cfg, self.fade_len, SlotInit::default())
                .map_err(|source| RackError::Activate {
                    index,
                    id,
                    name,
                    source,
                })?;
        }
        self.slots.insert(index, slot);
        Ok(())
    }

    /// \[control thread\] Appends `module` (see [`insert`](Self::insert)).
    pub fn push(&mut self, module: Box<dyn Module>) -> Result<(), RackError> {
        self.insert(self.slots.len(), module)
    }

    /// \[control thread\] Removes and returns the module at `index`, deactivated if the chain is
    /// active. Its queued events are discarded.
    pub fn remove(&mut self, index: usize) -> Result<Box<dyn Module>, RackError> {
        if index >= self.slots.len() {
            return Err(RackError::IndexOutOfRange {
                index,
                len: self.slots.len(),
            });
        }
        let mut slot = self.slots.remove(index);
        if self.config.is_some() {
            slot.deactivate();
        }
        slot.module.take().ok_or(RackError::NotLoaded { index })
    }

    /// \[control thread\] Moves the slot at `from` to position `to` (both `< len`); the others
    /// shift. Queued events move with the slot.
    pub fn move_slot(&mut self, from: usize, to: usize) -> Result<(), RackError> {
        let len = self.slots.len();
        for index in [from, to] {
            if index >= len {
                return Err(RackError::IndexOutOfRange { index, len });
            }
        }
        let slot = self.slots.remove(from);
        self.slots.insert(to, slot);
        Ok(())
    }

    /// \[control thread\] Activates every module and allocates the buffers. The v1 rack runs
    /// MONO. On error the modules activated so far are deactivated again.
    pub fn activate(&mut self, config: &ActivateConfig) -> Result<(), RackError> {
        if self.config.is_some() {
            return Err(RackError::AlreadyActive);
        }
        if config.max_block == 0 {
            return Err(RackError::InvalidConfig("max_block must be >= 1"));
        }
        if !(config.sample_rate.is_finite() && config.sample_rate > 0.0) {
            return Err(RackError::InvalidConfig(
                "sample_rate must be finite and > 0",
            ));
        }
        if config.layout != ChannelLayout::MONO {
            return Err(RackError::InvalidConfig("the v1 rack runs MONO"));
        }
        let fade_len = xfade_samples(config.sample_rate);
        for index in 0..self.slots.len() {
            let init = SlotInit {
                bypassed: self.slots[index].bypassed,
                fade_in: false,
            };
            if let Err(source) = self.slots[index].activate(config, fade_len, init) {
                for slot in &mut self.slots[..index] {
                    slot.deactivate();
                }
                let (id, name) = self.slots[index].module.as_ref().map_or_else(
                    || (String::new(), String::new()),
                    |m| (m.descriptor().id.clone(), m.descriptor().name.text.clone()),
                );
                return Err(RackError::Activate {
                    index,
                    id,
                    name,
                    source,
                });
            }
        }
        self.fade_len = fade_len;
        self.ping = vec![0.0; config.max_block as usize];
        self.pong = vec![0.0; config.max_block as usize];
        self.config = Some(*config);
        self.failure = None;
        Ok(())
    }

    /// \[control thread\] Deactivates every module (no-op if inactive).
    pub fn deactivate(&mut self) {
        if self.config.take().is_some() {
            for slot in &mut self.slots {
                slot.deactivate();
            }
        }
    }

    /// Total latency = sum of slot latencies, bypassed slots included (their dry path is
    /// latency-matched), placeholders and departing slots 0. 0 while inactive.
    pub fn latency_samples(&self) -> u32 {
        self.slots
            .iter()
            .fold(0u32, |acc, s| acc.saturating_add(s.contributing_latency()))
    }

    /// Latency of slot `index` (read at activation).
    pub fn slot_latency_samples(&self, index: usize) -> Option<u32> {
        self.slots.get(index).map(|s| s.latency)
    }

    /// Tail of slot `index` (read at activation).
    pub fn slot_tail(&self, index: usize) -> Option<Tail> {
        self.slots.get(index).map(|s| s.tail)
    }

    /// True once the module in slot `index` requested [`HostRequest::Restart`].
    ///
    /// [`HostRequest::Restart`]: vox_module_api::HostRequest::Restart
    pub fn restart_requested(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|s| s.restart_requested)
    }

    /// Why slot `index` failed (non-finite output or a module error), if it did.
    pub fn slot_failed(&self, index: usize) -> Option<FailReason> {
        self.slots.get(index).and_then(|s| s.failed)
    }

    /// The bypass flag of slot `index`.
    pub fn is_bypassed(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|s| s.bypassed)
    }

    /// \[control thread, chain not live\] Sets slot `index`'s bypass flag without a crossfade
    /// (building a chain from a saved rack).
    pub fn set_bypass_now(&mut self, index: usize, bypassed: bool) -> Result<(), RackError> {
        let len = self.slots.len();
        self.slots
            .get_mut(index)
            .ok_or(RackError::IndexOutOfRange { index, len })?
            .set_bypass_now(bypassed);
        Ok(())
    }

    /// RT-safe. Host bypass toggle of slot `index` with the 15 ms crossfade (or an event to the
    /// module's own BYPASS parameter).
    pub fn set_bypass(&mut self, index: usize, bypassed: bool) -> Result<(), PushEventError> {
        self.slots
            .get_mut(index)
            .ok_or(PushEventError::NoSuchSlot(index))?
            .set_bypass(bypassed);
        Ok(())
    }

    pub(crate) fn set_bypass_uid(&mut self, uid: SlotUid, bypassed: bool) {
        if let Some(i) = self.find(uid) {
            self.slots[i].set_bypass(bypassed);
        }
    }

    pub(crate) fn begin_remove_uid(&mut self, uid: SlotUid, hold: u32) {
        if let Some(i) = self.find(uid) {
            self.slots[i].begin_remove(hold);
        }
    }

    /// RT-safe. Queues a parameter event for slot `index`, for the **next** `process()` call:
    /// `offset` is relative to that call's first sample; values must already be
    /// `clamp_quantize`d. Events must be pushed in non-decreasing offset order. An event with
    /// the same `(offset, id)` as an already queued one overwrites its value in place (SPEC-012
    /// §4.2). An offset at or past the next call's length is delivered at offset 0 of the call
    /// after it. `Err(Full)`: the caller keeps the event and retries next block.
    pub fn push_event(&mut self, index: usize, event: ParamEvent) -> Result<(), PushEventError> {
        self.slots
            .get_mut(index)
            .ok_or(PushEventError::NoSuchSlot(index))?
            .push_event(event)
            .map_err(PushEventError::List)
    }

    /// [`push_event`](Self::push_event) addressed by slot identity.
    pub fn push_event_to(&mut self, uid: SlotUid, event: ParamEvent) -> Result<(), PushEventError> {
        let index = self
            .find(uid)
            .ok_or(PushEventError::NoSuchSlot(usize::MAX))?;
        self.push_event(index, event)
    }

    /// RT-safe. Resets every module and dry path (seek, loop wrap, transport start). Queued
    /// events and crossfades stay.
    pub fn reset(&mut self) {
        if self.config.is_some() {
            for slot in &mut self.slots {
                slot.reset();
            }
        }
    }

    /// RT-safe. The first slot failure since the last call: `(index, reason)`. Offline renders
    /// abort on it (SPEC-012 §2.9).
    pub fn take_failure(&mut self) -> Option<(usize, FailReason)> {
        self.failure.take()
    }

    /// RT-safe. Hands the RT events collected during `process` (module reports, restart
    /// requests, failures, finished transitions) to `f` and clears them.
    pub fn drain_events(&mut self, f: impl FnMut(RackEvent)) {
        self.outbox.drain(f);
    }

    /// RT events dropped because the outbox was full (module reports beyond the lifecycle
    /// reserve), since the last [`take_dropped_events`](Self::take_dropped_events).
    pub fn dropped_events(&self) -> u64 {
        self.outbox.dropped
    }

    /// RT-safe. [`dropped_events`](Self::dropped_events), resetting the count.
    pub fn take_dropped_events(&mut self) -> u64 {
        std::mem::take(&mut self.outbox.dropped)
    }

    /// RT-safe. Runs `input` through the chain into `output` (equal lengths; any length —
    /// blocks longer than `max_block` are split into sub-blocks, and each slot's queued events
    /// are split at the sub-block boundaries, keeping their exact sample positions). A
    /// zero-length call is a parameter flush. If an event list fills up, the remaining events
    /// are carried to offset 0 of the next sub-block or call.
    ///
    /// An inactive or empty chain copies input to output. Returns [`ProcessStatus::Error`] if a
    /// slot failed during this call (it is then bypassed; see [`take_failure`](Self::take_failure)).
    pub fn process(
        &mut self,
        transport: Transport,
        input: &[f32],
        output: &mut [f32],
    ) -> ProcessStatus {
        debug_assert_eq!(
            input.len(),
            output.len(),
            "rack input/output length mismatch"
        );
        let frames = input.len().min(output.len());
        let (input, output) = (&input[..frames], &mut output[..frames]);
        let Some(cfg) = self.config else {
            output.copy_from_slice(input);
            return ProcessStatus::Continue;
        };
        if self.slots.is_empty() {
            output.copy_from_slice(input);
            return ProcessStatus::Continue;
        }
        let max = cfg.max_block as usize;
        let mut status = ProcessStatus::Continue;
        let mut start = 0;
        loop {
            let len = (frames - start).min(max);
            let t = Transport {
                playing: transport.playing,
                position_samples: transport.position_samples.map(|p| p + start as u64),
            };
            if self.run_sub_block(start, len, frames, t, input, output) {
                status = ProcessStatus::Error;
            }
            start += len;
            if start >= frames {
                break;
            }
        }
        for slot in &mut self.slots {
            slot.finish_call();
        }
        status
    }

    /// One sub-block through every slot, ping-ponging between the two scratch buffers; the first
    /// slot reads `input`, the last writes `output`. Returns true if a slot failed.
    fn run_sub_block(
        &mut self,
        start: usize,
        len: usize,
        host_frames: usize,
        t: Transport,
        input: &[f32],
        output: &mut [f32],
    ) -> bool {
        let Self {
            slots,
            ping,
            pong,
            outbox,
            failure,
            ..
        } = self;
        let r = start..start + len;
        let count = slots.len();
        let mut failed = false;
        for (i, slot) in slots.iter_mut().enumerate() {
            let last = i + 1 == count;
            let even = i.is_multiple_of(2);
            // Slot i writes ping if i is even, pong if odd (unless last), and reads what slot
            // i - 1 wrote.
            let (src, dst): (&[f32], &mut [f32]) = match (i == 0, last, even) {
                (true, true, _) => (&input[r.clone()], &mut output[r.clone()]),
                (true, false, _) => (&input[r.clone()], &mut ping[..len]),
                (false, true, true) => (&pong[..len], &mut output[r.clone()]),
                (false, true, false) => (&ping[..len], &mut output[r.clone()]),
                (false, false, true) => (&pong[..len], &mut ping[..len]),
                (false, false, false) => (&ping[..len], &mut pong[..len]),
            };
            if slot.run(start, len, host_frames, t, src, dst, outbox) {
                failed = true;
                if failure.is_none() {
                    *failure = slot.failed.map(|reason| (i, reason));
                }
            }
        }
        failed
    }
}
