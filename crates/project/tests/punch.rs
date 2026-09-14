//! T-304 (SPEC-022 §2.4–§2.12, §4.1, §4.2, §4.6): record-operation takes committed through
//! `Session::commit_take_window` — Insert, Overwrite, Punch — exact sample results on the
//! SPEC-022 §5 default fixture, the crossfade formula, markers, undo/redo exactness, cancel and
//! crash recovery of the record window (`take_window`).

mod common;

use std::f64::consts::FRAC_PI_2;
use std::path::Path;

use common::*;
use vox_project::journal::{Record, TakeModeRecord, read_journal};
use vox_project::session::TAKES_DIR_NAME;
use vox_project::take::take_part_path;
use vox_project::{
    FinishedTake, MARKER_ADD_LABEL_KEY, Marker, MarkerId, PUNCH_LABEL_KEY, Session, SessionConfig,
    StoreOptions, TAKE_LABEL_KEY, TakeMode, TakeParams, TakeWriterOptions,
};
use vox_testkit::golden::fnv1a_hash;

/// SPEC-022 §5 default fixture length: 20.000 s at 48 kHz.
const L: usize = 960_000;
/// §5: crossfade 10 ms = 480 samples.
const X: u64 = 480;
const S: usize = 240_000;
const E: usize = 384_000;

fn new_session(dir: &Path) -> Session {
    let mut config = SessionConfig::new(RATE);
    config.store = StoreOptions::with_memory_budget(256 * MIB);
    Session::create(dir, config).unwrap()
}

/// §5 default markers: points at 2, 6, 10 and 20 s (the last at `L`), a region [6.5 s, 7.5 s).
fn fixture_markers() -> Vec<Marker> {
    vec![
        Marker::new(MarkerId(1), 2 * RATE_U64, 0, "m2"),
        Marker::new(MarkerId(2), 6 * RATE_U64, 0, "m6"),
        Marker::new(MarkerId(3), 10 * RATE_U64, 0, "m10"),
        Marker::new(MarkerId(4), 20 * RATE_U64, 0, "m20"),
        Marker::new(MarkerId(5), 6 * RATE_U64 + RATE_U64 / 2, RATE_U64, "region"),
    ]
}

/// Makes `a` (with the §5 markers that fit in it) the document's undo floor.
fn floor(session: &mut Session, a: &[f32]) {
    let audio = write_audio(session.store(), a);
    let markers = fixture_markers()
        .into_iter()
        .filter(|m| m.end_samples() <= a.len() as u64)
        .collect();
    session.set_floor(&audio, markers).unwrap();
}

/// Records `samples` as an operation take in 50 ms blocks.
fn take(
    session: &mut Session,
    mode: TakeMode,
    xfade: u64,
    aligned: bool,
    samples: &[f32],
) -> FinishedTake {
    let mut capture = session
        .begin_take_with(
            mode,
            TakeParams {
                xfade_samples: xfade,
                offset_ns: 0,
                aligned,
            },
            TakeWriterOptions::default(),
        )
        .unwrap();
    for block in samples.chunks(2400) {
        capture.append(block).unwrap();
    }
    let finished = capture.finish();
    assert!(finished.error.is_none(), "{:?}", finished.error);
    finished
}

fn xfade(f: f32, g: f32, i: usize, len: usize) -> f32 {
    let theta = FRAC_PI_2 * (i as f64 + 0.5) / len as f64;
    (theta.cos() * f64::from(f) + theta.sin() * f64::from(g)) as f32
}

/// §2.6/§4.2: `a` with `[s, s + t.len())` replaced by `t`, equal-power fades inside the range.
fn expected_punch(a: &[f32], t: &[f32], s: usize, x: usize) -> Vec<f32> {
    let n = t.len();
    let x = x.min(n / 2);
    let mut out = a.to_vec();
    for i in 0..n {
        out[s + i] = if i < x {
            xfade(a[s + i], t[i], i, x)
        } else if i >= n - x {
            xfade(t[i], a[s + i], i - (n - x), x)
        } else {
            t[i]
        };
    }
    out
}

