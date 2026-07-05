//! Regression test: unbounded native → Quoin re-entry must fail with a catchable
//! error, not overflow the machine stack and abort the process. A native method that
//! calls back into Quoin (a custom `==:` / `hash` / comparator / render hook) which
//! re-enters the same native op recurses on the *real* Rust/C stack — unlike pure-Quoin
//! recursion, which grows the heap frame stack and is already catchable. Without a bound
//! the classic shape (an `==:` that re-adds to the set it is a key of, driving
//! `set_add → == : → set_add …`) SIGBUS'd uncatchably. `call_method`/`call_method_value`
//! now cap the re-entry depth and raise a catchable error at the ceiling.

use std::process::Command;

#[test]
fn unbounded_native_reentry_is_a_catchable_error_not_a_crash() {
    let script = r#"
var ok = true;

Evil <- { | @bag |
    init: -> { |b| @bag = b };
    "* a custom equality that mutates the set it is being compared within: every
    "* membership check re-enters set_add, which re-enters == :, without bound
    #'==:' -> { |o| @bag.add:(Evil.new:{ var b = nil }); false }
};

var s = #<>;
s.add:(Evil.new:{ var b = s });
var r = { s.add:(Evil.new:{ var b = s }); 'no-error' }.catch:{ |e| 'caught' };
(r == 'caught').else:{ ok = false; ('FAIL: expected catch, got ' + r.s).print };

"* the VM is still alive and usable after catching the runaway recursion
((1 + 1) == 2).else:{ ok = false; 'FAIL: VM unusable after recursion'.print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let path = std::env::temp_dir().join("qn_native_recursion.qn");
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // A crash would be a signal (no clean exit); PASS proves it was caught in-VM.
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "did not pass (a crash shows as a non-zero signal).\nstatus: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
}
