# Async I/O Architecture — bridging `async-io`/smol to Quoin Fibers

Status: **Stages 0–8 implemented** (`Bytes` + `TcpSocket` land Stage 3; `TlsSocket` lands
Stage 4; `Async.timeout` is 5a; the `[HTTP]` client lands Stage 5; **Stage 6** is the lazy
`ByteStream`/`StringStream` layers — which also unblocked chunked HTTP — and file streams;
**Stage 7** is `TcpListener` — QN servers; **Stage 8** is `Channel` — CSP message passing
between tasks). The async-networking arc is feature-complete; remaining items are refinements.
Companion to `USE_ARCH.md`. See the *Staged plan* below.

## Decision

Build Quoin's networking on top of the existing **stackful Fiber** machinery
(`corosensei`). Add **one** new fiber suspension reason, `AwaitIo`, that carries a
plain-data I/O request up to the scheduler. The scheduler — the *only* async code
in the VM — fulfills the request through an **`IoBackend` trait** and resumes the
fiber with the result.

- **Backend (chosen): `async-io` / smol.** Small, modular, single-threaded
  executor (`!Send`-friendly, which fits gc_arena). TLS via `rustls`
  (`futures-rustls`); async DNS via the `blocking` thread pool under `async-net`
  or `async-io`.
- **HTTP: built in the Quoin stdlib**, not pulled from a crate. HTTP/1.1 over
  sockets+TLS, with `httparse` (the same parser hyper uses, runtime-independent)
  for robust header parsing. The smol-adjacent HTTP crates (`async-h1`, `surf`,
  `http-types`, async-std) are abandoned — we deliberately do not depend on them.
- **`IoBackend` is a seam, not just an abstraction.** tokio stays a deletable,
  swappable option for the future (heavy HTTP/2, QUIC, websockets), and WASM gets
  a host-import backend, all without touching anything above the trait.

## The core insight: async is contained to one function

Rust's function-coloring only propagates *up the call stack* through `.await`. We
stop it by **reifying "what I'm blocked on" as a value** instead of awaiting in
place. Quoin already does this for every other suspension: a fiber suspends by
bubbling a `YieldReason` up to the scheduler (`src/fiber.rs`, `src/runner.rs`).

So the async surface area of the entire VM is:

```
one async fn  (the scheduler loop)
   └─ calls  backend.perform(req).await    ← the single .await in the codebase
```

Everything below it — the interpreter (`vm.step`), every native method, the GC,
all of qnlib — stays synchronous and never names a `Future`. A native
`socket.read:` does **not** await; it suspends with a request *value*:

```rust
YieldReason::AwaitIo(IoRequest::Read { id, max })   // plain data, no Gc/Value
```

The backend turns request values into awaits. This gives us, almost for free:

1. **Swapping runtimes is one file** — smol → tokio → a `mio` loop → a WASM host
   backend changes only the `IoBackend` impl. The scheduler glue and every socket
   method stay put.
2. **Tests need no network** — a `MockBackend` returning canned bytes exercises
   the whole networking stdlib deterministically.
3. **The dangerous boundary is already policed** — `AwaitIo` is just a (long)
   yield, and the `no_gc_across_yield` lint already guarantees no `Gc`/`Value`
   survives a yield, which *forces* the request/result payloads to be plain data.

## Hard constraints (all already in force for fibers)

These are not new burdens — they are the rules the fiber system already lives by.

1. **gc_arena is single-threaded and `'gc`-bound; futures must be non-`Send`/
   non-`'gc`.** Use single-threaded `block_on` (from `futures-lite`/`async-io`),
   never a multi-thread executor. The scheduler future and its `FuturesUnordered`
   never need `Send`.
2. **Never hold the arena borrow across `.await`.** Every VM step runs inside a
   synchronous `arena.mutate_root(|mc, vm| …)` call; the `.await` happens
   *between* those calls, outside any arena borrow. The existing loop already
   yields cooperatively "so the scheduler can run the GC" — the await slots into
   that same gap.
3. **OS resources are never GC objects.** A `TcpStream` has a real `Drop` (closes
   the fd). The backend owns a side registry (`HashMap<StreamId, Box<dyn AsyncStream>>`,
   where `AsyncStream = dyn AsyncRead + AsyncWrite + Unpin`) outside the arena; QN
   sees an integer handle (`StreamId`) wrapped in a small GC object. The arena never
   owns a socket. See *Resource model & lifecycle* for why the registry is keyed on
   a generic stream rather than a concrete type, and how collected handles are
   reaped.
4. **I/O request/result payloads hold zero `Gc`/`Value`.** Enforced by
   `no_gc_across_yield`. On resume, the native method re-acquires `mc` and copies
   the plain `Vec<u8>` result into a GC string/bytes object (one copy — fine).
5. **Single-threaded cooperative scheduling.** CPU-bound QN in one fiber blocks
   all fibers — same model as asyncio/Lua/Node, and it keeps the GC race-free.
   True CPU parallelism (worker pool / multiple arenas) is explicitly out of scope.

## Data types (illustrative sketch)

```rust
// Plain data — no 'gc, no Value, no Gc. Crosses the yield boundary safely.
#[derive(Clone, Copy)]
pub struct StreamId(u64);

pub enum IoRequest {
    Sleep   { ms: u64 },
    Resolve { host: String, port: u16 },
    Connect { addr: SocketAddr },
    Read    { id: StreamId, max: usize },
    Write   { id: StreamId, bytes: Vec<u8> },
    Close   { id: StreamId },
    // Stage 4: TlsWrap { id, domain, insecure } (shipped);
    // Stage 6: OpenFile { path, mode } -> id (file streams); Stage 7: Listen / Accept
}

pub enum IoResult {
    Slept,
    Resolved(Vec<SocketAddr>),
    Connected(StreamId),
    Read(Vec<u8>),          // empty = EOF
    Wrote(usize),
    Closed,
    Err(IoError),           // plain error, mapped to a QuoinError on resume
}

/// Object-safe so the VM holds it as `Box<dyn IoBackend>`. Returns a boxed,
/// non-Send future the single-threaded scheduler owns and polls. Backends are
/// stateful: they own the resource registry.
pub trait IoBackend {
    fn perform(&self, req: IoRequest) -> LocalBoxFuture<'_, IoResult>;
}
```

