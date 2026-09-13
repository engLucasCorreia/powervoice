//! One rack slot: a module instance plus the host-owned machinery around it — the event queue
//! (coalescing, carry-over), the latency-matched dry path, the per-slot crossfade (bypass,
//! insert, remove, replacement), the non-finite guard and the output-event drain.

use vox_module_api::{
    ActivateConfig, EventList, EventListError, HostRequest, Module, ModuleError, OutputEvents,
    ParamEvent, ParamFlags, ParamId, ProcessContext, ProcessStatus, Tail, Transport,
};

use crate::delay::DelayLine;
use crate::mix::{AUX, DRY_AUX, DRY_LAT, DRY0, Mixer, SOURCES, WET};
use crate::{FailReason, RackEvent, RackOptions, SlotUid};

/// Entries of the outbox and of the RT event ring kept free for lifecycle events.
pub(crate) const LIFECYCLE_RESERVE: usize = 64;

/// Fixed-capacity buffer of RT events a chain collects during `process` (drained by the live
/// rack into the RT event ring, or by an offline render).
#[derive(Debug)]
pub(crate) struct Outbox {
    events: Vec<RackEvent>,
    capacity: usize,
    pub(crate) dropped: u64,
}

impl Outbox {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            capacity,
            dropped: 0,
        }
    }

    /// RT-safe. A module report: only while [`LIFECYCLE_RESERVE`] entries stay free for
    /// lifecycle events (failures, removals, replacements, restart requests); dropped and
    /// counted otherwise.
    pub(crate) fn post_report(&mut self, e: RackEvent) {
        if self.events.len() + LIFECYCLE_RESERVE < self.capacity {
            self.events.push(e);
        } else {
            self.dropped += 1;
        }
    }

    /// RT-safe; counts the event as dropped when full.
    pub(crate) fn post(&mut self, e: RackEvent) {
        if self.events.len() < self.capacity {
            self.events.push(e);
        } else {
            self.dropped += 1;
        }
    }

    /// RT-safe. Hands every event to `f` (in order) and empties the box.
    pub(crate) fn drain(&mut self, mut f: impl FnMut(RackEvent)) {
        for e in self.events.drain(..) {
            f(e);
        }
    }
}

/// How a slot starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotInit {
    /// The slot's bypass flag.
    pub(crate) bypassed: bool,
    /// Fade in from the undelayed input (a live insert) instead of starting steady.
    pub(crate) fade_in: bool,
}

pub(crate) struct Slot {
    pub(crate) uid: SlotUid,
    /// `None`: placeholder (missing module) or a moved-out shell.
    pub(crate) module: Option<Box<dyn Module>>,
    /// The replaced instance, crossfading out.
    pub(crate) outgoing: Option<Box<dyn Module>>,
    /// Display name (for error messages).
    pub(crate) name: String,
    queue: Vec<ParamEvent>,
    queue_capacity: usize,
    cursor: usize,
    block_events: EventList,
    out_events: OutputEvents,
    aux_out_events: OutputEvents,
    pub(crate) latency: u32,
    pub(crate) aux_latency: u32,
    pub(crate) tail: Tail,
    steady_time: u64,
    aux_steady_time: u64,
    dry: DelayLine,
    wet: Vec<f32>,
    aux: Vec<f32>,
    pub(crate) mixer: Mixer,
    fade_len: u32,
    /// Samples until a new instance's output and its fresh dry line are valid (its latency,
    /// after a live insert or a replacement; counted down in `run`). A bypass toggle or a
    /// failure inside this warm-up holds the current gains until then instead of fading to a
    /// source that is not valid yet (H-01).
    warm_left: u32,
    pub(crate) bypassed: bool,
    bypass_param: Option<ParamId>,
    pub(crate) failed: Option<FailReason>,
    pub(crate) removing: bool,
    pub(crate) dead: bool,
    pub(crate) restart_requested: bool,
    aux_done: bool,
}

