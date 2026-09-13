//! Shared helpers for the `vox-project` integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vox_project::{ChunkStore, DocSnapshot, StoreOptions, WrittenAudio};
use vox_testkit::prng::Pcg32;

pub const RATE: u32 = 48_000;
pub const RATE_U64: u64 = 48_000;
pub const MIB: u64 = 1024 * 1024;

/// A unique scratch directory, removed on drop. Root: `$POWERVOICE_TEST_TMP` or the OS temp dir.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("POWERVOICE_TEST_TMP")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!(
            "vox-project-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Seeded white noise in [-0.5, 0.5).
pub fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = Pcg32::new(seed, 7);
    (0..n).map(|_| (rng.next_signed() * 0.5) as f32).collect()
}

/// Incremental FNV-1a over f32 bits, identical to `vox_testkit::golden::fnv1a_hash` over the
/// concatenation of all updates.
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Fnv {
    pub fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    pub fn update(&mut self, samples: &[f32]) {
        for &x in samples {
            for byte in x.to_bits().to_le_bytes() {
                self.0 ^= u64::from(byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    pub fn finish(self) -> u64 {
        self.0
    }
}

pub fn store_in(dir: &Path, budget_bytes: u64) -> Arc<ChunkStore> {
    ChunkStore::create(dir, 0, StoreOptions::with_memory_budget(budget_bytes)).unwrap()
}

/// Streams `samples` into the store in 10 000-sample blocks.
pub fn write_audio(store: &Arc<ChunkStore>, samples: &[f32]) -> WrittenAudio {
    let mut writer = store.writer();
    for block in samples.chunks(10_000) {
        writer.append(block).unwrap();
    }
    writer.finish().unwrap()
}

/// Reads a whole snapshot.
pub fn read_all(store: &ChunkStore, snapshot: &DocSnapshot) -> Vec<f32> {
    let mut out = vec![0.0; snapshot.len_samples as usize];
    assert_eq!(store.read(snapshot, 0, &mut out).unwrap(), out.len());
    out
}

/// Bitwise equality of two sample buffers (NaN-safe, no float comparison).
pub fn bits_equal(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}
