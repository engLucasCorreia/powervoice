//! T-805: the built-in Gain packaged as a CLAP module (`vox-voxmod-gain`, built with
//! `vox-module-clap`) through PowerVoice's own CLAP host in real `powervoice-sandbox`
//! processes (ADR-006 §4):
//! - the scan recognises `org.powervoice.module-info/1`: bare id, the module's own schema, whose
//!   `ParamInfo`s round-trip to the built-in's exactly;
//! - params, state, latency and tail through the adapter;
//! - bit-exact with the in-process Gain after the reported latency, with sample-accurate
//!   automation (realtime and offline);
//! - state: the CLAP state is `ModuleState` JSON; a built-in Gain state loads into the package
//!   and the package's loads into the built-in; a state saved while active follows automation;
//! - the ADR-005 `ModuleTestHost` suite passes through the adapter;
//! - opt-in: `clap-validator` (on `PATH`, or `CLAP_VALIDATOR=<binary>`) accepts the `.clap`.

// Exact comparisons are the point: state values and bit-exact output.
#![allow(clippy::float_cmp)]

vox_module_api::install_test_allocator!();

mod common;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use common::*;
use vox_module_api::test_util::{ModuleTestHost, no_alloc};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleFactory, ModuleState, OutputEvents, ParamEvent,
    ParamId, ProcessContext, ProcessMode, ProcessStatus, Tail, Transport, param_text,
    prepare_state,
};
use vox_modules::Gain;
use vox_plugin_host::scan::{ScannedPlugin, scan_file};
use vox_plugin_host::state::unwrap;
use vox_plugin_host::{SandboxFactory, clap_spec, packaged_module_info};

/// `vox_voxmod_gain::ID` (not linked here: its `clap_entry` would be exported by this test).
const ID: &str = "org.powervoice.gain.packaged";
const NAME: &str = "Gain (packaged)";

fn config(mode: ProcessMode, max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block,
        mode,
        layout: ChannelLayout::MONO,
    }
}

/// The packaged Gain as the sandbox scan reports it (scanned once per test binary).
fn scanned() -> &'static ScannedPlugin {
    static SCANNED: OnceLock<ScannedPlugin> = OnceLock::new();
    SCANNED.get_or_init(|| {
        let plugins = scan_file(
            std::path::Path::new(SANDBOX),
            &voxmod_gain_library(),
            Duration::from_secs(30),
        )
        .expect("the packaged Gain scans");
        assert_eq!(plugins.len(), 1, "{plugins:?}");
        plugins.into_iter().next().unwrap()
    })
}

fn factory() -> Arc<SandboxFactory> {
    Arc::new(SandboxFactory::new(
        clap_spec(&voxmod_gain_library(), scanned()),
        exact_options(),
    ))
}

/// Runs `m` over `input` in blocks cycling through `blocks`, with `(position, gain_db)` events at
/// absolute positions; every `process` under the allocation checker.
fn run(m: &mut dyn Module, input: &[f32], blocks: &[usize], events: &[(usize, f64)]) -> Vec<f32> {
    let mut out = vec![0.0f32; input.len()];
    let mut out_events = OutputEvents::with_capacity(64);
    let mut evs: Vec<ParamEvent> = Vec::with_capacity(16);
    let (mut pos, mut k) = (0usize, 0usize);
    while pos < input.len() {
        let n = blocks[k % blocks.len()].min(input.len() - pos);
        k += 1;
        evs.clear();
        for &(at, v) in events {
            if at >= pos && at < pos + n {
                evs.push(ParamEvent {
                    offset: (at - pos) as u32,
                    id: Gain::GAIN_DB,
                    value: v,
                });
            }
        }
        out_events.clear();
        let (i, o) = (&input[pos..pos + n], &mut out[pos..pos + n]);
        let status = no_alloc(|| {
            let mut ctx = ProcessContext::new(
                n as u32,
                pos as u64,
                Transport::default(),
                &evs,
                &mut out_events,
            );
            let mut outs = [o];
            m.process(&mut ctx, &[i], &mut outs)
        })
        .expect("process allocated");
        assert_eq!(status, ProcessStatus::Continue);
        pos += n;
    }
    out
}

