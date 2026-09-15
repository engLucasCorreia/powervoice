//! T-801: host and plugin ends in one process (a second mapping of the same segment): the host's
//! miss/crossfade state machine driven deterministically, the gain round trip across threads
//! (futex and spin-yield), events, overrun resynchronisation, the monitor's faults, the POSIX
//! named segment, the wakeup bounds — every host call under the allocation checker.

vox_module_api::install_test_allocator!();

mod common;

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use common::{RATE, assert_bits, expected_gain_output, gained, max_step, noise, sine};
use vox_module_api::test_util::{alloc_checks_active, no_alloc};
use vox_sandbox_ipc::test_plugins::{TEST_GAIN, apply_gain};
use vox_sandbox_ipc::wakeup::{Doorbell, WaitStatus};
use vox_sandbox_ipc::{
    BlockOutcome, Channel, ChannelConfig, Chunk, Deadline, Fault, HostEnd, HostOptions, Monitor,
    PeerState, PeerStatus, PlatformWakeup, PluginEnd, Serviced, SharedRegion, SpinYieldWakeup,
    WaitBudget, Wakeup, WireEvent,
};

fn gain(c: Chunk<'_>) {
    apply_gain(c.input, c.output);
}

fn options(wait: WaitBudget, crossfade_samples: u32) -> HostOptions {
    HostOptions {
        wait,
        crossfade_samples,
        ..HostOptions::default()
    }
}

/// A channel with an in-process plugin end (second mapping) and the host's two ends.
fn setup<W: Wakeup>(
    config: ChannelConfig,
    opts: HostOptions,
) -> (PluginEnd<W>, HostEnd<W>, Monitor) {
    let chan = Channel::create(config).unwrap();
    let plugin = PluginEnd::attach(chan.region().duplicate().unwrap()).unwrap();
    let (host, monitor) = chan.into_ends_with::<W>(opts);
    (plugin, host, monitor)
}

fn run<W: Wakeup>(host: &mut HostEnd<W>, input: &[f32], output: &mut [f32]) -> BlockOutcome {
    no_alloc(|| host.process(input, output)).expect("HostEnd::process allocated")
}

#[test]
fn allocation_checker_is_active() {
    assert!(alloc_checks_active());
}

/// Miss → splice crossfade to dry; late output → linear crossfade back to wet; steady state
/// bit-exact. Budget 0, so every step is deterministic.
#[test]
fn miss_and_recovery_crossfade_deterministically() {
    const B: usize = 64;
    const F: usize = 16;
    let (mut plugin, mut host, mut monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig::pipelined(RATE, B as u32),
        options(WaitBudget::FractionOfBlock(0.0), F as u32),
    );
    let x = sine(B * 8, 440.0, 0.5);
    let blk = |k: usize| &x[k * B..(k + 1) * B];
    let mut y = vec![0.0f32; B];

    // Block 0: the latency fill (zeros), wet.
    assert_eq!(run(&mut host, blk(0), &mut y), BlockOutcome::Wet);
    assert!(y.iter().all(|v| v.to_bits() == 0));
    assert_eq!(
        plugin.service(Duration::ZERO, gain),
        Serviced::Processed {
            chunks: 1,
            frames: B as u64
        }
    );
    // Block 1 returns block 0 × gain.
    assert_eq!(run(&mut host, blk(1), &mut y), BlockOutcome::Wet);
    assert_bits(&y, &gained(blk(0)), "block 1");
    plugin.service(Duration::ZERO, gain);
    assert_eq!(run(&mut host, blk(2), &mut y), BlockOutcome::Wet);
    assert_bits(&y, &gained(blk(1)), "block 2");
    let last_wet = y[B - 1];

    // No service: block 3 misses → dry(block 2) spliced from the last wet sample.
    assert_eq!(run(&mut host, blk(3), &mut y), BlockOutcome::Missed);
    let dry = blk(2);
    let delta = last_wet - blk(1)[B - 1];
    for i in 0..F - 1 {
        let want = dry[i] + delta * ((F - 1 - i) as f32 / F as f32);
        assert!(
            (y[i] - want).abs() < 1e-6,
            "splice sample {i}: {} vs {want}",
            y[i]
        );
    }
    assert_bits(&y[F - 1..], &dry[F - 1..], "block 3 after the splice");
    assert!((y[0] - last_wet).abs() < 0.03, "splice continuity");

    // The plugin catches up (blocks 2 and 3 in one service call).
    assert_eq!(
        plugin.service(Duration::ZERO, gain),
        Serviced::Processed {
            chunks: 2,
            frames: 2 * B as u64
        }
    );
    // Block 4: block 3's output is late-but-there → crossfade dry → wet.
    assert_eq!(run(&mut host, blk(4), &mut y), BlockOutcome::Wet);
    let (wet, dry) = (gained(blk(3)), blk(3));
    for i in 0..F - 1 {
        let g = (i + 1) as f32 / F as f32;
        let want = g * wet[i] + (1.0 - g) * dry[i];
        assert!((y[i] - want).abs() < 1e-6, "crossfade sample {i}");
    }
    assert_bits(&y[F - 1..], &wet[F - 1..], "block 4 after the crossfade");
    plugin.service(Duration::ZERO, gain);
    assert_eq!(run(&mut host, blk(5), &mut y), BlockOutcome::Wet);
    assert_bits(&y, &gained(blk(4)), "block 5 (steady)");

    let c = host.counters();
    assert_eq!(
        (c.blocks, c.wet, c.missed, c.bypassed, c.waited),
        (6, 5, 1, 0, 0)
    );
    assert_eq!(c.max_consecutive_misses, 1);
    let h = monitor.poll(PeerStatus::Alive);
    assert_eq!(h.new_misses, 1);
    assert_eq!(h.fault, None);
    assert_eq!(h.state, PeerState::Running);
}

