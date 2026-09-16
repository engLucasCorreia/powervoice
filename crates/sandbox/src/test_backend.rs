//! The built-in **test backend** (T-802): format `"test"`, hosting the built-in Gain module
//! (`vox_modules::Gain`) inside the sandbox, with T-801's fault behaviours on top:
//!
//! | Plugin | Behaviour |
//! |---|---|
//! | `gain` | the Gain module, bit-identical to the in-process one |
//! | `crash` | Gain, then `abort()` inside the callback at chunk `after` |
//! | `hang` | Gain, then sleeps forever inside the callback at chunk `after` |
//! | `slow` | Gain, but `late_percent` % of chunks (seeded PRNG) first sleep `late_periods` × the chunk's period |
//!
//! Options follow the name: `crash?after=200`, `slow?late_percent=10&late_periods=2.5&seed=7`.
//! State = the Gain's `ModuleState` as JSON.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use vox_module_api::{
    ActivateConfig, EventList, Module, ModuleState, OutputEvents, ParamEvent, ParamId,
    ProcessContext, Tail, Transport, prepare_state,
};
use vox_modules::Gain;
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::test_plugins::{crash_now, hang_forever};
use vox_sandbox_ipc::{Chunk, EventKind, WireEvent};

use crate::backend::{ActiveInfo, PluginBackend, PluginInstance};

/// Default `after` (chunks) for `crash` / `hang`.
const DEFAULT_AFTER: u64 = 50;

/// The `"test"` backend.
pub struct TestBackend;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    None,
    Crash { after: u64 },
    Hang { after: u64 },
    Slow { percent: u64, periods: f64 },
}

fn parse_spec(spec: &str) -> Result<(Fault, u64, &'static str), String> {
    let (name, query) = spec.split_once('?').unwrap_or((spec, ""));
    let mut after = DEFAULT_AFTER;
    let mut seed = 1u64;
    let mut percent = 10u64;
    let mut periods = 2.5f64;
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| format!("bad option `{kv}`"))?;
        let bad = || format!("bad value for `{k}`: `{v}`");
        match k {
            "after" => after = v.parse().map_err(|_| bad())?,
            "seed" => seed = v.parse().map_err(|_| bad())?,
            "late_percent" => percent = v.parse().map_err(|_| bad())?,
            "late_periods" => periods = v.parse().map_err(|_| bad())?,
            _ => return Err(format!("unknown option `{k}`")),
        }
    }
    Ok(match name {
        "gain" => (Fault::None, seed, "Gain"),
        "crash" => (Fault::Crash { after }, seed, "Crash test"),
        "hang" => (Fault::Hang { after }, seed, "Hang test"),
        "slow" => (Fault::Slow { percent, periods }, seed, "Slow test"),
        other => return Err(format!("no test plugin named `{other}`")),
    })
}

impl PluginBackend for TestBackend {
    fn name(&self) -> &'static str {
        "test"
    }

    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        let (fault, seed, name) = parse_spec(plugin)?;
        Ok(Arc::new(TestPlugin {
            name,
            fault,
            inner: Mutex::new(Inner {
                module: Box::new(Gain::new()),
                events: EventList::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
                out: OutputEvents::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
                steady: 0,
                chunks: 0,
                rng: seed | 1,
                sample_rate: 48_000.0,
            }),
        }))
    }
}

struct Inner {
    module: Box<dyn Module>,
    events: EventList,
    out: OutputEvents,
    steady: u64,
    chunks: u64,
    rng: u64,
    sample_rate: f64,
}

struct TestPlugin {
    name: &'static str,
    fault: Fault,
    /// Locked by the audio thread per chunk and by the main thread for control calls (short,
    /// uncontended in practice; the sandbox's own business, not the editor's RT rules).
    inner: Mutex<Inner>,
}

impl TestPlugin {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PluginInstance for TestPlugin {
    fn info(&self) -> PluginInfo {
        let g = self.lock();
        let m = &g.module;
        PluginInfo {
            editor: false,
            name: self.name.to_owned(),
            vendor: "PowerVoice".to_owned(),
            version: m.descriptor().version.to_string(),
            params: m.params().to_vec(),
            groups: m.groups().to_vec(),
            values: m
                .params()
                .iter()
                .map(|p| ParamValue {
                    id: p.id,
                    value: m.param_value(p.id).unwrap_or(p.default),
                })
                .collect(),
            param_text: false,
        }
    }

    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String> {
        let mut g = self.lock();
        g.module.activate(config).map_err(|e| e.to_string())?;
        g.sample_rate = config.sample_rate;
        g.steady = 0;
        Ok(ActiveInfo {
            latency_samples: g.module.latency_samples(),
            tail_samples: match g.module.tail() {
                Tail::Samples(n) => Some(n),
                Tail::Infinite => None,
            },
        })
    }

