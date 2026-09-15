//! The LV2 **worker** (`worker:schedule` / `worker:interface`, ADR-008 Amendment 8 §2): two
//! preallocated byte rings per activation — requests (the thread running the plugin → the
//! worker) and responses (the worker → the thread running the plugin) — each carrying frames of
//! `[u32 size][size bytes]`, committed atomically, so neither side allocates or locks.
//!
//! - **Realtime:** a worker thread ([`WorkerThread`]) calls `work` for each request; the audio
//!   thread delivers the responses (`work_response`) after each `run`, then calls `end_run`.
//! - **Offline** (and the activation's latency probe): no thread; the thread running the plugin
//!   services the requests itself right after `run`, before delivering — the synchronous mode
//!   the worker spec allows, which keeps offline renders deterministic.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use vox_lv2_abi::{
    LV2_Handle, LV2_WORKER_ERR_NO_SPACE, LV2_WORKER_ERR_UNKNOWN, LV2_WORKER_SUCCESS,
    LV2_Worker_Interface,
};

/// Bytes per ring.
pub(crate) const RING_BYTES: usize = 1 << 16;
/// How long the idle worker thread sleeps between polls.
const IDLE: Duration = Duration::from_millis(1);

/// Writes one frame (no allocation): `LV2_WORKER_ERR_NO_SPACE` when it doesn't fit.
pub(crate) fn write_frame(tx: &mut Producer<u8>, data: *const c_void, size: u32) -> u32 {
    let len = size as usize;
    if data.is_null() && len > 0 {
        return LV2_WORKER_ERR_UNKNOWN;
    }
    let Ok(chunk) = tx.write_chunk_uninit(4 + len) else {
        return LV2_WORKER_ERR_NO_SPACE;
    };
    let body: &[u8] = if len == 0 {
        &[]
    } else {
        // SAFETY: the caller's `size` bytes at `data` (LV2's contract), read during the call.
        unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) }
    };
    chunk.fill_from_iter(size.to_le_bytes().into_iter().chain(body.iter().copied()));
    LV2_WORKER_SUCCESS
}

/// Reads one frame into `scratch` (≥ [`RING_BYTES`]), returning its size.
fn read_frame(rx: &mut Consumer<u8>, scratch: &mut [u8]) -> Option<usize> {
    let len = {
        let head = rx.read_chunk(4).ok()?;
        let (a, b) = head.as_slices();
        let mut bytes = [0u8; 4];
        for (dst, src) in bytes.iter_mut().zip(a.iter().chain(b)) {
            *dst = *src;
        }
        u32::from_le_bytes(bytes) as usize
    };
    // Frames are committed whole, so the body is there once the header is.
    if len + 4 > scratch.len() || rx.slots() < len + 4 {
        return None;
    }
    rx.pop_entire_slice(&mut scratch[..4 + len]).ok()?;
    scratch.copy_within(4..4 + len, 0);
    Some(len)
}

unsafe extern "C" fn respond(handle: *mut c_void, size: u32, data: *const c_void) -> u32 {
    if handle.is_null() {
        return LV2_WORKER_ERR_UNKNOWN;
    }
    // SAFETY: `handle` is the response producer of the side calling `work` (exclusive to it).
    write_frame(unsafe { &mut *(handle as *mut Producer<u8>) }, data, size)
}

/// The plugin thread's half: delivers responses.
pub(crate) struct AudioSide {
    resp_rx: Consumer<u8>,
    scratch: Vec<u8>,
}

impl AudioSide {
    /// Delivers every pending response (`work_response`), then calls `end_run`. No allocation.
    ///
    /// # Safety
    /// `iface` and `handle` belong to a live, active instance; called on the thread running it.
    pub(crate) unsafe fn deliver(&mut self, iface: &LV2_Worker_Interface, handle: LV2_Handle) {
        while let Some(len) = read_frame(&mut self.resp_rx, &mut self.scratch) {
            if let Some(f) = iface.work_response {
                // SAFETY: caller's contract; the body lives for the call.
                unsafe { f(handle, len as u32, self.scratch.as_ptr().cast()) };
            }
        }
        if let Some(f) = iface.end_run {
            // SAFETY: caller's contract.
            unsafe { f(handle) };
        }
    }
}

/// The worker's half: services requests.
pub(crate) struct WorkerSide {
    req_rx: Consumer<u8>,
    resp_tx: Producer<u8>,
    scratch: Vec<u8>,
}

