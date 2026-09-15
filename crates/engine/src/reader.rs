//! Reader/prefetch (ADR-002 §1, §5): owns the playback `SnapshotReader`, keeps ~200 ms of the
//! current epoch in the playback ring, and resamples document → device rate when they differ
//! (`vox_dsp::resample`, primed, so packet positions stay exact). Never runs on an audio thread:
//! reads can page-fault. Runs on its own thread in the app, or inline on the control thread under
//! `ManualEngine` (deterministic tests).
//!
//! T-304 (SPEC-022 §4.4): a record operation's playback **run** streams virtual positions
//! (`RunSpec`): silence before document position 0, the record range muted (or the original, with
//! Hear original), the 5 ms listening fades, and an END packet at the run end (none for cursor
//! recordings, which play silence until Stop). No `DISCONTINUITY` and no rack reset inside a run.
//!
//! H-37 (SPEC-003 §2.1, ADR-002 Amendment 3): **loop playback.** The input stream (what is read,
//! or fed to the resampler) jumps from the loop end back to the loop start, sample-exactly; each
//! jump is queued as a segment `(virtual input index, document position)`. Output packets never
//! straddle a jump: the first packet of a pass carries [`packet_flags::LOOP_WRAP`] and the loop
//! start as its `doc_pos` (with resampling, the first device frame whose input time reaches the
//! jump). The resampler is never reset at a seam, so the resampled stream stays continuous. Loop
//! off during a pass ends the stream at the old loop end (an END packet there).

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use rtrb::Producer;
use vox_dsp::resample::{ResampleError, StreamResampler};
use vox_project::SnapshotReader;

use crate::engine::PlaybackDoc;
use crate::record_op::RunSpec;
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
    /// T-304: stream a record operation's run `epoch` from virtual position `pos`.
    StartRun { epoch: u32, pos: u64, run: RunSpec },
    /// Stop streaming.
    Stop,
    /// H-37: the effective loop region (`None`: not looping). `finish`: loop was turned off
    /// during playback — a stream inside (or heading into) the old region ends at its end.
    SetLoop {
        region: Option<(u64, u64)>,
        finish: bool,
    },
}

/// Where the input stream (re)starts: from virtual input index `virt` on, it reads document
/// position `doc` onwards. `wrap`: a loop jump (not the stream start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seg {
    virt: u64,
    doc: u64,
    wrap: bool,
}

struct Active {
    epoch: u32,
    /// Next (virtual, for a run) position to read.
    next_in: u64,
    /// Output (device) frames emitted since the start.
    emitted: u64,
    /// Input samples read since the start (= `emitted` without resampling).
    fed: u64,
    /// Input segments not yet fully passed by the output, oldest first (never empty).
    segs: VecDeque<Seg>,
    /// H-37: loop turned off mid-pass — the stream ends at this document position.
    end_at: Option<u64>,
    done: bool,
    /// T-304: a record operation's run (positions are virtual).
    run: Option<RunSpec>,
}

/// The first output frame whose input time (frame · `num` / `den` input samples) reaches the
/// virtual input index `virt`.
fn first_frame(virt: u64, num: u128, den: u128) -> u64 {
    (u128::from(virt) * den).div_ceil(num) as u64
}

