//! The gzip write codec's close discipline at the PROCESS boundary, through
//! child qn runs — the paths the in-suite tests can't observe, because a
//! program cannot re-read a file whose finish happens at its own exit. A
//! stream the program never closes must still land on disk as a whole, valid
//! gzip member carrying the buffered bytes: the exit flush writes the buffer,
//! and backend teardown (`SmolInner::drop`) drives the encoder's `poll_close`
//! (the trailer). `Runtime.exit:` takes the same road.

use std::io::Read;
use std::process::Command;

/// Run `script` in a child qn; `tag` keeps concurrent tests' temp files apart
/// (under plain `cargo test` the tests are THREADS of one process — the
/// cli.rs lesson).
fn run_qn(tag: &str, script: &str) -> std::process::Output {
    let path = std::env::temp_dir().join(format!("quoin_gzw_{tag}_{}.qn", std::process::id()));
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    out
}

/// Decode a finished-on-exit .gz strictly; a missing trailer fails here.
fn read_gz(path: &std::path::Path) -> String {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut dec = flate2::read::GzDecoder::new(&raw[..]);
    let mut text = String::new();
    dec.read_to_string(&mut text)
        .expect("a whole, valid gzip member (trailer present)");
    let _ = std::fs::remove_file(path);
    text
}

#[test]
fn an_unclosed_gzip_stream_is_finished_at_exit() {
    let gz = std::env::temp_dir().join(format!("quoin_gzw_exit_{}.gz", std::process::id()));
    let script = format!(
        "var w = ([IO]File.create:'{p}').gzip\nw.writeAll:'finished at teardown'.asBytes\n",
        p = gz.display()
    );
    let out = run_qn("exit", &script);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read_gz(&gz), "finished at teardown");
}

#[test]
fn runtime_exit_still_finishes_the_stream() {
    // `Runtime.exit:` unwinds the driver rather than calling process exit, so
    // the flush + teardown finish must still happen on the way out.
    let gz = std::env::temp_dir().join(format!("quoin_gzw_rexit_{}.gz", std::process::id()));
    let script = format!(
        "var w = ([IO]File.create:'{p}').gzip\n\
         w.writeAll:'out through Runtime.exit'.asBytes\n\
         Runtime.exit:7\n",
        p = gz.display()
    );
    let out = run_qn("rexit", &script);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(read_gz(&gz), "out through Runtime.exit");
}
