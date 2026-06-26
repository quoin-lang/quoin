//! A tiny out-of-process Quoin extension used to exercise the Tier-1 transport
//! (see `tests/extension.rs`). The VM spawns it with a socket path as argv[1]; it
//! serves two ops over the `quoin-ext` SDK: `echo` (returns the arg) and `upper`
//! (uppercases it). It is a test/example fixture, not a shipped feature.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_echo <socket-path>");
    quoin_ext::serve(&path, |op, arg| match op {
        "echo" => arg.to_string(),
        "upper" => arg.to_uppercase(),
        other => format!("unknown op: {other}"),
    })
    .expect("ext_echo serve loop");
}
