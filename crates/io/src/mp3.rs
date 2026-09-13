//! MP3 export via runtime-loaded `libmp3lame` (ADR-007 §4, D-013): LGPL-2.0-or-later code is never
//! linked or vendored into this binary — only `libloading`'s dlopen/dlsym at runtime, against our
//! own minimal FFI table for the public LAME API. If no `libmp3lame` can be found, [`mp3_available`]
//! returns `false` and [`encode_mp3`] fails with [`IoError::Mp3Unavailable`] instead of the app
//! crashing or refusing to start.

use std::ffi::{c_int, c_void};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::OnceLock;

use libloading::{Library, Symbol};

use crate::atomic::{finish, temp_path_for};
use crate::error::{IoError, Result};
use crate::wav::WriteReport;

/// Opaque `lame_global_flags*` (ADR-007 §4): only pointers to it cross the FFI boundary, so its
/// concrete layout never matters to us.
type Gfp = c_void;

/// `MPEG_mode::MONO` (lame.h).
const MPEG_MODE_MONO: c_int = 3;
/// `vbr_mode::vbr_off` (lame.h): constant bitrate.
const VBR_OFF: c_int = 0;
/// `vbr_mode::vbr_mtrh` (lame.h), LAME's own recommended/default VBR algorithm.
const VBR_MTRH: c_int = 4;

#[cfg(target_os = "linux")]
const LIB_CANDIDATES: &[&str] = &["libmp3lame.so.0", "libmp3lame.so"];
#[cfg(target_os = "macos")]
const LIB_CANDIDATES: &[&str] = &["libmp3lame.0.dylib", "libmp3lame.dylib"];
#[cfg(target_os = "windows")]
const LIB_CANDIDATES: &[&str] = &["libmp3lame.dll", "lame_enc.dll"];
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const LIB_CANDIDATES: &[&str] = &["libmp3lame.so"];

/// The resolved LAME FFI table (ADR-007 §4's list): symbols are looked up once at load time and
/// kept as plain function pointers alongside the `Library` that must outlive them.
struct Lame {
    _lib: Library,
    lame_init: unsafe extern "C" fn() -> *mut Gfp,
    lame_set_in_samplerate: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_num_channels: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_mode: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_brate: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_vbr: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_vbr_q: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_set_quality: unsafe extern "C" fn(*mut Gfp, c_int) -> c_int,
    lame_init_params: unsafe extern "C" fn(*mut Gfp) -> c_int,
    lame_encode_buffer_ieee_float:
        unsafe extern "C" fn(*mut Gfp, *const f32, *const f32, c_int, *mut u8, c_int) -> c_int,
    lame_encode_flush: unsafe extern "C" fn(*mut Gfp, *mut u8, c_int) -> c_int,
    lame_get_lametag_frame: unsafe extern "C" fn(*const Gfp, *mut u8, usize) -> usize,
    lame_close: unsafe extern "C" fn(*mut Gfp) -> c_int,
}

