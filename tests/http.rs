//! Integration test for the `[HTTP]` client: drive the real `qn` binary over a
//! `use std:net/http` script that talks to local Rust HTTP/1.1 servers. Covers a
//! Content-Length GET, a POST body echo, a connection-close-delimited body, a chunked
//! transfer-encoding body (Stage 6c), and a Content-Length GET over HTTPS via `TlsSocket`
//! with `insecure: true`. The script decides pass/fail.

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
/// body is delimited by the connection close), `/chunked` is `Transfer-Encoding: chunked`,
/// the others carry Content-Length.
fn response_for(path: &str, req_body: &[u8]) -> Vec<u8> {
    if path == "/chunked" {
        // "Hello, world!" as two chunks ("Hello, " = 0x7, "world!" = 0x6) + the 0 terminator.
        return b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                 7\r\nHello, \r\n6\r\nworld!\r\n0\r\n\r\n"
            .to_vec();
    }
    if path == "/chunked-ext" {
        // The first chunk carries a chunk extension (sig=abc); the second is plain.
        return b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                 7;sig=abc\r\nHello, \r\n6\r\nworld!\r\n0\r\n\r\n"
            .to_vec();
    }
    if path == "/gzip-chunked" {
        // gzip body split across two transfer-chunks: chunked(gzip(entity)). The client must
        // de-chunk first, reassemble the gzip stream, then content-decode it as a whole.
        let body = quoin::runtime::compress::gzip_encode(b"hello gzip world").unwrap();
        let mid = body.len() / 2;
        let mut out =
            b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n\r\n"
                .to_vec();
        out.extend_from_slice(format!("{:x}\r\n", mid).as_bytes());
        out.extend_from_slice(&body[..mid]);
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(format!("{:x}\r\n", body.len() - mid).as_bytes());
        out.extend_from_slice(&body[mid..]);
        out.extend_from_slice(b"\r\n0\r\n\r\n");
        return out;
    }
    if path == "/truncated" {
        // Promises 10 bytes, delivers 5; the connection then closes (thread ends →
        // drop). The client must surface unexpectedEof, not a silent short 200 body.
        return b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhello".to_vec();
    }
    if path == "/chunked-eof" {
        // Chunked framing cut off after the first complete chunk (no next size line).
        return b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nHello, \r\n"
            .to_vec();
    }
    if path == "/redirect" {
        // 302 to a root-relative target on the same server.
        return b"HTTP/1.1 302 Found\r\nLocation: /cl\r\nContent-Length: 0\r\n\r\n".to_vec();
    }
    if path == "/redirect-loop" {
        return b"HTTP/1.1 302 Found\r\nLocation: /redirect-loop\r\nContent-Length: 0\r\n\r\n"
            .to_vec();
    }
    if path == "/redirect-307" {
        // 307 preserves method + body, re-POSTing to the echo endpoint.
        return b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /post\r\nContent-Length: 0\r\n\r\n"
            .to_vec();
    }
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
        "/json" => {
            let body = br#"{"hello":"world","n":7}"#.to_vec();
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                ),
                body,
            )
        }
        "/gzip" => {
            // Compress live with our own encoder so the client decodes what we produced.
            let body = quoin::runtime::compress::gzip_encode(b"hello gzip world").unwrap();
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                ),
                body,
            )
        }
        "/zstd" => {
            // A zstd frame of "hello zstd world" — no pure-Rust zstd compressor, so it is
            // precomputed; the client decodes it via ruzstd.
            let body = vec![
                0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x58, 0x81, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f,
                0x20, 0x7a, 0x73, 0x74, 0x64, 0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x7f, 0x81, 0x68,
                0x60,
            ];
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Encoding: zstd\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                ),
                body,
            )
        }
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
var ok = true;
var base = 'http://127.0.0.1:{port}';

"* Content-Length GET
var r1 = [HTTP]Client.get: base + '/cl';
(r1.status == 200).else:{{ ok = false }};
(r1.ok?).else:{{ ok = false }};
(r1.body.text == 'hello world').else:{{ ok = false }};
((r1.header:'CONTENT-TYPE') == 'text/plain').else:{{ ok = false }};

"* POST body echo
var r2 = [HTTP]Client.post: base + '/post' body: 'ping-pong'.asBytes;
(r2.body.text == 'ping-pong').else:{{ ok = false }};

"* connection-close-delimited body (no Content-Length)
var r3 = [HTTP]Client.get: base + '/close';
(r3.body.text == 'closed-body').else:{{ ok = false }};

"* chunked transfer-encoding, drained to one String
var r4 = [HTTP]Client.get: base + '/chunked';
(r4.status == 200).else:{{ ok = false }};
(r4.body.text == 'Hello, world!').else:{{ ok = false }};

