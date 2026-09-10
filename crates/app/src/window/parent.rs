//! Watching the tray process that spawned this one, so the window exits
//! along with it.
//!
//! A dashboard window with no tray behind it is a broken app — its server
//! may be gone, and there is nothing left to quit it from cleanly if the
//! tray crashed rather than exiting on purpose. Polling a pid on a timer
//! would work, but it means waking this idle process up forever just to
//! check something that changes at most once in its whole lifetime. Instead
//! this registers interest in the parent's exit with the kernel
//! (`EVFILT_PROC`/`NOTE_EXIT` on a `kqueue`) and blocks in `kevent` on a
//! spawned thread: zero CPU while waiting, and the wakeup happens exactly
//! once, exactly when it matters.

use std::thread;

/// Register interest in `pid`'s exit and block until it happens.
///
/// Returns `true` once the kernel has delivered the exit event, or `false`
/// immediately if registration itself failed — most likely because `pid`
/// was already gone by the time this ran, which the caller should treat
/// exactly like "it exited already."
///
/// Split out from `watch` so a test can call it directly without going
/// through `std::process::exit`, and shared with `update::relaunch`, which
/// waits on the tray the same way.
pub(crate) fn wait_for_exit(pid: u32) -> bool {
    // SAFETY: `kqueue()` takes no arguments; its only failure mode is
    // returning -1, which is checked below before the descriptor is used
    // for anything.
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return false;
    }

    // SAFETY: `kevent` is a plain C struct of integers and pointers, for
    // which all-zero is a valid (if meaningless) value; every field that
    // matters is set below before the struct is handed to the kernel.
    let mut change: libc::kevent = unsafe { std::mem::zeroed() };
    change.ident = pid as libc::uintptr_t;
    change.filter = libc::EVFILT_PROC;
    change.flags = libc::EV_ADD | libc::EV_ENABLE;
    change.fflags = libc::NOTE_EXIT;

    // SAFETY: `kq` is the valid, just-created descriptor above; `change` is
    // one well-formed registration and the call requests no output events
    // (`nevents` 0, so the trailing null pointer and immediate-return
    // timeout are never touched) — this call only registers interest, it
    // does not wait.
    let registered =
        unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if registered < 0 {
        // SAFETY: `kq` is a valid descriptor owned by this function; on
        // this early-return path nothing else references it.
        unsafe { libc::close(kq) };
        return false;
    }

    // SAFETY: as above — an all-zero `kevent` is a valid output slot.
    let mut event: libc::kevent = unsafe { std::mem::zeroed() };
    // SAFETY: `kq` is valid and has the registration above pending on it;
    // `event` is a single output slot for `kevent` to fill; a null timeout
    // means block indefinitely, which is the whole point — no wakeup until
    // the kernel actually has one to deliver.
    let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
    // SAFETY: `kq` is a valid descriptor owned by this function and is done
    // being used either way.
    unsafe { libc::close(kq) };
    n > 0
}

/// Spawn a thread that blocks until `parent` exits, then exits this process.
///
/// Called once at startup with the tray's pid. If the tray is already gone
/// by the time registration runs, `wait_for_exit` returns immediately and
/// this still exits — there is nothing to host a window for either way.
pub fn watch(parent: u32) {
    thread::spawn(move || {
        wait_for_exit(parent);
        std::process::exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn returns_soon_after_the_watched_process_exits() {
        let mut child = Command::new("sleep")
            .arg("0.2")
            .spawn()
            .expect("spawn sleep 0.2");
        let pid = child.id();

        let started = Instant::now();
        let exited = wait_for_exit(pid);
        let took = started.elapsed();

        let _ = child.wait(); // reap; the exit event already fired above
        assert!(exited, "wait_for_exit should observe the child exiting");
        assert!(
            took < Duration::from_secs(2),
            "took {took:?} to notice a 0.2s sleep exit"
        );
    }
}
