//! Device-lost / recovery state machine (SPEC-001 §2.3–§2.4, SPEC-002 §2.6, ADR-002 §7).
//!
//! Pure and synchronous: the control thread feeds it [`DeviceEvent`]s (stream flags seen on its
//! 16 ms tick, device-poll presence, open results) together with the current [`Activity`], and
//! executes the returned [`DeviceAction`]s. One state per direction.
//!
//! Rules:
//! - Loss is detected by the stream's `DEVICE_LOST` flag (immediate) or by the poll no longer
//!   reporting the device. `BACKEND_ERROR` alone is reported, not treated as loss.
//! - Output loss: close the output stream, stop playback and monitoring. **A recording keeps
//!   going** (owner exception, SPEC-001 §2.4 / SPEC-002 AC-16).
//! - Input loss while the input is in use (open): close it, stop recording (the take is kept),
//!   monitoring and playback.
//! - A failed (re)open counts as loss (SPEC-001 §2.2).
//! - Recovery: when the poll reports the device again after it was seen absent — or, once, when a
//!   flag-only loss finds the device still listed (transient error) — the stream is reopened.
//!   A manual rescan allows another retry. Nothing ever auto-resumes playback or recording.

use crate::backend::{Direction, flags};
use crate::devices::DeviceNotice;

/// What the engine is doing when an event arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// Playback is running.
    pub playing: bool,
    /// A take is being recorded.
    pub recording: bool,
    /// Monitoring is audible (input armed or recording, mode ≠ off).
    pub monitoring: bool,
}

/// Inputs to the state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceEvent {
    /// A device (or "none") was configured for `dir`; any old stream was closed by the caller.
    Configured {
        /// Direction.
        dir: Direction,
        /// Device name, `None` = no device.
        device: Option<String>,
    },
    /// A stream opened. `fallback`: a rate/buffer fallback was applied (amber status).
    Opened {
        /// Direction.
        dir: Direction,
        /// A fallback was applied.
        fallback: bool,
    },
    /// Opening a stream failed.
    OpenFailed {
        /// Direction.
        dir: Direction,
    },
    /// The caller closed a healthy stream on purpose (e.g. input disarmed).
    Closed {
        /// Direction.
        dir: Direction,
    },
    /// Flag bits taken from the stream's status on a control tick.
    Flags {
        /// Direction.
        dir: Direction,
        /// [`flags`] bits.
        bits: u32,
    },
    /// Device-poll pass: is the configured device of `dir` reported?
    Poll {
        /// Direction.
        dir: Direction,
        /// Reported by the host.
        present: bool,
    },
    /// Manual rescan requested: lost devices get another reopen attempt.
    Rescan,
}

/// What the control thread must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceAction {
    /// Stop playback with the short fade (engine-initiated stop = Pause semantics, D-018). Must
    /// not stop a recording.
    StopPlayback,
    /// Stop the recording and keep (finalize and commit) the take.
    StopRecording,
    /// Stop monitoring.
    StopMonitoring,
    /// Drop the stream handle of `dir`.
    CloseStream(Direction),
    /// Reopen `dir` with the previously configured settings (re-applying fallback rules), then
    /// report [`DeviceEvent::Opened`] or [`DeviceEvent::OpenFailed`].
    ReopenStream(Direction),
    /// Show a notice/banner.
    Notice(DeviceNotice),
}

/// Per-direction link state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// No device configured.
    NotSelected,
    /// Configured and present; no stream open (e.g. input not armed).
    Idle,
    /// Stream open and healthy.
    Open,
    /// A reopen was requested; waiting for its result.
    Reopening,
    /// Lost; waiting for the device to come back.
    Lost {
        /// The stream must be reopened on recovery (output always; input if it was in use).
        reopen: bool,
        /// The poll has reported the device absent since the loss.
        seen_absent: bool,
        /// A reopen was already attempted while the device stayed listed.
        retried: bool,
    },
}

/// Status dot for the UI (SPEC-001 §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    /// Grey: no device selected.
    NotSelected,
    /// Green.
    Healthy,
    /// Amber: running with a rate/buffer fallback.
    Fallback,
    /// Red: lost.
    Lost,
}

