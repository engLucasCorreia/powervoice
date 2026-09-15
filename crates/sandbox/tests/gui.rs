//! T-901: plugin editor windows owned by the sandbox, end to end through real
//! `powervoice-sandbox` processes and the test CLAP plugin's trivial GUI.
//!
//! Every test runs the sandbox with `POWERVOICE_SANDBOX_GUI=headless` (the editor opens against a
//! pretend window: no display needed, nothing ever appears on a developer's screen) — except
//! `a_real_x11_window_opens_and_closes`, which only runs with `POWERVOICE_TEST_GUI=1` and a
//! display, and closes its window within about a second.
//!
//! - the GUI's parameter edit reaches the rack's mirror while the plugin processes, and arrives
//!   as a notification while it doesn't;
//! - the GUI-only state change reaches the committed state (and a save's capture);
//! - a GUI that crashes takes only its sandbox down (restart policy; the window is gone), a GUI
//!   that hangs trips the watchdog;
//! - no window outlives its slot, its rack or its sandbox; the user closing it is reported.

vox_module_api::install_test_allocator!();

mod common;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, EditorRequest, ModuleFactory, ModuleRef, ModuleState, ParamId,
    ProcessMode, Transport, plugin_editor,
};
use vox_plugin_host::state::unwrap;
use vox_plugin_host::{SandboxFactory, SandboxFault, SandboxOptions};
use vox_rack::{
    LiveRack, MAX_BLOCK, RackError, RackHost, RackModel, RackNotice, RackOptions, SlotModel,
    SlotStatus,
};
use vox_test_clap as tc;

const GAIN_ID: &str = "clap:org.powervoice.test.gain";
const GUI_ENV: &str = "POWERVOICE_SANDBOX_GUI";

fn headless(mut o: SandboxOptions) -> SandboxOptions {
    o.env.push((GUI_ENV.into(), "headless".into()));
    o
}

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn request() -> EditorRequest {
    EditorRequest {
        title: "Test gain — PowerVoice".into(),
        parent: None,
    }
}

fn clap_slot(id: &str, gain_db: f64) -> SlotModel {
    let r: ModuleRef = format!("{id}@1.2.3").parse().unwrap();
    SlotModel::new(&r, false, &clap_gain_state(gain_db))
}

/// The test plugin's own state bytes of a rack slot's committed blob.
fn plugin_state(model: &RackModel, index: usize) -> Vec<u8> {
    let state: ModuleState = serde_json::from_value(model.slots[index].state.clone()).unwrap();
    let blob = state.blob.expect("a plugin blob");
    unwrap(&blob).unwrap().1.to_vec()
}

fn gui_marker(plugin_bytes: &[u8]) -> Option<u32> {
    (plugin_bytes.len() == 24).then(|| u32::from_le_bytes(plugin_bytes[20..24].try_into().unwrap()))
}

/// A rack over `factories` with `slots`, every slot loaded.
fn rack(factories: &[Arc<SandboxFactory>], slots: Vec<SlotModel>) -> (RackHost, LiveRack) {
    let (mut host, live) = RackHost::new(
        registry_with(factories),
        rt(),
        RackOptions::default(),
        &model(slots),
    )
    .unwrap();
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(t0.elapsed() < Duration::from_secs(10), "never loaded");
        host.tick();
        thread::sleep(Duration::from_millis(2));
    }
    host.tick();
    (host, live)
}

