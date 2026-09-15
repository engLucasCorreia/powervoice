//! The host's `IBStream` over a byte buffer (plugin state), with `seek`/`tell`.

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard, PoisonError};

use vst3::Steinberg::IBStream_::IStreamSeekMode_::{kIBSeekCur, kIBSeekEnd, kIBSeekSet};
use vst3::Steinberg::{
    IBStream, IBStreamTrait, int32, int64, kInvalidArgument, kResultOk, tresult,
};
use vst3::{Class, ComWrapper};

/// A growable in-memory stream.
pub(crate) struct MemoryStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}

impl MemoryStream {
    fn lock(&self) -> MutexGuard<'_, (Vec<u8>, usize)> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_read: *mut int32) -> tresult {
        if num_bytes < 0 || (buffer.is_null() && num_bytes > 0) {
            return kInvalidArgument;
        }
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        let start = (*pos).min(data.len());
        let n = (num_bytes as usize).min(data.len() - start);
        // SAFETY: the caller's buffer has room for `num_bytes` ≥ `n` bytes.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().add(start), buffer.cast(), n) };
        *pos = start + n;
        if !num_read.is_null() {
            // SAFETY: writable per the contract.
            unsafe { *num_read = n as int32 };
        }
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_written: *mut int32,
    ) -> tresult {
        if num_bytes < 0 || (buffer.is_null() && num_bytes > 0) {
            return kInvalidArgument;
        }
        let n = num_bytes as usize;
        let mut g = self.lock();
        let (data, pos) = &mut *g;
        if *pos + n > data.len() {
            data.resize(*pos + n, 0);
        }
        // SAFETY: the caller's buffer holds `num_bytes` bytes; `data` has room from `pos`.
        unsafe {
            std::ptr::copy_nonoverlapping(buffer.cast::<u8>(), data.as_mut_ptr().add(*pos), n);
        }
        *pos += n;
        if !num_written.is_null() {
            // SAFETY: writable per the contract.
            unsafe { *num_written = num_bytes };
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut g = self.lock();
        let (data, cur) = &mut *g;
        let base = match mode as u32 as vst3::Steinberg::IBStream_::IStreamSeekMode {
            m if m == kIBSeekSet => 0i64,
            m if m == kIBSeekCur => *cur as i64,
            m if m == kIBSeekEnd => data.len() as i64,
            _ => return kInvalidArgument,
        };
        let target = base.saturating_add(pos).max(0);
        *cur = target as usize;
        if !result.is_null() {
            // SAFETY: writable per the contract.
            unsafe { *result = target };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: writable per the contract.
        unsafe { *pos = self.lock().1 as int64 };
        kResultOk
    }
}

/// Runs `f` with an empty stream and returns what it wrote (`Err`: `f`'s result code).
pub(crate) fn save(f: impl FnOnce(*mut IBStream) -> tresult) -> Result<Vec<u8>, tresult> {
    let stream = ComWrapper::new(MemoryStream {
        inner: Mutex::new((Vec::new(), 0)),
    });
    let ptr = stream
        .as_com_ref::<IBStream>()
        .map_or(std::ptr::null_mut(), |r| r.as_ptr());
    match f(ptr) {
        r if r == kResultOk => Ok(std::mem::take(&mut stream.lock().0)),
        r => Err(r),
    }
}

/// Runs `f` with a stream positioned at the start of `data`; returns `f`'s result code.
pub(crate) fn load(data: &[u8], f: impl FnOnce(*mut IBStream) -> tresult) -> tresult {
    let stream = ComWrapper::new(MemoryStream {
        inner: Mutex::new((data.to_vec(), 0)),
    });
    let ptr = stream
        .as_com_ref::<IBStream>()
        .map_or(std::ptr::null_mut(), |r| r.as_ptr());
    f(ptr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    #[test]
    fn streams_read_write_seek_and_tell() {
        let data = save(|s| {
            // SAFETY: our own live stream.
            let s = unsafe { ComRef::from_raw(s) }.unwrap();
            let mut n = 0;
            // SAFETY: our own stream and buffers.
            unsafe {
                assert_eq!(
                    s.write(b"abcd".as_ptr() as *mut c_void, 4, &mut n),
                    kResultOk
                );
                assert_eq!(n, 4);
                let mut at = 0;
                s.seek(1, kIBSeekSet as int32, &mut at);
                assert_eq!(at, 1);
                s.write(b"XY".as_ptr() as *mut c_void, 2, &mut n);
                s.seek(0, kIBSeekEnd as int32, &mut at);
                s.write(b"!".as_ptr() as *mut c_void, 1, &mut n);
                s.tell(&mut at);
                assert_eq!(at, 5);
            }
            kResultOk
        })
        .unwrap();
        assert_eq!(data, b"aXYd!");
        assert_eq!(save(|_| 1), Err(1));
        let mut got = Vec::new();
        let r = load(&data, |s| {
            // SAFETY: our own live stream.
            let s = unsafe { ComRef::from_raw(s) }.unwrap();
            let mut buf = [0u8; 3];
            loop {
                let mut n = 0;
                // SAFETY: our own stream and buffer.
                unsafe { s.read(buf.as_mut_ptr().cast(), 3, &mut n) };
                if n == 0 {
                    return kResultOk;
                }
                got.extend_from_slice(&buf[..n as usize]);
            }
        });
        assert_eq!((r, got.as_slice()), (kResultOk, data.as_slice()));
    }
}
