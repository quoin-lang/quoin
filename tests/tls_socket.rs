//! Integration test for the Stage 4b `TlsSocket` layer: drive the real `qn` binary over
//! a QN script that talks to a Rust-side **TLS** echo server (self-signed cert, accepted
//! via `insecure: true` — the same escape hatch real users get for local debugging). It
//! mirrors `tcp_socket.rs`: a connect/write/read/close round-trip, the `do:` scope form,
//! a `wrap:host:` upgrade of a plaintext `TcpSocket` (which is then consumed/closed), and
//! N concurrent connections overlapping on the scheduler. The script decides pass/fail.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures_rustls::rustls::crypto::ring;
use futures_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use futures_rustls::rustls::{self, ServerConfig};

/// A rustls `ServerConfig` with a fresh self-signed cert for `localhost`. The client
/// trusts it only because the script connects with `insecure: true`.
fn server_config() -> Arc<ServerConfig> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der = cert.cert.der().clone();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    Arc::new(config)
}

/// Echo one connection over TLS using the blocking `rustls::Stream` (it drives the
/// handshake transparently on the first read). Reads in a loop and writes back until the
/// peer closes; a small delay so concurrent connections demonstrably overlap.
fn echo(config: Arc<ServerConfig>, mut tcp: TcpStream) {
    let mut conn = match rustls::ServerConnection::new(config) {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
    let mut buf = [0u8; 4096];
    loop {
        match tls.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                thread::sleep(Duration::from_millis(40));
                if tls.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = tls.flush();
            }
        }
    }
}

#[test]
fn tls_socket_echo_and_concurrency() {
    let config = server_config();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(sock) = conn {
                let config = config.clone();
                thread::spawn(move || echo(config, sock));
            }
        }
    });

    // PASS only if every check matched. Keyword sends bind looser than `==`, so the
    // `catch`/`read` sends are parenthesized before the comparison (as in tcp_socket.rs).
    let script = format!(
        r#"
var ok = true;

"* basic: connect (TLS, cert validation off), write, read, close
var s = TlsSocket.connect:'127.0.0.1:{port}' insecure: true;
s.writeAll:'ping'.asBytes;
((s.read:4).asString == 'ping').else:{{ ok = false }};
s.close;
(s.closed?).else:{{ ok = false }};

"* read after close throws (catchable)
((({{ s.read:1 }}.catch:{{ |e| 'threw' }}) == 'threw')).else:{{ ok = false }};

"* scope form returns the block value and closes
((TlsSocket.connect:'127.0.0.1:{port}' insecure: true do:{{ |sock|
    sock.writeAll:'hi'.asBytes;
    (sock.read:2).asString
}}) == 'hi').else:{{ ok = false }};

"* wrap: upgrade a plaintext TcpSocket -> TLS; the TcpSocket is consumed (closed) and
"* using it afterward throws, while the returned TlsSocket works.
var c = TcpSocket.connect:'127.0.0.1:{port}';
var t = TlsSocket.wrap: c host: 'localhost' insecure: true;
(c.closed?).else:{{ ok = false }};
t.writeAll:'up'.asBytes;
((t.read:2).asString == 'up').else:{{ ok = false }};
((({{ c.read:1 }}.catch:{{ |e| 'threw' }}) == 'threw')).else:{{ ok = false }};
t.close;

"* 8 concurrent TLS connections, each echoes its own message in spawn order
var results = Async.gather:((0..8).collect:{{ |k|
    {{
        var x = TlsSocket.connect:'127.0.0.1:{port}' insecure: true;
        x.writeAll:('m' + k).asBytes;
        var v = (x.read:8).asString;
        x.close;
        v
    }}
}});
(results == #( 'm0' 'm1' 'm2' 'm3' 'm4' 'm5' 'm6' 'm7' )).else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ ('FAIL: ' + results).print }};
"#
    );

    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_tls_test_{port}.qn"));
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
