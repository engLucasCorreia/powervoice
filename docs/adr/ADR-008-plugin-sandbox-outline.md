# ADR-008 — Plugin sandbox outline
- Status: accepted (owner, M0 checkpoint 2026-09-12)
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
PROMPT §2 (LOCKED) requires external plugins to run **out of process**, with shared-memory audio, so
that "a crashing plugin never takes down the editor or a recording". The rest of the requirements
come from PROMPT §3.7:
- a separate `plugin-sandbox` process per plugin or per chain (this ADR decides);
- shared-memory rings plus a control channel;
- a watchdog, and silence/bypass on failure with a user notice and a flag;
- added latency reported through `latency_samples()`;
- scanning inside the sandbox;
- native editors in M9, using X11/XWayland on Linux.

This is the outline M8 builds on (T-801 ring/wakeup, T-802 process/watchdog/proxy, T-804 scanning,
T-901 editors). Real-time rules (CLAUDE.md, ADR-002) apply to the engine side. Installed PowerVoice
modules run here too (ADR-006).

## Decision

### 1. One sandbox process per plugin instance
Each external plugin instance in the rack gets its own `powervoice-plugin-sandbox` process. Reasons:
- **Exact blame:** a crash or hang identifies exactly one plugin, so flagging and blocklisting are
  precise.
- **Minimal blast radius:** other slots keep running.
- **Simple recovery:** respawn one process and reload one state.
- A voice-over rack is small (typically 0–3 external plugins), so the costs (about 10–30 MB per
  process, one wakeup per slot per block) are negligible.

yabridge, Bitwig and REAPER all offer per-plugin isolation, alongside grouped modes. Our wire protocol
addresses *instances inside* a process, so "sandbox groups" (one process per chain, or per vendor) can
be added later as a policy change if T-801 measurements show that per-slot wakeups cost too much.

```mermaid
flowchart LR
  subgraph Editor process
    RT[audio thread\nrack → ProxyModule] -- write input + events, wake --> SHM[(shared memory\nSPSC rings)]
    SHM -- output after B samples --> RT
    CTL[control thread\nplugin-host] <-- control channel --> SBX
    WD[watchdog] -. heartbeat .- SHM
  end
  subgraph Sandbox process per plugin
    SBX[main thread: control + plugin main-thread calls + GUI] --> PL[plugin]
    SA[sandbox audio thread\nwaits on wakeup] --> PL
    SA <--> SHM
  end
```

### 2. Audio path: pipelined, never blocking the editor's audio thread
- On the engine side each sandboxed plugin is a **`ProxyModule`** (ADR-005 `Module`, in
  `plugin-host`).
- Per instance, one shared-memory segment holds:
  - an **input sample ring**, an **output sample ring** and a **parameter-event ring** (events stamped
    with absolute stream positions);
  - telemetry cells;
  - a header with the protocol version, sizes, sequence counters, heartbeat and a "currently in
    plugin call" marker.

  Everything is sized at `activate` for `max_block`.
- **Realtime mode:** each `process()` call:
  1. writes the block's input and events;
  2. issues one **non-blocking wake**;
  3. reads output delayed by exactly **B = max_block** samples.

  The sandbox always has at least one callback period to produce it. If the output isn't there yet,
  that is a **miss**: the proxy outputs its latency-matched dry signal for the block and counts it. The
  editor's audio thread never waits on another process, so a slow plugin causes a local glitch instead
  of an engine xrun.
- **Offline mode** (export, bake, ACX, CLI): same layout and same latency B, but the proxy **blocks**
  until the output is ready (with the hang timeout). No miss substitution, so renders are
  deterministic.
- **Choosing B.** ADR-002's realtime `MAX_BLOCK` = 1024 is only a ceiling; with it, B would add
  21 ms. The rack therefore activates each sandboxed slot with `max_block` = the current device
  period, rounded up to a power of two and at most 1024, and splits sub-blocks further for that slot
  only.
  - If the device period grows while active, callbacks exceed B and produce misses. The control
    thread sees the period change (RT event) and replaces the proxy with a new B (`Restart`).
- `latency_samples()` = **B + the plugin's own latency**. If the plugin's latency changes (CLAP
  `request_restart`, VST3 `kLatencyChanged`), the proxy raises `HostRequest::Restart` (ADR-005 §12).
  Example cost: B = 256 at 48 kHz adds 5.3 ms, which is compensated. It is noticeable only when
  monitoring through the rack.
- **RT-rule exception, to be recorded in ADR-002 and MEMORY:** the wake is a syscall
  (futex wake / `SetEvent` / `sem_post`). It is bounded and non-blocking, and it is the only syscall
  allowed on the audio thread, only inside `ProxyModule`.

