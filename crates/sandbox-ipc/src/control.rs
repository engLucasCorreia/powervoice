//! Control-channel framing (T-802, ADR-008 §4 + Amendment 2).
//!
//! The channel is a pair of anonymous pipes — the sandbox's stdin (host → sandbox) and a private
//! duplicate of its stdout (sandbox → host; the sandbox points its own fd 1 at stderr, so a
//! plugin that prints can't corrupt the stream). One frame:
//!
//! | Bytes | Content |
//! |---|---|
//! | 4 | `total` = 4 + `json_len` + payload length, `u32` little-endian |
//! | 4 | `json_len`, `u32` little-endian |
//! | `json_len` | the message, UTF-8 JSON ([`crate::protocol`]) |
//! | rest | an opaque binary payload (plugin state), possibly empty |
//!
//! Frames larger than [`MAX_FRAME_BYTES`] are refused on both sides. Control-thread code only:
//! reading and writing block.

use std::io::{self, Read, Write};

/// Largest frame (header excluded) either side accepts: 64 MiB — generous for plugin state
/// chunks (sample-based plugins), small enough that a garbage length can't exhaust memory.
pub const MAX_FRAME_BYTES: usize = 64 << 20;

/// One received frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The JSON message.
    pub json: Vec<u8>,
    /// The binary payload (empty if none).
    pub payload: Vec<u8>,
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Writes one frame and flushes.
pub fn write_frame(w: &mut impl Write, json: &[u8], payload: &[u8]) -> io::Result<()> {
    let total = 4 + json.len() + payload.len();
    if total > MAX_FRAME_BYTES {
        return Err(invalid("control frame too large"));
    }
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&(total as u32).to_le_bytes());
    header[4..].copy_from_slice(&(json.len() as u32).to_le_bytes());
    w.write_all(&header)?;
    w.write_all(json)?;
    w.write_all(payload)?;
    w.flush()
}

/// Reads one frame. `Ok(None)` on a clean end of stream at a frame boundary (the peer closed
/// its end); a stream that ends inside a frame is an `UnexpectedEof` error.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let total = u32::from_le_bytes(len) as usize;
    if !(4..=MAX_FRAME_BYTES).contains(&total) {
        return Err(invalid("bad control frame length"));
    }
    let mut body = vec![0u8; total];
    r.read_exact(&mut body)?;
    let json_len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
    if json_len > total - 4 {
        return Err(invalid("bad control frame json length"));
    }
    let payload = body.split_off(4 + json_len);
    body.drain(..4);
    Ok(Some(Frame {
        json: body,
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_back_to_back() {
        let mut buf = Vec::new();
        write_frame(&mut buf, br#"{"a":1}"#, &[]).unwrap();
        write_frame(&mut buf, b"{}", &[0, 1, 2, 255]).unwrap();
        let mut r = buf.as_slice();
        let a = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(a.json, br#"{"a":1}"#);
        assert!(a.payload.is_empty());
        let b = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(
            (b.json.as_slice(), b.payload.as_slice()),
            (&b"{}"[..], &[0, 1, 2, 255][..])
        );
        assert_eq!(read_frame(&mut r).unwrap(), None, "clean end of stream");
    }

    #[test]
    fn truncated_and_garbage_frames_are_errors() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{}", b"xyz").unwrap();
        let cut = &buf[..buf.len() - 1];
        assert!(read_frame(&mut &cut[..]).is_err());
        assert!(read_frame(&mut &buf[..2]).is_err());
        let huge = u32::MAX.to_le_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
        // json_len beyond the frame.
        let mut bad = Vec::new();
        bad.extend_from_slice(&8u32.to_le_bytes());
        bad.extend_from_slice(&100u32.to_le_bytes());
        bad.extend_from_slice(b"abcd");
        assert!(read_frame(&mut bad.as_slice()).is_err());
    }
}
