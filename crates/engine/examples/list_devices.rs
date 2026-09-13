//! Lists real audio hosts and devices through the cpal backend (SPEC-001 manual smoke test).
//!
//! `cargo run -p vox-engine --example list_devices [-- --open]`
//!
//! With `--open`, briefly opens the default output of the default host (silence) and prints the
//! first callback's timestamps.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use vox_engine::backend::cpal::CpalBackend;
use vox_engine::backend::{Backend, CapsState, DirectionCaps, Enumerate, OutputTimestamp};
use vox_engine::devices::{offered_buffer_sizes, offered_sample_rates, resolve_devices};
use vox_engine::prefs::DevicePrefs;

fn describe(caps: &DirectionCaps) -> String {
    let rates: Vec<String> = caps
        .rates
        .iter()
        .map(|r| {
            if r.min_hz == r.max_hz {
                r.min_hz.to_string()
            } else {
                format!("{}-{}", r.min_hz, r.max_hz)
            }
        })
        .collect();
    let buffer = caps.buffer_range.map_or("unknown".to_owned(), |b| {
        format!("{}..={}", b.min_frames, b.max_frames)
    });
    format!(
        "{} ch (default {}), default {} Hz, rates [{}], offered {:?}, buffer {} (offered {:?} + Auto)",
        caps.max_channels,
        caps.default_channels,
        caps.default_rate_hz,
        rates.join(", "),
        offered_sample_rates(caps),
        buffer,
        offered_buffer_sizes(caps),
    )
}

fn main() {
    let open = std::env::args().any(|a| a == "--open");
    let backend = CpalBackend::new();
    let hosts = backend.hosts();
    let default = vox_engine::backend::choose_default_host(&hosts);
    println!(
        "hosts available: {:?}; default: {:?}",
        hosts.iter().map(|h| h.as_str()).collect::<Vec<_>>(),
        default.map(|h| h.as_str())
    );
    for host in &hosts {
        println!("\n== {host}");
        let started = std::time::Instant::now();
        let quick = backend.enumerate(*host, Enumerate::Quick);
        let quick_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        match quick.and_then(|_| backend.enumerate(*host, Enumerate::Cached)) {
            Ok(snap) => {
                println!(
                    "   ({} devices; names in {quick_ms} ms, capabilities in {} ms; default input: {:?}, default output: {:?})",
                    snap.devices.len(),
                    started.elapsed().as_millis(),
                    snap.default_input,
                    snap.default_output
                );
                for d in &snap.devices {
                    let mut flags = Vec::new();
                    if d.system_default {
                        flags.push("system default");
                    }
                    if d.input_is_monitor {
                        flags.push("input is a monitor");
                    }
                    println!(" - {} [id {}] {flags:?}", d.name, d.id);
                    for (label, state) in [("in ", &d.input), ("out", &d.output)] {
                        match state {
                            Some(CapsState::Known(c)) => println!("     {label}: {}", describe(c)),
                            Some(other) => println!("     {label}: {other:?}"),
                            None => {}
                        }
                    }
                }
                let started = std::time::Instant::now();
                let _ = backend.enumerate(*host, Enumerate::Quick);
                println!("   (next quick pass: {} ms)", started.elapsed().as_millis());
            }
            Err(e) => println!("   enumeration failed: {e}"),
        }
    }

    if open && let Some(host) = default {
        let snap = backend
            .enumerate(host, Enumerate::Cached)
            .expect("enumerate");
        let res = resolve_devices(&DevicePrefs::default(), host, &snap);
        let Some(req) = res.output_request() else {
            println!("\nno default output to open");
            return;
        };
        let first = Arc::new(AtomicU64::new(0));
        let latency = Arc::new(AtomicU64::new(0));
        let (f, l) = (first.clone(), latency.clone());
        let cb = move |d: &mut [f32], _: usize, ts: OutputTimestamp| {
            d.fill(0.0);
            let _ = f.compare_exchange(0, ts.callback_ns, Ordering::Relaxed, Ordering::Relaxed);
            l.store(
                ts.playback_ns.saturating_sub(ts.callback_ns),
                Ordering::Relaxed,
            );
        };
        match backend.open_output(&req, Box::new(cb)) {
            Ok(stream) => {
                std::thread::sleep(Duration::from_millis(500));
                println!(
                    "\nopened {:?}: {} callbacks in 500 ms, output latency (playback - callback) {:.2} ms, flags {:#x}",
                    stream.info(),
                    stream.status().callback_count(),
                    latency.load(Ordering::Relaxed) as f64 / 1e6,
                    stream.status().peek()
                );
            }
            Err(e) => println!("\nopen failed: {e}"),
        }
    }
}
