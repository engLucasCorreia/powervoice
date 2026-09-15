//! Spectrogram tile service (T-204; SPEC-007 §2.8, §4.3–§4.6; ADR-003 `VXST` + Amendment 1;
//! ADR-001 §4: the tile service lives in `engine`, the STFT in `dsp`).
//!
//! - A spectral view attaches a [`TileSink`] once ([`SpectroService::attach`]), then sends
//!   [`SpectroRequest`]s: FFT size, hop, window and up to 64 tile indices, visible tiles first.
//! - Tiles are computed on the service's own worker threads (never the audio thread), read
//!   through [`vox_project::SnapshotReader`]s of the request's snapshot.
//! - Overview hops (`hop > N`) first get a fast `PREVIEW` tile (single frame per hop), then the
//!   refined mean-power tile of the same `tile_index`. `LAST` marks the final refined tile.
//! - A newer request on a view cancels the older one's queued and in-flight tiles (workers check
//!   between frames; a finished tile of a superseded request is not sent).
//! - Payloads are cached by content ([`tile`] keys: the chunk runs of the tile's input span plus
//!   the parameters), so a re-request after a marker-only edit, or of tiles an edit didn't touch,
//!   is served from the cache. The cache is LRU, capped at 25 % of the store's memory budget and
//!   counted against it (ADR-004 §4).
//! - Every tile is stamped with the snapshot's `audio_rev`; the UI drops stale ones (ADR-003).
//!
//! The service never holds a strong reference to a [`ChunkStore`] outside a tile computation, so
//! it never keeps `Session::close` from succeeding for longer than one tile.

mod cache;
mod tile;
mod vxst;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;

use vox_dsp::spectro::{
    FrameAnalyzer, Q_CEIL_DB, Q_FLOOR_DB, is_overview, is_valid_fft_size, is_valid_hop,
};
use vox_project::{ChunkStore, DocSnapshot, SnapshotReader};

pub use cache::TILE_CACHE_BUDGET_DIVISOR;
pub use tile::{ALGO_VERSION, TILE_FRAMES, TileGeom, tile_count, tile_frames, total_frames};
pub use vxst::{
    VXST_HEADER_LEN, VXST_MAGIC, VXST_VERSION, VxstHeader, WINDOW_HANN, decode_vxst, encode_vxst,
    vxst_flags,
};

use cache::TileCache;
use tile::{TileKey, compute_tile, tile_key};

/// At most this many tiles per request (SPEC-007 §4.6); extra indices are ignored.
pub const MAX_TILES_PER_REQUEST: usize = 64;
/// Largest accepted hop (a 256-frame tile then spans 4 Gi samples, ~24 h at 48 kHz).
pub const MAX_HOP: u32 = 1 << 24;

/// The document a request reads: the session's store and the snapshot to analyse.
#[derive(Clone)]
pub struct TileSource {
    pub store: Arc<ChunkStore>,
    pub snapshot: Arc<DocSnapshot>,
}

/// One `spectro_request` (ADR-003 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpectroRequest {
    pub request_id: u32,
    /// The `audio_rev` the UI believes is current. Informational: tiles are always stamped with
    /// the analysed snapshot's own `audio_rev`, and the UI drops stale ones.
    pub audio_rev: u64,
    pub fft_size: u32,
    pub hop: u32,
    pub window: u32,
    /// Tile indices, visible first. Duplicates and tiles past the document end are ignored.
    pub tiles: Vec<u32>,
}

/// Receives complete `VXST` frames (header + payload). Called from worker threads and from
/// [`SpectroService::request`]; must not block for long.
pub type TileSink = Box<dyn Fn(Vec<u8>) + Send>;

/// Service counters (SPEC-007 §4.5 `spectro_stats`), cumulative since construction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpectroStats {
    /// Tiles computed (previews included).
    pub computed: u64,
    /// Tiles served from the cache.
    pub cache_hits: u64,
    /// Queued or in-flight tiles dropped because a newer request (or detach) superseded them.
    pub cancelled: u64,
    /// Tiles whose samples couldn't be read (store errors).
    pub failed: u64,
    /// Payload bytes currently cached.
    pub bytes_cached: u64,
    /// The most tiles ever computed at the same time (T-602: stays ≤ half the workers while a
    /// background job holds [`SpectroService::begin_background_job`]).
    pub peak_running: usize,
}

