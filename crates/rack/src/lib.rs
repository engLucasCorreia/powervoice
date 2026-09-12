//! Pure chain host (ADR-001 §4): slots, parameter routing, host bypass, latency compensation,
//! offline render — shared by engine, export, bake and the CLI.
//!
//! T-005 skeleton: [`Rack`] runs an ordered chain of [`Module`]s with per-slot, sample-accurate
//! event routing across sub-block boundaries, and reports total latency. Not yet here: host
//! bypass/crossfade and the dual-mono shim (T-103), latency compensation and Restart handling
//! (T-401), the module registry and placeholder slots.
//!
//! **Chain edits (ADR-002 §3, ADR-005 §7/§12).** A `Rack` that runs on the audio thread is
//! *live*; only [`process`](Rack::process), [`reset`](Rack::reset) and
//! [`push_event`](Rack::push_event) may be called on it. Insert / remove / reorder never call
//! control-thread methods on a live rack: the control thread builds and activates a **new** chain,
//! hands it to the audio thread, which installs it with [`swap_chain`] at a block boundary, and
//! the retired chain comes back through the return ring to be deactivated and dropped
//! off-thread. The new chain's inserted or replaced slots hold fresh instances (activated off
//! the audio thread from the committed states). For **unchanged** slots, T-103 may either
//! cold-restart them the same way, or move their `Box<dyn Module>` instances from the retiring
//! chain into the new one at the swap (pointer moves on the audio thread: no allocation, no
//! `activate`, state stays warm). This skeleton's [`swap_chain`] only swaps whole chains.

use vox_module_api::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, EventList, EventListError, HostRequest,
    Module, ModuleError, OutputEvents, ParamEvent, ProcessContext, ProcessStatus, SchemaError,
    Tail, Transport, validate_schema,
};

/// Capacities of the per-slot event storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RackOptions {
    /// Events one `process()` call of a module receives at most (ADR-002: 512).
    pub event_capacity: usize,
    /// Events queued per slot for upcoming blocks (including carried-over ones).
    pub queue_capacity: usize,
}

impl Default for RackOptions {
    fn default() -> Self {
        Self {
            event_capacity: DEFAULT_EVENT_CAPACITY,
            queue_capacity: 2 * DEFAULT_EVENT_CAPACITY,
        }
    }
}

/// Error from a control-thread rack operation.
#[derive(Debug, thiserror::Error)]
pub enum RackError {
    /// Slot index out of range.
    #[error("slot index {index} out of range (rack has {len} slots)")]
    IndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Number of slots.
        len: usize,
    },
    /// The module cannot run 1-in/1-out.
    #[error(
        "module `{id}`: unsupported channel layout (the v1 rack needs MONO; the dual-mono shim is T-103)"
    )]
    UnsupportedLayout {
        /// Module id.
        id: String,
    },
    /// The module's parameter schema is invalid.
    #[error("module `{id}`: invalid parameter schema: {source}")]
    Schema {
        /// Module id.
        id: String,
        /// Violated invariant.
        source: SchemaError,
    },
    /// The activation config is invalid.
    #[error("invalid rack configuration: {0}")]
    InvalidConfig(&'static str),
    /// `activate` on an active rack.
    #[error("rack is already active")]
    AlreadyActive,
    /// A module failed to activate.
    #[error("slot {index} (`{id}`) failed to activate: {source}")]
    Activate {
        /// Slot index.
        index: usize,
        /// Module id.
        id: String,
        /// Module error.
        source: ModuleError,
    },
}

/// Error from [`Rack::push_event`] (RT-safe, `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PushEventError {
    /// No slot at that index.
    #[error("no slot at index {0}")]
    NoSuchSlot(usize),
    /// Queue full or event out of order.
    #[error(transparent)]
    List(#[from] EventListError),
}

struct Slot {
    module: Box<dyn Module>,
    /// Events for the current/next `process()` call, offsets relative to its start. Carried-over
    /// events sit at the front with offset 0.
    queue: Vec<ParamEvent>,
    queue_capacity: usize,
    /// Index of the first undelivered event in `queue` during a `process()` call.
    cursor: usize,
    block_events: EventList,
    out_events: OutputEvents,
    latency_samples: u32,
    tail: Tail,
    steady_time: u64,
    restart_requested: bool,
}

