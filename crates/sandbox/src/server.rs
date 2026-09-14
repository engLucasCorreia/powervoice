//! The sandbox's main loop: control requests on stdin, the audio thread on `Activate`.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use vox_module_api::{ActivateConfig, ChannelLayout, ProcessMode};
use vox_sandbox_ipc::protocol::{
    self, PROTOCOL_VERSION, Request, RequestBody, Response, ResponseBody,
};
use vox_sandbox_ipc::test_plugins::{die_with_parent, process_alive};
use vox_sandbox_ipc::{PluginEnd, Serviced, SharedRegion};

use crate::backend::{OUT_EVENT_CAPACITY, PluginBackend, PluginInstance, backends};

/// The audio thread's idle wait: the heartbeat moves at least this often (well below the host's
/// 250 ms hang timeout, ADR-008 Amendment 1 §3).
const IDLE_TIMEOUT: Duration = Duration::from_millis(20);

struct Args {
    shm: String,
    /// Checked against `getppid()` (unix) to catch a host that died before PDEATHSIG was armed.
    #[cfg_attr(not(unix), allow(dead_code))]
    host_pid: Option<u32>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut shm = None;
    let mut host_pid = None;
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
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        shm: shm.ok_or("--shm <handle> is required")?,
        host_pid,
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
    #[cfg(unix)]
    if let Some(pid) = args.host_pid
        // SAFETY: getppid has no preconditions.
        && u32::try_from(unsafe { libc::getppid() }).ok() != Some(pid)
    {
        // The host died before PR_SET_PDEATHSIG was armed.
        return ExitCode::from(5);
    }
    let mut out = match protocol_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("powervoice-sandbox: control output: {e}");
            return ExitCode::from(3);
        }
    };
    let region = match SharedRegion::open(&args.shm) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("powervoice-sandbox: open {}: {e}", args.shm);
            return ExitCode::from(3);
        }
    };
    let mut server = Server {
        region,
        backends: backends(),
        instance: None,
        audio: None,
    };
    let mut input = io::stdin().lock();
    // Ends when the host closes the channel (it exited or dropped us) or on a broken frame.
    while let Ok(Some((req, payload))) = protocol::recv::<Request>(&mut input) {
        let shutdown = matches!(req.body, RequestBody::Shutdown);
        let (body, reply_payload) = server.handle(req.body, payload);
        let sent = protocol::send(&mut out, &Response { id: req.id, body }, &reply_payload);
        if shutdown || sent.is_err() {
            break;
        }
    }
    server.stop_audio();
    ExitCode::SUCCESS
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

struct Audio {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Server {
    region: SharedRegion,
    backends: Vec<Box<dyn PluginBackend>>,
    instance: Option<Arc<dyn PluginInstance>>,
    audio: Option<Audio>,
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

impl Server {
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
                        let info = instance.info();
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
                self.stop_audio();
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
}

/// The sandbox audio thread: serves chunks until the host (host_command) or the main thread
/// (`stop`) asks it to stop.
fn audio_loop(mut end: PluginEnd, inst: Arc<dyn PluginInstance>, stop: Arc<AtomicBool>) {
    crate::rt::promote_current_thread();
    let host_pid = end.host_pid();
    let mut out_events = Vec::with_capacity(OUT_EVENT_CAPACITY);
    loop {
        if stop.load(Ordering::Acquire) {
            end.stop();
            return;
        }
        let served = end.service(IDLE_TIMEOUT, |chunk| inst.process(chunk, &mut out_events));
        for e in out_events.drain(..) {
            let _ = end.push_output_event(e);
        }
        match served {
            Serviced::Shutdown => {
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
        assert_eq!((a.shm.as_str(), a.host_pid), ("memfd:7", Some(12)));
        assert!(parse_args(["--host-pid", "x"].map(String::from)).is_err());
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(["--bogus"].map(String::from)).is_err());
    }
}
