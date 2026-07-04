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
