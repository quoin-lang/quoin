//! [Web]Pool — multi-core request serving (docs/internal/WEB_ARCH.md workers): the
//! same-unit provisioning model over real HTTP, both backings. Requests are
//! raw HTTP/1.1 over TcpStream (no client-side qn), so this pins the whole
//! path: parse -> lanes -> worker pipeline -> lanes -> serialize.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const APP: &str = r#"
use std:net/http;
use std:net/http_server;
use std:web/*;

var backing = '@BACKING@';
var app = [Web]App.new;
app.debug:true;
var hits = #{ 'n': 0 };
app.get:'/hello' do:{ 'hi' };
app.get:'/users/:id' do:{ |req| #{ 'id': (req.param:'id') } };
app.post:'/echo' do:{ |req| req.json };
app.get:'/count' do:{ hits.at:'n' put:((hits.at:'n') + 1); #{ 'n': (hits.at:'n') } };
app.get:'/boom' do:{ 'kaboom'.throw };
app.get:'/stream'
do:{ [Web]Response.stream:(Generator.from:{ ^>'a'; ^>'bb'; ^>'ccc' }) contentType:'text/plain' };
app.get:'/big' do:{
    [Web]Response.stream:(Generator.from:{
        var i = 0;
        { i < 200 }.whileDo:{ ^> 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; i = i + 1 }
    })
};
app.get:'/slow' do:{
    [Web]Response.stream:(Generator.from:{ ^>'s1'; Async.sleep:300; ^>'s2'; Async.sleep:300; ^>'s3' })
};

(Worker.worker?).if:{ app.serve:'127.0.0.1:0' workers:@N@ backing:backing }
else:{
    var server = app.start:'127.0.0.1:0' workers:@N@ backing:backing;
    ('PORT=' + server.port.s).print;
    server.join
}
"#;

struct App {
    child: Child,
    port: u16,
    dir: std::path::PathBuf,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start_app(name: &str, backing: &str) -> App {
    let dir = std::env::temp_dir().join(format!("qn_webworkers_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.qn");
    let (backing, n) = match backing.split_once(':') {
        Some((b, n)) => (b, n),
        None => (backing, "2"),
    };
    std::fs::write(&path, APP.replace("@BACKING@", backing).replace("@N@", n)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn app");
    // Read stdout until the PORT line (workers boot first; give CI time).
    let mut out = child.stdout.take().expect("stdout");
    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let port = loop {
        let mut byte = [0u8; 1];
        match out.read(&mut byte) {
            Ok(1) => {
                buf.push(byte[0]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(line) = text.lines().find(|l| l.starts_with("PORT="))
                    && text.contains('\n')
                {
                    break line[5..].trim().parse::<u16>().expect("port number");
                }
            }
            _ => panic!("app exited before printing PORT"),
        }
        assert!(Instant::now() < deadline, "no PORT line within 30s");
    };
    App { child, port, dir }
}

fn http(port: u16, request: &str) -> String {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    sock.write_all(request.as_bytes()).unwrap();
    let mut resp = String::new();
    sock.read_to_string(&mut resp).expect("read response");
    resp
}

fn get(port: u16, path: &str) -> String {
    http(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    )
}

fn body_of(resp: &str) -> &str {
    resp.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn exercise(app: &App, backing: &str) {
    let hello = get(app.port, "/hello");
    assert!(hello.starts_with("HTTP/1.1 200"), "hello:\n{hello}");
    assert_eq!(body_of(&hello), "hi");

    let param = get(app.port, "/users/42");
    assert_eq!(body_of(&param), r#"{"id":"42"}"#);

    let echo = http(
        app.port,
        "POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
         Content-Length: 7\r\nConnection: close\r\n\r\n{\"a\":1}",
    );
    assert_eq!(body_of(&echo), r#"{"a":1}"#, "echo:\n{echo}");

    // Round-robin + share-nothing: two workers each hold their OWN hits
    // map, so four requests land as 1,1,2,2 (order-independent check).
    let mut counts: Vec<String> = (0..4)
        .map(|_| body_of(&get(app.port, "/count")).to_string())
        .collect();
    counts.sort();
    assert_eq!(
        counts,
        vec![r#"{"n":1}"#, r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":2}"#],
        "per-worker state not sharded"
    );

    // A handler throw in a worker maps to 500 in the transport, and the
    // pool survives it.
    let boom = get(app.port, "/boom");
    assert!(boom.starts_with("HTTP/1.1 500"), "boom:\n{boom}");
    assert!(get(app.port, "/hello").starts_with("HTTP/1.1 200"));

    let missing = get(app.port, "/nope");
    assert!(missing.starts_with("HTTP/1.1 404"), "404:\n{missing}");

    // The debug observability route answers from the TRANSPORT VM: whole
    // topology, pool workers labeled and carrying live subtrees.
    let ps = get(app.port, "/_qn/ps");
    let ps_body = body_of(&ps);
    assert!(ps_body.contains("\"web:0\""), "ps labels:\n{ps_body}");
    assert!(ps_body.contains("\"web:1\""));
    assert!(
        ps_body.contains(&format!("\"backing\":\"{backing}\"")),
        "ps backing:\n{ps_body}"
    );
}

fn dechunk(body: &str) -> String {
    // Minimal HTTP/1.1 chunked decoder for test assertions.
    let mut out = String::new();
    let mut rest = body;
    while let Some(nl) = rest.find("\r\n") {
        let size = usize::from_str_radix(rest[..nl].trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.push_str(&rest[nl + 2..nl + 2 + size]);
        rest = &rest[nl + 2 + size + 2..];
    }
    out
}

fn exercise_streaming(app: &App) {
    // Streams stay CHUNKED through the lanes: head frame + a chunk frame
    // per yield + end frame, rebuilt as a Generator body in the transport.
    let stream = get(app.port, "/stream");
    assert!(
        stream.to_lowercase().contains("transfer-encoding: chunked"),
        "not chunked through the pool:\n{stream}"
    );
    assert_eq!(dechunk(body_of(&stream)), "abbccc");

    // Many frames, one body.
    let big = get(app.port, "/big");
    assert_eq!(dechunk(body_of(&big)).len(), 200 * 32, "big stream");

    // A slow stream must not block the pool: while /slow trickles (~600ms),
    // other requests keep completing through the same workers.
    let port = app.port;
    let slow = std::thread::spawn(move || get(port, "/slow"));
    let t0 = Instant::now();
    let hello = get(app.port, "/hello");
    assert!(hello.starts_with("HTTP/1.1 200"));
    assert!(
        t0.elapsed() < Duration::from_millis(2000),
        "request starved behind a slow stream"
    );
    let slow = slow.join().expect("slow stream thread");
    assert_eq!(dechunk(body_of(&slow)), "s1s2s3");
}

#[test]
fn workers_zero_is_the_single_vm_path() {
    // The caret lint's first catch: `(n < 1).if:{ ^.start:address }` FELL
    // THROUGH (block-return, value discarded) and built a zero-worker pool
    // that 503'd everything. workers:0 must serve single-VM: requests
    // succeed, one shared state (counts increment monotonically), no pool
    // rows in ps.
    let app = start_app("zero", "thread:0");
    let hello = get(app.port, "/hello");
    assert!(hello.starts_with("HTTP/1.1 200"), "hello:\n{hello}");
    let counts: Vec<String> = (0..3)
        .map(|_| body_of(&get(app.port, "/count")).to_string())
        .collect();
    assert_eq!(
        counts,
        vec![r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#],
        "single-VM state not shared"
    );
    let ps = get(app.port, "/_qn/ps");
    assert!(
        !body_of(&ps).contains("\"web:0\""),
        "workers:0 spawned a pool:\n{ps}"
    );
}

#[test]
fn thread_pool_serves_and_shards() {
    let app = start_app("thread", "thread");
    exercise(&app, "thread");
    exercise_streaming(&app);
}

#[test]
fn process_pool_serves_and_shards() {
    let app = start_app("process", "process");
    exercise(&app, "process");
    exercise_streaming(&app);
}
