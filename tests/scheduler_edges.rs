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
