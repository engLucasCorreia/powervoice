//! Transport state machine (SPEC-003 §2.1, D-018), pure and control-thread only.
//!
//! - **Play** starts at the playhead (from 0 when the playhead is at the document end).
//! - **Pause** halts (~5 ms fade) and keeps the heard position.
//! - **Stop** halts and returns the playhead to where Play (or Play from start) was last pressed;
//!   Stop while stopped does nothing.
//! - **Play from start** plays from the selection start, or 0 (a seek while playing).
//! - **Return to start** seeks to 0 (keeps playing when playing).
//! - **Seek** while playing restarts at the new position (fade-out, rack reset, fade-in).
//! - Engine-initiated stops (device loss, new document, document end) behave like Pause.
//! - **Loop** (H-37, SPEC-003 §2.1/§3): a toggle; while on, the effective loop region is the
//!   time selection (inert without one, or when shorter than the minimum). The document end and
//!   the end of a pass after loop off both stop at their position (Pause semantics).

/// A transport command (UI → engine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportCommand {
    /// Start playback at the playhead (no-op while playing).
    Play,
    /// Halt, keeping the heard position (no-op while stopped).
    Pause,
    /// Space: Pause while playing, else Play.
    PlayPause,
    /// Halt and return to the play-start position (no-op while stopped).
    Stop,
    /// Shift+Space: play from the selection start, or 0.
    PlayFromStart,
    /// Home: seek to 0.
    ReturnToStart,
    /// Move the playhead to a document position (clamped to the length).
    Seek(u64),
}

/// Transport state reported to the UI (`transport_state` event). While playing, the moving
/// playhead comes from telemetry; `playhead_samples` is then the position playback (re)started at.
#[derive(Clone, Debug, Default)]
pub struct TransportState {
    /// Playback is running.
    pub playing: bool,
    /// Stopped: the playhead. Playing: the position the current pass started from.
    pub playhead_samples: u64,
    /// Where Play / Play from start was last pressed (Stop returns here).
    pub play_start_samples: u64,
    /// Document length (0: no document).
    pub doc_len_samples: u64,
    /// Document sample rate (0: no document).
    pub doc_rate_hz: u32,
    /// A document is loaded and an output stream is open.
    pub can_play: bool,
    /// H-37: the Loop toggle.
    pub loop_enabled: bool,
    /// H-37: the effective loop region `[start, end)` (loop on and a long-enough selection).
    pub loop_range: Option<(u64, u64)>,
    /// H-81: bumped by the control thread every time the state actually changes (never by a
    /// plain query). Tauri's command responses and its `emit`/`listen` events are independent
    /// channels with no ordering guarantee relative to each other, so a slower concurrent command
    /// can otherwise deliver an older snapshot *after* a newer one already landed — the UI's
    /// `applyState` drops anything not newer than what it already has. Excluded from equality:
    /// two states are the same state whether or not this counter differs.
    pub revision: u64,
}

impl PartialEq for TransportState {
    fn eq(&self, other: &Self) -> bool {
        self.playing == other.playing
            && self.playhead_samples == other.playhead_samples
            && self.play_start_samples == other.play_start_samples
            && self.doc_len_samples == other.doc_len_samples
            && self.doc_rate_hz == other.doc_rate_hz
            && self.can_play == other.can_play
            && self.loop_enabled == other.loop_enabled
            && self.loop_range == other.loop_range
    }
}

impl Eq for TransportState {}

/// What the control thread must do after a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Start { epoch: u32, pos: u64, reset: bool },
    Seek { epoch: u32, pos: u64 },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopKind {
    Pause,
    Stop,
}

#[derive(Debug, Default)]
pub(crate) struct Transport {
    playing: bool,
    playhead: u64,
    play_start: u64,
    /// Target of the last start/seek (shown until the first heard block of its epoch).
    display_pos: u64,
    epoch: u32,
    len: u64,
    selection: Option<(u64, u64)>,
    /// Document position the rack's input stopped at (a resume there needs no rack reset).
    rack_pos: Option<u64>,
    /// A stop fade whose final position is still to come.
    awaiting: Option<(u32, StopKind)>,
    /// H-37: the Loop toggle.
    loop_enabled: bool,
}

impl Transport {
    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    pub(crate) fn playhead(&self) -> u64 {
        self.playhead
    }

    pub(crate) fn play_start(&self) -> u64 {
        self.play_start
    }

