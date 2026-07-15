//! Process backing (docs/internal/CONCURRENCY_ARCH.md §13.1): the same lanes with a
//! pump + child `qn worker-serve` on the other end. Covers data round-trips,
//! the block refusal in BOTH directions, mixed thread/process psTree,
//! process-backed services, and boot-failure contracts.

use std::process::Command;

fn assert_proc_script_passes(name: &str, script: &str, units: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 3;
    let dir = std::env::temp_dir().join(format!("qn_procw_{name}"));
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
            std::thread::sleep(std::time::Duration::from_millis(200 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("process-worker script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// Echo over the wire; also probes the CHILD-side block refusal (its lane
/// knows it is process-backed) and hosts a THREAD sub-worker so psTree
/// shows a mixed tree through the process boundary.
const PROC_UNIT: &str = r#"
var sub = Worker.start:{ Worker.receive };
var running = true;
{ running }.whileDo:{
    var m = Worker.receive;
    ((m == 'stop') || (m == nil)).if:{ running = false }
    else:{
        (m == 'try-block').if:{
            var r = { Worker.send:{ 1 }; 'sent' }.catch:{ |e| 'refused' };
            Worker.send:r
        }
        else:{
            (m.class.name == 'Integer').if:{ Worker.send:(m * 2) }
            else:{ Worker.send:m }
        }
    }
};
sub.send:0;
sub.join;
"#;

#[test]
fn process_worker_round_trips_and_mixed_ps_tree() {
    let script = r#"
var ok = true;
var w = Worker.spawn:'@proc.qn@' backing:'process';
w.send:21;
((w.receive) == 42).else:{ ok = false };

"* structured data crosses the wire both ways
w.send:#( 1 #{ 'k': 2 } );
var back = w.receive;
(back.class.name == 'List').else:{ ok = false };

"* blocks refuse in BOTH directions: parent send, and child send
var pb = { w.send:{ |x| x }; 'sent' }.catch:{ |e| 'refused' };
(pb == 'refused').else:{ ok = false };
w.send:'try-block';
((w.receive) == 'refused').else:{ ok = false };

"* psTree crosses the boundary: process row with pid, whose subtree holds
"* a THREAD sub-worker — the mixed topology in one call. Deadlines are
"* short by design; poll under cargo-test machine load.
var sub = nil;
var tree = nil;
var tries = 0;
{ (sub == nil) && (tries < 40) }.whileDo:{
    tries = tries + 1;
    tree = VM.psTree;
    var row = (tree.at:'workers').at:0;
    ((row.at:'backing') == 'process').else:{ ok = false };
    ((row.at:'pid') != nil).else:{ ok = false };
    var got = row.at:'ps';
    (got.class.name == 'Map').if:{ sub = got } else:{ Async.sleep:100 }
};
(sub == nil).if:{ ok = false }
else:{
    (((sub.at:'workers').count) == 1).else:{ ok = false };
    (((((sub.at:'workers').at:0).at:'backing')) == 'thread').else:{ ok = false }
};

w.send:'stop';
((w.join) == nil).else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + tree.s).print };
"#;
    assert_proc_script_passes("roundtrip", script, &[("proc.qn", PROC_UNIT)]);
}

const COUNTER_UNIT: &str = r#"
Counter <- { |@total|
    init -> { @total = 0 };
    add: -> { |n| @total = @total + n; @total };
    total -> { @total };
    boom -> { 'kaboom'.throw }
};
"#;

#[test]
fn process_backed_service_state_errors_and_stop() {
    let script = r#"
var ok = true;
var c = Worker.host:'@counter.qn@' with:{ Counter.new } backing:'process';
((c.add:5) == 5).else:{ ok = false };
((c.add:7) == 12).else:{ ok = false };
var thrown = { c.boom; 'no-error' }.catch:{ |e| e.s };
(thrown.contains?:'kaboom').else:{ ok = false };
var mnu = { c.frobnicate; 'no-error' }.catch:{ |e| e.s };
(mnu.contains?:'frobnicate').else:{ ok = false };
c.serviceStop;
var after = { c.total; 'no-error' }.catch:{ |e| 'stopped' };
(after == 'stopped').else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_proc_script_passes("service", script, &[("counter.qn", COUNTER_UNIT)]);
}

#[test]
fn process_boot_failure_contracts() {
    let script = r#"
var ok = true;
"* plain workers: the error surfaces at join, naming the unit
var j = { (Worker.spawn:'/nonexistent/nope.qn' backing:'process').join; 'ran' }
    .catch:{ |e| (e.s.contains?:'nope.qn').if:{ 'named' } else:{ e.s } };
(j == 'named').else:{ ok = false };
"* services handshake at host:, so THEY raise there
var h = { Worker.host:'/nonexistent/nope.qn' with:{ Counter.new } backing:'process'; 'hosted' }
    .catch:{ |e| (e.s.contains?:'nope.qn').if:{ 'named' } else:{ e.s } };
(h == 'named').else:{ ok = false };
"* start: with process backing is REAL now: the block ships as source,
"* runs in a child qn, and join carries its value home
var sb = (Worker.start:{ 1 } backing:'process').join;
(sb == 1).else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL ' + j + '/' + h + '/' + sb.s).print };
"#;
    assert_proc_script_passes("boot", script, &[]);
}
