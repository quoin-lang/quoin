//! Integration tests for the Tier-1 out-of-process extension transport.
//!
//! - `extension_transport_round_trip` (Slice 1): the `ext_echo` fixture round-trips scalar
//!   ops over the unix domain socket; the third case runs the call concurrently with an
//!   independent task to show the calling fiber parks on the socket (via the reactor) rather
//!   than blocking the VM.
//! - `extension_handle_round_trip` (Slice 3a): the `ext_handles` fixture exercises the
//!   re-entrant host-op conversation and the handle table — the extension makes a host String
//!   mid-call, retains its handle, and reads it back on a *later* call, proving the host keeps
//!   the value alive (rooted by the handle) across calls.
//!
//! Each script decides pass/fail and prints PASS/FAIL.

use std::process::Command;

/// Run a `.qn` script through the `qn` binary and assert it printed `PASS`.
fn assert_script_passes(name: &str, script: &str) {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS"),
        "extension script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn extension_transport_round_trip() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_echo");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';

"* basic scalar round-trips
((e.call:'echo' with:'hi') == 'hi').else:{{ ok = false }};
((e.call:'upper' with:'hello') == 'HELLO').else:{{ ok = false }};

"* the call parks on the socket: it runs concurrently with an independent task,
"* and gather still returns both results in order.
results = Async.gather:#( {{ e.call:'echo' with:'world' }} {{ 1 + 1 }} );
(results == #( 'world' 2 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_echo_test.qn", &script);
}

#[test]
fn extension_handle_round_trip() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_handles");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';

"* call 1: the extension makes a host String, retains its handle, and remembers it
((e.call:'stash' with:'kept-value') == 'ok').else:{{ ok = false }};

"* churn host allocations between the calls while the handle is retained
(1..2000).each:{{ |i| i.s }};

"* call 2: the extension reads its retained handle back -> proves the host rooted the value
((e.call:'fetch' with:'') == 'kept-value').else:{{ ok = false }};

"* release, then a fresh stash/fetch still works (the freed slot is reusable)
e.call:'release' with:'';
((e.call:'stash' with:'second') == 'ok').else:{{ ok = false }};
((e.call:'fetch' with:'') == 'second').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_handles_test.qn", &script);
}