### 3. Wakeup primitives (behind one `Wakeup` shim, T-801: `wake()` non-blocking, `wait(timeout)`)
- **Linux:** a futex on a 32-bit word in the shared segment (`FUTEX_WAKE`/`FUTEX_WAIT` *without*
  `FUTEX_PRIVATE_FLAG`, since it's cross-process). No fd passing is needed. eventfd is the fallback if
  futex turns out to be problematic.
- **Windows:** auto-reset event objects with random 128-bit names under `Local\`, passed on the command
  line. Hardening option: duplicate unnamed handles into the child (`DuplicateHandle`). The semantics
  are identical.
- **macOS:** POSIX named semaphores (`sem_open`, random names, `sem_unlink` right after both sides have
  opened them). Unnamed process-shared semaphores are unavailable, and private `__ulock` APIs are
  avoided.
- **Shared memory:**
  - Linux: `memfd_create` + seals, fd inherited by the child;
  - Windows: pagefile-backed `CreateFileMapping`;
  - macOS: `shm_open` + unlink after mapping.

  A thin hand-rolled OS layer (~300 lines) is preferred over a framework (see prior art).
- The sandbox's audio thread requests real-time priority itself: rtkit on Linux, MMCSS "Pro Audio" on
  Windows, time-constraint policy on macOS. In the editor, cpal owns the callback threads, so ADR-002
  has no such helper and T-802 implements one for the sandbox.

### 4. Control channel
- A local socket per instance: a Unix domain socket on Linux/macOS, a named pipe on Windows.
- Messages are length-prefixed `serde` messages, JSON in v1 because the traffic is low-rate; switch to a
  compact binary format only if profiling says so.
- Messages carry: handshake (protocol version), load/instantiate, activate/deactivate, param
  info/text (`ParamText`), state save/load (`LiveState` snapshots), extension calls (response curve),
  scan results, editor open/close (M9), log forwarding, shutdown.
- Every request has a timeout: 5 s for load/activate/state, 30 s for a scan.
- **Parent-death handling:** Linux `PR_SET_PDEATHSIG`; a Windows job object with `KILL_ON_JOB_CLOSE`;
  on macOS, EOF on the control socket or kqueue on the parent pid. The sandbox never outlives the
  editor.

### 5. Watchdog and failure policy
- The sandbox bumps a **heartbeat** counter every processed block and in its idle loop. A watchdog
  thread on the editor side (not the audio thread) checks it along with the miss counter.
- **Hang:** no progress for 250 ms while fed, or a control request timeout. The watchdog kills the
  process (`SIGKILL` / `TerminateProcess`). **Crash:** process exit or control-socket EOF.
- **On failure during playback or monitoring:**
  - the slot goes to host bypass (latency-matched dry, 15 ms crossfade, ADR-005 §9) and is marked
    *failed*;
  - the user gets a notice "‹Plugin› stopped responding and was bypassed" with **Restart plugin**
    (respawn + reload the last committed/`LiveState` state) and **Remove**;
  - the crash is counted and the plugin is **flagged** (warning badge) in the plugin manager.

  There is no automatic restart, because a deterministic crash would loop.
- **Recording is never affected:** the take is written from the dry input path, not through the rack
  (ADR-002). A plugin failure can only affect what is heard.
- **During an offline render:** failure **aborts the render with an error**. We never export a
  silently wrong file.
- **Blocklist policy:** plugins that crash or time out while *scanning* are auto-blocklisted
  (path + size + mtime + hash; they are cleared when the file changes or the user unblocks them).
  Runtime crashes only flag, and the user decides whether to blocklist.

### 6. Scanning happens in the sandbox
- `powervoice-plugin-sandbox --scan <file> --format <clap|vst3|lv2|jsfx>` loads **one plugin file per
  process**, enumerates its plugins (descriptor, audio ports, supported layouts, parameter count,
  PowerVoice `module-info` if present), prints JSON and exits.
- T-804 orchestrates scans: N parallel scans (N = cores/2), cached by (path, size, mtime).
- VST3 bundles with `moduleinfo.json` are indexed without loading code (T-806).
- A crashing scan can't kill the editor or the rest of the scan.

### 7. Editor windows (M9) are owned by the sandbox
- Plugin GUIs run on the sandbox's main thread as **floating top-level windows**.
- **Linux:** X11 windows, via XWayland under Wayland. Wayland has no cross-process embedding and the
  plugin GUI APIs (CLAP/VST3/LV2 on Linux) are X11-based, so the window is a normal floating window
  with a transient-for hint only where the editor window is also X11.
- **Windows:** a top-level HWND whose owner is the editor's main window. The handle is passed over the
  control channel; cross-process ownership is allowed.
- **macOS:** an NSWindow in the sandbox process, with no cross-process parenting.
- The editor never embeds foreign GUIs. Plugin-originated parameter changes come back as output events
  and update the host mirror.

### 8. What the sandbox links
The sandbox binary links the format hosts: CLAP (`clack-host`), VST3 (`vst3`), LV2 (`livi`), JSFX
(`ysfx`, **only here**; see ADR-007), and VST2 (T-811, gated; prefers a Carla bridge). The editor
binary links none of them. This keeps foreign code, crash risk and license-sensitive code in one
binary.

### 9. Security scope
The sandbox is a **crash-isolation boundary, not a security boundary**: plugins keep the user's
privileges. OS-level confinement (seccomp/Landlock, AppContainer, App Sandbox) is out of scope for v1.
32-bit Windows plugins are out of scope. Linux users can load Windows plugins through yabridge, which
exposes them as native VST3/CLAP.

## Consequences
**Positive**
- A crashing or hanging plugin can't take down the editor or a recording, and blame is exact.
- The audio thread never blocks on foreign code.
- Offline renders stay deterministic.
- Latency is constant and compensated through the normal Module API path.
- The same machinery serves scanning, installed modules and M9 editors.

**Negative**
- B samples of extra latency per sandboxed plugin (it only matters for monitoring through the rack).
- One process, and a little memory, per instance.
- Three OS-specific wakeup and shared-memory implementations, two of them untested on the owner's
  platform.
- A narrowly scoped exception to the "no syscalls" rule.

**Follow-ups**
- T-801: shim, rings, crash/hang/gain test plugins; measure wake round-trip cost and miss rate at
  B = 64/128/256.
- T-802: process lifecycle, watchdog, `ProxyModule`, failure UX.
- T-804: scan orchestration and blocklist.
- T-901: editors.
- ADR-002 and MEMORY: record the wake-syscall exception.

## Alternatives considered
- **One process per chain, or one process for all plugins:** fewer wakeups, but one crash silences
  every plugin in it and blame needs the in-call marker. Kept as a future grouping policy.
- **Synchronous round trip inside the callback (zero added latency; yabridge and Carla style):** the
  editor's audio thread would block on another process, so a slow plugin would xrun the whole engine.
  It could come later as a per-plugin "low-latency" option if monitoring latency becomes an issue.
- **`iceoryx2`** (MIT OR Apache-2.0, cross-platform zero-copy IPC): capable, but a large general-purpose
  pub/sub framework for what is two SPSC rings and a wake. Evaluate in T-801 as a benchmark reference.
- **`shmem-ipc`** (Apache-2.0/MIT): a clean memfd + eventfd SPSC design, but **Linux-only** (per its
  README). Used as a design reference.
- **Carla bridges (GPL-2.0-or-later) and yabridge (GPL-3.0):** proven designs (a shared-memory RT ring
  plus a non-RT channel, per-plugin processes with optional groups). Studied, **no code copied**
  (license). Bitwig and REAPER's per-plugin/grouped/dedicated modes are the UX reference.

## Open questions
1. Is B = one host block acceptable for "monitoring through rack" with external plugins, or does the
   owner want the synchronous low-latency option as well? Decide after the T-801 measurements.
2. Does the owner accept that sandboxing is crash isolation only, with no protection against
   malicious plugins?

## Amendment 1 — T-801 transport, as implemented (2026-09-14)
Crate `vox-sandbox-ipc` (std + libc; windows-sys on Windows; no new third-party crate).

### 1. Segment layout, version 1 (`layout.rs`, frozen by `tests/layout.rs`)
| Offset | Size | Content |
|---|---|---|
| 0 | 64 | header, host-written once: magic `PVSBXIPC`, version, sizes, sample rate, `max_block` B, ring capacity C, event capacity E, latency L, start position, host pid |
| 64 | 64 | host cursors: `in_write_pos`, event cursors, `host_command` (run/shutdown) |
| 128 | 64 | plugin cursors: `in_read_pos`, `out_write_pos`, `out_valid_from`, event cursors, overruns, chunks |
| 192, 256 | 64 + 64 | doorbells host → plugin and plugin → host (`seq`, `waiters`) |
| 320 | 64 | liveness, plugin-written: state (Created/Running/Stopped/Failed), plugin pid, heartbeat, in-call marker |
| 384 | 64 | 16 telemetry cells (`f32` bits) |
| 512 | 4C + 4C | input and output **sample rings** |
| 512 + 8C | 32E + 32E | event rings host → plugin and plugin → host (`pos`, `id`, `kind`, `value`) |

- The rings are **indexed by absolute stream position** (masked by C − 1), not by block, so the
  output delay is exactly L for any callback size ≤ B. C = next_pow2(8·B), at least 64. The
  total size is rounded up to 4 KiB.
- L = `latency_samples` ∈ [0, B]. The default L = B is §2's pipelined mode. L = 0 is the
  synchronous "low-latency" alternative, and uses the same code path.
- Every word is an atomic. The host never indexes with a plugin cursor unmasked, and it treats
  `out_write_pos` beyond its own published input as a miss. The dry signal comes from a host-local
  copy of the input.
- **Overrun:** the host never waits for ring space; it overwrites. The plugin re-checks
  `in_write_pos` after copying (a seqlock-style check). If it lags by more than C − B, it
  resynchronises to the present and publishes `out_valid_from`, and the host treats older output
  as a miss.
- **Event rings** are SPSC. When full, the event is dropped and counted. The plugin delivers the
  events with `pos` < chunk end; late events land at offset 0.

### 2. Wakeup and shared memory
- **Linux:** futex as decided in §3: `FUTEX_WAKE` only when the peer is parked (waiter count),
  `FUTEX_WAIT_BITSET` with an absolute deadline.
- **Shared memory, Linux:** memfd with shrink/grow/seal seals. The fd is inherited by exactly one
  child: `pre_exec` clears `FD_CLOEXEC` in that child only. The handle is `memfd:<fd>`.
- **Shared memory, macOS:** `shm_open` (also built and tested on Linux).
- **Shared memory, Windows:** `CreateFileMappingW`; only compile-checked (`just check-cross`).
- **macOS/Windows wakeup:** they use the portable `SpinYieldWakeup` for now: spin, then yield for up
  to 1 ms, then sleep in 100 µs steps, bounded by the deadline. The §3 primitives (named semaphores,
  named events) need per-doorbell OS handles passed next to the segment handle. They come with the
  process lifecycle (T-802) behind the same `Wakeup` trait.

### 3. Host wait and failure model
- **Wait budget:** `HostEnd::process` waits at most `WaitBudget::FractionOfBlock(0.25)` of the
  block period by default. `Fixed(hang_timeout)` is §2's offline mode (`HostOptions::offline`),
  where a miss means the render aborts. After 4 consecutive misses the host stops waiting until a
  block is on time. ADR-002 Amendment 2 records the syscall exception.
- **Miss:** the host outputs dry with a **splice crossfade** over 96 samples:
  `dry + δ·(1 − t)`, where δ = last output − dry at that position. The wet signal is unknown at a
  miss, so a true crossfade isn't possible. Recovery is a linear dry → wet crossfade over 96
  samples. Fully wet and fully dry blocks are bit-exact copies. This short glitch fade is
  separate from §5's 15 ms slot bypass, which T-802 applies on a fault.
- **Fault detection:** `Monitor::poll(PeerStatus)` runs on the control thread and never blocks.
  - `Crashed`: the process is gone (`Child::try_wait`; an unreaped crashed child is a zombie that
    `kill(pid, 0)` misses).
  - `Exited`: the plugin stopped cleanly without being asked to.
  - `Failed`: the plugin reported a fatal error.
  - `Hung { stale, in_call }`: the heartbeat has been stale for ≥ 250 ms. The clock starts at
    attach.
- **Effect of a fault:** the first fault sets the host's bypass flag. From then on the host
  outputs dry and makes no wake or wait. Misses are not faults; they reach the control thread as
  counters (`Health::new_misses`).
- **Heartbeat and shutdown:** the plugin bumps the heartbeat on every chunk and on every idle
  wake-up; the idle timeout must stay well below the hang timeout (the test plugins use 20 ms).
  Shutdown is `host_command` + a ring; the plugin then marks itself Stopped, which is not a fault.

### 4. Measurements (T-801 follow-up; this machine: 16 threads, Linux 7.2, release build)
Setup: the gain test plugin runs in a child process, paced at the real-time rate, 3 s per row,
with no RT priority on either side (`just bench`). Round trip = `HostEnd::process` duration.

| Block (period) | sync futex p50 / p99 / max µs | sync spin-yield p50 / p99 / max µs | pipelined futex (host cost) p50 / p99 / max µs | misses |
|---|---|---|---|---|
| 64 (1333 µs) | 113 / 211 / 365 | 70 / 200 / 268 | 7.8 / 21 / 95 | 0 |
| 128 (2667 µs) | 142 / 250 / 680 | 60 / 150 / 776 | 11 / 33 / 97 | 0 |
| 256 (5333 µs) | 115 / 226 / 526 | 66 / 144 / 605 | 17 / 26 / 70 | 0 |
| 512 (10667 µs) | 124 / 244 / 317 | 71 / 160 / 564 | 18 / 36 / 147 | 0 |

**Open question 1:** a synchronous (L = 0) round trip fits comfortably in the budget: its p99 is
16 % of a 64-frame period. That makes a per-plugin "low-latency" option viable later. Pipelined
L = B stays the default: it adds about 8–18 µs per callback and had no misses. Most of the
synchronous cost is waking the idle sandbox thread; RT priority (T-802) should shrink the tails.
iceoryx2 was not benchmarked, since that would add a dependency and the hand-rolled layer already
meets the budget.