fn round_trip<W: Wakeup>(config: ChannelConfig, sizes: &[usize]) {
    let (mut plugin, mut host, mut monitor) = setup::<W>(
        config,
        options(WaitBudget::Fixed(Duration::from_secs(5)), 96),
    );
    let worker = thread::spawn(move || {
        loop {
            if plugin.service(Duration::from_millis(20), gain) == Serviced::Shutdown {
                plugin.stop();
                return;
            }
        }
    });
    let x = noise(RATE as usize, 11);
    let mut y = vec![0.0f32; x.len()];
    let (mut at, mut k) = (0, 0);
    while at < x.len() {
        let n = sizes[k % sizes.len()].min(x.len() - at);
        let outcome = run(&mut host, &x[at..at + n], &mut y[at..at + n]);
        assert_eq!(outcome, BlockOutcome::Wet, "call {k} at {at}");
        at += n;
        k += 1;
    }
    monitor.request_shutdown();
    worker.join().unwrap();
    let h = monitor.poll(PeerStatus::Exited);
    assert_eq!(h.fault, None, "{h:?}");
    assert_eq!(h.state, PeerState::Stopped);
    assert_eq!(h.counters.missed, 0);
    assert_bits(
        &y,
        &expected_gain_output(&x, config.latency_samples as usize),
        W::NAME,
    );
    // Bypassed once the plugin stopped: dry, no wake.
    let mut tail = [0.0f32; 16];
    assert_eq!(
        run(&mut host, &[0.25; 16], &mut tail),
        BlockOutcome::Bypassed
    );
}

#[test]
fn gain_round_trip_threaded_is_bit_exact() {
    let b = 128;
    let sizes = [b, 1, b / 2, 7, b, b - 3, 2 * b + 5, 64];
    #[cfg(target_os = "linux")]
    {
        round_trip::<PlatformWakeup>(ChannelConfig::pipelined(RATE, b as u32), &sizes);
        round_trip::<PlatformWakeup>(ChannelConfig::synchronous(RATE, b as u32), &sizes);
    }
    round_trip::<SpinYieldWakeup>(ChannelConfig::pipelined(RATE, 64), &[64, 13, 64, 50]);
    round_trip::<SpinYieldWakeup>(ChannelConfig::synchronous(RATE, 256), &[256, 100]);
}

#[test]
fn events_reach_the_plugin_at_their_positions() {
    const B: usize = 64;
    let (mut plugin, mut host, _monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig {
            event_capacity: 4,
            ..ChannelConfig::pipelined(RATE, B as u32)
        },
        options(WaitBudget::FractionOfBlock(0.0), 16),
    );
    let p = host.position();
    let mut y = [0.0f32; B];
    no_alloc(|| {
        assert!(host.push_event(WireEvent::param(p + 3, 7, 0.5)));
        assert!(host.push_event(WireEvent::param(p + B as u64 + 10, 8, -1.0)));
    })
    .unwrap();
    run(&mut host, &[0.1; B], &mut y);
    let mut seen = Vec::new();
    plugin.service(Duration::ZERO, |c| {
        seen.extend(c.events.iter().map(|e| (c.offset(e), e.id, e.value)));
    });
    assert_eq!(seen, [(3, 7, 0.5)]);
    seen.clear();
    run(&mut host, &[0.1; B], &mut y);
    plugin.service(Duration::ZERO, |c| {
        seen.extend(c.events.iter().map(|e| (c.offset(e), e.id, e.value)));
    });
    assert_eq!(seen, [(10, 8, -1.0)]);

    // Ring of 4: the fifth unconsumed event is dropped and counted.
    let p = host.position();
    let pushed: Vec<bool> = (0..5)
        .map(|i| host.push_event(WireEvent::param(p + i, 1, 0.0)))
        .collect();
    assert_eq!(pushed, [true, true, true, true, false]);
    assert_eq!(host.counters().events_dropped, 1);

    // Plugin → host.
    assert!(plugin.push_output_event(WireEvent::param(5, 42, 0.25)));
    assert_eq!(
        no_alloc(|| host.pop_output_event()).unwrap(),
        Some(WireEvent::param(5, 42, 0.25))
    );
    assert_eq!(host.pop_output_event(), None);
}

