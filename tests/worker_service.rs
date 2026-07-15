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
var c = Worker.host:'@counter.qn@' with:{ Counter.new };

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
var c = Worker.host:'@counter.qn@' with:{ Counter.new };
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
var miss = { Worker.host:'/nonexistent/nope.qn' with:{ Counter.new }; 'hosted' }
    .catch:{ |e| 'boot-error' };
(miss == 'boot-error').else:{ ok = false };
"* unit loads but the class doesn't exist
var noClass = { Worker.host:'@counter.qn@' with:{ NoSuchClass.new }; 'hosted' }
    .catch:{ |e| 'no-class' };
(noClass == 'no-class').else:{ ok = false };
"* process backing is REAL now (§13): host, call, stop over the wire
var pc = Worker.host:'@counter.qn@' with:{ Counter.new } backing:'process';
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
Pool <- { |@made @stash|
    .meta <-- {
        poolKind -> { 'classy' }
    }
    init -> { @made = 0; @stash = nil };
    makeCell -> { @made = @made + 1; Cell.new };
    made -> { @made };
    sum: -> { |cell| cell.value + @made };
    stash: -> { |cell| @stash = cell; nil };
    stashed -> { @stash };
    thunk -> { { |n| n + @made } }
};
"#;

const HOSTED_OBJECTS_SCRIPT: &str = r#"
var ok = true;
var p = Worker.host:'@pool.qn@' with:{ Pool.new } backing:'@BACKING@';

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

