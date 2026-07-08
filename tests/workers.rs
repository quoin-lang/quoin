//! C2 v1 worker isolates (docs/CONCURRENCY_ARCH.md §5): spawn-by-unit-path,
//! deep-copy message lanes, join with error transparency — and the L2
//! property: a worker wait IS a parked task, so `Async.gather:` and
//! `Async.timeout:do:` compose over it with no new vocabulary.

use std::process::Command;

/// Write `units` (name → source) into a temp dir, substitute each
/// `@name@` in `script` with the unit's absolute path, run, expect PASS.
fn assert_worker_script_passes(name: &str, script: &str, units: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let dir = std::env::temp_dir().join(format!("qn_workers_{name}"));
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
    panic!("worker script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

const ECHO_UNIT: &str = r#"
"* Echo worker: double integers, upcase strings, echo structures verbatim;
"* stop on 'stop' or a closed inbox.
(Worker.worker?).else:{ Worker.send:'NOT-A-WORKER' };
var running = true;
{ running }.whileDo:{
    var m = Worker.receive;
    ((m == 'stop') || (m == nil)).if:{ running = false }
    else:{
        (m.class.name == 'Integer').if:{ Worker.send:(m * 2) }
        else:{
            (m.class.name == 'String').if:{ Worker.send:(m.upper) }
            else:{ Worker.send:m }
        }
    }
};
"#;

#[test]
fn worker_echo_round_trips_and_join() {
    let script = r#"
var ok = true;
(Worker.worker? == false).else:{ ok = false };
var w = Worker.spawn:'@echo.qn@';
w.send:21;
((w.receive) == 42).else:{ ok = false };
w.send:'quoin';
((w.receive) == 'QUOIN').else:{ ok = false };
"* structured data round-trips by deep copy
w.send:#( 1 'two' #( 3 ) );
var back = w.receive;
((((back.at:2).at:0) == 3)).else:{ ok = false };
w.send:#{ 'k': 7 };
(((w.receive).at:'k') == 7).else:{ ok = false };
w.send:'stop';
((w.join) == nil).else:{ ok = false };
var st = VM.stats.at:'workers';
(((st.at:'spawned') >= 1) && ((st.at:'messages') >= 8)).else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_worker_script_passes("echo", script, &[("echo.qn", ECHO_UNIT)]);
}

#[test]
fn worker_errors_surface_to_join_catchable() {
    let bad_unit = r#"
'boom from the worker'.throw;
"#;
    let unparsable_unit = r#"
this is not ( valid quoin
"#;
    let script = r#"
var ok = true;
"* a runtime error in the unit rides the done lane
var w1 = Worker.spawn:'@bad.qn@';
var got = { w1.join; 'no-error' }.catch:{ |e| e.s };
(got.contains?:'boom').else:{ ok = false };
"* a parse error too
var w2 = Worker.spawn:'@unparsable.qn@';
var got2 = { w2.join; 'no-error' }.catch:{ |e| e.s };
(got2.contains?:'parse error').else:{ ok = false };
"* a missing unit file too
var w3 = Worker.spawn:'/nonexistent/nope.qn';
var got3 = { w3.join; 'no-error' }.catch:{ |e| e.s };
(got3 == 'no-error').if:{ ok = false };
"* double join is a clear error
var got4 = { w1.join; 'no-error' }.catch:{ |e| e.s };
(got4.contains?:'already joined').else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL ' + got + ' / ' + got2).print };
"#;
    assert_worker_script_passes(
        "errors",
        script,
        &[("bad.qn", bad_unit), ("unparsable.qn", unparsable_unit)],
    );
}

/// The L2 property: worker waits are parked tasks, so the existing async
/// combinators compose — gather over two workers runs their waits
/// concurrently, and a timeout around a never-answering worker fires
/// instead of hanging.
#[test]
fn worker_waits_compose_with_async_combinators() {
    let silent_unit = r#"
"* Never sends; parks on an inbox nobody writes to.
Worker.receive;
"#;
    let script = r#"
var ok = true;
var w1 = Worker.spawn:'@echo.qn@';
var w2 = Worker.spawn:'@echo.qn@';
w1.send:100;
w2.send:'abc';
var outs = Async.gather:#( { w1.receive } { w2.receive } );
(outs == #( 200 'ABC' )).else:{ ok = false };
w1.send:'stop'; w2.send:'stop';
w1.join; w2.join;
"* timeout over a worker that never answers
var s = Worker.spawn:'@silent.qn@';
var timedOut = { Async.timeout:100 do:{ s.receive; 'answered' } }.catch:{ |e| 'timed-out' };
(timedOut == 'timed-out').else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ ('FAIL: ' + outs.s + ' ' + timedOut).print };
"#;
    assert_worker_script_passes(
        "compose",
        script,
        &[("echo.qn", ECHO_UNIT), ("silent.qn", silent_unit)],
    );
}

#[test]
fn worker_message_taxonomy() {
    // Since L3, BLOCKS cross the lanes as portable-block messages (the
    // combinator enabler) — the echo worker bounces one back and the parent
    // runs the rebuilt closure. Native-state instances (here: the worker
    // handle itself) still refuse at the send seam, catchable, worker
    // unharmed.
    let script = r#"
var ok = true;
var w = Worker.spawn:'@echo.qn@';
"* a native-state instance refuses at the send seam
var refused = { w.send:w; 'sent' }.catch:{ |e| 'refused' };
(refused == 'refused').else:{ ok = false };
"* a block crosses, echoes back, and runs on this side
w.send:{ |x| x * 7 };
var back = w.receive;
((back.valueWithSelfOrArg:6) == 42).else:{ ok = false };
w.send:5;
((w.receive) == 10).else:{ ok = false };
w.send:'stop';
w.join;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_worker_script_passes("taxonomy", script, &[("echo.qn", ECHO_UNIT)]);
}

#[test]
fn worker_receive_after_exit_answers_nil() {
    let one_shot = r#"
Worker.send:'only';
"#;
    let script = r#"
var ok = true;
var w = Worker.spawn:'@oneshot.qn@';
((w.receive) == 'only').else:{ ok = false };
w.join;
"* the worker is gone and the lane is drained: nil, not a hang
((w.receive) == nil).else:{ ok = false };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_worker_script_passes("drained", script, &[("oneshot.qn", one_shot)]);
}