#[test]
fn overrun_resyncs_and_recovers() {
    const B: usize = 64;
    let (mut plugin, mut host, mut monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig::pipelined(RATE, B as u32),
        options(WaitBudget::FractionOfBlock(0.0), 16),
    );
    let x = noise(B * 40, 3);
    let blk = |k: usize| &x[k * B..(k + 1) * B];
    let mut y = vec![0.0f32; B];
    // 20 blocks without the plugin: 1280 samples ≫ C − B = 448.
    for k in 0..20 {
        let want = if k == 0 {
            BlockOutcome::Wet
        } else {
            BlockOutcome::Missed
        };
        assert_eq!(run(&mut host, blk(k), &mut y), want, "block {k}");
    }
    assert_bits(&y, blk(18), "missed output is the latency-matched dry");
    assert_eq!(
        plugin.service(Duration::ZERO, gain),
        Serviced::Processed {
            chunks: 0,
            frames: 0
        }
    );
    assert_eq!(plugin.overruns(), 1);
    // Output before the resync point stays invalid.
    assert_eq!(run(&mut host, blk(20), &mut y), BlockOutcome::Missed);
    plugin.service(Duration::ZERO, gain);
    assert_eq!(run(&mut host, blk(21), &mut y), BlockOutcome::Wet);
    plugin.service(Duration::ZERO, gain);
    assert_eq!(run(&mut host, blk(22), &mut y), BlockOutcome::Wet);
    assert_bits(&y, &gained(blk(21)), "steady after the resync");
    assert_eq!(monitor.poll(PeerStatus::Alive).overruns, 1);
}

/// Random servicing (seeded): misses and recoveries interleave; the output never jumps and
/// fully wet blocks stay bit-exact.
#[test]
fn random_misses_stay_continuous() {
    const B: usize = 128;
    let (mut plugin, mut host, _monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig::pipelined(RATE, B as u32),
        options(WaitBudget::FractionOfBlock(0.0), 96),
    );
    let blocks = 400;
    let x = sine(B * blocks, 220.0, 0.5);
    let want = expected_gain_output(&x, B);
    let mut y = vec![0.0f32; x.len()];
    let mut outcomes = Vec::with_capacity(blocks);
    let mut rng = 0x9e37_79b9_u32;
    for k in 0..blocks {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        if rng % 100 < 70 {
            plugin.service(Duration::ZERO, gain);
        }
        outcomes.push(run(
            &mut host,
            &x[k * B..(k + 1) * B],
            &mut y[k * B..(k + 1) * B],
        ));
    }
    let missed = outcomes
        .iter()
        .filter(|o| **o == BlockOutcome::Missed)
        .count();
    assert!(missed > 20, "only {missed} misses");
    assert!(max_step(&y) < 0.03, "discontinuity {}", max_step(&y));
    for k in 1..blocks {
        if outcomes[k] == BlockOutcome::Wet && outcomes[k - 1] == BlockOutcome::Wet {
            let r = k * B..(k + 1) * B;
            assert_bits(&y[r.clone()], &want[r], "fully wet block");
        }
    }
}

