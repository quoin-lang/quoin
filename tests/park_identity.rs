//! Regression tests: a task's *park identity* must survive task-slot reuse. The
//! scheduler recycles finished tasks' slots, so anything that captures "which park
//! is this?" — a channel waiter-queue entry, a `JoinTimed` deadline timer, a
//! pending join wake — must not match a *later* occupant of the same slot (or a
//! later park of the same task). Three shapes that used to go wrong:
//!
//! 1. Channel waiter queues held bare `TaskId`s and liveness was just "that slot
//!    is parked on *a* channel": a cancelled receiver's ghost entry, plus slot
//!    reuse (or the same task re-parking on another channel), let `ch1.send:`
//!    deliver its value to a task parked on ch2 — silent cross-channel
//!    misdelivery, with the send reporting success. Entries now carry the park
//!    epoch and must match it exactly.
//! 2. `park_epoch` restarted at 0 for each slot occupant, and an aborted deadline
//!    future still emits its wakeup: a ghost `JoinTimed` deadline from a finished
//!    `Async.timeout:` matched a later `(joiner, target, epoch)` triple exactly —
//!    a fresh 60 s timeout threw `TimeoutError` instantly, and nested timeouts
//!    blamed the wrong nesting level (skipping the inner `onCancel:`). Epochs are
//!    now allocated from a scheduler-global counter, so no two parks ever share one.
//! 3. `complete_detached` woke a joiner (wake set, pushed ready) but left
//!    `joining` set, and `request_cancel`'s join branch had no `wake.is_none()`
//!    guard: cancelling the woken-but-not-yet-run joiner enqueued it a second
//!    time — "load_task_context: task slot is empty" panic once the first entry
//!    ran to completion.

use std::process::Command;

fn run_script(file_stem: &str, script: &str) {
    let path = std::env::temp_dir().join(format!("qn_{file_stem}.qn"));
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS") && !stdout.contains("FAIL"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn channel_ghost_waiter_cannot_misdeliver_across_channels() {
    run_script(
        "chan_ghost_misdeliver",
        r#"
var ok = true;
var ch1 = Channel.new;
var ch2 = Channel.new;

"* 1: park a receiver on ch1, then cancel it -> its queue entry becomes a ghost.
var entered = #();
var h = Task.spawn:{ entered.add:1; ch1.receive };
{ entered.count == 0 }.whileDo:{ Async.sleep:1 };
h.cancel;
{ h.join }.catch:{ |_| nil };

"* 2: a new task (reusing the freed slot) parks on ch2.
var got = #();
var entered2 = #();
var h2 = Task.spawn:{ entered2.add:1; got.add:(ch2.receive) };
{ entered2.count == 0 }.whileDo:{ Async.sleep:1 };
Async.sleep:20;

"* 3: a send on ch1 has no live ch1 waiter, so it must park — NOT hand the value
"*    to the ghost entry now impersonating the ch2 receiver.
var s = Task.spawn:{ { ch1.send:'fromCh1' }.catch:{ |_| nil } };
Async.sleep:60;
(got.count == 0).else:{ ok = false; 'FAIL: ch1 value delivered to a ch2 receiver'.print };

"* Unblock the parked sender and receiver so the program terminates.
ch1.close;
ch2.close;
{ s.join }.catch:{ |_| nil };
{ h2.join }.catch:{ |_| nil };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
    );
}

#[test]
fn stale_deadline_does_not_fire_on_reused_slots() {
    run_script(
        "stale_deadline",
        r#"
var ok = true;

"* A completed timeout leaves a disarmed-but-queued deadline future behind.
var t1 = Task.spawn:{ Async.timeout: 200 do:{ Async.sleep: 5; 'ok1' } };
(t1.join == 'ok1').else:{ ok = false; 'FAIL: warmup timeout'.print };

"* The next spawn reuses the same task slots; its 60 s timeout must not inherit
"* the ghost deadline (pre-fix: TimeoutError after ~0.05 s).
var t2 = Task.spawn:{ Async.timeout: 60000 do:{ Async.sleep: 300; 'ok2' } };
var r = { t2.join }.catch:{ |e| 'stale-timeout: ' + e.s };
(r == 'ok2').else:{ ok = false; ('FAIL: ' + r.s).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
    );
}

#[test]
fn nested_timeouts_blame_the_inner_deadline() {
    run_script(
        "nested_timeout",
        r#"
var ok = true;

"* Prime: a completed 2-deep nest churns task slots first.
Async.timeout:9998 do:{ Async.timeout:9997 do:{
    Async.timeout:1 do:{ Async.sleep:100000 } onCancel:{ 'x' } } };

"* The inner 2 ms deadline must be the one that fires (pre-fix: ms=22222, instantly).
var r = { Async.timeout:11111 do:{
    Async.timeout:22222 do:{
        Async.timeout:33333 do:{
            Async.timeout:2 do:{ Async.sleep:100000; 'no' }
        }
    }
} }.catch:{ |ex| 'ms=' + ex.ms.s };
(r == 'ms=2').else:{ ok = false; ('FAIL: deadline misattributed: ' + r.s).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
    );
}

#[test]
fn cancelling_an_already_woken_joiner_does_not_panic() {
    run_script(
        "cancel_woken_joiner",
        r#"
var t = Task.spawn:{ Async.sleep:30; 42 };
Async.sleep:1;

"* J parks joining T; main also joins T and (FIFO) is woken first when T finishes,
"* so at this point J sits in `ready` with its join wake already delivered.
var j = Task.spawn:{ { t.join }.catch:{ |_| nil } };
var r = t.join;

"* Cancelling J here used to enqueue it a second time -> 'task slot is empty' panic.
j.cancel;
Async.sleep:50;

(r == 42).if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#,
    );
}
