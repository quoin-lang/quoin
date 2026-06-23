//! Integration test for the Stage 6a `ByteStream` layer: drive the real `qn` binary over
//! a QN script that reads from a Rust-side server which streams a fixed payload in
//! deliberately awkward chunks (the `\r\n` delimiter straddles two underlying reads).
//! Exercises `readUntil:` (incl. across-read delimiter), `peek:`, `readExactly:`, short
//! `read:`/`read`, EOF behaviors, and both constructors (`ByteStream.over:` and
//! `socket.byteStream`). The script decides pass/fail and prints a marker.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Stream "ABCDE\r\nFGHIJK" to the peer in four pieces, with gaps + NODELAY so the pieces
/// land in separate reads — in particular the `\r` (end of piece 2) and `\n` (start of
/// piece 3) arrive separately, so the `\r\n` delimiter straddles two `ByteStream` fills.
/// The script's assertions hold regardless of how TCP actually coalesces the pieces; the
/// chunking just makes the cross-read path the likely one.
fn serve(mut sock: TcpStream) {
    sock.set_nodelay(true).ok();
    let pieces: [&[u8]; 4] = [b"AB", b"CDE\r", b"\nFGHI", b"JK"];
    for p in pieces {
        if sock.write_all(p).is_err() {
            return;
        }
        sock.flush().ok();
        thread::sleep(Duration::from_millis(50));
    }
    // Dropping `sock` closes the connection → the client sees EOF.
}

#[test]
fn byte_stream_buffered_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(sock) = conn {
                thread::spawn(move || serve(sock));
            }
        }
    });

    let script = format!(
        r#"
ok = true;

"* over: consumes the socket; the stream then owns the fd
s = TcpSocket.connect:'127.0.0.1:{port}';
st = ByteStream.over:s;
(s.closed?).else:{{ ok = false }};

"* readUntil: returns through-and-including the delimiter, even though \r\n straddles reads
((st.readUntil:'\r\n').asString == 'ABCDE\r\n').else:{{ ok = false }};

"* peek: looks ahead without consuming; a following readExactly: still sees those bytes
((st.peek:3).asString == 'FGH').else:{{ ok = false }};
((st.readExactly:4).asString == 'FGHI').else:{{ ok = false }};

"* EOF before the delimiter: readUntil: returns the partial remainder
((st.readUntil:'\r\n').asString == 'JK').else:{{ ok = false }};

"* further reads at EOF are empty
((st.read).size == 0).else:{{ ok = false }};
((st.read:5).size == 0).else:{{ ok = false }};

"* readExactly: past EOF throws (catchable). Keyword sends bind looser than ==, so the
"* catch send is parenthesized before the comparison.
((({{ st.readExactly:1 }}.catch:{{ |e| 'threw' }}) == 'threw')).else:{{ ok = false }};

st.close;
(st.closed?).else:{{ ok = false }};

"* the socket.byteStream constructor (separate connection)
s2 = TcpSocket.connect:'127.0.0.1:{port}';
st2 = s2.byteStream;
((st2.readUntil:'\r\n').asString == 'ABCDE\r\n').else:{{ ok = false }};
st2.close;

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_bytestream_test_{port}.qn"));
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
