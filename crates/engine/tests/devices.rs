//! SPEC-001 AC-1…AC-8, input-channel deinterleaving and SPEC-002 AC-16 (state-machine part),
//! all through the fake backend. Every callback runs under the allocation checker.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use vox_engine::backend::fake::{
    CallbackSizes, FakeBackend, FakeDevice, FakeDirection, FakeEvent, Signal,
};
use vox_engine::backend::{
    Backend, BackendError, BufferRequest, DeviceKey, Direction, Enumerate, HostId, InputTimestamp,
    OutputTimestamp, StreamHandle, flags, frames_to_ns,
};
use vox_engine::device_state::{
    Activity, DeviceAction, DeviceEvent, DeviceStateMachine, DeviceStatus, LinkState,
};
use vox_engine::devices::{
    DeviceList, DeviceNotice, DevicePollThread, DeviceWatcher, InputChannel, MonoInput, PollEvent,
    resolve_devices,
};
use vox_engine::prefs::DevicePrefs;
use vox_module_api::test_util::{alloc_checks_active, no_alloc};

vox_module_api::install_test_allocator!();

const MS: u64 = 1_000_000;
const SEC: u64 = 1_000_000_000;
const TICK: u64 = 16 * MS;
const HOST: HostId = HostId::PipeWire;

fn guarded(fake: &FakeBackend) {
    assert!(alloc_checks_active());
    fake.set_rt_guard(|f: &mut dyn FnMut()| match no_alloc(f) {
        Ok(()) => 0,
        Err(n) => n,
    });
}

fn out_dir() -> FakeDirection {
    FakeDirection::new(2, &[44_100, 48_000], 48_000)
        .buffer_range(Some((64, 2048)))
        .callback_sizes(CallbackSizes::FULL_RANDOM)
        .latency_ns(7 * MS)
}

fn in_dir(signals: Vec<Signal>) -> FakeDirection {
    FakeDirection::new(2, &[44_100, 48_000], 48_000)
        .callback_sizes(CallbackSizes::FULL_RANDOM)
        .latency_ns(5 * MS)
        .signals(signals)
}

/// PipeWire host with speakers, a 2-in USB interface and a headset.
fn studio(seed: u64) -> FakeBackend {
    let fake = FakeBackend::new(seed);
    fake.add_host(HostId::Alsa, true);
    fake.plug(HOST, FakeDevice::new("Speakers").with_output(out_dir()));
    fake.plug(
        HOST,
        FakeDevice::new("USB Interface").with_input(in_dir(vec![
            Signal::Sine {
                freq_hz: 997.0,
                amplitude: 0.25,
            },
            Signal::Silence,
        ])),
    );
    fake.plug(
        HOST,
        FakeDevice::new("Headset")
            .with_input(FakeDirection::new(1, &[48_000], 48_000))
            .with_output(FakeDirection::new(2, &[48_000], 48_000)),
    );
    guarded(&fake);
    fake
}

fn key(name: &str) -> DeviceKey {
    DeviceKey::new(HOST, name)
}

fn silent_output() -> Box<dyn vox_engine::OutputCallback> {
    Box::new(|d: &mut [f32], _: usize, _: OutputTimestamp| d.fill(0.0))
}

