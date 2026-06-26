//! Integration tests for the Tier-1 out-of-process extension transport.
//!
//! - `extension_transport_round_trip` (Slice 1): the `ext_echo` fixture round-trips scalar
//!   ops over the unix domain socket; the third case runs the call concurrently with an
//!   independent task to show the calling fiber parks on the socket (via the reactor) rather
//!   than blocking the VM.
//! - `extension_handle_round_trip` (Slice 3a/3b/4): the `ext_handles` fixture exercises the
//!   re-entrant host-op conversation and the handle table — the extension makes a host String
//!   mid-call, retains its handle and reads it back on a *later* call (proving the host keeps the
//!   value alive across calls), drives host objects via `call_method`, and runs a host block over
//!   a batch via `invoke_block`.
//! - `extension_crash_isolation` (Slice 5a): the `ext_crash` fixture exits mid-call; the host must
//!   surface a catchable error (not a hang), keep running, and fail fast on the next call.
//! - `extension_resource_handles` (Slice 5b): the `ext_resources` fixture returns an ext-side
//!   resource the host holds as an `ExtResource` token, passed back via `args:` across calls and
//!   reaped (freed extension-side) once the host drops it.
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

"* call_method (Slice 3b): the extension drives host objects via handles —
"* ('ab' +: '!') uppercased, the '+:' arg itself passed as a handle. -> 'AB!'
((e.call:'compute' with:'ab') == 'AB!').else:{{ ok = false }};

"* batched callback (Slice 4): the extension invokes a host block over a batch in one
"* round-trip. The block is now passed as a handle arg via args: (Slice 5b). -> 'A,B,C'
((e.call:'mapUpper' with:'' args:#( {{ |s| s.upper }} )) == 'A,B,C').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_handles_test.qn", &script);
}

#[test]
fn extension_crash_isolation() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';

"* a normal call works
((e.call:'ping' with:'') == 'pong').else:{{ ok = false }};

"* the extension exits mid-call: the host surfaces a catchable error (no hang), VM survives
crashed = {{ e.call:'crash' with:'' }}.catch:{{ |ex| 'caught' }};
(crashed == 'caught').else:{{ ok = false }};

"* the extension is now dead: a follow-up call fails fast, also catchable
again = {{ e.call:'ping' with:'' }}.catch:{{ |ex| 'dead' }};
(again == 'dead').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_crash_test.qn", &script);
}

#[test]
fn extension_resource_handles() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_resources");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';

"* create an ext-side counter; the host holds it as an opaque ExtResource token
c = e.call:'new' with:'';

"* pass the resource back into later calls via args: — it mutates the same ext-side counter
((e.call:'inc' with:'' args:#( c )) == '1').else:{{ ok = false }};
((e.call:'inc' with:'' args:#( c )) == '2').else:{{ ok = false }};
((e.call:'live' with:'') == '1').else:{{ ok = false }};

"* drop the only reference and churn allocations so GC reclaims the ExtResource (its Drop
"* queues the id); the release piggybacks on the next call, which frees it extension-side.
c = nil;
(1..5000).each:{{ |i| i.s }};
e.call:'live' with:'';
((e.call:'live' with:'') == '0').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_resources_test.qn", &script);
}