#[test]
fn monitor_detects_a_hang_and_bypass_stops_the_wakes() {
    let (plugin, mut host, mut monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig::pipelined(RATE, 64),
        HostOptions {
            hang_timeout: Duration::from_millis(50),
            ..options(WaitBudget::FractionOfBlock(0.0), 16)
        },
    );
    let t0 = Instant::now();
    let h = monitor.poll_at(PeerStatus::Alive, t0);
    assert_eq!((h.fault, h.state), (None, PeerState::Running));
    assert_eq!(h.plugin_pid, std::process::id());
    assert_eq!(
        monitor
            .poll_at(PeerStatus::Alive, t0 + Duration::from_millis(40))
            .fault,
        None
    );
    let h = monitor.poll_at(PeerStatus::Alive, t0 + Duration::from_millis(60));
    assert_eq!(
        h.new_fault,
        Some(Fault::Hung {
            stale: Duration::from_millis(60),
            in_call: false
        })
    );
    assert!(host.is_bypassed());
    // Reported once; sticky.
    let h = monitor.poll_at(PeerStatus::Alive, t0 + Duration::from_millis(90));
    assert_eq!(h.new_fault, None);
    assert!(matches!(h.fault, Some(Fault::Hung { .. })));

    let seq = monitor.control().to_plugin.seq.load(Ordering::SeqCst);
    let mut y = [0.0f32; 64];
    for _ in 0..4 {
        assert_eq!(run(&mut host, &[0.5; 64], &mut y), BlockOutcome::Bypassed);
    }
    assert_eq!(monitor.control().to_plugin.seq.load(Ordering::SeqCst), seq);
    assert!(y.iter().all(|v| v.to_bits() == 0.5f32.to_bits()));
    drop(plugin);
}

#[test]
fn monitor_classifies_failed_exited_crashed_and_shutdown() {
    let cfg = ChannelConfig::pipelined(RATE, 64);
    let opts = options(WaitBudget::FractionOfBlock(0.0), 16);

    let (plugin, mut host, mut monitor) = setup::<SpinYieldWakeup>(cfg, opts);
    plugin.fail();
    assert_eq!(
        monitor.poll(PeerStatus::Alive).new_fault,
        Some(Fault::Failed)
    );
    let mut y = [0.0f32; 64];
    assert_eq!(run(&mut host, &[0.1; 64], &mut y), BlockOutcome::Bypassed);

    let (plugin, _host, mut monitor) = setup::<SpinYieldWakeup>(cfg, opts);
    plugin.stop();
    assert_eq!(
        monitor.poll(PeerStatus::Alive).new_fault,
        Some(Fault::Exited)
    );

    let (_plugin, _host, mut monitor) = setup::<SpinYieldWakeup>(cfg, opts);
    assert_eq!(
        monitor.poll(PeerStatus::Exited).new_fault,
        Some(Fault::Crashed)
    );

    // Never attached, process gone.
    let chan = Channel::create(cfg).unwrap();
    let (_host, mut monitor) = chan.into_ends_with::<SpinYieldWakeup>(opts);
    let h = monitor.poll(PeerStatus::Alive);
    assert_eq!((h.fault, h.state), (None, PeerState::Created));
    assert_eq!(
        monitor.poll(PeerStatus::Exited).new_fault,
        Some(Fault::Crashed)
    );

    // Requested shutdown: not a fault.
    let (mut plugin, mut host, mut monitor) = setup::<SpinYieldWakeup>(cfg, opts);
    monitor.request_shutdown();
    assert_eq!(plugin.service(Duration::ZERO, gain), Serviced::Shutdown);
    plugin.stop();
    let h = monitor.poll(PeerStatus::Exited);
    assert_eq!((h.fault, h.state), (None, PeerState::Stopped));
    assert_eq!(run(&mut host, &[0.1; 64], &mut y), BlockOutcome::Bypassed);
}

#[test]
fn telemetry_cells_reach_the_monitor() {
    let (plugin, _host, monitor) =
        setup::<SpinYieldWakeup>(ChannelConfig::pipelined(RATE, 64), HostOptions::default());
    plugin.set_telemetry(3, -4.5);
    plugin.set_telemetry(99, 1.0); // ignored
    assert_eq!(
        monitor.telemetry(3).map(f32::to_bits),
        Some((-4.5f32).to_bits())
    );
    assert_eq!(monitor.telemetry(16), None);
}

#[cfg(unix)]
#[test]
fn posix_named_segment_round_trip_and_unlink() {
    const B: usize = 64;
    let config = ChannelConfig::pipelined(RATE, B as u32);
    let region = SharedRegion::create_named(config.layout().unwrap().total_size).unwrap();
    let handle = region.handle();
    assert!(handle.starts_with("posix:/pvs."), "{handle}");
    let chan = Channel::create_in(region, config).unwrap();
    let mut plugin =
        PluginEnd::<SpinYieldWakeup>::attach(SharedRegion::open(&handle).unwrap()).unwrap();
    chan.region().unlink();
    assert!(SharedRegion::open(&handle).is_err(), "name still present");
    let (mut host, _monitor) =
        chan.into_ends_with::<SpinYieldWakeup>(options(WaitBudget::FractionOfBlock(0.0), 16));
    let x = noise(B * 3, 5);
    let mut y = vec![0.0f32; B];
    run(&mut host, &x[..B], &mut y);
    plugin.service(Duration::ZERO, gain);
    assert_eq!(run(&mut host, &x[B..2 * B], &mut y), BlockOutcome::Wet);
    assert_bits(&y, &gained(&x[..B]), "posix segment");
    assert!(
        y.iter()
            .zip(&x)
            .all(|(a, b)| a.to_bits() == (b * TEST_GAIN).to_bits())
    );
}

