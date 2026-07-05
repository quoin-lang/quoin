//! Regression tests: a guest `Fiber` that is live inside one task must not be
//! resumable from another. While a fiber is a task's current fiber (or an
//! ancestor on its resume chain), its real execution context is live in — or
//! stashed with — that task, not in the fiber's own state; the task may be
//! parked mid-fiber on I/O, or preempted. `fiber_resume`'s guards only saw the
//! *current* task's chain, so a cross-task resume passed them, loaded an empty
//! context, and re-entered the coroutine at a foreign suspend point: the fiber
//! failed with a bogus "I/O resumed without a result", and when the owning task
//! later re-resumed the now-completed coroutine, corosensei aborted the whole
//! process ("attempt to resume a completed coroutine"). Fibers now track their
//! owning task; a cross-task resume of a live fiber raises a catchable
//! `FiberError`, and a fiber that has yielded (owner cleared) stays legally
//! resumable from any task.

use std::process::Command;

fn run_script(file_stem: &str, script: &str, envs: &[(&str, &str)]) {
    let path = std::env::temp_dir().join(format!("qn_{file_stem}.qn"));
    std::fs::write(&path, script).unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&path);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn resuming_a_fiber_parked_inside_another_task_is_a_catchable_fiber_error() {
    run_script(
        "fiber_foreign_resume",
        r#"
var ok = true;
var f = Fiber.new:{ Async.sleep:40; 'fiber-done' };

"* Task t enters the fiber, which parks t on I/O mid-fiber.
var entered = #();
var t = Task.spawn:{ entered.add:1; f.resume };
{ entered.count == 0 }.whileDo:{ Async.sleep:1 };
Async.sleep:5;

"* A foreign resume must be refused catchably — not corrupt the fiber.
var r = { f.resume; 'no-error' }.catch:{ |e| e.s };
(r == 'no-error').if:{ ok = false; 'FAIL: foreign resume was allowed'.print };

"* The owning task must still drive the fiber to completion (pre-fix: the
"* process aborted here on the completed coroutine).
(t.join == 'fiber-done').else:{ ok = false; 'FAIL: owning task result'.print };
f.done?.else:{ ok = false; 'FAIL: fiber not done'.print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
        &[],
    );
}

#[test]
fn sequential_cross_task_resumes_of_a_yielded_fiber_still_work() {
    run_script(
        "fiber_sequential_cross_task",
        r#"
var ok = true;

"* A yielded fiber belongs to no task; each resume may come from a different one.
var f = Fiber.new:{ |x|
    Fiber.yield:'a';
    Fiber.yield:'b';
    'c'
};
var r1 = (Task.spawn:{ f.resume }).join;
var r2 = (Task.spawn:{ f.resume }).join;
var r3 = f.resume;
((r1 == 'a') && (r2 == 'b') && (r3 == 'c'))
    .else:{ ok = false; ('FAIL: ' + r1.s + '/' + r2.s + '/' + r3.s).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
        &[],
    );
}

#[test]
fn racing_resumes_under_sched_stress_do_not_abort() {
    // Two tasks race to resume one suspended generator under scheduler stress.
    // When the preemption interleaves them mid-fiber, the loser must get a
    // FiberError (or a clean sequential result) — never the pre-fix process
    // abort. Seeded, so the run is reproducible.
    run_script(
        "fiber_racing_resumes",
        r#"
var f = Fiber.new:{ var n = 0; { true }.whileDo:{ ^> n; n = n + 1 } };
var a = Task.spawn:{ { f.resume.s }.catch:{ |e| 'refused' } };
var b = Task.spawn:{ { f.resume.s }.catch:{ |e| 'refused' } };
('a=' + a.join.s + ' b=' + b.join.s).print;
'PASS'.print;
"#,
        &[("QN_SCHED_STRESS", "11")],
    );
}
