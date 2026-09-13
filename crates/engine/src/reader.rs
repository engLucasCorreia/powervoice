//! Reader/prefetch (ADR-002 §1, §5): owns the playback `SnapshotReader`, keeps ~200 ms of the
//! current epoch in the playback ring, and resamples document → device rate when they differ
//! (`vox_dsp::resample`, primed, so packet positions stay exact). Never runs on an audio thread:
//! reads can page-fault. Runs on its own thread in the app, or inline on the control thread under
//! `ManualEngine` (deterministic tests).

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use rtrb::Producer;
use vox_dsp::resample::{ResampleError, StreamResampler};
use vox_project::SnapshotReader;

use crate::engine::PlaybackDoc;
use crate::rt::{PACKET_FRAMES, PLAYBACK_RING_PACKETS, Packet, READ_AHEAD_MS, packet_flags};

/// Control → reader.
pub(crate) enum ReaderCmd {
    /// A new output stream: its playback ring and rate.
    Attach {
        producer: Producer<Packet>,
        dev_rate_hz: u32,
    },
    /// The output stream closed.
    Detach,
    /// The document to play (`None`: nothing).
    SetDoc(Option<PlaybackDoc>),
    /// Stream `epoch` from document position `pos`.
    Start { epoch: u32, pos: u64 },
    /// Stop streaming.
    Stop,
}

struct Active {
    epoch: u32,
    start: u64,
    /// Next document position to read.
    next_in: u64,
    /// Device frames emitted since `start` (resampling only).
    emitted: u64,
    done: bool,
}

pub(crate) struct Reader {
    producer: Option<Producer<Packet>>,
    dev_rate_hz: u32,
    doc_rate_hz: u32,
    len: u64,
    reader: Option<SnapshotReader>,
    resampler: Option<StreamResampler>,
    /// The rates differ and no resampler could be built: nothing streams (never the wrong
    /// speed); the control thread posted the notice (`Control::build_output`).
    resample_failed: bool,
    active: Option<Active>,
}

/// The resampler playing a `doc_rate_hz` document at `dev_rate_hz` needs (`None`: same rate).
pub(crate) fn resampler_for(
    doc_rate_hz: u32,
    dev_rate_hz: u32,
) -> Result<Option<StreamResampler>, ResampleError> {
    if doc_rate_hz == dev_rate_hz {
        return Ok(None);
    }
    StreamResampler::new(doc_rate_hz, dev_rate_hz).map(Some)
}

impl Reader {
    pub(crate) fn new() -> Self {
        Self {
            producer: None,
            dev_rate_hz: 0,
            doc_rate_hz: 0,
            len: 0,
            reader: None,
            resampler: None,
            resample_failed: false,
            active: None,
        }
    }

    fn rebuild_resampler(&mut self) {
        let built = if self.reader.is_some() && self.dev_rate_hz > 0 && self.doc_rate_hz > 0 {
            resampler_for(self.doc_rate_hz, self.dev_rate_hz)
        } else {
            Ok(None)
        };
        self.resample_failed = built.is_err();
        self.resampler = built.ok().flatten();
    }

    pub(crate) fn handle(&mut self, cmd: ReaderCmd) {
        match cmd {
            ReaderCmd::Attach {
                producer,
                dev_rate_hz,
            } => {
                self.producer = Some(producer);
                self.dev_rate_hz = dev_rate_hz;
                self.active = None;
                self.rebuild_resampler();
            }
            ReaderCmd::Detach => {
                self.producer = None;
                self.active = None;
                if let Some(r) = self.reader.as_mut() {
                    r.release_segments();
                }
            }
            ReaderCmd::SetDoc(doc) => {
                self.active = None;
                match doc {
                    Some(d) => {
                        self.doc_rate_hz = d.snapshot.sample_rate_hz;
                        self.len = d.snapshot.len_samples;
                        // The previous reader (and its snapshot) is dropped here, off the audio
                        // thread.
                        self.reader = Some(SnapshotReader::new(d.store, d.snapshot));
                    }
                    None => {
                        self.reader = None;
                        self.len = 0;
                    }
                }
                self.rebuild_resampler();
            }
            ReaderCmd::Start { epoch, pos } => {
                self.active = Some(Active {
                    epoch,
                    start: pos,
                    next_in: pos,
                    emitted: 0,
                    done: false,
                });
                if let Some(r) = self.resampler.as_mut() {
                    r.reset();
                }
            }
            ReaderCmd::Stop => {
                self.active = None;
                if let Some(r) = self.reader.as_mut() {
                    r.release_segments();
                }
            }
        }
    }