fn names(list: &DeviceList, dir: Direction) -> Vec<String> {
    list.snapshot
        .devices_for(dir)
        .map(|d| d.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------

/// AC-1: every present device is listed once, nothing absent is listed.
#[test]
fn ac1_enumeration_lists_each_present_device_once() {
    let fake = studio(1);
    // Re-plugging a name replaces it instead of duplicating it.
    fake.plug(HOST, FakeDevice::new("Speakers").with_output(out_dir()));
    let mut w = DeviceWatcher::new(HOST);
    let mut list = DeviceList::default();
    list.apply(&w.poll(&fake, Enumerate::Fresh).unwrap().unwrap());
    assert_eq!(names(&list, Direction::Output), ["Speakers", "Headset"]);
    assert_eq!(names(&list, Direction::Input), ["USB Interface", "Headset"]);
    assert_eq!(list.snapshot.default_output.as_deref(), Some("Speakers"));
    assert_eq!(
        list.snapshot.default_input.as_deref(),
        Some("USB Interface")
    );

    fake.unplug(&key("Headset")).unwrap();
    list.apply(&w.poll(&fake, Enumerate::Cached).unwrap().unwrap());
    assert_eq!(names(&list, Direction::Output), ["Speakers"]);
    assert_eq!(names(&list, Direction::Input), ["USB Interface"]);
    assert!(w.poll(&fake, Enumerate::Cached).unwrap().is_none());
}

/// AC-1 through the real poll thread: the first pass arrives right after start.
#[test]
fn ac1_poll_thread_reports_first_pass_immediately() {
    let fake = studio(2);
    let (tx, rx) = mpsc::channel();
    let _poll = DevicePollThread::spawn(Arc::new(fake), HOST, Duration::from_secs(1), move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let hosts = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        hosts,
        PollEvent::Hosts {
            available: vec![HostId::PipeWire, HostId::Alsa],
            default: Some(HostId::PipeWire),
        }
    );
    match rx.recv_timeout(Duration::from_secs(1)).unwrap() {
        PollEvent::Devices(d) => {
            assert!(d.full);
            assert_eq!(d.added.len(), 3);
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// AC-2: only the selected channel reaches the (meter) consumer; switching needs no reopen and
/// takes effect at the next callback.
#[test]
fn ac2_input_channel_selection_by_deinterleaving() {
    for tone_on in [0usize, 1] {
        let fake = FakeBackend::new(3);
        let mut signals = vec![Signal::Silence, Signal::Silence];
        signals[tone_on] = Signal::Sine {
            freq_hz: 997.0,
            amplitude: 0.25,
        };
        fake.plug(HOST, FakeDevice::new("2-in").with_input(in_dir(signals)));
        guarded(&fake);

        let channel = InputChannel::new(1);
        let peak_bits = Arc::new(AtomicU32::new(0));
        let calls = Arc::new(AtomicU64::new(0));
        let (p, c) = (peak_bits.clone(), calls.clone());
        let meter = MonoInput::new(channel.clone(), 48_000, 4096, move |x: &[f32], _| {
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            p.store(peak.to_bits(), Ordering::Relaxed);
            c.fetch_add(1, Ordering::Relaxed);
        });
        let req = vox_engine::StreamRequest {
            device: key("2-in"),
            sample_rate_hz: 48_000,
            buffer: BufferRequest::Auto,
            channels: None,
        };
        let stream = fake.open_input(&req, Box::new(meter)).unwrap();
        assert_eq!(stream.info().channels, 2);
        let peak = || f32::from_bits(peak_bits.load(Ordering::Relaxed));

        for selected in [1u16, 2, 1, 2] {
            channel.set(selected);
            let before = calls.load(Ordering::Relaxed);
            // One callback is enough: the next buffer already reflects the new channel.
            while calls.load(Ordering::Relaxed) == before {
                fake.advance_by(MS);
            }
            fake.advance_by(200 * MS);
            let expect_tone = usize::from(selected - 1) == tone_on;
            if expect_tone {
                assert!((peak() - 0.25).abs() < 1e-3, "sel {selected}: {}", peak());
            } else {
                assert_eq!(peak().to_bits(), 0, "sel {selected}: {}", peak());
            }
        }
        assert_eq!(fake.streams().len(), 1, "no reopen");
        assert_eq!(fake.rt_violations(), 0);
    }
}

/// AC-3: unsupported 192 kHz → device default, notice naming both rates; audio starts.
#[test]
fn ac3_sample_rate_fallback() {
    let fake = FakeBackend::new(4);
    fake.plug(
        HOST,
        FakeDevice::new("DAC").with_output(FakeDirection::new(2, &[44_100, 48_000], 44_100)),
    );
    let snap = fake.enumerate(HOST, Enumerate::Fresh).unwrap();
    let prefs = DevicePrefs {
        sample_rate_hz: Some(192_000),
        ..DevicePrefs::default()
    };
    let res = resolve_devices(&prefs, HOST, &snap);
    assert_eq!(res.sample_rate_hz, Some(44_100));
    assert_eq!(
        res.notices,
        vec![DeviceNotice::RateFallback {
            device: "DAC".into(),
            requested_hz: 192_000,
            applied_hz: 44_100
        }]
    );
    let s = fake
        .open_output(&res.output_request().unwrap(), silent_output())
        .unwrap();
    assert_eq!(s.info().sample_rate_hz, 44_100);
    fake.advance_by(100 * MS);
    assert!(s.status().callback_count() > 0);
    assert_eq!(prefs.sample_rate_hz, Some(192_000), "intent kept");
}

/// AC-4: buffer range [128, 1024], 2048 selected → 1024 with a notice.
#[test]
fn ac4_buffer_size_fallback() {
    let fake = FakeBackend::new(5);
    fake.plug(
        HOST,
        FakeDevice::new("DAC").with_output(
            FakeDirection::new(2, &[48_000], 48_000)
                .buffer_range(Some((128, 1024)))
                .record_output(),
        ),
    );
    let snap = fake.enumerate(HOST, Enumerate::Fresh).unwrap();
    let prefs = DevicePrefs {
        buffer_size: BufferRequest::Frames(2048),
        ..DevicePrefs::default()
    };
    let res = resolve_devices(&prefs, HOST, &snap);
    assert_eq!(res.buffer, BufferRequest::Frames(1024));
    assert_eq!(
        res.notices,
        vec![DeviceNotice::BufferFallback {
            device: "DAC".into(),
            requested_frames: 2048,
            applied_frames: 1024
        }]
    );
    let s = fake
        .open_output(&res.output_request().unwrap(), silent_output())
        .unwrap();
    assert_eq!(s.info().nominal_frames, Some(1024));
    fake.advance_by(SEC);
    let rec = fake.recorded_output(s.info().id).unwrap();
    assert!(rec.blocks.len() > 40);
    assert!(rec.blocks.iter().all(|b| b.frames == 1024));
    // Opening an out-of-range size directly is refused (the resolver prevents it).
    let mut bad = res.output_request().unwrap();
    bad.buffer = BufferRequest::Frames(2048);
    assert!(matches!(
        fake.open_output(&bad, silent_output()),
        Err(BackendError::UnsupportedConfig(_))
    ));
}

/// AC-5 (simulated clock): hot-plug changes reach the list within 2 s (poll 1 s + tick).
#[test]
fn ac5_hotplug_reaches_the_list_within_two_seconds() {
    let fake = studio(6);
    let events = [
        (2_300 * MS, true),
        (5_700 * MS, false),
        (7_999 * MS, true),
        (9_001 * MS, false),
    ];
    for &(at, plug) in &events {
        let ev = if plug {
            FakeEvent::Plug(
                HOST,
                FakeDevice::new("USB DAC").with_output(FakeDirection::new(2, &[48_000], 48_000)),
            )
        } else {
            FakeEvent::Unplug(key("USB DAC"))
        };
        fake.schedule(at, ev);
    }
    let mut w = DeviceWatcher::new(HOST);
    let mut list = DeviceList::default();
    // Control ticks every 16 ms; the poll runs on the first tick after each whole second, and
    // its diff is visible to the control thread on the following tick.
    let mut seen = Vec::new();
    let mut prev = 0;
    for k in 0..=(12 * SEC / TICK) {
        let t = k * TICK;
        fake.advance_to(t);
        if k == 0 || t / SEC != prev / SEC {
            if let Some(d) = w.poll(&fake, Enumerate::Cached).unwrap() {
                list.apply(&d);
            }
            seen.push((t + TICK, list.is_present("USB DAC", Direction::Output)));
        }
        prev = t;
    }
    for &(at, plugged) in &events {
        let when = seen
            .iter()
            .find(|(ts, present)| *ts > at && *present == plugged)
            .map(|(ts, _)| *ts)
            .expect("change observed");
        assert!(when - at <= 2 * SEC, "event at {at} seen at {when}");
    }
}

/// AC-5 through the real poll thread (short interval) and manual rescan.
#[test]
fn ac5_poll_thread_posts_diffs_and_rescans() {
    let fake = studio(7);
    let (tx, rx) = mpsc::channel();
    let poll = DevicePollThread::spawn(
        Arc::new(fake.clone()),
        HOST,
        Duration::from_millis(20),
        move |e| {
            let _ = tx.send(e);
        },
    )
    .unwrap();
    let mut list = DeviceList::default();
    let mut wait_for = |pred: &dyn Fn(&DeviceList) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !pred(&list) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left).expect("poll event") {
                PollEvent::Devices(d) => list.apply(&d),
                PollEvent::Hosts { .. } => {}
                PollEvent::Error { error, .. } => panic!("{error}"),
            }
        }
    };
    wait_for(&|l| l.is_present("Speakers", Direction::Output));
    fake.plug(
        HOST,
        FakeDevice::new("USB DAC").with_output(FakeDirection::new(2, &[48_000], 48_000)),
    );
    wait_for(&|l| l.is_present("USB DAC", Direction::Output));
    fake.unplug(&key("USB DAC"));
    wait_for(&|l| !l.is_present("USB DAC", Direction::Output));
    poll.set_host(HostId::Alsa);
    wait_for(&|l| l.host == Some(HostId::Alsa) && l.snapshot.devices.is_empty());
    poll.rescan();
    drop(poll); // joins the thread
}

// ---------------------------------------------------------------------------------------------
// A minimal control loop around the device state machine (T-105 builds the real one).

struct Harness {
    fake: FakeBackend,
    prefs: DevicePrefs,
    machine: DeviceStateMachine,
    watcher: DeviceWatcher,
    list: DeviceList,
    output: Option<StreamHandle>,
    input: Option<StreamHandle>,
    activity: Activity,
    log: Vec<(u64, DeviceAction)>,
    input_gaps: Arc<AtomicU64>,
    input_frames: Arc<AtomicU64>,
}

impl Harness {
    fn new(fake: FakeBackend, prefs: DevicePrefs) -> Self {
        let mut h = Self {
            fake,
            prefs,
            machine: DeviceStateMachine::new(),
            watcher: DeviceWatcher::new(HOST),
            list: DeviceList::default(),
            output: None,
            input: None,
            activity: Activity::default(),
            log: Vec::new(),
            input_gaps: Arc::new(AtomicU64::new(0)),
            input_frames: Arc::new(AtomicU64::new(0)),
        };
        h.poll_devices();
        let res = resolve_devices(&h.prefs, HOST, &h.list.snapshot);
        let out = res.output.as_ref().map(|k| k.name.clone());
        let inp = res.input.as_ref().map(|i| i.device.name.clone());
        h.handle(DeviceEvent::Configured {
            dir: Direction::Output,
            device: out,
        });
        h.handle(DeviceEvent::Configured {
            dir: Direction::Input,
            device: inp,
        });
        h.open(Direction::Output);
        h
    }

    fn handle(&mut self, ev: DeviceEvent) {
        let mut queue: Vec<DeviceAction> = self.machine.handle(ev, self.activity);
        queue.reverse();
        while let Some(a) = queue.pop() {
            self.log.push((self.fake.now_ns(), a.clone()));
            let follow = match a {
                DeviceAction::StopPlayback => {
                    self.activity.playing = false;
                    None
                }
                DeviceAction::StopRecording => {
                    self.activity.recording = false;
                    None
                }
                DeviceAction::StopMonitoring => {
                    self.activity.monitoring = false;
                    None
                }
                DeviceAction::CloseStream(Direction::Output) => {
                    self.output = None;
                    None
                }
                DeviceAction::CloseStream(Direction::Input) => {
                    self.input = None;
                    None
                }
                DeviceAction::ReopenStream(dir) => Some(self.try_open(dir)),
                DeviceAction::Notice(_) => None,
            };
            if let Some(ev) = follow {
                let mut more = self.machine.handle(ev, self.activity);
                more.reverse();
                queue.splice(0..0, more);
            }
        }
    }

    fn try_open(&mut self, dir: Direction) -> DeviceEvent {
        let res = resolve_devices(&self.prefs, HOST, &self.list.snapshot);
        let fallback = res.notices.iter().any(|n| {
            matches!(
                n,
                DeviceNotice::RateFallback { .. } | DeviceNotice::BufferFallback { .. }
            )
        });
        let opened = match dir {
            Direction::Output => res.output_request().and_then(|r| {
                let cb = |d: &mut [f32], _: usize, _: OutputTimestamp| d.fill(0.1);
                self.fake.open_output(&r, Box::new(cb)).ok()
            }),
            Direction::Input => res.input_request(48_000).and_then(|r| {
                let (gaps, frames) = (self.input_gaps.clone(), self.input_frames.clone());
                let expected = Arc::new(AtomicU64::new(0));
                let cb = MonoInput::new(
                    InputChannel::new(1),
                    48_000,
                    4096,
                    move |x: &[f32], ts: InputTimestamp| {
                        let exp = expected.load(Ordering::Relaxed);
                        if exp != 0 && ts.capture_ns.abs_diff(exp) > 2 {
                            gaps.fetch_add(1, Ordering::Relaxed);
                        }
                        expected.store(
                            ts.capture_ns + frames_to_ns(x.len() as u64, 48_000),
                            Ordering::Relaxed,
                        );
                        frames.fetch_add(x.len() as u64, Ordering::Relaxed);
                    },
                );
                self.fake.open_input(&r, Box::new(cb)).ok()
            }),
        };
        match opened {
            Some(h) => {
                match dir {
                    Direction::Output => self.output = Some(h),
                    Direction::Input => self.input = Some(h),
                }
                DeviceEvent::Opened { dir, fallback }
            }
            None => DeviceEvent::OpenFailed { dir },
        }
    }

    fn open(&mut self, dir: Direction) {
        let ev = self.try_open(dir);
        self.handle(ev);
    }

    fn poll_devices(&mut self) {
        let (diffs, error) = self.watcher.pass(&self.fake, Enumerate::Cached);
        assert!(error.is_none(), "{error:?}");
        for d in &diffs {
            self.list.apply(d);
        }
    }

    fn poll(&mut self) {
        self.poll_devices();
        for dir in [Direction::Output, Direction::Input] {
            if let Some(name) = self.machine.device(dir).map(str::to_owned) {
                let present = self.list.is_present(&name, dir);
                self.handle(DeviceEvent::Poll { dir, present });
            }
        }
    }

    fn tick(&mut self) {
        for dir in [Direction::Output, Direction::Input] {
            let handle = match dir {
                Direction::Output => &self.output,
                Direction::Input => &self.input,
            };
            let bits = handle.as_ref().map_or(0, |h| h.status().take());
            if bits != 0 {
                self.handle(DeviceEvent::Flags { dir, bits });
            }
        }
    }

    /// Runs the control loop (16 ms ticks, 1 s polls) until `t_end`.
    fn run_until(&mut self, t_end: u64) {
        let mut t = self.fake.now_ns();
        while t < t_end {
            t = (t / TICK + 1) * TICK;
            let next_poll = (self.fake.now_ns() / SEC + 1) * SEC;
            let step = t.min(next_poll).min(t_end);
            self.fake.advance_to(step);
            if step == next_poll {
                self.poll();
            }
            if step == t {
                self.tick();
            }
            t = step;
        }
    }

    fn actions_between(&self, from: u64, to: u64) -> Vec<DeviceAction> {
        self.log
            .iter()
            .filter(|(t, _)| *t >= from && *t <= to)
            .map(|(_, a)| a.clone())
            .collect()
    }
}

/// AC-6 + AC-7: output lost mid-playback → stop within one tick, banner; replug → reopen with
/// the prior config, reconnected notice, no auto-resume.
#[test]
fn ac6_ac7_output_loss_and_recovery() {
    let fake = studio(8);
    let prefs = DevicePrefs {
        output_device: Some("Speakers".into()),
        sample_rate_hz: Some(44_100),
        buffer_size: BufferRequest::Frames(512),
        ..DevicePrefs::default()
    };
    let mut h = Harness::new(fake.clone(), prefs);
    assert_eq!(h.machine.state(Direction::Output), LinkState::Open);
    let before = h.output.as_ref().unwrap().info().clone();
    h.activity.playing = true;
    h.run_until(SEC + 300 * MS);

    let lost_at = SEC + 505 * MS;
    fake.schedule(lost_at, FakeEvent::Unplug(key("Speakers")));
    h.run_until(3 * SEC);

    // Transport stopped at the first tick after the loss, with the banner.
    let stop_at = h
        .log
        .iter()
        .find(|(_, a)| *a == DeviceAction::StopPlayback)
        .map(|(t, _)| *t)
        .expect("playback stopped");
    assert!(stop_at >= lost_at && stop_at - lost_at <= TICK, "{stop_at}");
    assert!(
        h.actions_between(stop_at, stop_at)
            .contains(&DeviceAction::Notice(DeviceNotice::DeviceLost {
                direction: Direction::Output,
                device: "Speakers".into(),
                recording_stopped: false,
                playback_stopped: true,
                recording_continues: false,
            }))
    );
    assert!(!h.activity.playing);
    assert!(h.output.is_none());
    assert_eq!(h.machine.status(Direction::Output), DeviceStatus::Lost);

    // Replug at 3.4 s: reopened by the next poll (≤ 2 s), same rate/buffer, no resume.
    let replug_at = 3_400 * MS;
    fake.schedule(
        replug_at,
        FakeEvent::Plug(HOST, FakeDevice::new("Speakers").with_output(out_dir())),
    );
    h.run_until(6 * SEC);
    let reconnect_at = h
        .log
        .iter()
        .find(|(_, a)| {
            matches!(
                a,
                DeviceAction::Notice(DeviceNotice::DeviceReconnected { .. })
            )
        })
        .map(|(t, _)| *t)
        .expect("reconnected");
    assert!(reconnect_at - replug_at <= 2 * SEC);
    let after = h.output.as_ref().expect("reopened").info().clone();
    assert_eq!(after.sample_rate_hz, before.sample_rate_hz);
    assert_eq!(after.buffer, before.buffer);
    assert!(!h.activity.playing, "playback must not auto-resume");
    assert_eq!(h.machine.status(Direction::Output), DeviceStatus::Healthy);
    assert!(
        h.actions_between(reconnect_at, u64::MAX)
            .iter()
            .all(|a| !matches!(a, DeviceAction::StopPlayback | DeviceAction::StopRecording))
    );
    assert_eq!(fake.rt_violations(), 0);
}

/// SPEC-002 AC-16 (device-layer part): output-only loss while recording keeps the input stream
/// running with no gap; monitoring stops; the banner appears.
#[test]
fn output_only_loss_while_recording_keeps_recording() {
    let fake = studio(9);
    let prefs = DevicePrefs {
        input_device: Some("USB Interface".into()),
        output_device: Some("Speakers".into()),
        ..DevicePrefs::default()
    };
    let mut h = Harness::new(fake.clone(), prefs);
    h.open(Direction::Input);
    h.activity = Activity {
        playing: false,
        recording: true,
        monitoring: true,
    };
    h.run_until(2 * SEC);
    let out_id = h.output.as_ref().unwrap().info().id;
    fake.schedule(2_222 * MS, FakeEvent::LoseStream(out_id));
    let frames_at_loss = h.input_frames.load(Ordering::Relaxed);
    h.run_until(5 * SEC);

    assert!(h.activity.recording, "recording continues");
    assert!(!h.activity.monitoring, "monitoring stopped");
    assert!(h.log.iter().all(|(_, a)| *a != DeviceAction::StopRecording));
    assert!(h.log.iter().any(|(_, a)| {
        *a == DeviceAction::Notice(DeviceNotice::DeviceLost {
            direction: Direction::Output,
            device: "Speakers".into(),
            recording_stopped: false,
            playback_stopped: false,
            recording_continues: true,
        })
    }));
    assert_eq!(h.machine.state(Direction::Input), LinkState::Open);
    assert!(h.input_frames.load(Ordering::Relaxed) > frames_at_loss + 2 * 48_000);
    assert_eq!(
        h.input_gaps.load(Ordering::Relaxed),
        0,
        "no gap in the take"
    );
    assert_eq!(fake.rt_violations(), 0);
}

/// Input loss while recording stops the recording and keeps the output running.
#[test]
fn input_loss_while_recording_stops_recording() {
    let fake = studio(10);
    let prefs = DevicePrefs {
        input_device: Some("USB Interface".into()),
        ..DevicePrefs::default()
    };
    let mut h = Harness::new(fake.clone(), prefs);
    h.open(Direction::Input);
    h.activity.recording = true;
    h.run_until(SEC);
    fake.schedule(1_100 * MS, FakeEvent::Unplug(key("USB Interface")));
    h.run_until(3 * SEC);
    assert!(!h.activity.recording);
    assert!(h.log.iter().any(|(_, a)| *a == DeviceAction::StopRecording));
    assert!(h.input.is_none());
    assert_eq!(h.machine.state(Direction::Output), LinkState::Open);
}

/// SPEC-001 §2.2: a stream that fails to open is handled as device loss.
#[test]
fn open_failure_is_treated_as_loss() {
    let fake = studio(11);
    fake.fail_opens(
        &key("Speakers"),
        Some(BackendError::DeviceBusy(key("Speakers"))),
    );
    let h = Harness::new(fake, DevicePrefs::default());
    assert_eq!(h.machine.status(Direction::Output), DeviceStatus::Lost);
    assert!(h.output.is_none());
}

/// AC-8: restart with all devices present restores everything silently; with the output absent,
/// the default is used with a notice and the saved preference survives a save.
#[test]
fn ac8_persistence_across_restart() {
    let saved = DevicePrefs {
        host: Some(HOST),
        input_device: Some("USB Interface".into()),
        input_channel: 2,
        output_device: Some("Headset".into()),
        sample_rate_hz: Some(48_000),
        buffer_size: BufferRequest::Frames(256),
    };
    let file = serde_json::to_string_pretty(&saved).unwrap();

    // Relaunch 1: everything present.
    let fake = studio(12);
    let loaded: DevicePrefs = serde_json::from_str(&file).unwrap();
    assert_eq!(loaded, saved);
    let res = resolve_devices(
        &loaded,
        HOST,
        &fake.enumerate(HOST, Enumerate::Fresh).unwrap(),
    );
    assert!(res.notices.is_empty(), "{:?}", res.notices);
    assert_eq!(res.output, Some(key("Headset")));
    let input = res.input.as_ref().unwrap();
    assert_eq!(
        (input.device.clone(), input.channel),
        (key("USB Interface"), 2)
    );
    assert_eq!(res.sample_rate_hz, Some(48_000));
    assert_eq!(res.buffer, BufferRequest::Frames(256));

    // Relaunch 2: the headset is unplugged.
    fake.unplug(&key("Headset"));
    let loaded: DevicePrefs = serde_json::from_str(&file).unwrap();
    let res = resolve_devices(
        &loaded,
        HOST,
        &fake.enumerate(HOST, Enumerate::Fresh).unwrap(),
    );
    assert_eq!(res.output, Some(key("Speakers")));
    assert_eq!(
        res.notices,
        vec![DeviceNotice::DeviceNotFound {
            direction: Direction::Output,
            saved: "Headset".into(),
            using: Some("Speakers".into()),
        }]
    );
    assert_eq!(
        serde_json::to_string_pretty(&loaded).unwrap(),
        file,
        "preference kept"
    );
}

// ---------------------------------------------------------------------------------------------
// Fake backend guarantees relied on by T-105…T-107.

#[test]
fn fake_timing_model_sizes_latency_skew_and_clocks() {
    let fake = FakeBackend::new(13);
    fake.plug(
        HOST,
        FakeDevice::new("Out").with_output(
            FakeDirection::new(2, &[48_000], 48_000)
                .callback_sizes(CallbackSizes::FULL_RANDOM)
                .latency_ns(7 * MS)
                .skew_ppm(200.0)
                .record_output(),
        ),
    );
    fake.plug(
        HOST,
        FakeDevice::new("In").with_input(
            FakeDirection::new(1, &[44_100], 44_100)
                .callback_sizes(CallbackSizes::Fixed(256))
                .latency_ns(5 * MS),
        ),
    );
    guarded(&fake);
    let out_lat = Arc::new(AtomicU64::new(0));
    let first_playback_app = Arc::new(AtomicU64::new(u64::MAX));
    let (o, fp) = (out_lat.clone(), first_playback_app.clone());
    let out_cb = move |_: &mut [f32], _: usize, ts: OutputTimestamp| {
        o.fetch_max(ts.playback_ns - ts.callback_ns, Ordering::Relaxed);
        let app = ts.to_app_ns(ts.playback_ns);
        let _ = fp.compare_exchange(u64::MAX, app, Ordering::Relaxed, Ordering::Relaxed);
    };
    let out = fake
        .open_output(
            &vox_engine::StreamRequest {
                device: DeviceKey::new(HOST, "Out"),
                sample_rate_hz: 48_000,
                buffer: BufferRequest::Auto,
                channels: None,
            },
            Box::new(out_cb),
        )
        .unwrap();
    let in_lat = Arc::new(AtomicU64::new(0));
    let first_capture = Arc::new(AtomicU64::new(0));
    let first_capture_app = Arc::new(AtomicU64::new(u64::MAX));
    let (i, fc, fca) = (
        in_lat.clone(),
        first_capture.clone(),
        first_capture_app.clone(),
    );
    let in_cb = move |_: &[f32], _: usize, ts: InputTimestamp| {
        i.fetch_max(ts.callback_ns - ts.capture_ns, Ordering::Relaxed);
        let _ = fc.compare_exchange(0, ts.capture_ns, Ordering::Relaxed, Ordering::Relaxed);
        let app = ts.to_app_ns(ts.capture_ns);
        let _ = fca.compare_exchange(u64::MAX, app, Ordering::Relaxed, Ordering::Relaxed);
    };
    let inp = fake
        .open_input(
            &vox_engine::StreamRequest {
                device: DeviceKey::new(HOST, "In"),
                sample_rate_hz: 44_100,
                buffer: BufferRequest::Auto,
                channels: None,
            },
            Box::new(in_cb),
        )
        .unwrap();
    fake.advance_to(10 * SEC);

    assert_eq!(out_lat.load(Ordering::Relaxed), 7 * MS);
    // Input blocks arrive once complete: callback − capture = 256 / 44 100 s + 5 ms (±1 ns
    // rounding), so a monitoring servo sees a real device's timing.
    let block_ns = frames_to_ns(256, 44_100);
    assert!(
        (5 * MS + block_ns..=5 * MS + block_ns + 1).contains(&in_lat.load(Ordering::Relaxed)),
        "{}",
        in_lat.load(Ordering::Relaxed)
    );
    // App clock: both streams map to the same simulated time line (both opened at 0).
    assert_eq!(first_capture_app.load(Ordering::Relaxed), 0);
    assert_eq!(first_playback_app.load(Ordering::Relaxed), 7 * MS);
    let rec = fake.recorded_output(out.info().id).unwrap();
    let sizes: Vec<u32> = rec.blocks.iter().map(|b| b.frames).collect();
    assert!(sizes.iter().all(|&n| (1..=4096).contains(&n)));
    assert!(sizes.iter().any(|&n| n < 256) && sizes.iter().any(|&n| n > 3000));
    // +200 ppm: 10 s of device time delivers 480 096 frames (± one callback).
    let frames: u64 = sizes.iter().map(|&n| u64::from(n)).sum();
    assert!(frames.abs_diff(480_096) <= 4096, "{frames}");
    // Stream clocks have independent origins.
    let out_origin = rec.blocks[0].playback_ns - 7 * MS;
    let in_origin = first_capture.load(Ordering::Relaxed);
    assert!(
        out_origin.abs_diff(in_origin) > MS,
        "clocks must not share an origin"
    );
    // Fixed 256-frame input blocks: block k is delivered at time(256·(k+1)) + 5 ms ≤ 10 s, i.e.
    // 256·(k+1) ≤ 9.995 s × 44 100 = 440 779.5 → k = 0…1720.
    assert_eq!(inp.status().callback_count(), 1721);
    assert_eq!(fake.rt_violations(), 0);

    // Same seed, same script → identical callback sizes.
    let again = FakeBackend::new(13);
    again.plug(
        HOST,
        FakeDevice::new("Out").with_output(
            FakeDirection::new(2, &[48_000], 48_000)
                .callback_sizes(CallbackSizes::FULL_RANDOM)
                .latency_ns(7 * MS)
                .skew_ppm(200.0)
                .record_output(),
        ),
    );
    let out2 = again
        .open_output(
            &vox_engine::StreamRequest {
                device: DeviceKey::new(HOST, "Out"),
                sample_rate_hz: 48_000,
                buffer: BufferRequest::Auto,
                channels: None,
            },
            silent_output(),
        )
        .unwrap();
    again.advance_to(10 * SEC);
    let sizes2: Vec<u32> = again
        .recorded_output(out2.info().id)
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.frames)
        .collect();
    assert_eq!(sizes, sizes2);
}

#[test]
fn fake_input_dropout_and_non_fatal_errors() {
    let fake = FakeBackend::new(14);
    fake.plug(
        HOST,
        FakeDevice::new("Mic").with_input(
            FakeDirection::new(1, &[48_000], 48_000).callback_sizes(CallbackSizes::Fixed(480)),
        ),
    );
    guarded(&fake);
    let gap_ns = Arc::new(AtomicU64::new(0));
    let next = Arc::new(AtomicU64::new(0));
    let (g, n) = (gap_ns.clone(), next.clone());
    let cb = move |d: &[f32], _: usize, ts: InputTimestamp| {
        let exp = n.load(Ordering::Relaxed);
        if exp != 0 && ts.capture_ns > exp {
            g.fetch_add(ts.capture_ns - exp, Ordering::Relaxed);
        }
        n.store(
            ts.capture_ns + frames_to_ns(d.len() as u64, 48_000),
            Ordering::Relaxed,
        );
    };
    let s = fake
        .open_input(
            &vox_engine::StreamRequest {
                device: DeviceKey::new(HOST, "Mic"),
                sample_rate_hz: 48_000,
                buffer: BufferRequest::Auto,
                channels: None,
            },
            Box::new(cb),
        )
        .unwrap();
    fake.schedule(3 * SEC, FakeEvent::InputDropout(s.info().id, 480));
    fake.advance_to(4 * SEC);
    assert_eq!(gap_ns.load(Ordering::Relaxed), 10 * MS, "10 ms dropout");

    fake.stream_error(s.info().id, flags::BACKEND_ERROR);
    let calls = s.status().callback_count();
    fake.advance_by(SEC);
    assert_eq!(s.status().take(), flags::BACKEND_ERROR);
    assert!(
        s.status().callback_count() > calls,
        "non-fatal error keeps running"
    );
    fake.lose_stream(s.info().id);
    let calls = s.status().callback_count();
    fake.advance_by(SEC);
    assert!(s.status().is_lost());
    assert_eq!(s.status().callback_count(), calls, "lost stream stops");
    assert!(
        fake.enumerate(HOST, Enumerate::Cached)
            .unwrap()
            .get("Mic")
            .is_some()
    );
}

#[test]
fn fake_rt_guard_catches_allocating_callbacks() {
    let fake = FakeBackend::new(15);
    fake.plug(
        HOST,
        FakeDevice::new("Out").with_output(FakeDirection::new(2, &[48_000], 48_000)),
    );
    guarded(&fake);
    let cb = |d: &mut [f32], _: usize, _: OutputTimestamp| {
        let v = vec![0.5f32; d.len()];
        d.copy_from_slice(&v);
    };
    let _s = fake
        .open_output(
            &vox_engine::StreamRequest {
                device: DeviceKey::new(HOST, "Out"),
                sample_rate_hz: 48_000,
                buffer: BufferRequest::Auto,
                channels: None,
            },
            Box::new(cb),
        )
        .unwrap();
    fake.advance_by(100 * MS);
    assert!(fake.rt_violations() > 0);
}

#[test]
fn fake_hosts_and_default_host() {
    let fake = FakeBackend::new(16);
    fake.add_host(HostId::Alsa, true);
    fake.add_host(HostId::PipeWire, false);
    assert_eq!(fake.hosts(), vec![HostId::Alsa]);
    assert_eq!(fake.default_host(), Some(HostId::Alsa));
    assert_eq!(
        fake.enumerate(HostId::PipeWire, Enumerate::Cached),
        Err(BackendError::HostUnavailable(HostId::PipeWire))
    );
    fake.set_host_available(HostId::PipeWire, true);
    assert_eq!(fake.default_host(), Some(HostId::PipeWire));
}

fn request(name: &str, rate: u32, buffer: BufferRequest) -> vox_engine::StreamRequest {
    vox_engine::StreamRequest {
        device: key(name),
        sample_rate_hz: rate,
        buffer,
        channels: None,
    }
}

/// Names first, capabilities second (the ALSA path): the device list is usable after the quick
/// phase and a `changed` diff fills in the capabilities; later passes stay quick.
#[test]
fn two_phase_enumeration_posts_names_then_capabilities() {
    let fake = studio(20);
    fake.set_pending_caps(true);
    let mut w = DeviceWatcher::new(HOST);
    let mut list = DeviceList::default();
    let (diffs, error) = w.pass(&fake, Enumerate::Cached);
    assert!(error.is_none());
    assert_eq!(diffs.len(), 2);
    assert!(diffs[0].full && diffs[0].added.len() == 3);
    list.apply(&diffs[0]);
    assert!(list.snapshot.has_pending());
    assert!(
        list.is_present("Speakers", Direction::Output),
        "present while pending"
    );
    assert!(
        resolve_devices(&DevicePrefs::default(), HOST, &list.snapshot).caps_pending,
        "resolution waits for capabilities"
    );
    assert_eq!(diffs[1].changed.len(), 3);
    list.apply(&diffs[1]);
    assert!(!list.snapshot.has_pending());
    assert!(list.snapshot.caps("Speakers", Direction::Output).is_some());
    let (diffs, _) = w.pass(&fake, Enumerate::Cached);
    assert!(
        diffs.is_empty(),
        "known capabilities are not re-read: {diffs:?}"
    );
    // A hot-plugged device goes through both phases again.
    fake.plug(
        HOST,
        FakeDevice::new("USB DAC").with_output(FakeDirection::new(2, &[48_000], 48_000)),
    );
    let (diffs, _) = w.pass(&fake, Enumerate::Cached);
    assert_eq!(diffs.len(), 2);
    assert_eq!(diffs[0].added.len(), 1);
    assert_eq!(diffs[1].changed.len(), 1);
}

/// SPEC-001 §2.2: a present device whose capabilities can't be read (busy/exclusive) is used as
/// configured — no "not found — using default" — and its failed open is handled as loss.
#[test]
fn unreadable_device_is_present_and_a_failed_open_is_loss() {
    let fake = studio(21);
    fake.plug(
        HOST,
        FakeDevice::new("Busy DAC")
            .with_output(FakeDirection::new(2, &[48_000], 48_000).unreadable()),
    );
    let prefs = DevicePrefs {
        output_device: Some("Busy DAC".into()),
        ..DevicePrefs::default()
    };
    let mut h = Harness::new(fake, prefs);
    assert!(h.list.is_present("Busy DAC", Direction::Output));
    assert_eq!(h.machine.device(Direction::Output), Some("Busy DAC"));
    assert_eq!(h.machine.status(Direction::Output), DeviceStatus::Lost);
    let notices: Vec<_> = h
        .log
        .iter()
        .filter_map(|(_, a)| match a {
            DeviceAction::Notice(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    assert!(
        notices
            .iter()
            .all(|n| !matches!(n, DeviceNotice::DeviceNotFound { .. })),
        "{notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, DeviceNotice::DeviceLost { .. }))
    );
    // Still listed: polling doesn't produce more loss notices.
    let before = h.log.len();
    h.run_until(3 * SEC);
    assert!(
        h.log[before..]
            .iter()
            .all(|(_, a)| !matches!(a, DeviceAction::Notice(DeviceNotice::DeviceLost { .. })))
    );
}

/// A stream that stops calling back without any error is caught by the stall detector within
/// max(500 ms, 4 periods) and then handled as DEVICE_LOST.
#[test]
fn silent_stream_death_is_detected() {
    use vox_engine::backend::{STALL_MIN_NS, StallDetector};
    let fake = studio(22);
    let s = fake
        .open_output(
            &request("Headset", 48_000, BufferRequest::Frames(256)),
            silent_output(),
        )
        .unwrap();
    let mut detector = StallDetector::new(s.info(), fake.now_ns());
    assert_eq!(detector.threshold_ns(), STALL_MIN_NS);
    let stall_at = SEC;
    fake.schedule(stall_at, FakeEvent::Stall(s.info().id));
    let mut detected = None;
    for k in 1..=(3 * SEC / TICK) {
        fake.advance_to(k * TICK);
        if detector.check(s.status().callback_count(), fake.now_ns()) {
            detected = Some(fake.now_ns());
            break;
        }
    }
    let detected = detected.expect("stall detected");
    assert_eq!(s.status().peek(), 0, "no flag was raised by the backend");
    // The tick sees the last callback up to one tick late and detects on a tick: ≤ 2 ticks slack.
    assert!(
        detected >= stall_at + STALL_MIN_NS - frames_to_ns(256, 48_000)
            && detected <= stall_at + STALL_MIN_NS + 2 * TICK,
        "{detected}"
    );
    let mut m = DeviceStateMachine::new();
    m.handle(
        DeviceEvent::Configured {
            dir: Direction::Output,
            device: Some("Headset".into()),
        },
        Activity::default(),
    );
    m.handle(
        DeviceEvent::Opened {
            dir: Direction::Output,
            fallback: false,
        },
        Activity::default(),
    );
    let actions = m.handle(
        DeviceEvent::Flags {
            dir: Direction::Output,
            bits: flags::DEVICE_LOST,
        },
        Activity::default(),
    );
    assert!(actions.contains(&DeviceAction::CloseStream(Direction::Output)));
}

/// Plugging a second identical device renames the first (both get their id appended); the
/// configured base name still counts as present, so nothing is stopped.
#[test]
fn identical_second_device_is_not_a_false_loss() {
    let mic = |name: &str, id: &str| {
        FakeDevice::new(name)
            .identity("USB Mic", id)
            .with_input(FakeDirection::new(1, &[48_000], 48_000))
    };
    let fake = FakeBackend::new(23);
    fake.plug(HOST, mic("USB Mic", "usb-a"));
    let mut w = DeviceWatcher::new(HOST);
    let mut list = DeviceList::default();
    for d in w.pass(&fake, Enumerate::Cached).0 {
        list.apply(&d);
    }
    let mut m = DeviceStateMachine::new();
    let recording = Activity {
        recording: true,
        ..Activity::default()
    };
    m.handle(
        DeviceEvent::Configured {
            dir: Direction::Input,
            device: Some("USB Mic".into()),
        },
        recording,
    );
    m.handle(
        DeviceEvent::Opened {
            dir: Direction::Input,
            fallback: false,
        },
        recording,
    );
    // The backend now reports both mics under suffixed names.
    fake.unplug(&key("USB Mic"));
    fake.plug(HOST, mic("USB Mic (usb-a)", "usb-a"));
    fake.plug(HOST, mic("USB Mic (usb-b)", "usb-b"));
    for d in w.pass(&fake, Enumerate::Cached).0 {
        list.apply(&d);
    }
    assert!(list.snapshot.get("USB Mic").is_none());
    let present = list.is_present("USB Mic", Direction::Input);
    assert!(present);
    let actions = m.handle(
        DeviceEvent::Poll {
            dir: Direction::Input,
            present,
        },
        recording,
    );
    assert!(actions.is_empty(), "{actions:?}");
    assert_eq!(m.state(Direction::Input), LinkState::Open);
}

#[test]
fn fake_dropout_is_ignored_for_outputs_and_jitter_only_delays_callbacks() {
    let fake = FakeBackend::new(24);
    fake.plug(
        HOST,
        FakeDevice::new("Out")
            .with_output(FakeDirection::new(2, &[48_000], 48_000).record_output()),
    );
    fake.plug(
        HOST,
        FakeDevice::new("Mic").with_input(
            FakeDirection::new(1, &[48_000], 48_000)
                .callback_sizes(CallbackSizes::Fixed(480))
                .latency_ns(2 * MS)
                .jitter_ns(MS),
        ),
    );
    guarded(&fake);
    let out = fake
        .open_output(
            &request("Out", 48_000, BufferRequest::Auto),
            silent_output(),
        )
        .unwrap();
    fake.schedule(500 * MS, FakeEvent::InputDropout(out.info().id, 1000));

    let lat_min = Arc::new(AtomicU64::new(u64::MAX));
    let lat_max = Arc::new(AtomicU64::new(0));
    let gaps = Arc::new(AtomicU64::new(0));
    let next = Arc::new(AtomicU64::new(0));
    let (lo, hi, g, n) = (lat_min.clone(), lat_max.clone(), gaps.clone(), next.clone());
    let cb = move |d: &[f32], _: usize, ts: InputTimestamp| {
        let lat = ts.callback_ns - ts.capture_ns;
        lo.fetch_min(lat, Ordering::Relaxed);
        hi.fetch_max(lat, Ordering::Relaxed);
        let exp = n.load(Ordering::Relaxed);
        if exp != 0 && ts.capture_ns.abs_diff(exp) > 1 {
            g.fetch_add(1, Ordering::Relaxed);
        }
        n.store(
            ts.capture_ns + frames_to_ns(d.len() as u64, 48_000),
            Ordering::Relaxed,
        );
    };
    let _mic = fake
        .open_input(&request("Mic", 48_000, BufferRequest::Auto), Box::new(cb))
        .unwrap();
    fake.advance_to(2 * SEC);

    let rec = fake.recorded_output(out.info().id).unwrap();
    for pair in rec.blocks.windows(2) {
        assert_eq!(
            pair[1].first_frame,
            pair[0].first_frame + u64::from(pair[0].frames)
        );
    }
    let base = 10 * MS + 2 * MS; // 480 frames at 48 kHz + latency
    assert!(lat_min.load(Ordering::Relaxed) >= base);
    assert!(lat_max.load(Ordering::Relaxed) <= base + MS);
    assert!(
        lat_max.load(Ordering::Relaxed) > lat_min.load(Ordering::Relaxed),
        "jitter applied"
    );
    assert_eq!(
        gaps.load(Ordering::Relaxed),
        0,
        "capture times stay continuous"
    );
    assert_eq!(fake.rt_violations(), 0);
}

/// H-59 (SPEC-001 §2.1 "Rescan", §2.4): a device lost to a backend error while it stays *listed*
/// is parked after one reopen attempt — the ~1 s background poll reports only diffs, so nothing
/// ever asks for it again and, before H-59, the user had no way to retry short of restarting the
/// app. `rescan_devices()` clears that one-shot latch and re-drives the state machine.
#[test]
fn rescan_reopens_a_device_parked_after_its_automatic_retry() {
    let fake = FakeBackend::new(21);
    fake.plug(HOST, FakeDevice::new("Speakers").with_output(out_dir()));
    guarded(&fake);
    let registry =
        Arc::new(vox_rack::Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = vox_engine::EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.prefs = DevicePrefs {
        output_device: Some("Speakers".into()),
        ..DevicePrefs::default()
    };
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = vox_engine::ManualEngine::new(cfg);
    eng.poll_devices();
    assert_eq!(
        eng.devices().output_status,
        DeviceStatus::Healthy,
        "the device opens to start with"
    );

    // The backend starts refusing to open it and the open stream reports the loss. The device is
    // never unplugged, so it stays listed throughout.
    let stream = fake
        .stream_for(&key("Speakers"), Direction::Output)
        .expect("output stream open");
    fake.fail_opens(
        &key("Speakers"),
        Some(BackendError::DeviceBusy(key("Speakers"))),
    );
    fake.lose_stream(stream);
    eng.tick();
    assert_eq!(eng.devices().output_status, DeviceStatus::Lost);

    // A rescan while the device is still broken spends the one reopen attempt it is granted.
    assert_eq!(eng.rescan_devices().output_status, DeviceStatus::Lost);

    // The device heals, but it never went absent, so polling produces no diff and no event.
    fake.fail_opens(&key("Speakers"), None);
    for _ in 0..4 {
        eng.tick();
        eng.poll_devices();
    }
    assert_eq!(
        eng.devices().output_status,
        DeviceStatus::Lost,
        "polling alone never retries a device that was never reported absent"
    );

    // Rescan: the latch is cleared, the stream reopens with its previous settings.
    let view = eng.rescan_devices();
    assert_eq!(view.output_status, DeviceStatus::Healthy);
    assert_eq!(view.output_device.as_deref(), Some("Speakers"));
    assert_eq!(fake.rt_violations(), 0);
}
