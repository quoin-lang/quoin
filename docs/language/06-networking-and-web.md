# Part VI — Networking & the web

Sockets and listeners, the stream layer over them, the pure-Quoin HTTP client and
server, and the `[Web]` application framework — whose core is a function you can
call without any network at all.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · **Networking & the web** · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · [Packages](10-packages.md) · [Appendices](11-appendices.md)

---

## 22. The I/O model

Everything in this chapter rides Part V's concurrency system: all I/O is
**asynchronous by construction**. A read, write, accept, or sleep that cannot
proceed **parks the current task**, and the cooperative scheduler runs other
tasks meanwhile — one API, no blocking/non-blocking split, so thousands of idle
connections multiplex over one program. Tasks and their handles, `Async.gather:`,
`Async.timeout:do:`, and cancellation are Part V's subject (§18–19); a CPU-bound
handler round-robins with its siblings at scheduler boundaries rather than
starving them (§18), but real multi-core parallelism is worker isolates (§21, or
`serve:workers:` in §27). Sockets, listeners, and streams are **native
built-ins** — always available, no `use`. The HTTP layers are **pure Quoin** in
the stdlib: `use std:net/http` loads the `[HTTP]` client, `use std:net/http_server`
the `[HTTP]Server` transport, `use std:web/*` the `[Web]` framework, and
`use std:net/*` both net layers at once.

Every runnable example in this chapter uses one shape, borrowed from the VM's
own test suite: bind `127.0.0.1:0` (the OS picks a free port), run server and
client as concurrent tasks under `Async.gather:`, and wrap the lot in
`Async.timeout:` as a hang guard — nothing below touches a non-loopback
network:

```quoin
var results = Async.timeout:5000 do:{
    Async.gather:#(
        { Async.sleep:10; 'slow' }
        { 'fast' }
    )
};
results        "* -> #(slow fast)
```

The two blocks run as concurrent tasks: the total wait is roughly the *slowest*
task, not the sum, and results come back in input order.

---

## 23. Sockets & streams

> **Rules**
> - `TcpSocket.connect:'host:port'` dials out (DNS included) and answers a byte-level socket: `read:n` (up to *n* Bytes — **empty Bytes means EOF**), `readAll` (until the peer closes), `writeAll:` (complete-or-throw), `close`, `closed?`. `TcpSocket.connect:target do:{ |sock| … }` scopes it: the socket closes on **every** exit path (normal, throw, cancel) and the block's value is answered.
> - `TcpListener.listen:'host:port'` binds; **port `0` picks an ephemeral port** — read it back with `port`. `accept` parks until a peer connects, answering a `TcpSocket` the caller owns; `acceptOnce:{ |conn| … }` accepts one and closes it after the block; `acceptLoop:{ |conn| … }` serves forever — it ends only via non-local return (`^^`), a throw, or task cancellation. `close` releases the port.
> - `TlsSocket.connect:` is the same surface, encrypted (the certificate is checked against the host name); `TlsSocket.wrap:tcp host:name` upgrades a plaintext socket in place (STARTTLS), consuming it. The `insecure:true` variants skip certificate validation — local debugging only.
> - Streams layer buffering, framing, and text on top. `socket.byteStream` answers a **ByteStream** (16 KiB read-ahead): `read` / `read:` / `readExactly:` (n or throw) / `readUntil:` (delimiter framing, delimiter included; `readUntil:limit:` throws IoError `#limitExceeded` past the bound) / `readAll` / `peek:` (look without consuming) / `writeAll:`. `.stringStream` (on a socket or a byte stream) answers a **StringStream**: `readLine` (terminator stripped; nil at EOF), `eachLine:`, `read`, `readAll`, `write:`, `writeln:` — UTF-8, with a catchable ParseError on invalid bytes.
> - **Sockets write through; file write streams buffer.** `[IO]File.create:` / `append:` answer a ByteStream that accumulates 16 KiB before draining — `flush!`, `close`, or program exit drains the rest. On a socket `flush!` is a no-op, so the same code runs over both. (`[IO]File.open:` answers a file object whose `byteStream` / `stringStream` open it for reading.)
> - `ByteStream.over:sock do:{ |st| … }` / `StringStream.over:st do:{ … }` scope a stream exactly as `connect:do:` scopes a socket: closed on every exit path, block value answered.
> - **`DNS`** exposes the resolver `connect:` already uses: `DNS.resolve:'host'` answers every address as Strings (IPv4 + IPv6, resolver order; `resolve4:` / `resolve6:` filter), a name that doesn't resolve throws a catchable IoError, and `DNS.reverse:'ip'` answers the PTR hostname or nil (unmapped is an answer, not an error). Lookups park the task, not the scheduler. Record-type queries (TXT/MX/SRV) are deliberately absent — that's a DNS client's job, not the system resolver's.