/// Drives the live rack (silence, allocation-checked) and ticks the host until `done` or
/// `timeout`; returns whether `done` held.
fn run_until(
    host: &mut RackHost,
    live: &mut LiveRack,
    notices: &mut Vec<RackNotice>,
    timeout: Duration,
    mut done: impl FnMut(&RackHost, &[RackNotice]) -> bool,
) -> bool {
    let x = [0.0f32; 256];
    let mut y = [0.0f32; 256];
    let t0 = Instant::now();
    let mut pos = 0u64;
    while t0.elapsed() < timeout {
        let t = Transport {
            playing: true,
            position_samples: Some(pos),
        };
        no_alloc(|| live.process(t, &x, &mut y)).expect("LiveRack allocated");
        pos += 256;
        notices.extend(host.tick());
        if done(host, notices) {
            return true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    done(host, notices)
}

/// A copy of the test plugin whose file name carries `marker` (the GUI's crash/hang switch).
fn marked_plugin(dir: &TempDir, marker: &str) -> std::path::PathBuf {
    let to = dir.0.join(format!("libvox_test_clap-{marker}.so"));
    std::fs::copy(test_clap_library(), &to).unwrap();
    to
}

#[test]
fn the_editor_opens_and_its_edit_reaches_the_rack_while_the_plugin_processes() {
    let f = clap_factory(tc::ID_GAIN, headless(exact_options()));
    let (mut host, mut live) = rack(std::slice::from_ref(&f), vec![clap_slot(GAIN_ID, 0.0)]);
    let info = host.slot_info(0).unwrap();
    assert!(info.sandboxed && info.has_editor && !info.editor_open);
    let mut notices = Vec::new();
    run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(50),
        |_, _| false,
    );

    let editor = host.editor_for_open(0, request()).unwrap();
    editor.open(&request()).unwrap();
    assert!(editor.is_open());
    let gain = ParamId(tc::PARAM_GAIN);
    assert!(
        run_until(
            &mut host,
            &mut live,
            &mut notices,
            Duration::from_secs(5),
            |h, _| { h.param_value(0, gain) == Some(tc::GUI_EDIT_GAIN_DB) }
        ),
        "the GUI's edit reached the mirror"
    );
    assert!(notices.iter().any(|n| matches!(
        n,
        RackNotice::ParamChanged { id, value, .. }
            if *id == gain && value.to_bits() == tc::GUI_EDIT_GAIN_DB.to_bits()
    )));
    assert!(notices.iter().any(|n| matches!(
        n,
        RackNotice::EditorChanged {
            index: 0,
            open: true,
            ..
        }
    )));
    assert!(host.slot_info(0).unwrap().editor_open);
    // The GUI-only change reaches the committed state (the document's dirty state follows it).
    assert!(
        run_until(
            &mut host,
            &mut live,
            &mut notices,
            Duration::from_secs(5),
            |h, _| { gui_marker(&plugin_state(&h.model(), 0)) == Some(tc::GUI_MARKER) }
        ),
        "the GUI-only state reached the committed blob"
    );
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::PluginStateChanged { index: 0, .. }))
    );
    let pid = f.live_instances()[0].pid;
    host.close_editor(0).unwrap();
    assert!(!host.slot_info(0).unwrap().editor_open);
    assert!(
        pid_exists(pid),
        "closing the window keeps the plugin running"
    );
    run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(50),
        |_, _| false,
    );
    assert_eq!(host.slot_info(0).unwrap().status, SlotStatus::Active);
    host.teardown(live);
}

#[test]
fn an_edit_while_the_plugin_is_inactive_arrives_as_a_notification() {
    let f = clap_factory(tc::ID_GAIN, headless(exact_options()));
    let m = f.create().unwrap();
    let editor = plugin_editor(m.as_ref()).unwrap();
    assert!(editor.available());
    editor.open(&request()).unwrap();
    let (mut params, mut state) = (Vec::new(), None);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) && (params.is_empty() || state.is_none()) {
        let u = editor.poll();
        assert!(u.open);
        params.extend(u.params);
        state = state.or(u.state);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        params,
        vec![(ParamId(tc::PARAM_GAIN), tc::GUI_EDIT_GAIN_DB)]
    );
    let blob = state.expect("the GUI-only change was reported");
    assert_eq!(gui_marker(unwrap(&blob).unwrap().1), Some(tc::GUI_MARKER));
    // The instance's own state agrees.
    let saved = m.save_state().unwrap();
    assert_eq!(
        gui_marker(unwrap(saved.blob.as_deref().unwrap()).unwrap().1),
        Some(tc::GUI_MARKER)
    );
    editor.close();
    assert!(!editor.is_open());
}