    pub(crate) fn display_pos(&self) -> u64 {
        self.display_pos
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn loop_enabled(&self) -> bool {
        self.loop_enabled
    }

    /// H-37: sets the Loop toggle.
    pub(crate) fn set_loop_enabled(&mut self, on: bool) {
        self.loop_enabled = on;
    }

    /// H-37/H-80 (SPEC-003 §3, Amendment 3): the effective loop region — the selection while loop
    /// is on, when it is at least `min_len` samples long; otherwise the whole document (no
    /// selection, or one shorter than the minimum). `None` only when loop is off or there is no
    /// document to loop.
    pub(crate) fn loop_region(&self, min_len: u64) -> Option<(u64, u64)> {
        if !self.loop_enabled || self.len == 0 {
            return None;
        }
        match self.selection {
            Some((s, e)) if e - s >= min_len.max(1) => Some((s, e)),
            _ => Some((0, self.len)),
        }
    }

    /// Applies `cmd`. `heard`: the heard position now (for Pause); `alive`: the output stream
    /// is open (a stop fade will report its final position).
    pub(crate) fn command(
        &mut self,
        cmd: TransportCommand,
        can_play: bool,
        heard: u64,
        alive: bool,
    ) -> Option<Action> {
        match cmd {
            TransportCommand::Play => self.play(can_play),
            TransportCommand::Pause => self.pause(heard, alive),
            TransportCommand::PlayPause if self.playing => self.pause(heard, alive),
            TransportCommand::PlayPause => self.play(can_play),
            TransportCommand::Stop => self.stop(alive),
            TransportCommand::PlayFromStart => {
                let pos = self.selection.map_or(0, |s| s.0).min(self.len);
                if self.playing {
                    self.play_start = pos;
                    self.seek(pos)
                } else {
                    self.playhead = pos;
                    self.play(can_play)
                }
            }
            TransportCommand::ReturnToStart => self.seek(0),
            TransportCommand::Seek(pos) => self.seek(pos),
        }
    }

    fn next_epoch(&mut self) -> u32 {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.epoch
    }

    /// T-304 (SPEC-022 §4.4): a fresh epoch for a record operation's playback run. The transport
    /// itself stays stopped (its state machine never sees the run); packets of older epochs are
    /// discarded, and the next Play gets a newer epoch and resets the rack.
    pub(crate) fn next_run_epoch(&mut self) -> u32 {
        self.rack_pos = None;
        self.awaiting = None;
        self.next_epoch()
    }

    fn play(&mut self, can_play: bool) -> Option<Action> {
        if self.playing || !can_play || self.len == 0 {
            return None;
        }
        let pos = if self.playhead >= self.len {
            0
        } else {
            self.playhead
        };
        let epoch = self.next_epoch();
        self.playing = true;
        self.play_start = pos;
        self.display_pos = pos;
        self.playhead = pos;
        self.awaiting = None;
        let reset = self.rack_pos != Some(pos);
        Some(Action::Start { epoch, pos, reset })
    }

    /// Pause semantics; also used for engine-initiated stops.
    pub(crate) fn pause(&mut self, heard: u64, alive: bool) -> Option<Action> {
        if !self.playing {
            return None;
        }
        self.playing = false;
        self.playhead = heard.min(self.len);
        self.rack_pos = None;
        self.awaiting = alive.then_some((self.epoch, StopKind::Pause));
        Some(Action::Stop)
    }

    fn stop(&mut self, alive: bool) -> Option<Action> {
        if !self.playing {
            return None;
        }
        self.playing = false;
        self.playhead = self.play_start;
        self.rack_pos = None;
        self.awaiting = alive.then_some((self.epoch, StopKind::Stop));
        Some(Action::Stop)
    }

    fn seek(&mut self, pos: u64) -> Option<Action> {
        let pos = pos.min(self.len);
        if self.playing {
            let epoch = self.next_epoch();
            self.display_pos = pos;
            Some(Action::Seek { epoch, pos })
        } else {
            self.playhead = pos;
            None
        }
    }

    /// A stop fade finished at `pos` (the next position that would have played).
    pub(crate) fn on_stopped(&mut self, epoch: u32, pos: u64) {
        if let Some((e, kind)) = self.awaiting
            && e == epoch
        {
            self.awaiting = None;
            self.rack_pos = Some(pos);
            if kind == StopKind::Pause {
                self.playhead = pos.min(self.len);
            }
        }
    }

    /// The document end was played. Returns true if that stopped the transport.
    pub(crate) fn on_ended(&mut self, epoch: u32, pos: u64) -> bool {
        if self.playing && epoch == self.epoch {
            self.playing = false;
            // The document end, or (H-37) the old loop end after loop off.
            self.playhead = pos.min(self.len);
            self.rack_pos = Some(pos);
            self.awaiting = None;
            return true;
        }
        if self.awaiting.is_some_and(|(e, _)| e == epoch) {
            self.awaiting = None;
        }
        false
    }

    /// The output stream closed: no stop fade will report back.
    pub(crate) fn stream_closed(&mut self) {
        self.awaiting = None;
        self.rack_pos = None;
    }

    /// A new document or revision (length `len`); the caller stopped playback first. The
    /// selection is the UI's (H-37: it syncs every change), so it is only clamped here.
    pub(crate) fn set_doc(&mut self, len: u64) {
        self.len = len;
        self.playhead = self.playhead.min(len);
        self.play_start = self.play_start.min(len);
        self.display_pos = self.display_pos.min(len);
        self.rack_pos = None;
        let sel = self.selection;
        self.set_selection(sel);
    }

    /// The time selection (Play from start, the loop region), clamped to the document; an
    /// empty one is none.
    pub(crate) fn set_selection(&mut self, sel: Option<(u64, u64)>) {
        self.selection = sel
            .map(|(a, b)| (a.min(b).min(self.len), a.max(b).min(self.len)))
            .filter(|&(a, b)| b > a);
    }
}

#[cfg(test)]
mod tests {
    use super::TransportCommand::*;
    use super::*;