A complete echo round-trip — server and client are just two tasks on the same
scheduler:

```quoin
var listener = TcpListener.listen:'127.0.0.1:0';       "* port 0: the OS picks
var target = '127.0.0.1:' + listener.port.s;
var results = Async.timeout:5000 do:{
    Async.gather:#(
        { listener.acceptOnce:{ |conn| conn.writeAll:(conn.read:5) }; 'served' }
        { TcpSocket.connect:target do:{ |c| c.writeAll:'hello'.asBytes; (c.read:5).asString } }
    )
};
listener.close;
results         "* -> #(served hello)
```

The server task parks in `acceptOnce:`; the client task connects, writes, and
parks in `read:`; the scheduler interleaves them until both finish. For anything
line- or delimiter-oriented, wrap the socket in a stream instead of framing by
hand:

```quoin
var listener = TcpListener.listen:'127.0.0.1:0';
var target = '127.0.0.1:' + listener.port.s;
var reply = Async.timeout:5000 do:{
    (Async.gather:#(
        { listener.acceptOnce:{ |conn|
            var st = conn.stringStream;        "* consumes conn: text framing on top
            st.writeln:('you said ' + st.readLine) } }
        { var c = TcpSocket.connect:target;
          var st = c.stringStream;
          st.writeln:'ping';
          var line = st.readLine;
          st.close;
          line }
    )).at:1
};
listener.close;
reply           "* -> 'you said ping'
```

> **⚠ Gotcha — wrapping consumes the handle below.** `socket.byteStream`,
> `stream.stringStream`, and `TlsSocket.wrap:host:` all *transfer* the connection
> upward: the lower handle is left closed and further operations on it throw. Keep
> the topmost wrapper and talk only to it. (`acceptOnce:` closing the original
> socket afterwards is still safe — `close` is idempotent.)

The write-side split matters once files enter the picture — a socket write goes
straight to the peer, but a file write stream holds bytes back until they're worth
a syscall:

```quoin
var path = '/tmp/qn-book-streams.txt';
var out = ([IO]File.create:path).stringStream;   "* a BUFFERED write stream
out.writeln:'alpha';
out.writeln:'beta';                              "* still in the 16 KiB buffer
out.close;                                       "* close (or flush!) drains it
var lines = #();
([IO]File.open:path).stringStream.eachLine:{ |l| lines.add:l };
[IO]File.delete:path;
lines           "* -> #(alpha beta)
```

> **⚠ Gotcha — `readAll` on a socket returns only at EOF.** It keeps reading until
> the *peer closes*. In a request/response protocol where the other side keeps the
> connection open, `readAll` parks forever — frame reads with `read:`,
> `readExactly:`, or `readUntil:` instead (that is what the HTTP layers do).

---

## 24. `TcpServer`

> **Rules**
> - `TcpServer.new:{ var address = '127.0.0.1:0' }` **binds immediately** — `port` works before accepting starts. `start:{ |conn| … }` then accepts in the background: each connection is handled **in its own Task**, so handlers overlap, and each socket is closed when its handler returns.
> - Termination is deliberately the caller's: `stop` cancels the accept loop (in-flight handlers keep running), `join` drains them, `close` releases the port.
> - `join` is a drain *barrier*, not a result collector — a handler communicates through its socket (or a channel). `connections` counts live handler tasks (finished ones are swept).

```quoin
var server = TcpServer.new:{ var address = '127.0.0.1:0' };
server.start:{ |conn| conn.writeAll:(conn.read:4) };
var target = '127.0.0.1:' + server.port.s;
var replies = Async.timeout:5000 do:{
    Async.gather:#(
        { TcpSocket.connect:target do:{ |c| c.writeAll:'AAAA'.asBytes; (c.read:4).asString } }
        { TcpSocket.connect:target do:{ |c| c.writeAll:'BBBB'.asBytes; (c.read:4).asString } }
    )
};
server.stop;
server.join;
server.close;
replies         "* -> #(AAAA BBBB)
```

