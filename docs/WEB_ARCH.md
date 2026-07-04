# HTTP Web Framework Architecture — `[Web]` over `[HTTP]Server`

Status: **design — nothing implemented yet**. Companion to `ASYNC_ARCH.md` (the I/O
substrate) and `qnlib/net/http.qn` (the client, whose body/framing machinery this
design reuses). See the *Staged plan* at the bottom.

## Decision

Build the web framework as **three layers**, keeping the client's philosophy — *the
only native piece is the head parser*; everything else is pure Quoin:

1. **Layer 0 (Rust, tiny):** `[HTTP]Parser.parseRequestHead:` — the request-side
   mirror of the existing response head parser (`httparse`, `src/runtime/http.rs`).
2. **Layer 1 (`qnlib/net/http_server.qn`):** `[HTTP]Server` — the HTTP/1.1 protocol
   machine. Task-per-connection, keep-alive loop, body framing, limits, response
   serialization. No routing, no policy: its handler contract is *request in,
   response out*.
3. **Layer 2 (`qnlib/web/*`):** the `[Web]` framework — routing, middleware, render
   conventions, errors, URL/query/form decoding. Its core is a **pure function**:
   `app.handle:req → response`. `[HTTP]Server` is merely the transport that drives
   it (Rack/WSGI-shaped), so the whole framework is unit-testable without sockets.

Settled scope decisions (2026-07-04):

- **Lean core v1**: routing, params, middleware, JSON + form decoding, errors,
  keep-alive, limits. *Deferred*: server TLS, WebSockets, cookies/sessions, response
  compression, multipart, static files, HTTP/2.
- **Native request-head parsing** (not pure Quoin) — robust against malformed input,
  consistent with the client.
- **Namespace `[Web]`** (`use std:web/*`). Error classes live at the **root**
  namespace (see *Grammar constraint* below).
- **Most-specific-wins routing** — static segment beats `:param` beats `*splat`,
  independent of registration order (the language's order-independent multimethod
  spirit; ties are registration-time errors, like `AmbiguousMethodError`).

## What exists today / the gaps

| Have | Gap the framework fills |
|---|---|
| `TcpListener` `accept`/`acceptLoop:` parks green threads | accept loop that *reaps* finished tasks (`TcpServer` accumulates its `@tasks` forever) |
| `ByteStream` `readUntil:` / `readExactly:` / `peek:` | bounded head reads (`readUntil:` alone buffers unboundedly) |
| `[HTTP]Body` — chunked/length framing, `.text`/`.json`/`.bytes`/`.chunks` | reused wholesale for **request** bodies |
| `[HTTP]Parser.parseHead:` (responses only) | `parseRequestHead:` (slice 0) |
| `Async.timeout:do:onCancel:` cancels in-flight I/O | head-read + keep-alive idle timeouts |
| `JSON.parse:`/`generate:`, `Bytes` gz/deflate codecs | percent/URL codec — **does not exist anywhere**; written in pure Quoin |
| `TlsSocket` (client connect/wrap only) | server TLS needs a new host primitive — **deferred** |
| `DateTime.nowUtc` + `year`/`month`/`day`/`weekday` | RFC-1123 `Date:` formatter, pure Quoin |

## Concurrency model

One `Task` per connection, all on the single-threaded cooperative scheduler. Sockets
park the green thread, so thousands of idle keep-alive connections are cheap; but a
CPU-bound handler blocks every other connection until it yields (same trade-off as
Node). Parallelism across OS threads is out of scope — the VM scheduler is
single-threaded by design (gc_arena).

The server owns its accept loop (a Task, cancellable like `TcpServer.stop`) and a
registry of live connection tasks. Each connection task removes itself from the
registry in a `finally:`, so long-lived servers don't leak handles. `stop` cancels
the accept loop; `join` drains in-flight connections; `close` releases the port.

## Layer 0 — native additions (`src/runtime/http.rs`)

- `[HTTP]Parser.parseRequestHead:bytes` → `#( method target versionInt headers )`
  or `nil` if the head is incomplete; throws (catchable `ParseError`) on malformed
  input. `httparse::Request`, same `MAX_HEADERS = 128` cap as the response side.
  `method` uppercase String, `target` the raw request-target, `versionInt` 0|1,
  `headers` the same order/duplicate-preserving `#(name value)` pair list the client
  uses.
