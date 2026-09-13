//! H-02 golden test, written *before* moving the TPDF dither/quantizer from `vox-io` into
//! `vox_dsp::dither` and before making the save path stream: pins the exact WAV bytes
//! `write_wav` produces today for a fixed seed/signal, at 16-bit, 24-bit and 32-bit float, so the
//! refactor is checked against byte-identical output (ticket H-02: "the saved bytes must stay
//! identical to today's output for the same dither seed"). The hashes below were captured from
//! the pre-refactor code (a throwaway scratch test, deleted) and must never change.
//!
//! `signal()` deliberately avoids `sin`/transcendental functions (see
//! `vox_testkit::golden::fnv1a_hash`'s doc comment): a fixed xorshift64 PRNG mixes exact 16-bit
//! grid points, off-grid noise and a silent stretch across several `DITHER_BLOCK_SAMPLES` (4096)
//! blocks, so both the grid-exact passthrough and the dithered path are exercised.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use vox_io::{BitDepth, write_wav};

fn tmp_path(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vox-io-golden-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("f.wav")
}

/// FNV-1a over raw bytes (header + data + any patched sizes), identical algorithm to
/// `vox_testkit::golden::fnv1a_hash` but over `u8` directly rather than `f32` bits, so it also
/// catches an accidental change to the container layout, not just the sample values.
fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn signal(n: usize) -> Vec<f32> {
    let mut state: u64 = 0x243F_6A88_85A3_08D3;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..n)
        .map(|i| {
            if i % 9000 < 500 {
                0.0f32 // a silent stretch: must stay exactly silent (no dither on zero)
            } else if i % 5 == 0 {
                // an exact 16-bit grid point
                ((next() % 65536) as i64 - 32768) as f32 / 32768.0
            } else {
                let v = (next() >> 40) as f32 / (1u32 << 24) as f32; // in [0, 1)
                (v * 2.0 - 1.0) * 0.98
            }
        })
        .collect()
}

#[test]
fn wav_bytes_are_unchanged_by_the_dither_move_and_streaming_refactor() {
    let samples = signal(50_000);
    let cases = [
        (
            "16",
            BitDepth::Int16,
            100_044usize,
            0x8b6c_925c_da91_a263u64,
        ),
        ("24", BitDepth::Int24, 150_068, 0xf931_977a_f3a2_17e7),
        ("32f", BitDepth::Float32, 200_068, 0x175a_8ba2_5098_bafe),
    ];
    for (tag, bits, expected_len, expected_hash) in cases {
        let path = tmp_path(tag);
        write_wav(&path, 48_000, bits, &samples).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), expected_len, "{tag}-bit: file length changed");
        assert_eq!(
            fnv1a_bytes(&bytes),
            expected_hash,
            "{tag}-bit: WAV bytes changed — the dither move and/or streaming refactor must be \
             byte-identical to the pre-H-02 output for the same seed"
        );
    }
}