    fn deactivate(&self) {
        self.lock().module.deactivate();
    }

    fn process(&self, chunk: Chunk<'_>, out_events: &mut Vec<WireEvent>) {
        let mut guard = self.lock();
        let g = &mut *guard;
        match self.fault {
            Fault::Crash { after } if g.chunks >= after => crash_now(),
            Fault::Hang { after } if g.chunks >= after => hang_forever(),
            Fault::Slow { percent, periods } => {
                // xorshift64
                g.rng ^= g.rng << 13;
                g.rng ^= g.rng >> 7;
                g.rng ^= g.rng << 17;
                if g.rng % 100 < percent {
                    let period = chunk.input.len() as f64 / g.sample_rate;
                    std::thread::sleep(Duration::from_secs_f64(period * periods));
                }
            }
            _ => {}
        }
        g.chunks += 1;
        g.events.clear();
        for ev in chunk.events {
            if ev.kind == EventKind::RESET {
                // At the chunk's first sample: the plugin end ends a chunk at a reset (H-55).
                g.module.reset();
            } else if ev.kind == EventKind::PARAM_VALUE {
                let _ = g.events.push(ParamEvent {
                    offset: chunk.offset(ev) as u32,
                    id: ParamId(ev.id),
                    value: ev.value,
                });
            }
        }
        g.out.clear();
        let frames = chunk.input.len() as u32;
        let mut ctx = ProcessContext::new(
            frames,
            g.steady,
            Transport::default(),
            g.events.as_slice(),
            &mut g.out,
        );
        let mut outputs = [chunk.output];
        g.module.process(&mut ctx, &[chunk.input], &mut outputs);
        g.steady += u64::from(frames);
        for e in g.out.as_slice() {
            if out_events.len() < out_events.capacity() {
                out_events.push(WireEvent::param(chunk.pos, e.id.0, e.value));
            }
        }
    }

    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String> {
        let mut g = self.lock();
        let key = g
            .module
            .params()
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.key.clone())
            .ok_or_else(|| format!("no parameter {}", id.0))?;
        let mut state = g.module.save_state().map_err(|e| e.to_string())?;
        state.params.insert(key, value);
        let state = prepare_state(&*g.module, state).map_err(|e| e.to_string())?;
        g.module.load_state(&state).map_err(|e| e.to_string())
    }

    fn save_state(&self) -> Result<Vec<u8>, String> {
        let state = self.lock().module.save_state().map_err(|e| e.to_string())?;
        serde_json::to_vec(&state).map_err(|e| e.to_string())
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        let state: ModuleState = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        let mut g = self.lock();
        let state = prepare_state(&*g.module, state).map_err(|e| e.to_string())?;
        g.module.load_state(&state).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse() {
        assert_eq!(parse_spec("gain").unwrap().0, Fault::None);
        assert_eq!(
            parse_spec("crash?after=7").unwrap().0,
            Fault::Crash { after: 7 }
        );
        assert_eq!(
            parse_spec("hang").unwrap().0,
            Fault::Hang {
                after: DEFAULT_AFTER
            }
        );
        let (f, seed, _) = parse_spec("slow?late_percent=20&late_periods=1.5&seed=9").unwrap();
        assert_eq!(
            (f, seed),
            (
                Fault::Slow {
                    percent: 20,
                    periods: 1.5
                },
                9
            )
        );
        assert!(parse_spec("nope").is_err());
        assert!(parse_spec("crash?after=x").is_err());
        assert!(parse_spec("crash?bogus=1").is_err());
    }

    #[test]
    fn state_and_params_round_trip() {
        let p = TestBackend.load("gain").unwrap();
        p.set_param(Gain::GAIN_DB, -9.5).unwrap();
        let data = p.save_state().unwrap();
        let q = TestBackend.load("gain").unwrap();
        q.load_state(&data).unwrap();
        let v = q.info().values[0];
        assert_eq!(
            (v.id, v.value.to_bits()),
            (Gain::GAIN_DB, (-9.5f64).to_bits())
        );
        assert!(p.set_param(ParamId(99), 0.0).is_err());
        assert!(q.load_state(b"not json").is_err());
    }
}