impl Slot {
    /// An empty shell (no module, no buffers). Allocation-free, so the audio thread can leave it
    /// behind in a retired chain.
    pub(crate) fn shell(uid: SlotUid) -> Self {
        Self {
            uid,
            module: None,
            outgoing: None,
            name: String::new(),
            queue: Vec::new(),
            queue_capacity: 0,
            cursor: 0,
            block_events: EventList::with_capacity(0),
            out_events: OutputEvents::with_capacity(0),
            aux_out_events: OutputEvents::with_capacity(0),
            latency: 0,
            aux_latency: 0,
            tail: Tail::Samples(0),
            steady_time: 0,
            aux_steady_time: 0,
            dry: DelayLine::default(),
            wet: Vec::new(),
            aux: Vec::new(),
            mixer: Mixer::steady(DRY0),
            fade_len: 0,
            warm_left: 0,
            bypassed: false,
            bypass_param: None,
            failed: None,
            removing: false,
            dead: false,
            restart_requested: false,
            aux_done: false,
        }
    }

    /// \[control thread\] An inactive slot around `module` (`None` = placeholder).
    pub(crate) fn new(
        uid: SlotUid,
        module: Option<Box<dyn Module>>,
        name: String,
        options: &RackOptions,
    ) -> Self {
        let bypass_param = module.as_ref().and_then(|m| {
            m.params()
                .iter()
                .find(|p| p.flags.contains(ParamFlags::BYPASS))
                .map(|p| p.id)
        });
        Self {
            module,
            name,
            queue: Vec::with_capacity(options.queue_capacity),
            queue_capacity: options.queue_capacity,
            block_events: EventList::with_capacity(options.event_capacity),
            out_events: OutputEvents::with_capacity(options.event_capacity),
            aux_out_events: OutputEvents::with_capacity(options.event_capacity),
            bypass_param,
            ..Self::shell(uid)
        }
    }

    /// \[control thread\] Activates the module (if any), then [`attach`](Self::attach)es.
    pub(crate) fn activate(
        &mut self,
        config: &ActivateConfig,
        fade_len: u32,
        init: SlotInit,
    ) -> Result<(), ModuleError> {
        if let Some(m) = self.module.as_mut() {
            m.activate(config)?;
        }
        self.attach(config, fade_len, init, 0);
        Ok(())
    }

    /// \[control thread\] For an **already active** module: reads latency and tail, allocates
    /// the buffers (the dry line covers `extra_tap` too, for a replacement's outgoing instance)
    /// and sets the initial crossfade state.
    pub(crate) fn attach(
        &mut self,
        config: &ActivateConfig,
        fade_len: u32,
        init: SlotInit,
        extra_tap: u32,
    ) {
        let (latency, tail) = self
            .module
            .as_ref()
            .map_or((0, Tail::Samples(0)), |m| (m.latency_samples(), m.tail()));
        self.latency = latency;
        self.tail = tail;
        self.steady_time = 0;
        self.restart_requested = false;
        let max_block = config.max_block as usize;
        self.wet = vec![0.0; max_block];
        self.aux = vec![0.0; max_block];
        self.dry = DelayLine::new(latency.max(extra_tap) as usize, max_block);
        self.fade_len = fade_len;
        self.warm_left = if init.fade_in { latency } else { 0 };
        self.bypassed = init.bypassed;
        let target = if self.module.is_none() {
            DRY0
        } else if init.bypassed && self.bypass_param.is_none() {
            DRY_LAT
        } else {
            WET
        };
        if let (true, Some(id)) = (init.bypassed, self.bypass_param) {
            let _ = self.push_event(ParamEvent {
                offset: 0,
                id,
                value: 1.0,
            });
        }
        self.mixer = if init.fade_in && target != DRY0 {
            // The new instance's output (and its dry line) is valid after `latency` samples:
            // hold the undelayed input until then, then fade (SPEC-012 §2.2).
            let mut from = [0.0; SOURCES];
            from[DRY0] = 1.0;
            Mixer::fading(from, target, fade_len, latency)
        } else {
            Mixer::steady(target)
        };
    }

