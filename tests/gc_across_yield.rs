//! Regression test: no GC-managed value may live in a Rust local across a
//! point where arbitrary Quoin runs, because that code can cooperatively park
//! the task — and the collector cannot scan a suspended coroutine's native
//! stack, so anything unrooted there is collectible (the `no_gc_across_yield`
//! lint's subject, enforced end-to-end here under GC stress).
//!
//! The pinned shape: a set literal dedups through user `==:`, which parks
//! (`Async.sleep:`); the literal's elements and the fresh set used to sit in
//! Rust locals across those yields and were collected by a concurrent task's
//! allocation churn. They now stay rooted on the VM stack throughout.

use std::process::Command;

#[test]
fn set_literal_survives_parking_equality() {
    let script = r#"
"* The hashed-Set contract: a class overriding ==: must override hash too
"* (a constant hash here forces every pair into one bucket, so the parking
"* ==: below actually runs — which is this test's whole point).
Sleepy <- { | @n |
    init -> { };
    n -> { @n };
    hash -> { 1 };
    #'==:' -> { |other| Async.sleep:2; ^@n == (other.n) }
};

var churn = Task.spawn:{
    var junk = 0;
    (1..2000).each:{ |i| junk = junk + ('pad-' + i).length };
    junk
};
var s = #<(Sleepy.new:{ var n = 1 }) (Sleepy.new:{ var n = 1 }) (Sleepy.new:{ var n = 2 })>;
churn.join;
(s.count == 2).if:{ 'newset: ok'.print } else:{ ('newset: FAIL got ' + s.count).print }
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_gc_across_yield_set_literal.qn");
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .env("QN_GC_STRESS", "1")
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("newset: ok"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
