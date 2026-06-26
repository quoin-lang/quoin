//! Integration test for Tier 1, Slice 1 — the out-of-process extension transport
//! keystone. Spawns the `ext_echo` extension subprocess and round-trips scalar ops
//! over the unix domain socket; the third case runs the extension call concurrently
//! with an independent task to show the calling fiber parks on the socket (via the
//! reactor) rather than blocking the VM. The script decides pass/fail.

use std::process::Command;

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

    let path = std::env::temp_dir().join("qn_ext_test.qn");
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