impl Active {
    /// Fills `buf` with the next input samples, jumping from the loop end back to the loop start
    /// (`looping`, only while the read position is before the loop end) and queueing each jump.
    fn feed(&mut self, reader: &mut SnapshotReader, looping: Option<(u64, u64)>, buf: &mut [f32]) {
        let mut off = 0;
        while off < buf.len() {
            let wrap = looping.filter(|&(_, e)| self.next_in < e);
            let mut n = buf.len() - off;
            if let Some((_, e)) = wrap {
                n = n.min((e - self.next_in) as usize);
            }
            let chunk = &mut buf[off..off + n];
            match self.run {
                Some(run) => run.render(self.next_in, chunk, |q, b| read_doc(reader, q, b)),
                None => read_doc(reader, self.next_in, chunk),
            }
            self.next_in += n as u64;
            self.fed += n as u64;
            off += n;
            if let Some((s, e)) = wrap
                && self.next_in == e
            {
                self.next_in = s;
                self.segs.push_back(Seg {
                    virt: self.fed,
                    doc: s,
                    wrap: true,
                });
            }
        }
    }
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
    /// H-37: the effective loop region (never applied to a record operation's run).
    loop_region: Option<(u64, u64)>,
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

/// Reads `A[pos, pos + buf.len())`, zero-filling whatever the reader didn't deliver.
fn read_doc(reader: &mut SnapshotReader, pos: u64, buf: &mut [f32]) {
    let got = reader.read(pos, buf).unwrap_or(0);
    let n = buf.len();
    buf[got.min(n)..].fill(0.0);
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
            loop_region: None,
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

    fn start(&mut self, epoch: u32, pos: u64, run: Option<RunSpec>) {
        let mut segs = VecDeque::with_capacity(8);
        segs.push_back(Seg {
            virt: 0,
            doc: pos,
            wrap: false,
        });
        self.active = Some(Active {
            epoch,
            next_in: pos,
            emitted: 0,
            fed: 0,
            segs,
            end_at: None,
            done: false,
            run,
        });
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
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
            ReaderCmd::Start { epoch, pos } => self.start(epoch, pos, None),
            ReaderCmd::StartRun { epoch, pos, run } => self.start(epoch, pos, Some(run)),
            ReaderCmd::Stop => {
                self.active = None;
                if let Some(r) = self.reader.as_mut() {
                    r.release_segments();
                }
            }
            ReaderCmd::SetLoop { region, finish } => {
                let old = self.loop_region;
                self.loop_region = region;
                if let Some(a) = self.active.as_mut()
                    && a.run.is_none()
                    && !a.done
                {
                    a.end_at = match (finish, old) {
                        // SPEC-003 §2.1: the pass finishes, then the stream ends at the old end.
                        (true, Some((_, e))) if a.next_in <= e => Some(e),
                        (true, _) => a.end_at,
                        (false, _) => None,
                    };
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
            loop_region,
        } = self
        else {
            return 0;
        };
        // Input samples per output frame, as a ratio (1:1 without resampling).
        let (num, den) = if resampler.is_some() {
            (u128::from(*doc_rate_hz), u128::from(*dev_rate_hz))
        } else {
            (1, 1)
        };
        let looping = if a.run.is_some() { None } else { *loop_region };
        // Where the stream ends: the document end (or the old loop end after loop off), or a
        // run's end (`None`: never).
        let limit = match a.run {
            Some(run) => run.end_v(),
            None => Some(a.end_at.map_or(*len, |e| e.min(*len))),
        };
        let mut pushed = 0;
        while !a.done && prod.slots() > 0 && PLAYBACK_RING_PACKETS - prod.slots() < target {
            let mut pkt = Packet::new(a.epoch);
            let j = a.emitted;
            while a.segs.len() > 1 && first_frame(a.segs[1].virt, num, den) <= j {
                a.segs.pop_front();
            }
            let seg = a.segs[0];
            let virt_j = (u128::from(j) * num / den) as u64;
            let doc_pos = seg.doc + virt_j.saturating_sub(seg.virt);
            // The next jump: already queued by the input side, or the one it will make at the
            // loop end.
            let next_wrap = if a.segs.len() > 1 {
                Some(a.segs[1].virt)
            } else {
                looping
                    .filter(|&(_, e)| a.next_in < e)
                    .map(|(_, e)| a.fed + (e - a.next_in))
            };
            // The stream end, in the newest segment — unless the input wraps before reaching it.
            let back = a.segs.back().copied().unwrap_or(seg);
            let end_frame = limit
                .filter(|&l| !looping.is_some_and(|(_, e)| a.next_in < e && e <= l))
                .map(|l| first_frame(back.virt + l.saturating_sub(back.doc), num, den));
            if end_frame.is_some_and(|f| j >= f) {
                pkt.flags = packet_flags::END;
                pkt.doc_pos = limit.unwrap_or(doc_pos);
                a.done = true;
            } else {
                let mut n = PACKET_FRAMES as u64;
                if let Some(f) = end_frame {
                    n = n.min(f - j);
                }
                if let Some(f) = next_wrap.map(|w| first_frame(w, num, den))
                    && f > j
                {
                    n = n.min(f - j);
                }
                let n = n as usize;
                if seg.wrap && j == first_frame(seg.virt, num, den) {
                    pkt.flags |= packet_flags::LOOP_WRAP;
                }
                match resampler.as_mut() {
                    None => a.feed(reader, looping, &mut pkt.samples[..n]),
                    Some(rs) => {
                        let pulled =
                            rs.pull(&mut pkt.samples[..n], |buf| a.feed(reader, looping, buf));
                        if pulled.is_err() {
                            pkt.samples[..n].fill(0.0);
                        }
                    }
                }
                pkt.len = n as u16;
                pkt.doc_pos = doc_pos;
                a.emitted += n as u64;
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
