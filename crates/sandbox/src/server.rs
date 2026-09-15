//! The sandbox's main loop: control requests on stdin, the audio thread on `Activate`, and —
//! T-901 — the plugin's editor window, all on the main thread (the plugin's main/UI thread).
//!
//! The loop waits for whichever comes first: a control request (the reader thread wakes it
//! through a pipe), the window system (the X11 connection), a descriptor or timer a plugin GUI
//! registered ([`crate::gui::runloop`]), or the 10 ms idle tick. While an editor is open it also
//! sends a heartbeat (a GUI that hangs the main thread stops it; the host's watchdog then kills
//! the process), and it reports what the window changed: parameters changed while inactive, a
//! state that changed outside the parameters (debounced), the window closing on its own.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use vox_module_api::{ActivateConfig, ChannelLayout, ProcessMode};
use vox_sandbox_ipc::protocol::{
    self, NOTIFY_ID, Notification, PROTOCOL_VERSION, Request, RequestBody, Response, ResponseBody,
    ScanReply,
};
use vox_sandbox_ipc::test_plugins::{die_with_parent, process_alive};
use vox_sandbox_ipc::{PluginEnd, Serviced, SharedRegion};

use crate::backend::{
    MainThreadEvents, OUT_EVENT_CAPACITY, PluginBackend, PluginInstance, backends,
};
use crate::gui::{EditorHost, WindowEvent, WindowSystem, runloop};

/// The audio thread's idle wait: the heartbeat moves at least this often (well below the host's
/// 250 ms hang timeout, ADR-008 Amendment 1 §3).
const IDLE_TIMEOUT: Duration = Duration::from_millis(20);

/// The main thread's idle tick: plugin main-thread callbacks (CLAP `request_callback`) are
/// served within this.
const MAIN_IDLE: Duration = Duration::from_millis(10);

/// While an editor is open, the main thread says it's alive this often (T-901).
const HEARTBEAT: Duration = Duration::from_millis(250);

/// A state change the plugin reports (CLAP `mark_dirty`) is sent this long after the first one
/// (a knob turned in the window marks it dirty many times a second).
const STATE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Without a pollable window system (Windows), the loop still pumps the window's messages this
/// often while one is open.
#[cfg(not(unix))]
const WINDOW_PUMP: Duration = Duration::from_millis(5);

struct Args {
    shm: Option<String>,
    /// Checked against `getppid()` (unix) to catch a host that died before PDEATHSIG was armed.
    #[cfg_attr(not(unix), allow(dead_code))]
    host_pid: Option<u32>,
    /// `--scan <file>`: scan mode (T-803).
    scan: Option<String>,
    /// `--format <clap|vst3|lv2|jsfx>` for `--scan`.
    format: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut shm = None;
    let mut host_pid = None;
    let mut scan = None;
    let mut format = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--shm" => shm = it.next(),
            "--host-pid" => {
                host_pid = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--host-pid needs a number")?,
                );
            }
            "--scan" => scan = Some(it.next().ok_or("--scan needs a file")?),
            "--format" => format = Some(it.next().ok_or("--format needs a name")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if scan.is_none() && shm.is_none() {
        return Err("--shm <handle> is required".into());
    }
    Ok(Args {
        shm,
        host_pid,
        scan,
        format,
    })
}

