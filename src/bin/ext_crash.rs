//! A tiny out-of-process Quoin extension that *crashes on demand*, to exercise Tier-1 crash
//! isolation (Slice 5a; see `tests/extension.rs`). The VM spawns it with a socket path as
//! argv[1]. It serves two ops:
//!
//! - `ping`  — returns `"pong"` (a normal, healthy call).
//! - `crash` — exits the process (status 7) *mid-call*, before replying, simulating a crash.
//!
//! The host must surface the crash as a catchable error (not a hang), mark the extension dead,
//! and fail fast on the next call. It is a test/example fixture, not a shipped feature.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_crash <socket-path>");
    quoin_ext::serve(&path, |_host, op, _arg| match op {
        "ping" => "pong".to_string(),
        // Exit before sending a reply: the host is parked reading the response and sees EOF.
        "crash" => std::process::exit(7),
        other => format!("unknown op: {other}"),
    })
    .expect("ext_crash serve loop");
}
