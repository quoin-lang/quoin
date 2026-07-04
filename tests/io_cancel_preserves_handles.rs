//! Regression test: cancelling a task parked on socket I/O (`Async.timeout:do:`, or
//! `handle.cancel`) must stop the wait — not destroy the socket. The backend performs
//! each op by taking the stream/listener out of its registry and owning it across the
//! await; aborting the op future used to drop that handle, closing the fd behind the
//! program's back: the peer saw a spurious EOF, and every later op on the id failed
//! with "unknown stream id". (Original sighting: an [HTTP]Server could never write its
//! 408 — timing out the head read had silently killed the connection it needed.) Now
//! Read/Write/Accept hold the handle in a drop-guard lease that returns it to the
//! registry even when aborted.

use std::process::Command;

#[test]
fn cancelled_io_leaves_stream_and_listener_usable() {
    let script = r#"
var ok = true;
var listener = TcpListener.listen:'127.0.0.1:0';
var target = '127.0.0.1:' + listener.port.s;

"* A read that times out must leave the stream fully usable: write after it,
"* then read the peer's reply on the same socket.
var t = Task.spawn:{
    var conn = listener.accept;
    var s = ByteStream.over:conn;
    { Async.timeout:60 do:{ s.readUntil:'\r\n' } }
        .catch:{ |e:TimeoutError| nil }
        catch:{ |e| ('FAIL server read err: ' + e.s).print };
    s.writeAll:'after-cancel\r\n'.asBytes;
    var got = (s.readUntil:'\r\n').asString;
    s.close;
    got
};
var c = ByteStream.over:(TcpSocket.connect:target);
var line = (c.readUntil:'\r\n').asStringLossy;
c.writeAll:'pong\r\n'.asBytes;
var served = t.join;
c.close;
(line == 'after-cancel\r\n').else:{ ok = false; ('FAIL client got: ' + line).print };
(served == 'pong\r\n').else:{ ok = false; ('FAIL server got: ' + served.s).print };

"* An accept that times out must leave the LISTENER usable: the next accept on
"* the same listener still serves a client.
{ Async.timeout:60 do:{ listener.accept } }
    .catch:{ |e:TimeoutError| nil }
    catch:{ |e| ok = false; ('FAIL accept err: ' + e.s).print };
var t2 = Task.spawn:{ listener.acceptOnce:{ |conn| conn.writeAll:'ok'.asBytes } };
var c2 = ByteStream.over:(TcpSocket.connect:target);
var again = c2.readAll.asStringLossy;
t2.join;
c2.close;
listener.close;
(again == 'ok').else:{ ok = false; ('FAIL post-cancel accept got: ' + again).print };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

    let dir = std::env::temp_dir();
    let path = dir.join("qn_io_cancel_preserves_handles_test.qn");
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
