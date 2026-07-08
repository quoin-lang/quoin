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
var busy = Worker.start:{ var s = 0; (0..30000000).each:{ |i| s = s + i }; s };

"* Workers must BOOT qnlib before their drivers answer control requests —
"* on a loaded CI runner that can far exceed one deadline window, and an
"* honest 'unresponsive' is the designed answer. Poll until the tree is
"* fully formed: the nested subtree (with ITS inner worker answering) and
"* the busy worker caught running mid-loop with a real ps map.
var sub = nil;
var busyOk = false;
var tree = nil;
var tries = 0;
{ ((sub == nil) || (busyOk == false)) && (tries < 120) }.whileDo:{
    tries = tries + 1;
    tree = VM.psTree;
    (tree.at:'workers').each:{ |r|
        ((r.at:'unit') == '<block>').if:{
            (((r.at:'state') == 'running')
                && (((r.at:'ps')).class.name == 'Map')).if:{ busyOk = true }
        }
        else:{
            var got = r.at:'ps';
            (got.class.name == 'Map').if:{
                var forming = got.at:'workers';
                ((forming.count == 1)
                    && ((((forming.at:0).at:'ps')).class.name == 'Map'))
                    .if:{ sub = got }
            }
        }
    };
    ((sub == nil) || (busyOk == false)).if:{ Async.sleep:100 }
};

var rows = tree.at:'workers';
(rows.count == 2).else:{ ok = false };
busyOk.else:{ ok = false };
(sub == nil).if:{ ok = false }
else:{
    ((sub.at:'worker?') == true).else:{ ok = false };
    var parked = false;
    (sub.at:'tasks').each:{ |t| ((t.at:'on') == 'worker receive').if:{ parked = true } };
    parked.else:{ ok = false }
};

"* labels ride the rows
((((rows.at:0).at:'label')) == (((rows.at:0).at:'unit'))).else:{ ok = false };

nested.send:nil;
nested.join;
"* busy is deliberately NOT joined: it only needs to outlive the poll
"* window above (even on a slow CI runner), and process exit reaps it —
"* joining would serialize the test on the whole 30M-iteration loop.
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
        .write_all(
            b"var w = Worker.start:{ Worker.receive };\n\
              var i = 0;\n\
              { (i < 120) && (((((VM.psTree.at:'workers').at:0).at:'ps')).class.name != 'Map') }.whileDo:{ i = i + 1; Async.sleep:100 };\n\
              $ps tree\n",
        )
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
