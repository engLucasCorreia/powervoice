//! One JSFX script as a sandbox [`PluginInstance`] (ADR-008 Amendment 11 §2).
//!
//! - **Load:** ysfx loads the script and its imports (from the script's folder, then its effects
//!   root) and compiles it without `@gfx`; `@init` runs at once (48 kHz), as in REAPER, so the
//!   values and `@serialize` state are the script's from the start. Scripts without audio input
//!   or output pins aren't effects and are refused.
//! - **Activation:** the rate and block size, `@init`, then `@slider` and a zero-length `@block`
//!   (a zero-frame process), so a `pdc_delay` set there is the reported latency. There is no
//!   per-rate instance: a new rate is just another `@init`.
//! - **Sample-accurate sliders:** a chunk is processed in segments split at its parameter
//!   events' offsets; each segment's slider changes run `@slider` before its first frame.
//! - **Pins and the mono shim:** the mono input feeds the first two input pins (the main pair;
//!   pins beyond are side-chain/aux by JSFX convention and get silence); the output is the mean
//!   of the first two output pins — CLAP's shim (ADR-008 Amendment 3 §2).
//! - **Latency:** `pdc_delay`; a change while active asks for a restart once
//!   (`EventKind::RESTART_REQUEST`), like CLAP, VST3 and LV2. No tail report: the tail is 0.
//! - **Sliders the script changes itself** (in `@slider`, `@block`, `@sample` or `@serialize`)
//!   are reported as parameter changes.
//! - **State:** every slider value plus the `@serialize` bytes ([`Blob`]).
//!
//! The effect lives behind one mutex: the audio thread holds it for a chunk, the main thread
//! while inactive — and, for a script with `@serialize`, while saving its state (ysfx can't run
//! `@serialize` alongside `@sample`), which the audio thread then waits for. Scripts that
//! allocate EEL2 memory on first use do so on the sandbox's audio thread, as in REAPER.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use vox_module_api::{ActivateConfig, ParamFlags, ParamId, ParamInfo};
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::{Chunk, EventKind, WireEvent};
use vox_ysfx_sys::{YSFX_MAX_SLIDERS, YSFX_SECTION_SERIALIZE};

use super::fx::Fx;
use super::params;
use super::state::Blob;
use crate::backend::{ActiveInfo, PluginInstance};

