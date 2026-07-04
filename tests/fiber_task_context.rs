//! Regression test: a task that parks on I/O *inside* a guest fiber must not have
//! its root context clobbered by another task's fiber switches. While a fiber runs,
//! the task-root frames sit in the scheduler's `main_saved_*` slot; that slot is
//! shared, so `save_task_context`/`load_task_context` must carry it per task. When
//! they didn't, two tasks concurrently inside fiber execution (here: two Generators
//! whose blocks read from sockets, fed with delays so their mid-fiber parks overlap)
//! corrupted each other — surfacing as "I/O resumed without a result", or a task
//! completing silently with a foreign/empty root context (an HTTP client draining a
//! chunked response while the server task streamed one was the original sighting).

use std::process::Command;

#[test]
fn concurrent_fiber_io_tasks_keep_their_root_contexts() {
    let script = r#"
var listener = TcpListener.listen:'127.0.0.1:0';
var target = '127.0.0.1:' + listener.port.s;

"* Dribble bytes to two connections with sleeps in between, forcing both readers
"* to park inside their generator fibers at overlapping times.
var feeder = Task.spawn:{
    var a = listener.accept;
    var b = listener.accept;
    a.writeAll:'11'.asBytes;
    Async.sleep:30;
    b.writeAll:'22'.asBytes;
    Async.sleep:30;
    a.writeAll:'33'.asBytes;
    Async.sleep:30;
    b.writeAll:'44'.asBytes;
    a.close;
    b.close
};

var t1 = Task.spawn:{
    var s = ByteStream.over:(TcpSocket.connect:target);
    var g = Generator.from:{ ^>(s.readExactly:2).asString; ^>(s.readExactly:2).asString };
    var out = '';
    g.each:{ |c| out = out + c };
    s.close;
    out
};

var s2 = ByteStream.over:(TcpSocket.connect:target);
var g2 = Generator.from:{ ^>(s2.readExactly:2).asString; ^>(s2.readExactly:2).asString };
var out2 = '';
g2.each:{ |c| out2 = out2 + c };
s2.close;

var one = t1.join;
feeder.join;
listener.close;
"* Which reader got connection a vs b depends on accept order — either way each
"* must see ITS OWN two chunks intact.
var straight = (out2 == '1133') && (one == '2244');
var swapped = (out2 == '2244') && (one == '1133');
(straight || swapped).if:{ 'PASS'.print } else:{
    ('FAIL main=' + out2 + ' t1=' + one).print
};
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_fiber_task_context_test.qn");
    std::fs::write(&path, script).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
