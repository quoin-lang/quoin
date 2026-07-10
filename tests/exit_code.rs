//! End-to-end process exit codes: `qn <file>` propagates failure (an uncaught error
//! exits 1, not 0), and `Runtime.exit:` requests a specific status — uncatchable by
//! `catch:`, running `finally` blocks on the way out, and process-wide even from a
//! spawned task.

use std::process::{Command, Output};

/// Run the built `qn` on an inline script (written to a temp file) from the package
/// root, so the CWD-relative `qnlib/` prelude resolves.
fn run_script(name: &str, src: &str) -> Output {
    let path = std::env::temp_dir().join(format!("quoin_exit_{}_{}.qn", name, std::process::id()));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    out
}

fn run_eval(expr: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("-e")
        .arg(expr)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn -e")
}

#[test]
fn clean_script_exits_zero() {
    let out = run_script("ok", "(1 + 1).print;\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn falsy_final_value_still_exits_zero() {
    // Run mode gates on errors only — a script whose last expression is falsy is
    // not a failure (unlike `qn test`, which gates on the harness's final boolean).
    let out = run_script("falsy", "'side effect'.print;\nfalse;\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn uncaught_error_exits_one() {
    let out = run_script("boom", "Error.throw:'boom';\n");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("VM execution error"),
        "stderr should report the error, got: {stderr}"
    );
}

#[test]
fn runtime_exit_sets_status() {
    let out = run_eval("Runtime.exit:3");
    assert_eq!(out.status.code(), Some(3));
    // The exit is a deliberate request, not an error — nothing should be printed.
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    assert!(out.stderr.is_empty(), "stderr: {:?}", out.stderr);
}

#[test]
fn runtime_exit_no_arg_is_zero() {
    let out = run_script("exit0", "Runtime.exit;\n'unreached'.print;\n");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
}

#[test]
fn catch_cannot_swallow_exit() {
    let out = run_script(
        "catch",
        "{ Runtime.exit:5 }.catch:{ |e| 'caught'.print };\n'after'.print;\n",
    );
    assert_eq!(out.status.code(), Some(5));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("caught"), "handler ran: {stdout}");
    assert!(!stdout.contains("after"), "execution continued: {stdout}");
}

#[test]
fn finally_runs_before_exit() {
    let out = run_script(
        "finally",
        "{ Runtime.exit:7 }.catch:{ |e| 'caught'.print } finally:{ 'cleanup'.print };\n",
    );
    assert_eq!(out.status.code(), Some(7));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cleanup"), "finally skipped: {stdout}");
    assert!(!stdout.contains("caught"), "handler ran: {stdout}");
}

#[test]
fn repl_piped_exit_sets_status() {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("repl")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn qn repl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Runtime.exit:8\n'unreached'.print\n")
        .unwrap();
    let out = child.wait_with_output().expect("repl exits");
    assert_eq!(out.status.code(), Some(8));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("unreached"), "repl continued: {stdout}");
}

#[test]
fn exit_from_spawned_task_is_process_wide() {
    let out = run_script(
        "task",
        "var t = Task.spawn:{ Runtime.exit:9 };\nt.join;\n'after'.print;\n",
    );
    assert_eq!(out.status.code(), Some(9));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("after"), "main task continued: {stdout}");
}

/// A *compile* error is a user error, not a VM bug. Every entry point must report it, with a
/// source location, and exit 1. The run path used to `panic!` inside `arena.mutate_root`,
/// printing a Rust backtrace note and exiting 101 — and it fired on exactly the two mistakes
/// the strict `var`/`let` rule invites, so it was the first thing a migrating user saw.
/// File-based modes (`qn FILE`, `qn check`) render `file:line:col: error: message` like a type
/// warning; string-based modes (`-e`) embed `(line …, column …)` like their parse errors.
mod compile_errors {
    use super::{run_eval, run_script};
    use std::process::Command;

    /// Every source that fails to compile: the phrase its message must contain, and the
    /// 1-based (line, column) the error must point at.
    const CASES: &[(&str, &str, &str, (usize, usize))] = &[
        (
            "undeclared",
            "typo = 5;\n",
            "undeclared local `typo`",
            (1, 1),
        ),
        (
            "let_reassign",
            "let x = 1;\nx = 2;\n",
            "cannot reassign `let`",
            (2, 1),
        ),
        // A top-level method definition used to compile and then die at runtime with
        // "Cannot extend sealed class [/]Nil" — an error about a class the user never
        // wrote (top-level `self` is nil). Now rejected at compile time, naming the fix.
        (
            "top_level_method",
            "greet -> { 42 };\n",
            "methods live in classes",
            (1, 1),
        ),
    ];

    fn assert_clean_failure(
        what: &str,
        code: Option<i32>,
        stderr: &str,
        needle: &str,
        loc_marker: &str,
    ) {
        assert_eq!(
            code,
            Some(1),
            "{what}: expected exit 1, got {code:?}\n{stderr}"
        );
        assert!(
            stderr.contains(needle),
            "{what}: message should name the problem ({needle})\n{stderr}"
        );
        assert!(
            stderr.contains(loc_marker),
            "{what}: message should carry the source location ({loc_marker})\n{stderr}"
        );
        // The two tells of a panic: never acceptable for a user's typo.
        assert!(
            !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
            "{what}: a compile error must not panic\n{stderr}"
        );
    }

    #[test]
    fn a_script_reports_and_exits_one() {
        for (name, src, needle, (line, col)) in CASES {
            let out = run_script(name, src);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_clean_failure(
                &format!("qn {name}.qn"),
                out.status.code(),
                &stderr,
                needle,
                &format!(":{line}:{col}: error:"),
            );
        }
    }

    #[test]
    fn eval_and_check_agree_with_the_run_path() {
        for (name, src, needle, (line, col)) in CASES {
            let out = run_eval(src);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_clean_failure(
                &format!("qn -e ({name})"),
                out.status.code(),
                &stderr,
                needle,
                &format!("(line {line}, column {col})"),
            );

            let path = std::env::temp_dir().join(format!(
                "quoin_check_{}_{}.qn",
                name,
                std::process::id()
            ));
            std::fs::write(&path, src).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_qn"))
                .arg("check")
                .arg(&path)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
                .expect("run qn check");
            let _ = std::fs::remove_file(&path);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_clean_failure(
                &format!("qn check ({name})"),
                out.status.code(),
                &stderr,
                needle,
                &format!(":{line}:{col}: error:"),
            );
        }
    }
}
