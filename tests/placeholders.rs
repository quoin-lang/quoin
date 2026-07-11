//! The placeholder statements end to end: `...` and `!!!` throw typed errors
//! (exit 1 with the class named in the report), `???` warns through `Log.warn:`
//! — a standard log line whose `file:line:col` comes from Log's uniform
//! caller-location capture — and execution continues. Statement-position only:
//! using one as an expression is a parse error.

use std::process::{Command, Output};

fn run_script(name: &str, src: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "quoin_placeholder_{}_{}.qn",
        name,
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    out
}

#[test]
fn todo_placeholder_throws_a_typed_error_and_fails_the_run() {
    let out = run_script("todo", "'before'.print;\n...;\n'after'.print;\n");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("before") && !stdout.contains("after"));
    assert!(
        stderr.contains("NotImplementedError") && stderr.contains("not implemented"),
        "expected the typed error in the report\n{stderr}"
    );
}

#[test]
fn unreachable_placeholder_throws_a_typed_error() {
    let out = run_script("bang", "!!!;\n");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("UnreachableError") && stderr.contains("reached unreachable code"),
        "{stderr}"
    );
}

#[test]
fn warn_placeholder_reports_its_location_and_continues() {
    let out = run_script("huh", "'a'.print;\n    ???;\n'b'.print;\n");
    assert_eq!(out.status.code(), Some(0), "??? must not fail the run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a") && stdout.contains("b"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // A standard log line: level label, then the ??? site (line 2, column 5,
    // 1-based) from Log's caller-location capture, then the message.
    assert!(
        stderr.contains("WARN") && stderr.contains(":2:5: reached `???` placeholder"),
        "expected the located warn log line\n{stderr}"
    );
}

#[test]
fn warn_placeholder_colors_like_a_checker_warning() {
    // The `???` warning is a `Log.warn:` — its default sink renders the level
    // label in the SAME yellow the checker's warnings use (bold, via the
    // markup engine), and decolorizes on non-color runs (covered by
    // `warn_placeholder_reports_its_location_and_continues`, which asserts the
    // plain form).
    let path =
        std::env::temp_dir().join(format!("quoin_placeholder_color_{}.qn", std::process::id()));
    std::fs::write(
        &path, "???;
",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .env("CLICOLOR_FORCE", "1")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("\u{1b}[1;38;2;255;204;0mWARN"),
        "expected the bold yellow WARN label\n{stderr:?}"
    );
}

#[test]
fn placeholders_are_statements_not_expressions() {
    for src in ["var x = ...;\n", "var x = ???;\n", "5 + !!!;\n"] {
        let out = run_script("expr", src);
        assert_eq!(out.status.code(), Some(1), "{src:?} must be rejected");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.to_lowercase().contains("parse") || stderr.contains("error"),
            "{src:?}: {stderr}"
        );
    }
}