    /// \[control thread\] Deactivates the instances this slot holds.
    pub(crate) fn deactivate(&mut self) {
        if let Some(m) = self.module.as_mut() {
            m.deactivate();
        }
        if let Some(m) = self.outgoing.as_mut() {
            m.deactivate();
        }
        self.latency = 0;
        self.tail = Tail::Samples(0);
    }

    /// Latency this slot contributes to the chain total (0 once it is being removed).
    pub(crate) fn contributing_latency(&self) -> u32 {
        if self.removing || self.dead {
            0
        } else {
            self.latency
        }
    }

    /// RT-safe. Queues an event for the next `process()` call with same-`(offset, id)`
    /// coalescing in place (SPEC-012 §4.2): the value of the last queued event with the same
    /// offset and id is overwritten and its position kept; otherwise the event is appended.
    pub(crate) fn push_event(&mut self, event: ParamEvent) -> Result<(), EventListError> {
        if let Some(last) = self.queue.last()
            && event.offset < last.offset
        {
            return Err(EventListError::OutOfOrder);
        }
        for e in self.queue.iter_mut().rev() {
            if e.offset != event.offset {
                break;
            }
            if e.id == event.id {
                e.value = event.value;
                return Ok(());
            }
        }
        if self.queue.len() >= self.queue_capacity {
            return Err(EventListError::Full);
        }
        self.queue.push(event);
        Ok(())
    }

    /// Offset of the last queued event (0 if none): where a UI change (no sub-block timing)
    /// can be queued without breaking the order.
    pub(crate) fn queued_last_offset(&self) -> u32 {
        self.queue.last().map_or(0, |e| e.offset)
    }

    /// RT-safe. Host bypass toggle: crossfade to the latency-matched dry path (or back), or an
    /// event to the module's own BYPASS parameter. A failed or departing slot stays as it is.
    /// Inside a new instance's warm-up the current gains are held until its output and dry
    /// line are valid, then the crossfade runs.
    pub(crate) fn set_bypass(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
        if self.module.is_none() || self.removing || self.failed.is_some() {
            return;
        }
        if let Some(id) = self.bypass_param {
            let _ = self.push_event(ParamEvent {
                offset: 0,
                id,
                value: if bypassed { 1.0 } else { 0.0 },
            });
            return;
        }
        self.mixer.fade_to_after(
            if bypassed { DRY_LAT } else { WET },
            self.fade_len,
            self.warm_left,
        );
    }

    /// \[control thread, not live\] Sets the bypass state without a crossfade.
    pub(crate) fn set_bypass_now(&mut self, bypassed: bool) {
        self.set_bypass(bypassed);
        if self.bypass_param.is_none() && self.module.is_some() && self.failed.is_none() {
            self.mixer = Mixer::steady(self.mixer.target_source());
        }
    }

    /// RT-safe. Starts the removal fade to the undelayed input after `hold` samples (a reorder
    /// lines it up with the moved module's fade-in); the slot turns into a pass-through once it
    /// completes ([`RackEvent::SlotRemoved`]).
    pub(crate) fn begin_remove(&mut self, hold: u32) {
        self.removing = true;
        self.mixer.fade_to_after(DRY0, self.fade_len, hold);
    }

    fn fail(&mut self, reason: FailReason, outbox: &mut Outbox) -> bool {
        if self.failed.is_some() {
            return false;
        }
        self.failed = Some(reason);
        if !self.removing {
            self.mixer
                .fade_to_after(DRY_LAT, self.fade_len, self.warm_left);
        }
        outbox.post(RackEvent::SlotFailed {
            slot: self.uid,
            reason,
        });
        true
    }