Both clients are served concurrently by one `TcpServer` — the `stop` / `join` /
`close` tail is the standard wind-down and reappears unchanged on `[HTTP]Server`
in §27.

---

## 25. The `[HTTP]` client

> **Rules**
> - `use std:net/http`. Verb helpers: `[HTTP]Client.get:url`, `get:headers:`, `post:body:`, `post:body:headers:`. A body argument normalizes: a **Map or List is JSON-encoded** (Content-Type `application/json`), a String sends as its UTF-8 bytes, Bytes goes as-is, an `[HTTP]Body` is used unchanged.
> - Everything else goes through the builder: `[HTTP]Client.request:url` answers an `[HTTP]Request`; thread `method:`, `header:value:`, `headers:`, `body:`, `insecure:`, `followRedirects:` (default true), `maxRedirects:` (default 10), and finish with `send`.
> - The response: `status`, `ok?` (2xx), `reason`, `headers` (a `#(name value)` pair list in wire order, duplicates preserved), `header:'name'` (case-insensitive first match), and `body` — an **`[HTTP]Body`**, stream-backed until drained.
> - Drain with `body.text` / `body.json` / `body.bytes`: reads everything, transparently decodes any Content-Encoding (gzip / zstd / deflate), closes the connection, and caches (idempotent). Or stream it: `body.chunks` is a lazy Generator of chunk bodies, `body.each:{ |chunk| … }` iterates them (`chunk.bytes` / `.text` / `.meta`), and `body.close` abandons early.
> - **Redirects are followed by default** (3xx + Location, up to `maxRedirects`): 307/308 preserve the method and body; 303 — and 301/302 from a non-GET — re-issue as a bodyless GET; credentials are stripped on a cross-origin hop. `https://` URLs ride `TlsSocket` transparently.
> - Failures are catchable: connect errors and truncated bodies throw `IoError`, a malformed head `ParseError`, too many redirects `ValueError`.

The everyday surface — shown, not run, because these lines touch a real network:

```quoin norun
use std:net/http;
var resp = [HTTP]Client.get:'https://example.org/';
resp.status;                      "* e.g. 200
resp.header:'content-type';      "* case-insensitive header lookup
resp.body.text;                   "* drain as a String — .json / .bytes likewise

"* POST a Map: it JSON-encodes itself, Content-Type included
[HTTP]Client.post:'https://api.example.org/users' body:#{ 'name':'Quoin' };

"* the builder covers custom verbs, headers, and redirect policy; .send fires
var deleted = (((([HTTP]Client.request:'https://api.example.org/users/7')
    .method:'DELETE')
    .header:'Authorization' value:'Bearer t0ken')
    .followRedirects:false)
    .send;

"* stream a large body chunk by chunk instead of draining it into memory
var out = [IO]File.create:'/tmp/big.bin';
([HTTP]Client.get:'https://example.org/big.bin').body.each:{ |chunk|
    out.writeAll:(chunk.bytes)
};
out.close
```

The client is pure Quoin over `TcpSocket` / `TlsSocket` — only the head parser is
native — so it happily talks to any server, including a four-line one faked out
of §23's parts:

```quoin
use std:net/http;
var listener = TcpListener.listen:'127.0.0.1:0';
var wire = 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{"ok":true}';
var served = Task.spawn:{ listener.acceptOnce:{ |conn| conn.read:1024; conn.writeAll:wire.asBytes } };
var resp = Async.timeout:5000 do:{ [HTTP]Client.get:('http://127.0.0.1:' + listener.port.s + '/') };
served.join;
listener.close;
#( resp.status (resp.body.json.at:'ok') )       "* -> #(200 true)
```

> **⚠ Gotcha — an undrained body owns the connection.** `send` returns as soon as
> the head is parsed; the socket stays open, owned by `resp.body`, until you drain
> it (`.text` / `.json` / `.bytes`), iterate it fully, or `.close` it. Always do one
> of those — a body left dangling holds its connection until GC reaps it.

---

## 26. WebSocket

