//! T-801 round-trip latency (ADR-008 follow-up): spawns the gain test plugin and drives it at
//! the real-time block rate (48 kHz), reporting the distribution of `HostEnd::process`
//! durations. Run by `just bench` (release).
//!
//! - `sync`: `L = 0`, the host waits for this block's own output (50 ms budget): the full round
//!   trip — write, wake, sandbox wake-up, process, publish, wake back.
//! - `pipelined`: the ADR-008 default (`L = B`, 25 % budget): misses and the host-side cost
//!   per callback (normally no wait at all).
//!
//! `futex` is the Linux primitive; `spin-yield` is the portable fallback (macOS/Windows today).
//! Neither side runs with real-time priority (the sandbox's rtkit request is T-802), so the
//! tails include ordinary scheduler latency. `VOX_SBX_BENCH_SECONDS` overrides the 3 s per row.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use vox_sandbox_ipc::{
    BlockOutcome, Channel, ChannelConfig, HostOptions, PeerState, PeerStatus, PlatformWakeup,
    SpinYieldWakeup, WaitBudget, Wakeup,
};

const RATE: f64 = 48_000.0;
const WARMUP: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sync,
    Pipelined,
}

fn main() {
    let seconds: f64 = std::env::var("VOX_SBX_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);
    println!("sandbox IPC round trip, 48 kHz, gain plugin in a child process, {seconds} s per row");
    println!("| block | period µs | mode | wakeup | blocks | p50 µs | p99 µs | max µs | misses |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for block in [64usize, 128, 256, 512] {
        run::<PlatformWakeup>(block, Mode::Sync, seconds);
        run::<SpinYieldWakeup>(block, Mode::Sync, seconds);
        run::<PlatformWakeup>(block, Mode::Pipelined, seconds);
    }
}

fn run<W: Wakeup>(block: usize, mode: Mode, seconds: f64) {
    let (config, options) = match mode {
        Mode::Sync => (
            ChannelConfig::synchronous(RATE, block as u32),
            HostOptions {
                wait: WaitBudget::Fixed(Duration::from_millis(50)),
                stall_after_misses: u32::MAX,
                ..HostOptions::default()
            },
        ),
        Mode::Pipelined => (
            ChannelConfig::pipelined(RATE, block as u32),
            HostOptions::default(),
        ),
    };
    let chan = Channel::create(config).expect("segment");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vox-sbx-test-gain"));
    let handle = chan.share_with(&mut cmd);
    cmd.arg("--shm")
        .arg(handle)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if W::NAME == SpinYieldWakeup::NAME {
        cmd.arg("--spin");
    }
    let mut child = cmd.spawn().expect("spawn the gain plugin");
    let t0 = Instant::now();
    while chan
        .control()
        .state
        .load(std::sync::atomic::Ordering::Acquire)
        != PeerState::Running as u32
    {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "the plugin never attached"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let (mut host, mut monitor) = chan.into_ends_with::<W>(options);

    let blocks = (seconds * RATE / block as f64) as usize + WARMUP;
    let input: Vec<f32> = (0..block)
        .map(|t| (0.5 * (std::f64::consts::TAU * 440.0 * t as f64 / RATE).sin()) as f32)
        .collect();
    let mut output = vec![0.0f32; block];
    let mut times = Vec::with_capacity(blocks);
    let mut misses = 0usize;
    let period = Duration::from_secs_f64(block as f64 / RATE);
    let start = Instant::now();
    for k in 0..blocks {
        let t = Instant::now();
        let outcome = host.process(&input, &mut output);
        let dt = t.elapsed();
        if k >= WARMUP {
            times.push(dt);
            if outcome != BlockOutcome::Wet {
                misses += 1;
            }
        }
        if let Some(d) = (start + period * (k as u32 + 1)).checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    monitor.request_shutdown();
    let t0 = Instant::now();
    while PeerStatus::of_child(&mut child) == PeerStatus::Alive
        && t0.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = child.kill();
    let _ = child.wait();

    times.sort_unstable();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let pick = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    println!(
        "| {block} | {:.0} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {misses} |",
        us(period),
        match mode {
            Mode::Sync => "sync",
            Mode::Pipelined => "pipelined",
        },
        W::NAME,
        times.len(),
        us(pick(0.5)),
        us(pick(0.99)),
        us(*times.last().expect("samples")),
    );
}