/// The in-process Gain from `state` with gain events at absolute positions.
fn reference(input: &[f32], state: &ModuleState, events: &[(usize, f64)]) -> Vec<f32> {
    let mut g = Gain::new();
    let state = prepare_state(&g, state.clone()).unwrap();
    g.load_state(&state).unwrap();
    g.activate(&config(ProcessMode::Realtime, 512)).unwrap();
    run(&mut g, input, &[512], events)
}

fn packaged(state: &ModuleState, mode: ProcessMode, max_block: u32) -> Box<dyn Module> {
    let mut m = factory().create().unwrap();
    let state = prepare_state(&*m, state.clone()).unwrap();
    m.load_state(&state).unwrap();
    m.activate(&config(mode, max_block)).unwrap();
    m
}

/// The plugin's own state inside a proxy state blob, parsed as `ModuleState` JSON.
fn inner_state(state: &ModuleState) -> ModuleState {
    let blob = state
        .blob
        .as_deref()
        .expect("a proxy state carries the plugin's state");
    let (header, data) = unwrap(blob).unwrap();
    assert_eq!(header.id, ID);
    serde_json::from_slice(data).expect("the CLAP state is ModuleState JSON")
}

#[test]
fn the_scan_recognises_a_powervoice_module_and_registers_it_under_its_bare_id() {
    let p = scanned();
    assert_eq!(p.id, ID);
    assert_eq!(
        (p.name.as_str(), p.vendor.as_str(), p.version.as_str()),
        (NAME, "PowerVoice", "1.0.0")
    );
    for f in ["audio-effect", "mono"] {
        assert!(p.features.iter().any(|x| x == f), "{:?}", p.features);
    }
    assert_eq!(
        (p.param_count, p.main_input_channels, p.main_output_channels),
        (1, 1, 1)
    );
    assert!(p.module_info.is_some(), "module-info read by the scan");

    let spec = clap_spec(&voxmod_gain_library(), p);
    assert_eq!(spec.descriptor.id, ID, "bare id, not clap:{ID}");
    assert_eq!(spec.format, "clap");
    assert_eq!(spec.descriptor.name.text, NAME);
    assert_eq!(
        spec.descriptor.state_format_version,
        Gain::STATE_FORMAT_VERSION
    );

    // ADR-006 §4: the module-info JSON round-trips to the same `ParamInfo`s.
    let info = packaged_module_info(p).unwrap();
    assert_eq!(info.params, Gain::new().params());
    assert!(info.groups.is_empty());
    assert!(info.extensions.is_empty());
}

#[test]
fn params_state_latency_and_tail_through_the_adapter() {
    let mut m = factory().create().unwrap();
    assert_eq!(m.descriptor().id, ID);
    assert_eq!(m.params(), Gain::new().params(), "the module's own schema");
    assert!(m.groups().is_empty());
    assert_eq!(m.param_value(Gain::GAIN_DB), Some(0.0));
    assert!(
        param_text(&*m).is_none(),
        "a PowerVoice module's text follows the module API rules (host-side)"
    );

    m.activate(&config(ProcessMode::Realtime, 1024)).unwrap();
    assert_eq!(
        m.latency_samples(),
        256,
        "the transport block, no plugin latency"
    );
    assert_eq!(m.tail(), Tail::Samples(0));
    m.deactivate();
    m.activate(&config(ProcessMode::Offline, 4096)).unwrap();
    assert_eq!(m.latency_samples(), 4096);
    m.deactivate();
}

#[test]
fn the_packaged_gain_is_bit_exact_with_the_built_in_after_its_latency() {
    let x = noise(RATE as usize, 7);
    // Sample-accurate automation, including −∞ (silence) and a restart mid-ramp.
    let events = [
        (1_000, -12.0),
        (7_777, 3.5),
        (7_900, -1.0),
        (20_011, -60.0),
        (30_000, 6.0),
    ];
    let start = Gain::state_with_gain_db(-6.0);
    let want = reference(&x, &start, &events);
    for (mode, max_block, blocks) in [
        (ProcessMode::Realtime, 1024, vec![256usize, 128, 200]),
        (ProcessMode::Offline, 4096, vec![4096]),
    ] {
        let mut m = packaged(&start, mode, max_block);
        let latency = m.latency_samples() as usize;
        // The events reach the plugin at the same absolute positions; its output is delayed.
        let got = run(&mut *m, &x, &blocks, &events);
        assert!(got[..latency].iter().all(|v| *v == 0.0), "{mode:?}");
        assert_bits(
            &got[latency..],
            &want[..x.len() - latency],
            &format!("{mode:?}: packaged vs built-in Gain"),
        );
        m.deactivate();
    }
}