/// §2.5/§4.2: Overwrite at `c` with `w`: fade-in only when `c < L`, fade-out only when
/// `c + n < L`; extends past the end.
fn expected_overwrite(a: &[f32], w: &[f32], c: usize, x: usize) -> Vec<f32> {
    let (l, n) = (a.len(), w.len());
    let xs = if c < l { x.min(l - c).min(n / 2) } else { 0 };
    let xe = if c + n < l { x.min(n / 2) } else { 0 };
    let mut out = a[..c].to_vec();
    for i in 0..n {
        out.push(if i < xs {
            xfade(a[c + i], w[i], i, xs)
        } else if i >= n - xe {
            xfade(w[i], a[c + i], i - (n - xe), xe)
        } else {
            w[i]
        });
    }
    if c + n < l {
        out.extend_from_slice(&a[c + n..]);
    }
    out
}

fn positions(session: &Session) -> Vec<(u64, u64, u64)> {
    session
        .current()
        .markers
        .iter()
        .map(|m| (m.id.0, m.pos_samples, m.len_samples))
        .collect()
}

fn fixture_positions() -> Vec<(u64, u64, u64)> {
    let mut v: Vec<_> = fixture_markers()
        .iter()
        .map(|m| (m.id.0, m.pos_samples, m.len_samples))
        .collect();
    v.sort_by_key(|m| m.1);
    v
}

fn assert_bits(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    if let Some(i) = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!("{what}: sample {i} differs: {} vs {}", got[i], want[i]);
    }
}

/// AC-5 (project part), AC-7, AC-11: a full punch replaces exactly `[S, E)` — outside
/// bit-identical, the interior bit-identical to the take's window, both boundaries on the fade
/// formula — keeps `L` and every marker, is one "Punch-in" entry journaled with its
/// `take_window`, and undo/redo restore the exact hashes and marker lists.
#[test]
fn punch_replaces_exactly_the_selection_with_inside_crossfades() {
    let tmp = TempDir::new("punch-full");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let x = noise(2, L);
    // The whole pass: 5 s pre-roll (+ 37 samples of start latency), the window, 1 s post-roll.
    let k_start = 240_037usize;
    let mut pass = noise(9, k_start);
    pass.extend_from_slice(&x[S..E]);
    pass.extend(noise(10, 48_000));
    let finished = take(
        &mut session,
        TakeMode::Punch {
            start_samples: S as u64,
            end_samples: E as u64,
        },
        X,
        true,
        &pass,
    );
    session
        .note_take_window(finished.take, k_start as u64)
        .unwrap();
    let step = session
        .commit_take_window(&finished, (k_start as u64, (k_start + E - S) as u64), &[])
        .unwrap()
        .expect("one edit");
    assert_eq!(&*step.label_key, PUNCH_LABEL_KEY);
    let out = read_all(session.store(), &session.current());
    assert_eq!(out.len(), L, "L′ = L");
    assert_bits(&out[..S], &a[..S], "A′[0, S)");
    assert_bits(&out[E..], &a[E..], "A′[E, L)");
    assert_bits(&out[S + 480..E - 480], &x[S + 480..E - 480], "interior");
    for i in 0..480 {
        assert!((out[S + i] - xfade(a[S + i], x[S + i], i, 480)).abs() <= 1e-6);
        let j = E - 480 + i;
        assert!((out[j] - xfade(x[j], a[j], i, 480)).abs() <= 1e-6);
    }
    assert_bits(&out, &expected_punch(&a, &x[S..E], S, 480), "whole");
    assert_eq!(
        positions(&session),
        fixture_positions(),
        "markers unchanged"
    );
    assert_eq!(session.history().undo_depth(), 1);

    let journal = read_journal(session.journal_path()).unwrap();
    let begin = journal.records.iter().find_map(|r| match r {
        Record::TakeBegin {
            mode,
            at,
            len,
            xfade_samples,
            aligned,
            ..
        } => Some((*mode, *at, *len, *xfade_samples, *aligned)),
        _ => None,
    });
    assert_eq!(
        begin,
        Some((TakeModeRecord::Punch, S as u64, (E - S) as u64, X, true))
    );
    assert!(journal.records.iter().any(|r| matches!(
        r,
        Record::TakeWindow { k_start: k, .. } if *k == k_start as u64
    )));

    let result_hash = fnv1a_hash(&out);
    session.undo().unwrap().unwrap();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        fnv1a_hash(&a)
    );
    assert_eq!(positions(&session), fixture_positions());
    session.redo().unwrap().unwrap();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        result_hash
    );
}

