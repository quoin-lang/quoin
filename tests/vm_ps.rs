//! `VM.ps` (and the REPL's `$ps`): a live tree of the scheduler — tasks
//! with park labels and live channel state, fiber depth, gather edges,
//! spawned workers with lane depths, in-flight IO/compute counts.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn assert_script_passes(name: &str, script: &str) {
    const ATTEMPTS: u32 = 4;
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&path)
            .output()
            .expect("run qn");
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
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
    panic!("vm-ps script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

#[test]
fn ps_reports_tasks_channels_gather_and_fibers() {
    let script = r#"
var ok = true;

"* a sender parked on a full channel, a receiver parked on an empty one
var full = Channel.buffered:1;
full.send:9;
Task.spawn:{ full.send:10 };
var empty = Channel.buffered:4;
Task.spawn:{ empty.receive };
"* a sleeper, a gather parent with two children, a task parked INSIDE a fiber
Task.spawn:{ Async.sleep:60000 };
Task.spawn:{ Async.gather:#( { Async.sleep:50000 } { Async.sleep:50000 } ) };
Task.spawn:{ var f = Fiber.new:{ |z| Async.sleep:60000; 0 }; f.resume:0 };
Async.sleep:100;

var ps = VM.ps;
((ps.at:'worker?') == false).else:{ ok = false };
var tasks = ps.at:'tasks';

var sendPark = nil;
var recvPark = nil;
var sleepPark = nil;
var gatherPark = nil;
var fiberPark = nil;
var runningSeen = false;
tasks.each:{ |t|
    ((t.at:'state') == 'running').if:{ runningSeen = true };
    ((t.at:'on') == 'channel send').if:{ sendPark = t };
    ((t.at:'on') == 'channel receive').if:{ recvPark = t };
    ((t.at:'on') == 'io: sleep 60000ms').if:{
        ((t.at:'fibers') > 0).if:{ fiberPark = t } else:{ sleepPark = t }
    };
    ((t.at:'on') == 'gather').if:{ gatherPark = t }
};
runningSeen.else:{ ok = false };

"* live channel state through the park subject
(sendPark == nil).if:{ ok = false }
else:{
    var c = sendPark.at:'channel';
    (((c.at:'cap') == 1) && ((c.at:'buffered') == 1) && ((c.at:'sendWaiters') == 1))
        .else:{ ok = false }
};
(recvPark == nil).if:{ ok = false }
else:{
    var c = recvPark.at:'channel';
    (((c.at:'cap') == 4) && ((c.at:'buffered') == 0) && ((c.at:'recvWaiters') == 1))
        .else:{ ok = false }
};

"* the plain sleeper counts toward in-flight io; the fiber park shows depth
(sleepPark == nil).if:{ ok = false };
(fiberPark == nil).if:{ ok = false }
else:{ ((fiberPark.at:'fibers') == 1).else:{ ok = false } };
(((ps.at:'io').at:'inFlight') >= 3).else:{ ok = false };

"* gather edges both directions
(gatherPark == nil).if:{ ok = false }
else:{
    var kids = gatherPark.at:'awaiting';
    ((kids.count) == 2).else:{ ok = false };
    var parentId = gatherPark.at:'id';
    var edges = 0;
    tasks.each:{ |t|
        ((t.at:'parent') == parentId).if:{ edges = edges + 1 }
    };
    (edges == 2).else:{ ok = false }
};

ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + VM.ps.s).print };
"#;
    assert_script_passes("qn_ps_tree.qn", script);
}

#[test]
fn ps_reports_workers_with_live_lanes() {
    let script = r#"
var ok = true;
var w = Worker.start:{ (Worker.receive) * 2 };
var b = Worker.start:{ 7 };
b.join;

"* Under a loaded machine (the whole cargo suite in parallel) worker boot
"* can lag; poll for the settled picture instead of trusting one sleep.
var running = 0;
var exited = 0;
var tries = 0;
{ ((running == 1) && (exited == 1)) == false && (tries < 100) }.whileDo:{
    Async.sleep:30;
    tries = tries + 1;
    running = 0;
    exited = 0;
    (VM.ps.at:'workers').each:{ |row|
        ((row.at:'unit') == '<block>').else:{ ok = false };
        ((row.at:'state') == 'running').if:{ running = running + 1 };
        ((row.at:'state') == 'exited').if:{ exited = exited + 1 }
    }
};
((running == 1) && (exited == 1)).else:{ ok = false };
((VM.ps.at:'workers').count == 2).else:{ ok = false };

"* feed the live worker; a block worker's value comes back via JOIN
w.send:21;
((w.join) == 42).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + VM.ps.s).print };
"#;
    assert_script_passes("qn_ps_workers.qn", script);
}

/// The REPL's `$ps` renders the same snapshot as a table. Task rows are
/// line-scoped in the REPL (the table is rebuilt per line), but the worker
/// registry persists — spawn on one line, see it on the next.
#[test]
fn repl_ps_renders_table() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("repl")
        .env("QN_NO_BANNER", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"var w = Worker.start:{ Worker.receive };\n$ps\n")
        .unwrap();
    let out = child.wait_with_output().expect("repl run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("TASKS"), "no TASKS header:\n{stdout}");
    assert!(stdout.contains("WORKERS"), "no WORKERS section:\n{stdout}");
    assert!(stdout.contains("<block>"), "no worker row:\n{stdout}");
    assert!(stdout.contains("io in flight:"), "no io summary:\n{stdout}");
}
