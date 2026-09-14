//! The control-channel client (host side): a writer thread and a reader thread per sandbox, so a
//! request never blocks the caller beyond its timeout even if the sandbox stops reading.

use std::io::{BufReader, BufWriter};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use vox_sandbox_ipc::protocol::{self, Request, RequestBody, Response, ResponseBody};

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RpcError {
    /// No answer within the timeout (ADR-008 §5: a hang).
    #[error("timed out")]
    Timeout,
    /// The channel closed (the sandbox exited or crashed).
    #[error("the sandbox closed the control channel")]
    Disconnected,
    /// The sandbox answered with an error.
    #[error("{0}")]
    Remote(String),
}

type Outgoing = (Request, Vec<u8>);
type Incoming = (Response, Vec<u8>);

/// The request side of one sandbox's control channel.
pub(crate) struct Rpc {
    tx: Option<Sender<Outgoing>>,
    rx: Receiver<Incoming>,
    next_id: u64,
}

impl Rpc {
    /// Starts the writer (→ `stdin`) and reader (← `stdout`) threads.
    pub(crate) fn start(
        stdin: ChildStdin,
        stdout: ChildStdout,
        name: &str,
    ) -> std::io::Result<Self> {
        let (out_tx, out_rx) = mpsc::channel::<Outgoing>();
        let (in_tx, in_rx) = mpsc::channel::<Incoming>();
        thread::Builder::new()
            .name(format!("sandbox-tx {name}"))
            .spawn(move || {
                let mut w = BufWriter::new(stdin);
                while let Ok((req, payload)) = out_rx.recv() {
                    if protocol::send(&mut w, &req, &payload).is_err() {
                        break;
                    }
                }
                // Dropping `w` closes the pipe: the sandbox sees end of stream and exits.
            })?;
        thread::Builder::new()
            .name(format!("sandbox-rx {name}"))
            .spawn(move || {
                let mut r = BufReader::new(stdout);
                while let Ok(Some(msg)) = protocol::recv::<Response>(&mut r) {
                    if in_tx.send(msg).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            tx: Some(out_tx),
            rx: in_rx,
            next_id: 1,
        })
    }

    /// Sends a request and waits for its response.
    pub(crate) fn call(
        &mut self,
        body: RequestBody,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<(ResponseBody, Vec<u8>), RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let tx = self.tx.as_ref().ok_or(RpcError::Disconnected)?;
        tx.send((Request { id, body }, payload))
            .map_err(|_| RpcError::Disconnected)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok((resp, payload)) if resp.id == id => {
                    return match resp.body {
                        ResponseBody::Error { message } => Err(RpcError::Remote(message)),
                        body => Ok((body, payload)),
                    };
                }
                // A late answer to an earlier, timed-out request.
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return Err(RpcError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(RpcError::Disconnected),
            }
        }
    }

    /// Sends a request without waiting (shutdown).
    pub(crate) fn notify(&mut self, body: RequestBody) {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(tx) = &self.tx {
            let _ = tx.send((Request { id, body }, Vec::new()));
        }
    }

    /// Closes the host → sandbox direction (the writer thread drops the pipe after sending what
    /// is queued).
    pub(crate) fn close(&mut self) {
        self.tx = None;
    }
}