#[test]
fn states_move_between_the_built_in_and_the_package() {
    // Built-in → package: the built-in's state loads as is (same keys, same format).
    let builtin_state = Gain::state_with_gain_db(-6.0);
    let mut m = packaged(&builtin_state, ProcessMode::Realtime, 1024);
    assert_eq!(m.param_value(Gain::GAIN_DB), Some(-6.0));

    // The package's state is key-based: `{ "gain_db": … }`, and the plugin's own CLAP state is
    // the same `ModuleState` as JSON. Saved while active, it follows automation (the wrapper's
    // parameter mirror answers while the module is on the audio thread).
    let x = noise(4_096, 3);
    let _ = run(&mut *m, &x, &[256], &[(100, 3.0)]);
    let saved = m.save_state().unwrap();
    assert_eq!(saved.format_version, Gain::STATE_FORMAT_VERSION);
    assert_eq!(state_gain_db(&saved), 3.0);
    let inner = inner_state(&saved);
    assert_eq!(inner.params, Gain::state_with_gain_db(3.0).params);
    assert_eq!(inner.format_version, Gain::STATE_FORMAT_VERSION);
    m.deactivate();

    // Package → built-in.
    let mut g = Gain::new();
    let prepared = prepare_state(&g, saved.clone()).unwrap();
    g.load_state(&prepared).unwrap();
    assert_eq!(g.param_value(Gain::GAIN_DB), Some(3.0));

    // Package → package: a fresh instance restores it and renders the same audio.
    let x = noise(12_000, 11);
    let mut a = packaged(&saved, ProcessMode::Offline, 4096);
    let mut b = packaged(&Gain::state_with_gain_db(3.0), ProcessMode::Offline, 4096);
    assert_eq!(a.param_value(Gain::GAIN_DB), Some(3.0));
    assert_bits(
        &run(&mut *a, &x, &[4096], &[]),
        &run(&mut *b, &x, &[4096], &[]),
        "restored state",
    );
    a.deactivate();
    b.deactivate();
}

#[test]
fn the_module_test_host_suite_passes_through_the_adapter() {
    let f: Arc<dyn ModuleFactory> = factory();
    let report = ModuleTestHost::from_factory(f)
        .sample_rates(&[RATE])
        .blocks(24)
        .assert_passes();
    assert!(report.alloc_checked);
}

/// The directories of `PATH`, searched for `name`.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Opt-in (ticket T-805): the `.clap` passes `clap-validator` when it's installed — on `PATH`,
/// or named by `CLAP_VALIDATOR`. Skipped (with a note) otherwise.
#[test]
fn the_clap_passes_clap_validator_when_installed() {
    let validator = std::env::var_os("CLAP_VALIDATOR")
        .map(PathBuf::from)
        .or_else(|| on_path("clap-validator"));
    let Some(validator) = validator else {
        eprintln!("skipped: clap-validator isn't installed (set CLAP_VALIDATOR to run this check)");
        return;
    };
    let dir = TempDir::new("clap-validator");
    let plugin = dir.0.join("vox-voxmod-gain.clap");
    std::fs::copy(voxmod_gain_library(), &plugin).unwrap();
    let out = std::process::Command::new(&validator)
        .arg("validate")
        .arg(&plugin)
        .output()
        .expect("clap-validator runs");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    eprintln!("{text}");
    assert!(out.status.success(), "clap-validator failed:\n{text}");
}

#[test]
fn a_bare_id_never_uses_the_plain_clap_mapping() {
    // The plain CLAP spec of the same file (no module info) is `clap:<id>` with keys `p<id>`.
    let mut plain = scanned().clone();
    plain.module_info = None;
    let spec = clap_spec(&voxmod_gain_library(), &plain);
    assert_eq!(spec.descriptor.id, format!("clap:{ID}"));
    // A tampered module-info that names another id is ignored, never trusted.
    let mut other = scanned().clone();
    other.module_info = scanned()
        .module_info
        .as_ref()
        .map(|j| j.replace(ID, "org.powervoice.gain"));
    assert!(packaged_module_info(&other).is_none());
    assert_eq!(
        clap_spec(&voxmod_gain_library(), &other).descriptor.id,
        format!("clap:{ID}")
    );
    assert_eq!(ParamId(0), Gain::GAIN_DB);
}
