//! The four test plugins (T-801), shared by the `vox-sbx-test-*` binaries:
//!
//! | Binary | Behaviour |
//! |---|---|
//! | `vox-sbx-test-gain` | output = input × [`TEST_GAIN`] (−6 dB) |
//! | `vox-sbx-test-crash` | gain, then `abort()` inside the callback before chunk `--after` |
//! | `vox-sbx-test-hang` | gain, then sleeps forever inside the callback before chunk `--after` |
//! | `vox-sbx-test-slow` | gain, but `--late-percent` % of chunks (seeded PRNG) first sleep `--late-periods` × the chunk's period |
//!
//! Arguments: `--shm <handle>` (required), `--after <chunks>` (default 50), `--seed <n>`,
//! `--late-percent <0..=100>` (default 10), `--late-periods <x>` (default 2.5), `--spin` (use
//! [`SpinYieldWakeup`] instead of the platform primitive). They exit when the host requests
//! shutdown (code 0), when the host process is gone, and (Linux) when the spawning thread dies
//! (`PR_SET_PDEATHSIG`).

use std::process::ExitCode;
use std::time::Duration;

use crate::plugin::{PluginEnd, Serviced};
use crate::shm::SharedRegion;
use crate::wakeup::{PlatformWakeup, SpinYieldWakeup, Wakeup};

/// −6 dB as one `f32` literal (10^(−6/20) = 0.501 187 23…), shared by the plugins and the tests,
/// so both sides compute `x × TEST_GAIN` identically.
pub const TEST_GAIN: f32 = 0.501_187_2;

/// The test plugins' idle wait (the heartbeat moves at least this often).
pub const IDLE_TIMEOUT: Duration = Duration::from_millis(20);

/// Which test plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestPluginKind {
    /// −6 dB.
    Gain,
    /// Aborts after `after` chunks.
    Crash,
    /// Hangs after `after` chunks.
    Hang,
    /// Randomly late.
    Slow,
}

/// Parsed command line.
#[derive(Clone, Debug, PartialEq)]
pub struct TestPluginArgs {
    /// Segment handle.
    pub shm: String,
    /// Chunks before crashing / hanging.
    pub after_chunks: u64,
    /// PRNG seed (slow).
    pub seed: u64,
    /// Percentage of late chunks (slow).
    pub late_percent: u32,
    /// Lateness in chunk periods (slow).
    pub late_periods: f64,
    /// Use the spin-then-yield wakeup.
    pub spin: bool,
}

impl TestPluginArgs {
    /// Parses `args` (without the program name).
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self {
            shm: String::new(),
            after_chunks: 50,
            seed: 1,
            late_percent: 10,
            late_periods: 2.5,
            spin: false,
        };
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--shm" => out.shm = value("--shm")?,
                "--after" => out.after_chunks = parse_num(&value("--after")?)?,
                "--seed" => out.seed = parse_num(&value("--seed")?)?,
                "--late-percent" => out.late_percent = parse_num(&value("--late-percent")?)?,
                "--late-periods" => out.late_periods = parse_num(&value("--late-periods")?)?,
                "--spin" => out.spin = true,
                other => return Err(format!("unknown argument {other}")),
            }
        }
        if out.shm.is_empty() {
            return Err("--shm <handle> is required".into());
        }
        Ok(out)
    }
}

fn parse_num<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("bad number {s}"))
}

/// The gain plugin's processing: `output[i] = input[i] × TEST_GAIN`.
pub fn apply_gain(input: &[f32], output: &mut [f32]) {
    for (o, &i) in output.iter_mut().zip(input) {
        *o = i * TEST_GAIN;
    }
}

/// Entry point of the `vox-sbx-test-*` binaries.
pub fn main(kind: TestPluginKind) -> ExitCode {
    let args = match TestPluginArgs::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    die_with_parent();
    let region = match SharedRegion::open(&args.shm) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("open {}: {e}", args.shm);
            return ExitCode::from(3);
        }
    };
    if args.spin {
        serve::<SpinYieldWakeup>(kind, &args, region)
    } else {
        serve::<PlatformWakeup>(kind, &args, region)
    }
}

fn serve<W: Wakeup>(kind: TestPluginKind, args: &TestPluginArgs, region: SharedRegion) -> ExitCode {
    let mut end = match PluginEnd::<W>::attach(region) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("attach: {e}");
            return ExitCode::from(4);
        }
    };
    let host_pid = end.host_pid();
    let sample_rate = end.config().sample_rate;
    let mut chunks: u64 = 0;
    let mut rng = args.seed | 1;
    loop {
        let served = end.service(IDLE_TIMEOUT, |chunk| {
            match kind {
                TestPluginKind::Crash if chunks >= args.after_chunks => crash_now(),
                TestPluginKind::Hang if chunks >= args.after_chunks => hang_forever(),
                TestPluginKind::Slow => {
                    // xorshift64
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    if rng % 100 < u64::from(args.late_percent) {
                        let period = chunk.input.len() as f64 / sample_rate;
                        std::thread::sleep(Duration::from_secs_f64(period * args.late_periods));
                    }
                }
                _ => {}
            }
            apply_gain(chunk.input, chunk.output);
            chunks += 1;
        });
        match served {
            Serviced::Shutdown => {
                end.stop();
                return ExitCode::SUCCESS;
            }
            Serviced::Idle if !process_alive(host_pid) => return ExitCode::from(5),
            _ => {}
        }
    }
}

/// Aborts the process without leaving a core dump (the crash plugins; T-802's test backend).
pub fn crash_now() -> ! {
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_DUMPABLE 0 only marks this process non-dumpable, so the deliberate abort
    // below leaves no core dump (and no crash-reporter notification).
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    std::process::abort()
}

/// Sleeps forever (the hang plugins; T-802's test backend).
pub fn hang_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// Linux: asks the kernel to SIGKILL this process when the thread that spawned it exits
/// (`PR_SET_PDEATHSIG`). No-op elsewhere.
pub fn die_with_parent() {
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_PDEATHSIG only asks the kernel to SIGKILL this process when its parent
    // (thread) exits.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
    }
}

/// Whether process `pid` still exists (`kill(pid, 0)`; unknown → true).
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if pid <= 0 {
        return true;
    }
    // SAFETY: `kill(pid, 0)` sends nothing; it only probes whether `pid` exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Whether process `pid` still exists (unknown → true).
#[cfg(not(unix))]
pub fn process_alive(_pid: u32) -> bool {
    true
}
