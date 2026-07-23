//! Randomized-timing stress for the graceful SIGINT path (issue #149): fire the
//! signal at a random instant while a child qn cycles through park states, and
//! hold the invariants — the exit is always a clean status 130 (never a signal
//! death once the driver is up, never an error exit, never a wedge), and a
//! `finally:` whose block was entered always runs. Half the iterations run under
//! `QN_SCHED_STRESS` so preemption and pick order randomize underneath the signal.
//!
//! Tunables: `QN_SIG_STRESS_ITERS` (default 16) scales a local soak;
//! `QN_SIG_STRESS_SEED` pins the run — every failure message carries the seed.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// The state cycler: main rapidly rotates through the park states the driver-side
/// cancel handles — timer sleep, spawn+join, rendezvous-channel send and receive
/// (a background echo task supplies the partner) — plus a compute spin for the
/// batch-boundary path. A `gather:` is deliberately absent: it has no early nudge
/// (`handle.cancel` v1 parity), so it would stall iterations until the watchdog.
const CYCLER: &str = "var ping = Channel.new\nvar pong = Channel.new\nTask.spawn:{ { true \
                      }.whileDo:{ ping.receive; pong.send:1 } }\n{\n    'STARTED'.print;\n    { \
                      true }.whileDo:{\n        Async.sleep:1;\n        var t = Task.spawn:{ 1 + \
                      1 };\n        t.join;\n        ping.send:1;\n        pong.receive;\n        \
                      var i = 0;\n        { i < 20000 }.whileDo:{ i = i + 1 }\n    \
                      }\n}.finally:{\n    'FINALLY-RAN'.print\n}\n";

/// As above, but the `finally:` itself hangs — the double-signal test's target.
const CYCLER_HUNG_FINALLY: &str = "var ping = Channel.new\nvar pong = Channel.new\nTask.spawn:{ \
                                   { true }.whileDo:{ ping.receive; pong.send:1 } }\n{\n    \
                                   'STARTED'.print;\n    { true }.whileDo:{\n        \
                                   Async.sleep:1;\n        ping.send:1;\n        pong.receive\n  \
                                   }\n}.finally:{\n    'FINALLY-HUNG'.print;\n    \
                                   Async.sleep:60000\n}\n";

/// SplitMix64 — no rand dev-dependency for one stream of test jitter.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn seed() -> u64 {
    env_u64(
        "QN_SIG_STRESS_SEED",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock after the epoch")
            .subsec_nanos() as u64
            | 1,
    )
}

struct StressChild {
    child: Child,
    stdout: BufReader<std::process::ChildStdout>,
    script: std::path::PathBuf,
}

impl Drop for StressChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_file(&self.script);
    }
}

fn spawn_cycler(tag: &str, script: &str, sched_stress: Option<u64>) -> StressChild {
    let path = std::env::temp_dir().join(format!("quoin_sigst_{tag}_{}.qn", std::process::id()));
    std::fs::write(&path, script).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = sched_stress {
        cmd.env("QN_SCHED_STRESS", s.to_string());
    }
    let mut child = cmd.spawn().expect("spawn qn");
    let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
    StressChild {
        child,
        stdout,
        script: path,
    }
}

impl StressChild {
    fn signal(&self, signo: libc::c_int, ctx: &str) {
        let rc = unsafe { libc::kill(self.child.id() as libc::pid_t, signo) };
        assert_eq!(rc, 0, "kill({signo}) failed — {ctx}");
    }

