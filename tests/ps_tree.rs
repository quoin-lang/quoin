//! `VM.psTree` + the §13.3 control lane: recursive ps across nested
//! workers, answered opportunistically by each worker's driver — including
//! workers that are BUSY computing (batch-boundary staleness, no
//! preemption) and workers idle-parked on their lanes (the race in the
//! driver's reactor wait).

use std::io::Write as _;
use std::process::{Command, Stdio};

fn assert_ps_script_passes(name: &str, script: &str, units: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let dir = std::env::temp_dir().join(format!("qn_pstree_{name}"));
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
    panic!("ps-tree script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

const NEST_UNIT: &str = r#"
"* hosts one sub-worker parked on its inbox, then waits for a stop message
var inner = Worker.start:{ Worker.receive };
var stop = Worker.receive;
inner.send:0;
inner.join;
"#;

#[test]
fn ps_tree_recurses_and_reads_busy_and_idle_workers() {
    let script = r#"
var ok = true;
var nested = Worker.spawn:'@nest.qn@';
var busy = Worker.start:{ var s = 0; (0..1500000).each:{ |i| s = s + i }; s };
Async.sleep:300;

var tree = VM.psTree;
var rows = tree.at:'workers';
(rows.count == 2).else:{ ok = false };

var nestedRow = nil;
var busyRow = nil;
rows.each:{ |r|
    ((r.at:'unit') == '<block>').if:{ busyRow = r } else:{ nestedRow = r }
};

"* the idle nested worker answers: its main task is parked on its inbox,
"* and ITS OWN sub-worker appears one level deeper with its own ps
(nestedRow == nil).if:{ ok = false }
else:{
    ((nestedRow.at:'backing') == 'thread').else:{ ok = false };
    var sub = nestedRow.at:'ps';
    (sub.class.name == 'Map').else:{ ok = false };
    ((sub.at:'worker?') == true).else:{ ok = false };
    var parked = false;
    (sub.at:'tasks').each:{ |t| ((t.at:'on') == 'worker receive').if:{ parked = true } };
    parked.else:{ ok = false };
    var subWorkers = sub.at:'workers';
    (subWorkers.count == 1).else:{ ok = false };
    var inner = (subWorkers.at:0).at:'ps';
    (inner.class.name == 'Map').else:{ ok = false }
};

"* the BUSY worker (mid 60M-iteration loop) still answers between batches
(busyRow == nil).if:{ ok = false }
else:{
    ((busyRow.at:'state') == 'running').else:{ ok = false };
    ((busyRow.at:'ps').class.name == 'Map').else:{ ok = false }
};

"* labels ride the rows
((((rows.at:0).at:'label')) == (((rows.at:0).at:'unit'))).else:{ ok = false };

nested.send:nil;
nested.join;
busy.join;
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + tree.s).print };
"#;
    assert_ps_script_passes("recurse", script, &[("nest.qn", NEST_UNIT)]);
}

#[test]
fn ps_tree_marks_exited_workers_without_hanging() {
    let script = r#"
var ok = true;
var b = Worker.start:{ 7 };
b.join;
var tree = VM.psTree;
var rows = tree.at:'workers';
(rows.count == 1).else:{ ok = false };
(((rows.at:0).at:'state') == 'exited').else:{ ok = false };
"* an exited worker is skipped, not awaited: no 250ms stall per corpse —
"* and its 'ps' reads unresponsive rather than hanging the caller
(((rows.at:0).at:'ps') == 'unresponsive').else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + tree.s).print };
"#;
    assert_ps_script_passes("exited", script, &[]);
}

#[test]
fn repl_ps_tree_renders_nested() {
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
        .write_all(b"var w = Worker.start:{ Worker.receive };\nAsync.sleep:400;\n$ps tree\n")
        .unwrap();
    let out = child.wait_with_output().expect("repl run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("'workers'"), "no workers key:\n{stdout}");
    assert!(stdout.contains("'ps'"), "no nested ps in tree:\n{stdout}");
    assert!(
        stdout.contains("worker receive"),
        "no child park label:\n{stdout}"
    );
}
