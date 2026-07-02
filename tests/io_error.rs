//! Integration test for structured I/O errors (`IoError`). Socket/stream/file failures
//! that used to throw a plain string now throw a typed Quoin `IoError` carrying a `kind`
//! symbol, so a handler can branch on the cause (`e.kind == #connectionRefused`) and
//! catch by type (`IoError ~ e`). Drives the real `qn` binary over a script that catches
//! each failure and inspects the caught value. Covers three native kinds end to end —
//! `#notFound` (missing file), `#connectionRefused` (dead port), `#closed` (closed
//! handle) — plus base-type matching and a `IoError` thrown from Quoin.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;

/// Echo one connection until the peer closes — just enough of a live peer that the script
/// can open a stream, close it, and then provoke a `#closed` error on the next op.
fn echo(mut sock: TcpStream) {
    let mut buf = [0u8; 4096];
    loop {
        match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if sock.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
}

#[test]
fn io_errors_are_typed_with_kinds() {
    // A live echo server (for the closed-handle case).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(sock) = conn {
                thread::spawn(move || echo(sock));
            }
        }
    });

    // A port that is bound then immediately dropped — connecting to it is refused.
    let refused = TcpListener::bind("127.0.0.1:0").unwrap();
    let refused_port = refused.local_addr().unwrap().port();
    drop(refused);

    let script = format!(
        r#"
var ok = true;

"* A missing file surfaces a typed IoError carrying kind #notFound.
var e1 = {{ [IO]File.open:'/no/such/quoin-file-xyz' }}.catch:{{ |e| e }};
(IoError ~ e1).else:{{ ok = false }};
(e1.kind == #notFound).else:{{ ok = false }};

"* Connecting to a closed port: a typed IoError, kind #connectionRefused.
var e2 = {{ TcpSocket.connect:'127.0.0.1:{refused_port}' }}.catch:{{ |e| e }};
(IoError ~ e2).else:{{ ok = false }};
(e2.kind == #connectionRefused).else:{{ ok = false }};

"* Operating on a closed stream throws IoError kind #closed. byteStream consumes the
"* socket; after close, the next read finds a closed handle.
var sock = TcpSocket.connect:'127.0.0.1:{port}';
var bs = sock.byteStream;
bs.close;
var e3 = {{ bs.read }}.catch:{{ |e| e }};
(IoError ~ e3).else:{{ ok = false }};
(e3.kind == #closed).else:{{ ok = false }};

"* An IoError is-an Error (base-type match) and exposes a non-empty message.
(Error ~ e3).else:{{ ok = false }};
(e3.message.length > 0).else:{{ ok = false }};

"* An IoError thrown from Quoin: typed and catchable; the bare throw: leaves kind nil.
var e4 = {{ IoError.throw:'manual' }}.catch:{{ |e| e }};
(IoError ~ e4).else:{{ ok = false }};
(e4.kind == nil).else:{{ ok = false }};
(e4.message == 'manual').else:{{ ok = false }};

"* ...and throw:kind: carries an explicit kind symbol from Quoin.
var e5 = {{ IoError.throw:'slow' kind:#timedOut }}.catch:{{ |e| e }};
(e5.kind == #timedOut).else:{{ ok = false }};
(e5.message == 'slow').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_io_error_test_{port}.qn"));
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
