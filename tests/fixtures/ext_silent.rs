//! A test-fixture extension that binds and accepts the socket but never speaks the
//! protocol — it does NOT answer the spawn-time `GetManifest` handshake. Used to prove
//! the host bounds the handshake read with a timeout (a silent extension must fail the
//! spawn, not park the VM forever) rather than going through `quoin_ext::serve`, which
//! would answer the handshake for us. A test fixture, not a shipped feature.

use std::os::unix::net::UnixListener;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ext_silent <socket-path>");
    let listener = UnixListener::bind(&path).expect("bind unix socket");
    // Accept the host's connection, then go silent forever. The host is killed/dropped
    // by the test once its handshake times out, which tears this process down.
    let _conn = listener.accept().expect("accept");
    std::thread::sleep(std::time::Duration::from_secs(3600));
}
