//! H-100: transfer curve generation benchmark. Measures the time to compute the Dynamics
//! transfer curve for varying point counts. The spec budget (AC-23, SPEC-016 §4.12) is 5 ms for
//! 512 points in a release build. This benchmark reports the actual timing in release mode,
//! replacing the flaky wall-clock assertion that previously lived in the test suite
//! (crates/engine/tests/rack_api.rs::transfer_curve_encodes_a_vxtc_frame_within_the_time_budget).

use std::sync::Arc;
use std::time::Instant;

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{EngineConfig, HostId, ManualEngine, RackCommand};
use vox_modules::Dynamics;
use vox_rack::Registry;
use vox_testkit::bench_report::{self, Target};

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn rig() -> ManualEngine {
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    eng
}

fn main() {
    let mut eng = rig();

    // Add a Dynamics module to the rack so we can call transfer_curve.
    eng.rack_command(RackCommand::Add {
        module_id: Dynamics::ID.into(),
        index: 0,
    })
    .expect("add Dynamics");

    // Benchmark varying point counts (16, 64, 256, 512 points) which affect the binary frame
    // size proportionally.
    for points in &[16, 64, 256, 512] {
        let mut times = Vec::new();

        for _ in 0..50 {
            let start = Instant::now();
            eng.transfer_curve(0, -80.0, 6.0, *points)
                .expect("transfer_curve")
                .encode(0);
            times.push(start.elapsed().as_secs_f64() * 1e3); // Convert to ms
        }

        times.sort_by(f64::total_cmp);
        let min = times[0];
        let p50 = times[times.len() / 2];
        let p95 = times[times.len() * 95 / 100];
        let max = times[times.len() - 1];

        let target = if *points <= 64 {
            Target::le(1.0) // Smaller point counts should be < 1 ms
        } else if *points == 256 {
            Target::le(3.0) // 256 points < 3 ms
        } else {
            Target::le(5.0) // 512 points budget is 5 ms per AC-23, SPEC-016 §4.12
        };

        eprintln!(
            "transfer_curve({points} points): min={min:.2}ms, p50={p50:.2}ms, p95={p95:.2}ms, max={max:.2}ms"
        );
        bench_report::result(
            "vox-engine",
            &format!("transfer_curve_{points}_points_ms"),
            p95,
            "ms",
            Some(target),
        );
    }
}