/// AC-7: DC 0.5 old audio against a silent take (and the reverse) traces `0.5·cos θᵢ` /
/// `0.5·sin θⱼ` exactly; a 500-sample selection gets two 250-sample fades; 0 ms is a pure splice.
#[test]
fn crossfades_follow_the_equal_power_formula_and_shrink_for_short_selections() {
    let tmp = TempDir::new("punch-dc");
    let len = 96_000;
    let (s, e) = (24_000usize, 72_000usize);
    for (old, new) in [(0.5f32, 0.0f32), (0.0, 0.5)] {
        let mut session = new_session(tmp.path());
        floor(&mut session, &vec![old; len]);
        let finished = take(
            &mut session,
            TakeMode::Punch {
                start_samples: s as u64,
                end_samples: e as u64,
            },
            X,
            false,
            &vec![new; e - s],
        );
        session
            .commit_take_window(&finished, (0, (e - s) as u64), &[])
            .unwrap()
            .unwrap();
        let out = read_all(session.store(), &session.current());
        for i in 0..480 {
            let th = FRAC_PI_2 * (i as f64 + 0.5) / 480.0;
            let (fin, fout) = if old > new {
                (0.5 * th.cos(), 0.5 * th.sin())
            } else {
                (0.5 * th.sin(), 0.5 * th.cos())
            };
            assert!((f64::from(out[s + i]) - fin).abs() <= 1e-6, "in {i}");
            assert!(
                (f64::from(out[e - 480 + i]) - fout).abs() <= 1e-6,
                "out {i}"
            );
        }
    }

    // 500-sample selection: each fade is 250 samples long.
    let mut session = new_session(tmp.path());
    floor(&mut session, &vec![0.5; len]);
    let finished = take(
        &mut session,
        TakeMode::Punch {
            start_samples: 10_000,
            end_samples: 10_500,
        },
        X,
        false,
        &[0.0; 500],
    );
    session
        .commit_take_window(&finished, (0, 500), &[])
        .unwrap()
        .unwrap();
    let out = read_all(session.store(), &session.current());
    for i in 0..250 {
        let th = FRAC_PI_2 * (i as f64 + 0.5) / 250.0;
        assert!((f64::from(out[10_000 + i]) - 0.5 * th.cos()).abs() <= 1e-6);
        assert!((f64::from(out[10_250 + i]) - 0.5 * th.sin()).abs() <= 1e-6);
    }

    // 0 ms: A′ = A[0,S) ‖ X[S,E) ‖ A[E,L) exactly.
    let mut session = new_session(tmp.path());
    let a = noise(3, len);
    floor(&mut session, &a);
    let t = noise(4, e - s);
    let finished = take(
        &mut session,
        TakeMode::Punch {
            start_samples: s as u64,
            end_samples: e as u64,
        },
        0,
        false,
        &t,
    );
    session
        .commit_take_window(&finished, (0, (e - s) as u64), &[])
        .unwrap()
        .unwrap();
    let mut want = a[..s].to_vec();
    want.extend_from_slice(&t);
    want.extend_from_slice(&a[e..]);
    assert_bits(
        &read_all(session.store(), &session.current()),
        &want,
        "splice",
    );
}

/// AC-9 (project part): a partial punch (Stop at `p`) replaces `[S, p)` only, the punch-out fade
/// ending at `p`; `A[p, L)` is untouched.
#[test]
fn a_partial_punch_ends_its_fade_at_the_stop_point() {
    let tmp = TempDir::new("punch-partial");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let x = noise(2, L);
    let p = S + 100_000;
    let k = 1_000usize;
    let mut pass = noise(9, k);
    pass.extend_from_slice(&x[S..p + 5_000]); // the take ran a little past `p`
    let finished = take(
        &mut session,
        TakeMode::Punch {
            start_samples: S as u64,
            end_samples: E as u64,
        },
        X,
        true,
        &pass,
    );
    session.note_take_window(finished.take, k as u64).unwrap();
    session
        .commit_take_window(&finished, (k as u64, (k + p - S) as u64), &[])
        .unwrap()
        .unwrap();
    let out = read_all(session.store(), &session.current());
    assert_bits(&out, &expected_punch(&a, &x[S..p], S, 480), "partial punch");
    assert_bits(&out[p..], &a[p..], "A′[p, L)");
}

