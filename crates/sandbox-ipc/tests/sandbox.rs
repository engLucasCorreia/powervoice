//! T-801: the four test plugins as real child processes (`CARGO_BIN_EXE_vox-sbx-test-*`):
//! the gain round trip is bit-exact (memfd and POSIX named segments, pipelined and
//! synchronous); crash / hang / slow → crossfaded dry on an allocation-free host "audio thread"
//! (paced at the real-time block rate), with the fault (or the misses) reported to the control
//! thread (this test's main thread) through `Monitor::poll`. Process-spawning tests live in
//! their own test binary (T-301/H-05 pattern).

vox_module_api::install_test_allocator!();

mod common;

use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::{RATE, assert_bits, expected_gain_output, max_step, noise, sine};
use vox_module_api::test_util::no_alloc;
use vox_sandbox_ipc::layout::ControlBlock;
use vox_sandbox_ipc::{
    BlockOutcome, Channel, ChannelConfig, Fault, HostEnd, HostOptions, Monitor, PeerState,
    PeerStatus, WaitBudget,
};

const GAIN: &str = env!("CARGO_BIN_EXE_vox-sbx-test-gain");
const CRASH: &str = env!("CARGO_BIN_EXE_vox-sbx-test-crash");
const HANG: &str = env!("CARGO_BIN_EXE_vox-sbx-test-hang");
const SLOW: &str = env!("CARGO_BIN_EXE_vox-sbx-test-slow");

/// Kills the child if the test ends first.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(bin: &str, chan: &Channel, extra: &[&str]) -> KillOnDrop {
    let mut cmd = Command::new(bin);
    let handle = chan.share_with(&mut cmd);
    cmd.arg("--shm")
        .arg(handle)
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    KillOnDrop(cmd.spawn().unwrap())
}

