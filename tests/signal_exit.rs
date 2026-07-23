//! Graceful Ctrl-C/SIGTERM at the PROCESS boundary (issue #149): a fatal signal
//! cancels the main task so its `finally:` blocks run — the `Runtime.exit:`
//! contract — and the process exits 128+signo quietly (no error banner, a real
//! exit status rather than a signal death). A second signal hard-exits, so a hung
//! `finally:` cannot make the process unkillable.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct QnRun {
    child: Child,
    stdout: BufReader<ChildStdout>,
    script: std::path::PathBuf,
    done: Arc<AtomicBool>,
}

impl Drop for QnRun {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = std::fs::remove_file(&self.script);
    }
}

/// Spawn a child qn on `script`, stdout piped. A watchdog SIGKILLs the child if a
/// test wedges (a blocked `read_line` then sees EOF and panics with context);
/// `tag` keeps concurrent tests' temp files apart.
fn spawn_qn(tag: &str, script: &str) -> QnRun {
    let path = std::env::temp_dir().join(format!("quoin_sig_{tag}_{}.qn", std::process::id()));
    std::fs::write(&path, script).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn");
    let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
    let done = Arc::new(AtomicBool::new(false));
    let pid = child.id() as libc::pid_t;
    let watchdog_done = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(45));
        if !watchdog_done.load(Ordering::SeqCst) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    });
    QnRun {
        child,
        stdout,
        script: path,
        done,
    }
}

impl QnRun {
    /// Block until the child prints exactly `want` on a line of its own. EOF first
    /// (the child died or the watchdog fired) fails the test with what did arrive.
    fn read_until_line(&mut self, want: &str) {
        let mut seen = String::new();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read child stdout");
            if n == 0 {
                panic!("child ended before printing {want:?}; stdout so far: {seen:?}");
            }
            seen.push_str(&line);
            if line.trim_end() == want {
                return;
            }
        }
    }

    fn signal(&self, signo: libc::c_int) {
        let rc = unsafe { libc::kill(self.child.id() as libc::pid_t, signo) };
        assert_eq!(rc, 0, "kill({signo}) failed");
    }

    /// Wait for exit (bounded well under the scripts' 60s sleeps), then hand back
    /// (exit code, rest of stdout, stderr) for the assertions.
    fn wait_for_exit(&mut self) -> (Option<i32>, String, String) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "child still running 20s after the signal"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut rest = String::new();
        self.stdout.read_to_string(&mut rest).expect("drain stdout");
        let mut err = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            stderr.read_to_string(&mut err).expect("drain stderr");
        }
        (status.code(), rest, err)
    }
}

/// The issue #149 repro: the signal lands mid-`Async.sleep:`, the `finally:` runs,
/// and the exit is a *status* of 128+signo — not an unhandled signal death (which
/// would surface here as `code() == None`) and not an error banner.
fn signal_runs_finally(tag: &str, signo: libc::c_int, code: i32) {
    let mut run = spawn_qn(
        tag,
        "{\n    'STARTED'.print;\n    Async.sleep:60000\n}.finally:{\n    'FINALLY-RAN'.print\n}\n",
    );
    run.read_until_line("STARTED");
    run.signal(signo);
    let (exit, rest, err) = run.wait_for_exit();
    assert_eq!(exit, Some(code), "stderr: {err}");
    assert!(rest.contains("FINALLY-RAN"), "stdout rest: {rest:?}");
    assert_eq!(err, "", "a signal exit is quiet");
}

#[test]
fn sigint_unwinds_finally_and_exits_130() {
    signal_runs_finally("int", libc::SIGINT, 130);
}

#[test]
fn sigterm_unwinds_finally_and_exits_143() {
    signal_runs_finally("term", libc::SIGTERM, 143);
}