"* the same response, streamed lazily: each chunk is an [HTTP]Body; boundaries preserved
var rs = [HTTP]Client.get: base + '/chunked';
var parts = rs.body.chunks.collect:{{ |c| c.text }};
(parts == #( 'Hello, ' 'world!' )).else:{{ ok = false }};

"* per-chunk metadata: a chunk extension surfaces on the chunk body's .meta
var rx = [HTTP]Client.get: base + '/chunked-ext';
var xs = rx.body.chunks.list;
(((xs.at:0).meta:'sig') == 'abc').else:{{ ok = false }};
((xs.at:0).text == 'Hello, ').else:{{ ok = false }};
(((xs.at:1).meta) == #{{}}).else:{{ ok = false }};

"* gzip Content-Encoding (transparently decoded)
var r5 = [HTTP]Client.get: base + '/gzip';
(r5.body.text == 'hello gzip world').else:{{ ok = false }};

"* streaming a content-encoded body: .chunks can't decode a transfer-chunk in isolation,
"* so it drains+decodes the whole entity and yields a single decoded chunk
var r5b = [HTTP]Client.get: base + '/gzip';
((r5b.body.chunks.collect:{{ |c| c.text }}) == #( 'hello gzip world' )).else:{{ ok = false }};

"* gzip delivered across multiple transfer-chunks: de-chunk, reassemble, then decode
var r5c = [HTTP]Client.get: base + '/gzip-chunked';
(r5c.body.text == 'hello gzip world').else:{{ ok = false }};
var r5d = [HTTP]Client.get: base + '/gzip-chunked';
((r5d.body.chunks.collect:{{ |c| c.text }}) == #( 'hello gzip world' )).else:{{ ok = false }};

"* zstd Content-Encoding (transparently decoded)
var r6 = [HTTP]Client.get: base + '/zstd';
(r6.body.text == 'hello zstd world').else:{{ ok = false }};

"* JSON response: .body.json parses, .json? reflects the Content-Type
var r7 = [HTTP]Client.get: base + '/json';
(r7.body.json?).else:{{ ok = false }};
((r7.body.json.at:'hello') == 'world').else:{{ ok = false }};
((r7.body.json.at:'n') == 7).else:{{ ok = false }};

"* POST of a Map auto-encodes to JSON (the echo server returns the bytes we sent)
var r8 = [HTTP]Client.post: base + '/post' body: #{{ 'k':1 'v':2 }};
(r8.body.text == '{{"k":1,"v":2}}').else:{{ ok = false }};

"* redirects: a 302 is followed by default to its (root-relative) Location
var r9 = [HTTP]Client.get: base + '/redirect';
((r9.status == 200) && (r9.body.text == 'hello world')).else:{{ ok = false }};

"* following can be turned off in the builder — the 3xx comes back as-is
var r10 = (([HTTP]Client.request: base + '/redirect').followRedirects:false).send;
((r10.status == 302) && r10.redirect?).else:{{ ok = false }};

"* a 307 preserves the method and body (re-POSTed to the echo endpoint)
var r11 = [HTTP]Client.post: base + '/redirect-307' body: 'keepme'.asBytes;
(r11.body.text == 'keepme').else:{{ ok = false }};

"* a redirect loop trips the max-redirects cap and throws
var caught = false;
{{ [HTTP]Client.get: base + '/redirect-loop' }}.catch:{{ |e| caught = true }};
(caught).else:{{ ok = false }};

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
var ok = true;

"* HTTPS via the Builder with insecure cert validation (local self-signed server)
var req = [HTTP]Client.request: 'https://127.0.0.1:{port}/cl';
req.insecure:true;
var r = req.send;
(r.status == 200).else:{{ ok = false }};
(r.body.text == 'hello world').else:{{ ok = false }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    run_pass(&script, &format!("https_{port}"));
}

/// The webpki secure (validating) path through the QN client against a real host — so
/// ignored by default. Exercises the full stack end to end: DNS + connect + TLS handshake
/// with real cert validation + head parse + body framing. Public hosts typically serve
/// `Transfer-Encoding: chunked`, which now decodes (Stage 6c), so this asserts a real
/// 200 with a non-empty body.
#[test]
#[ignore = "hits the public internet (example.org); run with --ignored"]
fn http_secure_real_host() {
    let script = r#"
use std:net/http;
var r = [HTTP]Client.get: 'https://example.org/';
var ok = (r.status == 200) && (r.body.bytes.size > 0);
ok.if:{ 'PASS'.print } else:{ ('FAIL status ' + r.status + ' size ' + r.body.bytes.size).print };
"#;
    run_pass(script, "realhost");
}

#[test]
fn truncated_bodies_error_instead_of_silent_success() {
    // Regression (audit): an EOF before the promised Content-Length used to be
    // treated as normal completion — status 200, short body, no error; a chunked
    // body cut at a chunk boundary surfaced as a misleading hex ValueError. Both
    // must be typed IoErrors with kind #unexpectedEof.
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
            });
        }
    });

    let script = format!(
        r#"
use std:net/http;
var ok = true;
var base = 'http://127.0.0.1:{port}';

"* Content-Length promises 10 bytes; the server sends 5 and closes.
var r1 = [HTTP]Client.get: base + '/truncated';
{{ r1.body.text; ok = false; 'FAIL: truncated body read as success'.print }}
    .catch:{{ |e:IoError| (e.kind == #unexpectedEof).else:{{ ok = false; ('FAIL kind1: ' + e.kind.s).print }} }}
    catch:{{ |e| ok = false; ('FAIL class1: ' + e.s).print }};

"* Chunked framing cut off between chunks.
var r2 = [HTTP]Client.get: base + '/chunked-eof';
{{ r2.body.text; ok = false; 'FAIL: chunked-eof body read as success'.print }}
    .catch:{{ |e:IoError| (e.kind == #unexpectedEof).else:{{ ok = false; ('FAIL kind2: ' + e.kind.s).print }} }}
    catch:{{ |e| ok = false; ('FAIL class2: ' + e.s).print }};

ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    run_pass(&script, "truncated");
}