Backends: `SmolBackend` (native, owns the `Async<_>` registry), `MockBackend`
(tests), later `TokioBackend` and `WasmHostBackend`.

## Resource model & lifecycle

**The request waist is per-operation, not per-resource.** Most resources are byte
streams — TCP, TLS-over-TCP, Unix sockets, pipes — and share `Read`/`Write`/`Close`.
So the registry is keyed on a generic `Box<dyn AsyncStream>` (`= dyn AsyncRead +
AsyncWrite + Unpin`), and only *creation* ops are resource-specific (`Connect`,
`TlsWrap`, `Listen`/`Accept`, `UdpBind`, …). Adding TLS, for instance, adds one
creation variant (`TlsWrap { id } -> StreamId`, producing a
`futures_rustls::TlsStream<…>` that drops into the *same* registry) and **zero**
changes to the byte ops or the scheduler.

`IoRequest`/`IoResult` stay a **closed, plain-data enum** rather than an open
`Box<dyn IoOp>`: that buys an exhaustive `MockBackend`, a single audit point for all
I/O, and — for the WASM goal — requests that marshal cleanly across a host boundary
(a `dyn IoOp` can't be serialized to JS). The cost is editing the enum + each
backend's match when a genuinely new *primitive* appears; backends are few and those
arms are mechanical. The **scheduler never changes** for any new resource — it only
ever sees `IoRequest`/`IoResult`/`IoBackend`.

**Lifecycle — explicit close is primary; a finalizer reap-queue is the backstop.**
fds are scarce *non-memory* resources, and GC timing keys off memory pressure, so
relying on collection to close sockets can exhaust the fd table long before a cycle
runs. The primary path is therefore explicit `socket.close`, ideally via a scope
combinator (`Socket connect:addr do:{ … }`) that closes on exit, exception-safe
(API settled at Stage 3).

The backstop catches forgotten sockets. The QN handle is a GC object holding only a
plain `StreamId`; the real stream lives in the backend registry *outside* the arena
(forced — we `.await` on it, and no arena borrow may be held across `.await`). When
the handle is collected, gc-arena runs its `Drop` — the codebase already relies on
this: `Gc<Fiber>` owns a `corosensei` stack freed on collection. That `Drop` must
not touch other `Gc` pointers and can't reach the async backend, so it does the one
safe thing: push the `StreamId` onto a non-GC reap queue (`Rc<RefCell<Vec<StreamId>>>`).
The scheduler drains the queue each turn and has the backend close the fd and remove
it from the registry. **The integer id is the only link** between the collected
handle and the live resource.

## Integration points (exact)

- **`src/fiber.rs`** — add `AwaitIo(IoRequest)` to `enum YieldReason`. It carries
  plain data, so `#[collect]` is trivial (no GC fields to trace).
- **`src/vm.rs`** — add `VmState::await_io(&mut self, req) -> IoResult`: suspends
  via `yielder.suspend(YieldReason::AwaitIo(req))`, and on resume re-derives
  `(vm, mc)` from the returned `VMContext` (exactly like `run_vm_loop`) and reads
  the result the scheduler stashed in a plain `VmState.pending_io_result` slot.
  Native I/O methods call this and convert the plain result into GC values.
- **`src/runner.rs`** — the scheduler in `compile_and_run_asts`. Two changes:
  (a) wrap the outer loop in single-threaded `block_on`; (b) add an arm to the
  `match res` block (currently `src/runner.rs:278`) for
  `CoroutineResult::Yield(YieldReason::AwaitIo(req))`. In its simplest form
  (**Level 1**, below) it — outside `mutate_root` — awaits `backend.perform(req)`,
  stashes the result, and resumes; **Level 2** replaces that single await with a
  `select` over the `FuturesUnordered` of *all* parked fibers' ops. The backend is
  owned by the runner (outside the arena) and passed into the driver.
- **`src/runtime/`** — new `net.rs` (Socket class over `StreamId` handles) and the
  HTTP layer; `timer.rs` / an `Async.sleep:` gains a non-blocking path via
  `IoRequest::Sleep`.
- **`qnlib/`** — `std:net/*`, later `std:http/*`, exposed through the `use` system.

## Concurrency model — two levels

The payoff (N fibers' I/O overlapping) needs a real scheduler on top of the
fiber primitive. Split into two levels so the seam can land before the scheduler:

- **Level 1 — the primitive (single in-flight op).** `AwaitIo` suspends the one
  running fiber; the scheduler awaits its single op, then resumes. This proves the
  round-trip (a value comes back into QN) but gives no inter-fiber overlap yet.
- **Level 2 — concurrent scheduler.** Maintain a **ready queue** and a
  **parked set**. Run ready fibers until each parks on `AwaitIo`; move a parked
  fiber's `backend.perform(req)` future into a `FuturesUnordered` keyed by fiber
  id. When the ready queue drains, `select` the `FuturesUnordered`; as each future
  completes, stash its result, mark that fiber ready, and resume it. Now I/O
  overlaps across all fibers — total wall-clock ≈ slowest op, not the sum.

Futures live in the scheduler's side table (non-GC, non-`'gc`), constructed from
plain `IoRequest`s over resources in the backend registry — completely decoupled
from the arena, which is touched only in the synchronous steps between awaits.

**The wait is readiness-driven, not a timed poll.** Awaiting an op parks the VM
thread in the reactor's `epoll`/`kqueue` (indefinitely, or until the nearest timer
for `Sleep`) and wakes it the instant the fd is ready — no interval, no spin. At
Level 1 the whole VM parks (only one fiber exists to wait on). At Level 2 the runner
parks *only when every fiber is blocked*; otherwise it keeps running other fibers'
synchronous steps, and the park is on the whole set of in-flight ops at once. So
`block_on` sleeps exactly when there is no CPU work left — the deliberate opposite
of the current CPU-bound step loop, which always has a next instruction.

### As implemented (Stages 1–2a)

The schedulable unit is a top-level **task** (`vm::Task`), kept distinct from a
guest `Fiber` (an asymmetric `resume`/`yield` coroutine): a task is scheduled by
the runner; a fiber is a generator driven from QN. Both ride the same `corosensei`
primitive. A task owns a private slice of `VmState` (the data/frame/native stacks
plus its guest-fiber chain, `current_fiber`/`resume_stack`); the *current* task
keeps that slice live in `VmState`, every other task stashes it in its `Task`.
`save_task_context`/`load_task_context` swap the slice at a task switch — the
I/O-parking analogue of `save_/load_fiber_context`, and it runs entirely inside one
`mutate_root`, so the swap is atomic with respect to collection and the task table
roots every parked task's stashed `Gc` context.

- **Stage 1 = Level 1.** `Async.sleep:` proves the round-trip; the whole VM parks.
- **Stage 2a = Level 2.** The run/test driver is a real scheduler: a ready queue
  plus a `FuturesUnordered<(TaskId, IoResult)>` (from `futures-util`), whose
  `.next().await` is the one reactor wait. The Stage-1 `pending_io_*` slots are
  unified into a single `Wake` delivery channel (`Io` / `Gather` / `GatherErr`).
- **The concurrency primitive is `Async.gather:[blocks] -> list`** (the only one in
  2a): spawn one child task per block, overlap their I/O, return results in spawn
  order (or propagate the first child error). Gather is structured — every task has
  exactly one parent awaiting it, so there are no orphans and teardown stays small.
  Detached `Task.spawn:`/join and cancellation are **Stage 2b** (deferred).

**Scheduling policy: round-robin at yield boundaries.** A task runs until it parks on
I/O, finishes, or reaches a cooperative-yield boundary (one `QN_BATCH` of instructions,
in every tier — the interpreter, `run_nested`, and AOT fuel checkpoints all emit the
same `CooperativeYield`). At a boundary the driver drains any *completed* background
futures without blocking (so expired timers and finished I/O wake their tasks even
while a CPU-bound task never parks), and if anything is runnable it stashes the current
task and requeues it FIFO. A task running alone pays two emptiness checks and keeps
going — measured free on the whole benchmark suite (±0.35%, noise). This replaced the
original run-to-block policy, under which a spinning task starved cancellation and
timers forever (RELEASE_PREP Tier 4b). What still holds: preemption is cooperative, so
a single long-running *native* call is not preempted mid-call, and cancellation lands
at the next checkpoint, within ~`QN_BATCH` instructions.
`QN_SCHED_STRESS` hardens this into a randomized scheduler for testing: it preempts at
*every* cooperative-yield boundary — forcing the `save_/load_task_context` round-trip
on every step — and picks ready tasks at random instead of FIFO, which also randomizes
gather-child and I/O-wakeup ordering. Seeded (SplitMix64) for reproducible replay; the
seed is announced once on stderr. The existing suites are expected to stay green across
a seed sweep, including combined with `QN_GC_STRESS`. See `src/tuning.rs`.

## Cross-cutting concerns

- **Cancellation.** If a parked fiber is killed/collected, the scheduler drops its
  in-flight future and issues `Close` on any owned resource (it owns both the
  future and, via the backend, the fd).
- **DNS.** `getaddrinfo` blocks; `IoRequest::Resolve` is fulfilled via the
  `blocking` thread pool (what `async-net` uses) so it never stalls the reactor.
- **Timers.** `IoRequest::Sleep` → `async_io::Timer`. This is the simplest possible
  `AwaitIo` (no sockets), making it the ideal Stage 1 proof.
- **WASM / embedding.** The browser has no sockets; a `WasmHostBackend` bridges
  `AwaitIo` to host imports (`fetch`, host-provided sockets). Same `IoRequest`
  values, different backend — the filesystem-agnostic goal from `USE_ARCH.md`
  carries straight over (this is the I/O analogue of `PackageResolver`).

## Dependencies

Added through Stage 2a:

- `async-io` (reactor) + `futures-lite` (`block_on`, `StreamExt`, combinators)
- `futures-util` (`alloc` only) for `FuturesUnordered` — the scheduler's set of
  in-flight ops

Still to add for later stages:

- ✅ `async-net` for async DNS + resolve-and-connect (Stage 3)
- ✅ `futures-rustls` + `rustls` + `webpki-roots`, **`ring`** provider, for TLS (Stage 4;
  `rustls-native-certs` for the OS trust store remains a future opt-in)
- `httparse` for HTTP/1.1 header parsing (Stage 5)
- `IoBackend::perform` returns a plain `Pin<Box<dyn Future<Output = IoResult>>>`
  (`'static` — each future owns the `Rc` clones it needs and borrows nothing from
  `&self`), which keeps the `FuturesUnordered` free of lifetime entanglement.

## Staged plan

Each stage is independently committable, builds clean, and ships a test.

**Stage 0 — backend scaffolding (no VM wiring). ✅ done.**
Add deps. Define `IoRequest`/`IoResult`/`StreamId`/`IoBackend`. Implement
`SmolBackend` (resource registry, `Sleep`/`Connect`/`Read`/`Write`/`Close`) and
`MockBackend`. *Test:* pure-Rust unit tests — `block_on` a connect to a local
`TcpListener`, echo bytes, assert; mock returns canned data. (`Resolve`/DNS deferred
to Stage 3.)

**Stage 1 — the seam (`AwaitIo` + async driver, single op). ✅ done.**
Added `YieldReason::AwaitIo`, `VmState::await_io`, the runner's `block_on` wrapper
and `AwaitIo` arm. Wired `Async.sleep:` to `IoRequest::Sleep`. (Also extracted the
fiber/coroutine fields into a `Scheduler` sub-struct.) *Test:* a `.qn` program that
sleeps and observes elapsed time — a value round-trips out to the backend and back.

**Stage 2a — concurrent scheduler. ✅ done.**
Promoted the run/test driver to a real scheduler over a top-level task table:
ready-queue + `FuturesUnordered` (Level 2), per-task context swap, the unified `Wake`
channel, and the `Async.gather:` primitive. Added `QN_SCHED_STRESS` (seeded
preemptive + randomized scheduling) to harden the state swap. *Test:* `Async.gather:`
of eight 30 ms sleeps finishes in ≈ 30 ms not 240 ms; results in spawn order; plus a
seed sweep over the existing suites. See *As implemented* above.

**Stage 2b — detached tasks (done).**
`Task.spawn:{block} -> handle`, `handle.join`, and `handle.cancel` — the unstructured
counterpart to 2a's structured `gather`. Introduces a third park flavor
(parked-on-task). The four design decisions below are settled; the staged plan follows.

Design notes:

- **Liveness model — fire-and-forget.** The scheduler owns a spawned task's liveness
  (it is rooted by `Scheduler.tasks`), *not* the handle. Dropping/collecting the
  handle never cancels a running task. This keeps task execution independent of
  collector timing, which is mandatory given `QN_GC_STRESS` (Model B — GC'ing the
  handle cancels — would make task lifetime nondeterministic and stress-dependent;
  cf. asyncio's "Task was destroyed but it is pending"). Structured "no task outlives
  its scope" guarantees, if wanted, come later from an explicit scope/nursery (the
  `gather` lineage), never from handle reachability.
  - **Outcome lives in the handle, not a retained slot.** *(Refined during 2b-i,
    replacing the reap-queue sketch.)* A running task roots its handle via `Gc`; on
    completion the scheduler writes the outcome (`status` + result/error) **into the
    handle** and frees the task slot immediately. The handle then lives by normal QN
    reachability and GC's with its result — no `Rc<RefCell>` reap queue, no GC-timed
    slot reclamation. The reap queue is the right tool for fds (a resource *outside*
    the arena; it stays in Stage 3), but a task result is a `Gc` *inside* the arena,
    so there is no boundary to bridge. The handle's `TaskId` is only dereferenced
    while `status == Running`, so a freed/reused slot is never touched through a
    finished handle.
  - **TODO:** expose `Task.running` — the list of currently-live task handles — as a
    `Task` class method. Fire-and-forget gives no built-in join-all; surfacing the
    running set lets a user write the structured fallback *in Quoin* (e.g. a scope
    helper that joins every running handle before returning) without baking nurseries
    into the core.
- **Program exit — abandon (not drain).** When `main` finishes, the program exits;
  still-running detached tasks are dropped (their coros and in-flight futures drop on
  teardown — zero new code, it is today's `break`-on-`Finished`). Matches Go / tokio /
  asyncio (program runtimes abandon; only server runtimes like Node drain). Drain is
  recoverable *on top of* abandon via `Task.running` + `join`; the reverse is not —
  making drain the default would force the opt-out to be *cancel-all*, saddling users
  with cancellation **ordering** hazards (logical deadlocks when A is mid-handoff to B
  and they cancel in the wrong order) that order-insensitive join-all is immune to.
  Optional: warn when exiting with N live detached tasks (à la asyncio's "task
  destroyed but pending").
- **Cancellation — cooperative unwind, honoring `finally`.** `cancel` sets a per-task
  flag; at the task's next yield point (a `CooperativeYield` step boundary, or a park
  resume) the scheduler injects a `Cancelled` throw that runs the existing exception
  machinery, so `finally`/ensure runs and resources close deterministically. In-flight
  futures are wrapped with `futures::abortable` so `cancel` interrupts a `sleep`/read
  promptly instead of waiting it out. Quoin's per-step `CooperativeYield` means even a
  tight CPU loop is cancellable (no uncancellable tasks — cooperative cancel's usual
  weakness doesn't apply here). `Cancelled` is **not swallowable**: `finally` runs but
  `catch` cannot suppress it (a task can't ignore its own cancellation). Abrupt-drop
  was rejected — it skips `finally`, which Quoin guarantees, and pushes cleanup onto
  the GC-timed reap backstop. Lands in 2b-ii (after spawn+join in 2b-i).
- **Join — multiple joiners, re-readable result (Promise/Future semantics).** A handle
  can be `join`ed any number of times by any number of tasks: parked-on-task is already
  a waiter *list*, and a finished task's result is already retained (liveness model
  above), so N joiners and re-reads cost nothing extra. `join` yields `Ok(value)`,
  re-throws the task's own exception (catchable, like `gather`'s first error), or
  signals a **catchable** joinee-cancelled error. The nuance: a task's *own*
  cancellation is the unswallowable `Cancelled`, but *observing another task's*
  cancellation through `join` is an ordinary catchable outcome — otherwise one `cancel`
  would virally, uncatchably cancel every joiner. One-shot (tokio `JoinHandle`) was
  rejected: it buys nothing here and fights the retention model `spawn` already needs.

Sharp edges (documented, accepted as user-error in v1 rather than engineered against):

- **Join an already-finished task** returns the retained result immediately, no park.
- **Self-join** (`h.join` from the very task `h` denotes) is a cheap guarded error, like
  2a's "a Fiber cannot resume itself".
- **Broader join cycles** (A joins B, B joins A) are *not* detected: both park forever
  and are abandoned on program exit (the abandon policy) — an actual hang only if `main`
  is in the cycle. Documented, not prevented.
- **`Task.running` is a snapshot, not a live view.** A handle can finish (or be reaped)
  between the snapshot and a later `join`, so the join-all idiom relies on `join` of a
  finished/reaped handle behaving — returning the retained result, or a clear
  already-collected error, never a crash.
- **Self-cancel / cancelling an already-finished or cancelled task** is a no-op (the
  latter two) or self-`Cancelled` at the next step (the former); never an error.
- Optional non-blocking `handle.status` (running / done / failed / cancelled) — nearly
  free since the state already exists, and lets code poll without parking.

Plan:

- **2b-i — spawn + join. ✅ done.** The `Task` handle (a GC object over a plain
  `TaskId`, like `StreamId`); `spawn_detached(mc, block) -> handle` (allocate a task,
  enqueue, return the handle — no park; `ready` moved into `Scheduler` so a native
  method can enqueue, which also retired `gather`'s `Spawned` hand-off and `Done{woke}`);
  the parked-on-task waiter list with `Wake::Joined` / `Wake::Failed` (re-raised,
  catchable); the **outcome-in-handle** model (no reap queue — see liveness note above);
  `Task.running`; `handle.status`/`done?`; a self-join guard. *Test:* a spawn/join round
  in `async_soak.qn` (spawn N, join all, check against the serial reference) plus Async
  suite tests — checksum-identical across plain / `QN_GC_STRESS` / `QN_SCHED_STRESS`.
- **2b-ii — cancel. ✅ done.** A new `QuoinError::Cancelled` (propagates like a throw,
  runs `finally`, but `catch:` re-propagates it). Per-task `cancel_requested` flag,
  mirrored to a live `Scheduler::cancel_current`; a checkpoint at `step_internal` and on
  each park-resume (`await_io`/`await_gather`/`await_join`) raises it. **One-shot:**
  `take_cancellation` clears *both* flags when consumed, so a preempt-reload during the
  ensuing `finally` doesn't re-raise (the bug found under `QN_SCHED_STRESS`). In-flight
  I/O is `futures::abortable`, so `cancel` interrupts a `sleep` promptly. `request_cancel`
  aborts the future / dequeues a join-parked task. `complete_detached` sets status
  `Cancelled` and delivers `Wake::JoinedCancelled`; `join` on a cancelled task is a
  *catchable* error. An infinite CPU loop no longer shields itself: the yield-boundary
  rotation (see *Scheduling policy*) lets the canceller run, and the flag lands at the
  spinner's next checkpoint (~`QN_BATCH` instructions). Remaining scope: a single
  long-running *native* call is not preempted mid-call, and cancelling a task parked on
  its *own* gather waits for the children. *Test:* a cancel-all round in `async_soak.qn`
  (checksum-stable across the stress knobs) + Async suite tests (cancel runs `finally`,
  `catch:` can't swallow it, `join` observes it, a spinner is cancellable).

**Stage 3 — `Bytes` + TCP sockets (done).**
The async *core* is done and generic, so a socket read already round-trips through the
scheduler and is cancellable (abortable) for free; Stage 3 is the QN surface, DNS, a
new `Bytes` primitive, and the resource lifecycle.

Design notes (all five settled):

- **`Bytes` — a binary-data primitive (prerequisite).** Quoin's `String` is UTF-8
  text and *cannot* hold arbitrary bytes (an image, a gzip stream, a TLS record), and a
  `Value`-per-byte list is wasteful — so socket I/O needs `Bytes`. Immutable
  `ObjectPayload::Bytes(Gc<Vec<u8>>)`, mirroring `String`'s `Gc<String>` (a GC leaf).
  The backend is *already* byte-based (`Read(Vec<u8>)` / `Write{bytes}` cross the yield
  as plain data), so `Bytes` is purely the QN-facing wrapper at the native boundary —
  one copy in/out, and `no_gc_across_yield` is satisfied because the `Gc` `Bytes` is
  never held across the suspend (the `Vec<u8>` is extracted *before* the await). The
  text boundary: `string.asBytes` (infallible UTF-8 encode), `bytes.asString` (throws on
  invalid UTF-8), `bytes.asStringLossy`. Min API: `size`/`count`, `at:` (→0–255),
  `from:to:`, `+`, `==`, `each:`, `Bytes of:#(…)` / `Bytes empty`; inspect = length +
  short hex preview. A mutable `BytesBuilder` and a `#b'HEX'` literal (the `#`-prefixed
  user-literal syntax, like `#(…)`/`#/…/`) are deferred.
- **Naming + hierarchy.** `TcpSocket` (not `Socket` — ambiguous with Unix sockets; and
  `Stream` is reserved for lazy streaming). Future (Stage 6): any conduit →
  **`ByteStream`** (lazy byte streaming) → **`StringStream`** (text) — conduit-neutral
  and *one class each*, shared by `TcpSocket`/`TlsSocket`/`[IO]File`, since the buffer
  and decode key only on the `StreamId`. **Lines are a text concept, not a byte one**, so
  `readLine` lives at `StringStream`, not `ByteStream`.
  This isn't just tidy — it's *necessary*: a fixed-size read is intrinsically a byte op
  (a UTF-8 code point is 1–4 bytes, and any `read:n` can land mid-sequence), so you
  cannot stream text as `{ read:n . asString }` (a chunk ending mid-character is invalid
  UTF-8 in isolation). The text layer must hold a **trailing-partial-byte buffer** and
  decode incrementally (`str::from_utf8`'s `valid_up_to`) — a *decoding* concern, which
  is why the buffer belongs to `TcpStringStream` and `TcpSocket` needs none. HTTP
  (Stage 5) does not block on these layers — `httparse` parses headers from raw
  `TcpSocket` bytes.
- **DNS folded into `Connect`.** The lower-level socket takes `'host:port'` and resolves
  internally (manual DNS is a rare need, a future class). `IoRequest::Connect { host,
  port }` resolves-and-connects in one op (async-net's connect does `getaddrinfo` on the
  blocking pool); a standalone `Resolve` stays available off the hot path.
- **`read:` semantics (byte-only on `TcpSocket`).** `read:n` returns *up to* n bytes
  (POSIX-style, may be short; empty = EOF), `readAll` loops to EOF. No buffer, no lines
  (those are `TcpStringStream`). `writeAll:` is complete-or-throw.
- **Errors are thrown.** `IoError` is a catchable exception (result objects fit poorly
  without generics). **EOF is not an error** — a read at end-of-stream returns empty
  `Bytes`, so `readAll` terminates cleanly; only genuine failures (refused, reset,
  timeout) throw.
- **Resource lifecycle — the reap queue's real home.** A `TcpSocket` wraps an fd
  (scarce, *outside* the arena), with three cleanup paths: (1) explicit `close`; (2) a
  scope combinator `TcpSocket connect:'host:port' do:{|s|…}` that closes on exit even on
  throw/cancel (`finally`, made cancel-safe in 2b-ii) — the idiomatic primary; (3) a GC
  **reap-queue backstop** — the handle's `Drop` pushes its `StreamId` onto a non-GC
  `Rc<RefCell<Vec<StreamId>>>` (the only thing `Drop` can do — it can't touch other `Gc`
  or reach the backend). This is the mechanism we *deliberately didn't* build for tasks
  (a task result is GC memory; an fd is an external resource — the boundary the reap
  queue exists to cross). The queue lives in `VmState`, the driver drains it between
  steps and closes **synchronously** (drop the stream; no `await`, no task context at
  `Drop` time). **Both** forms ship — scope as primary, bare `connect:` for sockets that
  must outlive a scope (a connection pool, accepted server sockets).

Sharp edges (documented, accepted): the reap backstop is GC-timed, so a leak-heavy
program can exhaust fds before a collection runs (`connect:do:` is the mitigation);
the backend's take-out/put-back enforces **one in-flight op per socket**, so two
concurrent tasks on the same `TcpSocket` → the second gets an `IoError`; double-`close`
is a no-op and read-after-close throws.

Plan:

- **3a — `Bytes`. ✅ done.** `src/runtime/bytes.rs`: the immutable type + `BytesClass` +
  the `String`↔`Bytes` conversions. *Test:* a `Bytes` suite (round-trip, concat, slice,
  `at:`, invalid-UTF-8 throws), green under the stress knobs.
- **3b — `TcpSocket`. ✅ done.** `src/runtime/sockets.rs`: `connect:` (bare) + `connect:do:`
  (scope), `read:n`, `readAll`, `writeAll:`, `close`/`closed?` over `Bytes`; backend
  `Connect{host,port}` (async-net) + sync `close`; the reap queue (`VmState` + driver
  drain). Errors thrown (a catchable string; structured `IoError` class is a noted
  refinement). *Test:* `tests/tcp_socket.rs` — the real `qn` binary against a Rust echo
  server: connect/write/read/close, scope close, use-after-close throws, and **8
  concurrent connections overlapping** (≈ one round-trip, not the sum).

**Stage 4 — TLS. ✅ done.** `TlsSocket`, sharing all byte ops with `TcpSocket` (both back
onto `NativeSocket` in `src/runtime/sockets.rs`; the two classes differ only in *creation*).
The proof that the per-operation request waist pays off: **one** new backend op and **zero**
scheduler/lint changes.
- **4a — backend.** `io_backend.rs`: `TlsWrap { id, domain, insecure }` — takes the stream
  at `id` out of the registry, runs the `futures-rustls` client handshake, and reinserts the
  `TlsStream` at the *same* id (in-place conduit swap; reuses `IoResult::Connected(id)`). On
  failure the stream drops (fd closed). Two lazily-built, cached `TlsConnector`s (secure =
  `webpki-roots`; insecure = an accept-any-cert verifier that still checks handshake sigs);
  the **`ring`** provider is pinned via `builder_with_provider` (with aws-lc-rs off there is
  no process-default provider — the default builder would panic). `MockBackend` returns the
  same id. *Tests:* a local rustls echo server via the `insecure` path (offline, no test-only
  trust plumbing) + an `#[ignore]`d real-host (`example.org:443`) check of the webpki path.
- **4b — `TlsSocket`.** `connect:` / `connect:do:` = `Connect` then `TlsWrap` (SNI from the
  host part); `wrap:host:` / `wrap:host:do:` upgrade an existing `TcpSocket` in place
  (STARTTLS et al.), **consuming** it — the plaintext handle is marked closed *without*
  reaping so its fd transfers to TLS instead of being closed (and its `Drop` backstop is
  disarmed). Each creation selector has an `insecure:` variant (cert validation off, for
  local debugging); the bare forms forward `false`. *Test:* `tests/tls_socket.rs` — the real
  `qn` binary vs a local rustls echo server (`insecure`): connect/write/read/close, scope,
  the `wrap:host:` upgrade (consumed `TcpSocket` throws on reuse), and 8 concurrent TLS
  connections overlapping. No new soak round — TLS adds no concurrency primitive (it rides
  the existing `await_io` path).

**Stage 5a — Timeout combinator. ✅ done.** `Async.timeout:ms do:{…}` (throws a catchable
`'timeout'`) and `… onCancel:{…}` (runs the handler on the deadline, returning *its* value;
`onCancel:{ nil }` is the non-throwing form). Deadline-cancellation on 2b-ii: the block runs
as a detached child raced against a deadline timer; the first to resolve wins and the loser
is disarmed. The race is **exact** because the new **timed-join** park (`YieldReason::JoinTimed`
+ `Wake::TimedOut` + a per-task park epoch) arms/disarms/resolves entirely inside the
single-threaded scheduler — the deadline is a `Sleep` leaf future in the reactor, and
`deliver_deadline` ignores a stale firing via the epoch (robust to slot-id reuse). On the
deadline the child is cancelled (its `finally` runs, in-flight I/O aborts via `abortable`),
then the handler runs / `'timeout'` throws; an *outer* cancel propagates `Cancelled` (handler
skipped); the block's *normal* errors propagate past `onCancel:`. Kept off `read:` (a timed
read is `Async.timeout:ms do:{ s.read:n } onCancel:{ nil }` — `nil` = timed out vs empty =
EOF). *Tests:* the `Async` suite (in-time, throws-on-deadline, `onCancel:` value/nil,
errors-propagate, `finally`-on-deadline, nesting) + a deterministic `timeout` round in
`async_soak.qn` (identical checksum across all four stress modes). Sharp edge: cooperative —
the deadline lands at a checkpoint, so a CPU loop feels it within ~`QN_BATCH` instructions,
but a single long-running *native* call is not preempted mid-call.

**Stage 5 — HTTP/1.1 client. ✅ done.** Pure Quoin in **`qnlib/net/http.qn`** (loaded on
demand via `use std:net/http`) over `TcpSocket`/`TlsSocket`, so HTTPS falls out for free.
The only native piece is **`[HTTP]Parser.parseHead:`** (`src/runtime/http.rs`, a thin
`httparse` wrapper returning `#( status reason headLen headers )` or nil-when-incomplete);
URL parsing, request building, and body framing are all QN. Surface (all under the `[HTTP]`
namespace): **`[HTTP]Client`** class-side convenience (`get:`, `get:headers:`, `post:body:`,
`post:body:headers:`, `request:`) — kept thin so a future stateful client can be an
*instance*; **`[HTTP]Request`** builder (`method:`, `header:value:`, `headers:`, `body:`,
`insecure:`, `send`); **`[HTTP]Response`** (`status`/`ok?`/`reason`/`header:`
case-insensitive/`headers`/`body`/`bodyText`). Bodies: **Content-Length** and
**connection-close**; **chunked is deferred to Stage 6** (it needs the lazy streams — the
client throws a clear error meanwhile). Timeouts compose via 5a; errors are catchable
strings. *Tests:* `tests/http.rs` (real `qn` vs local Rust HTTP servers — Content-Length
GET, POST echo, close-delimited body, case-insensitive headers; HTTPS via a local rustls
server + `insecure:`; an `#[ignore]`d real-host secure GET) + `qnlib/tests/23-http.qn`
(network-free URL-parse / request-build / Response unit tests).

**Stage 6 — Streams (`ByteStream` / `StringStream`). ✅ done** (6a `5697450`, 6b `72fe196`,
6c `06135ce`, 6d `3c68a51`). Conduit-neutral over TCP/TLS/files; chunked HTTP now decodes
(real-host verified); file streams via `OpenFile`/`async-fs`. Tests: `tests/byte_stream.rs`,
`string_stream.rs` (incl. 4-byte emoji + RTL), `file_stream.rs`, the chunked path in
`http.rs`. The plan below records the as-built design.
Two lazy buffering layers over any open conduit, each *consuming* the layer below
(transferring fd ownership exactly as `TlsSocket.wrap:` consumes a `TcpSocket`):
`TcpSocket`/`TlsSocket`/`[IO]File` → **`ByteStream`** (buffered bytes) → **`StringStream`**
(incremental UTF-8 + lines). Entirely *above the waist* — **zero** new scheduler/backend ops
for the buffering itself (it rides the existing `Read`/`Write`/`Close`); the only new backend
op is `OpenFile` (6d), so a file can join the stream world. Creation does **no** `await` — it
is a synchronous handle transfer.

*Conduit-neutral, one class each.* Once a conduit is a `StreamId` in the registry, the
buffer scan and the incremental UTF-8 decode key only on the id and are blind to whether the
bytes are TCP, TLS, or a file — so a file stream and a socket stream are the *same class*,
differing only in construction (the registry is already `dyn AsyncStream`). Hence unprefixed
`ByteStream`/`StringStream` (not `Tcp*`/`Io*`), built natively on a shared
`NativeStream { id, reap, closed, rbuf }` (`rbuf` = unconsumed read-ahead, doubling as the
trailing-partial-UTF-8 buffer). Native because `Bytes` has no `indexOf:` and no sub-codepoint
validation — the delimiter scan and `str::from_utf8().valid_up_to()` are byte-level concerns;
pure-QN HTTP sits on top.

*Why a buffer at all.* Lines are a *text* concept and a fixed `read:n` can land mid-UTF-8 (a
code point is 1–4 bytes), so you can't stream text as `{ read:n . asString }`. `StringStream`
holds the partial-byte buffer and decodes incrementally; `readLine` splits on the byte `\n`
(0x0A, which never appears inside a multibyte sequence) then decodes, so it's boundary-safe
for free. `ByteStream` is the same buffering minus the decode.

*Construction & consume semantics.* `aTcpSocket.byteStream` / `aTlsSocket.byteStream` /
`aFile.byteStream` (+ `.stringStream` convenience = `.byteStream.stringStream`), and the
explicit class forms `ByteStream.over: aConduit` / `StringStream.over: aByteStream` (+
`over:do:` scopes). Each consumes its source: the source's `id` **and** `rbuf` move up, the
source is marked **closed** (`closed? == true`, further ops throw) but its fd is **not** torn
down — it transfers — and its `Drop` reap-backstop is disarmed (no double-close; `.close` on a
spent handle is a safe no-op). `closed?` thus means "this handle is spent", not "the
connection is gone" — the same wrinkle `TlsSocket.wrap:` already has, kept for consistency
over a separate `consumed?` state. The topmost live handle owns the fd; closing it (or its
`Drop`) reaps.

*API.* `ByteStream`: `read` (drain buffer else one fill; empty `Bytes` = EOF), `read:n` (up
to n, may be short), `peek:n` (no consume), `readUntil:delim` (through and *including* the
first `delim`; returns the partial remainder if EOF hits first), `readExactly:n` (exactly n or
throw on premature EOF), `writeAll:`/`close`/`closed?`. `StringStream`: `readLine` (line as
`String`, trailing `\r?\n` stripped, `nil` at EOF), `eachLine:{…}`, `read` (available text,
partial trailing code point retained), `readAll` → `String`. `no_gc_across_yield` holds
because every fill reads `id` out, drops the borrow, awaits, then re-borrows to append — never
a `&mut` into native state across the suspend (`rbuf` is plain, non-`Gc`).

Sequenced *before* listeners deliberately: a server handed only raw `read:n` is painful for
line/record protocols. Not an HTTP prerequisite (`httparse` works on raw bytes), but it
*unblocks chunked* (6c) and is the natural substrate for server-side code.

Plan:
- **6a — `ByteStream`.** `src/runtime/streams.rs`: the `NativeStream` backing + buffer ops
  (`fill`/`read`/`read:`/`peek:`/`readUntil:`/`readExactly:`) + `writeAll:`/`close`/`closed?`,
  sharing `vm.socket_reap` and the `Drop` backstop; `byteStream` on `TcpSocket`/`TlsSocket`
  (+ `ByteStream.over:`/`over:do:`). *Test:* `tests/byte_stream.rs` — real `qn` vs a Rust
  peer: `readUntil:` with the delimiter split across reads, `readExactly:`, `peek:`, EOF.
- **6b — `StringStream`.** Same backing (consumes a `ByteStream`): `readLine`/`eachLine:`/
  `read`/`readAll`, incremental UTF-8. *Test:* `readLine` over lines split awkwardly across
  reads *and* across a multibyte UTF-8 boundary; `read` retaining a partial code point.
- **6c — chunked HTTP (the payoff).** Refactor `qnlib/net/http.qn` `send` to read the response
  over a `ByteStream`: head via `readUntil:'\r\n\r\n'` → `parseHead:`; body chunked (hex
  size-line + `readExactly:` + CRLF, loop to 0, consume trailers) / Content-Length
  (`readExactly:n`) / close (`readAll`). Delete the deferred-chunked throw. *Test:* a chunked
  server in `tests/http.rs`; flip the `#[ignore]` real-host secure GET to an actual pass.
- **6d — file streams (parity).** New `IoRequest::OpenFile { path, mode } -> Connected(id)` in
  `io_backend.rs` via **`async-fs::File`** (`AsyncRead+AsyncWrite+Unpin` → drops into the
  existing registry; blocking-pool under the hood, like async-net's DNS); `MockBackend`
  returns an id. `[IO]File.byteStream`/`stringStream`. *Test:* read a file via
  `StringStream.readLine`, incl. a line split across the buffer / a multibyte boundary.
  *Caveats:* a read-only file throws on `writeAll:` (a handle property, like read-after-close);
  files are seekable and sockets aren't — a sequential `ByteStream` ignores it (`seek:` would
  be the first file-only extra, YAGNI now); the existing sync whole-file `[IO]File` read/write
  stays — these are the async *streaming* entry alongside it.

No new soak round — streams add no concurrency primitive (they ride `await_io`), the call TLS
made. *Test (overall):* read-until-delimiter and `readLine` over a mock/echo peer, incl. a
sequence split awkwardly across reads and across a multi-byte UTF-8 boundary.

**Stage 7 — listeners/servers (`TcpListener`). ✅ done.**
Two backend creation ops (`Listen`/`Accept`) + a listener registry; `Listen` returns
`Listening { id, port }` (the actual bound port, so `:0` ephemeral binds are usable);
`Accept` parks until a peer connects and drops the accepted `TcpStream` into the shared
`AsyncStream` registry — zero scheduler change, rides `await_io` like every byte op. The
`TcpListener` class (backed by a `NativeListener`, reaped via the shared queue):
`listen:'host:port'`, `accept` (→ a connected `TcpSocket`), `acceptOnce:{|s| …}` (one
connection, scoped-closed), `acceptLoop:{|s| …}` (accept forever, each connection scoped;
the caller breaks out with a non-local return `^^` — which, like a throw/cancel, propagates
straight through the native loop, closing the in-flight connection first), `port`, `close`,
`closed?`. TLS server-side is deferred (needs a server cert/key config). *Test:*
`qnlib/tests/24-server.qn` — a self-contained QN server + client on one scheduler (no
external peer): `acceptOnce:` echo round-trip, and `acceptLoop:` handling two concurrent
clients then breaking via `^^`; `Async.timeout:`-guarded so a regression can't hang the suite.

**Stage 8 — channels (`Channel`). ✅ done.**
CSP-style message passing between tasks (`src/runtime/channel.rs`): `Channel.new` is an
unbuffered rendezvous, `Channel.buffered: n` queues up to `n` values; `send:`/`receive`/
`each:`/`close`/`closed?`/`count`/`capacity`. The first stdlib concurrency primitive that is
**pure in-VM coordination** — no I/O backend, no `IoRequest`/`SmolBackend` future — so it
follows the `gather`/`join` park/wake model, *not* `await_io`:

- **Park/wake.** A `send:`/`receive` with no ready counterpart registers the running task in
  the channel's waiter queue and suspends with a new `YieldReason::ChannelPark` (the driver's
  arm just parks the context, like `Join` — no backend op). A counterpart, or `close`, sets
  that task's `Wake` and re-enqueues it to `ready`. Three new `Wake` variants carry the
  outcome: `ChannelRecv { value }` (a receiver got a value), `ChannelSendOk` (a sender's value
  was accepted), `ChannelClosed` (woken by `close`). Registration-then-suspend is atomic
  (single-threaded, no yield between), exactly as `await_join` relies on.
- **State lives in the channel object** — the buffer (`VecDeque<Value>`) and the sender/
  receiver waiter queues — like `Map`/`Set`/`List`, *not* in the scheduler. So
  `NativeChannelState::trace_gc` roots the buffered values and each parked sender's pending
  value, and there is no reap/finalizer: when the channel object is collected, its queues go
  with it. The store-before-park discipline keeps `no_gc_across_yield` clean (a value is moved
  into native state before the suspend, never held on the native stack across it).
- **Cancellation.** A channel-parked task matches none of `request_cancel`'s I/O/join branches,
  so a new `Task.parked_on_channel` flag lets `cancel` make it runnable to observe the cancel.
  Its entry in the channel's waiter queue can't be reached from `request_cancel` (no `mc`), so
  it lingers as a **ghost** and is skipped when a counterpart next pops the queue (a waiter is
  "live" only if `parked_on_channel && !cancel_requested`). Cancel wins over a pending delivery
  — checked first on resume — consistent with `join`/`gather`.
- **Semantics.** Receive on a closed *and* drained channel returns nil (and ends `each:`);
  buffered values stay receivable after `close`; `send:` on a closed channel throws. `select`
  over multiple channels, an `Iterate` mixin (draining is destructive), and deadlock detection
  (a stuck main task exits silently, today's `break`-on-nothing-ready) are deferred.

*Test:* `qnlib/tests/38-channels.qn` (rendezvous, buffered fill/drain, sender/receiver
parking, fan-in, `close`+`each:`, cancellation, ghost recovery, `gather` producer/consumer,
heap-churn GC) — green under normal, `QN_SCHED_STRESS`, and `QN_GC_STRESS`; a Rust `trace_gc`
survival unit test in `vm_tests.rs`; and two soak phases (`channelPipeline`, `channelFanIn` in
`qnlib/stress/async_soak.qn`) whose checksum is identical across every stress combination.

## Deferred / open

- **HTTP/2, QUIC, websockets, connection pooling, proxies** — where tokio's
  ecosystem (h2, quinn, tungstenite, hyper-util) runs far ahead. If/when needed,
  add a `TokioBackend` behind the same trait rather than rewriting upward.
- **`hyper` 1.x core via a smol shim** — a maintained HTTP engine + HTTP/2 path
  while staying on smol, if hand-rolled HTTP/1.1 outgrows itself (its `hyper::rt`
  traits are runtime-agnostic; the adapter from `async-io` streams is small).
- **Structured concurrency / cancellation API in QN** (e.g. nurseries, deadlines,
  detached `Task.spawn:` + join) — this is **Stage 2b**, to design now that 2a's
  scheduler is in place; `Async.gather:` is the only 2a surface.
- **`Bytes` extras** — a mutable `BytesBuilder` (if concat churn shows up) and a
  `#b'HEX'` literal (the `#`-prefixed user-literal syntax; a parser change). Deferred
  until needed.
