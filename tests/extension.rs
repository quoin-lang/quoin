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
//! - `extension_backed_classes` (Phase 3): the `ext_vector` fixture *provides* a Quoin class
//!   `Vector` — the host installs it as a global from the spawn-time manifest, and `Vector ofFloats:`
//!   / `v sum` / `v scale:` dispatch over the socket as ordinary method sends (SDK-owned instances).
//! - `extension_backed_classes_python` (Phase 3b): the same, but the `Vector`-providing extension is
//!   a *Python* process (`ext_vector.py`) — proving the manifest + class-dispatch protocol is
//!   polyglot. Gated on `python3` + `flatbuffers`.
//! - `extension_python_sdk` (Slice 7): the extension is a *Python* process (`sdk/python`) speaking
//!   the same `ext.fbs` wire protocol — the polyglot proof. Gated on `python3` + `flatbuffers`.
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
fn extension_timeout() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';

"* a hung call is bounded by Async.timeout:do: — raises a catchable TimeoutError, VM survives
r = {{ Async.timeout:300 do:{{ e.call:'hang' with:'' }} }}.catch:{{ |ex| 'timedout' }};
(r == 'timedout').else:{{ ok = false }};

"* the conversation is desynced -> the extension is dead; a follow-up fails fast (does NOT hang
"* waiting on a child still stuck in `hang`)
again = {{ e.call:'ping' with:'' }}.catch:{{ |ex| 'dead' }};
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

#[test]
fn extension_array_data_plane() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_arrays");
    let script = format!(
        r#"
ok = true;

e = Extension.spawn:'{ext_bin}';
a = Array.ofFloats:#( 1.0 2.0 3.0 );

"* the bulk column crosses the socket; the extension sums the whole buffer -> '6'
((e.call:'sum' with:'' args:#( a )) == '6').else:{{ ok = false }};

"* scale: returns a new Array (the column round-trips back as an Array, not a List)
r = e.call:'scale' with:'2' args:#( a );
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
ok = true;

e = Extension.spawn:'{ext_bin}';

"* a structured value round-trips: Quoin Map -> DataValue -> (ext) -> DataValue -> Quoin Map
m = #{{ 'n': 42 'f': 1.5 's': 'hi' 'flag': true 'items': #( 1 2 3 ) }};
((e.call:'echoData' with:'' data:m) == m).else:{{ ok = false }};

"* a structured value built extension-side materializes as a real Quoin List
((e.call:'mkList' with:'') == #( 1 2 3 )).else:{{ ok = false }};

"* host reach (Phase 2): the ext reaches the Array class, builds it, and returns it live
arr = e.call:'buildArray' with:'';
(arr.dtype == #float64).else:{{ ok = false }};
(arr.sum == 6.0).else:{{ ok = false }};

"* host reach: the ext reads a passed value back as structured data (read_handle)
((e.call:'inspect' with:'' args:#( #{{ 'k': 7 }} )) == #{{ 'k': 7 }}).else:{{ ok = false }};

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
ok = true;

e = Extension.spawn:'{ext_bin}';

"* an extension-backed class (Phase 3): the class-side constructor builds a live instance, and an
"* ordinary method send dispatches over the socket — `Vector` is a real global, `v` a real instance
v = Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{{ ok = false }};
(v.length == 3).else:{{ ok = false }};

"* an instance method that *makes* a new instance returns another (socket-backed) `Vector`
w = v.scale:2.0;
(w.sum == 12.0).else:{{ ok = false }};
(w.length == 3).else:{{ ok = false }};

"* the receiver is unchanged — distinct instances, each its own ext-side object
(v.sum == 6.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_vector_test.qn", &script);
}

/// True if `python3` can import the `flatbuffers` runtime — the Python SDK's only external
/// dependency. When false, the polyglot tests skip cleanly (e.g. CI without Python set up).
fn python_fixture_runnable() -> bool {
    Command::new("python3")
        .args(["-c", "import flatbuffers"])
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
        eprintln!("skipping extension_python_sdk: python3 with `flatbuffers` runtime unavailable");
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
ok = true;

e = Extension.spawn:'{fixture}';
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
        eprintln!(
            "skipping extension_python_parity: python3 with `flatbuffers` runtime unavailable"
        );
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
ok = true;
e = Extension.spawn:'{fixture}';

"* host-ops: make_string + call_method ('ab' +: '!').upper -> 'AB!'
((e.call:'compute' with:'ab') == 'AB!').else:{{ ok = false }};

"* batched callback: invoke_block runs the passed block over a,b,c -> 'A,B,C'
((e.call:'mapUpper' with:'' args:#( {{ |s| s.upper }} )) == 'A,B,C').else:{{ ok = false }};

"* ext-resource handles: create a counter, mutate it across calls, then drop + reap
c = e.call:'new' with:'';
((e.call:'inc' with:'' args:#( c )) == '1').else:{{ ok = false }};
((e.call:'inc' with:'' args:#( c )) == '2').else:{{ ok = false }};
((e.call:'live' with:'') == '1').else:{{ ok = false }};
c = nil;
(1..5000).each:{{ |i| i.s }};
e.call:'live' with:'';
((e.call:'live' with:'') == '0').else:{{ ok = false }};

"* Array data plane: sum the whole buffer, and scale -> a new Array
a = Array.ofFloats:#( 1.0 2.0 3.0 );
((e.call:'sum' with:'' args:#( a )) == '6.0').else:{{ ok = false }};
r = e.call:'scale' with:'2' args:#( a );
(r.toList == #( 2.0 4.0 6.0 )).else:{{ ok = false }};

"* structured values (Phase 1): a Map round-trips, and a record built in Python -> a Quoin Map
m = #{{ 'a': 1 'b': #( 'x' 'y' ) 'c': true }};
((e.call:'echoData' with:'' data:m) == m).else:{{ ok = false }};
((e.call:'mkRecord' with:'') == #{{ 'name': 'quoin' 'items': #( 1 2 3 ) 'ok': true }}).else:{{ ok = false }};

"* host reach (Phase 2): Python reaches the Array class, builds it, returns it live; and inspects
arr = e.call:'buildArray' with:'';
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
        eprintln!(
            "skipping extension_backed_classes_python: python3 with `flatbuffers` runtime unavailable"
        );
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
ok = true;

e = Extension.spawn:'{fixture}';

"* a Python extension provides `Vector`: constructor + method send dispatch over the socket
v = Vector.ofFloats:#( 1.0 2.0 3.0 );
(v.sum == 6.0).else:{{ ok = false }};
(v.length == 3).else:{{ ok = false }};

"* a method returning a new instance is auto-detected Python-side (isinstance — no `makes`)
w = v.scale:2.0;
(w.sum == 12.0).else:{{ ok = false }};
(v.sum == 6.0).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_script_passes("qn_ext_vector_python_test.qn", &script);
}