/// Looks up one symbol in `lib`, copying the function pointer out (fn pointers are `Copy`, so the
/// borrowed `Symbol` doesn't need to outlive this call).
///
/// # Safety
/// `T` must exactly match the C signature of the symbol named `name` in `libmp3lame`'s public API
/// (lame.h), and `name` must be a NUL-terminated byte string.
unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> std::result::Result<T, String> {
    // SAFETY: forwarded to the caller's contract above.
    unsafe {
        lib.get::<T>(name)
            .map(|s: Symbol<'_, T>| *s)
            .map_err(|e| e.to_string())
    }
}

impl Lame {
    fn load() -> std::result::Result<Self, String> {
        let mut last_err = String::from("no candidate library name for this platform");
        for name in LIB_CANDIDATES {
            match Self::load_named(name) {
                Ok(lame) => return Ok(lame),
                Err(e) => last_err = format!("{name}: {e}"),
            }
        }
        Err(last_err)
    }

    fn load_named(name: &str) -> std::result::Result<Self, String> {
        // SAFETY: dlopen-ing a named system shared library. `libmp3lame` exposes a stable, widely
        // documented C API (lame.h); a mismatched or malicious library at this name could violate
        // memory safety, the same risk as any dlopen-based integration (ADR-007 §4).
        let lib = unsafe { Library::new(name) }.map_err(|e| e.to_string())?;
        // SAFETY: each function pointer type below matches lame.h's declared signature for that
        // symbol name (ADR-007 §4's FFI table); `_lib` keeps the library (and so these symbols)
        // alive for the lifetime of `Lame`.
        unsafe {
            Ok(Lame {
                lame_init: sym(&lib, b"lame_init\0")?,
                lame_set_in_samplerate: sym(&lib, b"lame_set_in_samplerate\0")?,
                lame_set_num_channels: sym(&lib, b"lame_set_num_channels\0")?,
                lame_set_mode: sym(&lib, b"lame_set_mode\0")?,
                lame_set_brate: sym(&lib, b"lame_set_brate\0")?,
                lame_set_vbr: sym(&lib, b"lame_set_VBR\0")?,
                lame_set_vbr_q: sym(&lib, b"lame_set_VBR_q\0")?,
                lame_set_quality: sym(&lib, b"lame_set_quality\0")?,
                lame_init_params: sym(&lib, b"lame_init_params\0")?,
                lame_encode_buffer_ieee_float: sym(&lib, b"lame_encode_buffer_ieee_float\0")?,
                lame_encode_flush: sym(&lib, b"lame_encode_flush\0")?,
                lame_get_lametag_frame: sym(&lib, b"lame_get_lametag_frame\0")?,
                lame_close: sym(&lib, b"lame_close\0")?,
                _lib: lib,
            })
        }
    }
}

/// Closes the LAME handle on every exit path (including an early `?` return), since the C API has
/// no destructor of its own.
struct LameSession<'a> {
    lame: &'a Lame,
    gfp: *mut Gfp,
}

impl Drop for LameSession<'_> {
    fn drop(&mut self) {
        if !self.gfp.is_null() {
            // SAFETY: `gfp` was returned by `lame_init` in `LameSession::open` and is closed
            // exactly once, here.
            unsafe { (self.lame.lame_close)(self.gfp) };
        }
    }
}

impl<'a> LameSession<'a> {
    fn open(lame: &'a Lame) -> Result<Self> {
        // SAFETY: `lame_init` takes no arguments and either returns a valid handle or null.
        let gfp = unsafe { (lame.lame_init)() };
        if gfp.is_null() {
            return Err(IoError::Mp3Encode("lame_init returned null".into()));
        }
        Ok(LameSession { lame, gfp })
    }
}

/// `true` if a `libmp3lame` shared library can be loaded on this system (ADR-007 §4). Checked once
/// per process and cached: the library doesn't appear or disappear while the app is running.
pub fn mp3_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| Lame::load().is_ok())
}

/// CBR/VBR settings for [`encode_mp3`] (PROMPT §3.5; ticket S4-02: CBR 128-320 kbps, VBR V0-V4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp3Settings {
    /// Constant bitrate in kbps, 128-320.
    Cbr { kbps: u32 },
    /// Variable bitrate, LAME quality 0 (V0, highest quality/largest files) to 4 (V4).
    Vbr { quality: u8 },
}

impl Mp3Settings {
    /// PowerVoice's ACX delivery preset (PROMPT §3.5): CBR 192 kbps. The caller pairs it with
    /// 44.1 kHz mono (S4-04's export dialog).
    pub const ACX: Mp3Settings = Mp3Settings::Cbr { kbps: 192 };

    fn validate(self) -> Result<()> {
        match self {
            Mp3Settings::Cbr { kbps } if !(128..=320).contains(&kbps) => Err(
                IoError::InvalidArgument("MP3 CBR bitrate must be 128..=320 kbps"),
            ),
            Mp3Settings::Vbr { quality } if quality > 4 => Err(IoError::InvalidArgument(
                "MP3 VBR quality must be 0 (V0) to 4 (V4)",
            )),
            _ => Ok(()),
        }
    }
}

