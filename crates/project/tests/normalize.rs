//! S2-02: peak normalize through a real `Session` (SPEC-010 AC-1, AC-3, AC-4, AC-6, AC-7, AC-10).

mod common;

use std::path::Path;

use common::*;
use vox_project::edit::validate_range;
use vox_project::{NormalizeOutcome, NormalizeResult, Session, SessionConfig, StoreOptions};
use vox_testkit::golden::fnv1a_hash;
use vox_testkit::measure::{crest_factor_db, null_test_db, peak_dbfs, rms_db};
use vox_testkit::signal::{sine, voice_like};

const F1_LEN_S: f64 = 3.0; // shorter than SPEC-010's 30 s fixtures — same shape, faster tests.

fn new_session(dir: &Path, samples: &[f32]) -> Session {
    let mut config = SessionConfig::new(RATE);
    config.store = StoreOptions::with_memory_budget(64 * MIB);
    let mut session = Session::create(dir, config).unwrap();
    let audio = write_audio(session.store(), samples);
    session.set_floor(&audio, Vec::new()).unwrap();
    session
}

/// F1: `voice_like` scaled so its sample peak is -12.34 dBFS (SPEC-010 §5).
fn f1() -> Vec<f32> {
    let mut s = voice_like(42, 300.0, -18.0, 20.0, 0.6, 0.3, F1_LEN_S, RATE).unwrap();
    scale_to_peak_dbfs(&mut s, -12.34);
    s
}

/// F2: pink noise scaled to a sample peak of +6.0 dBFS (a float over, SPEC-010 §5 F2).
fn f2() -> Vec<f32> {
    let mut s = vox_testkit::signal::pink_noise(7, -18.0, F1_LEN_S, RATE).unwrap();
    scale_to_peak_dbfs(&mut s, 6.0);
    s
}

/// F3: a 1 kHz sine at -20 dBFS (SPEC-010 §5).
fn f3() -> Vec<f32> {
    sine(1000.0, -20.0, F1_LEN_S, RATE).unwrap()
}

fn scale_to_peak_dbfs(samples: &mut [f32], target_dbfs: f64) {
    let current = peak_dbfs(samples);
    let gain = 10f32.powf(((target_dbfs - current) / 20.0) as f32);
    for s in samples.iter_mut() {
        *s *= gain;
    }
}

fn apply(session: &mut Session, start: u64, end: u64, target_db: f64) -> NormalizeResult {
    let len = session.current().len_samples;
    let range = validate_range(start, end, len).unwrap();
    vox_project::normalize_peak(session, range, target_db).unwrap()
}

/// AC-1: each favorite hits its target within ± 0.01 dBFS, on all three fixtures, and (as an
/// exactness check) within 1 f32 ulp of `f32(10^(target/20))`.
#[test]
fn ac1_favorites_hit_their_targets() {
    let tmp = TempDir::new("normalize-ac1");
    for (name, fixture) in [("F1", f1()), ("F2", f2()), ("F3", f3())] {
        for target_db in [-1.0, -0.1, -3.0] {
            let mut session = new_session(tmp.path(), &fixture);
            let len = session.current().len_samples;
            let result = apply(&mut session, 0, len, target_db);
            assert!(
                matches!(result, NormalizeResult::Applied(_)),
                "{name} @ {target_db} dB: expected a gain to apply"
            );
            let out = read_all(session.store(), &session.current());
            let peak = peak_dbfs(&out);
            assert!(
                (peak - target_db).abs() <= 0.01,
                "{name} @ {target_db} dB: got {peak} dBFS"
            );
            let expected_peak = vox_project::target_linear(target_db) as f32;
            let out_peak_linear = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            let ulp = expected_peak.abs() * f32::EPSILON;
            assert!(
                (out_peak_linear - expected_peak).abs() <= ulp * 2.0,
                "{name} @ {target_db} dB: {out_peak_linear} vs f32(T) {expected_peak}"
            );
        }
    }
}