> **Rules**
> - `use std:net/websocket`, then `WebSocket.connect:'ws://…'` (or `wss://` — TLS, like the HTTP client, falls out for free). The upgrade handshake sends a random `Sec-WebSocket-Key` and **verifies** the response's `Sec-WebSocket-Accept`; a wrong status or token throws a catchable IoError.
> - `receive` answers the next **message**: a String for a text frame, Bytes for binary, and `nil` once the connection has closed cleanly. Fragmented messages are reassembled and pings are ponged invisibly; `eachMessage:{ |m| … }` loops until close. Receives park the task, not the scheduler.
> - `sendText:` / `sendBytes:` send one masked frame each (clients must mask — `Bytes#maskWith:` natively); `ping` / `ping:` probe liveness.
> - `close` (or `close:code reason:`) runs the RFC 6455 close handshake: send a close frame, drain to the peer's reply. After close, `closeCode` / `closeReason` report the far side's; a connection torn *without* a close frame surfaces as an IoError instead of a silent nil.
> - `WebSocket.acceptFor:key` (class-side) computes the upgrade token, so a Quoin *server* can answer the handshake too — the test suite's in-file echo server (`qnlib/tests/79-websocket.qn`) is the reference. First-class server support in `[Web]App` is future work, as are subprotocols and permessage-deflate.

```quoin norun
use std:net/websocket;
var ws = WebSocket.connect:'wss://stream.example.org/live';
ws.sendText:(JSON.generate:#{ 'subscribe': 'trades' });
ws.eachMessage:{ |m| (JSON.parse:m).print };    "* until the server closes
ws.closeCode                                    "* why it ended
```

---

## 27. Serving HTTP: `[HTTP]Server` and `[Web]App`

Serving splits into two layers, mirroring the client's philosophy: `[HTTP]Server`
(`use std:net/http_server`) is the HTTP/1.1 *protocol machine* — no routing, no
policy — and the `[Web]` framework (`use std:web/*`) layers routing, middleware,
and render conventions on top of a **pure** request→response pipeline.

### The transport: `[HTTP]Server`

> **Rules**
> - The handler contract is *request in, response out*: `var handler = { |req| … }` receives an `[HTTP]ServerRequest` (`method`, `target` raw as sent, `version`, `headers`, `header:`, `body` — an `[HTTP]Body`; a bodyless request answers an empty body, never nil) and must return an `[HTTP]ServerResponse`.
> - Lifecycle is `TcpServer`'s: bind at construction (`port` before `start`), one Task per connection, HTTP/1.1 keep-alive, then `stop` / `join` / `close`.
> - Robustness defaults, overridable in the `new:{}` block: `maxHeadBytes` 16 KiB → 431; `maxBodyBytes` 8 MiB → 413; `headTimeoutMs` 30 s → 408; `idleTimeoutMs` 60 s → quiet close; `maxConnections` nil (unlimited) → 503.
> - A response body may be nil, Bytes (a String converts), or a **Generator** — each yield goes out as one chunked frame (this is what makes server-sent events work). HEAD suppresses body bytes but keeps the entity headers. An uncaught handler throw is answered as a plain 500.

```quoin norun
use std:net/http_server;
var server = [HTTP]Server.new:{
    var address = ':8080';
    var maxBodyBytes = 1048576;              "* non-default limits go here
    var handler = { |req|
        ([HTTP]ServerResponse.new:{ var status = 200 }).body:('hello ' + req.target)
    }
};
server.start        "* accept in the background; stop / join / close to wind down
```

Most programs never write that: the `[Web]` framework supplies the handler.

### The framework: `[Web]App`