#[derive(Clone, Debug)]
struct Side {
    dir: Direction,
    device: Option<String>,
    state: LinkState,
    fallback: bool,
    error_reported: bool,
}

impl Side {
    fn new(dir: Direction) -> Self {
        Self {
            dir,
            device: None,
            state: LinkState::NotSelected,
            fallback: false,
            error_reported: false,
        }
    }

    fn name(&self) -> String {
        self.device.clone().unwrap_or_default()
    }

    /// Handles a loss. `in_use`: the stream was open (or being opened for use).
    fn lose(
        &mut self,
        act: Activity,
        stream_open: bool,
        in_use: bool,
        out: &mut Vec<DeviceAction>,
    ) {
        if stream_open {
            out.push(DeviceAction::CloseStream(self.dir));
        }
        let mut playback_stopped = false;
        let mut recording_stopped = false;
        let mut recording_continues = false;
        if in_use {
            match self.dir {
                Direction::Output => {
                    recording_continues = act.recording;
                }
                Direction::Input => {
                    if act.recording {
                        out.push(DeviceAction::StopRecording);
                        recording_stopped = true;
                    }
                }
            }
            if act.playing {
                out.push(DeviceAction::StopPlayback);
                playback_stopped = true;
            }
            if act.monitoring {
                out.push(DeviceAction::StopMonitoring);
            }
        }
        out.push(DeviceAction::Notice(DeviceNotice::DeviceLost {
            direction: self.dir,
            device: self.name(),
            recording_stopped,
            playback_stopped,
            recording_continues,
        }));
        self.state = LinkState::Lost {
            reopen: self.dir == Direction::Output || in_use,
            seen_absent: false,
            retried: false,
        };
    }

    fn handle(&mut self, ev: &DeviceEvent, act: Activity, out: &mut Vec<DeviceAction>) {
        match *ev {
            DeviceEvent::Configured { ref device, .. } => {
                self.device = device.clone();
                self.fallback = false;
                self.error_reported = false;
                self.state = if device.is_some() {
                    LinkState::Idle
                } else {
                    LinkState::NotSelected
                };
            }
            DeviceEvent::Opened { fallback, .. } => {
                if self.device.is_none() {
                    return;
                }
                let recovered = matches!(self.state, LinkState::Reopening | LinkState::Lost { .. });
                self.state = LinkState::Open;
                self.fallback = fallback;
                self.error_reported = false;
                if recovered {
                    out.push(DeviceAction::Notice(DeviceNotice::DeviceReconnected {
                        direction: self.dir,
                        device: self.name(),
                    }));
                }
            }
            DeviceEvent::OpenFailed { .. } => match self.state {
                LinkState::NotSelected => {}
                LinkState::Reopening => {
                    self.state = LinkState::Lost {
                        reopen: true,
                        seen_absent: false,
                        retried: true,
                    };
                }
                LinkState::Lost { .. } => {}
                LinkState::Idle | LinkState::Open => {
                    let stream_open = self.state == LinkState::Open;
                    self.lose(act, stream_open, true, out);
                    if let LinkState::Lost { retried, .. } = &mut self.state {
                        *retried = true;
                    }
                }
            },
            DeviceEvent::Closed { .. } => match &mut self.state {
                LinkState::Open => self.state = LinkState::Idle,
                LinkState::Lost { reopen, .. } if self.dir == Direction::Input => *reopen = false,
                _ => {}
            },
            DeviceEvent::Flags { bits, .. } => {
                if bits & flags::DEVICE_LOST != 0 && self.state == LinkState::Open {
                    self.lose(act, true, true, out);
                } else if bits & flags::BACKEND_ERROR != 0
                    && self.state == LinkState::Open
                    && !self.error_reported
                {
                    self.error_reported = true;
                    out.push(DeviceAction::Notice(DeviceNotice::BackendError {
                        direction: self.dir,
                        device: self.name(),
                    }));
                }
            }
            DeviceEvent::Poll { present, .. } => match self.state {
                LinkState::Idle | LinkState::Open if !present => {
                    let open = self.state == LinkState::Open;
                    let in_use = open || self.dir == Direction::Output;
                    self.lose(act, open, in_use, out);
                    if let LinkState::Lost { seen_absent, .. } = &mut self.state {
                        *seen_absent = true;
                    }
                }
                LinkState::Lost {
                    reopen,
                    seen_absent,
                    retried,
                } => {
                    if !present {
                        self.state = LinkState::Lost {
                            reopen,
                            seen_absent: true,
                            retried,
                        };
                    } else if seen_absent || !retried {
                        if reopen {
                            self.state = LinkState::Reopening;
                            out.push(DeviceAction::ReopenStream(self.dir));
                        } else {
                            self.state = LinkState::Idle;
                            out.push(DeviceAction::Notice(DeviceNotice::DeviceReconnected {
                                direction: self.dir,
                                device: self.name(),
                            }));
                        }
                    }
                }
                _ => {}
            },
            DeviceEvent::Rescan => {
                if let LinkState::Lost { retried, .. } = &mut self.state {
                    *retried = false;
                }
            }
        }
    }

