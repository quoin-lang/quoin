//! The corpus guardrail: run every real `.qn` file in the repo through the formatter
//! and assert the invariants that make a formatter trustworthy —
//!
//!   1. **Semantics preserved** — `parse(src) == parse(format(src))` (positions aside).
//!   2. **Comments preserved** — no comment is dropped, added, or edited.
//!   3. **Idempotent** — `format(format(src)) == format(src)`.
//!
//! Files that don't parse are skipped (and counted): the formatter is not a linter, and
//! the corpus may contain intentionally-broken fixtures.

use quoin_fmt::format_source;
use quoin_fmt::verify::{ast_equal, comments_preserved};
use std::path::{Path, PathBuf};

/// Repo root, two levels up from this crate (`crates/quoin-fmt`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn collect_qn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build/vendor dirs; the Quoin sources live under qnlib/ and crates/.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name == ".git" {
                continue;
            }
            collect_qn_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("qn") {
            out.push(path);
        }
    }
}

#[test]
fn formats_the_whole_corpus_without_changing_meaning() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_qn_files(&root.join("qnlib"), &mut files);
    files.sort();

    assert!(
        !files.is_empty(),
        "no .qn files found under {}",
        root.display()
    );

    let mut formatted = 0usize;
    let mut skipped_unparsable = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };

        let out = match format_source(&src, &rel) {
            Ok(out) => out,
            Err(_) => {
                skipped_unparsable += 1;
                continue;
            }
        };
        formatted += 1;

        if ast_equal(&src, &out) != Some(true) {
            failures.push(format!("{rel}: AST changed after formatting"));
            continue;
        }
        if !comments_preserved(&src, &out) {
            failures.push(format!("{rel}: comments not preserved"));
            continue;
        }
        match format_source(&out, &rel) {
            Ok(twice) if twice == out => {}
            Ok(_) => failures.push(format!("{rel}: not idempotent")),
            Err(e) => failures.push(format!("{rel}: reformatting failed: {e}")),
        }
    }

    eprintln!("corpus: {formatted} formatted, {skipped_unparsable} skipped (unparsable)");
    assert!(
        failures.is_empty(),
        "{} file(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        formatted > 50,
        "expected a substantial corpus, only formatted {formatted}"
    );
}