> **Rules**
> - `use std:web/*` — with the transport already loaded (`use std:net/http_server`, or `use std:net/*` for everything).
> - **Routing**: `app.get:'/pattern' do:handler` (likewise `post:do:`, `put:do:`, `patch:do:`, `delete:do:`; any verb via `on:pattern:do:`). Patterns: static segments, `:name` (exactly one segment, percent-decoded into `req.params`), `*name` (the — possibly empty — rest of the path; must be last). **Most specific wins** — static beats `:param` beats `*splat`, segment by segment — so registration order never matters; a duplicate shape throws *at registration*. A path other verbs serve → 405 with `Allow`; nothing anywhere → 404; HEAD falls back to the GET route.
> - A handler block may take the request as a parameter (`{ |req| … }`) or address it as `self` (`{ .param:'id' }`). Its return value is normalized by the **`respondTo:` multimethod**: String → text/plain, Map/List → JSON, Integer → a bare status, Bytes → octet-stream, Generator → a chunked stream, a response passes through, nil → 404, anything else → the text of its `.s`. Subclass `[Web]App` and extend the multimethod (§13) for app-specific renderings.
> - **Request conveniences** (reopened onto `[HTTP]ServerRequest`): `param:'id'` / `params`, `query:'k'` / `query` (decoded, parsed on demand), `path` (query stripped) / `rawTarget`, `json` (catchable ParseError), `form` (urlencoded → Map), `mediaType`, `header:'name'`.
> - **Responses**: builders `[Web]Response.text:` / `html:` / `json:` / `json:status:` / `bytes:` / `status:` / `noContent` / `redirect:` / `redirect:status:` / `stream:` / `stream:contentType:`; the threading setters `status:`, `header:value:`, `contentType:`, `body:` chain further.
> - **Middleware**: `app.use:{ |req next| … }` — onion model, first `use:` outermost. Call `next.value:req` for the inner response; *returning without calling `next` short-circuits*. Either way the return value goes through `respondTo:` too.
> - **Errors**: `HttpError.throw:401` / `HttpError.throw:422 body:#{ … }` from anywhere under a handler maps onto the response (the body rendered through `respondTo:`). Any other uncaught error becomes a bare 500 — detail leaks only with `app.debug:true` (which also serves `VM.psTree` at `GET /_qn/ps`).
> - **`app.handle:req` is the whole app as a pure function** — middleware → router → handler → normalization → error mapping, no sockets — so apps unit-test in-process. `app.start:'host:port'` serves in the background and answers the `[HTTP]Server` handle; `app.serve:` blocks (start + join); `app.serve:':8080' workers:4` runs the pure pipeline on a pool of worker isolates (`backing:'process'` for real multicore).
> - **Request logging** is on by default for *served* requests — one `Log.info:` line per request (`GET /users/7 -> 200 (1ms 200µs)`) from the transport hop, so the pure `handle:` stays silent and unit tests don't log. `app.logRequests:false` turns it off; `Log.level:` / `Log.sink:` govern the lines like any other entry (§44).

Routing and the render conventions, driven entirely in-process — the requests are
plain objects, fabricated with `new:{}`:

```quoin
use std:net/http_server;
use std:web/*;
var app = [Web]App.new;
app.get:'/hello/:name' do:{ |req| 'hi ' + (req.param:'name') }   "* String -> text/plain
app.get:'/files/*rest' do:{ .param:'rest' }                      "* the request is self here
app.get:'/data' do:{ #{ 'n':1 } }                                "* Map -> JSON
app.get:'/teapot' do:{ 418 }                                     "* Integer -> a bare status

var get = { |t| [HTTP]ServerRequest.new:{ var method = 'GET'; var target = t } };
(app.handle:(get.value:'/hello/qn')).body.asString     "* -> 'hi qn'
(app.handle:(get.value:'/files/a/b')).body.asString    "* -> 'a/b'
(app.handle:(get.value:'/data')).body.asString         "* -> '{"n":1}'
(app.handle:(get.value:'/teapot')).status              "* -> 418
(app.handle:(get.value:'/nope')).status                "* -> 404
```

Routes are first-class values implementing the `~` match protocol (Part IV), so
they also compose with `case:` outside any app:

```quoin
use std:net/http_server;
use std:web/*;
var route = [Web]Route.of:'/users/:id/files/*rest';
route ~ '/users/7/files/a/b'         "* -> true
route.bind:'/users/7/files/a/b'      "* -> #{'id': '7' 'rest': 'a/b'}
```

Middleware wraps the pipeline as an onion — request work on the way in, response
work on the way out:

```quoin
use std:net/http_server;
use std:web/*;
var log = #();
var app = [Web]App.new;
app.get:'/' do:{ log.add:'handler'; 'ok' }
app.use:{ |req next| log.add:'auth>'; var r = next.value:req; log.add:'<auth'; r }
app.use:{ |req next| log.add:'trace>'; var r = next.value:req; log.add:'<trace'; r }
app.handle:([HTTP]ServerRequest.new:{ var method = 'GET'; var target = '/' })
log             "* -> #(auth> trace> handler <trace <auth)
```