fn wakeup_bounds<W: Wakeup>(wakes: bool) {
    let bell: &'static Doorbell = Box::leak(Box::new(unsafe_zeroed_doorbell()));
    // An expired deadline returns at once.
    let t = Instant::now();
    assert!(!bell.wait_until::<W>(Deadline::after(Duration::ZERO), || false));
    assert!(t.elapsed() < Duration::from_millis(50));
    // A 30 ms deadline is honoured.
    let t = Instant::now();
    assert!(!bell.wait_until::<W>(Deadline::after(Duration::from_millis(30)), || false));
    let e = t.elapsed();
    assert!(
        e >= Duration::from_millis(29) && e < Duration::from_millis(500),
        "{e:?}"
    );
    // A changed word returns at once.
    assert_eq!(
        W::wait(&bell.seq, 12345, Deadline::after(Duration::from_secs(5))),
        WaitStatus::Woken
    );
    // A ring from another thread wakes a parked waiter well before its deadline.
    let flag: &'static std::sync::atomic::AtomicBool =
        Box::leak(Box::new(std::sync::atomic::AtomicBool::new(false)));
    let ringer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        flag.store(true, Ordering::Release);
        bell.ring::<W>();
    });
    let t = Instant::now();
    assert!(
        bell.wait_until::<W>(Deadline::after(Duration::from_secs(5)), || {
            flag.load(Ordering::Acquire)
        })
    );
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    ringer.join().unwrap();
    assert_eq!(bell.waiters.load(Ordering::SeqCst), 0);
    let _ = wakes;
}

fn unsafe_zeroed_doorbell() -> Doorbell {
    // SAFETY: a Doorbell is atomics only; all-zero is a valid (idle) doorbell.
    unsafe { std::mem::zeroed() }
}

#[test]
fn wakeup_waits_are_bounded() {
    #[cfg(target_os = "linux")]
    wakeup_bounds::<PlatformWakeup>(true);
    wakeup_bounds::<SpinYieldWakeup>(false);
}

/// H-36: a chunk's plugin → host events are on the ring before the next chunk is processed (and
/// before the chunk's output is published), even when one `service_with_events` call works
/// through several chunks — as it does in an offline render, where the host keeps publishing.
/// Before, the sandbox pushed them only after the whole call, so a restart request raised early
/// in a render reached the host after its last block.
#[test]
fn output_events_travel_with_their_chunk() {
    const B: usize = 64;
    let (mut plugin, mut host, _monitor) = setup::<SpinYieldWakeup>(
        ChannelConfig::pipelined(RATE, B as u32),
        options(WaitBudget::FractionOfBlock(0.0), 16),
    );
    let mut y = vec![0.0f32; B];
    // Three blocks published before the plugin runs at all (the host doesn't wait: budget 0).
    for _ in 0..3 {
        run(&mut host, &[0.1; B], &mut y);
    }
    let mut out_events = Vec::with_capacity(8);
    let mut chunk = 0u32;
    let mut seen_during: Vec<(u32, Vec<u32>)> = Vec::new();
    let served = plugin.service_with_events(Duration::ZERO, &mut out_events, |c, events| {
        apply_gain(c.input, c.output);
        // What the host can already pop while this chunk is being processed.
        let mut ids = Vec::new();
        while let Some(e) = host.pop_output_event() {
            ids.push(e.id);
        }
        seen_during.push((chunk, ids));
        events.push(WireEvent::param(c.pos, 100 + chunk, f64::from(chunk)));
        chunk += 1;
    });
    assert_eq!(
        served,
        Serviced::Processed {
            chunks: 3,
            frames: 3 * B as u64
        }
    );
    assert_eq!(
        seen_during,
        vec![(0, vec![]), (1, vec![100]), (2, vec![101])],
        "each chunk's event is out before the next chunk runs"
    );
    assert_eq!(host.pop_output_event().map(|e| e.id), Some(102));
    assert_eq!(host.pop_output_event(), None);
    assert!(out_events.is_empty(), "drained into the ring");
}
