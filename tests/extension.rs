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
//! - `extension_timeout`: a hung `ext_crash` call times out via `Async.timeout:do:` (catchable);
//!   the now-desynced extension is marked dead so the next call fails fast instead of blocking.
//! - `extension_resource_handles` (Slice 5b): the `ext_resources` fixture returns an ext-side
//!   resource the host holds as an `ExtResource` token, passed back via `args:` across calls and
//!   reaped (freed extension-side) once the host drops it.
//! - `extension_array_data_plane` (Slice 6b): the `ext_arrays` fixture receives a bulk `Array` as a
//!   call argument (copy-through), operates on the whole buffer, and returns a scalar or a new
//!   `Array` — proving columnar data crosses the boundary without per-element exploding.
//! - `extension_structured_values` (Phase 1): the `ext_data` fixture round-trips a structured Quoin
//!   value through `call:with:data:` and returns a structured value built extension-side.
//! - `extension_backed_classes` (Phase 3): the `ext_vector` fixture *provides* the classes `Vector`
//!   and `Matrix` — the host installs them as globals from the spawn-time manifest, and method sends
//!   (`Vector ofFloats:` / `v sum` / `v scale:`) dispatch over the socket as ordinary sends. Also
//!   covers cross-class returns (`Matrix row:` -> `Vector`) and richer args (`v dot:` an ext-instance,
//!   `v map:` a host block).
//! - `extension_backed_classes_python` (Phase 3b): the same, but the `Vector`-providing extension is
//!   a *Python* process (`ext_vector.py`) — proving the manifest + class-dispatch protocol is
//!   polyglot. Gated on `python3` + `msgpack`.
//! - `extension_python_sdk` (Slice 7): the extension is a *Python* process (`sdk/python`) speaking
//!   the same MessagePack wire protocol (`quoin-ext-proto/PROTOCOL.md`) — the polyglot proof.
//!   Gated on `python3` + `msgpack`.
//! - `extension_structured_value_fidelity`: structured round-trips through the Python SDK,
//!   including the two ext-typed kinds the wire must preserve exactly (BigInteger = ext 1,
//!   decimal = ext 2), nesting, and bytes.
//! - `extension_protocol_version_mismatch`: an extension whose `ManifestReturn` names a protocol
//!   version this host doesn't speak is refused at the handshake with a catchable error naming
//!   both versions (not garbage decoding, not a hang).
//!
//! Each script decides pass/fail and prints PASS/FAIL.

use std::process::Command;

