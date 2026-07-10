//! Integer overflow in COMPILED code must raise the same catchable ArithmeticError the
//! interpreter raises (`TAG_INT_OVERFLOW` → "Integer overflow"). The interpreter half lives in
//! `qnlib/tests/63-overflow.qn`; this drives a speculatively-promoted method past the warmth
//! threshold so the overflowing send runs through Cranelift-generated arithmetic, not
//! `devirt_ops` — the two implementations that `codegen/tests.rs` sweeps against each other,
//! here proven to agree end to end through `catch:`.

use std::process::Command;

fn run(script: &str, aot_env: &[(&str, &str)]) -> (String, bool) {
    let path = std::env::temp_dir().join(format!(
        "qn_overflow_aot_{}_{}.qn",
        std::process::id(),
        aot_env.len()
    ));
    std::fs::write(&path, script).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&path).current_dir(env!("CARGO_MANIFEST_DIR"));
    for (k, v) in aot_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run qn");
    let _ = std::fs::remove_file(&path);
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

/// A typed self-adding method: an AOT candidate. Warm it far past any threshold, then feed it
/// the overflowing pair; the error must be catchable and execution must continue.
const SCRIPT: &str = "\
Acc <- {
    add: -> { |n: Integer ^Integer|
        ^n + n
    }
};
var a = Acc.new;
var warm = 0;
var i = 0;
{ i < 200 }.whileDo:{ warm = a.add:1; i = i + 1 };
var caught = nil;
{ a.add:9223372036854775807 }.catch:{ |e: ArithmeticError| caught = e.message };
caught.print;
(a.add:21).print;
";

#[test]
fn compiled_overflow_raises_the_same_catchable_error() {
    // Force-warm compilation so the overflowing call runs native code.
    let (out, ok) = run(SCRIPT, &[("QN_AOT_WARM", "1")]);
    assert!(ok, "qn failed:\n{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        ["Integer overflow", "42"],
        "compiled arithmetic must raise catchably and keep the VM usable"
    );
}

#[test]
fn interpreted_overflow_agrees() {
    // The same program with AOT off: byte-identical observable behavior.
    let (out, ok) = run(SCRIPT, &[("QN_AOT", "0")]);
    assert!(ok, "qn failed:\n{out}");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["Integer overflow", "42"]);
}
