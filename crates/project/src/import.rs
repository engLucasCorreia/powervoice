//! Import a WAV file into a session as the undo floor (ADR-004 §8, MEMORY.md T-101: use
//! [`Session::set_floor`]).

use std::sync::Arc;

use crate::session::Session;
use crate::snapshot::DocSnapshot;
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// Streams `source` through a [`crate::store::ChunkWriter`] and makes the result the session's
/// undo floor (SPEC-004 §2.1, ADR-004 §8): the document opens clean, at the imported audio.
///
/// `source`'s sample rate must match `session`'s (the document rate is the source rate — the
/// caller creates the session with [`crate::SessionConfig::new`]`(rate)` from the same file,
/// ADR-004 §8). `source` already downmixes multichannel input to mono by averaging
/// ([`vox_io::WavSource::read_mono`], SPEC-005 §2.4 default); picking a channel instead, cue
/// markers, and progress events are deferred (S1-02 ticket scope).
pub fn import_wav(
    session: &mut Session,
    source: &mut vox_io::WavSource,
) -> Result<Arc<DocSnapshot>> {
    if source.sample_rate_hz() != session.sample_rate_hz() {
        return Err(ProjectError::InvalidArgument(
            "import_wav: the source's sample rate must match the session's",
        ));
    }
    let mut writer = session.chunk_writer();
    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    loop {
        let n = source.read_mono(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.append(&buf[..n])?;
    }
    let audio = writer.finish()?;
    session.set_floor(&audio, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionConfig, StoreOptions};

    fn write_ref_wav(path: &std::path::Path, samples: &[f32], channels: u16, rate: u32) {
        vox_testkit::wav::write_wav_file(
            path,
            samples,
            channels,
            rate,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
    }

    #[test]
    fn import_mono_wav_becomes_the_undo_floor() {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-import-{}-{}",
            std::process::id(),
            "mono"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        write_ref_wav(&wav_path, &samples, 1, 48_000);

        let (rate, channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        assert_eq!(channels, 1);

        let mut config = SessionConfig::new(rate);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();

        let snapshot = import_wav(&mut session, &mut source).unwrap();
        assert_eq!(snapshot.len_samples, samples.len() as u64);
        assert!(!session.is_dirty(), "import is not an undoable edit");
        assert_eq!(
            session.history().undo_depth(),
            0,
            "no undo entry for import"
        );

        let mut out = vec![0.0f32; samples.len()];
        session.store().read(&snapshot, 0, &mut out).unwrap();
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_a_sample_rate_mismatch() {
        let dir = std::env::temp_dir().join(format!(
            "vox-project-import-{}-{}",
            std::process::id(),
            "ratemismatch"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::silence(0.05, 44_100).unwrap();
        write_ref_wav(&wav_path, &samples, 1, 44_100);
        let (_, _, mut source) = vox_io::read_wav(&wav_path).unwrap();

        let mut config = SessionConfig::new(48_000);
        config.store = StoreOptions::with_memory_budget(64 * 1024 * 1024);
        let mut session = Session::create(&dir, config).unwrap();
        assert!(matches!(
            import_wav(&mut session, &mut source),
            Err(ProjectError::InvalidArgument(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