/// AC-3: Overwrite inside the document — outside bit-identical, interior bit-identical to `W`,
/// both fades on the formula, `L′ = L`, every marker (the 6 s one inside the range included) and
/// the region stay, one "Record" entry.
#[test]
fn overwrite_inside_replaces_as_it_grows_and_keeps_markers() {
    let tmp = TempDir::new("overwrite-inside");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let c = 240_000usize;
    let w = noise(5, 144_000);
    let finished = take(
        &mut session,
        TakeMode::Overwrite {
            at_samples: c as u64,
        },
        X,
        false,
        &w,
    );
    let step = session
        .commit_take_window(&finished, (0, w.len() as u64), &[])
        .unwrap()
        .unwrap();
    assert_eq!(&*step.label_key, TAKE_LABEL_KEY);
    let out = read_all(session.store(), &session.current());
    assert_eq!(out.len(), L);
    assert_bits(&out[..c], &a[..c], "A′[0, c)");
    assert_bits(&out[c + w.len()..], &a[c + w.len()..], "A′[c + n, L)");
    assert_bits(
        &out[c + 480..c + w.len() - 480],
        &w[480..w.len() - 480],
        "interior",
    );
    assert_bits(&out, &expected_overwrite(&a, &w, c, 480), "whole");
    assert_eq!(positions(&session), fixture_positions(), "identity");
}

/// AC-4: Overwrite past the end extends the document (fade-in only), the 20 s marker stays at
/// 960 000; at `c = L` there is no fade at all, and Insert at `L` gives the same document.
#[test]
fn overwrite_past_the_end_extends_and_equals_insert_at_the_end() {
    let tmp = TempDir::new("overwrite-end");
    let a = noise(1, L);
    let w = noise(6, 192_000);
    let c = 864_000usize;
    let mut session = new_session(tmp.path());
    floor(&mut session, &a);
    let finished = take(
        &mut session,
        TakeMode::Overwrite {
            at_samples: c as u64,
        },
        X,
        false,
        &w,
    );
    session
        .commit_take_window(&finished, (0, w.len() as u64), &[])
        .unwrap()
        .unwrap();
    let out = read_all(session.store(), &session.current());
    assert_eq!(out.len(), c + w.len(), "L′ = c + n");
    assert_bits(&out[..c], &a[..c], "A′[0, c)");
    assert_bits(&out[c + 480..], &w[480..], "no end fade");
    assert_bits(&out, &expected_overwrite(&a, &w, c, 480), "whole");
    assert_eq!(
        positions(&session),
        fixture_positions(),
        "20 s marker stays"
    );

    // c = L: Overwrite and Insert give hash-equal documents with no fade.
    let mut hashes = Vec::new();
    for mode in [
        TakeMode::Overwrite {
            at_samples: L as u64,
        },
        TakeMode::Insert {
            at_samples: L as u64,
        },
    ] {
        let mut s2 = new_session(tmp.path());
        floor(&mut s2, &a);
        let finished = take(&mut s2, mode, X, false, &w);
        s2.commit_take_window(&finished, (0, w.len() as u64), &[])
            .unwrap()
            .unwrap();
        let out = read_all(s2.store(), &s2.current());
        let mut want = a.clone();
        want.extend_from_slice(&w);
        assert_bits(&out, &want, "append");
        hashes.push(fnv1a_hash(&out));
    }
    assert_eq!(hashes[0], hashes[1]);
}

