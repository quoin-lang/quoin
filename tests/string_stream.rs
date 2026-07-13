//! Integration tests for the Stage 6b `StringStream` layer: drive the real `qn` binary
//! over QN scripts reading from Rust-side servers that stream UTF-8 in pieces deliberately
//! cut mid-code-point and mid-line. Covers `readLine` (decoded lines, `\n`-stripped, nil at
//! EOF) and `read` (the incremental decode — looping + concatenating reproduces the text,
//! which only holds if a code point split across reads is retained rather than
//! corrupted/thrown). Two payloads: 2-byte accents (`café`/`résumé`), and a 4-byte emoji
//! (`😀`, the widest UTF-8 sequence) split across reads plus right-to-left scripts (Arabic,
//! Hebrew). Expected strings are built from the exact wire bytes — a source literal could be
//! a different Unicode normalization and not match (QN string `==` is exact byte compare).

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Stream "café\nrésumé\ntail" in four pieces. Each `é` is C3 A9; the pieces are cut so the
/// C3 ends one piece and the A9 starts the next (a multibyte code point straddling two
/// reads), and so the newlines fall across pieces too. With NODELAY + gaps the pieces tend
/// to land in separate reads; the assertions hold regardless of coalescing.
fn serve(mut sock: TcpStream) {
    sock.set_nodelay(true).ok();
    let pieces: [&[u8]; 4] = [b"caf\xC3", b"\xA9\nr\xC3", b"\xA9sum\xC3", b"\xA9\ntail"];
    for p in pieces {
        if sock.write_all(p).is_err() {
            return;
        }
        sock.flush().ok();
        thread::sleep(Duration::from_millis(50));
    }
    // Dropping `sock` closes the connection → EOF.
}

#[test]
fn string_stream_lines_and_incremental_decode() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            thread::spawn(move || serve(sock));
        }
    });

    let script = format!(
        r#"
var ok = true;

"* readLine: decoded lines with the newline stripped, nil at EOF. The é code points and
"* the line breaks both straddle underlying reads; the buffer reassembles them.
"* Expected values are built from the exact bytes the server sends — a source 'é' literal
"* could be NFC or NFD and so not match the wire bytes (NFC, C3 A9 here). 10 = '\n'.
var cafe = (Bytes.of:#(99 97 102 195 169)).asString;
var resume = (Bytes.of:#(114 195 169 115 117 109 195 169)).asString;
var full = (Bytes.of:#(99 97 102 195 169 10 114 195 169 115 117 109 195 169 10 116 97 105 108)).asString;

var s = TcpSocket.connect:'127.0.0.1:{port}';
var bs = s.byteStream;
var ss = StringStream.over:bs;
(s.closed?).else:{{ ok = false }};      "* socket consumed by byteStream
(bs.closed?).else:{{ ok = false }};     "* byte stream consumed by StringStream.over:
((ss.readLine) == cafe).else:{{ ok = false }};
((ss.readLine) == resume).else:{{ ok = false }};
((ss.readLine) == 'tail').else:{{ ok = false }};   "* final newline-less line
(ss.readLine).defined?.if:{{ ok = false }};        "* nil at EOF
ss.close;

"* read: looping and concatenating the available-text chunks reproduces the whole text,
"* which holds only if a code point split across reads is retained (not corrupted/thrown).
var s2 = TcpSocket.connect:'127.0.0.1:{port}';
var ss2 = s2.stringStream;
var acc = '';
var done = false;
{{ done == false }}.whileDo:{{
    var chunk = ss2.read;
    (chunk.length == 0).if:{{ done = true }} else:{{ acc = acc + chunk }};
}};
(acc == full).else:{{ ok = false }};
ss2.close;

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_stringstream_test_{port}.qn"));
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

/// Stream three lines — "😀ab", Arabic "مرحبا", Hebrew "שלום" — in pieces that split the
/// 4-byte emoji (F0 9F | 98 80) and a 2-byte Arabic letter across reads, and cut lines
/// across reads. The last line has no trailing newline (final-line-at-EOF case).
fn serve_unicode(mut sock: TcpStream) {
    sock.set_nodelay(true).ok();
    let pieces: [&[u8]; 5] = [
        b"\xF0\x9F",                         // first 2 bytes of 😀 (U+1F600)
        b"\x98\x80ab\n",                     // last 2 bytes of 😀, then "ab\n"
        b"\xD9\x85\xD8",                     // مـ + the lead byte of ر
        b"\xB1\xD8\xAD\xD8\xA8\xD8\xA7\n",   // ـرحبا (completing ر) + "\n"
        b"\xD7\xA9\xD7\x9C\xD7\x95\xD7\x9D", // שלום, no trailing newline
    ];
    for p in pieces {
        if sock.write_all(p).is_err() {
            return;
        }
        sock.flush().ok();
        thread::sleep(Duration::from_millis(50));
    }
    // Dropping `sock` closes the connection → EOF.
}

#[test]
fn string_stream_emoji_and_rtl() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            thread::spawn(move || serve_unicode(sock));
        }
    });

    let script = format!(
        r#"
var ok = true;

"* Expected lines, built from the exact wire bytes (10 = '\n'):
"*   emojiLine  = "😀ab"   F0 9F 98 80 'a' 'b'
"*   arabicLine = "مرحبا"  (Arabic, RTL; five 2-byte code points)
"*   hebrewLine = "שלום"   (Hebrew, RTL; four 2-byte code points)
var emojiLine = (Bytes.of:#(240 159 152 128 97 98)).asString;
var arabicLine = (Bytes.of:#(217 133 216 177 216 173 216 168 216 167)).asString;
var hebrewLine = (Bytes.of:#(215 169 215 156 215 149 215 157)).asString;
var fullU = (Bytes.of:#(240 159 152 128 97 98 10 217 133 216 177 216 173 216 168 216 167 10 215 169 215 156 215 149 215 157)).asString;

"* readLine: the 4-byte emoji and an Arabic letter both straddle underlying reads;
"* the buffer reassembles them before decoding.
var s = TcpSocket.connect:'127.0.0.1:{port}';
var ss = StringStream.over:(s.byteStream);
((ss.readLine) == emojiLine).else:{{ ok = false }};
((ss.readLine) == arabicLine).else:{{ ok = false }};
((ss.readLine) == hebrewLine).else:{{ ok = false }};   "* final newline-less line
(ss.readLine).defined?.if:{{ ok = false }};            "* nil at EOF
ss.close;

"* read: concatenating the available-text chunks reproduces the whole payload, which holds
"* only if the emoji split across reads is retained (4-byte sequence) rather than mangled.
var s2 = TcpSocket.connect:'127.0.0.1:{port}';
var ss2 = s2.stringStream;
var acc = '';
var done = false;
{{ done == false }}.whileDo:{{
    var chunk = ss2.read;
    (chunk.length == 0).if:{{ done = true }} else:{{ acc = acc + chunk }};
}};
(acc == fullU).else:{{ ok = false }};
ss2.close;

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_stringstream_uni_test_{port}.qn"));
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
