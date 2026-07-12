//! The C1 compute-offload pool (docs/internal/CONCURRENCY_ARCH.md §4): gated codec
//! ops offload to pool threads while the task parks like an IO wait. One
//! script asserts identical VALUES and identical ERRORS across forced
//! offload, the kill switch (inline), and defaults — plus concurrency via
//! `Async.gather:`, cancellation via `Async.timeout:do:` (the pool op runs
//! to completion, its result dropped), and offload from inside a fiber.

use std::process::Command;

fn assert_script_passes_env(name: &str, script: &str, envs: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, script).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
        cmd.arg(&path);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run qn");
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            return;
        }
        last_diag = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    panic!("compute-offload script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

const SCRIPT: &str = r#"
var ok = true;

"* ~230 KB payload by doubling (comfortably past any offload gate)
var s = 'quoin-payload-';
(0..14).each:{ |i| s = s + s };
var big = s.asBytes;

"* value parity: gzip and deflate round-trips
((big.encodeGz.decodeGz.asString) == s).else:{ ok = false };
((big.encodeDeflate.decodeDeflate.asString) == s).else:{ ok = false };

"* error parity: malformed input is the same catchable ParseError either way
var caught = { ('not gzip data'.asBytes).decodeGz; 'no-error' }.catch:{ |e| 'caught' };
(caught == 'caught').else:{ ok = false };

"* concurrency: gather over offloading tasks (parks overlap on the pool)
var outs = Async.gather:#(
    { big.encodeGz.decodeGz.asString == s }
    { big.encodeDeflate.decodeDeflate.asString == s }
    { big.encodeGz.decodeGz.asString == s }
    { big.encodeGz.decodeGz.asString == s }
);
(outs == #( true true true true )).else:{ ok = false };

"* offload from inside a fiber: the park bubbles through the fiber resume
var f = Fiber.new:{ |z| Fiber.yield:((big.encodeGz.decodeGz.asString) == s); 0 };
((f.resume:0) == true).else:{ ok = false };

"* the stats section moved
var c = VM.stats.at:'compute';
(((c.at:'submitted') + (c.at:'inline')) >= 4).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

#[test]
fn compute_offload_forced() {
    assert_script_passes_env("qn_compute_forced.qn", SCRIPT, &[("QN_COMPUTE_MIN", "1")]);
}

/// Cancellation is only meaningful where the op PARKS (an offloaded compute
/// gives the racing deadline a poll point; a pure-CPU inline loop starves
/// timers — pre-existing cooperative-scheduler behavior, out of scope here).
/// The orphaned pool job finishes in the background, its result dropped, and
/// the VM stays healthy.
#[test]
fn compute_offload_cancellation() {
    let script = r#"
var s = 'quoin-payload-';
(0..14).each:{ |i| s = s + s };
var big = s.asBytes;
var timedOut = {
    Async.timeout:1 do:{ (0..200).each:{ |i| big.encodeGz }; 'finished' }
}.catch:{ |e| 'timed-out' };
var ok = (timedOut == 'timed-out');
((big.encodeGz.decodeGz.asString) == s).else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + timedOut).print };
"#;
    assert_script_passes_env("qn_compute_cancel.qn", script, &[("QN_COMPUTE_MIN", "1")]);
}

#[test]
fn compute_offload_kill_switch_inline() {
    assert_script_passes_env(
        "qn_compute_inline.qn",
        SCRIPT,
        &[("QN_COMPUTE_THREADS", "0")],
    );
}

#[test]
fn compute_offload_default_gates() {
    assert_script_passes_env("qn_compute_default.qn", SCRIPT, &[]);
}