/// AC-2 (project part), AC-10: Insert at 10 s splices the take in exactly (first/last samples
/// unmodified), later markers shift by `n` per SPEC-008 §4.2, the region is unchanged, one
/// "Record" entry; undo restores the hash and markers.
#[test]
fn insert_shifts_later_audio_and_markers() {
    let tmp = TempDir::new("insert");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let c = 480_000usize;
    let w = noise(7, 144_007);
    let n = w.len() as u64;
    let finished = take(
        &mut session,
        TakeMode::Insert {
            at_samples: c as u64,
        },
        X,
        false,
        &w,
    );
    let step = session
        .commit_take_window(&finished, (0, n), &[])
        .unwrap()
        .unwrap();
    assert_eq!(&*step.label_key, TAKE_LABEL_KEY);
    let out = read_all(session.store(), &session.current());
    let mut want = a[..c].to_vec();
    want.extend_from_slice(&w);
    want.extend_from_slice(&a[c..]);
    assert_bits(&out, &want, "A[0,c) ‖ W ‖ A[c,L)");
    let got = positions(&session);
    let want_markers = vec![
        (1, 2 * RATE_U64, 0),
        (2, 6 * RATE_U64, 0),
        (5, 6 * RATE_U64 + RATE_U64 / 2, RATE_U64),
        (3, 480_000 + n, 0),
        (4, 960_000 + n, 0),
    ];
    assert_eq!(got, want_markers);
    session.undo().unwrap().unwrap();
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        fnv1a_hash(&a)
    );
    assert_eq!(positions(&session), fixture_positions());
}