    fn status(&self) -> DeviceStatus {
        match self.state {
            LinkState::NotSelected => DeviceStatus::NotSelected,
            LinkState::Lost { .. } | LinkState::Reopening => DeviceStatus::Lost,
            LinkState::Idle | LinkState::Open if self.fallback => DeviceStatus::Fallback,
            LinkState::Idle | LinkState::Open => DeviceStatus::Healthy,
        }
    }
}

/// Input + output device state.
#[derive(Clone, Debug)]
pub struct DeviceStateMachine {
    input: Side,
    output: Side,
}

impl Default for DeviceStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceStateMachine {
    /// Both directions unselected.
    pub fn new() -> Self {
        Self {
            input: Side::new(Direction::Input),
            output: Side::new(Direction::Output),
        }
    }

    fn side_mut(&mut self, dir: Direction) -> &mut Side {
        match dir {
            Direction::Input => &mut self.input,
            Direction::Output => &mut self.output,
        }
    }

    fn side(&self, dir: Direction) -> &Side {
        match dir {
            Direction::Input => &self.input,
            Direction::Output => &self.output,
        }
    }

    /// Feeds one event; returns the actions to execute, in order.
    pub fn handle(&mut self, ev: DeviceEvent, act: Activity) -> Vec<DeviceAction> {
        let mut out = Vec::new();
        match &ev {
            DeviceEvent::Configured { dir, .. }
            | DeviceEvent::Opened { dir, .. }
            | DeviceEvent::OpenFailed { dir }
            | DeviceEvent::Closed { dir }
            | DeviceEvent::Flags { dir, .. }
            | DeviceEvent::Poll { dir, .. } => self.side_mut(*dir).handle(&ev, act, &mut out),
            DeviceEvent::Rescan => {
                self.input.handle(&ev, act, &mut out);
                self.output.handle(&ev, act, &mut out);
            }
        }
        out
    }

    /// Link state of `dir`.
    pub fn state(&self, dir: Direction) -> LinkState {
        self.side(dir).state
    }

    /// UI status of `dir`.
    pub fn status(&self, dir: Direction) -> DeviceStatus {
        self.side(dir).status()
    }