/// The block host forms (slice 8): `host:'unit' with:{ … }` gives hosted
/// objects real constructor arguments (the block ships and runs IN the
/// worker after its unit loads); bare `with:{ … }` hosts a qnlib-only object
/// with no unit at all. Non-objects, unportable blocks, and parent-defined
/// globals all refuse with clear errors.
#[test]
fn worker_host_block_forms() {
    const SIZED_UNIT: &str = r#"
Sized <- { |@size|
    init -> { @size = 0 };
    size -> { @size };
    size: -> { |n| @size = n; self }
};
"#;
    let script = r#"
Marker <- { x -> { 1 } };
var ok = true;

"* host:with: — the init block runs IN the worker: real constructor args
var cfg = 6;
var s = Worker.host:'@sized.qn@' with:{ Sized.new.size:cfg };
((s.size) == 6).else:{ ok = false; ('FAIL size: ' + s.size.s).print };
((s.class.name.s) == 'Sized').else:{ ok = false; 'FAIL: class name'.print };
s.serviceStop;

"* host:with:lanes: — the lanes variant of the same
var s2 = Worker.host:'@sized.qn@' with:{ Sized.new.size:1 } lanes:2;
((s2.size) == 1).else:{ ok = false; 'FAIL: lanes form'.print };
s2.serviceStop;

"* bare with: — no unit, qnlib classes only; a hosted stdlib Map is an actor
var m = Worker.with:{ #{} };
m.at:'x' put:41;
((m.at:'x') == 41).else:{ ok = false; 'FAIL: hosted map'.print };
m.serviceStop;

"* a parent-defined global in the block errors clearly (workers boot qnlib)
var bad = { Worker.with:{ Marker.new }; 'hosted' }.catch:{ |e| e.s };
(bad.contains?:'not defined in the worker')
    .else:{ ok = false; ('FAIL bad: ' + bad).print };

"* a block answering a non-object refuses
var prim = { Worker.with:{ 42 }; 'hosted' }.catch:{ |e| 'refused' };
(prim == 'refused').else:{ ok = false; 'FAIL: primitive host'.print };

"* an unportable block refuses at the seam
var w = 0;
var unport = { Worker.with:{ w = 1; #{} }; 'hosted' }.catch:{ |e| 'refused' };
(unport == 'refused').else:{ ok = false; 'FAIL: unportable'.print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_forms", script, &[("sized.qn", SIZED_UNIT)]);
}

/// Hosted manifests (ACTOR_OBJECTS.md §2): proxies are REAL installed classes
/// — the VM dispatch hook is gone. Introspection answers the manifest, misses
/// are honest MNUs, `==` is hosted-object identity (the worker table dedupes),
/// class-side sends reach the hosted class, and classes the worker never
/// declared up front (here: Cell, and even Block) install lazily when their
/// first instance crosses.
#[test]
fn service_manifest_classes() {
    let script = r#"
var ok = true;
var p = Worker.host:'@pool.qn@' with:{ Pool.new };

"* the proxy's class is a REAL class named after the hosted one, and
"* introspection answers the manifest without any round trip
((p.class.name.s) == 'Pool').else:{ ok = false; ('FAIL name: ' + p.class.name.s).print };
(p.can?:#makeCell).else:{ ok = false; 'FAIL: can? manifest'.print };
(p.can?:#frobnicate).if:{ ok = false; 'FAIL: ghost selector'.print };

"* class-side sends reach the hosted CLASS (recv 0)
((p.class.poolKind) == 'classy').else:{ ok = false; 'FAIL: class-side'.print };

"* == is hosted-object identity: the same cell, stashed and re-returned,
"* answers the same proxy identity; distinct cells do not
var a = p.makeCell;
p.stash:a;
var b = p.stashed;
(a == b).else:{ ok = false; 'FAIL: identity =='.print };
var c = p.makeCell;
(a == c).if:{ ok = false; 'FAIL: distinct =='.print };

"* a returned BLOCK is a sub-proxy of the lazily-declared Block class —
"* value: dispatches remotely (the S10 question, answered)
var t = p.thunk;
p.makeCell;
((t.value:1) == 4).else:{ ok = false; ('FAIL thunk: ' + (t.value:1).s).print };

p.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("manifest", script, &[("pool.qn", POOL_UNIT)]);
}

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
var r = Worker.host:'@runner.qn@' with:{ Runner.new };

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
var other = Worker.host:'@runner.qn@' with:{ Runner.new };
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
var r = Worker.host:'@runner.qn@' with:{ Runner.new } backing:'process';

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
var r = Worker.host:'@runner.qn@' with:{ Runner.new } lanes:4;
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
var r = Worker.host:'@runner.qn@' with:{ Runner.new };
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
var r = Worker.host:'@runner.qn@' with:{ Runner.new } backing:'process' lanes:3;
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
var r = Worker.host:'@runner.qn@' with:{ Runner.new } backing:'process' lanes:2;
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
var r = Worker.host:'@runner.qn@' with:{ Runner.new } lanes:2;
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

/// Block forms on PROCESS backing: the block crosses as source + capture
/// snapshot, compiles in the child against its unit, and the object it
/// answers is hosted — including a block-valued capture (nested source
/// shipping) invoked worker-side, and a bare-qnlib unit-less actor.
#[test]
fn hosted_block_process_backing() {
    let script = r#"
var ok = true;

"* data capture: k ships as a snapshot and lands in the constructor
var k = 20;
var adder = { |x| x + 40 };
var svc = Worker.host:'@runner.qn@' with:{
    var r = Runner.new;
    r.tally:k;
    r.stash:adder;
    r
} backing:'process' lanes:2;
((svc.double:4) == 8).else:{ ok = false; 'FAIL double'.print };
"* the constructor really ran with the captured value
((svc.tally:1) == 21).else:{ ok = false; 'FAIL capture'.print };
"* the block-valued capture crossed as nested source and runs worker-side
((svc.runStash:2) == 42).else:{ ok = false; 'FAIL nested block'.print };
svc.serviceStop;

"* unit-less: a hosted stdlib Map in a bare-qnlib child process
var cache = Worker.with:{ #{} } backing:'process';
cache.at:'x' put:7;
((cache.at:'x') == 7).else:{ ok = false; 'FAIL map actor'.print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_proc", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// Process block-form refusals stay loud and catchable: a parent-defined
/// global can't exist in the child, and the error names it.
#[test]
fn hosted_block_process_refusals() {
    let script = r#"
var ok = true;
Local <- { ping -> { 'pong' } };
var r = { Worker.with:{ Local.new } backing:'process'; 'hosted' }
    .catch:{ |e| e.s };
(r == 'hosted').if:{ ok = false; 'FAIL: parent global crossed?'.print };
(r.contains?:'Local').else:{ ok = false; ('FAIL msg: ' + r).print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_proc_refuse", script, &[]);
}

/// Spawn-time `args:` — the 7c resolution: the block takes parameters instead
/// of capturing live things. Data snapshots, a portable block crosses as a
/// callable, and a CHANNEL becomes a live relay endpoint the hosted object
/// keeps — sends from inside hosted methods reach the parent. Thread backing.
#[test]
fn hosted_block_args_thread() {
    let script = r#"
var ok = true;

"* data + block args reach the constructor
var mult = { |x| x * 3 };
var svc = Worker.host:'@runner.qn@' with:{ |n f|
    var r = Runner.new;
    r.tally:n;
    r.stash:f;
    r
} args:#( 21 mult );
((svc.tally:1) == 22).else:{ ok = false; 'FAIL data arg'.print };
((svc.runStash:5) == 15).else:{ ok = false; 'FAIL block arg'.print };
svc.serviceStop;

"* a channel arg is a LIVE endpoint: the hosted object sends on it mid-method
var out = Channel.buffered:8;
var svc2 = Worker.host:'@runner.qn@' with:{ |ch|
    var r = Runner.new;
    r.stash:{ |n| ch.send:n; n };
    r
} args:#( out );
svc2.runStash:42;
((out.receive) == 42).else:{ ok = false; 'FAIL channel arg'.print };
svc2.serviceStop;

"* arity is checked before anything ships
var bad = { Worker.with:{ |a b| #{} } args:#( 1 ); 'spawned' }.catch:{ |e| e.s };
(bad.contains?:'parameter').else:{ ok = false; ('FAIL arity: ' + bad).print };

"* a non-portable arg refuses loudly, naming the element
var t = Task.spawn:{ 1 };
var np = { Worker.with:{ |x| #{} } args:#( t ); 'spawned' }.catch:{ |e| e.s };
(np.contains?:'element 1').else:{ ok = false; ('FAIL non-portable: ' + np).print };
t.join;

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_args_spawn", script, &[("runner.qn", RUNNER_UNIT)]);
}

/// Spawn-time `args:` over PROCESS backing: the same three kinds cross the
/// socket — data and blocks ride the mailbox as wire forms, the channel as a
/// relay endpoint on the chan socket.
#[test]
fn hosted_block_args_process() {
    let script = r#"
var ok = true;
var mult = { |x| x * 3 };
var out = Channel.buffered:8;
var svc = Worker.host:'@runner.qn@' with:{ |n f ch|
    var r = Runner.new;
    r.tally:n;
    r.stash:{ |v| ch.send:(f.value:v); v };
    r
} args:#( 21 mult out ) backing:'process';
((svc.tally:1) == 22).else:{ ok = false; 'FAIL data arg'.print };
svc.runStash:5;
((out.receive) == 15).else:{ ok = false; 'FAIL chan+block args'.print };
svc.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes(
        "block_args_spawn_proc",
        script,
        &[("runner.qn", RUNNER_UNIT)],
    );
}

/// The `start:` family joins the args + backing surface: a parameterized job
/// block takes spawn args (channels included) on thread backing, and plain
/// jobs now run on PROCESS backing too — the block crossing as source, the
/// value coming home through `join`.
#[test]
fn start_block_args_and_process() {
    let script = r#"
var ok = true;

"* a parameterized job: data + channel args, value via join
var out = Channel.buffered:4;
var w = Worker.start:{ |n ch| ch.send:(n * 2); n + 1 } args:#( 20 out );
((out.receive) == 40).else:{ ok = false; 'FAIL chan arg'.print };
((w.join) == 21).else:{ ok = false; 'FAIL join value'.print };

"* a plain job on process backing: source ships, join carries the value home
var p = Worker.start:{ var t = 0; (1..6).each:{ |i| t = t + i }; t } backing:'process';
((p.join) == 15).else:{ ok = false; 'FAIL process job'.print };

"* args over process backing
var p2 = Worker.start:{ |a b| a * b } args:#( 6 7 ) backing:'process';
((p2.join) == 42).else:{ ok = false; 'FAIL process args'.print };

"* a parameterized block without args: names the fix
var miss = { Worker.start:{ |x| x }; 'spawned' }.catch:{ |e| e.s };
(miss.contains?:'start:args:').else:{ ok = false; ('FAIL arity msg: ' + miss).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("start_args", script, &[]);
}