/// Entry point of `powervoice-sandbox`.
pub fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("powervoice-sandbox: {e}");
            return ExitCode::from(2);
        }
    };
    die_with_parent();
    // The plugin's main thread: GUI timers and descriptors are served here.
    runloop::mark_main_thread();
    if let Some(file) = &args.scan {
        return scan_mode(file, args.format.as_deref().unwrap_or("clap"));
    }
    #[cfg(unix)]
    if let Some(pid) = args.host_pid
        // SAFETY: getppid has no preconditions.
        && u32::try_from(unsafe { libc::getppid() }).ok() != Some(pid)
    {
        // The host died before PR_SET_PDEATHSIG was armed.
        return ExitCode::from(5);
    }
    let out = match protocol_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("powervoice-sandbox: control output: {e}");
            return ExitCode::from(3);
        }
    };
    let shm = args.shm.unwrap_or_default();
    let region = match SharedRegion::open(&shm) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("powervoice-sandbox: open {shm}: {e}");
            return ExitCode::from(3);
        }
    };
    let wake = match wake::Wake::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("powervoice-sandbox: wake pipe: {e}");
            return ExitCode::from(3);
        }
    };
    // Requests are read on their own thread, so the main thread (the plugin's main thread) can
    // also serve the plugin's main-thread callbacks and its window while the host is quiet.
    let (tx, rx) = mpsc::channel::<(Request, Vec<u8>)>();
    let waker = wake.waker();
    let reader = thread::Builder::new()
        .name("sandbox-control-rx".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            // Ends when the host closes the channel (it exited or dropped us) or on a broken
            // frame.
            while let Ok(Some(msg)) = protocol::recv::<Request>(&mut input) {
                if tx.send(msg).is_err() {
                    break;
                }
                waker.wake();
            }
            drop(tx);
            waker.wake();
        });
    if let Err(e) = reader {
        eprintln!("powervoice-sandbox: control reader: {e}");
        return ExitCode::from(3);
    }
    let mut server = Server {
        region,
        backends: backends(),
        instance: None,
        audio: None,
        out,
        windows: WindowSystem::from_env(),
        editor: None,
        state_due: None,
        host_pid: args.host_pid,
    };
    server.run(&rx, &wake);
    server.close_editor(false);
    server.stop_audio();
    // Destroy the plugin (and unload its library) on the main thread.
    drop(server);
    ExitCode::SUCCESS
}

/// `--scan <file> --format <format>`: prints a [`ScanReply`] and exits (0 when scanned, 4 when
/// the file couldn't be scanned). A crash or hang in the plugin's code ends the process instead.
fn scan_mode(file: &str, format: &str) -> ExitCode {
    let mut out = match protocol_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("powervoice-sandbox: scan output: {e}");
            return ExitCode::from(3);
        }
    };
    let reply = match format {
        "clap" => match crate::clap::scan::scan(std::path::Path::new(file)) {
            Ok(report) => ScanReply::Ok(report),
            Err(e) => ScanReply::Error(e),
        },
        "vst3" => match crate::vst3::scan::scan(std::path::Path::new(file)) {
            Ok(report) => ScanReply::Ok(report),
            Err(e) => ScanReply::Error(e),
        },
        "lv2" => match crate::lv2::scan::scan(std::path::Path::new(file)) {
            Ok(report) => ScanReply::Ok(report),
            Err(e) => ScanReply::Error(e),
        },
        "jsfx" => match crate::jsfx::scan::scan(std::path::Path::new(file)) {
            Ok(report) => ScanReply::Ok(report),
            Err(e) => ScanReply::Error(e),
        },
        other => ScanReply::Error(format!("unknown plugin format `{other}`")),
    };
    let code = if matches!(reply, ScanReply::Ok(_)) {
        0
    } else {
        4
    };
    let written = serde_json::to_writer(&mut out, &reply)
        .map_err(io::Error::other)
        .and_then(|()| out.flush());
    if let Err(e) = written {
        eprintln!("powervoice-sandbox: scan output: {e}");
        return ExitCode::from(3);
    }
    ExitCode::from(code)
}

