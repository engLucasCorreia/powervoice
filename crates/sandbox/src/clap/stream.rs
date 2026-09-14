//! `clap_ostream` / `clap_istream` over byte buffers (plugin state).

use std::ffi::c_void;

use vox_clap_abi::{clap_istream, clap_ostream};

unsafe extern "C" fn write_cb(
    stream: *const clap_ostream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    if stream.is_null() || (buffer.is_null() && size > 0) {
        return -1;
    }
    // SAFETY: `ctx` is the `Vec<u8>` of `save`, alive for the call.
    let out = unsafe { &mut *((*stream).ctx as *mut Vec<u8>) };
    let n = size.min(i64::MAX as u64) as usize;
    if n > 0 {
        // SAFETY: the plugin's buffer of `size` bytes.
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) });
    }
    n as i64
}

/// Runs `f` with an output stream and returns what it wrote (`None` if `f` failed).
pub(crate) fn save(f: impl FnOnce(&clap_ostream) -> bool) -> Option<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let stream = clap_ostream {
        ctx: (&raw mut data).cast(),
        write: Some(write_cb),
    };
    f(&stream).then_some(data)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "C" fn read_cb(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    if stream.is_null() || (buffer.is_null() && size > 0) {
        return -1;
    }
    // SAFETY: `ctx` is the `Reader` of `load`, alive for the call.
    let r = unsafe { &mut *((*stream).ctx as *mut Reader<'_>) };
    let left = r.data.len() - r.pos;
    let n = (size.min(i64::MAX as u64) as usize).min(left);
    // SAFETY: the plugin's buffer has room for `size` ≥ `n` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(r.data.as_ptr().add(r.pos), buffer.cast::<u8>(), n);
    }
    r.pos += n;
    n as i64
}

/// Runs `f` with an input stream over `data`.
pub(crate) fn load(data: &[u8], f: impl FnOnce(&clap_istream) -> bool) -> bool {
    let mut reader = Reader { data, pos: 0 };
    let stream = clap_istream {
        ctx: (&raw mut reader).cast(),
        read: Some(read_cb),
    };
    f(&stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_round_trip_in_pieces() {
        let data = save(|s| {
            for chunk in [&b"ab"[..], b"", b"cdef"] {
                // SAFETY: our own stream and buffers.
                let n = unsafe { (s.write.unwrap())(s, chunk.as_ptr().cast(), chunk.len() as u64) };
                assert_eq!(n, chunk.len() as i64);
            }
            true
        })
        .unwrap();
        assert_eq!(data, b"abcdef");
        assert!(save(|_| false).is_none());
        let mut got = Vec::new();
        assert!(load(&data, |s| {
            let mut buf = [0u8; 4];
            loop {
                // SAFETY: our own stream and buffer.
                let n = unsafe { (s.read.unwrap())(s, buf.as_mut_ptr().cast(), 4) };
                if n == 0 {
                    return true;
                }
                got.extend_from_slice(&buf[..n as usize]);
            }
        }));
        assert_eq!(got, data);
    }
}
