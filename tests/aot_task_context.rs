//! Regression test: `VmState.aot_enclosing_env` is PER-TASK state, like
//! `aot_fuel` — a compiled body can park (a native outcall like `Async.sleep:`,
//! or a fuel checkpoint) and another task can run compiled frames while it is
//! suspended. The field must be saved/restored with the task context, or a
//! cold-path `make_closure` after the resume chains its snapshot to whichever
//! lexical environment the OTHER task's compiled frame installed last.
//!
//! The pinned shape: two classes bind the same NAME (`secret`) to different
//! values in their class-body environments; each task runs a compiled `work:`
//! that parks at a DIRECT native outcall (`Async.sleep:` — no nested compiled
//! invocation in between, whose exit-path restore would mask the bug) and then
//! materializes `{ secret }`. Unfixed, task A's closure resolved `secret`
//! through task B's class-body env ("from-B").

use std::process::Command;

#[test]
fn materialized_closure_env_survives_task_interleaving() {
    let script = r#"
A <- {
    var secret = 'from-A';
    grab: -> { |b| ^b.value };
    work: -> { |ms| Async.sleep:ms; ^.grab:{ secret } }
};
B <- {
    var secret = 'from-B';
    grab: -> { |b| ^b.value };
    work: -> { |ms| Async.sleep:ms; ^.grab:{ secret } }
};

var a = A.new;
var b = B.new;
"* warm both methods so they promote (QN_AOT_WARM=1)"
var i = 0;
{ i < 3 }.whileDo:{ a.work:1; b.work:1; i = i + 1 };

var ta = Task.spawn:{ a.work:30 };
var tb = Task.spawn:{ Async.sleep:5; b.work:60 };
var ra = ta.join;
var rb = tb.join;
((ra == 'from-A') && (rb == 'from-B')).if:{ 'ctx: ok'.print }
else:{ ('ctx: FAIL a=' + ra + ' b=' + rb).print }
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_aot_task_context.qn");
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .env("QN_AOT_WARM", "1")
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("ctx: ok"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
