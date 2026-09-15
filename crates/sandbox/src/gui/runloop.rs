//! The sandbox main thread's **run loop** for plugin GUIs (T-901): the timers and file
//! descriptors a plugin registers (CLAP `timer-support` / `posix-fd-support`, VST3
//! `Linux::IRunLoop`), serviced by the server's event loop between control requests.
//!
//! Main thread only: the registrations live in a thread-local, and a registration from any other
//! thread is refused (the formats declare these calls main-thread). A callback may register or
//! unregister (itself included) while it runs: callbacks are called with no borrow held, and a
//! timer is looked up again before each call.

// Descriptors are a unix thing (the Windows loop pumps messages instead).
#![cfg_attr(not(unix), allow(dead_code))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The descriptor is readable.
pub(crate) const FD_READ: u32 = 1 << 0;
/// The descriptor is writable.
pub(crate) const FD_WRITE: u32 = 1 << 1;
/// The descriptor has an error (or hung up).
pub(crate) const FD_ERROR: u32 = 1 << 2;

/// The shortest timer period served (a plugin asking for 0 ms doesn't spin the main thread).
const MIN_PERIOD: Duration = Duration::from_millis(1);

/// A timer callback; gets its timer's id.
type TimerFn = Rc<dyn Fn(u32)>;
type FdFn = Rc<dyn Fn(u32)>;

struct Timer {
    id: u32,
    period: Duration,
    due: Instant,
    f: TimerFn,
}

struct Fd {
    fd: i32,
    flags: u32,
    f: FdFn,
}

#[derive(Default)]
struct RunLoop {
    timers: Vec<Timer>,
    fds: Vec<Fd>,
    next_id: u32,
}

thread_local! {
    static LOOP: RefCell<RunLoop> = RefCell::new(RunLoop::default());
    static MAIN: Cell<bool> = const { Cell::new(false) };
}

/// Declares the calling thread the sandbox's main thread (the server, at start).
pub(crate) fn mark_main_thread() {
    MAIN.with(|m| m.set(true));
}

fn on_main_thread() -> bool {
    MAIN.with(Cell::get)
}

/// Registers a timer firing every `period_ms` (at least 1 ms); `None` off the main thread.
pub(crate) fn add_timer(period_ms: u32, f: TimerFn) -> Option<u32> {
    if !on_main_thread() {
        return None;
    }
    let period = Duration::from_millis(u64::from(period_ms)).max(MIN_PERIOD);
    LOOP.with(|l| {
        let mut l = l.borrow_mut();
        l.next_id = l.next_id.wrapping_add(1).max(1);
        let id = l.next_id;
        l.timers.push(Timer {
            id,
            period,
            due: Instant::now() + period,
            f,
        });
        Some(id)
    })
}

/// Unregisters timer `id`; `false` if unknown (or off the main thread).
pub(crate) fn remove_timer(id: u32) -> bool {
    on_main_thread()
        && LOOP.with(|l| {
            let mut l = l.borrow_mut();
            let before = l.timers.len();
            l.timers.retain(|t| t.id != id);
            l.timers.len() != before
        })
}

/// Watches `fd` for `flags` (`FD_*`); `false` if it is already watched (or off the main
/// thread).
pub(crate) fn add_fd(fd: i32, flags: u32, f: FdFn) -> bool {
    on_main_thread()
        && LOOP.with(|l| {
            let mut l = l.borrow_mut();
            if fd < 0 || l.fds.iter().any(|x| x.fd == fd) {
                return false;
            }
            l.fds.push(Fd { fd, flags, f });
            true
        })
}

/// Changes what `fd` is watched for; `false` if it isn't watched.
pub(crate) fn modify_fd(fd: i32, flags: u32) -> bool {
    on_main_thread()
        && LOOP.with(|l| {
            l.borrow_mut()
                .fds
                .iter_mut()
                .find(|x| x.fd == fd)
                .map(|x| x.flags = flags)
                .is_some()
        })
}

/// Stops watching `fd`; `false` if it wasn't watched.
pub(crate) fn remove_fd(fd: i32) -> bool {
    on_main_thread()
        && LOOP.with(|l| {
            let mut l = l.borrow_mut();
            let before = l.fds.len();
            l.fds.retain(|x| x.fd != fd);
            l.fds.len() != before
        })
}

/// When the next timer is due.
pub(crate) fn next_due() -> Option<Instant> {
    LOOP.with(|l| l.borrow().timers.iter().map(|t| t.due).min())
}

