//! A tiny out-of-process Quoin extension that *misbehaves on demand*, to exercise Tier-1 crash
//! isolation (Slice 5a) and timeout safety. The VM spawns it with a socket path as argv[1]. Ops:
//!
//! - `ping`  — returns `"pong"` (a normal, healthy call).
//! - `crash` — exits the process (status 7) *mid-call*, before replying, simulating a crash.
//! - `hang`  — never replies (sleeps indefinitely), simulating an unresponsive extension; the host
//!   must time out (via `Async.timeout:do:`) rather than block forever.
//!
//! The host must surface a crash/timeout as a catchable error (not a hang), mark the extension
//! dead, and fail fast on the next call. A test/example fixture, not a shipped feature.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_crash <socket-path>");
    quoin_ext::serve(&path, |_host, op, _arg| match op {
        "ping" => "pong".to_string(),
        // Exit before sending a reply: the host is parked reading the response and sees EOF.
        "crash" => std::process::exit(7),
        // Never reply: block the handler so the host's read parks until it times out. (The child
        // is killed when the host drops the Extension, so this sleep doesn't outlive the test.)
        "hang" => {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            unreachable!()
        }
        other => format!("unknown op: {other}"),
    })
    .expect("ext_crash serve loop");
}
