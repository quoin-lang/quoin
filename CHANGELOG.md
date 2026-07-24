# Changelog

All notable changes to Quoin are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

Quoin is pre-1.0. Minor versions may make breaking language changes; each one is called out
under **Changed**, with the migration.

## [Unreleased]

### Added

- **`Random` — the seedable PRNG the docs already promised** (#146).
  `Random.seed:` answers a deterministic generator: the same seed answers the
  same stream on every platform and every Quoin version (xoshiro256** seeded
  via SplitMix64 — the algorithm is part of the contract, so simulations and
  property tests can replay). `Random.new` seeds from OS entropy but remembers
  the seed it drew (`seed`), so a failing run can print it and be replayed
  exactly. Draws: `next` (Double in [0, 1)), `int:` (unbiased, end-exclusive
  like ranges), `pick:`, `shuffle:`, `bytes:`. State is per instance — there
  is no hidden global generator. For secrets, `[Crypto]Random` remains the
  deliberately-unseedable answer.

- **`PeerDiedError` — peer deaths are typed** (SUPERVISION.md slice 0). When the
  isolate hosting a receiver *dies* — its process exits, its connection closes
  under a call, a thread worker's body panics — the raised error is now the new
  root class `PeerDiedError`, carrying `reason` (`#exited` / `#panicked`) and
  `peer` (the hosted class, worker label, or extension name). This covers every
  seam: an extension crashing mid-call or found dead, a hosted service dying
  mid-conversation or refusing as a corpse, `Worker.join` on a vanished or
  panicked worker, and `w.send:` meeting an exited worker's closed inbox
  (all previously untyped strings — the send seam was found by the web soak
  leaking through the pool's typed catch as a 500). Errors a *live* peer reports are
  unchanged — death is the peer disappearing, never a value it raised.
  **Breaking**: an extension crash was previously an `IoError` of kind
  `#closed`; a `catch:{ |e:IoError| … }` around extension calls that meant to
  catch the crash should catch `PeerDiedError` instead (`IoError` was too
  user-error-adjacent to share a catch clause with a dead isolate).
  Death housekeeping rides along: a dead service now releases its parent-held
  block handles (previously leaked until VM exit), `VM.claims` rows carry an
  explicit `gone` marker (`died` / `stopped`; `VM.claimsReport` renders it),
  and a dead link's parked remote channel receivers are purged with their
  owner-side roots released — a later `send:` reaches a live receiver instead
  of vanishing into the closed lane.

- **Supervision policies — automatic restart with a circuit breaker**
  (SUPERVISION.md slice 3, closing arc 3's runtime surface). Attach a policy
  post-spawn — `svc.serviceSupervise:(Supervise.always)` on a hosted worker,
  `e.supervise:` on an extension, or `quoin.toml [extension]` keys
  (`restart = "always"`, `backoff-ms`, `cap-ms`, `max-restarts`, `window-ms`)
  for package extensions — and every *death* (never an error, never a stop)
  triggers an automatic respawn from the frozen recipe, with delays doubling
  from `backoff` to `cap`. Sends arriving during the cycle park and land on
  the new incarnation (the in-flight call at death time still errors typed:
  nothing is ever replayed). More than `max` deaths inside `window` ms and the
  peer **gives up permanently** — the circuit breaker: calls raise
  `PeerDiedError` with the new reason `#gaveUp`, and `VM.peers` shows the
  final incarnation as `gaveUp`. `Supervise` is plain immutable data
  (`Supervise.always`, refined via `backoff:cap:` and `max:within:`);
  supervised extensions get the exit watch armed automatically, so idle
  crashes restart too. Manual `serviceRestart`/`restart` refuse on a
  supervised peer — the policy owns the budget.

- **Restart hooks — user code re-establishes its own state after a respawn.**
  `svc.serviceOnRestart:{ |s| … }` (hosted workers, root proxy) and
  `e.onRestart:{ |x| … }` (extensions) install a one-argument block that runs
  inside every restart attempt — supervised or manual — after the fresh
  incarnation is up. Its purpose is the state supervision deliberately does
  not restore: ambient child-side configuration with no handle (an API key, a
  log level, a registration) silently resets on respawn, and the hook is
  where whatever *uses* the peer re-applies it — package `init.qn` glue stays
  once-per-VM, define-once. For services the hook runs before the restart
  gate reopens (parked senders resume only against a hooked-up incarnation),
  and its own sends to the service pass the closed gate. A hook failure fails
  the attempt: the fresh peer is stopped, a supervisor counts it against the
  budget — a permanently broken hook spends to `#gaveUp` rather than serving
  a half-configured peer. Re-installing replaces the hook; `nil` clears it.
  Internal: hook blocks root through the new generic pin table (`vm.pins`,
  one traced side table replacing the per-feature GC-root fields — recipe
  channel args, lifecycle event channels, and the worker-side hosted-object
  table all live there now; `VM.stats` gained a `pins` section reporting
  every Value-retaining registry on one dashboard).

- **The web worker pool self-heals** (WEB_ARCH.md workers — supervision's
  first consumer, built as a *library* strategy over the lifecycle events).
  A pool worker's death fails only its own in-flight requests (a clean 502)
  while the pool routes around the respawn window — siblings absorb the
  load, nothing parks — and the slot respawns from the pool's recipe under
  `app.poolSupervise:`'s `Supervise` value (default `Supervise.always`;
  `Supervise.never` restores fail-fast decay). A slot past its budget gives
  up permanently; when every slot has, the pool answers a permanent 503
  that says the supervision budget is spent. With `app.debug:true`, `GET
  /_qn/peers` answers the `VM.peers` roster as JSON — dead incarnations,
  reasons, and respawned successors in one request. `qn
  qnlib/stress/web_soak.qn` grew two supervision phases: a chaos phase
  (mixed concurrent load while `/boom` kills workers every round, verdicts
  checked, healed-pool recovery wave asserted) and a give-up phase
  (budget spent, 503 permanence asserted), with `QN_SOAK_ROUNDS` /
  `QN_SOAK_CHAOS_ROUNDS` knobs.

- **`VM.abort`** — kill the current VM's process immediately
  (`std::process::abort`): no teardown, no lifecycle terminal, no exit code
  ceremony — a real crash, for testing supervision and death handling.
  (`Runtime.exit:` in a worker is a *stop* — the done terminal crosses
  first — so it never triggers a restart; `VM.abort` is the death.)

- **`serviceRestart` — manual restart of a dead hosted worker** (SUPERVISION.md
  slice 2, the respawn mechanics; policy automation is the next slice). A hosted
  worker's spawn is a frozen recipe — the portable block's captures froze when
  it first shipped, and `args:` are retained (channels as live values, re-shipped
  against the new incarnation's link) — so after a death, `svc.serviceRestart`
  re-runs it in a fresh isolate and REBINDS the root proxy in place: new sends
  just work, callers keep their reference. Restart follows *deaths only* (a
  stopped or running service refuses); the new incarnation must present the
  same class manifest, selector for selector, or the restart refuses to rebind
  and the service stays dead-but-retryable. Sends arriving during the restart
  window park and resume against the new incarnation (cancellable,
  `Async.timeout:`-composable). Everything the dead incarnation minted —
  sub-proxies, block handles, shipped endpoints — is permanently stale:
  touching one raises `PeerDiedError` with the new reason `#staleIncarnation`.
  Per-incarnation bookkeeping: fresh claims and lifecycle rows (`VM.peers` rows
  carry an `incarnation` number; `serviceEvents` after a restart answers the
  fresh incarnation's stream), merged boundary-profiling rows.
  **Extensions restart too**: `e.restart` re-runs the frozen spawn recipe and
  rebinds the handle in place — the installed classes keep working; instances
  minted by the dead incarnation are permanently `#staleIncarnation`. Same
  death-only and manifest-equality rules. And a hardening the work surfaced: an
  extension connection failing *under* a call is now always the typed death,
  even in the window where the child's exit is not yet reap-visible.

- **Peer lifecycle events + `VM.peers`** (SUPERVISION.md slice 1). Every spawned
  peer — hosted worker, plain worker, extension — now has a lifecycle stream:
  `w.events` on worker handles, `e.events` on extensions, `svc.serviceEvents` on
  hosted-object proxies answer a Channel of event Maps (`kind` =
  `spawned` / `stopped` / `died`, plus `reason` symbol and `message` for
  deaths). History is kept from spawn time, so a late consumer still sees the
  whole story; the channel closes after the terminal event; asking twice
  answers the same channel. An extension's first `events` ask arms an **OS
  child-exit watch** (kqueue on macOS, pidfd on Linux — observation only, never
  a reap), so an *idle* extension crash finally surfaces without anyone calling
  it. `terminate` and a dropped extension handle count as *stops*, not deaths —
  the supervision surface distinguishes an instruction from a failure even
  while `join` still reports the kill honestly as `PeerDiedError`. The roster
  is `VM.peers`: one row per peer with kind, backing, pid, status
  (`running`/`stopped`/`died`), the death reason/message, and an
  `eventsDropped` counter. All lifecycle wakes ride the logged scheduler path:
  a run that consumes events records and replays identically.

- **Fixed: the entombed-dispatch race.** A send racing a worker's death could
  `try_send` into a dispatch queue whose pump had already decided to exit —
  the request sat entombed in the closed channel's buffer, its reply lane
  never dropped, and the caller parked forever with no deadlock report
  (a live in-flight future kept the detector silent). The pumps now close and
  drain their queues on exit, so the racing caller gets the typed death like
  everyone else. Pre-existing since hosted dispatch; surfaced by CI load,
  reproduced and verified in a Linux VM.

- **Generic Map keys** — `Map(K V)` annotations now take any key type; the old
  "Map keys are String" resolve-time warning is gone (the runtime has keyed by
  any value since the hash-ladder map store). `V` stays runtime-tag-enforced
  exactly as before; `K` is checker-only: a definitely off-`K` key in a map
  literal, `at:`, `at:put:`, `containsKey?:`, or `remove:` warns (new kind
  `key-type`, suppressible with `"* allow:`), and `keys`/`values` on a
  `Map(K V)` receiver now type as `List(K)`/`List(V)`. Portability learned the
  matching wire truth: a captured `Map(K V)` with a non-String key type
  classifies non-portable (the wire's Map is String-keyed), where the old
  value-only rule would have called it shippable.

- **Portable-block classification** — the portability rules are now visible at
  compile time, computed by the same scan the isolate boundary runs (so tooling
  can never disagree with the runtime). Every block literal classifies as
  portable / conditional (naming the captures its verdict depends on) /
  non-portable (with the boundary's reason); `qn check --json` emits the
  classifications alongside diagnostics (the output is now an object of
  `{diagnostics, blocks}`), which the Quoin language server renders as
  whole-block highlighting with hover detail. And a block literal passed to
  `Worker.with:`/`host:with:`/`start:` that can never cross now **warns at
  compile time** (kind `portability`, suppressible with `"* allow:`) — the
  ship-time error, moved to edit time.

- **Boundary profiling** (`VM.boundaryStats` / `VM.boundaryReport`): every extension
  call is counted per (peer, class, selector) — calls, errors, bytes both ways, and a
  cost decomposition in microseconds: in-call wall time, time parked waiting for the
  peer's connection (contention), and the peer's own servicing time (`handler_micros`,
  a new append-only protocol field both SDKs now report; 0 from older SDKs). The
  rendered report sorts by total cost and flags transport-dominated hot rows — the
  chatty-vs-slow placement diagnosis (`docs/internal/ACTOR_OBJECTS.md` §7). Always on;
  rows survive a dead extension.
- Scheduler (experimental): **wake-log record/replay hooks**. `QN_WAKE_RECORD=<path>`
  records the scheduler's decision stream (ready-picks, yield preemptions, I/O delivery
  order); `QN_WAKE_REPLAY=<path>` re-runs the program forcing those decisions,
  reproducing a recorded concurrent execution exactly — the groundwork for deterministic
  replay debugging (`docs/internal/ACTOR_OBJECTS.md` §8). `QN_WAKE_LOG=1` keeps a ring
  of recent wake events and dumps it when the scheduler reports a global deadlock;
  `QN_WAKE_DEBUG=1` traces deliveries. Scope: replay re-performs real I/O and forces
  its order, so it covers programs whose external inputs are deterministic (timers,
  channels, file reads, schedule races); timing-dependent externals (extensions,
  sockets, subprocesses) report a divergence naming the mismatched op — replaying
  those needs result injection, a later arc.
- Extensions (experimental): **cross-process stack traces**. A failed extension call now
  carries an opaque stack blob — a Python extension sends its real traceback, a Rust one
  its error chain under a dispatch-frame line, and failures that cross the boundary
  several times interleave each side's segment in unwind order. The default traceback
  printer shows the blob fenced (`--- in extension ---`) at the failing call, and Quoin
  code reads it as `ex.remoteStack` (nil on ordinary errors). Old SDKs interoperate
  unchanged (message-only errors).

### Fixed

- **`[HTTP]ServerResponse.new:{ … var body = 'text' }` killed the connection.**
  The class contract says a String body converts to Bytes for convenience, but
  only the `body:` setter honored it — the construction form assigned the raw
  String, the wire serializer failed on it, and the transport's write guard
  (which conflates a write error with "peer gone") closed the connection with
  no response at all. Found by the web soak: the pool's 503 bodies never
  reached any client. `init:` now converts, matching the setter.

### Changed

- **Extension SDKs serve multiple lanes; the Rust SDK's handler bounds tightened**
  (breaking, Rust SDK only). An extension can now declare `lanes` — how many
  connections it serves concurrently — via `Extension::lanes(n)` (Rust) or
  `Extension(lanes=n)` (Python); the declaration rides `ManifestReturn` as an
  append-only protocol field, so older peers on either side interoperate
  unchanged at one lane. Both SDKs serve each accepted connection on its own
  thread over the shared object table. Consequences in the Rust SDK's types:
  registered instance types and handler closures now need `Send` (+ `Sync` for
  closures), and `Host::instance` — which lent a reference into the now-locked
  table — is replaced by `Host::with_instance(value, |v| …)`, which takes the
  instance out for the closure's duration (the same discipline as a call's
  receiver). Typical extensions compile unchanged apart from that rename; the
  Python SDK's surface is purely additive.

  Host-side, an extension declaring N lanes gets N connections and the same
  claim machinery hosted services run: calls to one instance serialize on its
  per-object mailbox, calls to different instances overlap up to the lane
  count, and class-side sends (constructors) contend only on lanes — so a DB
  extension's connections genuinely run queries in parallel. Extension claims
  now appear in `VM.claims`/`VM.claimsReport` beside services', and a claim
  cycle through extensions (even mixed with hosted services) raises the same
  catchable deadlock error at the task that closes it, instead of hanging.
  Extensions declaring nothing keep exactly the old one-connection behavior.
- **`WorkerService` is removed; hosting lives on `Worker`, and a block is the
  only constructor** (experimental, breaking). `Worker.host:'unit.qn'
  with:{ Pool.new:cfg }` runs the portable block *in* the worker after its
  unit loads and hosts the object it answers — the block is the constructor
  call site, so there is no separate class-name form (Quoin constructors are
  keyword selectors; a "default constructor with args" spelling would fit
  nothing). Bare `Worker.with:{ … }` is the unit-less version (qnlib classes
  only). Both work on either backing: on `backing:'process'` the block
  crosses as its **source text** plus a snapshot of its captures and is
  compiled in the child against its own unit (so it must come from source,
  not runtime assembly). `args:` fills the block's parameters at spawn —
  arity checked before anything ships; portable values snapshot, a portable
  block crosses as a callable, and a **Channel arrives as a live endpoint**
  in the worker (a channel *capture* still refuses: parameters are the honest
  spelling for live things). `Worker.start:` joins the family with
  `start:args:` and real `backing:'process'` (the job ships as source and
  `join` carries its value home). Proxies are
  **real installed classes** built from a manifest the worker sends at
  ready: introspection (`can?:`, `class.name`) answers locally, an unknown
  selector raises an honest MessageNotUnderstood instead of a round trip,
  class-side selectors dispatch to the hosted class, `==` compares hosted-object
  identity, and classes appearing for the first time in a return install
  themselves lazily — even a returned Block works, with remote `value:`.
  Worker processes whose parent dies now exit immediately instead of
  lingering as orphans.
- `WorkerService` (experimental): hosted services now speak the peer protocol and
  gained **hosted object returns** — a hosted method that returns a non-portable
  object no longer refuses: the object is kept in the worker and the caller gets a
  live **sub-proxy** for it, usable like any receiver (including as an argument to
  further calls on the same service, where it travels as a live reference; a dropped
  proxy's object is released on the next call). Hosted errors now surface with the
  worker's rendered stack as `ex.remoteStack`, and an unknown selector raises
  MessageNotUnderstood naming it. `serviceStop` is now explicitly worker-wide.
- `WorkerService` (experimental): **block arguments always cross now**. A portable
  block ships to a thread-backed service as a capture snapshot and runs *inside*
  the worker — one boundary crossing however many times the hosted method invokes
  it, and the method may keep it for later calls (the batch-API answer to chatty
  proxies; `docs/internal/ACTOR_OBJECTS.md` §3a). Every other block — one that
  captures live state, or any block to a process-backed service — crosses as a
  **handle** the worker invokes back in the parent (one round trip per invocation,
  write-captures see live parent state; portable blocks freeze their captures at
  send time on either path, so the backing never changes meaning).
- **Cross-isolate channels** (experimental, `docs/internal/ACTOR_OBJECTS.md` §6):
  a `Channel` now crosses to a worker — thread- OR process-backed — as a **live
  endpoint**: pass it through `Worker.send:`, as a hosted-service method argument
  (nested calls included), or get one back as a method's return, and the far
  side's `send:` / `receive` / `close` / `each:` relay to the owning isolate with
  channel semantics intact: values serialize at the boundary, one FIFO fairness
  order for local and remote waiters alike, **backpressure crosses** (a full
  buffer parks remote senders; a cap-0 rendezvous works at round-trip latency),
  `close` propagates both directions, and a value committed to a receiver that
  got cancelled is redelivered, never dropped. Several workers can hold endpoints
  on one channel — the worker-pool pattern (fan a jobs channel out, fan results
  in) works directly, across processes too. Sends of non-portable values on a
  shipped channel raise immediately at the sender. Not yet: re-shipping an
  endpoint onward, and `closed?`/`count`/`capacity` on an endpoint (the state
  lives with the owner) — each refuses with a clear error. Note the honest
  limitation: a wait cycle through channels *across isolates* is not detected
  (unlike hosted-object call cycles, which raise catchably) — `VM.ps` park labels
  show the shape.
- `WorkerService` (experimental): **per-object mailboxes and lanes**
  (`docs/internal/ACTOR_OBJECTS.md` §5.1). `host:class:lanes:` gives a service N
  concurrent lanes: calls to *different* hosted objects overlap (worker-side, each
  lane is a cooperative fiber — an object parked on IO doesn't block its
  isolate-mates), while calls to one object still serialize in arrival order (its
  mailbox, fairly queued). The acquisition discipline is deadlock-free by
  construction for everything except calls that genuinely wait on each other —
  and those now raise a **catchable deadlock error naming the cycle** at call
  time instead of hanging, verified end to end. Lanes work on **both backings**
  (`host:class:backing:lanes:`): a thread service runs one cooperative fiber per
  lane, a process service opens one conversation socket per lane (never frame
  multiplexing — each socket speaks the protocol unchanged), with identical
  semantics. Process services also now report their real servicing time
  (`ReplyMeta` crosses the socket pumps), so their `VM.boundaryStats` rows
  decompose into handler/transport/queue like everything else's.
- **`VM.claims` / `VM.claimsReport`**: live lock-shape observability for hosted
  services — per object: holder, re-entry depth, queued waiters and their wait so
  far; per service: lane occupancy and contention counters (acquisitions,
  contended, wait totals, queue high-water, deadlocks detected); plus the
  waits-for edges themselves, with the report calling out the longest live wait
  chain — the pre-deadlock warning. Hosted-service calls also now feed
  `VM.boundaryStats` rows beside extensions, with real `handler_micros` on both
  backings.
- Workers (experimental): **conversations, not round trips** — the peer protocol's
  re-entrancy now works for worker services, both backings. While a call is in
  flight the worker can invoke parent-held block handles (serviced on the caller's
  fiber), and code running that way can call back into the same service — the
  nested call rides the open conversation (strictly LIFO, depth-capped, mutual
  recursion errors catchably). A timeout or cancellation mid-conversation abandons
  it cleanly: the worker unwinds catchably and the service stays usable —
  cancelled *extension* calls still kill their peer (framed-socket desync), but
  worker services survive.
- Workers (experimental): the process-worker wire now speaks the **extension
  protocol's frames** instead of a bespoke envelope — one remote-peer protocol
  (`docs/internal/ACTOR_OBJECTS.md`). Two sockets per process worker: a conversation
  socket that opens with the `GetManifest` **version handshake** (a mixed-binary
  worker is now refused with a clear error instead of misbehaving) and carries
  control conversations, and a mailbox socket whose `send:`s are `Call` frames and
  whose done report is a `CallReturn*` terminal. Behavior of
  `spawn:`/`send:`/`receive`/`join`/`terminate`/`psTree` is unchanged; thread
  workers are untouched.
- Extensions (experimental): SDK manifests now list a class's selectors in **sorted
  order** (both SDKs). The Rust SDK serialized them in hash order, so the manifest's
  wire bytes differed from process to process for the same extension — semantically
  harmless, but wire bytes must be deterministic. No interop impact: hosts treat the
  lists as sets.
- Extensions (experimental): concurrent calls to one extension connection now **queue
  fairly** instead of raising a "busy" error — a waiting caller parks and is handed the
  connection FIFO when the in-flight call finishes, so `Async.gather:` over one long-lived
  connection (e.g. an `[ADBC]Connection`) just works. A cancelled waiter leaves the queue
  cleanly, and callers queued behind a dying extension fail fast with the usual catchable
  error.
- Extensions (experimental): **re-entrant calls now work** — a Quoin block an extension
  is invoking may call back into that same extension; the nested call's frames ride the
  same connection strictly LIFO while the extension services them, bounded by a nesting
  depth cap (a catchable error past 16 levels). In the Rust SDK a nested call to the
  outer call's own receiver (or one of its instance arguments) reports "no live instance"
  (they are taken out for the handler's `&mut`); Python has no such limit.

## [0.1.1] — 2026-07-13

The package release: installing, using, and writing Quoin packages — extension processes,
pure-Quoin source libraries, and executables on your `PATH` — plus interpolation fixes and
extension-SDK parity.

### Added

- `qn pkg install DIR` / `qn pkg list`: install a package folder into the per-user home
  (`$QUOIN_HOME`, default `~/.quoin`). Installed packages resolve via `use name:*` with no
  `QUOIN_PATH` entry — `$QUOIN_HOME/packages` is a built-in search root after the
  project-local `./quoin_packages/` and `$QUOIN_PATH` — and each `[bin]` manifest entry
  links into `$QUOIN_HOME/bin` (put that directory on your `PATH` once). The book gained a
  packages chapter (Part X).
- Source packages: a package's `[lib]` section names a folder of `.qn` units that
  `use name:*` loads through the ordinary pipeline (and `use name:unit` loads singly) —
  pure-Quoin libraries now ship as packages. Inside a package's units, `use self:`
  addresses the package's own units rather than the consuming project. A package unit
  that defines a bare-global class is refused at load time — package classes must be
  namespaced (reopening existing classes stays allowed). In a package with both
  `[extension]` and `[lib]`, the extension's classes install before the source units run.
- Extensions (experimental): the Rust SDK reaches resources-in-data parity with the Python
  SDK. A handler can return a structured `Value` tree carrying new live instances
  (`Value::instance`, e.g. a List of instances), register class-side selectors that return
  values rather than instances (`ClassBuilder::class_method`), and resolve live-instance
  references nested inside data arguments (`Host::instance`). No wire change — trees lower
  to the existing live-instance references (protocol v2, ext type 3) before encoding.

### Changed

- The package manifest is `quoin.toml` (was `extension.toml`) — a package is now any folder
  with a `quoin.toml`, providing any mix of `[extension]` (a subprocess providing classes),
  `[lib]` (source units), and `[bin]` (executables). Rename the file; the contents are
  unchanged.
- A `%'…'` interpolation literal is now lowered to string concatenation at compile time, so
  `%{…}` expressions see the full enclosing scope — including instance variables, which the
  old runtime recompilation silently read as nil (`%'%{@name}'` rendered empty). Methods
  containing interpolation literals are also no longer excluded from ahead-of-time
  compilation. Migration: a malformed `%{…}` in a literal is now a compile-time parse error
  instead of a runtime-catchable `ParseError`; sending `%` to a *computed* string keeps the
  reflective runtime path and its catchable `ParseError`.

### Fixed

- The reflective path (`%` sent to a computed string) now sees the caller's `self` too:
  `%{@ivar}`, `%{self}`, and `%{.send}` resolve against the calling method's receiver
  instead of silently reading nil — the interpolated unit compiles like `eval:self:`,
  without the top-level `self = nil` default that shadowed the caller's binding.

## [0.1.0] — 2026-07-12

The first release of Quoin: a small, dynamically-typed, object-oriented language in the
Smalltalk tradition — everything is an object, everything happens by sending messages, and
control flow is blocks responding to messages. It runs on a stack-based bytecode VM written in
Rust, with a tracing garbage collector and stackful coroutines.

`qn` is a single self-contained binary. The shipping standard library is compiled into it, so it
runs from any directory with nothing else installed.

### Language

- Objects, classes, and single inheritance, with instance variables (`@name`), class-side methods
  (`.meta`), and mixins.
- Blocks as first-class objects. `^` returns from the block; `^^` returns from the enclosing
  method.
- Declarations are strict: `var` for a mutable local, `let` for a binding. Assignment does not
  implicitly declare, and reading an unbound name raises `NameError` rather than yielding `nil`.
- Optional, gradual type annotations, checked by `qn check` and used by the optimizer. Nullable
  types (`Integer?`), generic collections (`List(Int)`), and block types.
- Literals for lists `#(1 2 3)`, maps `#{'a': 1}`, sets `#<1 2 3>`, symbols `#name`, and regular
  expressions. String interpolation is `%'total: %{a + b}'`. Comments start with `"`.
- Keyword-message selectors, including variadic ones.
- Errors are objects: typed `Error` subclasses, raised and caught by type, with multi-catch.
- `Class.exists?:#Name` asks whether a class is defined, without reading the name.
- `use` loads files explicitly — script-relative (`self:`), by glob, or by package.
- Fibers, generators, and lazy iteration; `^>` yields a value from a fiber.
- Placeholder statements for unfinished code: `...` raises `NotImplementedError`, `!!!` raises
  `UnreachableError`, and `???` warns and continues.

### Tooling

- `qn FILE` runs a program; `qn -e EXPR` evaluates one expression.
- `qn test [DIR]` runs the test suites in a directory, with `--coverage[=lcov|cobertura]` and
  `setup:`/`teardown:` and `setupAll:`/`teardownAll:` lifecycle hooks.
- `qn repl` — an interactive loop with editing, history, syntax highlighting, `$`-commands, and
  tab completion.
- `qn check` type-checks without running.
- `qn doc` generates API documentation for the current project — classes, methods, extensions,
  and commands — with `--check` to run every documented example and `--md` to render Markdown to
  HTML.
- `qn fmt` formats source. It re-parses its own output and refuses to write anything that would
  change the meaning of the program.
- `qn debug` — breakpoints, stepping, frame inspection, and evaluation in a frame, with
  `--break-on-throw` / `--break-on-uncaught`. `qn debug --dap` speaks the Debug Adapter Protocol,
  for editor integration.
- `qn highlight` prints syntax-highlighted source.

### Standard library

- Collections: `List`, `Map`, `Set`, `Bytes`, ranges, and a shared iteration protocol.
- Numbers: `Integer`, `Double`, `BigInteger`, `BigDecimal`, `Math`, `Statistics`.
- Time: `Instant`, `Duration`, `DateTime`, `Timestamp`, `TimeZone`, civil `Date` and `Time`, and
  `Span`.
- Data formats: `JSON`, `YAML`, `TOML`, `CSV`, `MessagePack`, `Base64`, `Hex`. A value's `asData`
  method controls how it serializes.
- Archives: `[Archive]Tar` and `[Archive]Zip`, read and write, with streaming gzip.
- Text: `String`, `Symbol`, `Regex`, and `Match` (named and positional capture groups).
- Cryptography: `[Crypto]Digest` (SHA-256/512/1, MD5, BLAKE3), `[Crypto]Hmac`, and
  `[Crypto]Random`.
- Identifiers: `UUID`, `ULID`.
- I/O: `[IO]File`, `[IO]Folder`, `[IO]Stdin`, and byte/string streams over files and sockets.
  Files are read *and* written: `[IO]File.create:` / `append:` return a buffered stream, with
  `[IO]File.write:to:` / `append:to:` / `read:` for the one-shot cases, plus `delete:`,
  `rename:to:`, `exists?:` and `[IO]Folder.create:` / `delete:`.
- OS: `[OS]Path` (lexical path manipulation), `[OS]Env` (read-only process environment), and
  `[OS]Process` for running subprocesses without a shell (`run:` / `start:`).
- Terminal: `Term` renders inline `[red bold]…[/]` markup to ANSI (stripping it when stdout is not
  a terminal), and `Log` provides leveled logging with lazy message blocks.
- Networking: `TcpSocket`, `TlsSocket`, `TcpListener`, `DNS` (the system resolver), an `[HTTP]`
  client, `[HTTP]Server`, and a `WebSocket` client.
- The `[Web]` framework: routing, requests and responses, and a worker pool.
- Concurrency: `Task`, `Async` (`sleep:`, `timeout:do:`, `gather:`), CSP `Channel`s, worker
  isolates, and a compute-offload pool for CPU-bound native work.
- Metaprogramming: `[Lang]Parser` and `[Lang]Node` expose the parser and AST as Quoin objects;
  `[Lang]Rewrite` makes span-precise source edits.

I/O is asynchronous and cooperative: a read or a write parks the task, it does not block the
scheduler.

File writes are **buffered** (16 KiB) and reach the disk on `flush!`, on `close`, or when the
program ends. Socket writes are **not** buffered, because a server writes a response and then
waits for the client; `flush!` is a no-op there, so the same code works over both.

### Extensions (experimental)

An out-of-process extension mechanism exists and is used internally, but is **not** a supported,
installable surface in v0.1 — the SDK crates are unpublished and the packaging and install story
lands post-v0.1.

- Extensions run out-of-process and speak a MessagePack wire protocol over a unix socket, so a
  crash or a hang in an extension cannot take the VM with it.
- SDKs for Rust and Python, at parity. An extension can provide real Quoin classes, hold
  resources, and call back into the host mid-call.
- An extension is packaged as a folder with an `extension.toml` manifest, loaded with
  `use <name>:*`.
- `adbc` (SQLite and PostgreSQL, via Apache Arrow ADBC) and `numpy` ship in the source tree as
  in-tree examples, not distributable packages.

### Performance

- The typed subset is compiled to native code ahead of time. This is on by default;
  `QN_AOT=0` disables it, and the interpreter path is always available.
- Untyped code is compiled speculatively from observed types, guarded and deoptimized on
  mismatch.
- Inline caches, devirtualized arithmetic and collection operations, and generics-aware dispatch.
- Cross-language comparisons are tracked in `bench/CROSS.md`; the environment variables that
  tune or disable each tier are in `docs/internal/ENV_FLAGS.md`.

### Known limitations

- A buffered file write stream is flushed on `close`, on `flush!`, and when the program ends —
  but **not on signal death**, exactly as in C. `[IO]File.write:to:` avoids the question.
- The extension SDK crates (`quoin-ext`, `quoin-ext-proto`) are not published to crates.io, so a
  third-party extension must vendor them. File-descriptor passing and a WASM tier are designed
  but not built.
- The debugger pauses the whole VM: there is no per-task debugging, and no watchpoints.
- The language reference (`docs/language/`) does not yet cover the whole shipped surface.