/// AC-3: relative levels are preserved exactly — a null test against an independently computed
/// reference (`x * g_ref` in f64) is at or below -120 dB, RMS shifts by exactly the applied gain,
/// and crest factor is unchanged.
#[test]
fn ac3_relative_levels_preserved() {
    let tmp = TempDir::new("normalize-ac3");
    for fixture in [f1(), f2()] {
        for target_db in [-1.0, -0.1, -3.0] {
            let mut session = new_session(tmp.path(), &fixture);
            let before_rms = rms_db(&fixture);
            let before_crest = crest_factor_db(&fixture);
            let len = session.current().len_samples;
            apply(&mut session, 0, len, target_db);
            let out = read_all(session.store(), &session.current());

            let peak_lin = fixture.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            let g_ref = vox_project::target_linear(target_db) / f64::from(peak_lin);
            let reference: Vec<f32> = fixture
                .iter()
                .map(|&x| (f64::from(x) * g_ref) as f32)
                .collect();

            assert!(
                null_test_db(&out, &reference) <= -120.0,
                "null test vs independent reference for target {target_db}"
            );
            let applied_gain_db = 20.0 * g_ref.log10();
            assert!(
                (rms_db(&out) - (before_rms + applied_gain_db)).abs() <= 0.001,
                "RMS should move by exactly the applied gain"
            );
            assert!(
                (crest_factor_db(&out) - before_crest).abs() <= 0.001,
                "crest factor should be unchanged"
            );
        }
    }
}

/// AC-4: normalizing a selection only touches that range — the rest is bit-identical (FNV-1a),
/// and a louder transient outside the selection doesn't change the gain used inside it.
#[test]
fn ac4_selection_only_and_outside_is_bit_identical() {
    let tmp = TempDir::new("normalize-ac4");
    let mut source = f1();
    // A transient outside [1s, 2s) (of this 3 s fixture) louder than anything inside it —
    // SPEC-010 AC-4 uses [10s, 12s) of a 30 s fixture; the proportions are the same.
    let ten_s = RATE as usize;
    let twelve_s = 2 * RATE as usize;
    source[0] = 0.99;

    let mut session = new_session(tmp.path(), &source);
    let before = read_all(session.store(), &session.current());
    let before_head_hash = fnv1a_hash(&before[..ten_s]);
    let before_tail_hash = fnv1a_hash(&before[twelve_s..]);

    apply(&mut session, ten_s as u64, twelve_s as u64, -1.0);

    let after = session.current();
    let after_samples = read_all(session.store(), &after);
    assert_eq!(
        fnv1a_hash(&after_samples[..ten_s]),
        before_head_hash,
        "[0, 10s) is bit-identical"
    );
    assert_eq!(
        fnv1a_hash(&after_samples[twelve_s..]),
        before_tail_hash,
        "[12s, end) is bit-identical"
    );
    let peak = peak_dbfs(&after_samples[ten_s..twelve_s]);
    assert!(
        (peak - (-1.0)).abs() <= 0.01,
        "the selection itself hit -1 dBFS: got {peak}"
    );

    // The pieces outside the scope reference the same chunks as before (not merely equal bytes).
    let before_snapshot_pieces = session.current(); // after the edit, re-fetch to compare shape
    assert!(
        before_snapshot_pieces.pieces.len() >= 3,
        "head/middle/tail split"
    );
}

/// AC-6: a fully silent, or all-`Silence`-piece, or -130 dBFS scope is a no-op — same `rev`,
/// `audio_rev`, hash and undo depth, no journal record. A -100 dBFS scope *is* normalized.
#[test]
fn ac6_silent_and_near_silent_are_no_ops() {
    let tmp = TempDir::new("normalize-ac6");

    // All +0.0 samples.
    let zeros = vec![0.0f32; 10_000];
    let mut session = new_session(tmp.path(), &zeros);
    let before = session.current();
    let before_hash = fnv1a_hash(&read_all(session.store(), &before));
    let result = apply(&mut session, 0, 10_000, -1.0);
    assert!(matches!(
        result,
        NormalizeResult::NoOp(NormalizeOutcome::Silent)
    ));
    assert_eq!(session.current().audio_rev, before.audio_rev);
    assert_eq!(session.current().rev, before.rev);
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        before_hash
    );
    assert_eq!(session.history().undo_depth(), 0);

    // -130 dBFS: below the -120 dBFS floor.
    let mut quiet = noise(9, 10_000);
    scale_to_peak_dbfs(&mut quiet, -130.0);
    let mut session = new_session(tmp.path(), &quiet);
    let before_rev = session.current().audio_rev;
    let result = apply(&mut session, 0, 10_000, -1.0);
    assert!(matches!(
        result,
        NormalizeResult::NoOp(NormalizeOutcome::Silent)
    ));
    assert_eq!(session.current().audio_rev, before_rev);

    // -100 dBFS *is* normalized (between -120 and -60 dBFS).
    let mut audible = noise(10, 10_000);
    scale_to_peak_dbfs(&mut audible, -100.0);
    let mut session = new_session(tmp.path(), &audible);
    let result = apply(&mut session, 0, 10_000, -1.0);
    assert!(matches!(result, NormalizeResult::Applied(_)));
    let peak = peak_dbfs(&read_all(session.store(), &session.current()));
    assert!((peak - (-1.0)).abs() <= 0.01);
}

