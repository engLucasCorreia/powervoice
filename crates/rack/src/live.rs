//! The audio-thread side of the live rack: [`LiveRack`] owns the installed [`Chain`], drains
//! [`RackCommand`]s, performs chain swaps (moving untouched instances), whole-rack A/B, and
//! sends RT events and retired chains back to the control thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer, PushError, RingBuffer};
use vox_module_api::{EventListError, ParamEvent, ParamId, Transport};

use crate::chain::PlanEntry;
use crate::delay::DelayLine;
use crate::mix::{AUX, DRY_AUX, DRY_LAT, DRY0, Mixer, WET};
use crate::slot::LIFECYCLE_RESERVE;
use crate::{
    COMMAND_RING_CAPACITY, Chain, EVENT_RING_CAPACITY, MAX_COMMANDS_PER_CALL, PushEventError,
    RETURN_RING_CAPACITY, RackEvent, SlotUid,
};

/// Control → audio commands (ADR-002 §3). Boxed chains leave the audio thread only through the
/// return ring.
pub enum RackCommand {
    /// Install a new chain at the next block boundary (built and activated off-thread).
    Swap(Box<Chain>),
    /// A parameter change (already `clamp_quantize`d), delivered at offset 0 of the next block.
    Param {
        /// Slot.
        slot: SlotUid,
        /// Parameter.
        id: ParamId,
        /// Plain value.
        value: f64,
    },
    /// Host bypass toggle (15 ms crossfade).
    SetBypass {
        /// Slot.
        slot: SlotUid,
        /// New flag.
        bypassed: bool,
    },
    /// Fade the slot out (removal) after `hold` samples; it reports
    /// [`RackEvent::SlotRemoved`] when done.
    Remove {
        /// Slot.
        slot: SlotUid,
        /// Samples to keep the slot sounding first (a reorder lines the fade-out up with the
        /// moved module's fade-in, which waits for its latency).
        hold: u32,
    },
    /// Whole-rack A/B (listening only).
    SetAb(bool),
    /// Seek / loop wrap / transport start.
    Reset,
}

impl std::fmt::Debug for RackCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Swap(c) => write!(f, "Swap({} slots)", c.len()),
            Self::Param { slot, id, value } => write!(f, "Param({slot:?}, {id:?}, {value})"),
            Self::SetBypass { slot, bypassed } => write!(f, "SetBypass({slot:?}, {bypassed})"),
            Self::Remove { slot, hold } => write!(f, "Remove({slot:?}, hold {hold})"),
            Self::SetAb(on) => write!(f, "SetAb({on})"),
            Self::Reset => write!(f, "Reset"),
        }
    }
}

/// The control-thread ends of the live rack's rings.
pub(crate) struct RackLink {
    pub(crate) commands: Producer<RackCommand>,
    pub(crate) events: Consumer<RackEvent>,
    pub(crate) retired: Consumer<Box<Chain>>,
    pub(crate) dropped: Arc<AtomicU64>,
}

/// The live rack on the audio thread (output callback, T-105). Every method is RT-safe unless
/// marked otherwise: `process` drains up to [`MAX_COMMANDS_PER_CALL`] commands at its start, so
/// commands take effect at call boundaries.
///
/// Teardown: hand it back to the control thread and call
/// [`RackHost::teardown`](crate::RackHost::teardown) (or [`into_chains`](Self::into_chains)).
pub struct LiveRack {
    chain: Box<Chain>,
    commands: Consumer<RackCommand>,
    events: Producer<RackEvent>,
    retired: Producer<Box<Chain>>,
    dropped: Arc<AtomicU64>,
    /// A retired chain the full return ring could not take yet.
    pending_retired: Option<Box<Chain>>,
    /// A command that could not be applied yet (full event queue, pending retire, A/B tap fade).
    stalled: Option<RackCommand>,
    /// Lifecycle events the full event ring could not take yet (fixed capacity).
    pending_events: Vec<RackEvent>,
    ab_on: bool,
    /// A/B mixer: WET = chain output, DRY_LAT = input delayed by the total latency, DRY_AUX = the
    /// previous total latency (while a swap changed it).
    ab_mix: Mixer,
    ab_line: DelayLine,
    ab_tap: u32,
    ab_prev_tap: u32,
    fade_len: u32,
    max_block: usize,
}

