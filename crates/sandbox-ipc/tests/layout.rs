//! T-801: the shared-memory layout golden test (ADR-008 Amendment 1 §1). A failure here means
//! the segment layout changed: bump `LAYOUT_VERSION` and update the amendment.

use std::mem::{align_of, offset_of, size_of};
use std::sync::atomic::Ordering;

use vox_sandbox_ipc::layout::{
    CONTROL_SIZE, ControlBlock, EVENT_SLOT_SIZE, EventSlot, LAYOUT_VERSION, Layout, LayoutError,
    MAGIC, PeerState,
};
use vox_sandbox_ipc::wakeup::Doorbell;
use vox_sandbox_ipc::{
    Channel, ChannelConfig, ChannelError, PluginEnd, SharedRegion, SpinYieldWakeup,
};

#[test]
fn control_block_offsets_are_frozen() {
    let got = [
        ("magic", offset_of!(ControlBlock, magic)),
        ("version", offset_of!(ControlBlock, version)),
        ("control_size", offset_of!(ControlBlock, control_size)),
        ("total_size", offset_of!(ControlBlock, total_size)),
        (
            "sample_rate_bits",
            offset_of!(ControlBlock, sample_rate_bits),
        ),
        ("max_block", offset_of!(ControlBlock, max_block)),
        ("ring_capacity", offset_of!(ControlBlock, ring_capacity)),
        ("event_capacity", offset_of!(ControlBlock, event_capacity)),
        ("latency_samples", offset_of!(ControlBlock, latency_samples)),
        ("start_pos", offset_of!(ControlBlock, start_pos)),
        ("host_pid", offset_of!(ControlBlock, host_pid)),
        ("flags", offset_of!(ControlBlock, flags)),
        ("in_write_pos", offset_of!(ControlBlock, in_write_pos)),
        ("ev_in_write", offset_of!(ControlBlock, ev_in_write)),
        ("ev_out_read", offset_of!(ControlBlock, ev_out_read)),
        ("host_command", offset_of!(ControlBlock, host_command)),
        ("in_read_pos", offset_of!(ControlBlock, in_read_pos)),
        ("out_write_pos", offset_of!(ControlBlock, out_write_pos)),
        ("out_valid_from", offset_of!(ControlBlock, out_valid_from)),
        ("ev_in_read", offset_of!(ControlBlock, ev_in_read)),
        ("ev_out_write", offset_of!(ControlBlock, ev_out_write)),
        ("overruns", offset_of!(ControlBlock, overruns)),
        ("chunks", offset_of!(ControlBlock, chunks)),
        ("to_plugin", offset_of!(ControlBlock, to_plugin)),
        ("to_host", offset_of!(ControlBlock, to_host)),
        ("state", offset_of!(ControlBlock, state)),
        ("plugin_pid", offset_of!(ControlBlock, plugin_pid)),
        ("heartbeat", offset_of!(ControlBlock, heartbeat)),
        ("in_call", offset_of!(ControlBlock, in_call)),
        ("telemetry", offset_of!(ControlBlock, telemetry)),
    ];
    let want = [
        ("magic", 0),
        ("version", 8),
        ("control_size", 12),
        ("total_size", 16),
        ("sample_rate_bits", 24),
        ("max_block", 32),
        ("ring_capacity", 36),
        ("event_capacity", 40),
        ("latency_samples", 44),
        ("start_pos", 48),
        ("host_pid", 56),
        ("flags", 60),
        ("in_write_pos", 64),
        ("ev_in_write", 72),
        ("ev_out_read", 80),
        ("host_command", 88),
        ("in_read_pos", 128),
        ("out_write_pos", 136),
        ("out_valid_from", 144),
        ("ev_in_read", 152),
        ("ev_out_write", 160),
        ("overruns", 168),
        ("chunks", 176),
        ("to_plugin", 192),
        ("to_host", 256),
        ("state", 320),
        ("plugin_pid", 324),
        ("heartbeat", 328),
        ("in_call", 336),
        ("telemetry", 384),
    ];
    assert_eq!(got, want);
    assert_eq!(size_of::<ControlBlock>(), CONTROL_SIZE);
    assert_eq!(CONTROL_SIZE, 512);
    assert_eq!(align_of::<ControlBlock>(), 64);

    assert_eq!(offset_of!(Doorbell, seq), 0);
    assert_eq!(offset_of!(Doorbell, waiters), 4);
    assert_eq!(size_of::<Doorbell>(), 64);

    assert_eq!(offset_of!(EventSlot, pos), 0);
    assert_eq!(offset_of!(EventSlot, id), 8);
    assert_eq!(offset_of!(EventSlot, kind), 12);
    assert_eq!(offset_of!(EventSlot, value_bits), 16);
    assert_eq!(size_of::<EventSlot>(), EVENT_SLOT_SIZE);
    assert_eq!(EVENT_SLOT_SIZE, 32);

    assert_eq!(MAGIC.to_le_bytes(), *b"PVSBXIPC");
    assert_eq!(LAYOUT_VERSION, 1);
    assert_eq!(PeerState::Created as u32, 0);
    assert_eq!(PeerState::Running as u32, 1);
    assert_eq!(PeerState::Stopped as u32, 2);
    assert_eq!(PeerState::Failed as u32, 3);
}