/// Run a `.qn` script through the `qn` binary once, returning whether it printed `PASS` plus a
/// diagnostic string (exit status + captured stdout/stderr).
fn run_script_once(name: &str, script: &str) -> (bool, String) {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let passed = stdout.contains("PASS");
    let diag = format!(
        "status: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    (passed, diag)
}

/// Run a `.qn` script through the `qn` binary and assert it printed `PASS`.
///
/// Retries a few times before failing: these tests spawn a `qn` subprocess that itself spawns an
/// extension subprocess, and under the full `cargo test` suite's aggregate process/memory load the
/// `qn` child can occasionally be killed before it runs (the captured symptom is empty stdout *and*
/// stderr — i.e. not a Quoin error, which prints to stderr, but a transient subprocess kill). A
/// genuine logic bug fails every attempt deterministically and is still caught; only a transient is
/// masked. Retries are spaced slightly so transient pressure can subside.
fn assert_script_passes(name: &str, script: &str) {
    const ATTEMPTS: u32 = 4;
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let (passed, diag) = run_script_once(name, script);
        if passed {
            return;
        }
        last_diag = diag;
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    panic!("extension script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// Drive `qn repl` with `lines` piped to stdin (one REPL input each), returning stdout+stderr.
/// Each REPL line runs in its own driver pass, so this exercises whether a long-lived resource
/// survives across evaluations.
fn run_repl_lines(lines: &[String]) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("repl")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn repl");
    {
        let mut stdin = child.stdin.take().expect("repl stdin");
        for line in lines {
            writeln!(stdin, "{line}").expect("write repl line");
        }
        // Dropping stdin closes it -> EOF, so the piped REPL finishes and exits.
    }
    let out = child.wait_with_output().expect("wait qn repl");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn extension_survives_across_repl_lines() {
    // Regression: spawning an extension on one REPL line and calling it on the next used to fail
    // ("unknown stream id" / "Extension process died") because the REPL recreated its I/O backend
    // per line, closing the extension's socket. The backend now persists for the session.
    let ext_bin = env!("CARGO_BIN_EXE_ext_echo");
    const ATTEMPTS: u32 = 4;
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        let out = run_repl_lines(&[
            format!("var e = Extension.spawn:'{ext_bin}'"),
            "(e.call:'echo' with:'hi') == 'hi'".to_string(),
        ]);
        // The call on the *second* line must reach the extension spawned on the first.
        if out.contains("=> true") {
            assert!(
                !out.contains("unknown stream") && !out.contains("process died"),
                "extension errored across REPL lines.\n{out}"
            );
            return;
        }
        last = out;
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    panic!("extension call across REPL lines did not succeed after {ATTEMPTS} attempts.\n{last}");
}

#[test]
fn extension_transport_round_trip() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_echo");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* basic scalar round-trips
((e.call:'echo' with:'hi') == 'hi').else:{{ ok = false }};
((e.call:'upper' with:'hello') == 'HELLO').else:{{ ok = false }};

"* the call parks on the socket: it runs concurrently with an independent task,
"* and gather still returns both results in order.
var results = Async.gather:#( {{ e.call:'echo' with:'world' }} {{ 1 + 1 }} );
(results == #( 'world' 2 )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_echo_test.qn", &script);
}

#[test]
fn extension_concurrent_calls_are_serialized() {
    // Regression (audit): the transport is a single request/response socket with no
    // request ids, so two tasks calling one extension at once interleaved frames and
    // desynced the connection ("unknown stream id", or a killed serve loop). A second
    // top-level call while one is in flight must now fail with a catchable "busy"
    // error — deterministic, and the connection survives for the winner and for later
    // sequential calls.
    let ext_bin = env!("CARGO_BIN_EXE_ext_echo");
    let script = format!(
        r#"
var ok = true;
var e = Extension.spawn:'{ext_bin}';

"* Two calls raced on one connection: exactly one wins, the other gets a catchable
"* busy error — never a desync/corruption.
var r = Async.gather:#(
    {{ {{ e.call:'echo' with:'A' }}.catch:{{ |ex| 'busy' }} }}
    {{ {{ e.call:'echo' with:'B' }}.catch:{{ |ex| 'busy' }} }}
);
var a = r.at:0;
var b = r.at:1;
var oneWon = ((a == 'A') && (b == 'busy')) || ((a == 'busy') && (b == 'B'));
oneWon.else:{{ ok = false; ('FAIL race: ' + r.s).print }};

"* The connection is intact: sequential calls still work after the rejected one.
((e.call:'echo' with:'again') == 'again').else:{{ ok = false; 'FAIL: connection desynced'.print }};
((e.call:'upper' with:'z') == 'Z').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_serialize_test.qn", &script);
}

#[test]
fn extension_handle_round_trip() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_handles");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

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
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* a normal call works
((e.call:'ping' with:'') == 'pong').else:{{ ok = false }};

"* the extension exits mid-call: the host surfaces a catchable error (no hang), VM survives
var crashed = {{ e.call:'crash' with:'' }}.catch:{{ |ex| 'caught' }};
(crashed == 'caught').else:{{ ok = false }};

"* the extension is now dead: a follow-up call fails fast, also catchable
var again = {{ e.call:'ping' with:'' }}.catch:{{ |ex| 'dead' }};
(again == 'dead').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_crash_test.qn", &script);
}

#[test]
fn extension_silent_handshake_times_out() {
    // Regression (audit): the spawn-time GetManifest read was unbounded, so an
    // extension that binds+accepts the socket but never answers the handshake parked
    // the spawning task forever. `ext_silent` does exactly that; with a short
    // handshake budget the spawn must fail catchably (not hang), and no orphan child
    // survives (the failed spawn kills it).
    let ext_bin = env!("CARGO_BIN_EXE_ext_silent");
    // The core property: a silent extension makes spawn FAIL catchably and promptly — it
    // must not hang the VM (a hang would make this test time out). The common failure is
    // the handshake read timing out (#timedOut); under heavy parallel load the connect
    // retry can lose to a slow-to-bind child and fail connect-side instead, which is
    // equally "fast, catchable, no hang" — so assert the no-hang/catchable property, not
    // the exact kind.
    let script = format!(
        r#"
{{ Extension.spawn:'{ext_bin}'; 'FAIL: silent extension spawned'.print }}
    .catch:{{ |e:IoError| 'PASS'.print }}
    catch:{{ |e| ('FAIL class: ' + e.s).print }};
"#
    );
    let path = std::env::temp_dir().join("qn_ext_silent_test.qn");
    std::fs::write(&path, &script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .env("QN_EXT_HANDSHAKE_TIMEOUT_MS", "1500")
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "silent-handshake test did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn extension_timeout() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* a hung call is bounded by Async.timeout:do: — raises a catchable TimeoutError, VM survives
var r = {{ Async.timeout:300 do:{{ e.call:'hang' with:'' }} }}.catch:{{ |ex| 'timedout' }};
(r == 'timedout').else:{{ ok = false }};

"* the conversation is desynced -> the extension is dead; a follow-up fails fast (does NOT hang
"* waiting on a child still stuck in `hang`)
var again = {{ e.call:'ping' with:'' }}.catch:{{ |ex| 'dead' }};
(again == 'dead').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_timeout_test.qn", &script);
}

#[test]
fn extension_resource_handles() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_resources");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* create an ext-side counter; the host holds it as an opaque ExtResource token
var c = e.call:'new' with:'';

"* pass the resource back into later calls via args: — it mutates the same ext-side counter
((e.call:'inc' with:'' args:#( c )) == '1').else:{{ ok = false }};
((e.call:'inc' with:'' args:#( c )) == '2').else:{{ ok = false }};
((e.call:'live' with:'') == '1').else:{{ ok = false }};

"* drop the only reference and churn allocations so GC reclaims the ExtResource (its Drop
"* queues the id); the release piggybacks on the next call, which frees it extension-side.
"* The churn allocates a list + strings per element so it forces collection debt in ANY
"* execution tier (a compiled block body allocates no per-element frame/env — B3a).
c = nil;
(1..5000).each:{{ |i| #( i.s i.s ) }};
e.call:'live' with:'';
((e.call:'live' with:'') == '0').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_resources_test.qn", &script);
}

#[test]
fn extension_array_data_plane() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_arrays");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';
var a = Array.ofFloats:#( 1.0 2.0 3.0 );

"* the bulk column crosses the socket; the extension sums the whole buffer -> '6'
((e.call:'sum' with:'' args:#( a )) == '6').else:{{ ok = false }};

"* scale: returns a new Array (the column round-trips back as an Array, not a List)
var r = e.call:'scale' with:'2' args:#( a );
(r.dtype == #float64).else:{{ ok = false }};
(r.toList == #( 2.0 4.0 6.0 )).else:{{ ok = false }};
(r.sum == 12.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_arrays_test.qn", &script);
}

#[test]
fn extension_structured_values() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_data");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* a structured value round-trips: Quoin Map -> DataValue -> (ext) -> DataValue -> Quoin Map
var m = #{{ 'n': 42 'f': 1.5 's': 'hi' 'flag': true 'items': #( 1 2 3 ) }};
((e.call:'echoData' with:'' data:m) == m).else:{{ ok = false }};

"* a structured value built extension-side materializes as a real Quoin List
((e.call:'mkList' with:'') == #( 1 2 3 )).else:{{ ok = false }};

"* host reach (Phase 2): the ext reaches the Array class, builds it, and returns it live
var arr = e.call:'buildArray' with:'';
(arr.dtype == #float64).else:{{ ok = false }};
(arr.sum == 6.0).else:{{ ok = false }};

"* host reach: the ext reads a passed value back as structured data (read_handle)
((e.call:'inspect' with:'' args:#( #{{ 'k': 7 }} )) == #{{ 'k': 7 }}).else:{{ ok = false }};

"* a pathologically deep structured value is rejected catchably — the host must not
"* overflow its stack decoding it (crash isolation is the whole point of Tier 1)
{{ e.call:'deepData' with:''; ok = false; 'FAIL: deep value accepted'.print }}
    .catch:{{ |err| nil }};

"* and the extension is still alive and usable after the rejected call
((e.call:'mkList' with:'') == #( 1 2 3 )).else:{{ ok = false; 'FAIL: ext dead after deep'.print }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_data_test.qn", &script);
}

#[test]
fn extension_backed_classes() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_vector");
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{ext_bin}';

"* an extension-backed class (Phase 3): the class-side constructor builds a live instance, and an
"* ordinary method send dispatches over the socket — `Vector` is a real global, `v` a real instance
var v = Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{{ ok = false }};
(v.length == 3).else:{{ ok = false }};

"* an instance method that *makes* a new instance returns another (socket-backed) `Vector`
var w = v.scale:2.0;
(w.sum == 12.0).else:{{ ok = false }};
(w.length == 3).else:{{ ok = false }};

"* the receiver is unchanged — distinct instances, each its own ext-side object
(v.sum == 6.0).else:{{ ok = false }};

"* cross-class return: a `Matrix` method returns a `Vector` instance, wrapped as the `Vector` class
"* (so it responds to Vector's methods) — a method may return an instance of any of the ext's classes
var m = Matrix.ofRows:#( #( 1.0 2.0 ) #( 3.0 4.0 ) );
(m.rowCount == 2).else:{{ ok = false }};
var r0 = m.row:0;
(r0.sum == 3.0).else:{{ ok = false }};
(r0.length == 2).else:{{ ok = false }};
((m.row:1).sum == 7.0).else:{{ ok = false }};

"* an ext-instance argument: `dot:` takes another Vector (resolved to a live instance) -> a scalar
var va = Vector.ofFloats:#( 1.0 2.0 3.0 );
var vb = Vector.ofFloats:#( 4.0 5.0 6.0 );
((va.dot:vb) == 32.0).else:{{ ok = false }};

"* a host-block argument: `map:` applies the passed block to each element -> a new Vector
var mapped = va.map:{{ |x| x * 10.0 }};
(mapped.sum == 60.0).else:{{ ok = false }};

"* a bulk `Array` argument: the whole column crosses the data plane into a constructor
var fromCol = Vector.ofArray:(Array.ofFloats:#( 2.0 4.0 6.0 ));
(fromCol.sum == 12.0).else:{{ ok = false }};
(fromCol.length == 3).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_vector_test.qn", &script);
}

/// A *recoverable* error from an extension-backed class method: the SDK sends `CallReturnError`, the
/// host raises a *catchable* Quoin error, and — unlike a crash (see `extension_crash_isolation`) —
/// the extension stays alive and the same instance keeps answering.
#[test]
fn extension_class_error_is_catchable() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_vector");
    let script = format!(
        r#"
var ok = true;
var e = Extension.spawn:'{ext_bin}';
var v = Vector.ofFloats:#( 1.0 2.0 3.0 );

"* a valid index returns normally
((v.at:0) == 1.0).else:{{ ok = false }};

"* an out-of-range index raises a CATCHABLE error carrying the handler's message
var caught = {{ v.at:9 }}.catch:{{ |ex| ex.message }};
(caught == 'index 9 out of range (length 3)').else:{{ ok = false }};

"* ...and the extension SURVIVED — the same instance still answers the next sends
((v.at:1) == 2.0).else:{{ ok = false }};
(v.sum == 6.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_class_error_test.qn", &script);
}

/// True if `python3` can import `msgpack` — the Python SDK's only external
/// dependency. When false, the polyglot tests skip cleanly (e.g. CI without Python set up).
fn python_fixture_runnable() -> bool {
    Command::new("python3")
        .args(["-c", "import msgpack"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensure a Python fixture is executable, so `Extension.spawn:` can exec it via its shebang
/// regardless of the checkout's recorded file mode.
fn ensure_executable(path: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o111);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

#[test]
fn extension_python_sdk() {
    if !python_fixture_runnable() {
        eprintln!("skipping extension_python_sdk: python3 with `msgpack` unavailable");
        return;
    }
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sdk/python/examples/ext_echo.py"
    );
    ensure_executable(fixture);

    // A Python extension, spoken to over the exact same protocol as the Rust fixtures.
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{fixture}';
((e.call:'echo' with:'hi') == 'hi').else:{{ ok = false }};
((e.call:'upper' with:'hello') == 'HELLO').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_python_test.qn", &script);
}

#[test]
fn extension_python_parity() {
    if !python_fixture_runnable() {
        eprintln!("skipping extension_python_parity: python3 with `msgpack` unavailable");
        return;
    }
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sdk/python/examples/ext_full.py"
    );
    ensure_executable(fixture);

    // The Python extension exercises the *full* host surface — the same ops/assertions the Rust
    // fixtures get: host-ops (compute), batched callbacks (mapUpper), ext-resources (new/inc/live
    // + reap), and the Array data plane (sum/scale).
    let script = format!(
        r#"
var ok = true;
var e = Extension.spawn:'{fixture}';

"* host-ops: make_string + call_method ('ab' +: '!').upper -> 'AB!'
((e.call:'compute' with:'ab') == 'AB!').else:{{ ok = false }};

"* batched callback: invoke_block runs the passed block over a,b,c -> 'A,B,C'
((e.call:'mapUpper' with:'' args:#( {{ |s| s.upper }} )) == 'A,B,C').else:{{ ok = false }};

"* ext-resource handles: create a counter, mutate it across calls, then drop + reap
var c = e.call:'new' with:'';
((e.call:'inc' with:'' args:#( c )) == '1').else:{{ ok = false }};
((e.call:'inc' with:'' args:#( c )) == '2').else:{{ ok = false }};
((e.call:'live' with:'') == '1').else:{{ ok = false }};
c = nil;
(1..5000).each:{{ |i| #( i.s i.s ) }};
e.call:'live' with:'';
((e.call:'live' with:'') == '0').else:{{ ok = false }};

"* Array data plane: sum the whole buffer, and scale -> a new Array
var a = Array.ofFloats:#( 1.0 2.0 3.0 );
((e.call:'sum' with:'' args:#( a )) == '6.0').else:{{ ok = false }};
var r = e.call:'scale' with:'2' args:#( a );
(r.toList == #( 2.0 4.0 6.0 )).else:{{ ok = false }};

"* structured values (Phase 1): a Map round-trips, and a record built in Python -> a Quoin Map
var m = #{{ 'a': 1 'b': #( 'x' 'y' ) 'c': true }};
((e.call:'echoData' with:'' data:m) == m).else:{{ ok = false }};
((e.call:'mkRecord' with:'') == #{{ 'name': 'quoin' 'items': #( 1 2 3 ) 'ok': true }}).else:{{ ok = false }};

"* host reach (Phase 2): Python reaches the Array class, builds it, returns it live; and inspects
var arr = e.call:'buildArray' with:'';
(arr.dtype == #float64).else:{{ ok = false }};
(arr.sum == 6.0).else:{{ ok = false }};
((e.call:'inspect' with:'' args:#( #{{ 'k': 7 }} )) == #{{ 'k': 7 }}).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_python_parity_test.qn", &script);
}

#[test]
fn extension_backed_classes_python() {
    if !python_fixture_runnable() {
        eprintln!("skipping extension_backed_classes_python: python3 with `msgpack` unavailable");
        return;
    }
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sdk/python/examples/ext_vector.py"
    );
    ensure_executable(fixture);

    // The Python parity of `extension_backed_classes`: a *Python* process provides the Quoin class
    // `Vector` over the identical manifest + dispatch protocol — the Rust and Python extensions are
    // interchangeable to the host.
    let script = format!(
        r#"
var ok = true;

var e = Extension.spawn:'{fixture}';

"* a Python extension provides `Vector`: constructor + method send dispatch over the socket
var v = Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{{ ok = false }};
(v.length == 3).else:{{ ok = false }};

"* a method returning a new instance is auto-detected Python-side (isinstance — no `makes`)
var w = v.scale:2.0;
(w.sum == 12.0).else:{{ ok = false }};
(v.sum == 6.0).else:{{ ok = false }};

"* cross-class return: a Python `Matrix` method returns a `Vector`, wrapped as the `Vector` class
var m = Matrix.ofRows:#( #( 1.0 2.0 ) #( 3.0 4.0 ) );
(m.rowCount == 2).else:{{ ok = false }};
var r0 = m.row:0;
(r0.sum == 3.0).else:{{ ok = false }};
((m.row:1).sum == 7.0).else:{{ ok = false }};

"* richer args: an ext-instance argument (`dot:`) and a host-block argument (`map:`)
var va = Vector.ofFloats:#( 1.0 2.0 3.0 );
var vb = Vector.ofFloats:#( 4.0 5.0 6.0 );
((va.dot:vb) == 32.0).else:{{ ok = false }};
var mapped = va.map:{{ |x| x * 10.0 }};
(mapped.sum == 60.0).else:{{ ok = false }};

"* a Python handler that raises -> a CATCHABLE Quoin error (the SDK's CallReturnError), and the
"* extension stays alive: the same instance still answers the next send
var caught = {{ va.at:9 }}.catch:{{ |ex| ex.message }};
(caught == 'index 9 out of range (length 3)').else:{{ ok = false }};
((va.at:1) == 2.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_vector_python_test.qn", &script);
}

/// Slice 1 of extension packaging (`docs/EXT_PACKAGING.md`): `Extension loadPackage:` loads a
/// *folder* — an `extension.toml` (launch spec + namespace) plus an optional `init.qn` of Quoin
/// glue. The `ext_vector` fixture is packaged here with namespace `Vec` and an `init.qn` that
/// reopens the installed class to add a convenience method. Proves: classes install **namespaced**
/// (`[Vec]Vector` — the binary only declares a simple `Vector`, never a bare global), `init.qn` runs
/// after install and its Quoin method composes a socket-backed primitive (`scale:` then `sum`), and
/// a repeat `loadPackage:` of the same folder is idempotent (no re-spawn, classes still work).
#[test]
fn extension_load_package() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_vector");
    let pkg_dir = std::env::temp_dir().join(format!("qn_ext_pkg_{}", std::process::id()));
    std::fs::create_dir_all(&pkg_dir).expect("create package dir");
    std::fs::write(
        pkg_dir.join("extension.toml"),
        format!(
            "[package]\nname = \"vectors\"\n\n[extension]\ncommand = \"{ext_bin}\"\nnamespace = \"Vec\"\n"
        ),
    )
    .expect("write extension.toml");
    // init.qn reopens the (namespaced) class to add a Quoin method composing a socket primitive:
    // `tripledSum` scales the vector by 3 (a socket `scale:` -> new instance) and sums it.
    std::fs::write(
        pkg_dir.join("init.qn"),
        "[Vec]Vector <-- {\n    tripledSum -> { (self.scale:3.0).sum }\n}\n",
    )
    .expect("write init.qn");

    let dir = pkg_dir.to_string_lossy().to_string();
    let script = format!(
        r#"
var ok = true;

"* load the package folder: spawns the extension, installs its classes under the [Vec] namespace,
"* then runs init.qn
Extension.loadPackage:'{dir}';

"* the class is reachable *namespaced* — the binary declares a simple `Vector`, installed as
"* `[Vec]Vector` (so a package can never register a bare global)
var v = [Vec]Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{{ ok = false }};

"* init.qn's Quoin method ran after install and composes a socket-backed primitive (scale: then sum)
(v.tripledSum == 18.0).else:{{ ok = false }};

"* idempotent: re-loading the same folder returns the cached extension (no re-spawn); classes work
Extension.loadPackage:'{dir}';
(([Vec]Vector.ofFloats:#( 2.0 2.0 )).tripledSum == 12.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_load_package_test.qn", &script);

    let _ = std::fs::remove_dir_all(&pkg_dir);
}

/// Slice 2 of extension packaging (`docs/EXT_PACKAGING.md` §5): `use <pkg>:*` resolves a named
/// extension package on the search path to synthesized `Extension.loadPackage:` glue. The
/// `ext_vector` fixture is dropped as `vectors/` under a temp `$QUOIN_PATH` root; `use vectors:*`
/// then spawns it, installs `[Vec]Vector`, and runs the package's init.qn — all driven by the `use`
/// statement (the whole-package `*` glob is the grammar addition this slice makes).
#[test]
fn extension_use_package() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_vector");
    let root = std::env::temp_dir().join(format!("qn_usepkg_{}", std::process::id()));
    let pkg = root.join("vectors");
    std::fs::create_dir_all(&pkg).expect("create package dir");
    std::fs::write(
        pkg.join("extension.toml"),
        format!(
            "[package]\nname = \"vectors\"\n\n[extension]\ncommand = \"{ext_bin}\"\nnamespace = \"Vec\"\n"
        ),
    )
    .expect("write extension.toml");
    std::fs::write(
        pkg.join("init.qn"),
        "[Vec]Vector <-- {\n    tripledSum -> { (self.scale:3.0).sum }\n}\n",
    )
    .expect("write init.qn");

    let script = r#"
var ok = true;

"* `use vectors:*` finds vectors/ on $QUOIN_PATH, synthesizes Extension.loadPackage:, spawns +
"* installs [Vec]Vector (namespaced), and runs init.qn — all via the use statement
use vectors:*;

var v = [Vec]Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{ ok = false };
(v.tripledSum == 18.0).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let script_path = root.join("main.qn");
    std::fs::write(&script_path, script).expect("write script");

    // Retry a few times like `assert_script_passes` (transient subprocess pressure under the full
    // suite); the package dir is on `$QUOIN_PATH` for the `qn` child, found from the repo-root CWD.
    let mut last = String::new();
    for attempt in 1..=4u32 {
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&script_path)
            .env("QUOIN_PATH", &root)
            .output()
            .expect("run qn");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        last = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    panic!("use-package script did not pass after 4 attempts.\n{last}");
}

/// Structured round-trips through the Python `ext_full` fixture: nesting, bytes, and the two
/// ext-typed kinds whose fidelity the wire contract must preserve exactly — BigInteger
/// (MessagePack ext 1) and a decimal (ext 2).
#[test]
fn extension_structured_value_fidelity() {
    if !python_fixture_runnable() {
        eprintln!("skipping extension_structured_value_fidelity: python3 + `msgpack` unavailable");
        return;
    }
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sdk/python/examples/ext_full.py"
    );
    ensure_executable(fixture);
    let script = format!(
        r#"
var ok = true;
var e = Extension.spawn:'{fixture}';

"* nested structure round-trips unchanged
var m = #{{ 'xs': #( 1 2.5 'three' true nil ) 'blob': (Bytes.of:#( 1 2 255 )) }};
((e.call:'echoData' with:'' data:m) == m).else:{{ ok = false }};

"* BigInteger fidelity (must come back a BigInteger, not a string or truncated int)
var big = BigInteger.of:'123456789012345678901234567890';
var backBig = e.call:'echoData' with:'' data:big;
(backBig == big).else:{{ ok = false }};
((backBig + 1.asBigInteger) == (BigInteger.of:'123456789012345678901234567891'))
    .else:{{ ok = false }};

"* decimal fidelity (a BigDecimal beyond f64 precision)
var dec = JSON.parse:'0.12345678901234567890123';
((e.call:'echoData' with:'' data:dec) == dec).else:{{ ok = false }};

"* a structured value built extension-side still materializes
var rec = e.call:'mkRecord' with:'';
(rec.defined?).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_fidelity_test.qn", &script);
}

/// Ownership of live-instance references: an extension-backed instance can only cross to the
/// extension that owns it. Sending another extension's instance inside a structured value is a
/// catchable error naming the cause — its resource id would be misread in the wrong extension's
/// object table.
#[test]
fn extension_cross_extension_instance_refused() {
    let vector_bin = env!("CARGO_BIN_EXE_ext_vector");
    let data_bin = env!("CARGO_BIN_EXE_ext_data");
    let script = format!(
        r#"
var ev = Extension.spawn:'{vector_bin}';
var v = Vector.ofFloats:#( 1.0 2.0 );
var ed = Extension.spawn:'{data_bin}';
var msg = {{ ed.call:'echoData' with:'' data:#( v ); 'no-error' }}.catch:{{ |ex| ex.s }};
(msg.contains?:'different extension').if:{{ 'PASS'.print }} else:{{ ('FAIL: ' + msg).print }};
"#
    );
    assert_script_passes("qn_ext_cross_ownership_test.qn", &script);
}

/// The protocol-version handshake: a peer whose `ManifestReturn` names a version this host
/// doesn't speak must be refused at spawn with a catchable error naming both versions — never
/// garbage-decoded, never hung. The fixture is a minimal inline Python peer that answers the
/// `GetManifest` with version 99 and then just holds the socket.
#[test]
fn extension_protocol_version_mismatch() {
    if !python_fixture_runnable() {
        eprintln!("skipping extension_protocol_version_mismatch: python3 + `msgpack` unavailable");
        return;
    }
    let fixture_src = r#"#!/usr/bin/env python3
import socket, struct, sys
import msgpack

srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sys.argv[1])
srv.listen(1)
conn, _ = srv.accept()
n = struct.unpack("<I", conn.recv(4))[0]
conn.recv(n)  # the GetManifest; answer with a bogus protocol version
payload = msgpack.packb([8, 99, []])  # [ManifestReturn, version, classes]
conn.sendall(struct.pack("<I", len(payload)) + payload)
conn.recv(4)  # hold the connection until the host gives up and kills us
"#;
    let fixture = std::env::temp_dir().join("qn_ext_version99.py");
    std::fs::write(&fixture, fixture_src).unwrap();
    ensure_executable(fixture.to_str().unwrap());
    let script = format!(
        r#"
var msg = {{ Extension.spawn:'{}'; 'no-error' }}.catch:{{ |ex| ex.s }};
var ok = (msg.contains?:'protocol version 99') && (msg.contains?:'this host speaks');
ok.if:{{ 'PASS'.print }} else:{{ ('FAIL: ' + msg).print }};
"#,
        fixture.display()
    );
    assert_script_passes("qn_ext_version_mismatch_test.qn", &script);
    let _ = std::fs::remove_file(&fixture);
}