- **Slice-3 hardening (optional):** `ByteStream.readUntil:delim limit:n` — same as
  `readUntil:` but throws once `n` bytes are buffered without the delimiter. Without
  it a single header *line* of hostile length buffers unboundedly (the head timeout
  bounds wall-clock, not memory). Small addition to `src/runtime/streams.rs`.

## Layer 1 — `[HTTP]Server` (`qnlib/net/http_server.qn`)

### Configuration

```quoin
var server = [HTTP]Server.new:{
    var address = ':8080';
    var handler = { |req| ... };        "* must return an [HTTP]ServerResponse
    var maxHeadBytes = 16384;           "* → 431 Request Header Fields Too Large
    var maxBodyBytes = 8388608;         "* 8 MiB → 413 Content Too Large
    var headTimeoutMs = 30000;          "* reading the head → 408 Request Timeout
    var idleTimeoutMs = 60000;          "* keep-alive idle → close quietly
    var maxConnections = nil            "* nil = unlimited; over → 503 + close
};
server.start;                           "* accept in the background
server.port;                            "* bound port (after ':0')
server.stop; server.join; server.close
```

### The connection loop

Per accepted socket, wrapped in a `ByteStream`, loop:

1. **Bounded head read** — accumulate `readUntil:'\r\n'` lines up to the blank line,
   capping total bytes at `maxHeadBytes` (→ 431). The first request's head read (and
   each subsequent one, from its first byte) runs under
   `Async.timeout:headTimeoutMs` (→ 408 then close). Waiting for byte one of a
   *subsequent* request uses `idleTimeoutMs` instead (→ close, no response).
2. **Parse** via `[HTTP]Parser.parseRequestHead:` (malformed → 400 then close).
   Origin-form targets only in v1 (`/path?query`); anything else → 400.