fn wait_attached(cb: &ControlBlock, child: &mut Child) {
    let t0 = Instant::now();
    while cb.state.load(Ordering::Acquire) != PeerState::Running as u32 {
        assert!(
            child.try_wait().unwrap().is_none(),
            "the plugin exited before attaching"
        );
        assert!(t0.elapsed() < Duration::from_secs(10), "no attach");
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let t0 = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "the plugin didn't exit"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

struct Run {
    output: Vec<f32>,
    outcomes: Vec<BlockOutcome>,
    violations: u32,
}

/// The host "audio thread": consecutive `block`-frame callbacks over `input`, each `process`
/// under the allocation checker; paced at the block period when `paced`.
fn audio_thread(mut host: HostEnd, input: Vec<f32>, block: usize, paced: bool) -> JoinHandle<Run> {
    thread::spawn(move || {
        let mut output = vec![0.0f32; input.len()];
        let mut outcomes = Vec::with_capacity(input.len() / block + 1);
        let mut violations = 0;
        let period = Duration::from_secs_f64(block as f64 / RATE);
        let start = Instant::now();
        for (k, (i, o)) in input
            .chunks(block)
            .zip(output.chunks_mut(block))
            .enumerate()
        {
            match no_alloc(|| host.process(i, o)) {
                Ok(outcome) => outcomes.push(outcome),
                Err(n) => {
                    violations += n;
                    outcomes.push(BlockOutcome::Missed);
                }
            }
            if paced
                && let Some(d) =
                    (start + period * (k as u32 + 1)).checked_duration_since(Instant::now())
            {
                thread::sleep(d);
            }
        }
        drop(host);
        Run {
            output,
            outcomes,
            violations,
        }
    })
}

/// Polls (control thread) until the first fault; returns it with the misses reported so far.
fn poll_until_fault(monitor: &mut Monitor, child: &mut Child) -> (Fault, u64) {
    let t0 = Instant::now();
    let mut misses = 0;
    loop {
        let h = monitor.poll(PeerStatus::of_child(child));
        misses += h.new_misses;
        if let Some(f) = h.new_fault {
            return (f, misses);
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "no fault: {h:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn gain_round_trip(chan: Channel, block: usize, unlink_after_attach: bool) {
    let config = chan.config();
    let mut child = spawn(GAIN, &chan, &[]);
    wait_attached(chan.control(), &mut child.0);
    if unlink_after_attach {
        chan.region().unlink();
    }
    let (host, mut monitor) = chan.into_ends(HostOptions {
        wait: WaitBudget::Fixed(Duration::from_secs(5)),
        ..HostOptions::default()
    });
    let x = noise(block * 400, 21);
    let run = audio_thread(host, x.clone(), block, false).join().unwrap();
    assert_eq!(run.violations, 0, "the host allocated");
    assert!(
        run.outcomes.iter().all(|o| *o == BlockOutcome::Wet),
        "{:?}",
        monitor.counters()
    );
    assert_bits(
        &run.output,
        &expected_gain_output(&x, config.latency_samples as usize),
        "gain round trip",
    );
    monitor.request_shutdown();
    assert!(wait_exit(&mut child.0).success());
    let h = monitor.poll(PeerStatus::Exited);
    assert_eq!((h.fault, h.state), (None, PeerState::Stopped));
    assert_eq!(h.counters.missed, 0);
}

#[test]
fn gain_round_trip_is_bit_exact_pipelined() {
    gain_round_trip(
        Channel::create(ChannelConfig::pipelined(RATE, 128)).unwrap(),
        128,
        false,
    );
}

#[test]
fn gain_round_trip_is_bit_exact_synchronous() {
    gain_round_trip(
        Channel::create(ChannelConfig::synchronous(RATE, 256)).unwrap(),
        256,
        false,
    );
}

#[cfg(unix)]
#[test]
fn gain_round_trip_is_bit_exact_over_a_named_segment() {
    let config = ChannelConfig::pipelined(RATE, 64);
    let region =
        vox_sandbox_ipc::SharedRegion::create_named(config.layout().unwrap().total_size).unwrap();
    gain_round_trip(Channel::create_in(region, config).unwrap(), 64, true);
}

/// The tail after a fault is exactly the latency-matched dry signal.
fn assert_dry_tail(run: &Run, x: &[f32], block: usize, blocks: usize) {
    let n = run.output.len();
    let from = n - blocks * block;
    assert_bits(
        &run.output[from..],
        &x[from - block..n - block],
        "latency-matched dry tail",
    );
}

const FAULT_BLOCK: usize = 128;
const FAULT_BLOCKS: usize = 400; // ≈ 1.07 s at 48 kHz

fn fault_run(bin: &str, extra: &[&str], options: HostOptions) -> (Fault, u64, Run, Vec<f32>) {
    let chan = Channel::create(ChannelConfig::pipelined(RATE, FAULT_BLOCK as u32)).unwrap();
    let mut child = spawn(bin, &chan, extra);
    wait_attached(chan.control(), &mut child.0);
    let (host, mut monitor) = chan.into_ends(options);
    let x = sine(FAULT_BLOCK * FAULT_BLOCKS, 220.0, 0.5);
    let audio = audio_thread(host, x.clone(), FAULT_BLOCK, true);
    let (fault, misses) = poll_until_fault(&mut monitor, &mut child.0);
    let run = audio.join().unwrap();
    assert_eq!(run.violations, 0, "the host allocated");
    let c = monitor.counters();
    assert!(c.bypassed > 0, "{c:?}");
    assert_eq!(run.outcomes.last(), Some(&BlockOutcome::Bypassed));
    assert!(
        !run.outcomes[..30].contains(&BlockOutcome::Bypassed),
        "bypassed before the fault"
    );
    // Crossfaded: never a jump (a hard wet → dry switch would step by up to 0.25).
    assert!(
        max_step(&run.output) < 0.03,
        "discontinuity {}",
        max_step(&run.output)
    );
    assert_dry_tail(&run, &x, FAULT_BLOCK, 50);
    (fault, misses, run, x)
}

/// H-50: crash detection (`Fault::Crashed`) comes from the child's real exit status
/// (`Monitor::poll_at`'s `peer == PeerStatus::Exited`), polled independently of `HostEnd`'s own
/// wait — it doesn't depend on, or get delayed by, the host's per-block wait budget at all.
/// `HostOptions::default()`'s `FractionOfBlock(0.25)` is a genuine RT deadline (~0.67 ms at
/// 128 frames/48 kHz) with no bearing on that detection, so it only added a wall-clock race
/// against real OS scheduling of the plugin's real child process: under a loaded machine the
/// child could occasionally miss that tiny window for an ordinary (pre-crash) block, wrongly
/// reporting it `Missed` (this test flaked under full-suite load). The fix waits
/// deterministically instead of racing a tight budget that isn't what this test is about (that
/// RT tightness *is* what `slow_misses_are_counted_crossfaded_and_not_a_fault` exercises, on the
/// SLOW plugin, on purpose) — `process_chunk`'s wait already wakes at once on a real fault
/// (`wet_ready(..) || bypass.load(..)`), so this doesn't slow crash detection or reporting.
#[test]
fn crash_is_bypassed_and_reported() {
    let (fault, _, run, _) = fault_run(
        CRASH,
        &["--after", "40"],
        HostOptions {
            wait: WaitBudget::Fixed(Duration::from_secs(1)),
            ..HostOptions::default()
        },
    );
    assert_eq!(fault, Fault::Crashed);
    assert!(
        run.outcomes[..30].iter().all(|o| *o == BlockOutcome::Wet),
        "{:?}",
        &run.outcomes[..30]
    );
}

/// Unlike crash detection above, `Fault::Hung` *is* timed off the host's own wait budget: the
/// plugin never returns, so blocks genuinely miss the deadline until `hang_timeout` (250 ms)
/// confirms the fault — this test's `misses > 0` checks exactly that graceful-degradation
/// window, so it keeps the real RT default (`HostOptions::default()`), unlike the crash test
/// above.
#[test]
fn hang_is_bypassed_and_reported() {
    let (fault, misses, _, _) = fault_run(HANG, &["--after", "40"], HostOptions::default());
    match fault {
        Fault::Hung { stale, in_call } => {
            assert!(stale >= Duration::from_millis(250), "{stale:?}");
            assert!(in_call, "the hang is inside the plugin call");
        }
        other => panic!("expected a hang, got {other:?}"),
    }
    assert!(misses > 0, "the hang produced misses before the fault");
}

#[test]
fn slow_misses_are_counted_crossfaded_and_not_a_fault() {
    let block = 128;
    let blocks = 400;
    let chan = Channel::create(ChannelConfig::pipelined(RATE, block as u32)).unwrap();
    let mut child = spawn(
        SLOW,
        &chan,
        &[
            "--late-percent",
            "10",
            "--late-periods",
            "2.5",
            "--seed",
            "7",
        ],
    );
    wait_attached(chan.control(), &mut child.0);
    let (host, mut monitor) = chan.into_ends(HostOptions::default());
    let x = sine(block * blocks, 220.0, 0.5);
    let audio = audio_thread(host, x.clone(), block, true);
    let mut reported = 0;
    while !audio.is_finished() {
        let h = monitor.poll(PeerStatus::of_child(&mut child.0));
        assert_eq!(h.fault, None, "{h:?}");
        reported += h.new_misses;
        thread::sleep(Duration::from_millis(5));
    }
    let run = audio.join().unwrap();
    let h = monitor.poll(PeerStatus::of_child(&mut child.0));
    assert_eq!(h.fault, None, "{h:?}");
    reported += h.new_misses;
    assert_eq!(run.violations, 0, "the host allocated");
    let c = h.counters;
    assert!(c.missed > 0, "the slow plugin never missed: {c:?}");
    assert_eq!(reported, c.missed, "every miss reaches the control thread");
    assert_eq!(c.bypassed, 0);
    assert!(
        max_step(&run.output) < 0.03,
        "discontinuity {}",
        max_step(&run.output)
    );
    let want = expected_gain_output(&x, block);
    let mut exact = 0;
    for k in 1..blocks {
        if run.outcomes[k] == BlockOutcome::Wet && run.outcomes[k - 1] == BlockOutcome::Wet {
            let r = k * block..(k + 1) * block;
            assert_bits(&run.output[r.clone()], &want[r], "fully wet block");
            exact += 1;
        }
    }
    assert!(exact > blocks / 2, "only {exact} fully wet blocks");
    monitor.request_shutdown();
    assert!(wait_exit(&mut child.0).success());
}