#[test]
fn layout_sizes_are_frozen() {
    // (max_block, events) → (C, input, output, events_in, events_out, total)
    let cases = [
        ((1, 2), (64, 512, 768, 1024, 1088, 4096)),
        ((64, 512), (512, 512, 2560, 4608, 20_992, 40_960)),
        ((256, 512), (2048, 512, 8704, 16_896, 33_280, 53_248)),
        ((480, 64), (4096, 512, 16_896, 33_280, 35_328, 40_960)),
        ((1024, 512), (8192, 512, 33_280, 66_048, 82_432, 102_400)),
    ];
    for ((max_block, events), (c, i, o, ei, eo, total)) in cases {
        let l = Layout::new(max_block, events).unwrap();
        assert_eq!(
            (
                l.ring_capacity,
                l.input_offset,
                l.output_offset,
                l.events_in_offset,
                l.events_out_offset,
                l.total_size
            ),
            (c, i, o, ei, eo, total),
            "max_block {max_block}, events {events}"
        );
        assert_eq!(l.max_block, max_block);
        assert_eq!(l.event_capacity, events);
    }
    assert_eq!(Layout::new(0, 512), Err(LayoutError::MaxBlock(0)));
    assert_eq!(Layout::new(8193, 512), Err(LayoutError::MaxBlock(8193)));
    assert_eq!(Layout::new(256, 1), Err(LayoutError::EventCapacity(1)));
    assert_eq!(Layout::new(256, 3), Err(LayoutError::EventCapacity(3)));
    assert_eq!(
        Layout::new(256, 131_072),
        Err(LayoutError::EventCapacity(131_072))
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn initialised_header_bytes_are_frozen() {
    let chan = Channel::create(ChannelConfig {
        sample_rate: 48_000.0,
        max_block: 256,
        latency_samples: 256,
        event_capacity: 512,
    })
    .unwrap();
    // SAFETY: the first 384 bytes of a mapping only this thread touches (no peer attached).
    let mut bytes = unsafe { std::slice::from_raw_parts(chan.region().as_ptr(), 384) }.to_vec();
    bytes[56..60].fill(0); // host_pid varies
    let z = |n: usize| "0".repeat(n);
    let want = [
        // magic, version 1, control 512, total 53248, 48000.0, B 256, C 2048, E 512, L 256,
        // start 256, host_pid (masked), flags
        [
            "5056534258495043",
            "01000000",
            "00020000",
            "00d0000000000000",
            "000000000070e740",
            "00010000",
            "00080000",
            "00020000",
            "00010000",
            "0001000000000000",
            "00000000",
            "00000000",
        ]
        .concat(),
        // in_write_pos 256, ev_in_write, ev_out_read, host_command, reserved
        ["0001000000000000", &z(16), &z(16), &z(8), &z(72)].concat(),
        // in_read_pos 256, out_write_pos 256, out_valid_from 0, ev cursors, overruns, chunks
        ["0001000000000000", "0001000000000000", &z(96)].concat(),
        z(128), // doorbell → plugin
        z(128), // doorbell → host
        z(128), // state Created, pid, heartbeat, in_call
    ];
    for (line, want) in want.iter().enumerate() {
        assert_eq!(
            hex(&bytes[line * 64..(line + 1) * 64]),
            *want,
            "control block line {line}"
        );
    }
    assert_eq!(
        chan.control().host_pid.load(Ordering::Relaxed),
        std::process::id()
    );
}

fn config() -> ChannelConfig {
    ChannelConfig::pipelined(48_000.0, 128)
}

fn attach(region: SharedRegion) -> Result<PluginEnd<SpinYieldWakeup>, ChannelError> {
    PluginEnd::attach(region)
}

#[test]
fn attach_validates_the_header() {
    // Never initialised: no magic.
    let blank = SharedRegion::create(64 * 1024).unwrap();
    assert!(matches!(attach(blank), Err(ChannelError::BadMagic)));

    // Smaller than a control block.
    let tiny = SharedRegion::create(100).unwrap();
    assert!(matches!(attach(tiny), Err(ChannelError::TooSmall { .. })));

    let tamper = |f: &dyn Fn(&ControlBlock)| {
        let chan = Channel::create(config()).unwrap();
        f(chan.control());
        attach(chan.region().duplicate().unwrap())
    };
    assert!(matches!(
        tamper(&|cb| cb.version.store(2, Ordering::SeqCst)),
        Err(ChannelError::Version { found: 2 })
    ));
    assert!(matches!(
        tamper(&|cb| cb.ring_capacity.store(4096, Ordering::SeqCst)),
        Err(ChannelError::Header("ring_capacity"))
    ));
    assert!(matches!(
        tamper(&|cb| cb.total_size.store(1 << 30, Ordering::SeqCst)),
        Err(ChannelError::Header("total_size"))
    ));
    assert!(matches!(
        tamper(&|cb| {
            cb.max_block.store(0, Ordering::SeqCst);
            cb.latency_samples.store(0, Ordering::SeqCst);
        }),
        Err(ChannelError::Layout(LayoutError::MaxBlock(0)))
    ));
    assert!(matches!(
        tamper(&|cb| cb.latency_samples.store(129, Ordering::SeqCst)),
        Err(ChannelError::Config(_))
    ));
    assert!(matches!(
        tamper(&|cb| cb.start_pos.store(0, Ordering::SeqCst)),
        Err(ChannelError::Header("start_pos"))
    ));

    // A valid segment: attach marks the plugin running.
    let chan = Channel::create(config()).unwrap();
    let plugin = attach(chan.region().duplicate().unwrap()).unwrap();
    assert_eq!(plugin.config(), config());
    assert_eq!(
        chan.control().state.load(Ordering::SeqCst),
        PeerState::Running as u32
    );
    assert_eq!(
        chan.control().plugin_pid.load(Ordering::SeqCst),
        std::process::id()
    );
}

#[test]
fn config_validation() {
    let bad = |c: ChannelConfig| matches!(Channel::create(c), Err(ChannelError::Config(_)));
    assert!(bad(ChannelConfig {
        latency_samples: 257,
        ..ChannelConfig::pipelined(48_000.0, 256)
    }));
    assert!(bad(ChannelConfig::pipelined(0.0, 256)));
    assert!(bad(ChannelConfig::pipelined(f64::NAN, 256)));
    assert!(matches!(
        Channel::create(ChannelConfig::pipelined(48_000.0, 0)),
        Err(ChannelError::Layout(LayoutError::MaxBlock(0)))
    ));
}
