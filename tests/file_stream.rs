//! Integration test for the Stage 6d file streams: `[IO]File.byteStream` / `.stringStream`
//! open a file into the async backend (the `OpenFile` op) and return the same `ByteStream`
//! / `StringStream` a socket yields. No network peer needed — write a temp file, then drive
//! the real `qn` binary to stream it back. Expected values are built from the exact file
//! bytes (a source literal could be a different Unicode normalization; QN `==` is exact).

use std::process::Command;

#[test]
fn file_stream_read_lines_and_bytes() {
    let dir = std::env::temp_dir();
    let data_path = dir.join(format!("qn_filestream_data_{}", std::process::id()));
    // "alpha\nbéta\ngamma" — é is C3 A9 (NFC); written as exact bytes (normalization-proof).
    std::fs::write(&data_path, b"alpha\nb\xC3\xA9ta\ngamma").unwrap();

    let script = format!(
        r#"
ok = true;
f = [IO]File.open: '{data}';

"* stringStream: readLine decodes each line, nil at EOF
ss = f.stringStream;
beta = (Bytes.of:#(98 195 169 116 97)).asString;   "* "béta"
((ss.readLine) == 'alpha').else:{{ ok = false }};
((ss.readLine) == beta).else:{{ ok = false }};
((ss.readLine) == 'gamma').else:{{ ok = false }};   "* final newline-less line
(ss.readLine).defined?.if:{{ ok = false }};         "* nil at EOF
ss.close;

"* byteStream on the SAME [IO]File (not consumed -> a fresh fd): readAll = exact bytes
bs = f.byteStream;
full = Bytes.of:#(97 108 112 104 97 10 98 195 169 116 97 10 103 97 109 109 97);
((bs.readAll) == full).else:{{ ok = false }};
bs.close;

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#,
        data = data_path.display()
    );

    let script_path = dir.join(format!("qn_filestream_test_{}.qn", std::process::id()));
    std::fs::write(&script_path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&script_path)
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&data_path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS"),
        "script did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
