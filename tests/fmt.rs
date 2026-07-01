//! End-to-end tests for the `qn fmt` subcommand, driving the built binary.

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
fn fmt_writes_canonical_output_to_stdout() {
    let path = tmp_file("stdout.qn");
    std::fs::write(&path, "x.foo: a bar: b\ny=1").unwrap();
    let out = qn().arg("fmt").arg(&path).output().expect("run qn fmt");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // No space after `:`, explicit `;` between statements, trailing newline.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "x.foo:a bar:b;\ny=1\n"
    );
    // stdout mode never touches the file.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x.foo: a bar: b\ny=1"
    );
}

#[test]
fn fmt_check_exits_nonzero_and_lists_unformatted_files() {
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
fn fmt_write_rewrites_in_place_and_is_idempotent() {
    let path = tmp_file("write.qn");
    std::fs::write(&path, "x.foo: a bar: b").unwrap();
    let out = qn()
        .arg("fmt")
        .arg("--write")
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo:a bar:b\n");
    // Second write leaves it unchanged and reports nothing.
    let out2 = qn()
        .arg("fmt")
        .arg("--write")
        .arg(&path)
        .output()
        .expect("run");
    assert!(out2.status.success());
    assert!(String::from_utf8_lossy(&out2.stderr).is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x.foo:a bar:b\n");
}

#[test]
fn fmt_reports_a_parse_error_and_fails() {
    let path = tmp_file("bad_syntax.qn");
    std::fs::write(&path, "x = (((").unwrap();
    let out = qn().arg("fmt").arg(&path).output().expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad_syntax.qn"));
}

#[test]
fn fmt_without_paths_prints_usage() {
    let out = qn().arg("fmt").output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
}
