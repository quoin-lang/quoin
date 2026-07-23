//! Graceful fatal-signal exits (issue #149): SIGINT (Ctrl-C) and SIGTERM follow the
//! `Runtime.exit:` contract instead of the OS default of instant death.
//!
//! The first signal latches the conventional exit status (128+signo) and writes one
//! byte down a self-pipe, so a driver idling in its reactor wait wakes immediately.
//! The driver then cancels the main task — whose unwind runs its `finally:` blocks,
//! uncatchable, exactly like `Runtime.exit:` — and `drive_with_frontend` maps the
//! run's result to `ExitRequested(128+signo)` for the normal quiet-exit teardown. A
//! second signal `_exit`s on the spot: a hung `finally:` must never make the process
//! unkillable.
//!
//! Nothing is installed until a driver starts (`drive_to_completion`), so the
//! non-driving subcommands (`qn fmt`, `qn doc`, …) keep the OS default behavior.

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::os::fd::FromRawFd;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Once, OnceLock};

    /// 0 = no signal yet; otherwise the latched exit status (128 + signo).
    static PENDING: AtomicI32 = AtomicI32::new(0);
    /// Self-pipe write end; -1 until `install`. The handler's only channel to the
    /// reactor: `write` is async-signal-safe, and readability wakes `wait`.
    static WAKE_FD: AtomicI32 = AtomicI32::new(-1);
    /// The reactor-registered read end.
    static READER: OnceLock<async_io::Async<File>> = OnceLock::new();

    extern "C" fn on_signal(signo: libc::c_int) {
        let code = 128 + signo;
        // A second fatal signal means the graceful unwind is stuck (or the user is
        // insisting): hard-exit NOW. `_exit`, not `exit` — only async-signal-safe
        // calls in a handler.
        if PENDING.swap(code, Ordering::SeqCst) != 0 {
            unsafe { libc::_exit(code) };
        }
        let fd = WAKE_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            let byte = [1u8];
            unsafe { libc::write(fd, byte.as_ptr().cast(), 1) };
        }
    }

    pub fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let mut fds = [0 as libc::c_int; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return; // no pipe, no graceful path: the OS default stays
            }
            for fd in fds {
                unsafe {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                    libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
                }
            }
            let read_end = unsafe { File::from_raw_fd(fds[0]) };
            let Ok(reader) = async_io::Async::new(read_end) else {
                // `read_end` (fds[0]) closes with the failed `Async`'s drop.
                unsafe { libc::close(fds[1]) };
                return;
            };
            let _ = READER.set(reader);
            WAKE_FD.store(fds[1], Ordering::SeqCst);
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = on_signal as *const () as usize;
                // SA_RESTART: blocking reads elsewhere (rustyline's prompt, worker
                // pump threads) resume seamlessly; the self-pipe carries the wake.
                sa.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut sa.sa_mask);
                libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
                libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
            }
        });
    }

    /// The latched exit status, if a fatal signal has arrived (130 = SIGINT,
    /// 143 = SIGTERM). Monotonic: never unlatches.
    pub fn pending() -> Option<i32> {
        match PENDING.load(Ordering::SeqCst) {
            0 => None,
            code => Some(code),
        }
    }

    /// Complete when a signal's wake byte lands (never, if none arrives). Purely a
    /// waker for the driver's idle reactor wait — `pending` is the source of truth.
    pub async fn wait() {
        let Some(reader) = READER.get() else {
            return std::future::pending().await;
        };
        use futures_lite::AsyncReadExt;
        let mut byte = [0u8; 1];
        let _ = (&mut &*reader).read(&mut byte).await;
    }
}

#[cfg(not(unix))]
mod imp {
    pub fn install() {}

    pub fn pending() -> Option<i32> {
        None
    }

    pub async fn wait() {
        std::future::pending().await
    }
}

pub use imp::{install, pending, wait};
