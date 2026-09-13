//! The content-keyed tile cache (SPEC-007 §2.8, §4.5; ADR-004 §4): LRU, capped at 25 % of the
//! store's memory budget, each entry counted against that budget through a
//! [`MemoryReservation`].

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use vox_project::{ChunkStore, MemoryReservation};

use super::tile::TileKey;

/// Share of the memory budget the tile cache may use (SPEC-007 §3 `tile_cache_share`).
pub const TILE_CACHE_BUDGET_DIVISOR: u64 = 4;

struct Entry {
    payload: Arc<[u8]>,
    last_use: u64,
    _reservation: MemoryReservation,
}

pub(crate) struct TileCache {
    map: HashMap<Arc<TileKey>, Entry>,
    tick: u64,
    bytes: u64,
    /// The store whose chunk ids the keys refer to. A `Weak` never keeps the store alive (a
    /// session can close while tiles are cached) and keeps its address from being reused, so
    /// `ptr_eq` is a sound identity check.
    store: Weak<ChunkStore>,
    cap_override: Option<u64>,
}

impl TileCache {
    pub(crate) fn new(cap_override: Option<u64>) -> Self {
        TileCache {
            map: HashMap::new(),
            tick: 0,
            bytes: 0,
            store: Weak::new(),
            cap_override,
        }
    }

    fn is_bound_to(&self, store: &Arc<ChunkStore>) -> bool {
        std::ptr::eq(self.store.as_ptr(), Arc::as_ptr(store))
    }

    /// Binds the cache to `store`: a different store (another document) clears it, since chunk
    /// ids restart per store.
    pub(crate) fn bind(&mut self, store: &Arc<ChunkStore>) {
        if !self.is_bound_to(store) {
            self.clear();
            self.store = Arc::downgrade(store);
        }
    }

    /// The cached payload for `key`, marking it most recently used.
    pub(crate) fn get(&mut self, key: &TileKey) -> Option<Arc<[u8]>> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|e| {
            e.last_use = tick;
            Arc::clone(&e.payload)
        })
    }

    /// Caches `payload` under `key`, evicting least-recently-used entries to stay under the cap.
    /// Ignored when `store` isn't the bound store (a job of a previous document finishing late).
    pub(crate) fn insert(
        &mut self,
        store: &Arc<ChunkStore>,
        key: Arc<TileKey>,
        payload: Arc<[u8]>,
    ) {
        if !self.is_bound_to(store) || self.map.contains_key(&key) {
            return;
        }
        let cap = self
            .cap_override
            .unwrap_or(store.memory_budget() / TILE_CACHE_BUDGET_DIVISOR);
        let len = payload.len() as u64;
        if len > cap {
            return;
        }
        while self.bytes + len > cap && self.evict_lru() {}
        self.tick += 1;
        let reservation = store.reserve_external(len);
        self.bytes += len;
        self.map.insert(
            key,
            Entry {
                payload,
                last_use: self.tick,
                _reservation: reservation,
            },
        );
    }

    fn evict_lru(&mut self) -> bool {
        let Some(key) = self
            .map
            .iter()
            .min_by_key(|(_, e)| e.last_use)
            .map(|(k, _)| Arc::clone(k))
        else {
            return false;
        };
        if let Some(e) = self.map.remove(&key) {
            self.bytes -= e.payload.len() as u64;
        }
        true
    }

    /// Drops every entry (and its memory reservation).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Payload bytes currently cached.
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
}