impl LiveRack {
    /// \[control thread\] Wraps an **active** chain and creates the rings. `ab_capacity` is the
    /// largest total latency the A/B dry path serves without a new line.
    pub(crate) fn new(chain: Box<Chain>, ab_capacity: usize) -> (Self, RackLink) {
        let cfg = chain
            .config()
            .expect("LiveRack::new: the chain must be active");
        let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_RING_CAPACITY);
        let (ev_tx, ev_rx) = RingBuffer::new(EVENT_RING_CAPACITY);
        let (ret_tx, ret_rx) = RingBuffer::new(RETURN_RING_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let max_block = cfg.max_block as usize;
        let tap = chain.latency_samples();
        let live = Self {
            fade_len: chain.fade_len(),
            chain,
            commands: cmd_rx,
            events: ev_tx,
            retired: ret_tx,
            dropped: dropped.clone(),
            pending_retired: None,
            stalled: None,
            pending_events: Vec::with_capacity(LIFECYCLE_RESERVE),
            ab_on: false,
            ab_mix: Mixer::steady(WET),
            ab_line: DelayLine::new(ab_capacity.max(tap as usize), max_block),
            ab_tap: tap,
            ab_prev_tap: tap,
            max_block,
        };
        let link = RackLink {
            commands: cmd_tx,
            events: ev_rx,
            retired: ret_rx,
            dropped,
        };
        (live, link)
    }

    /// The installed chain (inspection; control-thread methods on it are only valid while the
    /// audio thread is not processing, e.g. in tests).
    pub fn chain(&self) -> &Chain {
        &self.chain
    }

    /// Total latency of the installed chain (for the heard-position formula, ADR-002 §8).
    pub fn latency_samples(&self) -> u32 {
        self.chain.latency_samples()
    }

    /// True while whole-rack A/B is on.
    pub fn ab_enabled(&self) -> bool {
        self.ab_on
    }

    /// Seek / loop wrap / transport start: resets every module and dry path.
    pub fn reset(&mut self) {
        self.chain.reset();
        self.ab_line.clear();
    }

    /// \[control thread, after the audio thread let go\] Every chain this rack holds: the
    /// installed one first, then any not yet retired or still queued in the command ring. The
    /// caller deactivates and drops them.
    pub fn into_chains(self) -> Vec<Box<Chain>> {
        let Self {
            chain,
            mut commands,
            stalled,
            pending_retired,
            ..
        } = self;
        let mut out = vec![chain];
        out.extend(pending_retired);
        if let Some(RackCommand::Swap(c)) = stalled {
            out.push(c);
        }
        while let Ok(cmd) = commands.pop() {
            if let RackCommand::Swap(c) = cmd {
                out.push(c);
            }
        }
        out
    }

    /// Queues a parameter event with a real offset (relative to the next `process` call) for
    /// slot `slot` — for drivers with sample-accurate automation; UI changes use
    /// [`RackCommand::Param`].
    pub fn push_event(&mut self, slot: SlotUid, event: ParamEvent) -> Result<(), PushEventError> {
        self.chain.push_event_to(slot, event)
    }

    /// Applies one command now. `Err(cmd)`: it cannot be applied yet (the slot's event queue is
    /// full, a retired chain is still waiting for the return ring, or a swap would change the
    /// A/B dry tap while an A/B crossfade runs) — retry later, in order.
    pub fn handle(&mut self, cmd: RackCommand) -> Result<(), RackCommand> {
        match cmd {
            RackCommand::Swap(next) => {
                let tap = next.expected_latency.unwrap_or(self.ab_tap);
                if self.pending_retired.is_some() || (tap != self.ab_tap && self.ab_mix.is_fading())
                {
                    return Err(RackCommand::Swap(next));
                }
                self.apply_swap(next);
            }
            RackCommand::Param { slot, id, value } => {
                let Some(i) = self.chain.find(slot) else {
                    return Ok(());
                };
                // UI changes land at offset 0 of the next block — or after any events already
                // queued with a real offset, keeping the queue sorted.
                let offset = self.chain.slots[i].queued_last_offset();
                let ev = ParamEvent { offset, id, value };
                if let Err(PushEventError::List(EventListError::Full)) =
                    self.chain.push_event(i, ev)
                {
                    return Err(RackCommand::Param { slot, id, value });
                }
            }
            RackCommand::SetBypass { slot, bypassed } => self.chain.set_bypass_uid(slot, bypassed),
            RackCommand::Remove { slot, hold } => self.chain.begin_remove_uid(slot, hold),
            RackCommand::SetAb(on) => {
                self.ab_on = on;
                self.ab_mix
                    .fade_to(if on { DRY_LAT } else { WET }, self.fade_len);
            }
            RackCommand::Reset => self.reset(),
        }
        Ok(())
    }

    fn pump(&mut self) {
        if let Some(r) = self.pending_retired.take()
            && let Err(PushError::Full(r)) = self.retired.push(r)
        {
            self.pending_retired = Some(r);
        }
        if let Some(cmd) = self.stalled.take()
            && let Err(cmd) = self.handle(cmd)
        {
            self.stalled = Some(cmd);
            return;
        }
        for _ in 0..MAX_COMMANDS_PER_CALL {
            let Ok(cmd) = self.commands.pop() else {
                break;
            };
            if let Err(cmd) = self.handle(cmd) {
                self.stalled = Some(cmd);
                break;
            }
        }
    }