/// Encodes mono `samples` to `path` as MP3 via runtime-loaded LAME (ADR-007 §4), atomically
/// (ADR-004 §8). Returns [`IoError::Mp3Unavailable`] if no `libmp3lame` can be loaded.
pub fn encode_mp3(
    path: impl AsRef<Path>,
    sample_rate_hz: u32,
    samples: &[f32],
    settings: Mp3Settings,
) -> Result<WriteReport> {
    settings.validate()?;
    let path = path.as_ref();
    let lame = Lame::load().map_err(IoError::Mp3Unavailable)?;
    let tmp = temp_path_for(path);
    if let Err(e) = write_temp(&lame, &tmp, sample_rate_hz, samples, settings) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    finish(&tmp, path)?;
    // LAME's internal quantization can clip on wildly over-0dBFS input, but it doesn't report a
    // count the way our TPDF quantizer does; the overs check (SPEC-005 §2.8) is a Save/Export
    // concern (S4-04), not this encoder.
    Ok(WriteReport::default())
}

fn write_temp(
    lame: &Lame,
    tmp: &Path,
    sample_rate_hz: u32,
    samples: &[f32],
    settings: Mp3Settings,
) -> Result<()> {
    let session = LameSession::open(lame)?;
    let gfp = session.gfp;
    // SAFETY: `gfp` is a live handle from `LameSession::open`, closed once by `session`'s `Drop`;
    // every call below matches the signature declared for it in `Lame` (ADR-007 §4).
    unsafe {
        (lame.lame_set_in_samplerate)(gfp, sample_rate_hz as c_int);
        (lame.lame_set_num_channels)(gfp, 1);
        (lame.lame_set_mode)(gfp, MPEG_MODE_MONO);
        match settings {
            Mp3Settings::Cbr { kbps } => {
                (lame.lame_set_vbr)(gfp, VBR_OFF);
                (lame.lame_set_brate)(gfp, kbps as c_int);
            }
            Mp3Settings::Vbr { quality } => {
                (lame.lame_set_vbr)(gfp, VBR_MTRH);
                (lame.lame_set_vbr_q)(gfp, c_int::from(quality));
            }
        }
        // LAME's own default speed/quality trade-off knob (0 best/slowest .. 9 worst/fastest); 2
        // is LAME's documented "near-best quality, recommended" setting. Not spec-mandated.
        (lame.lame_set_quality)(gfp, 2);
        let ret = (lame.lame_init_params)(gfp);
        if ret < 0 {
            return Err(IoError::Mp3Encode(format!(
                "lame_init_params failed ({ret})"
            )));
        }
    }

    let mut file = File::create(tmp)?;
    const CHUNK: usize = 8192;
    // LAME's documented worst-case output buffer size: 1.25 * nsamples + 7200 bytes.
    let mp3buf_cap = (5 * CHUNK) / 4 + 7200;
    let mut mp3buf = vec![0u8; mp3buf_cap];
    for block in samples.chunks(CHUNK) {
        // SAFETY: `mp3buf` is sized per LAME's documented worst case for up to `CHUNK` input
        // samples (`block.len() <= CHUNK`); `pcm_r` is null because mono has no right channel.
        let n = unsafe {
            (lame.lame_encode_buffer_ieee_float)(
                gfp,
                block.as_ptr(),
                std::ptr::null(),
                block.len() as c_int,
                mp3buf.as_mut_ptr(),
                mp3buf.len() as c_int,
            )
        };
        if n < 0 {
            return Err(IoError::Mp3Encode(format!("lame encode error ({n})")));
        }
        file.write_all(&mp3buf[..n as usize])?;
    }
    // SAFETY: same buffer/handle contract as the loop above.
    let n = unsafe { (lame.lame_encode_flush)(gfp, mp3buf.as_mut_ptr(), mp3buf.len() as c_int) };
    if n < 0 {
        return Err(IoError::Mp3Encode(format!("lame_encode_flush error ({n})")));
    }
    file.write_all(&mp3buf[..n as usize])?;
    file.sync_data()?;
    drop(file);

    // Best-effort: patch the real LAME/Xing header tag over the placeholder first frame, so
    // decoders read the correct duration/gapless info (ADR-007 §4 lists `lame_get_lametag_frame`).
    // Not fatal if it doesn't fit or there's nothing to write.
    let mut tag_buf = vec![0u8; mp3buf_cap];
    // SAFETY: same handle contract; called after the encode/flush calls above, before `session`'s
    // `Drop` closes `gfp`.
    let tag_len =
        unsafe { (lame.lame_get_lametag_frame)(gfp, tag_buf.as_mut_ptr(), tag_buf.len()) };
    if tag_len > 0
        && tag_len <= tag_buf.len()
        && let Ok(mut f) = OpenOptions::new().write(true).open(tmp)
    {
        let _ = f.seek(SeekFrom::Start(0));
        let _ = f.write_all(&tag_buf[..tag_len]);
        let _ = f.sync_data();
    }

    drop(session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-io-mp3-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sine(freq_hz: f64, level_dbfs: f64, duration_s: f64, rate_hz: u32) -> Vec<f32> {
        let amp = 10f64.powf(level_dbfs / 20.0);
        let n = (duration_s * f64::from(rate_hz)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate_hz);
                (amp * (std::f64::consts::TAU * freq_hz * t).sin()) as f32
            })
            .collect()
    }

    /// A valid MPEG audio frame sync is 11 set bits: byte 0 = 0xFF, byte 1's top 3 bits set.
    fn starts_with_frame_sync_or_id3(bytes: &[u8]) -> bool {
        if bytes.len() < 2 {
            return false;
        }
        if &bytes[..3.min(bytes.len())] == b"ID3" {
            return true; // the LAME/Xing tag frame's own ID3v2 header, if `id3tag_*` ever runs
        }
        bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0
    }

    #[test]
    fn cbr_192_encodes_a_plausible_mp3_stream() {
        if !mp3_available() {
            eprintln!("skipping: libmp3lame not installed");
            return;
        }
        let dir = tmp_dir("cbr");
        let path = dir.join("out.mp3");
        let samples = sine(1000.0, -20.0, 2.0, 44_100);
        encode_mp3(&path, 44_100, &samples, Mp3Settings::Cbr { kbps: 192 }).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        assert!(
            starts_with_frame_sync_or_id3(&bytes),
            "no MPEG frame sync at the start of the file"
        );
        // CBR 192 kbps for 2 s should be roughly 192_000/8*2 = 48_000 bytes; generous bounds
        // account for the LAME/Xing header frame and encoder overhead.
        assert!(
            bytes.len() > 30_000 && bytes.len() < 70_000,
            "unexpected size {} bytes for 2 s at CBR 192 kbps",
            bytes.len()
        );
    }

    #[test]
    fn vbr_quality_0_encodes_a_plausible_mp3_stream() {
        if !mp3_available() {
            eprintln!("skipping: libmp3lame not installed");
            return;
        }
        let dir = tmp_dir("vbr");
        let path = dir.join("out.mp3");
        let samples = sine(440.0, -20.0, 1.0, 48_000);
        encode_mp3(&path, 48_000, &samples, Mp3Settings::Vbr { quality: 0 }).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        assert!(starts_with_frame_sync_or_id3(&bytes));
    }

    #[test]
    fn invalid_settings_are_rejected_before_touching_the_filesystem() {
        let dir = tmp_dir("invalid");
        let path = dir.join("out.mp3");
        let samples = sine(440.0, -20.0, 0.1, 48_000);
        let err = encode_mp3(&path, 48_000, &samples, Mp3Settings::Cbr { kbps: 64 }).unwrap_err();
        assert!(matches!(err, IoError::InvalidArgument(_)));
        assert!(!path.exists());

        let err = encode_mp3(&path, 48_000, &samples, Mp3Settings::Vbr { quality: 9 }).unwrap_err();
        assert!(matches!(err, IoError::InvalidArgument(_)));
        assert!(!path.exists());
    }

    #[test]
    fn mp3_unavailable_is_reported_without_a_panic_when_no_candidate_library_exists() {
        // This only exercises the reporting path when the library really is missing; when it's
        // present (as in this dev environment), it just confirms `mp3_available` agrees with a
        // direct load attempt.
        assert_eq!(mp3_available(), Lame::load().is_ok());
    }

    #[test]
    fn encode_mp3_leaves_no_temp_file_on_success() {
        if !mp3_available() {
            eprintln!("skipping: libmp3lame not installed");
            return;
        }
        let dir = tmp_dir("atomic-ok");
        let path = dir.join("out.mp3");
        let samples = sine(440.0, -20.0, 0.2, 44_100);
        encode_mp3(&path, 44_100, &samples, Mp3Settings::Cbr { kbps: 128 }).unwrap();
        assert!(path.exists());
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.pop().unwrap(), "out.mp3");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