    /// RT-safe (pointer moves, bounded copies). `self` is a freshly built slot with the
    /// replacement instance; `old` is the live slot it replaces. The old instance becomes this
    /// slot's outgoing instance and keeps sounding until the new one's output is valid (its
    /// latency), then crossfades out. Events still queued for the old slot move over; those the
    /// replacement's queue cannot take are counted as dropped. If there is nothing to
    /// crossfade out (the old slot had no instance: a slot that failed to start at load) or no
    /// fade at all, [`RackEvent::ReplaceDone`] is posted at once, so a held-back replacement
    /// can follow.
    pub(crate) fn take_over(&mut self, old: &mut Slot, outbox: &mut Outbox) {
        self.outgoing = old.module.take();
        self.aux_latency = old.latency;
        self.aux_steady_time = old.steady_time;
        self.dry.copy_history_from(&old.dry);
        for i in old.cursor..old.queue.len() {
            let e = old.queue[i];
            if self.push_event(e).is_err() {
                outbox.dropped += 1;
            }
        }
        old.queue.clear();
        old.cursor = 0;
        let from = old.mixer.remapped([AUX, AUX, DRY0, DRY_AUX, DRY_AUX]);
        let target = if self.bypassed && self.bypass_param.is_none() {
            DRY_LAT
        } else {
            WET
        };
        self.mixer = Mixer::fading(from, target, self.fade_len, self.latency);
        self.warm_left = self.latency;
        if self.outgoing.is_none() || !self.mixer.is_fading() {
            self.aux_done = true;
            outbox.post(RackEvent::ReplaceDone { slot: self.uid });
        }
    }

    /// RT-safe. Drops the outgoing instance's mixing state after a completed replacement (the
    /// instance itself is swapped back into the retired chain by the caller).
    pub(crate) fn clear_aux(&mut self) {
        self.aux_done = false;
        self.aux_latency = 0;
    }

    /// RT-safe. Seek / loop wrap / transport start.
    pub(crate) fn reset(&mut self) {
        if let Some(m) = self.module.as_mut() {
            m.reset();
        }
        if let Some(m) = self.outgoing.as_mut() {
            m.reset();
        }
        self.dry.clear();
    }

    /// Fills `block_events` for the sub-block `[start, start + len)` of a `host_frames` call.
    /// Carried events (offset before `start`) come first at offset 0; events that do not fit
    /// stay queued for the next sub-block (or the next call).
    fn collect_events(&mut self, start: usize, len: usize, host_frames: usize) {
        self.block_events.clear();
        while let Some(&e) = self.queue.get(self.cursor) {
            let off = e.offset as usize;
            let offset = if host_frames == 0 || off < start {
                0
            } else if off < start + len {
                (off - start) as u32
            } else {
                break;
            };
            if self.block_events.push(ParamEvent { offset, ..e }).is_err() {
                break;
            }
            self.cursor += 1;
        }
    }

    /// End of a `process()` call: undelivered events move to the front at offset 0 (in order;
    /// they are not merged with each other, so no value is lost — ADR-005 §4).
    pub(crate) fn finish_call(&mut self) {
        let rest = self.queue.len() - self.cursor;
        self.queue.copy_within(self.cursor.., 0);
        self.queue.truncate(rest);
        for e in &mut self.queue {
            e.offset = 0;
        }
        self.cursor = 0;
    }

