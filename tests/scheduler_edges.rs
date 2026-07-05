//! Regression tests for scheduler edge cases found by the 2026-07 async audit
//! (repro corpus: `qnlib/stress/audit/sched/`).

use std::process::Command;

fn run_qn(file_stem: &str, script: &str) -> (String, String, bool) {
    let path = std::env::temp_dir().join(format!("qn_{file_stem}.qn"));
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn assert_pass(file_stem: &str, script: &str) {
    let (stdout, stderr, _) = run_qn(file_stem, script);
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn global_deadlock_reports_an_error_instead_of_silent_success() {
    // Pre-fix: the driver's idle break exited 0 with the main task still parked
    // and nothing printed — a deadlocked program looked identical to a successful
    // one. (Uncaught errors exit 0 by convention in run mode, so assert on the
    // diagnostic, not the status.)
    let (stdout, stderr, _) = run_qn(
        "deadlock_exit",
        r#"
'before'.print;
Channel.new.receive;
'unreachable'.print;
"#,
    );
    assert!(
        stdout.contains("before") && !stdout.contains("unreachable"),
        "unexpected stdout:\n{stdout}"
    );
    assert!(
        stderr.contains("deadlock"),
        "no deadlock diagnostic.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn empty_gather_returns_an_empty_list() {
    // Pre-fix: `Async.gather:#()` spawned no children, so nothing ever delivered
    // the result — the caller parked forever and the program exited 0 silently.
    assert_pass(
        "empty_gather",
        r#"
var r = Async.gather:#();
((r == #()) && (r.count == 0)).if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
    );
}
