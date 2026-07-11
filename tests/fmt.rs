//! End-to-end tests for the `qn fmt` subcommand, driving the built binary.
//!
//! Formatting is in place by default; `--dry-run` prints to stdout without writing, and
//! `--check` reports unformatted files without writing.

use std::process::Command;

fn qn() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qn"))
}

/// A unique temp path under the OS temp dir (no external temp-file crate).
fn tmp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("qn-fmt-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn fmt_rewrites_in_place_by_default() {
    let path = tmp_file("default.qn");
    std::fs::write(&path, "x.foo: a bar: b\ny=1").unwrap();
    let out = qn().arg("fmt").arg(&path).output().expect("run qn fmt");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // No space after `:`, no `;` at an unambiguous line break, trailing newline — written to the file.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x.foo:a bar:b\ny=1\n"
    );
    // The formatted file is on stderr as a changed file, and nothing goes to stdout.
    assert!(String::from_utf8_lossy(&out.stderr).contains("default.qn"));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}

#[test]
fn fmt_is_idempotent_and_reports_no_change_the_second_time() {
    let path = tmp_file("idem.qn");
    std::fs::write(&path, "x.foo: a bar: b").unwrap();
    qn().arg("fmt").arg(&path).output().expect("run");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo:a bar:b\n");
    // Second run leaves it unchanged and reports nothing (no "formatted" line).
    let out2 = qn().arg("fmt").arg(&path).output().expect("run");
    assert!(out2.status.success());
    assert!(String::from_utf8_lossy(&out2.stderr).is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo:a bar:b\n");
}

#[test]
fn fmt_dry_run_writes_to_stdout_without_touching_the_file() {
    let path = tmp_file("dry.qn");
    std::fs::write(&path, "x.foo: a bar: b\ny=1").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--dry-run")
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "x.foo:a bar:b\ny=1\n");
    // The file on disk is untouched.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x.foo: a bar: b\ny=1"
    );
}

#[test]
fn fmt_check_exits_nonzero_and_lists_unformatted_files_without_writing() {
    let path = tmp_file("check_bad.qn");
    std::fs::write(&path, "x.foo: a").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("check_bad.qn"));
    // --check never writes.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo: a");
}

#[test]
fn fmt_check_exits_zero_for_already_formatted() {
    let path = tmp_file("check_ok.qn");
    std::fs::write(&path, "x.foo:a\n").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}

#[test]
fn fmt_reports_a_parse_error_and_fails() {
    let path = tmp_file("bad_syntax.qn");
    std::fs::write(&path, "x = (((").unwrap();
    let out = qn().arg("fmt").arg(&path).output().expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad_syntax.qn"));
    // A file that doesn't parse is left untouched.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x = (((");
}

#[test]
fn fmt_check_and_dry_run_conflict() {
    let out = qn()
        .arg("fmt")
        .arg("--check")
        .arg("--dry-run")
        .arg("x.qn")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("mutually exclusive"));
}

#[test]
fn fmt_diff_shows_a_unified_diff_without_writing() {
    let path = tmp_file("diff.qn");
    std::fs::write(&path, "x.foo: a bar: b").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--diff")
        .arg(&path)
        .output()
        .expect("run");
    // Differs, so exit is non-zero and a unified diff is printed.
    assert_eq!(out.status.code(), Some(1));
    let diff = String::from_utf8_lossy(&out.stdout);
    assert!(diff.contains("@@"), "no hunk header:\n{diff}");
    assert!(
        diff.contains("-x.foo: a bar: b"),
        "no removed line:\n{diff}"
    );
    assert!(diff.contains("+x.foo:a"), "no added line:\n{diff}");
    assert!(
        diff.contains("(formatted)"),
        "temp path not relabeled:\n{diff}"
    );
    // The original file is never touched.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo: a bar: b");
}

#[test]
fn fmt_diff_on_already_formatted_file_is_silent_and_succeeds() {
    let path = tmp_file("diff_ok.qn");
    std::fs::write(&path, "x.foo:a\n").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--diff")
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}

#[test]
fn fmt_without_paths_prints_usage() {
    let out = qn().arg("fmt").output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
}

#[test]
fn fmt_stdin_formats_to_stdout() {
    use std::io::Write;
    let mut child = qn()
        .arg("fmt")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn qn fmt -");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"x = 1 y = 2\nfoo -> {\n@bar\n}\n")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success());
    // The unsaved buffer is formatted and written to stdout; nothing touches disk.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "x = 1\ny = 2\nfoo -> { @bar }\n"
    );
}

#[test]
fn fmt_stdin_reports_parse_error_and_emits_no_stdout() {
    use std::io::Write;
    let mut child = qn()
        .arg("fmt")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn qn fmt -");
    child.stdin.take().unwrap().write_all(b"x = (((\n").unwrap();
    let out = child.wait_with_output().expect("wait");
    // Non-zero exit, error on stderr, and nothing on stdout — so a caller keeps the buffer as-is.
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("parse error"));
    assert!(String::from_utf8_lossy(&out.stdout).is_empty());
}