#[test]
fn a_save_captures_the_state_a_window_changed() {
    let f = clap_factory(tc::ID_GAIN, headless(exact_options()));
    let (mut host, mut live) = rack(std::slice::from_ref(&f), vec![clap_slot(GAIN_ID, 0.0)]);
    let editor = host.editor_for_open(0, request()).unwrap();
    editor.open(&request()).unwrap();
    let mut notices = Vec::new();
    let gain = ParamId(tc::PARAM_GAIN);
    assert!(run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_secs(5),
        |h, _| { h.param_value(0, gain) == Some(tc::GUI_EDIT_GAIN_DB) }
    ));
    // A save doesn't wait for the debounced report: it captures every used editor's state.
    let captured: Vec<_> = host
        .editors_to_capture()
        .into_iter()
        .map(|(uid, e)| (uid, e.capture_state().expect("captured")))
        .collect();
    assert_eq!(captured.len(), 1);
    for (uid, blob) in captured {
        host.apply_plugin_state(uid, blob);
    }
    assert_eq!(
        gui_marker(&plugin_state(&host.model(), 0)),
        Some(tc::GUI_MARKER)
    );
    host.teardown(live);
}

#[test]
fn a_crash_in_the_gui_takes_only_its_sandbox_down() {
    let dir = TempDir::new("gui-crash");
    let crashing = clap_factory_at(
        &marked_plugin(&dir, tc::GUI_CRASH_MARKER),
        tc::ID_GAIN,
        headless(exact_options()),
    );
    let healthy = clap_factory(tc::ID_LATENCY, headless(exact_options()));
    let (mut host, mut live) = rack(
        &[crashing.clone(), healthy.clone()],
        vec![
            clap_slot(GAIN_ID, 0.0),
            clap_slot("clap:org.powervoice.test.latency", 0.0),
        ],
    );
    let crashed_pid = crashing.live_instances()[0].pid;
    let healthy_pid = healthy.live_instances()[0].pid;
    let editor = host.editor_for_open(0, request()).unwrap();
    editor.open(&request()).unwrap();
    let mut notices = Vec::new();
    assert!(
        run_until(
            &mut host,
            &mut live,
            &mut notices,
            Duration::from_secs(10),
            |_, n| {
                n.iter()
                    .any(|n| matches!(n, RackNotice::SlotRestarted { index: 0, .. }))
            }
        ),
        "the crashed plugin was restarted once (T-802 policy)"
    );
    assert!(wait_until(Duration::from_secs(5), || !pid_exists(
        crashed_pid
    )));
    assert!(
        pid_exists(healthy_pid),
        "the other plugin's sandbox is untouched"
    );
    assert_eq!(host.slot_info(1).unwrap().status, SlotStatus::Active);
    assert!(!editor.is_open(), "the window died with its sandbox");
    assert!(notices.iter().any(|n| matches!(
        n,
        RackNotice::EditorChanged {
            index: 0,
            open: false,
            ..
        }
    )));
    // The restarted instance doesn't reopen a window that crashed.
    run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(300),
        |_, _| false,
    );
    assert!(!host.slot_info(0).unwrap().editor_open);
    host.teardown(live);
}

#[test]
fn a_hanging_gui_trips_the_watchdog() {
    let dir = TempDir::new("gui-hang");
    let mut options = headless(exact_options());
    options.editor_hang_timeout = Duration::from_millis(800);
    let f = clap_factory_at(
        &marked_plugin(&dir, tc::GUI_HANG_MARKER),
        tc::ID_GAIN,
        options,
    );
    let m = f.create().unwrap();
    let pid = f.live_instances()[0].pid;
    let editor = plugin_editor(m.as_ref()).unwrap();
    editor.open(&request()).unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || {
            f.live_instances()
                .first()
                .is_some_and(|i| i.fault == Some(SandboxFault::Hung))
        }),
        "the frozen main thread was noticed"
    );
    assert!(
        wait_until(Duration::from_secs(5), || !pid_exists(pid)),
        "and killed"
    );
    assert!(!editor.is_open());
    drop(m);
}

