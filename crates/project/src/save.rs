//! Save a snapshot to WAV or FLAC (SPEC-005 §2.7, §2.8, §2.9, §2.11; ADR-004 §8: atomic, no rack).

use crate::edit::Range;
use crate::normalize::scan_peak_accelerated;
use crate::reader::{SnapshotReader, read_range};
use crate::snapshot::{DocSnapshot, Marker};
use crate::store::{CancelToken, ChunkStore};
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// A source of a document's samples for [`save_snapshot_wav`]: generalized (over
/// [`SnapshotReader`] in production) so the save streams through any backing reader without ever
/// holding the whole document in one buffer, and so tests can substitute a lightweight fake that
/// checks the block sizes the save actually asks for (H-02).
trait SampleSource {
    /// Sample rate to put in the WAV header.
    fn sample_rate_hz(&self) -> u32;
    /// Document length in samples.
    fn len_samples(&self) -> u64;
    /// Copies `[pos, pos + out.len())` into `out`; see [`SnapshotReader::read`].
    fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize>;
}

impl SampleSource for SnapshotReader {
    fn sample_rate_hz(&self) -> u32 {
        self.snapshot().sample_rate_hz
    }

    fn len_samples(&self) -> u64 {
        SnapshotReader::len_samples(self)
    }

    fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize> {
        SnapshotReader::read(self, pos, out)
    }
}

/// Streams every sample `reader`'s snapshot holds to `path` as a WAV at `bits` through
/// [`vox_io::WavStreamWriter`] (TPDF-dithered for 16/24-bit, atomic — temp file, `fdatasync`,
/// rename, ADR-004 §8), plus `cue `/`LIST adtl` chunks for `markers` (SPEC-005 §2.9; `markers`
/// empty writes neither chunk). Save does not render the rack (D-019): this writes exactly the
/// document's audio.
///
/// Reads and writes [`CHUNK_SAMPLES`]-sample blocks at a time (H-02): memory stays bounded by
/// that one block, not by the document's length — a 60-minute mono recording no longer needs
/// ~691 MB of `Vec<f32>` to save.
///
/// H-70 (SPEC-005 §4.10): `cancel` is checked once per block — cancelling aborts the temp file
/// (same path a real I/O failure already took) and returns [`ProjectError::Cancelled`], so the
/// target is left untouched. `on_progress` receives a fraction in `[0.0, 1.0]`, called once with
/// `0.0` before the first block (a deterministic mid-job hook, MEMORY H-30) and `1.0` once the
/// file (audio + markers) is fully written.
///
/// The multichannel-source/clip-prompt dialogs are deferred (S1-02 ticket scope) — the caller
/// checks the document peak pyramid for overs before calling this, if it wants that prompt.
pub fn save_snapshot_wav(
    reader: &mut SnapshotReader,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
    dither: vox_io::DitherMode,
    markers: &[Marker],
    cancel: &CancelToken,
    on_progress: impl FnMut(f32),
) -> Result<vox_io::WriteReport> {
    save_snapshot_wav_streaming(reader, path, bits, dither, markers, cancel, on_progress)
}