/// Where responses go: a private duplicate of stdout, with fd 1 itself redirected to stderr
/// (unix), so plugin output can't corrupt the control stream.
fn protocol_output() -> io::Result<Box<dyn Write>> {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        // SAFETY: dup of our own stdout; the new fd is owned by the File below.
        let fd = unsafe { libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: point fd 1 at stderr (both valid fds of this process).
        if unsafe { libc::dup2(2, 1) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a fresh descriptor nothing else owns.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(Box::new(io::BufWriter::new(file)))
    }
    #[cfg(not(unix))]
    {
        Ok(Box::new(io::stdout()))
    }
}

/// How the reader thread wakes the main loop (unix: a non-blocking pipe it can `poll` together
/// with the window system and the plugin's descriptors).
#[cfg(unix)]
mod wake {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;

    pub(super) struct Wake {
        read: OwnedFd,
        write: Arc<OwnedFd>,
    }

    pub(super) struct Waker(Arc<OwnedFd>);

    impl Wake {
        pub(super) fn new() -> io::Result<Self> {
            let mut fds = [0i32; 2];
            // SAFETY: `fds` has room for the two descriptors `pipe` writes.
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in fds {
                // SAFETY: descriptors this process just created.
                unsafe {
                    let fl = libc::fcntl(fd, libc::F_GETFL);
                    libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            // SAFETY: fresh descriptors, owned from here on.
            let (read, write) =
                unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
            Ok(Self {
                read,
                write: Arc::new(write),
            })
        }

        pub(super) fn waker(&self) -> Waker {
            Waker(self.write.clone())
        }

        pub(super) fn fd(&self) -> i32 {
            self.read.as_raw_fd()
        }

        pub(super) fn drain(&self) {
            let mut buf = [0u8; 64];
            // SAFETY: reading into our own buffer from our non-blocking pipe.
            while unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
        }
    }

    impl Waker {
        pub(super) fn wake(&self) {
            let b = 1u8;
            // SAFETY: a one-byte write to our non-blocking pipe (a full pipe already wakes).
            unsafe { libc::write(self.0.as_raw_fd(), (&raw const b).cast(), 1) };
        }
    }
}

/// Without `poll` (Windows), the main loop waits on the request channel itself.
#[cfg(not(unix))]
mod wake {
    pub(super) struct Wake;
    pub(super) struct Waker;

    impl Wake {
        pub(super) fn new() -> std::io::Result<Self> {
            Ok(Self)
        }

        pub(super) fn waker(&self) -> Waker {
            Waker
        }
    }

    impl Waker {
        pub(super) fn wake(&self) {}
    }
}

struct Audio {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// The open editor window (T-901).
struct EditorSession {
    size: (u32, u32),
    /// The plugin's GUI is embedded in our window (else it floats on its own).
    embedded: bool,
    /// Next heartbeat.
    next_alive: Instant,
}

struct Server {
    region: SharedRegion,
    backends: Vec<Box<dyn PluginBackend>>,
    instance: Option<Arc<dyn PluginInstance>>,
    audio: Option<Audio>,
    /// The control channel's sandbox → host direction (replies and notifications; main thread
    /// only).
    out: Box<dyn Write>,
    windows: WindowSystem,
    editor: Option<EditorSession>,
    /// A state push is due (T-901: debounced `mark_dirty`, or an editor that just closed).
    state_due: Option<Instant>,
    host_pid: Option<u32>,
}

fn error(message: impl Into<String>) -> (ResponseBody, Vec<u8>) {
    (
        ResponseBody::Error {
            message: message.into(),
        },
        Vec::new(),
    )
}

fn ok() -> (ResponseBody, Vec<u8>) {
    (ResponseBody::Ok, Vec::new())
}

/// What one wait ended with.
enum Waited {
    /// Something may be ready (requests, windows, descriptors in `ready`, a deadline).
    Ready,
    /// A request (the channel wait of platforms without `poll`).
    #[cfg_attr(unix, allow(dead_code))]
    Request(Request, Vec<u8>),
    /// The host closed the channel (the channel wait of platforms without `poll`).
    #[cfg_attr(unix, allow(dead_code))]
    Disconnected,
}

/// Waits until `deadline` for a request, the window system's descriptor or a plugin descriptor
/// (`ready` gets those that are ready, with `runloop::FD_*` flags).
#[cfg(unix)]
fn wait(
    _rx: &Receiver<(Request, Vec<u8>)>,
    wake: &wake::Wake,
    window_fd: Option<i32>,
    _has_window: bool,
    deadline: Instant,
    ready: &mut Vec<(i32, u32)>,
) -> Waited {
    ready.clear();
    let mut pfds = vec![libc::pollfd {
        fd: wake.fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    if let Some(fd) = window_fd {
        pfds.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }
    let first_plugin = pfds.len();
    let plugin = runloop::fds();
    for &(fd, flags) in &plugin {
        let mut events = 0;
        if flags & runloop::FD_READ != 0 {
            events |= libc::POLLIN;
        }
        if flags & runloop::FD_WRITE != 0 {
            events |= libc::POLLOUT;
        }
        pfds.push(libc::pollfd {
            fd,
            events,
            revents: 0,
        });
    }
    let left = deadline.saturating_duration_since(Instant::now());
    // Round up: a 0.4 ms wait must not become a busy 0 ms poll.
    let timeout_ms = i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
    // SAFETY: `pfds` is a valid array of `pfds.len()` entries for the call.
    let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout_ms) };
    if n > 0 {
        for p in &pfds[first_plugin..] {
            if p.revents == 0 {
                continue;
            }
            let mut flags = 0;
            if p.revents & libc::POLLIN != 0 {
                flags |= runloop::FD_READ;
            }
            if p.revents & libc::POLLOUT != 0 {
                flags |= runloop::FD_WRITE;
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                flags |= runloop::FD_ERROR;
            }
            ready.push((p.fd, flags));
        }
    }
    wake.drain();
    Waited::Ready
}

#[cfg(not(unix))]
fn wait(
    rx: &Receiver<(Request, Vec<u8>)>,
    _wake: &wake::Wake,
    _window_fd: Option<i32>,
    has_window: bool,
    deadline: Instant,
    ready: &mut Vec<(i32, u32)>,
) -> Waited {
    ready.clear();
    let mut left = deadline.saturating_duration_since(Instant::now());
    if has_window {
        left = left.min(WINDOW_PUMP);
    }
    match rx.recv_timeout(left) {
        Ok((req, payload)) => Waited::Request(req, payload),
        Err(mpsc::RecvTimeoutError::Timeout) => Waited::Ready,
        Err(mpsc::RecvTimeoutError::Disconnected) => Waited::Disconnected,
    }
}

impl Server {
    /// The main loop; returns when the host is gone or asked to shut down.
    fn run(&mut self, rx: &Receiver<(Request, Vec<u8>)>, wake: &wake::Wake) {
        let mut next_idle = Instant::now() + MAIN_IDLE;
        let mut ready = Vec::new();
        let mut window_events = Vec::new();
        loop {
            let deadline = self.deadline(next_idle);
            let waited = wait(
                rx,
                wake,
                self.windows.fd(),
                self.windows.has_window(),
                deadline,
                &mut ready,
            );
            match waited {
                Waited::Ready => {}
                Waited::Request(req, payload) => {
                    if self.dispatch(req, payload) {
                        return;
                    }
                }
                Waited::Disconnected => return,
            }
            loop {
                match rx.try_recv() {
                    Ok((req, payload)) => {
                        if self.dispatch(req, payload) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            self.service_windows(&mut window_events);
            for &(fd, flags) in &ready {
                runloop::fire_fd(fd, flags);
            }
            runloop::fire_timers(Instant::now());
            let now = Instant::now();
            if now >= next_idle {
                next_idle = now + MAIN_IDLE;
                self.idle();
                if self.host_pid.is_some_and(|pid| !process_alive(pid)) {
                    return;
                }
            }
            self.housekeeping(Instant::now());
        }
    }

    /// The next time the loop must run on its own.
    fn deadline(&self, next_idle: Instant) -> Instant {
        let mut d = next_idle;
        if let Some(t) = runloop::next_due() {
            d = d.min(t);
        }
        if let Some(e) = &self.editor {
            d = d.min(e.next_alive);
        }
        if let Some(t) = self.state_due {
            d = d.min(t);
        }
        d
    }

    /// Handles one request and sends its answer; `true` when the loop must end.
    fn dispatch(&mut self, req: Request, payload: Vec<u8>) -> bool {
        let shutdown = matches!(req.body, RequestBody::Shutdown);
        let (body, reply_payload) = self.handle(req.body, payload);
        let sent = protocol::send(
            &mut self.out,
            &Response { id: req.id, body },
            &reply_payload,
        );
        shutdown || sent.is_err()
    }

    /// Sends an unsolicited message (T-901); a broken channel ends the loop at the next read.
    fn notify(&mut self, n: Notification, payload: &[u8]) {
        let _ = protocol::send(
            &mut self.out,
            &Response {
                id: NOTIFY_ID,
                body: ResponseBody::Notify(n),
            },
            payload,
        );
    }

    fn handle(&mut self, body: RequestBody, payload: Vec<u8>) -> (ResponseBody, Vec<u8>) {
        match body {
            RequestBody::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    return error(format!(
                        "protocol {protocol} unsupported (sandbox speaks {PROTOCOL_VERSION})"
                    ));
                }
                (
                    ResponseBody::Hello {
                        protocol: PROTOCOL_VERSION,
                        pid: std::process::id(),
                    },
                    Vec::new(),
                )
            }
            RequestBody::Load { backend, plugin } => {
                if self.instance.is_some() {
                    return error("a plugin is already loaded");
                }
                let Some(b) = self.backends.iter().find(|b| b.name() == backend) else {
                    return error(format!("unknown plugin format `{backend}`"));
                };
                match b.load(&plugin) {
                    Ok(instance) => {
                        let mut info = instance.info();
                        info.editor = instance.has_editor();
                        self.instance = Some(instance);
                        (ResponseBody::Loaded(info), Vec::new())
                    }
                    Err(e) => error(e),
                }
            }
            RequestBody::Activate {
                sample_rate,
                max_block,
                mode,
            } => self.activate(sample_rate, max_block, mode),
            RequestBody::Deactivate => {
                self.stop_audio();
                ok()
            }
            RequestBody::SetParams { values } => {
                let Some(inst) = &self.instance else {
                    return error("no plugin loaded");
                };
                if self.audio.is_some() {
                    return error("parameters of an active plugin change through events");
                }
                for v in values {
                    if let Err(e) = inst.set_param(v.id, v.value) {
                        return error(e);
                    }
                }
                ok()
            }
            RequestBody::SaveState => match &self.instance {
                Some(inst) => match inst.save_state() {
                    Ok(data) => (ResponseBody::State, data),
                    Err(e) => error(e),
                },
                None => error("no plugin loaded"),
            },
            RequestBody::LoadState => match &self.instance {
                Some(_) if self.audio.is_some() => error("the plugin is active"),
                Some(inst) => match inst.load_state(&payload) {
                    Ok(()) => (
                        ResponseBody::Params {
                            values: inst.info().values,
                        },
                        Vec::new(),
                    ),
                    Err(e) => error(e),
                },
                None => error("no plugin loaded"),
            },
            RequestBody::Shutdown => {
                self.close_editor(false);
                self.stop_audio();
                ok()
            }
            RequestBody::ParamTexts { values } => match &self.instance {
                Some(inst) => (
                    ResponseBody::Texts {
                        texts: values
                            .iter()
                            .map(|v| inst.param_to_text(v.id, v.value))
                            .collect(),
                    },
                    Vec::new(),
                ),
                None => error("no plugin loaded"),
            },
            RequestBody::TextToParam { id, text } => match &self.instance {
                Some(inst) => (
                    ResponseBody::Value {
                        value: inst.text_to_param(id, &text),
                    },
                    Vec::new(),
                ),
                None => error("no plugin loaded"),
            },
            RequestBody::OpenEditor { title, parent } => self.open_editor(&title, parent),
            RequestBody::CloseEditor => {
                self.close_editor(false);
                ok()
            }
        }
    }

    fn activate(
        &mut self,
        sample_rate: f64,
        max_block: u32,
        mode: ProcessMode,
    ) -> (ResponseBody, Vec<u8>) {
        let Some(inst) = self.instance.clone() else {
            return error("no plugin loaded");
        };
        self.stop_audio();
        let region = match self.region.duplicate() {
            Ok(r) => r,
            Err(e) => return error(format!("segment: {e}")),
        };
        let end = match PluginEnd::attach(region) {
            Ok(e) => e,
            Err(e) => return error(format!("segment: {e}")),
        };
        let segment = end.config();
        if segment.max_block != max_block || segment.sample_rate.to_bits() != sample_rate.to_bits()
        {
            end.fail();
            return error("the segment doesn't match the activation request");
        }
        let config = ActivateConfig {
            sample_rate,
            max_block,
            mode,
            layout: ChannelLayout::MONO,
        };
        let info = match inst.activate(&config) {
            Ok(i) => i,
            Err(e) => {
                end.fail();
                return error(e);
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            thread::Builder::new()
                .name("sandbox-audio".into())
                .spawn(move || audio_loop(end, inst, stop))
        };
        match thread {
            Ok(thread) => {
                self.audio = Some(Audio { stop, thread });
                (
                    ResponseBody::Activated {
                        latency_samples: info.latency_samples,
                        tail_samples: info.tail_samples,
                    },
                    Vec::new(),
                )
            }
            Err(e) => error(format!("audio thread: {e}")),
        }
    }

    fn stop_audio(&mut self) {
        if let Some(a) = self.audio.take() {
            a.stop.store(true, Ordering::Release);
            let _ = a.thread.join();
            if let Some(inst) = &self.instance {
                inst.deactivate();
            }
        }
    }

    /// `OpenEditor` (T-901): the plugin's GUI embedded in a new top-level window (shown once the
    /// plugin is attached), or floating on its own; an open window is raised.
    fn open_editor(&mut self, title: &str, parent: Option<u64>) -> (ResponseBody, Vec<u8>) {
        let Some(inst) = self.instance.clone() else {
            return error("no plugin loaded");
        };
        if let Some(e) = &self.editor {
            let (width, height) = e.size;
            self.windows.raise();
            return (ResponseBody::EditorOpened { width, height }, Vec::new());
        }
        if !inst.has_editor() {
            return error("the plugin has no window of its own");
        }
        let mut host = EditorHost::new(&mut self.windows, title, parent);
        let result = inst.open_editor(&mut host);
        let embedded = host.created();
        match result {
            Ok((width, height)) => {
                if embedded {
                    self.windows.show();
                }
                self.editor = Some(EditorSession {
                    size: (width, height),
                    embedded,
                    next_alive: Instant::now(),
                });
                (ResponseBody::EditorOpened { width, height }, Vec::new())
            }
            Err(e) => {
                self.windows.destroy();
                error(e)
            }
        }
    }

    /// Closes the editor (no-op when closed): the plugin's GUI first, then our window.
    /// `by_itself`: the window closed on its own (the user, the plugin) — tell the host. Either
    /// way the plugin's state is sent right after (whatever the window changed is in it now).
    fn close_editor(&mut self, by_itself: bool) {
        if self.editor.take().is_none() {
            return;
        }
        if let Some(inst) = &self.instance {
            inst.close_editor();
        }
        self.windows.destroy();
        if by_itself {
            self.notify(Notification::EditorClosed, &[]);
        }
        self.state_due = Some(Instant::now());
    }

    /// The window system's events: a close request closes the editor; a user resize is offered
    /// to the plugin, and the window follows the size it accepts.
    fn service_windows(&mut self, events: &mut Vec<WindowEvent>) {
        events.clear();
        self.windows.pump(events);
        for e in events.drain(..) {
            match e {
                WindowEvent::CloseRequested => self.close_editor(true),
                WindowEvent::Resized(w, h) => {
                    let (Some(inst), Some(ed)) = (&self.instance, &mut self.editor) else {
                        continue;
                    };
                    let (aw, ah) = inst.editor_resized(w, h).unwrap_or((w, h));
                    ed.size = (aw, ah);
                    if (aw, ah) != (w, h) {
                        self.windows.resize(aw, ah);
                    }
                }
            }
        }
    }

    /// The idle tick: the plugin's main-thread work, and what it reports.
    fn idle(&mut self) {
        let Some(inst) = self.instance.clone() else {
            return;
        };
        let mut ev = MainThreadEvents::default();
        inst.main_thread_idle(&mut ev);
        if !ev.params.is_empty() {
            self.notify(Notification::Params { values: ev.params }, &[]);
        }
        if ev.state_dirty && self.state_due.is_none() {
            self.state_due = Some(Instant::now() + STATE_DEBOUNCE);
        }
        if let (Some((w, h)), Some(ed)) = (ev.editor_resize, self.editor.as_mut())
            && ed.embedded
        {
            ed.size = (w, h);
            self.windows.resize(w, h);
        }
        if ev.editor_closed {
            self.close_editor(true);
        }
    }

    /// The heartbeat while an editor is open, and a due state push.
    fn housekeeping(&mut self, now: Instant) {
        if let Some(e) = self.editor.as_mut()
            && now >= e.next_alive
        {
            e.next_alive = now + HEARTBEAT;
            self.notify(Notification::Alive, &[]);
        }
        if self.state_due.is_some_and(|t| now >= t) {
            self.state_due = None;
            let state = self.instance.as_ref().map(|i| i.save_state());
            if let Some(Ok(data)) = state {
                self.notify(Notification::StateChanged, &data);
            }
        }
    }
}

/// The sandbox audio thread: serves chunks until the host (host_command) or the main thread
/// (`stop`) asks it to stop.
fn audio_loop(mut end: PluginEnd, inst: Arc<dyn PluginInstance>, stop: Arc<AtomicBool>) {
    crate::rt::promote_current_thread();
    let host_pid = end.host_pid();
    let mut out_events = Vec::with_capacity(OUT_EVENT_CAPACITY);
    loop {
        if stop.load(Ordering::Acquire) {
            inst.audio_thread_stopping();
            end.stop();
            return;
        }
        // H-36: each chunk's plugin → host events (restart requests, parameter changes) reach
        // the event ring before that chunk's output is published — not after the whole
        // `service` call, which in an offline render can span the entire stream.
        let served = end.service_with_events(IDLE_TIMEOUT, &mut out_events, |chunk, events| {
            inst.process(chunk, events);
        });
        match served {
            Serviced::Shutdown => {
                inst.audio_thread_stopping();
                end.stop();
                return;
            }
            Serviced::Idle if !process_alive(host_pid) => std::process::exit(5),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse() {
        let a = parse_args(["--shm", "memfd:7", "--host-pid", "12"].map(String::from)).unwrap();
        assert_eq!((a.shm.as_deref(), a.host_pid), (Some("memfd:7"), Some(12)));
        assert!(parse_args(["--host-pid", "x"].map(String::from)).is_err());
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(["--bogus"].map(String::from)).is_err());
        let a = parse_args(["--scan", "/p/x.clap", "--format", "clap"].map(String::from)).unwrap();
        assert_eq!(
            (a.scan.as_deref(), a.format.as_deref(), a.shm),
            (Some("/p/x.clap"), Some("clap"), None)
        );
        assert!(parse_args(["--scan"].map(String::from)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_wake_pipe_wakes_a_waiting_loop() {
        let wake = wake::Wake::new().unwrap();
        let (_tx, rx) = mpsc::channel::<(Request, Vec<u8>)>();
        let waker = wake.waker();
        let t0 = Instant::now();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            waker.wake();
        });
        let mut ready = Vec::new();
        let w = wait(
            &rx,
            &wake,
            None,
            false,
            Instant::now() + Duration::from_secs(5),
            &mut ready,
        );
        assert!(matches!(w, Waited::Ready));
        assert!(
            t0.elapsed() < Duration::from_secs(4),
            "woken, not timed out"
        );
        h.join().unwrap();
        // A deadline in the past doesn't block.
        let t1 = Instant::now();
        wait(&rx, &wake, None, false, t1, &mut ready);
        assert!(t1.elapsed() < Duration::from_secs(1));
    }
}
