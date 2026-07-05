//! Regression test: a closed socket's fd must actually close before the scheduler
//! parks on the reactor. `ByteStream.close`/socket close only *enqueue* the fd on
//! `socket_reap`; the driver used to drain that queue every 10 driver steps and
//! nowhere else, so a program that closed a connection and then went idle could
//! deadlock — the peer waits for EOF, EOF needs the reap, and the reap needs a
//! driver step that never comes. Whether it struck depended on `step_count % 10`
//! at the moment of idling, which step-batching (QN_BATCH) made coarse enough to
//! miss routinely. The script sweeps the phase with a growing number of
//! sleep-parks per round, so at least one round lands on the bad residue without
//! the fix (each timing out after 8s); with the reap flushed before every reactor
//! park they all finish instantly.

use std::process::Command;

#[test]
fn close_reaches_peer_when_scheduler_idles() {
    let script = r#"
var ok = true;
var round = 1;
{ round <= 10 }.whileDo:{
    var listener = TcpListener.listen:'127.0.0.1:0';
    var target = '127.0.0.1:' + listener.port.s;
    var results = nil;
    { results = Async.timeout:8000 do:{
        Async.gather:#(
            {
                "* phase padding: `round` sleep-parks shift step_count before the close
                var pad = 0;
                { pad < round }.whileDo:{ Async.sleep:1; pad = pad + 1 };
                listener.acceptOnce:{ |conn| conn.writeAll:'x'.asBytes };
                'served'
            }
            {
                var c = TcpSocket.connect:target;
                "* readAll returns only on EOF — i.e. only once the server-side close
                "* is truly reaped, not merely enqueued.
                var got = (ByteStream.over:c).readAll;
                got.asString
            }
        )
    } }.catch:{ |e| ok = false; ('round ' + round + ': ' + e.s).print };
    listener.close;
    results.defined?.if:{
        ((results.at:1) == 'x').else:{ ok = false; ('round ' + round + ' bad data').print }
    };
    round = round + 1
};
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_socket_close_reap_test.qn");
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

#[test]
fn listener_close_releases_the_port() {
    // Regression: the sync reap-path close only removed from the `streams`
    // registry, never `listeners` — every `TcpListener.close` leaked the bound
    // socket, so the port stayed unavailable and the OS backlog kept accepting.
    let script = r#"
var ok = true;
var l = TcpListener.listen:'127.0.0.1:0';
var port = l.port;
l.close;
Async.sleep:50;

{ var l2 = TcpListener.listen:('127.0.0.1:' + port); l2.close }
    .catch:{ |e| ok = false; ('FAIL rebind: ' + e.s).print };

{ var c = TcpSocket.connect:('127.0.0.1:' + port);
  ok = false; 'FAIL: connect to closed listener succeeded'.print;
  c.close }
    .catch:{ |_| nil };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let dir = std::env::temp_dir();
    let path = dir.join("qn_listener_close_releases_port.qn");
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
fn close_wakes_a_parked_reader_and_closes_the_fd() {
    // Regression: while a read leased the stream out of the registry, `close`
    // was a silent no-op — the parked reader hung until the peer acted, and the
    // lease re-inserted the fd afterwards, resurrecting a closed handle. Now the
    // close tombstones the lease (fd drops) and aborts the in-flight op, so the
    // reader wakes with a catchable error and the peer sees EOF.
    let script = r#"
var ok = true;
var listener = TcpListener.listen:'127.0.0.1:0';
var target = '127.0.0.1:' + listener.port;
var peerSawEof = #();
Task.spawn:{
    var c = listener.accept;
    "* the peer's read must end (EOF/reset) once the client fd actually closes
    { var d = c.read:10; (d.count == 0).if:{ peerSawEof.add:1 } }
        .catch:{ |_| peerSawEof.add:1 };
    c.close
};

var sock = TcpSocket.connect:target;
var entered = #();
var readerWoke = #();
var reader = Task.spawn:{
    entered.add:1;
    { sock.read:10; nil }.catch:{ |e| readerWoke.add:(e.s) }
};
{ entered.count == 0 }.whileDo:{ Async.sleep:1 };
Async.sleep:30;

sock.close;
{ Async.timeout:1500 do:{ reader.join } }.catch:{ |e|
    ok = false; 'FAIL: reader still parked after close'.print };
(readerWoke.count == 1).else:{ ok = false; 'FAIL: reader did not get an error'.print };

{ Async.timeout:1500 do:{ { peerSawEof.count == 0 }.whileDo:{ Async.sleep:5 } } }
    .catch:{ |e| ok = false; 'FAIL: peer never saw EOF (fd not closed)'.print };

listener.close;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let dir = std::env::temp_dir();
    let path = dir.join("qn_close_wakes_parked_reader.qn");
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
fn closing_the_listener_ends_the_accept_loop_cleanly() {
    // Regression (audit): one accept error silently killed the accept task while
    // the server object looked healthy. With close now aborting a parked accept
    // (see close_wakes_a_parked_reader_and_closes_the_fd), the loop must treat
    // the #closed error as clean shutdown — join returns instead of surfacing an
    // uncaught IoError or hanging.
    let script = r#"
var srv = TcpServer.new:{ var address = '127.0.0.1:0' };
srv.start:{ |conn| conn.read:10 };
Async.sleep:20;
srv.close;
Async.timeout:3000 do:{ srv.join };
'PASS'.print;
"#;
    let dir = std::env::temp_dir();
    let path = dir.join("qn_listener_close_ends_accept.qn");
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

#[test]
fn tcp_server_task_registry_stays_bounded() {
    // Contract change (audit finding 13): TcpServer.join is now a drain barrier, not a
    // result collector, so the accept loop sweeps finished handlers and the task
    // registry stays bounded by live concurrency instead of growing one entry per
    // connection ever served. 30 sequential connections must leave `connections` small.
    let script = r#"
var ok = true;
var srv = TcpServer.new:{ var address = '127.0.0.1:0' };
srv.start:{ |conn| conn.writeAll:(conn.read:4); conn.close };
var tgt = '127.0.0.1:' + srv.port.s;

var n = 0;
{ n < 30 }.whileDo:{
    var c = TcpSocket.connect:tgt;
    c.writeAll:'ping'.asBytes;
    ((c.read:4).asString == 'ping').else:{ ok = false; 'FAIL: bad echo'.print };
    c.close;
    n = n + 1
};

"* Swept per accept: bounded by concurrency (~1 here), NOT the 30 served.
(srv.connections <= 5).else:{ ok = false; ('FAIL: registry grew to ' + srv.connections.s).print };

srv.stop;
"* Drain must return promptly (join is a barrier, no hang, no result collection).
Async.timeout:3000 do:{ srv.join } onCancel:{ ok = false; 'FAIL: join hung'.print };
srv.close;

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let dir = std::env::temp_dir();
    let path = dir.join("qn_tcp_server_bounded.qn");
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