And `HttpError` short-circuits from any depth — thrown in a handler (or anything
it calls), mapped by the dispatcher:

```quoin
use std:net/http_server;
use std:web/*;
var app = [Web]App.new;
app.get:'/admin' do:{ |req|
    (req.header:'authorization').defined?.else:{ HttpError.throw:401 };
    'welcome'
}
app.get:'/signup' do:{ HttpError.throw:422 body:#{ 'error':'bad email' } }
var get = { |t| [HTTP]ServerRequest.new:{ var method = 'GET'; var target = t } };
(app.handle:(get.value:'/admin')).status               "* -> 401
(app.handle:(get.value:'/signup')).body.asString       "* -> '{"error":"bad email"}'
```

Streaming responses and the workers pool round out the surface — illustrative,
since one runs forever and the other wants real cores:

```quoin norun
"* server-sent events: a Generator return streams one chunked frame per yield
app.get:'/events' do:{
    Generator.from:{
        (1..5).each:{ |n|
            ^> ('data: tick ' + n + '\n\n');
            Async.sleep:1000
        }
    }
}

app.serve:':8080'                               "* single VM: blocks until stopped
app.serve:':8080' workers:4                     "* + a pool of 4 worker isolates
app.serve:':8080' workers:8 backing:'process'   "* child processes: real multicore
```

> **⚠ Gotcha — pool-mode handlers cannot capture main-VM mutable state.** With
> `workers:n`, the transport VM keeps the sockets and ships each request *as data*
> to a worker isolate that re-runs your program's unit and executes `handle:`
> there. Isolates share nothing: a captured counter increments per-worker, not
> globally. Keep shared state in an external store — or serve single-VM, where
> handlers may close over anything. (The pure `handle:` core is exactly what makes
> requests shippable.)

---

## 28. End to end: a JSON service, tested in-process

The framework's testing story *is* `handle:` — build the app, then call it like a
function. No listener, no ports, no concurrency; requests are constructed and
responses inspected directly:

```quoin
use std:net/http_server;
use std:web/*;

var users = #{ '7': #{ 'id':'7' 'name':'Ada' } };
var app = [Web]App.new;
app.get:'/users/:id' do:{ |req|
    var user = users.at:(req.param:'id');
    user.defined?.if:{ user } else:{ 404 }
}
app.post:'/users' do:{ |req|
    var u = req.json;
    users.at:(u.at:'id') put:u;
    [Web]Response.json:u status:201
}

var get = { |t| [HTTP]ServerRequest.new:{ var method = 'GET'; var target = t } };
(app.handle:(get.value:'/users/7')).body.asString      "* -> '{"id":"7","name":"Ada"}'
(app.handle:(get.value:'/users/9')).status             "* -> 404

var post = [HTTP]ServerRequest.new:{
    var method = 'POST';
    var target = '/users';
    var body = [HTTP]Body.of:('{"id":"8","name":"Grace"}'.asBytes) contentType:'application/json'
};
(app.handle:post).status                               "* -> 201
(app.handle:(get.value:'/users/8')).body.asString      "* -> '{"id":"8","name":"Grace"}'
```

Serving the same app over real sockets is one line — `start:` — and the round
trip composes with everything from this chapter: the `[HTTP]Client` from §25, the
ephemeral-port-and-timeout pattern from §22:

```quoin
use std:net/*;
use std:web/*;
var app = [Web]App.new;
app.get:'/users/:id' do:{ |req| #{ 'id':(req.param:'id') } }

var server = app.start:'127.0.0.1:0';        "* bind + accept in the background
var base = 'http://127.0.0.1:' + server.port;
var got = Async.timeout:5000 do:{ [HTTP]Client.get:(base + '/users/7') };
server.stop;
server.join;
server.close;
#( got.status (got.body.json.at:'id') )      "* -> #(200 7)
```

This split — pure tests against `handle:`, a few loopback round-trips for the
transport — is how the stdlib tests itself (`qnlib/tests/47`–`49` are the pure
half; `46` and the live half of `49` drive real sockets).

---

Next: **[Part VII — The gradual type system](07-types.md)**.