impl WorkerSide {
    /// Calls `work` for every pending request; `true` if there was any.
    ///
    /// # Safety
    /// `iface` and `handle` belong to a live instance; `work` may run concurrently with `run`
    /// (the worker spec), but never on two threads at once.
    pub(crate) unsafe fn service(
        &mut self,
        iface: &LV2_Worker_Interface,
        handle: LV2_Handle,
    ) -> bool {
        let Some(work) = iface.work else {
            return false;
        };
        let mut any = false;
        while let Some(len) = read_frame(&mut self.req_rx, &mut self.scratch) {
            any = true;
            // SAFETY: caller's contract; the request lives for the call; `respond` gets the
            // response producer, exclusive to this side.
            unsafe {
                work(
                    handle,
                    respond,
                    (&raw mut self.resp_tx).cast(),
                    len as u32,
                    self.scratch.as_ptr().cast(),
                )
            };
        }
        any
    }
}

/// One activation's rings: the request producer (for `worker:schedule`), and both halves.
pub(crate) fn rings() -> (Producer<u8>, AudioSide, WorkerSide) {
    let (req_tx, req_rx) = RingBuffer::new(RING_BYTES);
    let (resp_tx, resp_rx) = RingBuffer::new(RING_BYTES);
    (
        req_tx,
        AudioSide {
            resp_rx,
            scratch: vec![0; RING_BYTES],
        },
        WorkerSide {
            req_rx,
            resp_tx,
            scratch: vec![0; RING_BYTES],
        },
    )
}

/// A raw pointer that may cross to the worker thread (see [`WorkerThread::spawn`]'s contract).
struct SendPtr<T>(*const T);

// SAFETY: the pointee outlives the thread (joined before the instance deactivates) and the
// worker spec allows `work` on another thread.
unsafe impl<T> Send for SendPtr<T> {}

/// The realtime worker thread of one activation (stopped and joined on drop).
pub(crate) struct WorkerThread {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerThread {
    /// Starts servicing `side` on its own thread.
    ///
    /// # Safety
    /// `iface` and `handle` stay valid until the returned thread is dropped (the instance
    /// deactivates only after that).
    pub(crate) unsafe fn spawn(
        mut side: WorkerSide,
        iface: *const LV2_Worker_Interface,
        handle: LV2_Handle,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (iface, handle) = (SendPtr(iface), SendPtr(handle.cast_const()));
        let thread = std::thread::Builder::new()
            .name("sandbox-lv2-worker".into())
            .spawn(move || {
                let (iface, handle) = (iface, handle);
                while !flag.load(Ordering::Acquire) {
                    // SAFETY: the spawn contract keeps both valid while the thread runs.
                    if !unsafe { side.service(&*iface.0, handle.0.cast_mut()) } {
                        std::thread::sleep(IDLE);
                    }
                }
            })
            .map_err(|e| format!("worker thread: {e}"))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for WorkerThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static SEEN: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn work(
        _h: LV2_Handle,
        respond: vox_lv2_abi::LV2_Worker_Respond_Function,
        handle: *mut c_void,
        size: u32,
        data: *const c_void,
    ) -> u32 {
        // SAFETY: the host's respond with its handle; echoes the request.
        unsafe { respond(handle, size, data) }
    }

    unsafe extern "C" fn work_response(_h: LV2_Handle, size: u32, body: *const c_void) -> u32 {
        // SAFETY: `size` bytes of body.
        let bytes = unsafe { std::slice::from_raw_parts(body.cast::<u8>(), size as usize) };
        SEEN.fetch_add(bytes.iter().map(|&b| u32::from(b)).sum(), Ordering::Relaxed);
        LV2_WORKER_SUCCESS
    }

    #[test]
    fn frames_round_trip_through_work_and_back() {
        let iface = LV2_Worker_Interface {
            work: Some(work),
            work_response: Some(work_response),
            end_run: None,
        };
        let (mut tx, mut audio, mut worker) = rings();
        for req in [[1u8, 2, 3].as_slice(), &[40], &[]] {
            assert_eq!(
                write_frame(&mut tx, req.as_ptr().cast(), req.len() as u32),
                0
            );
        }
        // SAFETY: the test's own interface; no instance handle is used.
        unsafe {
            assert!(worker.service(&iface, std::ptr::null_mut()));
            assert!(!worker.service(&iface, std::ptr::null_mut()));
            audio.deliver(&iface, std::ptr::null_mut());
        }
        assert_eq!(SEEN.load(Ordering::Relaxed), 46);
        let big = vec![0u8; RING_BYTES];
        assert_eq!(
            write_frame(&mut tx, big.as_ptr().cast(), big.len() as u32),
            LV2_WORKER_ERR_NO_SPACE
        );
    }
}