impl Slot {
    fn new(module: Box<dyn Module>, options: &RackOptions) -> Self {
        Self {
            module,
            queue: Vec::with_capacity(options.queue_capacity),
            queue_capacity: options.queue_capacity,
            cursor: 0,
            block_events: EventList::with_capacity(options.event_capacity),
            out_events: OutputEvents::with_capacity(options.event_capacity),
            latency_samples: 0,
            tail: Tail::Samples(0),
            steady_time: 0,
            restart_requested: false,
        }
    }

    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        self.module.activate(config)?;
        self.latency_samples = self.module.latency_samples();
        self.tail = self.module.tail();
        self.steady_time = 0;
        self.restart_requested = false;
        Ok(())
    }

    fn deactivate(&mut self) {
        self.module.deactivate();
        self.latency_samples = 0;
        self.tail = Tail::Samples(0);
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

    fn run(
        &mut self,
        start: usize,
        len: usize,
        host_frames: usize,
        transport: Transport,
        input: &[f32],
        output: &mut [f32],
    ) -> ProcessStatus {
        self.collect_events(start, len, host_frames);
        // TODO(T-103): module output events (READ_ONLY reports, adapter-originated parameter
        // changes) are cleared here and therefore dropped. They need an RT drain to the control
        // thread's parameter mirror (RT event ring).
        self.out_events.clear();
        let mut ctx = ProcessContext::new(
            len as u32,
            self.steady_time,
            transport,
            self.block_events.as_slice(),
            &mut self.out_events,
        );
        let status = self.module.process(&mut ctx, &[input], &mut [output]);
        if ctx.requested(HostRequest::Restart) {
            self.restart_requested = true;
        }
        self.steady_time += len as u64;
        status
    }

    /// End of a `process()` call: undelivered events move to the front at offset 0.
    fn finish_call(&mut self) {
        let rest = self.queue.len() - self.cursor;
        self.queue.copy_within(self.cursor.., 0);
        self.queue.truncate(rest);
        for e in &mut self.queue {
            e.offset = 0;
        }
        self.cursor = 0;
    }
}

/// An ordered chain of modules (one "chain" in ADR-002's `SwapChain(Box<Chain>)`).
///
/// Lifecycle: build it on the control thread ([`insert`](Self::insert) /
/// [`remove`](Self::remove) / [`move_slot`](Self::move_slot)), [`activate`](Self::activate) it,
/// then hand it to the audio thread (or an offline render), which only calls the RT-safe
/// [`process`](Self::process), [`reset`](Self::reset) and [`push_event`](Self::push_event).
/// Deactivate it after it comes back.
pub struct Rack {
    slots: Vec<Slot>,
    options: RackOptions,
    config: Option<ActivateConfig>,
    ping: Vec<f32>,
    pong: Vec<f32>,
}

impl Default for Rack {
    fn default() -> Self {
        Self::new()
    }
}

impl Rack {
    /// Empty, inactive rack with default capacities.
    pub fn new() -> Self {
        Self::with_options(RackOptions::default())
    }

