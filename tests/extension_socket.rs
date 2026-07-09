//! The extension transport's unix-socket path must never outlive the connection it advertises.
//!
//! Both SDKs bind a socket, accept the host's single connection, and then unlink the path: the
//! established connection is unaffected, the protocol never reconnects, and unlinking at accept
//! is the only cleanup that survives a *signal* death of either process. Nothing else can cover
//! that case -- the host's `NativeExtension::drop` runs on graceful exits only, and SIGKILL runs
//! no destructor at all. (`src/worker.rs` has used this idiom since it landed; measured on the
//! dev box before this fix: 0 stray `quoin-worker-*.sock` against 66 stray `quoin-ext-*.sock`.)
//!
//! These tests assert the *absence* of a file, which is the kind of assertion that passes for
//! bad reasons. Two guards: each test first establishes that the extension is connected and
//! answering (so `accept` has certainly returned), and each scopes its search to a path that
//! only it can create -- by pid for the spawned host, by a private prefix for the direct-to-SDK
//! cases -- so a concurrently running test cannot make it pass or fail.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// A socket path only this test process can produce. Deliberately *not* of the form
/// `quoin-ext-<pid>-<n>.sock`, so it can never be picked up by the pid-scoped scan below.
/// `/tmp` rather than `temp_dir()` for the reason `unique_sock_path` gives: on macOS the latter
/// is a long `/var/folders/...` path, and `sun_path` caps a socket address at ~104 bytes.
fn private_sock_path() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("/tmp/qn-sdktest-{}-{n}.sock", std::process::id()))
}

/// Connect to `path`, retrying until the child has bound and listened. Mirrors the host's own
/// retry loop in `spawn_and_connect`.
fn connect_with_retry(path: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(s) = UnixStream::connect(path) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "extension never bound {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Wait for `path` to disappear. The child unlinks *after* `accept` returns, which is strictly
/// after our `connect` succeeds -- so a small wait is expected, an unbounded one is the bug.
fn wait_until_gone(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Kill and reap, so a failing assertion doesn't leave the fixture running.
fn cleanup(mut child: Child, path: &Path) {
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(path);
}

/// Every `quoin-ext-*` socket that a host with this pid could have created. Pid-scoping is what
/// makes these tests safe under nextest, which runs them concurrently with the other extension
/// tests -- each spawned `qn` tags its sockets with its own pid (`unique_sock_path`).
fn sockets_owned_by(pid: u32) -> Vec<PathBuf> {
    let prefix = format!("quoin-ext-{pid}-");
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".sock"))
        })
        .collect()
}

#[test]
fn rust_sdk_unlinks_the_socket_once_the_host_connects() {
    let path = private_sock_path();
    let child = Command::new(env!("CARGO_BIN_EXE_ext_echo"))
        .arg(&path)
        .spawn()
        .expect("spawn ext_echo");

    let _stream = connect_with_retry(&path);
    let gone = wait_until_gone(&path);
    cleanup(child, &path);

    assert!(
        gone,
        "quoin_ext::serve left {} on disk after accepting the host",
        path.display()
    );
}

#[test]
fn python_sdk_unlinks_the_socket_once_the_host_connects() {
    if !python_fixture_runnable() {
        eprintln!("skipping python_sdk_unlinks_the_socket: python3 with `msgpack` unavailable");
        return;
    }
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sdk/python/examples/ext_echo.py"
    );
    let path = private_sock_path();
    // Invoked through `python3` rather than its shebang, so the checkout's file mode is irrelevant.
    let child = Command::new("python3")
        .arg(fixture)
        .arg(&path)
        .spawn()
        .expect("spawn ext_echo.py");

    let _stream = connect_with_retry(&path);
    let gone = wait_until_gone(&path);
    cleanup(child, &path);

    assert!(
        gone,
        "quoin_ext.serve left {} on disk after accepting the host",
        path.display()
    );
}

/// The case that motivated the fix: SIGKILL runs no destructor in either process, so if anything
/// still had to clean up at exit, this would strand a socket. Nothing does.
#[test]
fn a_killed_host_strands_no_socket() {
    let ext_bin = env!("CARGO_BIN_EXE_ext_echo");
    // READY is printed only after a successful round trip, so by the time we read it the child has
    // accepted and answered. `e` stays rooted across the sleep, so the extension is alive -- the
    // socket is gone because it was unlinked, not because the value was collected and dropped.
    let script = format!(
        "var e = Extension.spawn:'{ext_bin}';\n\
         ((e.call:'echo' with:'hi') == 'hi').if:{{ 'READY'.print }};\n\
         Async.sleep:30000\n"
    );
    let script_path = std::env::temp_dir().join(format!("qn_ext_sock_{}.qn", std::process::id()));
    std::fs::write(&script_path, script).unwrap();

    // Both streams are piped, not inherited. Killing the host orphans its `ext_echo` grandchild
    // for the moment it takes to notice the closed peer, and an orphan holding this test binary's
    // stderr is what nextest reports as a leaky test.
    let mut host = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&script_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qn");
    let pid = host.id();

    let mut line = String::new();
    let mut out = BufReader::new(host.stdout.take().expect("qn stdout"));
    out.read_line(&mut line).expect("read READY");

    let while_alive = sockets_owned_by(pid);
    let _ = host.kill();
    let _ = host.wait();
    let after_kill = sockets_owned_by(pid);

    let _ = std::fs::remove_file(&script_path);
    for stray in after_kill.iter().chain(&while_alive) {
        let _ = std::fs::remove_file(stray);
    }

    assert_eq!(line.trim(), "READY", "the extension never answered a call");
    assert!(
        while_alive.is_empty(),
        "a live, connected extension still advertises {while_alive:?}"
    );
    assert!(
        after_kill.is_empty(),
        "SIGKILL on the host stranded {after_kill:?}"
    );
}

/// True if `python3` can import `msgpack` -- the Python SDK's only external dependency.
fn python_fixture_runnable() -> bool {
    Command::new("python3")
        .args(["-c", "import msgpack"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
