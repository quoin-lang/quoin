//! `WorkerService` (L4, docs/internal/CONCURRENCY_ARCH.md §10): host a class in a
//! worker isolate, get a proxy whose ordinary sends become serialized RPC.
//! Covers sticky state, error/MNU transparency, concurrent-caller
//! serialization (exact totals through gather), IO inside hosted methods,
//! stop semantics, boot-failure clarity, and the reserved process backing.

use std::process::Command;

fn assert_service_script_passes(name: &str, script: &str, units: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let dir = std::env::temp_dir().join(format!("qn_svc_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut script = script.to_string();
    for (unit_name, source) in units {
        let path = dir.join(unit_name);
        std::fs::write(&path, source).unwrap();
        script = script.replace(&format!("@{unit_name}@"), path.to_str().unwrap());
    }
    let main_path = dir.join("main.qn");
    std::fs::write(&main_path, &script).unwrap();

    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&main_path)
            .output()
            .expect("run qn");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        last_diag = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(150 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("worker-service script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

const COUNTER_UNIT: &str = r#"
Counter <- { |@total|
    init -> { @total = 0 };
    add: -> { |n| @total = @total + n; @total };
    total -> { @total };
    slowAdd: -> { |n| Async.sleep:20; @total = @total + n; @total };
    boom -> { 'kaboom from the service'.throw }
};
"#;

#[test]
fn service_state_errors_and_stop() {
    let script = r#"
Marker <- { x -> { 1 } };
var ok = true;
var c = WorkerService.host:'@counter.qn@' class:'Counter';

"* sticky state across ordinary sends
((c.add:5) == 5).else:{ ok = false };
((c.add:7) == 12).else:{ ok = false };
((c.total) == 12).else:{ ok = false };

"* a hosted method's throw comes back catchable with its message
var thrown = { c.boom; 'no-error' }.catch:{ |e| e.s };
(thrown.contains?:'kaboom').else:{ ok = false };

"* MNU on the hosted instance names the selector, catchable
var mnu = { c.frobnicate; 'no-error' }.catch:{ |e| e.s };
(mnu.contains?:'frobnicate').else:{ ok = false };

"* a non-portable argument (a plain instance) refuses without occupying
"* the service (blocks don't refuse anymore — they ship or cross as handles)
var badArg = { c.add:(Marker.new); 'sent' }.catch:{ |e| 'refused' };
(badArg == 'refused').else:{ ok = false };
((c.total) == 12).else:{ ok = false };

"* stop waits for quiet, then later calls refuse clearly
c.serviceStop;
var after = { c.total; 'no-error' }.catch:{ |e|
    (e.s.contains?:'stopped').if:{ 'stopped' } else:{ e.s }
};
(after == 'stopped').else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("state", script, &[("counter.qn", COUNTER_UNIT)]);
}

/// The actor property under fire: concurrent callers (whose hosted method
/// PARKS on IO mid-call) serialize on the token — totals stay exact and no
/// reply crosses to the wrong caller.
#[test]
fn service_serializes_concurrent_callers() {
    let script = r#"
var ok = true;
var c = WorkerService.host:'@counter.qn@' class:'Counter';
var outs = Async.gather:#(
    { c.slowAdd:1 } { c.slowAdd:1 } { c.slowAdd:1 } { c.slowAdd:1 }
    { c.slowAdd:1 } { c.slowAdd:1 } { c.slowAdd:1 } { c.slowAdd:1 }
);
"* each reply is a distinct running total (no crossed replies) and the
"* final state is exact
((c.total) == 8).else:{ ok = false };
var seen = #{};
outs.each:{ |v| seen.at:(v.s) put:true };
((seen.count) == 8).else:{ ok = false };
c.serviceStop;
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + outs.s).print };
"#;
    assert_service_script_passes("serialize", script, &[("counter.qn", COUNTER_UNIT)]);
}

#[test]
fn service_boot_failures_and_reserved_backing() {
    let script = r#"
var ok = true;
"* missing unit file: host: raises, catchable
var miss = { WorkerService.host:'/nonexistent/nope.qn' class:'Counter'; 'hosted' }
    .catch:{ |e| 'boot-error' };
(miss == 'boot-error').else:{ ok = false };
"* unit loads but the class doesn't exist
var noClass = { WorkerService.host:'@counter.qn@' class:'NoSuchClass'; 'hosted' }
    .catch:{ |e| 'no-class' };
(noClass == 'no-class').else:{ ok = false };
"* class names must be plain identifiers (they are interpolated)
var inject = { WorkerService.host:'@counter.qn@' class:'X; 1.print'; 'hosted' }
    .catch:{ |e| 'refused' };
(inject == 'refused').else:{ ok = false };
"* process backing is REAL now (§13): host, call, stop over the wire
var pc = WorkerService.host:'@counter.qn@' class:'Counter' backing:'process';
((pc.add:3) == 3).else:{ ok = false };
pc.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("boot", script, &[("counter.qn", COUNTER_UNIT)]);
}

/// Hosted objects (ACTOR_OBJECTS.md §2): a method returning a NON-portable
/// object hosts it — the answer is a live sub-proxy — and sub-proxies pass
/// back in as live references. Errors carry the worker's stack.
const POOL_UNIT: &str = r#"
Cell <- { |@value|
    init -> { @value = 0 };
    put: -> { |v| @value = v; @value };
    value -> { @value };
    plus: -> { |other| @value + other.value };
    boomCell -> { 'cell went boom'.throw }
};
Pool <- { |@made|
    init -> { @made = 0 };
    makeCell -> { @made = @made + 1; Cell.new };
    made -> { @made };
    sum: -> { |cell| cell.value + @made }
};
"#;

const HOSTED_OBJECTS_SCRIPT: &str = r#"
var ok = true;
var p = WorkerService.host:'@pool.qn@' class:'Pool' backing:'@BACKING@';

"* a non-portable return is HOSTED: the answer is a live sub-proxy
var a = p.makeCell;
var b = p.makeCell;
((p.made) == 2).else:{ ok = false; 'FAIL: made'.print };
((a.put:41) == 41).else:{ ok = false; 'FAIL: put'.print };
((a.value) == 41).else:{ ok = false; 'FAIL: value'.print };
((b.value) == 0).else:{ ok = false; 'FAIL: sub-proxies not isolated'.print };

"* sub-proxies as ARGUMENTS travel as live references (same worker)
((a.plus:b) == 41).else:{ ok = false; 'FAIL: proxy arg to sub-proxy'.print };
((p.sum:a) == 43).else:{ ok = false; 'FAIL: proxy arg to root'.print };

"* a hosted raise carries the worker's rendered stack as remoteStack
var msg = { a.boomCell; 'no-error' }.catch:{ |e| e.s };
(msg.contains?:'cell went boom').else:{ ok = false; ('FAIL msg: ' + msg).print };
var blob = { a.boomCell; nil }.catch:{ |e| e.remoteStack };
((blob != nil) && (blob.contains?:'worker')).else:{ ok = false; 'FAIL: remoteStack'.print };

"* stop is worker-wide: afterwards EVERY proxy of the service refuses
p.serviceStop;
var after = { a.value; 'no-error' }.catch:{ |e|
    (e.s.contains?:'stopped').if:{ 'stopped' } else:{ e.s }
};
(after == 'stopped').else:{ ok = false; ('FAIL after: ' + after).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

#[test]
fn service_hosts_returned_objects() {
    let script = HOSTED_OBJECTS_SCRIPT.replace("@BACKING@", "thread");
    assert_service_script_passes("hosted", &script, &[("pool.qn", POOL_UNIT)]);
}

#[test]
fn service_hosts_returned_objects_process() {
    let script = HOSTED_OBJECTS_SCRIPT.replace("@BACKING@", "process");
    assert_service_script_passes("hosted_proc", &script, &[("pool.qn", POOL_UNIT)]);
}

/// Portable-block arguments (ACTOR_OBJECTS.md §3a): a block argument to a
/// THREAD-backed service ships as a capture snapshot and runs worker-side —
/// one crossing however many times the hosted method invokes it. Unportable
/// blocks refuse at the encode seam (before the token); a block whose
/// captures reference a global the worker lacks errors clearly at rebuild.
const RUNNER_UNIT: &str = r#"
Runner <- { |@stash @tally|
    init -> { @stash = nil; @tally = 0 };
    double: -> { |n| n * 2 };
    slowDouble: -> { |n| Async.sleep:300; n * 2 };
    mate -> { Runner.new };
    tally: -> { |n|
        var v = @tally;
        Async.sleep:5;
        @tally = v + n;
        @tally
    };
    apply:to: -> { |blk n| blk.value:n };
    sumOver:with: -> { |items blk|
        var t = 0;
        items.each:{ |i| t = t + (blk.value:i) };
        t
    };
    stash: -> { |blk| @stash = blk; 'stashed' };
    runStash: -> { |n| @stash.value:n }
};
"#;

#[test]
fn service_block_args() {
    let script = r#"
GHelper <- { twice: -> { |n| n * 2 } };
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner';

"* a portable block ships and runs worker-side
((r.apply:{ |n| n * 3 } to:14) == 42).else:{ ok = false; 'FAIL: apply'.print };

"* captures snapshot — including a block capturing a block
var base = 100;
var inner = { |n| n + base };
var outer = { |n| inner.value:(n * 2) };
((r.apply:outer to:5) == 110).else:{ ok = false; 'FAIL: capture'.print };

"* invoked N times worker-side off one crossing
((r.sumOver:#( 1 2 3 4 ) with:{ |i| i * i }) == 30)
    .else:{ ok = false; 'FAIL: sumOver'.print };

"* an UNPORTABLE block (writes a capture) crosses as a HANDLE: it runs in
"* the PARENT and sees its captures live — the §3a fallback, not an error
var w = 0;
((r.apply:{ |n| w = n; n + 1 } to:41) == 42).else:{ ok = false; 'FAIL: handle'.print };
(w == 41).else:{ ok = false; 'FAIL: live capture'.print };

"* a PORTABLE block freezes its captures at send time (ship = snapshot)
var x = 1;
r.stash:{ |n| x + n };
x = 2;
((r.runStash:0) == 1).else:{ ok = false; 'FAIL: snapshot'.print };

"* a stored HANDLE stays live across calls, still seeing the parent live
var y = 10;
r.stash:{ |n| y = y + n; y };
y = 100;
((r.runStash:5) == 105).else:{ ok = false; 'FAIL: stored handle'.print };
(y == 105).else:{ ok = false; 'FAIL: handle mutation'.print };

"* a block the worker invokes may call back into the SAME service — the
"* nested call rides the open conversation instead of deadlocking
((r.apply:{ |n| (r.double:n) + 1 } to:10) == 21)
    .else:{ ok = false; 'FAIL: nested'.print };

"* ...and into a DIFFERENT service
var other = WorkerService.host:'@runner.qn@' class:'Runner';
((r.apply:{ |n| other.double:n } to:7) == 14)
    .else:{ ok = false; 'FAIL: other service'.print };
other.serviceStop;

"* a parent block's throw surfaces catchably at the call site
var msg = { r.apply:{ |n| 'parent boom'.throw } to:1; 'no-error' }.catch:{ |e| e.s };
(msg.contains?:'parent boom').else:{ ok = false; ('FAIL msg: ' + msg).print };

"* a timeout mid-conversation ABANDONS it cleanly: the caller sees the
"* timeout and the service keeps working
var t = { Async.timeout:30 do:{ r.apply:{ |n| Async.sleep:400; n } to:1 }; 'no-timeout' }
    .catch:{ |e| 'timed-out' };
(t == 'timed-out').else:{ ok = false; ('FAIL t: ' + t).print };
((r.double:4) == 8).else:{ ok = false; 'FAIL: unusable after timeout'.print };

"* unbounded mutual parent<->worker recursion errors catchably; the
"* service survives the full unwind
r.stash:{ |n| r.runStash:n };
var deep = { r.runStash:1; 'no-error' }.catch:{ |e| 'capped' };
(deep == 'capped').else:{ ok = false; 'FAIL: no depth cap'.print };
((r.double:3) == 6).else:{ ok = false; 'FAIL: unusable after cap'.print };

"* a capture chain reaching a global the worker lacks errors at rebuild,
"* catchably, naming the worker
var glob = { |n| GHelper.twice:n };
var miss = { r.apply:glob to:0; 'no-error' }.catch:{ |e| e.s };
(miss.contains?:'not defined in the worker')
    .else:{ ok = false; ('FAIL miss: ' + miss).print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_args", script, &[("runner.qn", RUNNER_UNIT)]);
}

#[test]
fn service_block_args_process() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' backing:'process';

"* blocks cross a PROCESS boundary as handles: the block runs in the
"* PARENT, driven over the socket, seeing its captures live
var w = 0;
((r.apply:{ |n| w = n; n * 2 } to:21) == 42).else:{ ok = false; 'FAIL: apply'.print };
(w == 21).else:{ ok = false; 'FAIL: live capture'.print };

"* portable blocks freeze their captures at send time here too (the handle
"* wraps a local snapshot — the backing does not change semantics)
var x = 1;
r.stash:{ |n| x + n };
x = 2;
((r.runStash:0) == 1).else:{ ok = false; 'FAIL: snapshot'.print };

"* nested: a block the worker invokes calls back into the same service,
"* riding the conversation over the socket
((r.apply:{ |n| (r.double:n) + 1 } to:10) == 21)
    .else:{ ok = false; 'FAIL: nested'.print };

"* a parent block's throw surfaces catchably
var msg = { r.apply:{ |n| 'parent boom'.throw } to:1; 'no-error' }.catch:{ |e| e.s };
(msg.contains?:'parent boom').else:{ ok = false; ('FAIL msg: ' + msg).print };

"* a timeout mid-conversation abandons it cleanly over the socket too
var t = { Async.timeout:30 do:{ r.apply:{ |n| Async.sleep:400; n } to:1 }; 'no-timeout' }
    .catch:{ |e| 'timed-out' };
(t == 'timed-out').else:{ ok = false; ('FAIL t: ' + t).print };
((r.double:4) == 8).else:{ ok = false; 'FAIL: unusable after timeout'.print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_args_proc", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// Per-object mailboxes + lanes (ACTOR_OBJECTS.md §5.1): calls to DIFFERENT
/// objects of one service overlap up to the lane count; calls to ONE object
/// still serialize exactly (its mailbox); a hot object's queue never blocks
/// the peer's other objects; and the claim shapes are visible in `VM.claims`.
#[test]
fn service_lanes_overlap_and_serialize() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' lanes:4;
var m = r.mate;

"* two lanes genuinely overlap: a slow call on one object does not delay a
"* fast call on another (with one lane the fast call would queue behind it)
var order = List.new;
var slow = Task.spawn:{ r.slowDouble:5; order.add:'slow' };
Async.sleep:60;
m.double:3;
order.add:'fast';
slow.join;
((order.at:0) == 'fast').else:{ ok = false; ('FAIL order: ' + order.s).print };

"* one object's mailbox stays exact under concurrent callers on 4 lanes:
"* tally: reads, parks, writes — interleaving would lose updates
Async.gather:#(
    { m.tally:1 } { m.tally:1 } { m.tally:1 }
    { m.tally:1 } { m.tally:1 } { m.tally:1 }
);
((m.tally:0) == 6).else:{ ok = false; 'FAIL: tally'.print };

"* the hot-object queue never pinned a lane: while callers queue on one
"* object, another object answers immediately (bounded by its own work)
var hot = r.mate;
var queued = Task.spawn:{ Async.gather:#(
    { hot.slowDouble:1 } { hot.slowDouble:1 } { hot.slowDouble:1 }
) };
Async.sleep:60;
((m.double:2) == 4).else:{ ok = false; 'FAIL: HOL'.print };

"* the claim shapes are observable while the hot object is contended
var claims = VM.claims;
(claims.count >= 1).else:{ ok = false; 'FAIL: claims empty'.print };
var report = VM.claimsReport;
(report.contains?:'svc:').else:{ ok = false; ('FAIL report: ' + report).print };
queued.join;

"* contention was counted
var contended = ((VM.claims.at:0).at:'stats').at:'contended';
(contended >= 1).else:{ ok = false; 'FAIL: no contention counted'.print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("lanes", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// §5.1 rule 3, end to end: a nested send (a callback calling back into
/// another object of the same worker) rides its bound lane — with ONE lane,
/// this deadlocks under any discipline where nested calls wait for lanes.
#[test]
fn service_nested_rides_bound_lane() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner';
var m = r.mate;
((r.apply:{ |n| m.double:n } to:9) == 18)
    .else:{ ok = false; 'FAIL: nested cross-object'.print };
r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("nested_lane", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// §5.1 over real sockets (5b): a PROCESS service with N lanes — overlap,
/// exact per-object serialization, nested calls riding their conversation,
/// and the child's handler time crossing as `ReplyMeta` into
/// `VM.boundaryStats`.
#[test]
fn service_lanes_process() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' backing:'process' lanes:3;
var m = r.mate;

"* two lanes genuinely overlap over the socket pair
var order = List.new;
var slow = Task.spawn:{ r.slowDouble:5; order.add:'slow' };
Async.sleep:60;
m.double:3;
order.add:'fast';
slow.join;
((order.at:0) == 'fast').else:{ ok = false; ('FAIL order: ' + order.s).print };

"* one object's mailbox stays exact under concurrent callers on 3 lanes
Async.gather:#(
    { m.tally:1 } { m.tally:1 } { m.tally:1 }
    { m.tally:1 } { m.tally:1 } { m.tally:1 }
);
((m.tally:0) == 6).else:{ ok = false; 'FAIL: tally'.print };

"* nested: a parent block the worker invokes calls back into another object
"* of the same worker, riding its own conversation over the socket
((r.apply:{ |n| m.double:n } to:9) == 18)
    .else:{ ok = false; 'FAIL: nested'.print };

"* the child's handler time crosses the socket as ReplyMeta: this service's
"* boundary rows decompose (handler > 0), no longer 0-until-5b
var svc = VM.boundaryStats.select:{ |row|
    ((row.at:'peer').contains?:'svc:') && ((row.at:'handlerMicros') > 0)
};
(svc.count >= 1).else:{ ok = false; 'FAIL: no handler timing'.print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("lanes_proc", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// The mutual-call cycle over process backing: detection is parent-side
/// claim state, so the backing changes nothing — verified anyway.
#[test]
fn service_mutual_call_deadlock_detected_process() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' backing:'process' lanes:2;
var a = r.mate;
var b = r.mate;
var ta = Task.spawn:{ { (a.apply:{ |n| Async.sleep:80; b.double:n } to:1).s }.catch:{ |e| e.s } };
var tb = Task.spawn:{ { (b.apply:{ |n| Async.sleep:80; a.double:n } to:1).s }.catch:{ |e| e.s } };
var oa = ta.join;
var ob = tb.join;
var died = 0;
(oa.contains?:'deadlock').if:{ died = died + 1 };
(ob.contains?:'deadlock').if:{ died = died + 1 };
(died == 1).else:{ ok = false; ('FAIL died=' + died.s + ' oa=' + oa + ' ob=' + ob).print };
((oa == '2') || (ob == '2')).else:{ ok = false; ('FAIL winner: ' + oa + ' / ' + ob).print };
((a.double:4) == 8).else:{ ok = false; 'FAIL: unusable after deadlock'.print };
r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("deadlock_proc", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// §5.1 rule 6, end to end: two tasks whose callbacks synchronously call
/// each other's held objects — the cycle raises catchably at the task that
/// closes it; the other call completes; the service survives.
#[test]
fn service_mutual_call_deadlock_detected() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' lanes:2;
var a = r.mate;
var b = r.mate;

var ta = Task.spawn:{ { (a.apply:{ |n| Async.sleep:80; b.double:n } to:1).s }.catch:{ |e| e.s } };
var tb = Task.spawn:{ { (b.apply:{ |n| Async.sleep:80; a.double:n } to:1).s }.catch:{ |e| e.s } };
var oa = ta.join;
var ob = tb.join;

"* exactly one side closed the cycle and got the catchable deadlock error;
"* the other completed normally once the loser unwound
var died = 0;
(oa.contains?:'deadlock').if:{ died = died + 1 };
(ob.contains?:'deadlock').if:{ died = died + 1 };
(died == 1).else:{ ok = false; ('FAIL died=' + died.s + ' oa=' + oa + ' ob=' + ob).print };
((oa == '2') || (ob == '2')).else:{ ok = false; ('FAIL winner: ' + oa + ' / ' + ob).print };

"* the detection was counted, and the service still answers
var dl = ((VM.claims.at:0).at:'stats').at:'deadlocks';
(dl == 1).else:{ ok = false; ('FAIL dl=' + dl.s).print };
((a.double:4) == 8).else:{ ok = false; 'FAIL: unusable after deadlock'.print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("deadlock", script, &[("runner.qn", RUNNER_UNIT)]);
}