    fn target_packets(&self) -> usize {
        let frames = u64::from(self.dev_rate_hz) * READ_AHEAD_MS / 1000;
        (frames as usize)
            .div_ceil(PACKET_FRAMES)
            .clamp(1, PLAYBACK_RING_PACKETS - 2)
    }

    /// Tops the playback ring up to the read-ahead. Returns the packets pushed.
    pub(crate) fn fill(&mut self) -> usize {
        let target = self.target_packets();
        let Reader {
            producer: Some(prod),
            reader: Some(reader),
            active: Some(a),
            resample_failed: false,
            resampler,
            len,
            doc_rate_hz,
            dev_rate_hz,
        } = self
        else {
            return 0;
        };
        let len = *len;
        let mut pushed = 0;
        while !a.done && prod.slots() > 0 && PLAYBACK_RING_PACKETS - prod.slots() < target {
            let mut pkt = Packet::new(a.epoch);
            match resampler.as_mut() {
                None => {
                    if a.next_in >= len {
                        pkt.flags = packet_flags::END;
                        pkt.doc_pos = len;
                        a.done = true;
                    } else {
                        let n = (len - a.next_in).min(PACKET_FRAMES as u64) as usize;
                        let got = reader.read(a.next_in, &mut pkt.samples[..n]).unwrap_or(0);
                        pkt.samples[got.min(n)..n].fill(0.0);
                        pkt.len = n as u16;
                        pkt.doc_pos = a.next_in;
                        a.next_in += n as u64;
                    }
                }
                Some(rs) => {
                    let num = u128::from(*doc_rate_hz);
                    let den = u128::from(*dev_rate_hz);
                    let doc_pos = a.start + (u128::from(a.emitted) * num / den) as u64;
                    let total =
                        (u128::from(len.saturating_sub(a.start)) * den).div_ceil(num) as u64;
                    if doc_pos >= len || a.emitted >= total {
                        pkt.flags = packet_flags::END;
                        pkt.doc_pos = len;
                        a.done = true;
                    } else {
                        let n = (total - a.emitted).min(PACKET_FRAMES as u64) as usize;
                        let next_in = &mut a.next_in;
                        let pulled = rs.pull(&mut pkt.samples[..n], |buf| {
                            let n = buf.len();
                            let got = reader.read(*next_in, buf).unwrap_or(0);
                            buf[got.min(n)..].fill(0.0);
                            *next_in += n as u64;
                        });
                        if pulled.is_err() {
                            pkt.samples[..n].fill(0.0);
                        }
                        pkt.len = n as u16;
                        pkt.doc_pos = doc_pos;
                        a.emitted += n as u64;
                    }
                }
            }
            if prod.push(pkt).is_err() {
                break;
            }
            pushed += 1;
        }
        pushed
    }
}

/// The reader thread's loop: handle commands (waking at once), top up every ≤ 5 ms.
pub(crate) fn run(mut reader: Reader, rx: Receiver<ReaderCmd>) {
    loop {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(cmd) => {
                reader.handle(cmd);
                while let Ok(cmd) = rx.try_recv() {
                    reader.handle(cmd);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        reader.fill();
    }
}

#[cfg(test)]
mod tests {
    use super::resampler_for;

    #[test]
    fn resampler_only_when_the_rates_differ() {
        assert!(matches!(resampler_for(48_000, 48_000), Ok(None)));
        assert!(matches!(resampler_for(44_100, 48_000), Ok(Some(_))));
        assert!(resampler_for(0, 48_000).is_err(), "unsupported rates fail");
    }
}
