//! T-802: nothing outlives its owner — dropped proxies leave no sandbox process, no file
//! descriptor and no shared-memory object; an editor that exits or is killed takes its
//! sandboxes with it (`PR_SET_PDEATHSIG` on the watchdog thread, end of stream on stdin). One
//! sequential test: it counts this process's file descriptors.

vox_module_api::install_test_allocator!();

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleFactory, OutputEvents, ProcessContext,
    ProcessMode, Transport,
};

const FAKE_APP_ENV: &str = "VOX_T802_FAKE_APP";

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: 1024,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

/// H-120: the deadline every blocking wait on the `fake_app` child in this file uses (same value
/// and rationale as `crates/project/tests/crash.rs`'s `CHILD_TIMEOUT`, H-119) — generous enough
/// to absorb a machine under heavy parallel load, while still turning a genuinely stuck child
/// into a fast, readable test failure instead of a hang that eats the whole gate's ceiling.
const CHILD_TIMEOUT: Duration = Duration::from_secs(20);

/// Reads a child's stdout on a background thread and hands lines to the caller with a bounded
/// wait, so a stalled child fails the read in seconds instead of blocking the test thread forever
/// (H-119's pattern, copied from `crates/project/tests/crash.rs`).
struct LineReader {
    rx: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl LineReader {
    fn spawn(stdout: std::process::ChildStdout) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("teardown-crash-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the child's stdout reader thread");
        LineReader {
            rx,
            seen: Vec::new(),
        }
    }

    /// The next line within `timeout`. Panics naming `context` and every line seen from the child
    /// so far, if the deadline passes or the child's output ends before a line arrives.
    fn next_line(&mut self, timeout: Duration, context: &str) -> String {
        match self.rx.recv_timeout(timeout) {
            Ok(line) => {
                self.seen.push(line.clone());
                line
            }
            Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                "{context}: the child printed nothing for {timeout:?} (stalled or deadlocked); \
                 lines seen from it so far: {:?}",
                self.seen
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!(
                "{context}: the child's output ended before the expected line arrived; \
                 lines seen from it so far: {:?}",
                self.seen
            ),
        }
    }
}

/// Waits up to `timeout` for `child` to be reaped (after a kill, or a natural exit). Panics
/// naming `context` if it doesn't exit in time — a stuck OS wait shouldn't hang either.
fn reap_within(child: &mut Child, timeout: Duration, context: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() >= deadline => {
                panic!("{context}: the child didn't exit within {timeout:?}")
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("{context}: lost the child process while reaping it: {e}"),
        }
    }
}

fn spin(p: &mut dyn Module) {
    let x = vec![0.25f32; 256];
    let mut y = vec![0.0f32; 256];
    let mut oe = OutputEvents::with_capacity(4);
    for _ in 0..4 {
        let mut ctx = ProcessContext::new(256, 0, Transport::default(), &[], &mut oe);
        let mut outs = [&mut y[..]];
        p.process(&mut ctx, &[&x], &mut outs);
    }
}

#[test]
fn nothing_outlives_its_owner() {
    // Warm up the watchdog thread (lives for the process) before taking the baseline. The
    // proxy's own bookkeeping (`live_instances`) clears before its sandbox has actually finished
    // closing its pipes/shm and getting reaped (H-34), so the fd baseline is taken only once the
    // count has settled — a bounded poll, never a fixed sleep.
    let f = factory("gain", "gain", exact_options());
    drop(f.create().unwrap());
    assert!(wait_until(Duration::from_secs(3), || f
        .live_instances()
        .is_empty()));
    let fds = stable_fd_count(Duration::from_secs(3));
    let shm = pvs_shm_objects();

    let mut proxies: Vec<Box<dyn Module>> = (0..3).map(|_| f.create().unwrap()).collect();
    for p in proxies.iter_mut().take(2) {
        p.activate(&rt()).unwrap();
        spin(&mut **p);
    }
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    assert_eq!(pids.len(), 3);
    assert!(pids.iter().all(|p| pid_exists(*p)));
    proxies[1].deactivate();
    drop(proxies);
    assert!(
        wait_until(Duration::from_secs(5), || pids
            .iter()
            .all(|p| !pid_exists(*p))),
        "sandboxes survived their proxies"
    );
    assert!(
        wait_until(Duration::from_secs(5), || fd_count() == fds),
        "file descriptors leaked: {} → {}",
        fds,
        fd_count()
    );
    assert_eq!(pvs_shm_objects(), shm);

    // The editor exits (`process::exit`, no teardown) or is killed: its sandboxes go too.
    for mode in ["exit", "kill"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fake_app",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FAKE_APP_ENV, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        // H-120: bounded — an unbounded `lines.by_ref().find_map(...)`/`wait()` here was the hang
        // class H-119 fixed in `crates/project/tests/crash.rs`; a fake_app that stalled before
        // printing its sandbox pid (or exiting) used to block this test until the gate's outer
        // ceiling instead of failing fast.
        let mut reader = LineReader::spawn(child.stdout.take().unwrap());
        let context = format!("fake_app ({mode})");
        // libtest prints "test fake_app ... " without a newline: match anywhere in the line.
        let pid: u32 = loop {
            let line = reader.next_line(CHILD_TIMEOUT, &context);
            if let Some(v) = line
                .split("SANDBOX_PID=")
                .nth(1)
                .and_then(|v| v.trim().parse().ok())
            {
                break v;
            }
        };
        assert!(pid_exists(pid));
        if mode == "kill" {
            child.kill().unwrap();
        }
        reap_within(&mut child, CHILD_TIMEOUT, &context);
        assert!(
            wait_until(Duration::from_secs(5), || !pid_exists(pid)),
            "{mode}: the sandbox outlived the editor"
        );
    }
}

/// Helper process for `nothing_outlives_its_owner`: an "editor" with one active sandboxed slot.
#[test]
#[ignore = "helper process, run by nothing_outlives_its_owner"]
fn fake_app() {
    let Ok(mode) = std::env::var(FAKE_APP_ENV) else {
        return;
    };
    let f = factory("gain", "gain", exact_options());
    let mut p = f.create().unwrap();
    p.activate(&rt()).unwrap();
    spin(&mut *p);
    println!("SANDBOX_PID={}", f.live_instances()[0].pid);
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    if mode == "exit" {
        std::process::exit(0);
    }
    std::thread::sleep(Duration::from_secs(60));
    std::mem::forget(p);
}