#[test]
fn a_catch_cannot_swallow_the_signal() {
    let mut run = spawn_qn(
        "catch",
        "{\n    'STARTED'.print;\n    Async.sleep:60000\n}.catch:{ |e| 'CAUGHT'.print } \
         finally:{ 'FINALLY-RAN'.print }\n",
    );
    run.read_until_line("STARTED");
    run.signal(libc::SIGINT);
    let (exit, rest, err) = run.wait_for_exit();
    assert_eq!(exit, Some(130), "stderr: {err}");
    assert!(rest.contains("FINALLY-RAN"), "stdout rest: {rest:?}");
    assert!(
        !rest.contains("CAUGHT"),
        "the signal must be uncatchable; stdout rest: {rest:?}"
    );
}

/// The signal lands with main parked somewhere other than plain I/O; each caller
/// picks the park state. Same invariants as the sleep tests: exit 130, `finally:`
/// ran.
fn sigint_unwinds(tag: &str, script: &str) {
    let mut run = spawn_qn(tag, script);
    run.read_until_line("STARTED");
    run.signal(libc::SIGINT);
    let (exit, rest, err) = run.wait_for_exit();
    assert_eq!(exit, Some(130), "stderr: {err}");
    assert!(rest.contains("FINALLY-RAN"), "stdout rest: {rest:?}");
}

/// Main parked on a rendezvous-channel receive (the sender is 60s away): the
/// driver-side cancel takes the parked-on-channel nudge.
#[test]
fn a_channel_parked_task_is_interrupted() {
    sigint_unwinds(
        "chan",
        "var ch = Channel.new\n{\n    'STARTED'.print;\n    Task.spawn:{ Async.sleep:60000; \
         ch.send:1 };\n    ch.receive\n}.finally:{\n    'FINALLY-RAN'.print\n}\n",
    );
}

/// Main parked joining a sleeping task: the cancel dequeues it from the target's
/// waiters and wakes it.
#[test]
fn a_join_parked_task_is_interrupted() {
    sigint_unwinds(
        "join",
        "var t = Task.spawn:{ Async.sleep:60000 }\n{\n    'STARTED'.print;\n    \
         t.join\n}.finally:{\n    'FINALLY-RAN'.print\n}\n",
    );
}

/// Main parked in a timed join (`Async.timeout:do:` around a join): the cancel
/// must also disarm the deadline timer on its way through.
#[test]
fn a_timed_join_parked_task_is_interrupted() {
    sigint_unwinds(
        "tjoin",
        "var t = Task.spawn:{ Async.sleep:60000 }\n{\n    'STARTED'.print;\n    \
         Async.timeout:60000 do:{ t.join }\n}.finally:{\n    'FINALLY-RAN'.print\n}\n",
    );
}

/// A spinning main task never parks, so the cancel lands via the live flag at a
/// batch boundary (the `loaded` path of `request_cancel_from_driver`) rather than
/// by aborting an I/O future.
#[test]
fn a_compute_bound_task_is_interrupted_at_a_batch_boundary() {
    let mut run = spawn_qn(
        "spin",
        "var i = 0\n{\n    'STARTED'.print;\n    { true }.whileDo:{ i = i + 1 }\n}.finally:{\n    \
         'FINALLY-RAN'.print\n}\n",
    );
    run.read_until_line("STARTED");
    run.signal(libc::SIGINT);
    let (exit, rest, err) = run.wait_for_exit();
    assert_eq!(exit, Some(130), "stderr: {err}");
    assert!(rest.contains("FINALLY-RAN"), "stdout rest: {rest:?}");
}

#[test]
fn a_second_signal_escapes_a_hung_finally() {
    let mut run = spawn_qn(
        "twice",
        "{\n    'STARTED'.print;\n    Async.sleep:60000\n}.finally:{\n    \
         'FINALLY-HUNG'.print;\n    Async.sleep:60000\n}\n",
    );
    run.read_until_line("STARTED");
    run.signal(libc::SIGINT);
    // The first signal's unwind is now stuck in the finally's own sleep …
    run.read_until_line("FINALLY-HUNG");
    // … so the second one must hard-exit instead of waiting politely.
    run.signal(libc::SIGINT);
    let (exit, _rest, _err) = run.wait_for_exit();
    assert_eq!(exit, Some(130));
}