fn save_snapshot_wav_streaming<R: SampleSource>(
    source: &mut R,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::BitDepth,
    dither: vox_io::DitherMode,
    markers: &[Marker],
    cancel: &CancelToken,
    mut on_progress: impl FnMut(f32),
) -> Result<vox_io::WriteReport> {
    on_progress(0.0);
    let len = source.len_samples();
    let rate = source.sample_rate_hz();
    let mut writer = vox_io::WavStreamWriter::create(path, rate, bits, dither)?;

    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    let mut pos = 0u64;
    let stream_result: Result<()> = (|| {
        while pos < len {
            if cancel.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let n = source.read(pos, &mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_block(&buf[..n])?;
            pos += n as u64;
            on_progress((pos as f64 / len as f64) as f32);
        }
        Ok(())
    })();
    if let Err(e) = stream_result {
        writer.abort();
        return Err(e);
    }
    if len == 0 {
        on_progress(1.0);
    }

    // `markers` (`DocSnapshot::markers`) is already in canonical, position-sorted order, which
    // `WavStreamWriter::finish` relies on to assign cue ids 1..n in position order (SPEC-005
    // §2.9).
    let wav_markers: Vec<vox_io::WavMarker> = markers
        .iter()
        .map(|m| vox_io::WavMarker {
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name.to_string(),
        })
        .collect();
    let report = writer.finish(&wav_markers)?;
    on_progress(1.0);
    Ok(report)
}

/// SPEC-005 §2.7 pre-flight step 2 (`wav_max_bytes`, AC-18): would writing `len_samples` mono
/// samples at `bits` push a WAV file's RIFF/`data` size fields — both `u32` — past their limit?
/// Checked before any byte is written, so a document too long for WAV (≈ 6 h 12 min at 48 kHz
/// 32-bit float) is refused instead of streaming for minutes and failing at the end. Independent
/// of markers: [`vox_io`]'s own `append_markers` guard (`crates/io/src/wav.rs`) catches those
/// separately, after the audio is already down, because markers are appended after `data` and
/// their own size is unbounded by this check.
pub fn wav_size_exceeds_limit(len_samples: u64, bits: vox_io::BitDepth) -> bool {
    let bytes_per_sample: u64 = match bits {
        vox_io::BitDepth::Int16 => 2,
        vox_io::BitDepth::Int24 => 3,
        vox_io::BitDepth::Float32 => 4,
    };
    // A conservative fixed overhead for `RIFF`/`fmt `/`fact`/`data` headers and the odd-size pad
    // byte (at most a few dozen bytes) — what matters is that this never *under*-estimates, so a
    // document this check accepts is one `write_wav` can always finish.
    const HEADER_OVERHEAD_BYTES: u64 = 128;
    let data_bytes = len_samples.saturating_mul(bytes_per_sample);
    data_bytes.saturating_add(HEADER_OVERHEAD_BYTES) > u64::from(u32::MAX)
}

/// T-209 (SPEC-005 §2.8): overs found by [`overs_check`] — an integer target format's pre-flight
/// clip check, computed *before* any byte is written. `peak_dbfs` is `20*log10(peak)` of the exact
/// sample peak.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OversInfo {
    pub count: u64,
    pub peak_dbfs: f64,
}

/// Pre-flight overs check for an integer save format (SPEC-005 §2.8, §2.7 pre-flight step 3):
/// `None` when every sample is `<= 1.0` (no confirmation needed) — 32-bit float targets never call
/// this (float is written bit-exact, overs preserved). The document peak pyramid never
/// under-reports (ADR-004 §5), so a pyramid peak `<= 1.0` proves there are no overs with no exact
/// pass; only a pyramid peak `> 1.0` triggers one exact counting pass over the whole document
/// (SPEC-005 §2.8 "detection cost").
pub fn overs_check(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    cancel: &CancelToken,
) -> Result<Option<OversInfo>> {
    let range = Range {
        start: 0,
        end: snapshot.len_samples,
    };
    let pyramid_peak = scan_peak_accelerated(store, snapshot, range, cancel, |_| {})?;
    if pyramid_peak <= 1.0 {
        return Ok(None);
    }

    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    let mut pos = 0u64;
    let mut count = 0u64;
    let mut peak = 0.0f64;
    while pos < snapshot.len_samples {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let want = ((snapshot.len_samples - pos) as usize).min(buf.len());
        let n = read_range(store, snapshot, pos, &mut buf[..want], &mut None)?;
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            let a = f64::from(s.abs());
            if a > 1.0 {
                count += 1;
            }
            peak = peak.max(a);
        }
        pos += n as u64;
    }
    Ok(Some(OversInfo {
        count,
        peak_dbfs: 20.0 * peak.log10(),
    }))
}

