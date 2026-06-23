//! Integration test for the Stage 5 `[HTTP]` client: drive the real `qn` binary over a
//! `use std:net/http` script that talks to local Rust HTTP/1.1 servers. Covers a
//! Content-Length GET, a POST body echo, a connection-close-delimited body, and the
//! same over HTTPS via `TlsSocket` with `insecure: true`. The script decides pass/fail.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::thread;

use futures_rustls::rustls::crypto::ring;
use futures_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use futures_rustls::rustls::{self, ServerConfig};

/// Read one HTTP/1.1 request (request line + headers + Content-Length body) and return
/// `(path, body)`. `reader` is a buffered view over the connection.
fn read_request(reader: &mut impl BufRead) -> (String, Vec<u8>) {
    let mut request_line = String::new();
    let _ = reader.read_line(&mut request_line);
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    (path, body)
}

/// The canned response for a given request path. `/close` is Content-Length-less (the
/// body is delimited by the connection close), the others carry Content-Length.
fn response_for(path: &str, req_body: &[u8]) -> Vec<u8> {
    let (head, body): (String, Vec<u8>) = match path {
        "/cl" => (
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\n\r\n".into(),
            b"hello world".to_vec(),
        ),
        "/post" => (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                req_body.len()
            ),
            req_body.to_vec(),
        ),
        "/close" => (
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into(),
            b"closed-body".to_vec(),
        ),
        _ => (
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into(),
            Vec::new(),
        ),
    };
    head.into_bytes().into_iter().chain(body).collect()
}

/// A self-signed rustls `ServerConfig` for `localhost` (the client trusts it only via
/// `insecure: true`).
fn tls_config() -> Arc<ServerConfig> {
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

/// Run `qn` on `script`, returning combined stdout — asserting it contains `PASS`.
fn run_pass(script: &str, tag: &str) {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("qn_http_{tag}.qn"));
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
        "{tag} did not pass.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn http_get_post_and_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let (path, body) = read_request(&mut reader);
                let mut conn = conn;
                let _ = conn.write_all(&response_for(&path, &body));
                let _ = conn.flush();
                // For /close we just drop `conn` (EOF signals the body end).
            });
        }
    });

    let script = format!(
        r#"
use std:net/http;
ok = true;
base = 'http://127.0.0.1:{port}';

"* Content-Length GET
r1 = [HTTP]Client.get: base + '/cl';
(r1.status == 200).else:{{ ok = false }};
(r1.ok?).else:{{ ok = false }};
(r1.bodyText == 'hello world').else:{{ ok = false }};
((r1.header:'CONTENT-TYPE') == 'text/plain').else:{{ ok = false }};

"* POST body echo
r2 = [HTTP]Client.post: base + '/post' body: 'ping-pong'.asBytes;
(r2.bodyText == 'ping-pong').else:{{ ok = false }};

"* connection-close-delimited body (no Content-Length)
r3 = [HTTP]Client.get: base + '/close';
(r3.bodyText == 'closed-body').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    run_pass(&script, &format!("plain_{port}"));
}

#[test]
fn https_get_insecure() {
    let config = tls_config();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let config = config.clone();
            thread::spawn(move || {
                let mut tcp = conn;
                let mut sc = match rustls::ServerConnection::new(config) {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut tls = rustls::Stream::new(&mut sc, &mut tcp);
                let mut reader = BufReader::new(&mut tls);
                let (path, body) = read_request(&mut reader);
                let resp = response_for(&path, &body);
                let _ = tls.write_all(&resp);
                let _ = tls.flush();
            });
        }
    });

    let script = format!(
        r#"
use std:net/http;
ok = true;

"* HTTPS via the Builder with insecure cert validation (local self-signed server)
req = [HTTP]Client.request: 'https://127.0.0.1:{port}/cl';
req.insecure:true;
r = req.send;
(r.status == 200).else:{{ ok = false }};
(r.bodyText == 'hello world').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    run_pass(&script, &format!("https_{port}"));
}

/// The webpki secure (validating) path through the QN client against a real host — so
/// ignored by default. Many public hosts serve `Transfer-Encoding: chunked`, which is a
/// Stage 6 item, so reaching the chunked refusal still proves connect + TLS handshake +
/// head parse worked end to end; a Content-Length host gives a clean 200.
#[test]
#[ignore = "hits the public internet (example.org); run with --ignored"]
fn http_secure_real_host() {
    let script = r#"
use std:net/http;
{ r = [HTTP]Client.get: 'https://example.org/';
  (r.status == 200).if:{ 'PASS'.print } else:{ ('FAIL status ' + r.status).print }
}.catch:{ |e|
    (e.contains?:'chunked').if:{ 'PASS'.print } else:{ ('FAIL ' + e).print }
};
"#;
    run_pass(script, "realhost");
}
