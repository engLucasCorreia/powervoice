//! T-802: nothing outlives its owner — dropped proxies leave no sandbox process, no file
//! descriptor and no shared-memory object; an editor that exits or is killed takes its
//! sandboxes with it (`PR_SET_PDEATHSIG` on the watchdog thread, end of stream on stdin). One
//! sequential test: it counts this process's file descriptors.

vox_module_api::install_test_allocator!();

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

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
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        // libtest prints "test fake_app ... " without a newline: match anywhere in the line.
        let pid: u32 = lines
            .by_ref()
            .map_while(Result::ok)
            .find_map(|l| {
                l.split("SANDBOX_PID=")
                    .nth(1)
                    .and_then(|v| v.trim().parse().ok())
            })
            .expect("the fake app never reported its sandbox");
        assert!(pid_exists(pid));
        if mode == "kill" {
            child.kill().unwrap();
        }
        child.wait().unwrap();
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