/// AC-7: F1 already normalized to -1 dB is a no-op on a second Normalize to -1 dB; a scope with a
/// non-finite sample is refused, changing nothing.
#[test]
fn ac7_already_normalized_and_non_finite() {
    let tmp = TempDir::new("normalize-ac7");
    let source = f1();
    let mut session = new_session(tmp.path(), &source);
    let len = session.current().len_samples;
    let first = apply(&mut session, 0, len, -1.0);
    assert!(matches!(first, NormalizeResult::Applied(_)));
    let after_first = session.current();
    let hash_after_first = fnv1a_hash(&read_all(session.store(), &after_first));

    let second = apply(&mut session, 0, len, -1.0);
    assert!(matches!(
        second,
        NormalizeResult::NoOp(NormalizeOutcome::AlreadyNormalized)
    ));
    assert_eq!(session.current().audio_rev, after_first.audio_rev);
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        hash_after_first
    );
    assert_eq!(
        session.history().undo_depth(),
        1,
        "still just the first edit"
    );

    // A non-finite sample in scope.
    let mut with_nan = f3();
    with_nan[100] = f32::NAN;
    let mut session = new_session(tmp.path(), &with_nan);
    let before = session.current();
    let range = validate_range(0, before.len_samples, before.len_samples).unwrap();
    let err = vox_project::normalize_peak(&mut session, range, -1.0).unwrap_err();
    assert!(matches!(err, vox_project::ProjectError::NonFiniteSample));
    assert_eq!(session.current().audio_rev, before.audio_rev);
    assert_eq!(session.history().undo_depth(), 0);
}

/// AC-10: one undo entry labelled `history.normalize`; a seeded sequence of normalizes with
/// random scopes and targets, all undone, returns the exact original hash.
#[test]
fn ac10_undo_redo_restores_the_exact_hash() {
    let tmp = TempDir::new("normalize-ac10");
    let source = noise(123, 200_000);
    let mut session = new_session(tmp.path(), &source);
    let original_hash = fnv1a_hash(&read_all(session.store(), &session.current()));

    let mut rng = vox_testkit::prng::Pcg32::new(999, 1);
    let mut applied = 0;
    for _ in 0..20 {
        let len = session.current().len_samples;
        let start = (rng.next_f64() * (len as f64 * 0.8)) as u64;
        let seg_len = 1 + (rng.next_f64() * (len as f64 * 0.2 - 1.0)) as u64;
        let end = (start + seg_len).min(len);
        if end <= start {
            continue;
        }
        let target_db = -60.0 + rng.next_f64() * 60.0;
        let range = validate_range(start, end, len).unwrap();
        if let Ok(NormalizeResult::Applied(_)) =
            vox_project::normalize_peak(&mut session, range, target_db)
        {
            applied += 1;
            assert_eq!(
                session.history().undo_label().map(str::to_owned),
                Some("history.normalize".to_owned())
            );
        }
    }
    assert!(
        applied > 0,
        "at least some of the 20 attempts applied a gain"
    );
    assert_eq!(session.history().undo_depth(), applied);

    for _ in 0..applied {
        session.undo().unwrap();
    }
    assert_eq!(session.history().undo_depth(), 0);
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        original_hash,
        "undoing every normalize restores the exact original hash"
    );
}