    /// Empty, inactive rack.
    pub fn with_options(options: RackOptions) -> Self {
        Self {
            slots: Vec::new(),
            options,
            config: None,
            ping: Vec::new(),
            pong: Vec::new(),
        }
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

    /// The module in slot `index` (control thread, rack not live).
    pub fn module(&self, index: usize) -> Option<&dyn Module> {
        self.slots.get(index).map(|s| &*s.module)
    }

    /// \[control thread\] Inserts `module` (inactive) at `index` (`0..=len`). The module must
    /// support MONO and have a valid schema. If the rack is active, the module is activated with
    /// the rack's configuration.
    pub fn insert(&mut self, index: usize, module: Box<dyn Module>) -> Result<(), RackError> {
        if index > self.slots.len() {
            return Err(RackError::IndexOutOfRange {
                index,
                len: self.slots.len(),
            });
        }
        let id = || module.descriptor().id.clone();
        if !module.supported_layouts().contains(&ChannelLayout::MONO) {
            return Err(RackError::UnsupportedLayout { id: id() });
        }
        validate_schema(module.params(), module.groups())
            .map_err(|source| RackError::Schema { id: id(), source })?;
        let mut slot = Slot::new(module, &self.options);
        if let Some(cfg) = self.config {
            slot.activate(&cfg).map_err(|source| RackError::Activate {
                index,
                id: slot.module.descriptor().id.clone(),
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

    /// \[control thread\] Removes and returns the module at `index`, deactivated if the rack is
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
        Ok(slot.module)
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

    /// \[control thread\] Activates every module and allocates the ping-pong buffers. The v1 rack
    /// runs MONO. On error the modules activated so far are deactivated again.
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
        for index in 0..self.slots.len() {
            if let Err(source) = self.slots[index].activate(config) {
                for slot in &mut self.slots[..index] {
                    slot.deactivate();
                }
                return Err(RackError::Activate {
                    index,
                    id: self.slots[index].module.descriptor().id.clone(),
                    source,
                });
            }
        }
        self.ping = vec![0.0; config.max_block as usize];
        self.pong = vec![0.0; config.max_block as usize];
        self.config = Some(*config);
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

    /// Total latency = sum of slot latencies (reporting only; compensation is T-401).
    /// 0 while inactive.
    pub fn latency_samples(&self) -> u32 {
        self.slots
            .iter()
            .fold(0u32, |acc, s| acc.saturating_add(s.latency_samples))
    }

    /// Latency of slot `index` (read at activation).
    pub fn slot_latency_samples(&self, index: usize) -> Option<u32> {
        self.slots.get(index).map(|s| s.latency_samples)
    }

    /// Tail of slot `index` (read at activation).
    pub fn slot_tail(&self, index: usize) -> Option<Tail> {
        self.slots.get(index).map(|s| s.tail)
    }

    /// True once the module in slot `index` requested [`HostRequest::Restart`] (handled by
    /// T-401).
    pub fn restart_requested(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|s| s.restart_requested)
    }

    /// RT-safe. Queues a parameter event for slot `index`, for the **next** `process()` call:
    /// `offset` is relative to that call's first sample; values must already be
    /// `clamp_quantize`d (the control thread's mirror does it). Events must be pushed in
    /// non-decreasing offset order. An offset at or past the next call's length is delivered at
    /// offset 0 of the call after it. `Err(Full)`: the caller keeps the event and retries next
    /// block (values are delayed, never lost).
    pub fn push_event(&mut self, index: usize, event: ParamEvent) -> Result<(), PushEventError> {
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(PushEventError::NoSuchSlot(index))?;
        if slot.queue.len() >= slot.queue_capacity {
            return Err(EventListError::Full.into());
        }
        if slot.queue.last().is_some_and(|l| event.offset < l.offset) {
            return Err(EventListError::OutOfOrder.into());
        }
        slot.queue.push(event);
        Ok(())
    }

    /// RT-safe. Resets every module (seek, loop wrap, transport start). Queued events stay.
    pub fn reset(&mut self) {
        if self.config.is_some() {
            for slot in &mut self.slots {
                slot.module.reset();
            }
        }
    }

    /// RT-safe. Runs `input` through the chain into `output` (equal lengths; any length —
    /// blocks longer than `max_block` are split into sub-blocks, and each slot's queued events
    /// are split at the sub-block boundaries, keeping their exact sample positions). A
    /// zero-length call is a parameter flush. If an event list fills up, the remaining events are
    /// carried to offset 0 of the next sub-block or call.
    ///
    /// An inactive or empty rack copies input to output. Returns
    /// [`ProcessStatus::Error`] if any slot did (failure handling is T-103).
    ///
    /// Module output events (`ProcessContext::out_events`) are currently discarded;
    /// TODO(T-103): drain them to the control thread.
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
            if self.run_sub_block(start, len, frames, t, input, output) == ProcessStatus::Error {
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
    /// slot reads `input`, the last writes `output`.
    fn run_sub_block(
        &mut self,
        start: usize,
        len: usize,
        host_frames: usize,
        t: Transport,
        input: &[f32],
        output: &mut [f32],
    ) -> ProcessStatus {
        let Self {
            slots, ping, pong, ..
        } = self;
        let r = start..start + len;
        let count = slots.len();
        let mut status = ProcessStatus::Continue;
        for (i, slot) in slots.iter_mut().enumerate() {
            let last = i + 1 == count;
            let even = i.is_multiple_of(2);
            // Slot i writes ping if i is even, pong if odd (unless last), and reads what slot
            // i - 1 wrote.
            let s = match (i == 0, last, even) {
                (true, true, _) => slot.run(
                    start,
                    len,
                    host_frames,
                    t,
                    &input[r.clone()],
                    &mut output[r.clone()],
                ),
                (true, false, _) => slot.run(
                    start,
                    len,
                    host_frames,
                    t,
                    &input[r.clone()],
                    &mut ping[..len],
                ),
                (false, true, true) => slot.run(
                    start,
                    len,
                    host_frames,
                    t,
                    &pong[..len],
                    &mut output[r.clone()],
                ),
                (false, true, false) => slot.run(
                    start,
                    len,
                    host_frames,
                    t,
                    &ping[..len],
                    &mut output[r.clone()],
                ),
                (false, false, true) => {
                    slot.run(start, len, host_frames, t, &pong[..len], &mut ping[..len])
                }
                (false, false, false) => {
                    slot.run(start, len, host_frames, t, &ping[..len], &mut pong[..len])
                }
            };
            if s == ProcessStatus::Error {
                status = ProcessStatus::Error;
            }
        }
        status
    }
}

/// \[audio thread, block boundary\] Installs `next` as the live chain and returns the retired
/// one. RT-safe: moves two pointers, allocates and frees nothing.
///
/// **Placeholder for T-103.** The real rack-edit swap receives `next` through the command ring
/// (`SwapChain(Box<Chain>)`), crossfades old → new over 15 ms, and pushes the retired chain onto
/// the return ring; the control thread deactivates and drops it. `next` must already be active
/// with the same sample rate and `max_block` as the live chain (debug builds assert it when the
/// live chain is active).
#[must_use = "the retired chain must go back to the control thread (return ring); never drop it on the audio thread"]
pub fn swap_chain(live: &mut Box<Rack>, next: Box<Rack>) -> Box<Rack> {
    debug_assert!(
        next.is_active(),
        "swap_chain: the next chain must be active"
    );
    debug_assert!(
        match (live.config, next.config) {
            (Some(a), Some(b)) =>
                a.sample_rate.to_bits() == b.sample_rate.to_bits() && a.max_block == b.max_block,
            _ => true,
        },
        "swap_chain: the next chain's sample_rate/max_block differ from the live chain"
    );
    std::mem::replace(live, next)
}
