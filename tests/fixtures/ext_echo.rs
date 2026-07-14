//! A tiny out-of-process Quoin extension used to exercise the Tier-1 transport
//! (see `tests/extension.rs`). The VM spawns it with a socket path as argv[1]; it
//! serves three ops over the `quoin-ext` SDK: `echo` (returns the arg), `upper`
//! (uppercases it), and `slow` (sleeps ~150ms then echoes — a deliberately
//! long-running call for the fair-queuing tests). A test/example fixture, not a
//! shipped feature.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_echo <socket-path>");
    // Scalar-only: doesn't touch the host-callback client (`_host`).
    quoin_ext::serve(&path, |_host, op, arg| match op {
        "echo" => arg.to_string(),
        "upper" => arg.to_uppercase(),
        "slow" => {
            std::thread::sleep(std::time::Duration::from_millis(150));
            arg.to_string()
        }
        other => format!("unknown op: {other}"),
    })
    .expect("ext_echo serve loop");
}
