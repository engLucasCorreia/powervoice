//! Shared helpers for the `vox-project` integration tests.
#![allow(dead_code)]

pub mod script;

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
        static SWEEP: std::sync::Once = std::sync::Once::new();
        let base = std::env::var_os("POWERVOICE_TEST_TMP")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        // H-17 item 6: a killed test run (SIGKILL crash tests, H-05/T-301) never runs `Drop`, so
        // its `vox-project-*` dirs leak — on `/tmp`'s tmpfs that eventually fills the quota and
        // fails unrelated tests with `DiskFull`/EDQUOT (MEMORY.md). Sweep once per test binary,
        // mirroring `src-tauri/src/test_util.rs::tmp_dir`.
        SWEEP.call_once(|| sweep_finished_runs(&base));
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

/// Removes `vox-project-<tag>-<pid>-<n>` dirs under `base` whose `pid` is no longer a live
/// process. `pid` is always the field just before the trailing counter `n`, read off from the
/// end so it doesn't matter how many dashes `tag` itself contains. A no-op without `/proc`
/// (H-05's crash tests — the only ones that can leak these — are Linux-only anyway).
pub fn sweep_finished_runs(base: &Path) {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix("vox-project-") else {
            continue;
        };
        let fields: Vec<&str> = rest.split('-').collect();
        if fields.len() < 2 {
            continue;
        }
        let Ok(pid) = fields[fields.len() - 2].parse::<u32>() else {
            continue;
        };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
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