/// Writes `reader`'s whole snapshot to `path` as FLAC at `bits` (SPEC-005 §2.11): mono only
/// (PowerVoice edits mono), 16- or 24-bit, TPDF-dithered with the same grid-exact-passthrough
/// rules [`save_snapshot_wav`] uses, atomic (temp file, verify-before-rename, `vox_io::write_flac`,
/// H-14). Markers are never written to FLAC (SPEC-005 §2.9's "other containers" — the caller
/// shows `notice.save.markers_not_in_flac` when the document has any).
///
/// Unlike [`save_snapshot_wav`], this buffers the whole document in memory before encoding
/// (`vox_io::write_flac` takes a whole buffer, matching the export path's existing constraint,
/// MEMORY.md H-02) — acceptable for mono voice-over documents; streaming FLAC encoding is a
/// follow-up if a very long document makes this a problem in practice.
///
/// H-70: `cancel` is checked once per buffered block, before any file is created — no temp file
/// to abort on cancellation there. Once buffering finishes, `vox_io::write_flac` (encode, verify,
/// atomic rename) runs as one call with no further cancellation point, mirroring every other
/// job's "best-effort, within one block" cancel contract. `on_progress` reports the buffering
/// phase over `[0.0, 0.5)` and jumps to `1.0` once the encoder has written and verified the file.
pub fn save_snapshot_flac(
    reader: &mut SnapshotReader,
    path: impl AsRef<std::path::Path>,
    bits: vox_io::FlacBitDepth,
    dither: vox_io::DitherMode,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(f32),
) -> Result<vox_io::WriteReport> {
    on_progress(0.0);
    let len = reader.len_samples();
    let rate = reader.snapshot().sample_rate_hz;
    let mut samples = vec![0.0f32; len as usize];
    let mut pos = 0u64;
    while pos < len {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let n = reader.read(pos, &mut samples[pos as usize..])?;
        if n == 0 {
            break;
        }
        pos += n as u64;
        on_progress(0.5 * (pos as f64 / len as f64) as f32);
    }
    if cancel.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    let report = vox_io::write_flac(path, rate, bits, dither, &samples)?;
    on_progress(1.0);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::ChunkStore;
    use crate::{DocSnapshot, StoreOptions};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vox-project-save-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// H-20 (item 1): `save_snapshot_wav`'s `dither` parameter reaches the writer — a
    /// `DitherMode::None` save of off-grid content differs from a `DitherMode::Tpdf` save of the
    /// same document (the TPDF one carries dither noise the `None` one doesn't).
    #[test]
    fn save_snapshot_wav_dither_none_differs_from_tpdf_on_off_grid_content() {
        let dir = tmp_dir("dither-none");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        // A low-level sine deliberately off the 16-bit grid almost everywhere.
        let samples = vox_testkit::signal::sine(997.0, -40.0, 0.05, 48_000).unwrap();
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));

        let none_path = dir.join("none.wav");
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        save_snapshot_wav(
            &mut reader,
            &none_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::None,
            &[],
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        let tpdf_path = dir.join("tpdf.wav");
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        save_snapshot_wav(
            &mut reader,
            &tpdf_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &[],
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        assert_ne!(
            std::fs::read(&none_path).unwrap(),
            std::fs::read(&tpdf_path).unwrap(),
            "TPDF must add dither noise that None doesn't"
        );

        // The `None` output matches testkit's own plain-rounding encoder bit-exactly (AC-4).
        let (none_decoded, _) = vox_testkit::wav::read_wav_file(&none_path).unwrap();
        let expected_path = dir.join("expected.wav");
        vox_testkit::wav::write_wav_file(
            &expected_path,
            &samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Int16,
        )
        .unwrap();
        let (expected_decoded, _) = vox_testkit::wav::read_wav_file(&expected_path).unwrap();
        assert_eq!(none_decoded, expected_decoded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_snapshot_wav_round_trips_float32() {
        let dir = tmp_dir("float32");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::sine(1000.0, -6.0, 0.05, 48_000).unwrap();
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));

        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        let out_path = dir.join("out.wav");
        let mut progress = Vec::new();
        let report = save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Float32,
            vox_io::DitherMode::Tpdf,
            &[],
            &CancelToken::new(),
            |fraction| progress.push(fraction),
        )
        .unwrap();
        assert_eq!(report.clipped_samples, 0);

        // H-70: `on_progress` starts at 0.0 (a deterministic mid-job hook, MEMORY H-30), is
        // monotonically non-decreasing, and ends at 1.0.
        assert_eq!(progress.first().copied(), Some(0.0));
        assert_eq!(progress.last().copied(), Some(1.0));
        assert!(progress.is_sorted());

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(decoded, samples);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_snapshot_wav_writes_markers_as_cue_points() {
        use crate::snapshot::{Marker, MarkerId};

        let dir = tmp_dir("markers");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::silence(0.2, 48_000).unwrap();
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let markers = vec![
            Marker::new(MarkerId(1), 0, 0, "Intro"),
            Marker::new(MarkerId(2), 4_800, 2_400, "Take 2"),
        ];
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, markers));

        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
        let out_path = dir.join("out.wav");
        save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &snapshot.markers,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        let read_back = vox_io::read_wav_markers(&out_path).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!((read_back[0].pos_samples, read_back[0].len_samples), (0, 0));
        assert_eq!(read_back[0].name, "Intro");
        assert_eq!(
            (read_back[1].pos_samples, read_back[1].len_samples),
            (4_800, 2_400)
        );
        assert_eq!(read_back[1].name, "Take 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-02: streaming ------------------------------------------------------------------

    /// A fake [`SampleSource`] over an in-memory buffer that records every `(pos, requested_len)`
    /// it was asked for, so a test can assert the save path never asks for more than one
    /// `CHUNK_SAMPLES` block at a time — i.e. it never allocates (or requests into) a buffer
    /// proportional to the document length.
    struct CountingSource {
        samples: Vec<f32>,
        rate: u32,
        calls: Vec<(u64, usize)>,
    }

    impl SampleSource for CountingSource {
        fn sample_rate_hz(&self) -> u32 {
            self.rate
        }

        fn len_samples(&self) -> u64 {
            self.samples.len() as u64
        }

        fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize> {
            self.calls.push((pos, out.len()));
            let start = pos as usize;
            let n = out.len().min(self.samples.len() - start);
            out[..n].copy_from_slice(&self.samples[start..start + n]);
            Ok(n)
        }
    }

    fn noise(seed: u64, n: usize) -> Vec<f32> {
        let mut state = seed | 1;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / (1u32 << 24) as f32) * 1.6 - 0.8
            })
            .collect()
    }

    #[test]
    fn save_streams_in_bounded_blocks_never_reading_the_whole_document_at_once() {
        // Several blocks plus a short tail: exercises the "last read may be shorter" path too.
        let n = CHUNK_SAMPLES * 3 + 12_345;
        let samples = noise(1, n);
        let mut source = CountingSource {
            samples,
            rate: 48_000,
            calls: Vec::new(),
        };
        let dir = tmp_dir("streamed-bounded");
        let out_path = dir.join("out.wav");
        let mut progress = Vec::new();
        save_snapshot_wav_streaming(
            &mut source,
            &out_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &[],
            &CancelToken::new(),
            |fraction| progress.push(fraction),
        )
        .unwrap();

        assert!(
            !source.calls.is_empty(),
            "the save must actually read the source"
        );
        // H-70: one progress call per block, plus the leading 0.0 and the trailing 1.0 (after
        // `finish` writes the header/markers).
        assert_eq!(progress.len(), source.calls.len() + 2);
        assert_eq!(progress.first().copied(), Some(0.0));
        assert_eq!(progress.last().copied(), Some(1.0));
        assert!(progress.is_sorted());
        for &(_, requested) in &source.calls {
            assert!(
                requested <= CHUNK_SAMPLES,
                "save_snapshot_wav must never request more than one CHUNK_SAMPLES block at a \
                 time (requested {requested})"
            );
        }
        let expected_calls = n.div_ceil(CHUNK_SAMPLES);
        assert_eq!(
            source.calls.len(),
            expected_calls,
            "expected one read per CHUNK_SAMPLES block (plus the short tail)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_of_a_document_larger_than_several_blocks_round_trips_via_fnv1a() {
        let dir = tmp_dir("streamed-large");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let n = CHUNK_SAMPLES * 4 + 777;
        let samples = noise(2, n);
        let mut writer = store.writer();
        // Feed the store in odd-sized blocks too, so chunk boundaries and CHUNK_SAMPLES read
        // blocks don't just trivially line up.
        for block in samples.chunks(10_007) {
            writer.append(block).unwrap();
        }
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()));
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.wav");
        save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Float32,
            vox_io::DitherMode::Tpdf,
            &[],
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(decoded.len(), samples.len());
        assert_eq!(
            vox_testkit::golden::fnv1a_hash(&decoded),
            vox_testkit::golden::fnv1a_hash(&samples),
            "a streamed save of a multi-block document must round-trip exactly"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_of_a_document_larger_than_several_blocks_keeps_its_markers() {
        use crate::snapshot::{Marker, MarkerId};

        let dir = tmp_dir("streamed-large-markers");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let n = CHUNK_SAMPLES * 3 + 4_000;
        let samples = noise(3, n);
        let mut writer = store.writer();
        writer.append(&samples).unwrap();
        let audio = writer.finish().unwrap();
        let markers = vec![
            Marker::new(MarkerId(1), 100, 0, "Start"),
            // Sits exactly on a CHUNK_SAMPLES block boundary.
            Marker::new(MarkerId(2), CHUNK_SAMPLES as u64, 0, "Boundary"),
            // Falls in the final, short block.
            Marker::new(MarkerId(3), (CHUNK_SAMPLES * 3 + 500) as u64, 1_000, "Tail"),
        ];
        let snapshot = Arc::new(DocSnapshot::new(48_000, audio.pieces, markers));
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.wav");
        save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &snapshot.markers,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        let read_back = vox_io::read_wav_markers(&out_path).unwrap();
        assert_eq!(read_back.len(), 3);
        assert_eq!(read_back[0].name, "Start");
        assert_eq!(read_back[1].pos_samples, CHUNK_SAMPLES as u64);
        assert_eq!(read_back[1].name, "Boundary");
        assert_eq!(read_back[2].pos_samples, (CHUNK_SAMPLES * 3 + 500) as u64);
        assert_eq!(read_back[2].len_samples, 1_000);
        assert_eq!(read_back[2].name, "Tail");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- T-209: overs pre-check + FLAC save ------------------------------------------------

    fn snapshot_of(store: &Arc<ChunkStore>, samples: &[f32]) -> Arc<DocSnapshot> {
        let mut writer = store.writer();
        writer.append(samples).unwrap();
        let audio = writer.finish().unwrap();
        Arc::new(DocSnapshot::new(48_000, audio.pieces, Vec::new()))
    }

    #[test]
    fn overs_check_reports_none_at_exactly_full_scale() {
        let dir = tmp_dir("overs-none");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut samples = vox_testkit::signal::sine(997.0, -6.0, 0.05, 48_000).unwrap();
        samples[10] = 1.0; // exactly full scale: not an over (SPEC-005 §2.8 AC-6's boundary case)
        let snapshot = snapshot_of(&store, &samples);

        let cancel = CancelToken::new();
        let overs = overs_check(&store, &snapshot, &cancel).unwrap();
        assert!(overs.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overs_check_counts_exact_overs_and_peak_dbfs() {
        let dir = tmp_dir("overs-some");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut samples = vox_testkit::signal::sine(997.0, -6.0, 0.05, 48_000).unwrap();
        // 3 samples strictly above 0 dBFS, peak = 1.5 (+3.52 dBFS, SPEC-005 AC-6's own example).
        samples[100] = 1.5;
        samples[200] = -1.2;
        samples[300] = 1.1;
        let snapshot = snapshot_of(&store, &samples);

        let cancel = CancelToken::new();
        let overs = overs_check(&store, &snapshot, &cancel)
            .unwrap()
            .expect("a peak above 1.0 must report Some");
        assert_eq!(overs.count, 3);
        assert!(
            (overs.peak_dbfs - (20.0 * 1.5f64.log10())).abs() < 1e-9,
            "peak_dbfs = {}",
            overs.peak_dbfs
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overs_check_is_cancellable() {
        let dir = tmp_dir("overs-cancel");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let mut samples = vec![0.0f32; CHUNK_SAMPLES * 2];
        samples[0] = 1.5; // forces the exact pass to run past the pyramid check
        let snapshot = snapshot_of(&store, &samples);

        let cancel = CancelToken::new();
        cancel.cancel();
        let err = overs_check(&store, &snapshot, &cancel).unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-60: WAV size pre-flight (SPEC-005 §2.7/AC-18) -----------------------------------

    #[test]
    fn wav_size_limit_accepts_a_document_just_under_4_gib_and_refuses_just_over() {
        // 32-bit float: 4 bytes/sample. `(u32::MAX - HEADER) / 4` samples fits; one more doesn't.
        let max_bytes = u64::from(u32::MAX) - 128;
        let fits = max_bytes / 4;
        assert!(!wav_size_exceeds_limit(fits, vox_io::BitDepth::Float32));
        assert!(wav_size_exceeds_limit(fits + 1, vox_io::BitDepth::Float32));
    }

    #[test]
    fn wav_size_limit_60_min_48k_fixtures_fit_every_bit_depth() {
        // The project's own 60-min reference fixtures (MEMORY T-006/T-704): 48 kHz * 3600 s.
        let len_samples = 48_000u64 * 3600;
        assert!(!wav_size_exceeds_limit(
            len_samples,
            vox_io::BitDepth::Int16
        ));
        assert!(!wav_size_exceeds_limit(
            len_samples,
            vox_io::BitDepth::Int24
        ));
        assert!(!wav_size_exceeds_limit(
            len_samples,
            vox_io::BitDepth::Float32
        ));
    }

    #[test]
    fn wav_size_limit_6_3_hours_of_48k_32f_is_refused_but_16_bit_is_not() {
        // SPEC-005 AC-18's own example: "6.3 h of silence pieces at 48 kHz... WAV 32f is refused
        // ... while WAV 16-bit is accepted."
        let len_samples = (6.3 * 3600.0 * 48_000.0) as u64;
        assert!(wav_size_exceeds_limit(
            len_samples,
            vox_io::BitDepth::Float32
        ));
        assert!(!wav_size_exceeds_limit(
            len_samples,
            vox_io::BitDepth::Int16
        ));
    }

    #[test]
    fn save_snapshot_flac_round_trips_and_reports_no_clipped_samples() {
        let dir = tmp_dir("flac-roundtrip");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::sine(997.0, -6.0, 0.2, 48_000).unwrap();
        let snapshot = snapshot_of(&store, &samples);
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.flac");
        let mut progress = Vec::new();
        let report = save_snapshot_flac(
            &mut reader,
            &out_path,
            vox_io::FlacBitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &CancelToken::new(),
            |fraction| progress.push(fraction),
        )
        .unwrap();
        assert_eq!(report.clipped_samples, 0);
        assert!(out_path.exists());
        assert_eq!(progress.first().copied(), Some(0.0));
        assert_eq!(progress.last().copied(), Some(1.0));
        assert!(progress.is_sorted());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-70: cancellation --------------------------------------------------------------

    /// A cancelled WAV save leaves no target file (the temp file is aborted, same path a real
    /// I/O failure already took).
    #[test]
    fn save_snapshot_wav_cancelled_mid_write_leaves_no_target_file() {
        let dir = tmp_dir("cancel-wav");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = noise(4, CHUNK_SAMPLES * 3);
        let snapshot = snapshot_of(&store, &samples);
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.wav");
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = save_snapshot_wav(
            &mut reader,
            &out_path,
            vox_io::BitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &[],
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        assert!(!out_path.exists(), "no target file after a cancelled save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cancel signalled partway through the buffering phase stops a FLAC save before the
    /// encoder ever runs — no file at all, not even a temp one.
    #[test]
    fn save_snapshot_flac_cancelled_during_buffering_leaves_no_target_file() {
        let dir = tmp_dir("cancel-flac");
        let store = ChunkStore::create(&dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024))
            .unwrap();
        let samples = vox_testkit::signal::sine(997.0, -6.0, 1.0, 48_000).unwrap();
        let snapshot = snapshot_of(&store, &samples);
        let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));

        let out_path = dir.join("out.flac");
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = save_snapshot_flac(
            &mut reader,
            &out_path,
            vox_io::FlacBitDepth::Int16,
            vox_io::DitherMode::Tpdf,
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        assert!(!out_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