/// AC-9 (project part): an empty window cancels — no edit, no undo entry, `rev` unchanged,
/// `take_cancel` journaled and the take file deleted.
#[test]
fn an_empty_window_cancels_the_take() {
    let tmp = TempDir::new("cancel");
    let mut session = new_session(tmp.path());
    floor(&mut session, &noise(1, 96_000));
    let rev = session.current().rev;
    let finished = take(
        &mut session,
        TakeMode::Punch {
            start_samples: 10_000,
            end_samples: 20_000,
        },
        X,
        true,
        &noise(2, 30_000),
    );
    let wav = take_part_path(&session.dir().join(TAKES_DIR_NAME), finished.take.0, 0);
    assert!(wav.exists());
    assert!(
        session
            .commit_take_window(&finished, (5_000, 5_000), &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(session.current().rev, rev);
    assert_eq!(session.history().undo_depth(), 0);
    assert!(!session.is_recording());
    assert!(!wav.exists(), "the take file is deleted");
    let journal = read_journal(session.journal_path()).unwrap();
    assert!(matches!(
        journal.records.last(),
        Some(Record::TakeCancel { .. })
    ));
}

/// AC-15 (project part): after a crash during the recording phase, "Apply as recorded" commits
/// the window from the journaled `take_window` to the last recovered sample — hash-equal to the
/// live partial punch at the same point. A crash in pre-roll (no `take_window`) offers nothing and
/// leaves the document as it was.
#[test]
fn recovery_applies_the_window_from_take_window_or_discards_a_pre_roll_crash() {
    let tmp = TempDir::new("punch-recover");
    let a = noise(1, L);
    let x = noise(2, L);
    let k = 240_000usize;
    let recovered = 70_000usize; // window samples captured before the "crash"
    let dir = {
        let mut session = new_session(tmp.path());
        floor(&mut session, &a);
        let mut capture = session
            .begin_take_with(
                TakeMode::Punch {
                    start_samples: S as u64,
                    end_samples: E as u64,
                },
                TakeParams {
                    xfade_samples: X,
                    offset_ns: 0,
                    aligned: true,
                },
                TakeWriterOptions::default(),
            )
            .unwrap();
        let mut pass = noise(9, k);
        pass.extend_from_slice(&x[S..S + recovered]);
        for block in pass.chunks(2400) {
            capture.append(block).unwrap();
        }
        capture.sync().unwrap();
        session.note_take_window(capture.id(), k as u64).unwrap();
        let dir = session.dir().to_path_buf();
        drop(capture); // crash: never finished
        drop(session);
        dir
    };
    let (mut session, report) =
        Session::recover(&dir, StoreOptions::with_memory_budget(256 * MIB)).unwrap();
    let info = report.open_take.expect("the interrupted punch is offered");
    assert_eq!(info.at_samples, S as u64);
    assert_eq!(info.samples, recovered as u64);
    let step = session.apply_open_take_from_wav().unwrap().unwrap();
    assert_eq!(&*step.label_key, PUNCH_LABEL_KEY);
    let out = read_all(session.store(), &session.current());
    assert_bits(
        &out,
        &expected_punch(&a, &x[S..S + recovered], S, 480),
        "recovered partial punch",
    );
    drop(session);

    // Pre-roll crash: an aligned take with no `take_window` is not offered.
    let tmp2 = TempDir::new("punch-recover-preroll");
    let dir = {
        let mut session = new_session(tmp2.path());
        floor(&mut session, &a);
        let mut capture = session
            .begin_take_with(
                TakeMode::Punch {
                    start_samples: S as u64,
                    end_samples: E as u64,
                },
                TakeParams {
                    xfade_samples: X,
                    offset_ns: 0,
                    aligned: true,
                },
                TakeWriterOptions::default(),
            )
            .unwrap();
        capture.append(&noise(9, 10_000)).unwrap();
        capture.sync().unwrap();
        let dir = session.dir().to_path_buf();
        drop(capture);
        drop(session);
        dir
    };
    let (session, report) =
        Session::recover(&dir, StoreOptions::with_memory_budget(256 * MIB)).unwrap();
    assert!(report.open_take.is_none());
    assert!(!session.is_recording());
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        fnv1a_hash(&a)
    );
}

/// `positions` sorted by (position, id), so marker lists compare as sets.
fn sorted_positions(mut v: Vec<(u64, u64, u64)>) -> Vec<(u64, u64, u64)> {
    v.sort_by_key(|m| (m.1, m.0));
    v
}

/// Opens an aligned punch take over `[S, E)` and appends `pass`.
fn open_punch(session: &mut Session, pass: &[f32]) -> vox_project::TakeCapture {
    let mut capture = session
        .begin_take_with(
            TakeMode::Punch {
                start_samples: S as u64,
                end_samples: E as u64,
            },
            TakeParams {
                xfade_samples: X,
                offset_ns: 0,
                aligned: true,
            },
            TakeWriterOptions::default(),
        )
        .unwrap();
    for block in pass.chunks(2400) {
        capture.append(block).unwrap();
    }
    capture
}

/// H-21 / AC-10 (project part): markers added during a punch — in pre-roll (heard position before
/// `S`), in the window (`S + k`) and in post-roll (heard position after `E`) — land exactly where
/// they were added, inside the operation's single "Punch-in" edit, with the fixture markers kept
/// (identity). Undo restores the original marker list exactly (ids, names, positions), redo the
/// result.
#[test]
fn ac10_markers_added_during_a_punch_land_in_its_edit() {
    let tmp = TempDir::new("punch-markers");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let (pre, post) = (240_000usize, 48_000usize);
    let mut capture = open_punch(&mut session, &noise(3, pre + (E - S) + post));
    let id = capture.id();
    session.note_take_window(id, pre as u64).unwrap();
    let added = [
        (100, S as u64 - 96_000, "pre"),
        (101, S as u64 + 48_000, "window"),
        (102, E as u64 + 24_000, "post"),
    ];
    for (mid, pos, name) in added {
        session
            .note_take_marker(id, Marker::new(MarkerId(mid), pos, 0, name))
            .unwrap();
    }
    assert_eq!(session.open_take_markers().len(), 3);
    capture.sync().unwrap();
    let finished = capture.finish();
    let step = session
        .commit_take_window(&finished, (pre as u64, (pre + E - S) as u64), &[])
        .unwrap()
        .unwrap();
    assert_eq!(&*step.label_key, PUNCH_LABEL_KEY);
    assert_eq!(session.history().undo_depth(), 1, "all in the one edit");
    assert!(session.open_take_markers().is_empty());
    let mut want = fixture_positions();
    want.extend(added.iter().map(|&(mid, pos, _)| (mid, pos, 0)));
    let want = sorted_positions(want);
    assert_eq!(sorted_positions(positions(&session)), want);
    let window = session
        .current()
        .markers
        .iter()
        .find(|m| m.id == MarkerId(101))
        .map(|m| m.name.to_string());
    assert_eq!(window.as_deref(), Some("window"));

    session.undo().unwrap().unwrap();
    assert_eq!(
        sorted_positions(positions(&session)),
        sorted_positions(fixture_positions())
    );
    session.redo().unwrap().unwrap();
    assert_eq!(sorted_positions(positions(&session)), want);
}

/// H-21 / AC-10 (project part): in Insert mode a marker added at take offset `k` lands at `c + k`
/// (inside the new audio) while the existing markers at or after `c` shift by `n`; a marker added
/// just past the take's end (extrapolated) is clamped to `c + n`; one before `c` stays put.
#[test]
fn ac10_insert_markers_land_at_the_take_offset() {
    let tmp = TempDir::new("insert-markers");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let c = 480_000u64;
    let w = noise(7, 96_000);
    let n = w.len() as u64;
    let mut capture = session
        .begin_take_with(
            TakeMode::Insert { at_samples: c },
            TakeParams::default(),
            TakeWriterOptions::default(),
        )
        .unwrap();
    let id = capture.id();
    for block in w.chunks(2400) {
        capture.append(block).unwrap();
    }
    for (mid, pos) in [(100, c - 1_000), (101, c + 30_000), (102, c + n + 300)] {
        session
            .note_take_marker(id, Marker::new(MarkerId(mid), pos, 0, "m"))
            .unwrap();
    }
    let finished = capture.finish();
    session
        .commit_take_window(&finished, (0, n), &[])
        .unwrap()
        .unwrap();
    let want = sorted_positions(vec![
        (1, 2 * RATE_U64, 0),
        (2, 6 * RATE_U64, 0),
        (5, 6 * RATE_U64 + RATE_U64 / 2, RATE_U64),
        (3, 480_000 + n, 0),
        (4, 960_000 + n, 0),
        (100, c - 1_000, 0),
        (101, c + 30_000, 0),
        (102, c + n, 0),
    ]);
    assert_eq!(sorted_positions(positions(&session)), want);
    session.undo().unwrap().unwrap();
    assert_eq!(
        sorted_positions(positions(&session)),
        sorted_positions(fixture_positions())
    );
}

/// H-21 / AC-10 (project part): an operation cancelled after markers were added during it (Stop
/// in pre-roll) keeps each marker as its own ordinary "Add Marker" undo entry; the audio is
/// untouched and the take is journaled as cancelled.
#[test]
fn ac10_a_cancelled_operation_keeps_its_markers_as_add_marker_entries() {
    let tmp = TempDir::new("cancel-markers");
    let mut session = new_session(tmp.path());
    let a = noise(1, L);
    floor(&mut session, &a);
    let capture = open_punch(&mut session, &noise(2, 30_000));
    let id = capture.id();
    for (mid, pos) in [(100, S as u64 - 48_000), (101, S as u64 - 24_000)] {
        session
            .note_take_marker(id, Marker::new(MarkerId(mid), pos, 0, "pre"))
            .unwrap();
    }
    let finished = capture.finish();
    session.cancel_take(finished.take).unwrap();
    assert!(!session.is_recording());
    assert_eq!(session.history().undo_depth(), 2, "one entry per marker");
    assert_eq!(session.history().undo_label(), Some(MARKER_ADD_LABEL_KEY));
    let mut want = fixture_positions();
    want.extend([(100, S as u64 - 48_000, 0), (101, S as u64 - 24_000, 0)]);
    assert_eq!(
        sorted_positions(positions(&session)),
        sorted_positions(want)
    );
    assert_eq!(
        fnv1a_hash(&read_all(session.store(), &session.current())),
        fnv1a_hash(&a)
    );
    let journal = read_journal(session.journal_path()).unwrap();
    assert!(
        journal
            .records
            .iter()
            .any(|r| matches!(r, Record::TakeCancel { .. }))
    );
    session.undo().unwrap().unwrap();
    session.undo().unwrap().unwrap();
    assert_eq!(
        sorted_positions(positions(&session)),
        sorted_positions(fixture_positions())
    );
}

/// H-21 / AC-10 + AC-15 (project part): markers journaled during an interrupted punch are
/// restored by "Apply as recorded" where a live commit would put them — a pre-roll marker stays at
/// its heard position before `S`, a window marker past the recovered end is clamped to it.
#[test]
fn ac10_recovered_punch_places_its_markers_like_a_live_commit() {
    let tmp = TempDir::new("punch-recover-markers");
    let a = noise(1, L);
    let (k, recovered) = (240_000usize, 70_000usize);
    let dir = {
        let mut session = new_session(tmp.path());
        floor(&mut session, &a);
        let mut capture = open_punch(&mut session, &noise(9, k + recovered));
        let id = capture.id();
        capture.sync().unwrap();
        session.note_take_window(id, k as u64).unwrap();
        for (mid, pos) in [
            (100, S as u64 - 12_000),
            (101, S as u64 + 10_000),
            (102, S as u64 + 100_000),
        ] {
            session
                .note_take_marker(id, Marker::new(MarkerId(mid), pos, 0, "m"))
                .unwrap();
        }
        let dir = session.dir().to_path_buf();
        drop(capture); // crash
        drop(session);
        dir
    };
    let (mut session, _) =
        Session::recover(&dir, StoreOptions::with_memory_budget(256 * MIB)).unwrap();
    session.apply_open_take_from_wav().unwrap().unwrap();
    let mut want = fixture_positions();
    want.extend([
        (100, S as u64 - 12_000, 0),
        (101, S as u64 + 10_000, 0),
        (102, (S + recovered) as u64, 0),
    ]);
    assert_eq!(
        sorted_positions(positions(&session)),
        sorted_positions(want)
    );
}

/// H-21 / SPEC-022 AC-11: a seeded sequence of 20 mixed record operations — Insert, Overwrite
/// inside, Overwrite past the end, full punch, partial punch, every other one with a marker added
/// during it — undone completely and then redone completely, matches the hash, length and marker
/// list of every step (the start and end states in particular).
#[test]
fn ac11_seeded_sequence_of_20_operations_undoes_and_redoes_exactly() {
    let tmp = TempDir::new("ac11-sequence");
    let mut session = new_session(tmp.path());
    floor(&mut session, &noise(1, 240_000));
    let fingerprint = |s: &Session| {
        let doc = read_all(s.store(), &s.current());
        (fnv1a_hash(&doc), doc.len(), sorted_positions(positions(s)))
    };
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: u64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng % n.max(1)
    };
    let mut states = vec![fingerprint(&session)];
    let mut kinds = [0usize; 5];
    for i in 0..20u64 {
        let len = session.current().len_samples;
        let n = 4_000 + next(20_000);
        let w = noise(100 + i, n as usize);
        let kind = next(5) as usize;
        kinds[kind] += 1;
        let (mode, aligned, window, at) = match kind {
            0 => {
                let c = next(len + 1);
                (TakeMode::Insert { at_samples: c }, false, (0, n), c)
            }
            1 => {
                let c = next(len - n);
                (TakeMode::Overwrite { at_samples: c }, false, (0, n), c)
            }
            2 => {
                let c = len - next(n / 2);
                (TakeMode::Overwrite { at_samples: c }, false, (0, n), c)
            }
            _ => {
                let span = n.min(len / 2);
                let s = next(len - span);
                let p = if kind == 3 {
                    span
                } else {
                    span / 2 + next(span / 2)
                };
                // The take holds a 1 000-sample pre-roll before the window.
                let mode = TakeMode::Punch {
                    start_samples: s,
                    end_samples: s + span,
                };
                (mode, true, (1_000, 1_000 + p), s)
            }
        };
        let samples = if aligned {
            let mut v = noise(500 + i, 1_000);
            v.extend_from_slice(&w);
            v
        } else {
            w
        };
        let finished = take(&mut session, mode, X, aligned, &samples);
        if i % 2 == 0 {
            let marker = Marker::new(MarkerId(1_000 + i), at + next(n / 2), 0, "during");
            session.note_take_marker(finished.take, marker).unwrap();
        }
        session
            .commit_take_window(&finished, window, &[])
            .unwrap()
            .expect("one edit");
        states.push(fingerprint(&session));
    }
    assert!(kinds.iter().all(|&k| k > 0), "every kind occurs: {kinds:?}");
    assert_eq!(session.history().undo_depth(), 20);
    for i in (0..20).rev() {
        session.undo().unwrap().unwrap();
        assert_eq!(
            fingerprint(&session),
            states[i],
            "after undoing op {}",
            i + 1
        );
    }
    for (i, state) in states.iter().enumerate().skip(1) {
        session.redo().unwrap().unwrap();
        assert_eq!(&fingerprint(&session), state, "after redoing op {i}");
    }
}