3. **Build the request** — `[HTTP]ServerRequest` with `method`, `target`, `version`,
   `headers`, and a **stream-backed `[HTTP]Body`** (reusing the client's class):
   `Transfer-Encoding: chunked` → chunked framing; else `Content-Length` → length
   framing (a preflight `cl > maxBodyBytes` → 413); else **no body** (per RFC 7230 a
   request without CL/TE has none — no close-delimited request bodies, unlike
   responses). A `Content-Encoding` rides through — the body already decodes on
   drain. Chunked bodies enforce `maxBodyBytes` cumulatively while draining.
4. **`Expect: 100-continue`** — write `HTTP/1.1 100 Continue\r\n\r\n` eagerly before
   invoking the handler.
5. **Dispatch** — call the handler; any uncaught throw → 500 (the framework layer
   normally catches first and this is only a backstop).
6. **Serialize** — status line (canonical reason-phrase table), headers, `Date:`
   (RFC-1123, pure Quoin from `DateTime.nowUtc`), then the body: materialized
   `Bytes` → `Content-Length`; a `Generator`/stream body → `Transfer-Encoding:
   chunked`, writing each yielded chunk as it arrives (this is what makes SSE work).
   `HEAD` suppresses the body but keeps the entity headers.
7. **Keep-alive decision** — persist when HTTP/1.1 and neither side sent
   `Connection: close` (HTTP/1.0 closes unless `keep-alive` was requested). Before
   the next iteration, **auto-drain** any unread request body (`maxBodyBytes` still
   applies). Error responses from steps 1–3 (400/408/413/431) always close.
   Pipelined requests need no special handling — buffered bytes are simply the next
   iteration's input; responses are inherently sequential.

### Classes

- `[HTTP]ServerRequest` — `method`, `target` (raw), `version`, `headers` (pair
  list), `header:` (case-insensitive first match, same helper shape as
  `[HTTP]Response.find:named:`), `body` (an `[HTTP]Body`). The framework *reopens*
  this class with conveniences (`param:`, `query:`, `json`, …) — extension by
  `<--` is the Quoin way to layer without wrappers.
- `[HTTP]ServerResponse` — `status`, `headers`, `body` (nil | Bytes | Generator),
  threading setters (`status:`, `header:value:`, `contentType:` returning self).
  Layer 2 aliases it as `[Web]Response <- [HTTP]ServerResponse` and reopens it with
  the builder conveniences, so users only ever see `[Web]Response`.

## Layer 2 — the `[Web]` framework (`qnlib/web/*`)

### The API, by example

```quoin
use std:web/*;

var app = [Web]App.new;

app.get:'/' do:{ 'hello from Quoin' };

app.get:'/users/:id' do:{ |req|
    var user = users.at:(req.param:'id');
    user.defined?.if:{ user } else:{ 404 }      "* Map → JSON, Integer → bare status
};

app.post:'/users' do:{ |req|
    var u = req.json;
    users.at:(u.at:'id') put:u;
    [Web]Response.json:u status:201
};

app.use:{ |req next|
    var resp = next.value:req;
    [IO]Stdout.writeln:(%'%{req.method} %{req.path} -> %{resp.status.s}');
    resp
};

app.serve:':8080'                               "* start + join (blocks)
```

`app.start:':8080'` returns the `[HTTP]Server` handle instead (for `stop`/`join` —
tests, graceful shutdown). `app.handle:req` is the pure core: middleware onion →
router → render normalization → error mapping, no sockets involved.

### Routing (`[Web]Route`)

- Pattern syntax: static segments, `:name` (one segment, percent-decoded into
  params), `*name` (rest of the path, must be last). Trailing slashes normalize away
  (except root).
- **Matching via the `~` protocol**: `[Web]Route` implements `~:`, so
  `route ~ path` works standalone and routes compose with `case:` for anyone
  hand-rolling dispatch. `route.bind:path` → params Map or nil.
- **Specificity**: per-segment kind vector (static=0, `:param`=1, `*splat`=2)
  compared lexicographically; lowest wins. Two *distinct* patterns can't tie while
  both matching, so the only tie is a duplicate registration — which **throws at
  registration time** (the router's `AmbiguousMethodError` analog).
- Method tables per verb; `HEAD` falls back to a `GET` route (layer 1 suppresses the
  body). A path that matches other verbs but not this one → **405 with `Allow`**.
- Percent-decoding happens **per segment after splitting on `/`**, so an encoded
  `%2F` cannot create a segment boundary.

### Render conventions

The dispatcher normalizes a handler's return value with a `respondTo:` multimethod —
type-directed dispatch where users see it daily:

```quoin
respondTo: -> { |v:String|    [Web]Response.text:v };
respondTo: --> { |v:Map|       [Web]Response.json:v };
respondTo: --> { |v:List|      [Web]Response.json:v };
respondTo: --> { |v:Integer|   [Web]Response.status:v };
respondTo: --> { |v:Bytes|     [Web]Response.bytes:v };
respondTo: --> { |v:Generator| [Web]Response.stream:v };
respondTo: --> { |v:[Web]Response| v };                    "* already a response
respondTo: --> { |v| v.defined?.if:{ [Web]Response.text:v.s } else:{ [Web]Response.status:404 } }
```

Builders: `text:`, `html:`, `json:` / `json:status:`, `bytes:`, `status:`,
`redirect:` / `redirect:status:`, `stream:` / `stream:contentType:` (default
`text/event-stream` fits SSE), `noContent`.

### Middleware

Onion model. `app.use:` appends anything callable with `(req, next)` — a two-arg
block, or an object with a matching `value:value:`-shaped method (duck typing, no
interface machinery). `next` is a one-arg callable. A middleware may short-circuit
by returning without calling `next`; its return value goes through the same
`respondTo:` normalization.

### Errors — `HttpError`

```quoin
app.get:'/admin' do:{ |req|
    (req.header:'authorization').defined?.else:{ HttpError.throw:401 };
    ...
};
"* elsewhere: HttpError.throw:422 body:#{ 'error': 'bad email' }
```

`HttpError` is a **root-namespace** `Error` subclass carrying `@status @body`. The
dispatcher wraps every handler in a typed catch chain: `HttpError` → its status
(+ optional body through `respondTo:`); any other `Error` → 500 (body is the plain
reason phrase unless `app.debug:true`, which includes `e.message`).

> **Why root-level:** namespaced type annotations parse (`catch:{ |e:[Web]Halt| … }`
> is legal since the `type_ref` grammar fix), so this is a convention choice, not a
> constraint: `HttpError` reads as a sibling of the built-in error family —
> `IoError`, `ValueError`, `TimeoutError`, `ParseError` — which is all root-level.

### Request conveniences (reopening `[HTTP]ServerRequest`)

- `path` — decoded path, query stripped; `rawTarget` preserved.
- `query` — lazily parsed query-string Map; `query:'k'` single value.
- `param:'id'` / `params` — route bindings (set by the router).
- `json` — `body.json` (throws catchable `ParseError`).
- `form` — `application/x-www-form-urlencoded` body → Map (`+` as space).
- `mediaType`, `header:'name'`.

### `[Web]Url` — the percent codec (pure Quoin)

`encode:` / `decode:` (percent + UTF-8), `queryParse:` (`a=1&b=2` → Map),
`formDecode:` (adds `+` → space). Not hot enough to justify a native impl; fully
unit-testable with no network.

## Limits & robustness defaults

| Limit | Default | Over-limit behavior |
|---|---|---|
| `maxHeadBytes` | 16 KiB | 431, close |
| `maxBodyBytes` | 8 MiB | 413 (preflight on CL; cumulative on chunked/drain), close |
| `headTimeoutMs` | 30 s | 408, close |
| `idleTimeoutMs` (keep-alive) | 60 s | close quietly |
| `maxConnections` | unlimited | accept → 503, close |

Known v1 gap: a single delimiter-less line can buffer past `maxHeadBytes` until the
head timeout fires — fixed by the optional `readUntil:limit:` primitive (slice 3).

## Testing strategy

- **No-network unit tests** (`qnlib/tests/`): `[Web]Url` codec vectors; router
  specificity/405/params; `app.handle:` with constructed `[HTTP]ServerRequest`s —
  the pure-core payoff.
- **Round-trip tests**: the `tests/24-server.qn` idiom — bind `127.0.0.1:0`,
  `Async.gather:` the server plus `[HTTP]Client` calls, `Async.timeout:` as hang
  guard. The existing client exercises the server's chunked output and keep-alive
  from the other side for free.
- **Rust side**: unit tests for `parseRequestHead:` (and `readUntil:limit:` if
  added), mirroring the existing parser tests.
- **Soak**: a `qnlib/stress/` script — many concurrent keep-alive connections,
  slowloris-style trickle, oversized heads/bodies.

## Staged plan

Each slice lands green (tests + `cargo fmt` + `qn fmt`) and is independently useful.

- **Slice 0 — native request parser.** `[HTTP]Parser.parseRequestHead:` + Rust unit
  tests.
- **Slice 1 — minimal `[HTTP]Server`.** Close-mode (no keep-alive), bytes bodies
  only, request object, response serializer, reason-phrase table; first round-trip
  test against `[HTTP]Client`.
- **Slice 2 — body framing.** Request CL + chunked bodies via `[HTTP]Body`,
  100-continue, 413; response streaming (chunked from a Generator), HEAD
  suppression.
- **Slice 3 — keep-alive & limits.** Persistent connections, auto-drain, head/idle
  timeouts, 400/408/431 paths, `Date:` header, task-reaping registry,
  stop/join/close; optional `readUntil:limit:` hardening.
- **Slice 4 — `[Web]Url`.** Percent/query/form codec + no-network tests.
- **Slice 5 — router.** `[Web]Route` (`~` protocol, `bind:`), specificity dispatch,
  405 + `Allow`, duplicate-registration error.
- **Slice 6 — `[Web]App`.** Middleware onion, `respondTo:` conventions, `HttpError`
  mapping, request conveniences, `handle:` pure-core tests, `serve:`/`start:`.
- **Slice 7 — polish.** Example app, stress soak, docs pass.

### File layout

```
src/runtime/http.rs            "* + parseRequestHead:            (slice 0)
src/runtime/streams.rs         "* + readUntil:limit: (optional)  (slice 3)
qnlib/net/http_server.qn       "* [HTTP]Server{,Request,Response} (slices 1–3)
qnlib/web/00-url.qn            "* [Web]Url                        (slice 4)
qnlib/web/01-error.qn          "* HttpError (root-level)          (slice 6)
qnlib/web/02-route.qn          "* [Web]Route + router             (slice 5)
qnlib/web/03-response.qn       "* [Web]Response alias + builders  (slice 6)
qnlib/web/04-app.qn            "* [Web]App                        (slice 6)
qnlib/tests/46-url.qn …        "* per-slice suites
```

## Deferred (with sketches)

- **Server TLS** — needs a host-side `IoRequest::TlsAccept` + `TlsAcceptor`
  (rustls `ServerConfig`, cert/key loading) mirroring the client's `TlsWrap`; a
  test-only acceptor already exists in `io_backend.rs` tests. Until then: terminate
  TLS in a fronting proxy.
- **WebSockets** — `Upgrade` handshake is easy (`Base64` + SHA-1 — SHA-1 would need
  a native or pure-Quoin digest); frame codec over `ByteStream`.
- **Response compression** — `encodeGz` already exists; negotiate via
  `Accept-Encoding`, skip for small/streamed bodies.
- **Cookies/sessions, multipart, static files, HTTP/2** — post-v1, in roughly that
  order.