    fn doc(len: u64) -> Transport {
        let mut t = Transport::default();
        t.set_doc(len);
        t
    }

    #[test]
    fn play_pause_keeps_and_stop_returns() {
        let mut t = doc(1000);
        t.command(Seek(100), true, 0, true);
        assert_eq!(
            t.command(Play, true, 0, true),
            Some(Action::Start {
                epoch: 1,
                pos: 100,
                reset: true
            })
        );
        assert_eq!(t.command(Pause, true, 400, true), Some(Action::Stop));
        assert_eq!(t.playhead(), 400);
        t.on_stopped(1, 410);
        assert_eq!(t.playhead(), 410, "final heard position");
        // Resume without a rack reset.
        assert_eq!(
            t.command(PlayPause, true, 0, true),
            Some(Action::Start {
                epoch: 2,
                pos: 410,
                reset: false
            })
        );
        assert_eq!(t.command(Stop, true, 700, true), Some(Action::Stop));
        assert_eq!(t.playhead(), 410, "Stop returns to where Play was pressed");
        t.on_stopped(2, 720);
        assert_eq!(t.playhead(), 410);
        assert_eq!(t.command(Stop, true, 0, true), None, "Stop while stopped");
    }

    #[test]
    fn play_from_start_return_to_start_and_seek() {
        let mut t = doc(1000);
        t.set_selection(Some((300, 200)));
        assert_eq!(
            t.command(PlayFromStart, true, 0, true),
            Some(Action::Start {
                epoch: 1,
                pos: 200,
                reset: true
            })
        );
        assert_eq!(
            t.command(ReturnToStart, true, 0, true),
            Some(Action::Seek { epoch: 2, pos: 0 })
        );
        assert!(t.playing());
        assert_eq!(t.play_start(), 200, "a seek is not a Play press");
        assert_eq!(
            t.command(Seek(5000), true, 0, true),
            Some(Action::Seek {
                epoch: 3,
                pos: 1000
            })
        );
        t.command(Pause, true, 999, false);
        t.command(ReturnToStart, true, 0, true);
        assert_eq!(t.playhead(), 0);
    }

    #[test]
    fn end_and_guards() {
        let mut t = doc(1000);
        assert_eq!(t.command(Play, false, 0, true), None, "no output / no doc");
        t.command(Play, true, 0, true);
        assert!(t.on_ended(1, 1000));
        assert_eq!(t.playhead(), 1000);
        // Play at the end starts over.
        assert_eq!(
            t.command(Play, true, 0, true),
            Some(Action::Start {
                epoch: 2,
                pos: 0,
                reset: true
            })
        );
        assert_eq!(doc(0).command(Play, true, 0, true), None, "empty document");
    }

    /// H-37/H-80 (SPEC-003 §2.1/§3, Amendment 3): the loop region is the selection while loop is
    /// on, when it's at least the minimum length; otherwise (no selection, or one shorter than
    /// the minimum) the whole document. `None` only while loop is off, or with no document. A
    /// document change clamps (not clears) the selection.
    #[test]
    fn loop_region_follows_the_toggle_and_the_selection() {
        let mut t = doc(10_000);
        t.set_selection(Some((6_000, 2_000)));
        assert_eq!(t.loop_region(480), None, "loop off");
        t.set_loop_enabled(true);
        assert_eq!(t.loop_region(480), Some((2_000, 6_000)));
        assert_eq!(
            t.loop_region(4_001),
            Some((0, 10_000)),
            "shorter than the minimum: falls back to the whole document"
        );
        t.set_doc(5_000);
        assert_eq!(
            t.loop_region(480),
            Some((2_000, 5_000)),
            "clamped to the new length"
        );
        t.set_doc(1_000);
        assert_eq!(
            t.loop_region(1),
            Some((0, 1_000)),
            "the selection fell outside the document: falls back to the whole document"
        );
        t.set_selection(None);
        assert_eq!(
            t.loop_region(1),
            Some((0, 1_000)),
            "no selection: the whole document (H-80)"
        );
        t.set_loop_enabled(false);
        assert!(!t.loop_enabled());
        assert_eq!(t.loop_region(1), None, "loop off: inert again");
        t.set_loop_enabled(true);
        t.set_doc(0);
        assert_eq!(t.loop_region(1), None, "no document: nothing to loop");
    }

    /// H-37: the end of a pass after loop off stops at its position (Pause semantics).
    #[test]
    fn an_end_before_the_document_end_keeps_its_position() {
        let mut t = doc(10_000);
        t.command(Play, true, 0, true);
        assert!(t.on_ended(1, 6_000));
        assert_eq!(t.playhead(), 6_000);
        assert!(!t.playing());
    }
}