/// The watched descriptors and what for (`FD_*`).
pub(crate) fn fds() -> Vec<(i32, u32)> {
    LOOP.with(|l| l.borrow().fds.iter().map(|x| (x.fd, x.flags)).collect())
}

/// Fires every timer due at `now`, once each; a timer that fell behind skips the periods it
/// missed.
pub(crate) fn fire_timers(now: Instant) {
    let due: Vec<u32> = LOOP.with(|l| {
        l.borrow()
            .timers
            .iter()
            .filter(|t| t.due <= now)
            .map(|t| t.id)
            .collect()
    });
    for id in due {
        let f = LOOP.with(|l| {
            let mut l = l.borrow_mut();
            let t = l.timers.iter_mut().find(|t| t.id == id)?;
            t.due += t.period;
            if t.due <= now {
                t.due = now + t.period;
            }
            Some(t.f.clone())
        });
        if let Some(f) = f {
            f(id);
        }
    }
}

/// Calls `fd`'s callback with `flags` (what `poll` reported), if it is still watched.
pub(crate) fn fire_fd(fd: i32, flags: u32) {
    let f = LOOP.with(|l| {
        l.borrow()
            .fds
            .iter()
            .find(|x| x.fd == fd)
            .map(|x| x.f.clone())
    });
    if let Some(f) = f {
        f(flags);
    }
}

/// Drops every registration (the plugin that made them is going away).
pub(crate) fn clear() {
    LOOP.with(|l| {
        let mut l = l.borrow_mut();
        l.timers.clear();
        l.fds.clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> (Rc<Cell<u32>>, TimerFn) {
        let n = Rc::new(Cell::new(0));
        let c = n.clone();
        (n, Rc::new(move |_| c.set(c.get() + 1)))
    }

    #[test]
    fn timers_fire_when_due_and_can_be_removed_even_from_a_callback() {
        std::thread::spawn(|| {
            assert!(
                add_timer(10, Rc::new(|_| {})).is_none(),
                "not the main thread"
            );
            mark_main_thread();
            let (a, fa) = counter();
            let id_a = add_timer(10, fa).unwrap();
            let (b, fb) = counter();
            let _id_b = add_timer(0, fb).unwrap();
            let t0 = Instant::now();
            fire_timers(t0);
            assert_eq!((a.get(), b.get()), (0, 0), "nothing due yet");
            assert!(next_due().unwrap() <= t0 + Duration::from_millis(10));
            fire_timers(t0 + Duration::from_millis(15));
            assert_eq!((a.get(), b.get()), (1, 1));
            // Far behind: one call, not a burst.
            fire_timers(t0 + Duration::from_secs(1));
            assert_eq!(a.get(), 2);
            assert!(remove_timer(id_a));
            assert!(!remove_timer(id_a));
            fire_timers(t0 + Duration::from_secs(2));
            assert_eq!(a.get(), 2);
            // A callback that removes another timer due in the same round: that one isn't called.
            let (c, fc) = counter();
            let id_c = Rc::new(Cell::new(0));
            let id_c2 = id_c.clone();
            let first = add_timer(
                1,
                Rc::new(move |_| {
                    remove_timer(id_c2.get());
                }),
            )
            .unwrap();
            id_c.set(add_timer(1, fc).unwrap());
            fire_timers(Instant::now() + Duration::from_secs(3));
            assert_eq!(c.get(), 0);
            assert!(remove_timer(first));
            clear();
            assert!(next_due().is_none());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn descriptors_are_watched_modified_and_fired() {
        std::thread::spawn(|| {
            mark_main_thread();
            let got = Rc::new(Cell::new(0u32));
            let g = got.clone();
            assert!(add_fd(7, FD_READ, Rc::new(move |f| g.set(f))));
            assert!(!add_fd(7, FD_READ, Rc::new(|_| {})), "already watched");
            assert!(!add_fd(-1, FD_READ, Rc::new(|_| {})));
            assert_eq!(fds(), vec![(7, FD_READ)]);
            assert!(modify_fd(7, FD_READ | FD_WRITE));
            assert_eq!(fds(), vec![(7, FD_READ | FD_WRITE)]);
            fire_fd(7, FD_WRITE);
            assert_eq!(got.get(), FD_WRITE);
            fire_fd(8, FD_READ);
            assert!(remove_fd(7));
            assert!(!remove_fd(7));
            assert!(!modify_fd(7, FD_READ));
            assert!(fds().is_empty());
            let _ = FD_ERROR;
        })
        .join()
        .unwrap();
    }
}