    /// Block until `want` arrives on a line of its own; the child never exits on
    /// its own, so EOF means it died early — fail with what it said.
    fn read_until_line(&mut self, want: &str, ctx: &str) {
        let mut seen = String::new();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read child stdout");
            if n == 0 {
                panic!("child ended before printing {want:?} — {ctx}; stdout: {seen:?}");
            }
            seen.push_str(&line);
            if line.trim_end() == want {
                return;
            }
        }
    }

    /// The wedge detector: a graceful exit must land promptly. SIGKILL + fail
    /// otherwise.
    fn wait_bounded(&mut self, ctx: &str) -> (std::process::ExitStatus, String, String) {
        self.wait_bounded_kicking(None, ctx)
    }

    /// As `wait_bounded`, re-sending `kick` every 300ms while the child lives.
    /// Standard signals do not queue: one raised while its predecessor is still
    /// pending is DISCARDED, so two back-to-back `kill`s can coalesce into a
    /// single delivery — keep pressing, as a user would; any delivered invocation
    /// past the first `_exit`s. A kick may race the child's death; that is fine.
    fn wait_bounded_kicking(
        &mut self,
        kick: Option<libc::c_int>,
        ctx: &str,
    ) -> (std::process::ExitStatus, String, String) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut next_kick = Instant::now() + Duration::from_millis(300);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("child still alive 15s after the signal (wedged) — {ctx}");
            }
            if let Some(signo) = kick
                && Instant::now() >= next_kick
            {
                let _ = unsafe { libc::kill(self.child.id() as libc::pid_t, signo) };
                next_kick = Instant::now() + Duration::from_millis(300);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut out = String::new();
        self.stdout.read_to_string(&mut out).expect("drain stdout");
        let mut err = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            stderr.read_to_string(&mut err).expect("drain stderr");
        }
        (status, out, err)
    }
}

/// Whatever else happens, the VM must never panic or report an execution error on
/// a signal exit. (`QN_SCHED_STRESS` announces its seed on stderr; that is fine.)
fn assert_stderr_clean(err: &str, ctx: &str) {
    assert!(
        !err.contains("panic") && !err.contains("VM execution error"),
        "dirty stderr — {ctx}: {err:?}"
    );
}

#[test]
fn randomized_signal_timing_holds_the_invariants() {
    let seed = seed();
    let iters = env_u64("QN_SIG_STRESS_ITERS", 16);
    eprintln!("signal stress: seed={seed} iters={iters}");
    let mut rng = Rng(seed);
    for i in 0..iters {
        // Alternate steady-state (signal after STARTED, mid-cycle) with the
        // startup boundary (blind delay from spawn: compile, prelude, first
        // drive), and interleave scheduler stress through both.
        let steady = i % 2 == 0;
        let sched = ((i / 2) % 2 == 0).then(|| rng.next());
        let ctx = format!("seed={seed} iter={i} steady={steady} sched_stress={sched:?}");
        let mut run = spawn_cycler("timing", CYCLER, sched);
        if steady {
            run.read_until_line("STARTED", &ctx);
            std::thread::sleep(Duration::from_millis(rng.below(40)));
        } else {
            std::thread::sleep(Duration::from_millis(rng.below(250)));
        }
        run.signal(libc::SIGINT, &ctx);
        let (status, out, err) = run.wait_bounded(&ctx);
        assert_stderr_clean(&err, &ctx);
        let started = out.contains("STARTED");
        match (status.code(), status.signal()) {
            (Some(130), _) => {
                // The graceful path: a `finally:` whose block was entered ran.
                if started {
                    assert!(
                        out.contains("FINALLY-RAN"),
                        "finally skipped — {ctx}: {out:?}"
                    );
                }
            }
            // Before the first drive installs the handler (parse/compile), the
            // OS default still applies: a signal death without any guest output.
            (None, Some(libc::SIGINT)) if !steady && !started => {}
            other => panic!("unexpected exit {other:?} — {ctx}; stdout: {out:?} stderr: {err:?}"),
        }
    }
}

#[test]
fn a_second_signal_always_wins_promptly() {
    let seed = seed();
    let iters = env_u64("QN_SIG_STRESS_ITERS", 16) / 2;
    eprintln!("double-signal stress: seed={seed} iters={iters}");
    let mut rng = Rng(seed);
    for i in 0..iters.max(1) {
        let sched = (i % 2 == 0).then(|| rng.next());
        let ctx = format!("seed={seed} iter={i} sched_stress={sched:?}");
        let mut run = spawn_cycler("double", CYCLER_HUNG_FINALLY, sched);
        run.read_until_line("STARTED", &ctx);
        run.signal(libc::SIGINT, &ctx);
        // Anywhere from "before the driver even served the first" to "deep in
        // the hung finally": the second signal must hard-exit regardless.
        std::thread::sleep(Duration::from_millis(rng.below(30)));
        run.signal(libc::SIGINT, &ctx);
        let (status, out, err) = run.wait_bounded_kicking(Some(libc::SIGINT), &ctx);
        assert_stderr_clean(&err, &ctx);
        assert_eq!(
            status.code(),
            Some(130),
            "{ctx}; stdout: {out:?} stderr: {err:?}"
        );
    }
}