/// A refused request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpectroError {
    #[error("no spectral view {0} is attached")]
    UnknownView(u32),
    #[error("unsupported FFT size {0} (powers of two 256 ..= 16384)")]
    InvalidFftSize(u32),
    #[error("invalid hop {hop} for FFT size {fft_size} (a power of two ≥ fft_size/16)")]
    InvalidHop { fft_size: u32, hop: u32 },
    #[error("unsupported window {0} (0 = Hann)")]
    UnsupportedWindow(u32),
}

/// Construction options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectroConfig {
    /// Worker threads (≥ 1).
    pub workers: usize,
    /// Cache cap in bytes; `None` = 25 % of the store's memory budget (SPEC-007 §2.8).
    pub cache_cap_bytes: Option<u64>,
}

impl SpectroConfig {
    /// ADR-002 §1's worker count: `clamp(cores − 2, 1, 8)`.
    pub fn default_workers() -> usize {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .saturating_sub(2)
            .clamp(1, 8)
    }
}

impl Default for SpectroConfig {
    fn default() -> Self {
        SpectroConfig {
            workers: Self::default_workers(),
            cache_cap_bytes: None,
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Generation of a view that is detached or replaced: no job matches it any more.
const DEAD: u64 = u64::MAX;

struct ViewOut {
    sink: TileSink,
    generation: u64,
    /// Refined (final) tiles of the current request not delivered yet.
    pending: usize,
    /// Tile indices of the current request whose refined tile was delivered (a late preview of
    /// the same tile is then dropped).
    refined_sent: HashSet<u32>,
}

struct View {
    /// The current request's generation, read by workers between frames.
    generation: AtomicU64,
    out: Mutex<ViewOut>,
}

/// What a tile delivery needs besides the payload.
#[derive(Clone, Copy)]
struct Delivery {
    generation: u64,
    request_id: u32,
    audio_rev: u64,
    window: u32,
    geom: TileGeom,
    preview: bool,
}

impl View {
    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    /// Sends one tile if its request is still current. Refined tiles count down the request's
    /// pending tiles; the one reaching zero carries `LAST`.
    fn deliver(&self, d: &Delivery, payload: &[u8]) {
        let mut out = lock(&self.out);
        if out.generation != d.generation {
            return;
        }
        let mut flags = 0;
        if d.preview {
            if out.refined_sent.contains(&d.geom.tile_index) {
                return;
            }
            flags |= vxst_flags::PREVIEW;
        } else {
            out.refined_sent.insert(d.geom.tile_index);
            out.pending = out.pending.saturating_sub(1);
            if out.pending == 0 {
                flags |= vxst_flags::LAST;
            }
        }
        let header = VxstHeader {
            request_id: d.request_id,
            flags,
            audio_rev: d.audio_rev,
            first_frame_center_sample: d.geom.first_frame_center_sample(),
            hop_samples: d.geom.hop,
            fft_size: d.geom.fft_size,
            frames: d.geom.frames,
            bins: d.geom.bins(),
            q_floor_db: Q_FLOOR_DB,
            q_ceil_db: Q_CEIL_DB,
            tile_index: d.geom.tile_index,
            window: d.window,
        };
        (out.sink)(encode_vxst(&header, payload));
    }

    /// A refined tile that failed: count it off so a later tile can still carry `LAST`.
    fn skip(&self, d: &Delivery) {
        let mut out = lock(&self.out);
        if out.generation == d.generation && !d.preview {
            out.pending = out.pending.saturating_sub(1);
        }
    }
}

struct Job {
    view: Arc<View>,
    delivery: Delivery,
    store: Weak<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    key: Arc<TileKey>,
}

struct State {
    views: HashMap<u32, Arc<View>>,
    queue: VecDeque<Job>,
    cache: TileCache,
    shutdown: bool,
    /// Tiles being computed right now.
    running: usize,
    /// Live [`BackgroundJob`] guards (T-602, SPEC-007 §4.1).
    background_jobs: usize,
    /// Highest `running` so far ([`SpectroStats::peak_running`]).
    peak_running: usize,
}

impl State {
    /// Drops `view`'s queued jobs; returns how many.
    fn drop_queued(&mut self, view: &Arc<View>) -> u64 {
        let before = self.queue.len();
        self.queue.retain(|j| !Arc::ptr_eq(&j.view, view));
        (before - self.queue.len()) as u64
    }
}

struct Shared {
    /// Worker threads started.
    workers: usize,
    state: Mutex<State>,
    work: Condvar,
    next_generation: AtomicU64,
    computed: AtomicU64,
    cache_hits: AtomicU64,
    cancelled: AtomicU64,
    failed: AtomicU64,
}

/// The spectrogram tile service: worker threads, per-view request state and the tile cache.
/// Dropping it stops and joins the workers.
pub struct SpectroService {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl SpectroService {
    /// Starts `config.workers` worker threads.
    pub fn new(config: SpectroConfig) -> Self {
        let shared = Arc::new(Shared {
            workers: config.workers.max(1),
            state: Mutex::new(State {
                views: HashMap::new(),
                queue: VecDeque::new(),
                cache: TileCache::new(config.cache_cap_bytes),
                shutdown: false,
                running: 0,
                background_jobs: 0,
                peak_running: 0,
            }),
            work: Condvar::new(),
            next_generation: AtomicU64::new(1),
            computed: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cancelled: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        });
        let threads = (0..config.workers.max(1))
            .map(|i| {
                let shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name(format!("spectro-{i}"))
                    .spawn(move || worker(&shared))
                    .expect("spawning a spectrogram worker thread")
            })
            .collect();
        SpectroService { shared, threads }
    }

    /// Attaches (or re-attaches) view `view_id` with the sink its tiles go to. A previous sink
    /// of the same view is replaced and its pending work cancelled.
    pub fn attach(&self, view_id: u32, sink: TileSink) {
        let view = Arc::new(View {
            generation: AtomicU64::new(0),
            out: Mutex::new(ViewOut {
                sink,
                generation: 0,
                pending: 0,
                refined_sent: HashSet::new(),
            }),
        });
        let mut state = lock(&self.shared.state);
        if let Some(old) = state.views.insert(view_id, view) {
            self.retire(&mut state, &old);
        }
    }

    /// Detaches view `view_id`, cancelling its pending work. Unknown ids are ignored.
    pub fn detach(&self, view_id: u32) {
        let mut state = lock(&self.shared.state);
        if let Some(old) = state.views.remove(&view_id) {
            self.retire(&mut state, &old);
        }
    }

    fn retire(&self, state: &mut State, view: &Arc<View>) {
        view.generation.store(DEAD, Ordering::Release);
        lock(&view.out).generation = DEAD;
        let dropped = state.drop_queued(view);
        self.shared.cancelled.fetch_add(dropped, Ordering::Relaxed);
    }

    /// Schedules `request` for view `view_id` over `source`, cancelling the view's older
    /// request. Cached tiles are sent before this returns; the rest arrive from the workers.
    pub fn request(
        &self,
        view_id: u32,
        source: TileSource,
        request: SpectroRequest,
    ) -> Result<(), SpectroError> {
        let SpectroRequest {
            request_id,
            fft_size,
            hop,
            window,
            ..
        } = request;
        if window != WINDOW_HANN {
            return Err(SpectroError::UnsupportedWindow(window));
        }
        if !is_valid_fft_size(fft_size) {
            return Err(SpectroError::InvalidFftSize(fft_size));
        }
        if !is_valid_hop(fft_size, hop) || hop > MAX_HOP {
            return Err(SpectroError::InvalidHop { fft_size, hop });
        }
        let snapshot = &source.snapshot;
        let overview = is_overview(fft_size, hop);
        let mut seen = HashSet::new();
        let planned: Vec<(TileGeom, Arc<TileKey>, Option<Arc<TileKey>>)> = request
            .tiles
            .iter()
            .filter(|&&t| seen.insert(t))
            .filter_map(|&t| TileGeom::new(snapshot.len_samples, fft_size, hop, t))
            .take(MAX_TILES_PER_REQUEST)
            .map(|g| {
                let refined = Arc::new(tile_key(snapshot, &g, false, window));
                let preview = overview.then(|| Arc::new(tile_key(snapshot, &g, true, window)));
                (g, refined, preview)
            })
            .collect();

        let generation = self.shared.next_generation.fetch_add(1, Ordering::Relaxed);
        let mut immediate: Vec<(Delivery, Arc<[u8]>)> = Vec::new();
        let view = {
            let mut state = lock(&self.shared.state);
            let view = state
                .views
                .get(&view_id)
                .cloned()
                .ok_or(SpectroError::UnknownView(view_id))?;
            view.generation.store(generation, Ordering::Release);
            let dropped = state.drop_queued(&view);
            self.shared.cancelled.fetch_add(dropped, Ordering::Relaxed);
            {
                let mut out = lock(&view.out);
                out.generation = generation;
                out.pending = planned.len();
                out.refined_sent.clear();
            }
            state.cache.bind(&source.store);

            let mut previews = Vec::new();
            let mut finals = Vec::new();
            for (geom, key, preview_key) in planned {
                let delivery = Delivery {
                    generation,
                    request_id,
                    audio_rev: snapshot.audio_rev,
                    window,
                    geom,
                    preview: false,
                };
                let job = |key: Arc<TileKey>, preview: bool| Job {
                    view: Arc::clone(&view),
                    delivery: Delivery {
                        preview,
                        ..delivery
                    },
                    store: Arc::downgrade(&source.store),
                    snapshot: Arc::clone(snapshot),
                    key,
                };
                if let Some(payload) = state.cache.get(&key) {
                    immediate.push((delivery, payload));
                    continue;
                }
                if let Some(preview_key) = preview_key {
                    if let Some(payload) = state.cache.get(&preview_key) {
                        immediate.push((
                            Delivery {
                                preview: true,
                                ..delivery
                            },
                            payload,
                        ));
                    } else {
                        previews.push(job(preview_key, true));
                    }
                }
                finals.push(job(key, false));
            }
            // Previews of every tile first, then the refined/plain tiles, both in request order.
            state.queue.extend(previews);
            state.queue.extend(finals);
            view
        };
        self.shared.work.notify_all();
        self.shared
            .cache_hits
            .fetch_add(immediate.len() as u64, Ordering::Relaxed);
        for (delivery, payload) in immediate {
            view.deliver(&delivery, &payload);
        }
        Ok(())
    }

    /// The service counters.
    pub fn stats(&self) -> SpectroStats {
        let (bytes_cached, peak_running) = {
            let state = lock(&self.shared.state);
            (state.cache.bytes(), state.peak_running)
        };
        SpectroStats {
            peak_running,
            computed: self.shared.computed.load(Ordering::Relaxed),
            cache_hits: self.shared.cache_hits.load(Ordering::Relaxed),
            cancelled: self.shared.cancelled.load(Ordering::Relaxed),
            failed: self.shared.failed.load(Ordering::Relaxed),
            bytes_cached,
        }
    }

    /// SPEC-007 §4.1 (T-602): "while a save, export or bake job runs, tile work uses at most half
    /// of the workers, so it never starves those jobs". Hold the returned guard for the job's
    /// whole duration; dropping it lifts the limit (tiles already running finish either way).
    pub fn begin_background_job(&self) -> BackgroundJob {
        lock(&self.shared.state).background_jobs += 1;
        BackgroundJob {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Empties the tile cache (tests; evicted tiles are recomputed on demand).
    pub fn clear_cache(&self) {
        lock(&self.shared.state).cache.clear();
    }

    /// The concurrent-tile cap in effect right now: `workers`, or half of them while a
    /// [`BackgroundJob`] is held (SPEC-007 §4.1). A synchronous witness that a save/export/bake
    /// job is holding the guard — cheaper and non-flaky for a job service's own tests (H-30) than
    /// racing real tile computation against the job to observe [`SpectroStats::peak_running`].
    pub fn worker_cap_now(&self) -> usize {
        let state = lock(&self.shared.state);
        worker_cap(self.shared.workers, state.background_jobs)
    }
}

impl Drop for SpectroService {
    fn drop(&mut self) {
        lock(&self.shared.state).shutdown = true;
        self.shared.work.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// How many tiles may be computed at once: every worker, or half of them (at least one) while a
/// background job runs (SPEC-007 §4.1).
pub fn worker_cap(workers: usize, background_jobs: usize) -> usize {
    if background_jobs > 0 {
        (workers / 2).max(1)
    } else {
        workers.max(1)
    }
}

/// A running save/export/bake job, from [`SpectroService::begin_background_job`]: tile work is
/// limited to half the workers until it is dropped.
pub struct BackgroundJob {
    shared: Arc<Shared>,
}

impl Drop for BackgroundJob {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.shared.state);
            state.background_jobs = state.background_jobs.saturating_sub(1);
        }
        self.shared.work.notify_all();
    }
}

fn next_job(shared: &Shared) -> Option<Job> {
    let mut state = lock(&shared.state);
    loop {
        if state.shutdown {
            return None;
        }
        if state.running < worker_cap(shared.workers, state.background_jobs)
            && let Some(job) = state.queue.pop_front()
        {
            state.running += 1;
            state.peak_running = state.peak_running.max(state.running);
            return Some(job);
        }
        state = shared
            .work
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

fn worker(shared: &Shared) {
    // Denormal arithmetic only costs time here; results below −150 dB quantize to 0 anyway.
    let _ftz = vox_dsp::fp::DenormalGuard::new();
    let mut analyzers: Vec<FrameAnalyzer> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    while let Some(job) = next_job(shared) {
        run_job(shared, job, &mut analyzers, &mut buf);
        lock(&shared.state).running -= 1;
        // A worker waiting on the background-job cap may take the next tile now.
        shared.work.notify_all();
    }
}

/// Computes and delivers one tile (or drops it: superseded, closed document, cache hit).
fn run_job(shared: &Shared, job: Job, analyzers: &mut Vec<FrameAnalyzer>, buf: &mut Vec<f32>) {
    let d = job.delivery;
    if !job.view.is_current(d.generation) {
        shared.cancelled.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // Another request may have computed this tile since it was queued.
    let cached = lock(&shared.state).cache.get(&job.key);
    if let Some(payload) = cached {
        shared.cache_hits.fetch_add(1, Ordering::Relaxed);
        job.view.deliver(&d, &payload);
        return;
    }
    let Some(store) = job.store.upgrade() else {
        // The document was closed meanwhile.
        shared.cancelled.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let n = d.geom.fft_size as usize;
    let index = match analyzers.iter().position(|a| a.fft_size() == n) {
        Some(i) => i,
        None => {
            analyzers.push(FrameAnalyzer::new(n));
            analyzers.len() - 1
        }
    };
    let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&job.snapshot));
    let view = &job.view;
    let result = compute_tile(
        &mut reader,
        &d.geom,
        d.preview,
        &mut analyzers[index],
        buf,
        &|| !view.is_current(d.generation),
    );
    drop(reader);
    match result {
        Ok(Some(payload)) => {
            shared.computed.fetch_add(1, Ordering::Relaxed);
            let payload: Arc<[u8]> = payload.into();
            lock(&shared.state)
                .cache
                .insert(&store, Arc::clone(&job.key), Arc::clone(&payload));
            drop(store);
            job.view.deliver(&d, &payload);
        }
        Ok(None) => {
            shared.cancelled.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            shared.failed.fetch_add(1, Ordering::Relaxed);
            job.view.skip(&d);
        }
    }
}
