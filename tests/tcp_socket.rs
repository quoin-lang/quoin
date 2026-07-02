//! Integration test for the Stage 3b `TcpSocket` layer: drive the real `qn` binary
//! over a QN script that talks to a Rust-side echo server. Covers a basic
//! connect/write/read/close round-trip and N concurrent connections overlapping on
//! the scheduler. The script itself decides pass/fail and prints a marker.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Echo one connection: read in a loop and write back until the peer closes.
fn echo(mut sock: TcpStream) {
    let mut buf = [0u8; 4096];
    loop {
        match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A small delay so concurrent connections demonstrably overlap.
                thread::sleep(Duration::from_millis(40));
                if sock.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
}

#[test]
fn tcp_socket_echo_and_concurrency() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(sock) = conn {
                thread::spawn(move || echo(sock));
            }
        }
    });

    // A QN script that exercises the socket API and prints PASS only if everything
    // matched. `Test.is:` is avoided to keep the script self-contained (no framework).
    let script = format!(
        r#"
var ok = true;

"* basic: connect, write, read, close
var s2 = TcpSocket.connect:'127.0.0.1:{port}';
s2.writeAll:'ping'.asBytes;
((s2.read:4).asString == 'ping').else:{{ ok = false }};
s2.close;
(s2.closed?).else:{{ ok = false }};

"* read after close throws (catchable). Keyword sends bind looser than `==`, so the
"* catch send is parenthesized before the comparison.
((({{ s2.read:1 }}.catch:{{ |e| 'threw' }}) == 'threw')).else:{{ ok = false }};

"* scope form returns the block value and closes
((TcpSocket.connect:'127.0.0.1:{port}' do:{{ |sock|
    sock.writeAll:'hi'.asBytes;
    (sock.read:2).asString
}}) == 'hi').else:{{ ok = false }};

"* 8 concurrent connections, each echoes its own message in spawn order
var results = Async.gather:((0..8).collect:{{ |k|
    {{
        var c = TcpSocket.connect:'127.0.0.1:{port}';
        c.writeAll:('m' + k).asBytes;
        var v = (c.read:8).asString;
        c.close;
        v
    }}
}});
(results == #( 'm0' 'm1' 'm2' 'm3' 'm4' 'm5' 'm6' 'm7' )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ ('FAIL: ' + results).print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_tcp_test_{port}.qn"));
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
