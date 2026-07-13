//! `qn check --json` is the machine contract the language server consumes: one JSON
//! array on stdout — checker warnings, compile errors, and parse errors with their
//! spans — while the exit-code contract matches text mode (1 with findings, 0 clean).
//! These tests pin the shape so an LSP on the other end can't be silently broken.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Write `source` to a uniquely named temp file (pid + name — plain `cargo test`
/// runs tests as threads, so pid alone collides) and return its path.
fn fixture(name: &str, source: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("qn-check-json-{}-{name}.qn", std::process::id()));
    std::fs::write(&path, source).expect("write fixture");
    path
}

fn check_json(paths: &[&PathBuf]) -> (Output, serde_json::Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg("check").arg("--json");
    for p in paths {
        cmd.arg(p);
    }
    let out = cmd
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn check --json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{stdout}"));
    (out, json)
}

#[test]
fn warnings_and_parse_errors_come_back_structured() {
    let warn = fixture("warn", "var n: Integer = 'not a number'\n");
    let bad = fixture("bad", "var x = (((\n");
    let (out, json) = check_json(&[&warn, &bad]);

    assert_eq!(out.status.code(), Some(1), "findings exit 1");
    let diags = json.as_array().expect("a JSON array");
    assert_eq!(diags.len(), 2, "{json:#?}");

    // Parse errors are `error`-severity with `parse-error` kind and a real span.
    let parse = diags
        .iter()
        .find(|d| d["kind"] == "parse-error")
        .unwrap_or_else(|| panic!("no parse-error entry: {json:#?}"));
    assert_eq!(parse["severity"], "error");
    assert_eq!(parse["file"], bad.display().to_string());
    assert!(
        parse["line"].is_u64() && parse["start"].is_u64(),
        "{parse:#?}"
    );

    // Checker warnings carry their WARNING_KINDS slug and 1-based line / 0-based column.
    let warn_diag = diags
        .iter()
        .find(|d| d["kind"] == "type-mismatch")
        .unwrap_or_else(|| panic!("no type-mismatch entry: {json:#?}"));
    assert_eq!(warn_diag["severity"], "warning");
    assert_eq!(warn_diag["file"], warn.display().to_string());
    assert_eq!(warn_diag["line"], 1);
    assert_eq!(
        warn_diag["column"], 17,
        "column is 0-based (SourceInfo/LSP)"
    );
    assert_eq!(warn_diag["start"], 17);
    assert_eq!(warn_diag["end"], 31);
    assert!(
        warn_diag["message"]
            .as_str()
            .unwrap()
            .contains("type mismatch"),
        "{warn_diag:#?}"
    );
}

#[test]
fn clean_file_emits_empty_array_and_exit_zero() {
    let clean = fixture("clean", "var x = 1\nx.print\n");
    let (out, json) = check_json(&[&clean]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(json, serde_json::json!([]), "clean run is an empty array");
}

#[test]
fn text_mode_is_unchanged_by_the_flag() {
    let warn = fixture("text", "var n: Integer = 'not a number'\n");
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["check"])
        .arg(&warn)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr.contains(": warning:") && stderr.contains("^"),
        "human rendering with caret intact:\n{stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "text mode writes no JSON to stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
