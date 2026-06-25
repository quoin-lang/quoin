//! Regression tests for running REPL / `-e` / piped input **through the scheduler**.
//!
//! The REPL, `qn -e`, and piped stdin used to execute each line synchronously, with no task
//! scheduler — so any async op (a fiber resume, `Async.sleep`, a spawned `Task`, even an
//! iterator, which is fiber-backed) failed with "… outside the VM scheduler". These drive the
//! real `qn` binary to confirm those now work, and that top-level bindings still persist
//! across lines. No network: fibers, `Async.sleep`, and `Task` are deterministic and local.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `qn -e <expr>` and return `(stdout, stderr)`.
fn eval(expr: &str) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("-e")
        .arg(expr)
        .output()
        .expect("run qn -e");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Feed `input` to `qn repl` over a (non-TTY) stdin pipe and return `(stdout, stderr)`.
fn piped_repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn repl");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait qn repl");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn eval_resumes_a_fiber() {
    // The exact reported failure: a `Fiber.resume` from a top-level line.
    let (stdout, stderr) = eval("f = Fiber.new:{ Fiber.yield:10; 20 }; f.resume");
    assert!(
        !stderr.contains("outside the VM scheduler"),
        "fiber resume still hit the no-scheduler path.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.trim() == "10",
        "expected the first yield (10).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn eval_runs_an_iterator() {
    // Iterators are fiber-backed, so `drop:` used to fail outside the scheduler too.
    let (stdout, stderr) = eval("#(1 2 3 4 5).drop:2");
    assert!(
        !stderr.contains("outside the VM scheduler"),
        "iterator hit the no-scheduler path.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("#(3 4 5)"),
        "expected #(3 4 5).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn eval_does_real_async_io() {
    // `Async.sleep` parks on a real timer in the backend — only the scheduler can fulfill it.
    let (stdout, stderr) = eval("Async.sleep: 1; 7");
    assert!(
        !stderr.contains("outside the VM scheduler"),
        "sleep hit the no-scheduler path.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.trim() == "7",
        "expected 7 after sleeping.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn eval_spawns_and_joins_a_task() {
    let (stdout, stderr) = eval("h = Task.spawn:{ Async.sleep: 1; 6 * 7 }; h.join");
    assert!(
        !stderr.contains("outside the VM scheduler"),
        "task spawn/join hit the no-scheduler path.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.trim() == "42",
        "expected the joined result 42.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn piped_repl_resumes_a_fiber_across_lines() {
    // The interactive/piped REPL path (`eval_value`): a fiber bound on one line is resumed on
    // later lines — exercising both the scheduler *and* that top-level bindings persist.
    let (stdout, stderr) = piped_repl(
        "f = Fiber.new:{ Fiber.yield:1; Fiber.yield:2; 3 }\nf.resume\nf.resume\nf.resume\n",
    );
    assert!(
        !stderr.contains("outside the VM scheduler")
            && !stdout.contains("outside the VM scheduler"),
        "piped REPL hit the no-scheduler path.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    for want in ["=> 1", "=> 2", "=> 3"] {
        assert!(
            stdout.contains(want),
            "missing {want:?} in REPL output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}