    /// Configured device name of `dir`.
    pub fn device(&self, dir: Direction) -> Option<&str> {
        self.side(dir).device.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::{Input, Output};

    const IDLE: Activity = Activity {
        playing: false,
        recording: false,
        monitoring: false,
    };
    const PLAYING: Activity = Activity {
        playing: true,
        recording: false,
        monitoring: false,
    };
    const RECORDING_DRY: Activity = Activity {
        playing: false,
        recording: true,
        monitoring: true,
    };

    fn open(m: &mut DeviceStateMachine, dir: Direction, name: &str) {
        assert!(
            m.handle(
                DeviceEvent::Configured {
                    dir,
                    device: Some(name.into())
                },
                IDLE
            )
            .is_empty()
        );
        assert!(
            m.handle(
                DeviceEvent::Opened {
                    dir,
                    fallback: false
                },
                IDLE
            )
            .is_empty()
        );
        assert_eq!(m.state(dir), LinkState::Open);
    }

    fn lost_notice(
        dir: Direction,
        dev: &str,
        rec_stop: bool,
        play_stop: bool,
        rec_cont: bool,
    ) -> DeviceAction {
        DeviceAction::Notice(DeviceNotice::DeviceLost {
            direction: dir,
            device: dev.into(),
            recording_stopped: rec_stop,
            playback_stopped: play_stop,
            recording_continues: rec_cont,
        })
    }

    /// SPEC-001 AC-6 (state machine): output lost mid-playback stops the transport at once.
    #[test]
    fn output_loss_during_playback_stops_playback() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Output, "DAC");
        let a = m.handle(
            DeviceEvent::Flags {
                dir: Output,
                bits: flags::DEVICE_LOST,
            },
            PLAYING,
        );
        assert_eq!(
            a,
            vec![
                DeviceAction::CloseStream(Output),
                DeviceAction::StopPlayback,
                lost_notice(Output, "DAC", false, true, false),
            ]
        );
        assert_eq!(m.status(Output), DeviceStatus::Lost);
        // A second flag report does nothing.
        assert!(
            m.handle(
                DeviceEvent::Flags {
                    dir: Output,
                    bits: flags::DEVICE_LOST
                },
                IDLE
            )
            .is_empty()
        );
    }

    /// SPEC-001 AC-7 (state machine): replug → reopen → reconnected; never resumes playback.
    #[test]
    fn replug_reopens_and_does_not_resume() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Output, "DAC");
        m.handle(
            DeviceEvent::Flags {
                dir: Output,
                bits: flags::DEVICE_LOST,
            },
            PLAYING,
        );
        assert!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: false
                },
                IDLE
            )
            .is_empty()
        );
        assert!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: false
                },
                IDLE
            )
            .is_empty()
        );
        assert_eq!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            ),
            vec![DeviceAction::ReopenStream(Output)]
        );
        assert_eq!(m.state(Output), LinkState::Reopening);
        let a = m.handle(
            DeviceEvent::Opened {
                dir: Output,
                fallback: false,
            },
            IDLE,
        );
        assert_eq!(
            a,
            vec![DeviceAction::Notice(DeviceNotice::DeviceReconnected {
                direction: Output,
                device: "DAC".into()
            })]
        );
        assert_eq!(m.status(Output), DeviceStatus::Healthy);
    }

    /// SPEC-002 AC-16 (state machine): output-only loss while recording keeps recording.
    #[test]
    fn output_loss_while_recording_keeps_recording() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Input, "Mic");
        open(&mut m, Output, "Phones");
        let a = m.handle(
            DeviceEvent::Flags {
                dir: Output,
                bits: flags::DEVICE_LOST,
            },
            RECORDING_DRY,
        );
        assert_eq!(
            a,
            vec![
                DeviceAction::CloseStream(Output),
                DeviceAction::StopMonitoring,
                lost_notice(Output, "Phones", false, false, true),
            ]
        );
        assert!(!a.contains(&DeviceAction::StopRecording));
        assert_eq!(m.state(Input), LinkState::Open);
    }

    /// SPEC-002 AC-14 (state machine): input loss while recording stops and keeps the take.
    #[test]
    fn input_loss_while_recording_stops_recording() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Input, "Mic");
        open(&mut m, Output, "Phones");
        let a = m.handle(
            DeviceEvent::Flags {
                dir: Input,
                bits: flags::DEVICE_LOST,
            },
            RECORDING_DRY,
        );
        assert_eq!(
            a,
            vec![
                DeviceAction::CloseStream(Input),
                DeviceAction::StopRecording,
                DeviceAction::StopMonitoring,
                lost_notice(Input, "Mic", true, false, false),
            ]
        );
        assert_eq!(m.state(Output), LinkState::Open);
        // Replug reopens the input (it was in use) but never restarts the recording.
        m.handle(
            DeviceEvent::Poll {
                dir: Input,
                present: false,
            },
            IDLE,
        );
        let a = m.handle(
            DeviceEvent::Poll {
                dir: Input,
                present: true,
            },
            IDLE,
        );
        assert_eq!(a, vec![DeviceAction::ReopenStream(Input)]);
    }

    #[test]
    fn idle_input_lost_by_poll_recovers_without_reopen() {
        let mut m = DeviceStateMachine::new();
        m.handle(
            DeviceEvent::Configured {
                dir: Input,
                device: Some("USB Mic".into()),
            },
            IDLE,
        );
        let a = m.handle(
            DeviceEvent::Poll {
                dir: Input,
                present: false,
            },
            PLAYING,
        );
        assert_eq!(a, vec![lost_notice(Input, "USB Mic", false, false, false)]);
        assert_eq!(m.status(Input), DeviceStatus::Lost);
        let a = m.handle(
            DeviceEvent::Poll {
                dir: Input,
                present: true,
            },
            IDLE,
        );
        assert_eq!(
            a,
            vec![DeviceAction::Notice(DeviceNotice::DeviceReconnected {
                direction: Input,
                device: "USB Mic".into()
            })]
        );
        assert_eq!(m.state(Input), LinkState::Idle);
    }

    #[test]
    fn transient_flag_loss_retries_once_then_waits_for_replug_or_rescan() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Output, "DAC");
        m.handle(
            DeviceEvent::Flags {
                dir: Output,
                bits: flags::DEVICE_LOST,
            },
            IDLE,
        );
        // Still listed: one retry.
        assert_eq!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            ),
            vec![DeviceAction::ReopenStream(Output)]
        );
        assert!(
            m.handle(DeviceEvent::OpenFailed { dir: Output }, IDLE)
                .is_empty()
        );
        // Still listed, already retried: wait.
        assert!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            )
            .is_empty()
        );
        // Manual rescan grants another attempt.
        m.handle(DeviceEvent::Rescan, IDLE);
        assert_eq!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            ),
            vec![DeviceAction::ReopenStream(Output)]
        );
    }

    /// SPEC-001 §2.2: a stream that fails to open is treated as lost.
    #[test]
    fn open_failure_is_loss() {
        let mut m = DeviceStateMachine::new();
        m.handle(
            DeviceEvent::Configured {
                dir: Output,
                device: Some("DAC".into()),
            },
            IDLE,
        );
        let a = m.handle(DeviceEvent::OpenFailed { dir: Output }, IDLE);
        assert_eq!(a, vec![lost_notice(Output, "DAC", false, false, false)]);
        assert_eq!(m.status(Output), DeviceStatus::Lost);
        // Already retried (the failed open): waits until seen absent → present.
        assert!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            )
            .is_empty()
        );
        m.handle(
            DeviceEvent::Poll {
                dir: Output,
                present: false,
            },
            IDLE,
        );
        assert_eq!(
            m.handle(
                DeviceEvent::Poll {
                    dir: Output,
                    present: true
                },
                IDLE
            ),
            vec![DeviceAction::ReopenStream(Output)]
        );
    }

    #[test]
    fn backend_error_is_reported_once_and_is_not_loss() {
        let mut m = DeviceStateMachine::new();
        open(&mut m, Output, "DAC");
        let ev = DeviceEvent::Flags {
            dir: Output,
            bits: flags::BACKEND_ERROR,
        };
        assert_eq!(m.handle(ev.clone(), PLAYING).len(), 1);
        assert!(m.handle(ev, PLAYING).is_empty());
        assert_eq!(m.state(Output), LinkState::Open);
    }

    #[test]
    fn statuses() {
        let mut m = DeviceStateMachine::new();
        assert_eq!(m.status(Output), DeviceStatus::NotSelected);
        m.handle(
            DeviceEvent::Configured {
                dir: Output,
                device: Some("DAC".into()),
            },
            IDLE,
        );
        m.handle(
            DeviceEvent::Opened {
                dir: Output,
                fallback: true,
            },
            IDLE,
        );
        assert_eq!(m.status(Output), DeviceStatus::Fallback);
        m.handle(DeviceEvent::Closed { dir: Output }, IDLE);
        assert_eq!(m.state(Output), LinkState::Idle);
        m.handle(
            DeviceEvent::Configured {
                dir: Output,
                device: None,
            },
            IDLE,
        );
        assert_eq!(m.status(Output), DeviceStatus::NotSelected);
        assert_eq!(m.device(Output), None);
    }
}