/// The rate of the `@init` run at load.
const LOAD_RATE: f64 = 48_000.0;
/// The block size announced at load.
const LOAD_MAX_BLOCK: u32 = 4096;
/// Input pins that get the mono signal / output pins averaged into it (the main pair).
pub(crate) const MAIN_PINS: usize = 2;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn latency_samples(v: f64) -> u32 {
    if v.is_finite() && v > 0.0 {
        v.round().min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// The audio thread's working set, built at activation.
struct AudioState {
    max_block: usize,
    /// One buffer per output pin.
    outs: Vec<Vec<f32>>,
    /// Silence for the input pins beyond the main pair.
    zero: Vec<f32>,
    in_ptrs: Vec<*const f32>,
    out_ptrs: Vec<*mut f32>,
    latency: u32,
    restart_sent: bool,
}

// SAFETY: the pointers are scratch space, re-aimed before every use at buffers this state owns
// or at the chunk being processed; the state moves to the audio thread behind the mutex.
unsafe impl Send for AudioState {}

impl AudioState {
    fn new(num_in: usize, num_out: usize, max_block: usize) -> Self {
        let mut outs: Vec<Vec<f32>> = (0..num_out).map(|_| vec![0.0; max_block]).collect();
        let zero = vec![0.0; max_block];
        let in_ptrs = vec![zero.as_ptr(); num_in];
        let out_ptrs = outs.iter_mut().map(|b| b.as_mut_ptr()).collect();
        Self {
            max_block,
            outs,
            zero,
            in_ptrs,
            out_ptrs,
            latency: 0,
            restart_sent: false,
        }
    }

    /// Aims the main input pins at `input[offset..]`, the others at silence, and the output
    /// pins at their buffers' `offset`. No allocation.
    fn aim(&mut self, input: &[f32], offset: usize) {
        let main = input.get(offset..).unwrap_or_default().as_ptr();
        for (ch, p) in self.in_ptrs.iter_mut().enumerate() {
            *p = if ch < MAIN_PINS {
                main
            } else {
                self.zero.as_ptr()
            };
        }
        for (p, b) in self.out_ptrs.iter_mut().zip(&mut self.outs) {
            *p = b.get_mut(offset..).unwrap_or_default().as_mut_ptr();
        }
    }
}

struct Core {
    fx: Fx,
    audio: Option<AudioState>,
}

/// A loaded JSFX script (ADR-008 Amendment 11).
pub struct JsfxInstance {
    name: String,
    vendor: String,
    version: String,
    params: Vec<ParamInfo>,
    num_in: usize,
    num_out: usize,
    has_serialize: bool,
    /// Per slider index: the last known value (`f64` bits), readable at any time.
    values: Box<[AtomicU64]>,
    core: Mutex<Core>,
}

impl JsfxInstance {
    /// Loads the script at `path` (main thread).
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut fx = Fx::load(path)?;
        let name = match fx.name() {
            n if n.trim().is_empty() => path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            n => n,
        };
        let (num_in, num_out) = (fx.num_inputs() as usize, fx.num_outputs() as usize);
        if num_in == 0 || num_out == 0 {
            return Err(format!(
                "{name} has no audio {} (not an audio effect)",
                if num_in == 0 { "input" } else { "output" }
            ));
        }
        let params = params::map(&fx.sliders());
        fx.prepare(LOAD_RATE, LOAD_MAX_BLOCK);
        let values = (0..YSFX_MAX_SLIDERS)
            .map(|i| AtomicU64::new(fx.slider_value(i).to_bits()))
            .collect();
        let text = std::fs::read(path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        Ok(Self {
            name,
            vendor: fx.author(),
            version: vox_sandbox_ipc::jsfx::version(&text).unwrap_or_default(),
            has_serialize: fx.has_section(YSFX_SECTION_SERIALIZE),
            params,
            num_in,
            num_out,
            values,
            core: Mutex::new(Core { fx, audio: None }),
        })
    }

    fn param(&self, id: ParamId) -> Option<&ParamInfo> {
        self.params.iter().find(|p| p.id == id)
    }

    fn mirror(&self, index: u32) -> f64 {
        self.values
            .get(index as usize)
            .map_or(0.0, |a| f64::from_bits(a.load(Ordering::Acquire)))
    }

    fn store(&self, index: u32, v: f64) {
        if let Some(a) = self.values.get(index as usize) {
            a.store(v.to_bits(), Ordering::Release);
        }
    }

    /// Re-reads every parameter's slider into the mirror.
    fn refresh(&self, fx: &Fx) {
        for p in &self.params {
            self.store(p.id.0, fx.slider_value(p.id.0));
        }
    }
}

impl PluginInstance for JsfxInstance {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            editor: false,
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version.clone(),
            params: self.params.clone(),
            groups: Vec::new(),
            values: self
                .params
                .iter()
                .map(|p| {
                    let v = self.mirror(p.id.0);
                    ParamValue {
                        id: p.id,
                        value: if v.is_finite() {
                            v.clamp(p.min, p.max)
                        } else {
                            p.default
                        },
                    }
                })
                .collect(),
            param_text: false,
        }
    }

    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String> {
        let mut guard = lock(&self.core);
        let core = &mut *guard;
        if core.audio.is_some() {
            return Err("already active".into());
        }
        let max_block = config.max_block.max(1);
        core.fx.prepare(config.sample_rate, max_block);
        let mut st = AudioState::new(self.num_in, self.num_out, max_block as usize);
        // `@slider` (and a zero-length `@block`) now, as REAPER runs `@slider` after `@init`: a
        // latency the script sets there is known before the stream starts.
        let silence = [0.0f32; 1];
        st.aim(&silence, 0);
        // SAFETY: every pin points at a live buffer; no frame is read or written.
        unsafe { core.fx.process(&st.in_ptrs, &st.out_ptrs, 0) };
        st.latency = latency_samples(core.fx.pdc_delay());
        self.refresh(&core.fx);
        let latency = st.latency;
        core.audio = Some(st);
        Ok(ActiveInfo {
            latency_samples: latency,
            // JSFX has no tail report.
            tail_samples: Some(0),
        })
    }

    fn deactivate(&self) {
        lock(&self.core).audio = None;
    }

    fn process(&self, chunk: Chunk<'_>, out_events: &mut Vec<WireEvent>) {
        let mut guard = lock(&self.core);
        let Core { fx, audio } = &mut *guard;
        let n = chunk.input.len().min(chunk.output.len());
        let Some(st) = audio.as_mut().filter(|s| n <= s.max_block) else {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
            return;
        };
        if n == 0 {
            return;
        }
        let input = &chunk.input[..n];
        // Segments split at the parameter events' offsets: `@slider` runs at a segment's start.
        let events = chunk.events;
        let (mut i, mut seg) = (0usize, 0usize);
        while seg < n {
            while let Some(ev) = events.get(i) {
                if ev.kind == EventKind::PARAM_VALUE {
                    if chunk.offset(ev) > seg {
                        break;
                    }
                    if self
                        .param(ParamId(ev.id))
                        .is_some_and(|p| !p.flags.contains(ParamFlags::READ_ONLY))
                    {
                        fx.set_slider(ev.id, ev.value);
                        self.store(ev.id, ev.value);
                    }
                }
                i += 1;
            }
            let end = events.get(i).map_or(n, |ev| chunk.offset(ev).min(n));
            st.aim(input, seg);
            // SAFETY: the pins point at `input[seg..]`, silence and the output buffers, each
            // holding at least `end - seg` frames (`end <= n <= max_block`); the audio thread
            // holds the effect's lock.
            unsafe { fx.process(&st.in_ptrs, &st.out_ptrs, (end - seg) as u32) };
            seg = end;
        }
        // Output: the main output pins averaged (a single pin is copied).
        let mains = self.num_out.min(MAIN_PINS);
        chunk.output[..n].copy_from_slice(&st.outs[0][..n]);
        for b in &st.outs[1..mains] {
            for (o, s) in chunk.output[..n].iter_mut().zip(&b[..n]) {
                *o += *s;
            }
        }
        if mains > 1 {
            let scale = 1.0 / mains as f32;
            for o in &mut chunk.output[..n] {
                *o *= scale;
            }
        }
        // Sliders the script moved itself.
        for p in &self.params {
            let v = fx.slider_value(p.id.0);
            if v.to_bits() != self.mirror(p.id.0).to_bits()
                && out_events.len() < out_events.capacity()
            {
                self.store(p.id.0, v);
                out_events.push(WireEvent {
                    pos: chunk.pos,
                    kind: EventKind::PARAM_VALUE,
                    id: p.id.0,
                    value: v,
                });
            }
        }
        let l = latency_samples(fx.pdc_delay());
        if l != st.latency && !st.restart_sent && out_events.len() < out_events.capacity() {
            st.restart_sent = true;
            out_events.push(WireEvent {
                pos: chunk.pos,
                kind: EventKind::RESTART_REQUEST,
                id: 0,
                value: 0.0,
            });
        }
    }

    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String> {
        let Some(p) = self.param(id) else {
            return Err(format!("no parameter {}", id.0));
        };
        if p.flags.contains(ParamFlags::READ_ONLY) {
            return Err(format!("parameter {} is read-only", id.0));
        }
        lock(&self.core).fx.set_slider(id.0, value);
        self.store(id.0, value);
        Ok(())
    }

    fn save_state(&self) -> Result<Vec<u8>, String> {
        let blob = if self.has_serialize {
            // `@serialize` can't run alongside `@sample`: while active, the audio thread waits
            // for it.
            let (sliders, data) = lock(&self.core).fx.save_state()?;
            Blob { sliders, data }
        } else {
            Blob {
                sliders: self
                    .params
                    .iter()
                    .map(|p| (p.id.0, self.mirror(p.id.0)))
                    .collect(),
                data: Vec::new(),
            }
        };
        Ok(blob.encode())
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let blob = Blob::decode(data)?;
        let mut core = lock(&self.core);
        core.fx.load_state(&blob.sliders, &blob.data)?;
        self.refresh(&core.fx);
        Ok(())
    }
}
