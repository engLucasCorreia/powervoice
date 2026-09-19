//! H-96 "belt and braces" recovery: a bounded, last-known-status cache shared by every job
//! service (export, normalize peak/LUFS, bake), queried by the `job_status` command.
//!
//! The root fix for H-96 is ordering (the UI subscribes to `job_progress` before it starts a
//! job), but a UI that ever misses a terminal event anyway — a dropped IPC message, a webview
//! reload mid-job — must still be able to recover rather than showing "running" forever. The UI
//! polls this cache on a timeout while a job looks stuck; a service records every progress tick
//! (running or terminal) it emits, so a query always answers with the most recent truth, not just
//! "done vs unknown".
//!
//! Bounded by `cap`: only the most recent `cap` job ids are kept (oldest evicted first). A job id
//! is only ever interesting to query for the few seconds after it starts, so this never grows
//! without bound over an app session.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::ipc::JobProgressDto;

struct Inner {
    cap: usize,
    by_id: HashMap<u32, JobProgressDto>,
    order: VecDeque<u32>,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state and shared by every job service's
/// composition-root `start()` call (`lib.rs`).
#[derive(Clone)]
pub struct JobStatusRegistry(Arc<Mutex<Inner>>);

impl JobStatusRegistry {
    pub fn new(cap: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            cap,
            by_id: HashMap::new(),
            order: VecDeque::new(),
        })))
    }

    /// Records the latest known status for `dto.job_id` (called for every progress tick a job
    /// service emits, running or terminal — the last write for an id is always what a query
    /// returns).
    pub fn record(&self, dto: JobProgressDto) {
        let mut inner = self.0.lock().unwrap();
        if !inner.by_id.contains_key(&dto.job_id) {
            inner.order.push_back(dto.job_id);
            while inner.order.len() > inner.cap {
                if let Some(oldest) = inner.order.pop_front() {
                    inner.by_id.remove(&oldest);
                }
            }
        }
        inner.by_id.insert(dto.job_id, dto);
    }

    /// The last known status for `job_id`, or `None` for an id this registry never saw (unknown,
    /// or evicted long ago).
    pub fn get(&self, job_id: u32) -> Option<JobProgressDto> {
        self.0.lock().unwrap().by_id.get(&job_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{JobKind, JobState};

    fn dto(job_id: u32, fraction: f32, state: JobState) -> JobProgressDto {
        JobProgressDto {
            job_id,
            kind: JobKind::Export,
            state,
            fraction,
        }
    }

    #[test]
    fn unknown_id_is_none() {
        let reg = JobStatusRegistry::new(4);
        assert!(reg.get(1).is_none());
    }

    #[test]
    fn records_the_latest_status_for_an_id() {
        let reg = JobStatusRegistry::new(4);
        reg.record(dto(1, 0.0, JobState::Running));
        assert_eq!(reg.get(1).unwrap().state, JobState::Running);
        reg.record(dto(1, 1.0, JobState::Done));
        let got = reg.get(1).unwrap();
        assert_eq!(got.state, JobState::Done);
        assert!((got.fraction - 1.0).abs() < 1e-6);
    }

    #[test]
    fn evicts_the_oldest_id_once_over_capacity() {
        let reg = JobStatusRegistry::new(2);
        reg.record(dto(1, 1.0, JobState::Done));
        reg.record(dto(2, 1.0, JobState::Done));
        reg.record(dto(3, 1.0, JobState::Done));
        assert!(reg.get(1).is_none(), "the oldest id is evicted");
        assert!(reg.get(2).is_some());
        assert!(reg.get(3).is_some());
    }
}