    /// RT-safe. One sub-block `[start, start + len)` of a `host_frames` call. Returns true if
    /// the slot failed during it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        start: usize,
        len: usize,
        host_frames: usize,
        transport: Transport,
        input: &[f32],
        output: &mut [f32],
        outbox: &mut Outbox,
    ) -> bool {
        if self.dead {
            output.copy_from_slice(input);
            return false;
        }
        let base = self.dry.write(input);
        let mut newly_failed = false;
        if self.module.is_some() {
            self.collect_events(start, len, host_frames);
        }
        let mut outcome = None;
        if let Some(module) = self.module.as_mut() {
            self.out_events.clear();
            let mut ctx = ProcessContext::new(
                len as u32,
                self.steady_time,
                transport,
                self.block_events.as_slice(),
                &mut self.out_events,
            );
            let status = module.process(&mut ctx, &[input], &mut [&mut self.wet[..len]]);
            let restart = ctx.requested(HostRequest::Restart);
            self.steady_time += len as u64;
            if restart && !self.restart_requested {
                self.restart_requested = true;
                outbox.post(RackEvent::RestartRequested { slot: self.uid });
            }
            for e in self.out_events.as_slice() {
                outbox.post_report(RackEvent::ParamReport {
                    slot: self.uid,
                    id: e.id,
                    value: e.value,
                });
            }
            // Non-finite guard (SPEC-012 §2.9): one pass per slot per block.
            let mut non_finite = false;
            for x in &mut self.wet[..len] {
                if !x.is_finite() {
                    *x = 0.0;
                    non_finite = true;
                }
            }
            outcome = Some((status, non_finite));
        }
        if let Some((status, non_finite)) = outcome {
            if status == ProcessStatus::Error {
                newly_failed |= self.fail(FailReason::ModuleError, outbox);
            }
            if non_finite {
                newly_failed |= self.fail(FailReason::NonFinite, outbox);
            }
        }
        if !self.aux_done
            && let Some(outgoing) = self.outgoing.as_mut()
        {
            self.aux_out_events.clear();
            let mut ctx = ProcessContext::new(
                len as u32,
                self.aux_steady_time,
                transport,
                &[],
                &mut self.aux_out_events,
            );
            outgoing.process(&mut ctx, &[input], &mut [&mut self.aux[..len]]);
            self.aux_steady_time += len as u64;
            for x in &mut self.aux[..len] {
                if !x.is_finite() {
                    *x = 0.0;
                }
            }
        }
        self.mix(input, base, output);
        self.warm_left = self.warm_left.saturating_sub(len as u32);
        if self.mixer.advance(len) {
            let target = self.mixer.target_source();
            if self.removing && target == DRY0 {
                self.dead = true;
                outbox.post(RackEvent::SlotRemoved { slot: self.uid });
            }
            if self.outgoing.is_some() && !self.aux_done && target != AUX && target != DRY_AUX {
                self.aux_done = true;
                outbox.post(RackEvent::ReplaceDone { slot: self.uid });
            }
        }
        newly_failed
    }

    #[inline]
    fn tap(&self, input: &[f32], base: usize, i: usize, tap: u32) -> f32 {
        if tap == 0 {
            input[i]
        } else {
            self.dry.get(base, i, tap as usize)
        }
    }

    /// Writes the slot output: a single source when steady, the gain-weighted sum while fading.
    fn mix(&self, input: &[f32], base: usize, output: &mut [f32]) {
        let n = input.len();
        if let Some(src) = self.mixer.steady_source() {
            match src {
                WET => output.copy_from_slice(&self.wet[..n]),
                AUX => output.copy_from_slice(&self.aux[..n]),
                DRY_LAT if self.latency > 0 => {
                    self.dry.read_into(base, self.latency as usize, output);
                }
                DRY_AUX if self.aux_latency > 0 => {
                    self.dry.read_into(base, self.aux_latency as usize, output);
                }
                _ => output.copy_from_slice(input),
            }
            return;
        }
        let active = self.mixer.active_sources();
        for (i, o) in output.iter_mut().enumerate() {
            let (g, _) = self.mixer.gains_at(i);
            let mut acc = 0.0f32;
            if active[WET] {
                acc += g[WET] * self.wet[i];
            }
            if active[AUX] {
                acc += g[AUX] * self.aux[i];
            }
            if active[DRY0] {
                acc += g[DRY0] * input[i];
            }
            if active[DRY_LAT] {
                acc += g[DRY_LAT] * self.tap(input, base, i, self.latency);
            }
            if active[DRY_AUX] {
                acc += g[DRY_AUX] * self.tap(input, base, i, self.aux_latency);
            }
            *o = acc;
        }
    }
}