    /// Installs `next`: moves kept slots in (pointer swaps), hands replaced instances over,
    /// adopts a bigger A/B line, and sends the retired chain to the return ring.
    fn apply_swap(&mut self, mut next: Box<Chain>) {
        {
            let old = &mut *self.chain;
            for j in 0..next.plan.len() {
                match next.plan[j] {
                    PlanEntry::Fresh => {}
                    PlanEntry::Keep { uid, drop_aux } => {
                        if let Some(i) = old.find(uid) {
                            std::mem::swap(&mut next.slots[j], &mut old.slots[i]);
                            if drop_aux {
                                std::mem::swap(
                                    &mut next.slots[j].outgoing,
                                    &mut old.slots[i].outgoing,
                                );
                                next.slots[j].clear_aux();
                            }
                        }
                    }
                    PlanEntry::Replace { uid } => {
                        if let Some(i) = old.find(uid) {
                            next.slots[j].take_over(&mut old.slots[i]);
                        }
                    }
                }
            }
        }
        let mut spare = next.spare_ab.take();
        if let Some(line) = spare.as_mut() {
            line.copy_history_from(&self.ab_line);
            std::mem::swap(line, &mut self.ab_line);
        }
        let tap = next.latency_samples().min(self.ab_line.max_tap() as u32);
        if tap != self.ab_tap {
            if self.ab_on || self.ab_mix.is_fading() {
                let from = self.ab_mix.remapped([WET, AUX, DRY0, DRY_AUX, DRY_AUX]);
                let target = if self.ab_on { DRY_LAT } else { WET };
                self.ab_mix = Mixer::fading(from, target, self.fade_len, 0);
            }
            self.ab_prev_tap = self.ab_tap;
            self.ab_tap = tap;
        }
        let mut retired = std::mem::replace(&mut self.chain, next);
        retired.spare_ab = spare;
        if let Err(PushError::Full(r)) = self.retired.push(retired) {
            self.pending_retired = Some(r);
        }
    }

    #[inline]
    fn ab_dry(&self, input: &[f32], base: usize, i: usize, tap: u32) -> f32 {
        if tap == 0 {
            input[i]
        } else {
            self.ab_line.get(base, i, tap as usize)
        }
    }

    /// Runs `input` through the rack into `output` (equal lengths, any length; see
    /// [`Chain::process`]). Drains commands first and RT events last.
    pub fn process(&mut self, transport: Transport, input: &[f32], output: &mut [f32]) {
        self.pump();
        let frames = input.len().min(output.len());
        let (input, output) = (&input[..frames], &mut output[..frames]);
        self.chain.process(transport, input, output);
        let mut start = 0;
        while start < frames {
            let n = (frames - start).min(self.max_block);
            let (inp, out) = (&input[start..start + n], &mut output[start..start + n]);
            let base = self.ab_line.write(inp);
            match self.ab_mix.steady_source() {
                Some(WET) => {}
                Some(_) => {
                    if self.ab_tap == 0 {
                        out.copy_from_slice(inp);
                    } else {
                        self.ab_line.read_into(base, self.ab_tap as usize, out);
                    }
                }
                None => {
                    for (i, o) in out.iter_mut().enumerate() {
                        let (g, _) = self.ab_mix.gains_at(i);
                        *o = g[WET] * *o
                            + g[DRY_LAT] * self.ab_dry(inp, base, i, self.ab_tap)
                            + g[DRY_AUX] * self.ab_dry(inp, base, i, self.ab_prev_tap);
                    }
                }
            }
            self.ab_mix.advance(n);
            start += n;
        }
        self.send_events();
    }

    /// Moves the chain's RT events to the event ring. Module reports only use the ring while
    /// [`LIFECYCLE_RESERVE`] slots stay free; lifecycle events (failures, removals, finished
    /// replacements, restart requests) that find it full wait in a fixed buffer and go first
    /// next time. Every dropped event is counted.
    fn send_events(&mut self) {
        let Self {
            chain,
            events,
            dropped,
            pending_events,
            ..
        } = self;
        let mut sent = 0;
        for &e in pending_events.iter() {
            if events.push(e).is_err() {
                break;
            }
            sent += 1;
        }
        pending_events.drain(..sent);
        chain.drain_events(|e| match e {
            RackEvent::ParamReport { .. } => {
                if events.slots() <= LIFECYCLE_RESERVE || events.push(e).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {
                if !pending_events.is_empty() || events.push(e).is_err() {
                    if pending_events.len() < pending_events.capacity() {
                        pending_events.push(e);
                    } else {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        let d = chain.take_dropped_events();
        if d > 0 {
            dropped.fetch_add(d, Ordering::Relaxed);
        }
    }
}