#[test]
fn no_window_outlives_its_slot_or_its_rack() {
    let f = clap_factory(tc::ID_GAIN, headless(exact_options()));
    let (mut host, mut live) = rack(
        std::slice::from_ref(&f),
        vec![clap_slot(GAIN_ID, 0.0), clap_slot(GAIN_ID, -3.0)],
    );
    let pids: Vec<u32> = f.live_instances().iter().map(|i| i.pid).collect();
    assert_eq!(pids.len(), 2);
    let first = host.editor_for_open(0, request()).unwrap();
    first.open(&request()).unwrap();
    let second = host.editor_for_open(1, request()).unwrap();
    second.open(&request()).unwrap();
    host.remove(0).unwrap();
    assert!(!first.is_open(), "closed with its slot, at once");
    assert!(second.is_open());
    let mut notices = Vec::new();
    run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(200),
        |_, _| false,
    );
    // Tear the rack down: the other window goes too, and no sandbox is left.
    host.teardown(live);
    assert!(!second.is_open());
    assert!(wait_until(Duration::from_secs(5), || pids
        .iter()
        .all(|&p| !pid_exists(p))));
}

#[test]
fn the_user_closing_the_window_is_reported() {
    let mut options = exact_options();
    options
        .env
        .push((GUI_ENV.into(), "headless:close-after-ms=150".into()));
    let f = clap_factory(tc::ID_GAIN, options);
    let (mut host, mut live) = rack(std::slice::from_ref(&f), vec![clap_slot(GAIN_ID, 0.0)]);
    let editor = host.editor_for_open(0, request()).unwrap();
    editor.open(&request()).unwrap();
    let mut notices = Vec::new();
    assert!(run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_secs(5),
        |_, n| {
            n.iter().any(|n| {
                matches!(
                    n,
                    RackNotice::EditorChanged {
                        index: 0,
                        open: false,
                        ..
                    }
                )
            })
        }
    ));
    assert!(!host.slot_info(0).unwrap().editor_open);
    // Opening again works (and raising an open window is fine too).
    let editor = host.editor_for_open(0, request()).unwrap();
    editor.open(&request()).unwrap();
    editor.open(&request()).unwrap();
    assert!(editor.is_open());
    host.teardown(live);
}

#[test]
fn a_plugin_without_a_window_offers_none() {
    let f = factory("gain", "gain", exact_options());
    let (mut host, _live) = rack(
        std::slice::from_ref(&f),
        vec![gain_slot("test:gain@1.0.0", 0.0)],
    );
    let info = host.slot_info(0).unwrap();
    assert!(info.sandboxed && !info.has_editor, "{info:?}");
    assert!(matches!(
        host.editor_for_open(0, request()),
        Err(RackError::NoEditor { .. })
    ));
    let m = f.create().unwrap();
    let editor = plugin_editor(m.as_ref()).unwrap();
    assert!(!editor.available());
    assert!(editor.open(&request()).is_err());
}

/// Opt-in (`POWERVOICE_TEST_GUI=1`, and an X display): the real window system. The window
/// appears for about a second, then closes.
#[test]
fn a_real_x11_window_opens_and_closes() {
    if std::env::var("POWERVOICE_TEST_GUI").as_deref() != Ok("1")
        || std::env::var_os("DISPLAY").is_none()
    {
        eprintln!("skipped: set POWERVOICE_TEST_GUI=1 (with an X display) to open a real window");
        return;
    }
    let f = clap_factory(tc::ID_GAIN, exact_options());
    let (mut host, mut live) = rack(std::slice::from_ref(&f), vec![clap_slot(GAIN_ID, 0.0)]);
    let editor = host.editor_for_open(0, request()).unwrap();
    let t0 = Instant::now();
    editor
        .open(&EditorRequest {
            title: "T-901 smoke — PowerVoice".into(),
            parent: None,
        })
        .unwrap();
    let mut notices = Vec::new();
    let gain = ParamId(tc::PARAM_GAIN);
    assert!(run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(900),
        |h, _| { h.param_value(0, gain) == Some(tc::GUI_EDIT_GAIN_DB) }
    ));
    run_until(
        &mut host,
        &mut live,
        &mut notices,
        Duration::from_millis(600),
        |_, _| false,
    );
    host.close_editor(0).unwrap();
    assert!(!editor.is_open());
    host.teardown(live);
    assert!(t0.elapsed() < Duration::from_secs(3));
}
