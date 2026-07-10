//! `[IO]Stdin` — reading the process's standard input. These spawn `qn` with a pipe (or a file)
//! on stdin, because a test running under `qn test` inherits the harness's stdin and cannot
//! provide one; the cases that need no input live in `qnlib/tests/59-io-stdin.qn`.
//!
//! Two properties are load-bearing and easy to regress:
//!   * reads **park** the task rather than freezing the single-threaded scheduler, and
//!   * the stream is **memoized** — a stream buffers, so a second stream over fd 0 would hold
//!     bytes the first never sees.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `qn` (with `args`) feeding `input` on stdin; return trimmed stdout.
fn run_with_stdin(args: &[&str], input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    // Dropping the pipe signals EOF, which `eachLine:` / `readAll` need to terminate.
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("qn exits");
    assert!(
        out.status.success(),
        "qn failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn eval_with_stdin(expr: &str, input: &str) -> String {
    run_with_stdin(&["-e", expr], input)
}

#[test]
fn read_line_returns_one_line_without_its_terminator() {
    assert_eq!(
        eval_with_stdin("([IO]Stdin.readLine).pp", "alpha\nbeta\n"),
        "'alpha'"
    );
}

#[test]
fn read_line_at_end_of_input_is_nil() {
    assert_eq!(eval_with_stdin("([IO]Stdin.readLine).pp", ""), "nil");
}

#[test]
fn read_all_returns_everything_remaining() {
    assert_eq!(
        eval_with_stdin("([IO]Stdin.readAll).pp", "p\nq\n"),
        r"'p\nq\n'"
    );
}

#[test]
fn each_line_is_the_filter_idiom() {
    // Trailing `; nil` only because `qn -e` prints its expression's value, and `eachLine:`
    // returns the stream so it can be chained. In a script file nothing extra is printed.
    let out = eval_with_stdin(
        "[IO]Stdin.eachLine:{ |l| l.upper.print }; nil",
        "one\ntwo\nthree\n",
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ["ONE", "TWO", "THREE"]);
}

/// The shape a real filter program takes, run as a script rather than `-e`.
#[test]
fn a_filter_script_reads_stdin_and_writes_stdout() {
    let path = std::env::temp_dir().join(format!("qn_filter_{}.qn", std::process::id()));
    std::fs::write(
        &path,
        "var needle = Runtime.arguments.first;\n\
         [IO]Stdin.eachLine:{ |line| (line.contains?:needle).if:{ line.print } }\n",
    )
    .unwrap();
    let out = run_with_stdin(
        &[path.to_str().unwrap(), "an"],
        "apple\nbanana\ncherry\nmango\n",
    );
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.lines().collect::<Vec<_>>(), ["banana", "mango"]);
}

#[test]
fn the_stream_is_memoized_so_no_input_is_lost() {
    // Three reads through two *different* expressions. A fresh stream per call would leave `b`
    // and `c` buffered in a stream nobody reads again, and print `a` then EOF.
    let out = eval_with_stdin(
        "([IO]Stdin.readLine).print; \
         (([IO]Handle.stdin).stringStream.readLine).print; \
         ([IO]Stdin.readLine).print",
        "a\nb\nc\n",
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ["a", "b", "c"]);
}

#[test]
fn the_text_and_byte_views_cannot_both_be_opened() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["-e", "[IO]Stdin.readLine; [IO]Stdin.byteStream"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn");
    child.stdin.as_mut().unwrap().write_all(b"x\n").unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("qn exits");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("already open as a StringStream"),
        "expected the mixed-view refusal, got: {err}"
    );
}

#[test]
fn stdin_may_be_a_file_redirect_not_only_a_pipe() {
    // The reason stdin is backed by `blocking::Unblock` and not `async_io::Async`: a regular
    // file is not pollable, so `qn app.qn < file` would fail under an `Async` registration.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_stdin_{}.txt", std::process::id()));
    std::fs::write(&path, "from-a-file\n").unwrap();
    let file = std::fs::File::open(&path).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args(["-e", "([IO]Stdin.readLine).print"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::from(file))
        .output()
        .expect("run qn");
    let _ = std::fs::remove_file(&path);

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-a-file");
}

#[test]
fn a_read_parks_the_task_rather_than_freezing_the_scheduler() {
    // The property: a task parked on stdin must not freeze the single-threaded scheduler —
    // the spawned task's TICK must print while the main task is waiting on a line.
    //
    // Event-driven, not clock-driven. An earlier version slept a fixed 700ms before writing
    // LINE and lost that race on a loaded CI runner: the child took longer than that to boot,
    // so LINE was already buffered when the VM first ran, the read never parked, and the
    // output was ["LINE", "TICK"]. Here the writer releases LINE only after TICK has been
    // OBSERVED — and TICK can only arrive while the read is parked, so a regression that
    // froze the scheduler shows up as "TICK never arrived" at the reader deadline rather
    // than as a wrong order or a wedged CI job.
    use std::io::BufRead;
    use std::sync::mpsc;
    use std::time::Duration;

    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .args([
            "-e",
            "var t = Task.spawn:{ Async.sleep:100; 'TICK'.print }; \
             ([IO]Stdin.readLine).print; t.join",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn qn");

    // Lines arrive through a channel so each wait can carry a deadline; a blocking read
    // would turn a scheduler freeze into a hang.
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut next_line = |what: &str, child: &mut std::process::Child| -> String {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(line) => line,
            Err(_) => {
                let _ = child.kill();
                panic!("{what}");
            }
        }
    };

    let first = next_line(
        "TICK never arrived: the parked read froze the scheduler",
        &mut child,
    );

    // Only now — with the sleeping task provably finished and the main task parked on the
    // read — release the line.
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"LINE\n").unwrap();
    drop(stdin);

    let second = next_line("LINE was never echoed after it was written", &mut child);
    let status = child.wait().expect("qn exits");
    reader.join().expect("reader thread");

    assert!(status.success());
    assert_eq!(
        (first.as_str(), second.as_str()),
        ("TICK", "LINE"),
        "the sleeping task must finish while the read is parked"
    );
}
