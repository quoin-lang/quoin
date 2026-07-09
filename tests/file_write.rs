//! Two properties of `[IO]File.create:` that only a real process can show.
//!
//! * The **exit flush**: a buffered write stream the program never closed must still reach the
//!   disk. C does this at `exit()`; a GC finaliser cannot, because a `Drop` may not perform
//!   async I/O. The driver flushes after the program ends — including after `Runtime.exit:`
//!   and after an uncaught error.
//! * **Sockets stay write-through.** Only file streams buffer; buffering a socket would stall
//!   `[HTTP]Server`, which writes a response and then waits for the client.
//!
//! In-process cases (buffer visibility, `flush!`, `close`, the filesystem ops) live in
//! `qnlib/tests/61-file-write.qn`.

use std::path::PathBuf;
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("qn_fw_{}_{}", name, std::process::id()))
}

/// Run a script through `qn` from the package root (so the `qnlib/` prelude resolves).
fn run(name: &str, src: &str) -> Output {
    let path = std::env::temp_dir().join(format!("qn_fw_{}_{}.qn", name, std::process::id()));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);
    out
}

fn read(p: &PathBuf) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[test]
fn an_unclosed_stream_is_flushed_when_the_program_ends() {
    let out_path = tmp("noclose.txt");
    let _ = std::fs::remove_file(&out_path);
    let out = run(
        "noclose",
        &format!(
            "var s = ([IO]File.create:'{}').stringStream;\n\
             s.writeln:'survived';\n",
            out_path.display()
        ),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let contents = read(&out_path);
    let _ = std::fs::remove_file(&out_path);
    assert_eq!(
        contents, "survived\n",
        "a buffered stream the program never closed lost its contents"
    );
}

#[test]
fn the_exit_flush_survives_runtime_exit() {
    let out_path = tmp("exit.txt");
    let _ = std::fs::remove_file(&out_path);
    let out = run(
        "exit",
        &format!(
            "var s = ([IO]File.create:'{}').stringStream;\n\
             s.write:'before exit';\n\
             Runtime.exit:3;\n",
            out_path.display()
        ),
    );
    assert_eq!(out.status.code(), Some(3), "Runtime.exit: status lost");
    let contents = read(&out_path);
    let _ = std::fs::remove_file(&out_path);
    assert_eq!(
        contents, "before exit",
        "Runtime.exit: skipped the exit flush"
    );
}

/// An uncaught error still ends the program; the bytes written before it should not vanish.
#[test]
fn the_exit_flush_survives_an_uncaught_error() {
    let out_path = tmp("raise.txt");
    let _ = std::fs::remove_file(&out_path);
    let out = run(
        "raise",
        &format!(
            "var s = ([IO]File.create:'{}').stringStream;\n\
             s.write:'partial';\n\
             ValueError.throw:'boom';\n",
            out_path.display()
        ),
    );
    assert_eq!(out.status.code(), Some(1), "an uncaught error must exit 1");
    let contents = read(&out_path);
    let _ = std::fs::remove_file(&out_path);
    assert_eq!(
        contents, "partial",
        "an aborted program lost its buffered writes"
    );
}

/// A closed stream is flushed once, not twice: the exit pass must skip it.
#[test]
fn a_closed_stream_is_not_flushed_again_at_exit() {
    let out_path = tmp("once.txt");
    let _ = std::fs::remove_file(&out_path);
    let out = run(
        "once",
        &format!(
            "var s = ([IO]File.create:'{}').stringStream;\n\
             s.write:'once';\n\
             s.close;\n",
            out_path.display()
        ),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let contents = read(&out_path);
    let _ = std::fs::remove_file(&out_path);
    assert_eq!(
        contents, "once",
        "the exit flush wrote a closed stream's bytes twice"
    );
}

/// Sockets must NOT buffer: a server writes a response and then waits for the client, so a
/// write has to reach the peer without an explicit flush. If a socket stream buffered, the
/// client's read would never see the reply and the timeout would fire.
#[test]
fn socket_writes_are_not_buffered() {
    let out = run(
        "socket",
        "var listener = TcpListener.listen:'127.0.0.1:0';\n\
         var target = '127.0.0.1:' + listener.port.s;\n\
         var results = Async.timeout:5000 do:{\n\
         \x20   Async.gather:#(\n\
         \x20     { listener.acceptOnce:{ |conn|\n\
         \x20         var cs = conn.stringStream;\n\
         \x20         var line = cs.readLine;\n\
         \x20         cs.writeln:('echo: ' + line) }; 'served' }\n\
         \x20     { var c = TcpSocket.connect:target;\n\
         \x20       var ss = c.stringStream;\n\
         \x20       ss.writeln:'ping';\n\
         \x20       var reply = ss.readLine; ss.close; reply }\n\
         \x20   )\n\
         };\n\
         listener.close;\n\
         (results.at:1).print;\n",
    );
    assert!(
        out.status.success(),
        "socket round trip failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "echo: ping",
        "a socket write did not reach the peer without an explicit flush"
    );
}

/// `flush!` exists on a write-through stream too, so the same code runs over a file and a
/// socket. It must be a no-op, not an error.
#[test]
fn flush_on_a_socket_stream_is_a_no_op() {
    let out = run(
        "flushsock",
        "var listener = TcpListener.listen:'127.0.0.1:0';\n\
         var target = '127.0.0.1:' + listener.port.s;\n\
         Async.timeout:5000 do:{\n\
         \x20   Async.gather:#(\n\
         \x20     { listener.acceptOnce:{ |conn| conn.close }; nil }\n\
         \x20     { var c = TcpSocket.connect:target;\n\
         \x20       var ss = c.stringStream;\n\
         \x20       ss.flush!;\n\
         \x20       ss.close; nil }\n\
         \x20   )\n\
         };\n\
         listener.close;\n\
         'ok'.print;\n",
    );
    assert!(
        out.status.success(),
        "flush! on a socket stream raised: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}

/// The REPL drives (and so flushes) once per line, but its arena persists across lines. A
/// stream opened on one line and written on the next must still reach disk: the exit flush
/// therefore drains buffers without *untracking* streams that are still open.
#[test]
fn a_stream_written_across_repl_lines_is_flushed() {
    use std::io::Write;
    use std::process::Stdio;

    let out_path = tmp("repl.txt");
    let _ = std::fs::remove_file(&out_path);

    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg("repl")
        .env("NO_COLOR", "1")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn repl");
    {
        let mut stdin = child.stdin.take().expect("repl stdin");
        // Opened on one line...
        writeln!(
            stdin,
            "var s = ([IO]File.create:'{}').stringStream",
            out_path.display()
        )
        .unwrap();
        // ...written on the next. Dropping stdin ends the session.
        writeln!(stdin, "s.write:'across lines'").unwrap();
    }
    let out = child.wait_with_output().expect("wait qn repl");
    assert!(
        out.status.success(),
        "repl failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let contents = read(&out_path);
    let _ = std::fs::remove_file(&out_path);
    assert_eq!(
        contents, "across lines",
        "a stream opened on an earlier REPL line lost its buffered writes"
    );
}
