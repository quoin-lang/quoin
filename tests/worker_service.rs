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

"* a non-portable argument (a block writing a capture) refuses without
"* occupying the service
var w = 0;
var badArg = { c.add:{ w = 1 }; 'sent' }.catch:{ |e| 'refused' };
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
Runner <- { |@stash|
    init -> { @stash = nil };
    double: -> { |n| n * 2 };
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

"* a stored block stays live for later dispatches
r.stash:{ |n| n + 1 };
((r.runStash:9) == 10).else:{ ok = false; 'FAIL: stash'.print };

"* an unportable block (writes a capture) refuses at the seam, naming the
"* argument; the service stays usable
var w = 0;
var bad = { r.apply:{ |n| w = n } to:1; 'sent' }.catch:{ |e| e.s };
(bad.contains?:'argument 1').else:{ ok = false; ('FAIL bad: ' + bad).print };
((r.runStash:1) == 2).else:{ ok = false; 'FAIL: usable after refusal'.print };

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
fn service_block_args_process_refuse() {
    let script = r#"
var ok = true;
var r = WorkerService.host:'@runner.qn@' class:'Runner' backing:'process';

"* blocks refuse at the encode seam for process backing, pointing at thread
var msg = { r.apply:{ |n| n } to:1; 'sent' }.catch:{ |e| e.s };
((msg.contains?:'process boundary') && (msg.contains?:'thread backing'))
    .else:{ ok = false; ('FAIL msg: ' + msg).print };

"* data arguments still cross fine afterwards
((r.double:21) == 42).else:{ ok = false; 'FAIL: double'.print };

r.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_service_script_passes("block_args_proc", script, &[("runner.qn", RUNNER_UNIT)]);
}
