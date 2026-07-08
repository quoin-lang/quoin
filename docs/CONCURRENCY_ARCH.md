# Concurrency Architecture — compute offload + worker isolates

Design survey for multicore Quoin, in the style of `FUTURE_EXT_ARCH.md`: what is
decided, why, and what stays open. Companion to `ASYNC_ARCH.md` (the
single-threaded cooperative scheduler this builds on) and `AOT_ARCH.md` (whose
invariants constrain — and unexpectedly enable — everything here.)

The one-line thesis: **gc_arena forbids exactly one architecture (a shared
mutable heap across threads), and it is the architecture we don't want anyway.**
The two tracks worth building — a compute-offload pool (C1) and share-nothing
worker isolates (C2) — require no GC change at all, and most of their hard 20%
already exists in the reactor seam, the extension wire, and the scheduler's
park/wake machinery.

---

## 1. Goal

Use the other 17 cores. Specifically:

- **Throughput for bulk native work** — hashing, codecs, regex over big
  buffers, `[Num]`-style numeric kernels — without blocking the VM thread
  (C1).
- **Horizontal scaling for task-shaped programs** — a web server sharding
  connections, pipeline stages, N independent computations — with real
  parallelism, not just concurrency (C2).
- **No semantic tax on the 99% case.** Single-threaded Quoin keeps its exact
  semantics, its GC, its dispatch/IC/AOT performance profile. Concurrency is
  something a program *opts into* at explicit seams, the same posture the
  extension system took.

Non-goal: shared mutable Quoin objects across threads (§8).

## 2. The crux is the arena, not the scheduler

What `gc_arena` actually forbids: `Gc<'gc>` is `!Send`/`!Sync`; `RefLock`
borrow flags are non-atomic; the write barrier assumes one mutator; collection
runs inside one thread's `mutate` fence. **No Quoin value may ever be touched
from two threads.** Everything else is negotiable.

What the VM's own discipline has already banked — the assets this design
spends:

- **Precise, enumerable roots.** The suspension invariants (no `Gc` on
  coroutine native stacks — `tests/gc_across_yield.rs`; no borrows across
  yields — `tests/borrow_across_yield.rs`) mean every root lives in
  `vm.stack`/`frames`/the swapped per-task and per-fiber slices. There is no
  conservative stack scanning anywhere. Any future GC evolution starts from
  the property most VMs never achieve.
- **A GC-free value codec.** The wire `DataValue` + the direct
  `value_to_wire`/`wire_to_value` walkers (`src/runtime/extension.rs`) are a
  deep-copy codec for structured values, wire-tested, with a worked-out
  taxonomy of what crosses a boundary as data, what crosses as a handle, and
  what refuses (blocks). C2's message layer is this, minus the socket.
- **Park/wake machinery that already models remote completion.** `await_io`
  parks a task; a future completes; the task wakes with a result in its `wake`
  slot (`src/vm_scheduler.rs:423-437`, `src/runner_driver.rs:454-469`). Ghost
  wakeups are already handled (`park_epoch`, `tests/park_identity.rs`);
  cancellation of in-flight work exists (`Task.abort_handle`,
  `vm_scheduler.rs:1231`).
- **The process is already multi-threaded at the edges.** The smol stack
  (`async-io`/`async-net`/`async-fs`, `Cargo.toml:68-72`) runs its own
  reactor thread for fd polling and a blocking pool for DNS/file-open
  (`io_backend.rs:451,634`) — threads that never touch the arena, fed and
  drained through plain data. C1 generalizes a pattern that already ships.
- **Plain-data boundary types.** `IoRequest`/`IoResult` carry owned
  `Vec<u8>`/`String`/ids — every field `'static + Send` today
  (`io_backend.rs:58-127`). `Bytes` and `Array` buffers are detachable owned
  `Vec<u8>` behind the `Gc` handle (`value.rs:239`, `array.rs:48-52,122`).
- **Thread-safe process globals, already.** The full `static` inventory is
  atomics/`Mutex`/`RwLock`/`OnceLock`/`thread_local!` — no `static mut`, no
  non-`Sync` global anywhere (§7 of the audit; e.g. the Symbol interner is
  `OnceLock<Mutex<HashMap>>` with leaked `&'static str`, `symbol.rs:31-49`,
  correct under concurrent interning today). The AOT epoch comment even
  anticipates this: "Shared across VMs: cross-VM bumps only cost conservative
  Bails" (`codegen/mod.rs:275`).

## 3. The tiers

- **C0 (shipped):** async-io's reactor thread + blocking pool; out-of-process
  extensions as multicore-across-processes. The status quo already overlaps
  IO and Python-side compute with the VM thread.
- **C1 (v1 SHIPPED):** a compute-offload pool behind the `IoBackend`
  seam — parallelism for bulk native ops on detachable data, zero semantic
  surface. v1 = the `Bytes` codec family (`src/compute.rs`; gather of
  8 × 4 MB encodes measured 4.4×; gates in `ENV_FLAGS.md`).
- **C2 (v1 SHIPPED):** worker isolates — one arena + one `VmState` + one
  cooperative scheduler per OS thread; message passing by deep copy through
  the wire walkers; tasks pinned to their worker. v1 = `Worker.spawn:` +
  send/receive/join lanes with the L2 handle-as-task property
  (`src/worker.rs`, `src/runtime/worker.rs`); pools/portable blocks next.
- **C3 (sketched, deferred):** shared *immutable* values (`Arc`-backed frozen
  Bytes/strings/collections) crossing workers by reference — the
  copy-cost-killer, added only when C2 traffic shows the need.
- **C4 (rejected):** shared mutable heap / GC replacement (§8).

---

## 4. Tier C1 — the compute-offload pool

### Mechanism: a new request kind on the existing seam

An offloadable native op does today what `Connect`'s DNS lookup already does:
park the task, run elsewhere, wake with plain data. Concretely:

- `IoRequest::Compute(ComputeJob)` where a job is **a label plus a pure
  `Send + Sync` closure over inputs the call site already detached** — a
  JOB, not an op: the pool is pure transport and never learns what the job
  does, so new families never touch the seam. The bound makes the
  eligibility rule a compile error rather than a review convention.
  Results stay a small closed plain-data enum (`ComputeOut`) — result
  shapes are structurally boring, and keeping them plain preserves
  `IoResult`'s derives. (v1 shipped as a closed op enum; replaced the same
  day — the enum walls in the input shape and centralizes what should be
  local.)
- The guest side never sees a new mechanism: a native method (say
  `Bytes.sha256` on a 100 MB buffer, or a `[Num]` matmul) detaches its
  buffer, calls `await_io(IoRequest::Compute(...))`, and rebuilds a Value
  from the `IoResult` on resume — exactly the `array_parts` →
  copy-through → rebuild shape the extension data plane uses.

**The one real wrinkle is `!Send` futures.** The driver's `FuturesUnordered`
holds `!Send` boxed futures under a single-threaded `block_on`
(`runner_driver.rs:19,378,454`; `io_backend.rs:129-134`). An offloaded op
therefore does NOT run *as* one of those futures — it is spawned onto a pool
(rayon or smol's `blocking`), and what goes into the local `FuturesUnordered`
is a trivial `!Send` future awaiting a oneshot channel whose sender lives on
the pool. The pool thread computes on owned data, sends the plain result,
async-io's waker machinery wakes the VM's `block_on` — the same wake path an
fd event takes. No arena access off-thread, ever.

### Eligibility rule (the C1 analog of AOT refusal)

An op may offload iff its inputs detach to owned `Send` data, it makes **no
callback into the VM** (no comparator blocks, no host reach), and its result
is plain data. `sort:` with a Quoin comparator block stays on the VM thread
forever; `sort` on a `[Num]` f64 buffer offloads. This rule is what keeps C1
semantics-free: an offloaded op is observationally a slow native op.

### Semantics decided

- **Parking:** identical to IO — the task parks, other tasks run, the
  scheduler stays cooperative. A single-task program still benefits: the VM
  thread is free to run *other tasks* while the pool crunches, and
  `Async.gather:` over N offloading calls parallelizes N-wide today-shaped
  code with zero new API.
- **Cancellation:** aborting the *await* (task cancel) detaches the waiter;
  the pool op runs to completion and its result is dropped (same
  deliver-and-ignore posture as an aborted blocking DNS lookup). Compute ops
  are not interruptible mid-flight — eligibility requires they be bounded.
- **Deadlock detection:** the `futures.is_empty()` check
  (`runner_driver.rs:417-438`) keeps working unmodified for C1, because the
  bridge future sits in the local set while the pool works. (C2 is the tier
  that must touch it — §5.)
- **Sizing:** pool threads default to `cores - 2` (leave the VM thread and
  async-io's reactor breathing room), `QN_COMPUTE_THREADS` to override, per
  the `QN_*` tunable convention.

### First candidates

Bytes hashing/compression/codecs (buffers detach today), regex scans over
big strings (pattern + buffer both detachable), msgpack/JSON encode of
already-walked trees, and — the designed-for case — the shelved `[Num]`
native backend, whose typed `NumBuf` Vecs are the ideal offload payload
(`memory: [Num] design`). Measurement discipline per house rules:
`profiling/compute-offload/` with crossovers, since small payloads will lose
to the round trip exactly like numexpr's gates did.

## 5. Tier C2 — worker isolates

### The model

One OS thread per worker; each owns an `Arena` + `VmState` + scheduler +
`SmolBackend`, created exactly as `runner.rs` does today (`Arena::new` +
`register_builtins` + prelude — self-contained per the audit, ~52 native
classes + qnlib compile per worker). Workers communicate by **message
passing with deep copy**; the message layer is the extension wire's value
taxonomy, in-memory:

- **Data** crosses via `value_to_wire` → (move the `DataValue` tree, which is
  plain `Send` data) → `wire_to_value` into the receiving arena. No socket, no
  msgpack — the `DataValue` tree itself is the transfer format.
- **Resources** (worker-owned sockets, extensions, big handles) cross as
  handles only if/when a use case demands it (open question; the `DvResource`
  ownership discipline — reap queues, owner checks — is the template).
- **Arbitrary blocks/closures refuse**, exactly as they do on the extension
  wire — but §10's PORTABLE BLOCKS carve out the restricted-capture subset
  that can cross, which is what the ergonomic layer is built on. The raw
  worker entry point stays source-shaped: a unit path or a class+selector
  (`Worker.spawn:'jobs/resize.qn'` — same decision the extension manifest
  made, and for the same reason).

**Tasks pin to their worker.** Task migration means migrating a Gc graph —
that is the shared-heap problem in disguise, and it is what makes
work-stealing schedulers hard. Placement happens at spawn; a worker is a
scheduling domain, not a thread in a pool. This is BEAM's model minus
preemption (within a worker, cooperation via fuel is unchanged).

### The Send-shaping audit (the real Stage-2 work)

The audit found the process globals already safe (§2) and exactly one
Send-hostile family: **code objects.** `StaticBlock` is `!Send` via
`SharedBytecode(Rc<Vec<Instruction>>)`, `SharedSourceMap(Rc<...>)`,
`decl_block: Option<Rc<StaticBlock>>` (`instruction.rs:9,41,88`),
`Constant::Block(Rc<StaticBlock>)` (`:148`),
`Instruction::NewWithFields(Rc<Vec<Symbol>>)` (`:365`), the runtime
`Block.template: Rc<StaticBlock>` (`value.rs:616`), plus one
`Cell<u8>` (`spec_state`, `instruction.rs:111`). The migration is
`Rc→Arc` + `Cell<u8>→AtomicU8` — wide but mechanical, with two watch-items:

- The JIT leaks `&'static Rc<StaticBlock>` into compiled code
  (`codegen/translate.rs:2031`, `helpers.rs:435`) — becomes a leaked `Arc`,
  fine, but the sites must be audited together.
- Per-VM runtime `Rc`s (reap queues, `SmolInner`, compiler `ClassTable`)
  are deliberately NOT converted — they never cross; keeping them `Rc`
  documents that.

Do the `Rc→Arc` migration as its own perf-measured slice: it swaps some
refcount traffic to atomic on clone-heavy paths (closure creation bumps the
template refcount — `bench.qn`/CROSS before/after per house rules).

Strictly speaking, raw source-entry workers could skip even this by
compiling their own source (share nothing, not even bytecode). But §10's
portable blocks — the mechanism the whole ergonomic layer stands on — ship
*template references* across workers, which puts `Rc→Arc` on the critical
path rather than leaving it a boot-time optimization (build order, §11).

### Cross-worker channels

Today's CSP channels are arena-local: `Value<'static>` buffers inside a `Gc`
object, single-scheduler waiter queues (`channel.rs:33-47`). A cross-worker
channel is a different animal and should be a different type (worker mailbox
first, generalized channels later — open question §12):

- Payloads are `DataValue` trees (converted at send time — so send cost is
  explicit and the sender's arena is never touched by the receiver).
- The shared structure is an ordinary `Arc<Mutex<VecDeque<DataValue>>>` +
  wake handles — plain Rust, no Gc.
- **Wakeup** rides the same bridge as C1: a parked receiver holds a local
  `!Send` future on a notification channel; the sending worker fires the
  notification; async-io wakes the receiver's `block_on`.
- **Deadlock detection must learn remote wakeups**: the
  `futures.is_empty()` check (`runner_driver.rs:417-438`) counts only local
  in-flight futures. A task parked on a cross-worker receive HAS a local
  bridge future, so the mechanical fix may be free — but the *semantic* one
  (a true cross-worker cycle: A waits on B, B on A) becomes undetectable
  locally, and the error message must stop promising otherwise. Accepted:
  distributed deadlock detection is out of scope (BEAM doesn't do it either).

### What is naturally already right

- Symbols intern globally under a Mutex — workers share the table, `Symbol`
  stays `Copy` + pointer-equal across workers (`symbol.rs:31-49`).
- The two `thread_local!`s (`fiber.rs:167` batch stats, `value.rs:448`
  Display cycle guard) become per-worker automatically — correct by
  construction.
- Extensions: per-`VmState`, and the two process atomics (ext ids, socket
  paths incl. pid) make two workers' spawns collision-free
  (`extension.rs:384-395`). Each worker owning its own extension processes is
  the correct semantic (they're resources).
- The AOT registry/epochs/template-id mint are shared atomics — safe; each
  worker compiles its own units into fresh template ids, so no false sharing.
  (Sharing *compiled qnlib* across workers is a boot-time optimization for
  later, behind the `Rc→Arc` migration.)
- GC pacing, `VM.stats` (Mutex/atomic-backed) — fine; `VM.stats` should
  eventually grow a `'workers'` section.

### Class/global story

Workers boot the full qnlib and then load their entry unit. Definitions made
after boot are **worker-local** — Ractor's answer, and the extension system's
(`install_ext_class` is per-VM). No global class mutation protocol; if two
workers must agree on a class, they load the same unit. Boot cost is a few
ms today; if worker fleets make it matter, the answer is a snapshot/fork
mechanism, not shared tables.

## 6. What stays single-threaded — deliberately

Per worker, everything that makes Quoin fast stays exactly as tuned:
uncontended dispatch/IC caches, the interpreter loop, `RefLock` object
access, the address-stable `vm.aot` raw pointers, collection between resume
segments. The AOT tier needs zero changes for C1 and only the (optional)
`Arc` migration for C2. This is the point of the isolate cut: the perf
identity built over the last arcs is preserved *by construction*, not by
re-audit.

## 7. Tier C3 — shared immutables (sketch, deferred)

When C2 message traffic shows real copy cost, add a `Value` payload variant
holding `Arc`-backed frozen data — big `Bytes`, strings, later frozen
collections — that crosses workers **by reference**. It traces as
`require_static` (no `Gc` inside), so gc_arena is indifferent; the wire
walkers pass the `Arc` through instead of copying. The audit confirms no
`Arc` exists in value payloads today (`value.rs:232-239`), so this is a
green-field variant with a freeze/thaw discipline to design (Ractor's
"shareable" rules; BEAM's refcounted binary heap is the 20-year precedent
that this exact cut — mutable graph private, big blobs shared — is the one
that pays).

## 8. Rejected: shared mutable heap (C4)

Replacing gc_arena with a concurrent collector (mmtk-rs or hand-rolled) plus
atomic object access would mean: safepoints across compiled code, write
barriers everywhere, `RefLock`→locking or confinement protocols, every
dispatch/IC/AOT structure re-audited for contention, and the loss of the
single-mutator assumptions the interpreter's hot loop is built on. That is
the OCaml-multicore project — years, for a semantics Quoin doesn't sell.
Every dynamic language that added threads late converged on isolates +
restricted sharing (JS workers, Ruby Ractors, Erlang processes); the shared
heaps that work (Java, Go) were born that way.

**What would reopen this:** a demonstrated workload class where per-worker
heaps + C3 sharing genuinely cannot express the program (fine-grained shared
mutable state with cross-thread invariants) AND matters to Quoin's users.
Record it, don't build it.

## 9. Prior art / design templates

- **BEAM (Erlang)** — per-process heaps, copy-on-send, refcounted shared
  binary heap, no distributed deadlock detection. The strongest validation
  of C2+C3; also of tasks-pinned (BEAM schedulers steal *processes*, but only
  because per-process heaps make migration trivial — the exact property
  arena-pinning trades away, accepted).
- **Ruby Ractors** — isolates bolted onto a mature single-threaded runtime:
  the shareable-value taxonomy, worker-local definitions, and the lesson
  that the API seam (what may cross) is the whole design.
- **V8 isolates / Web Workers** — arena-per-worker with structured-clone
  messaging; transferables ≈ C3.
- **The VM's own extension system** — `FUTURE_EXT_ARCH.md` §2's "the crux is
  the GC boundary" is this doc's §2 with the boundary moved in-process; the
  wire's data/resource/refused taxonomy, ownership checks, and manifest-style
  entry points transfer nearly verbatim.
- **smol/async-io** — already-shipped evidence that auxiliary threads + a
  plain-data seam coexist fine with the arena.
- **Pony/ORCA** — message-passing ownership transfer without copying;
  interesting for a far-future C3 extension (send-by-move of isolated
  subgraphs), not v1.

## 10. The library: portable blocks and the disappearing pool

Raw workers (`Worker.spawn:` + mailbox) are the L0 primitive almost no
program should touch. The question that shapes everything above them: can a
BLOCK cross a worker boundary? If yes — with restrictions — then pooling,
placement, and lifecycle all disappear into combinators.

### Portable blocks — the one new mechanism

A block is code (`StaticBlock` — `Send` after the §5 `Arc` migration) + a
captured env chain (`Gc`, can never cross) + `self`/home. Only the middle is
hostile. Define a **portable block**: every free name is either

- a **read-only capture of a wire-representable value** (numbers, strings,
  Bytes, data collections — anything `value_to_wire` accepts), or
- a **global** (class/constant), or
- nothing.

Write-captures, `^^`, and a data-bearing `self` refuse, loudly, at submit
time. A portable block crosses as `(template reference, deep-copied capture
snapshot)` — captures ride the same walkers as any message.

**SHIPPED (v1)** as `Worker.start:{...}`: the scanner (`scan_portable`,
`src/worker.rs`) recurses nested literals incl. fused constants, collects
free reads + global references, and refuses write-captures / `^^` /
self-and-@fields / guarded blocks / class-method definition; the worker
verifies the global list against its own globals before running (clear
error over silent nil), rebuilds the closure over a snapshot `EnvFrame`,
and `join` returns the block's VALUE. Pools/combinators (L1/L3) still
ahead.

Every piece already exists somewhere in this VM: the compiler's capture
analysis (the AOT candidacy prescans classify free names, write-captures,
`^^` — `compiler/mod.rs`), the B3b cold-path materialization already builds
closures over environment *snapshots*, and the wire walkers are the copy
codec with the refusal taxonomy worked out. Ruby's Ractors proved the
semantics (isolated Procs with shareable captures) are ergonomic enough in
practice.

The wrinkle to design early: a portable block referencing a user global
(`{ |u| Fetcher.checksum:u }`) needs `Fetcher` defined in the worker. The
free-global set is statically knowable, so the pool checks at submit time:
v1 errors naming the missing global (or the program preloads via
`pool.use:'fetcher'`); the later magic version auto-ships the defining unit,
BEAM-code-loading style (units know their source).

### The layers

- **L0 — `Worker`** (§5): spawn by unit path, mailbox, join. Explicit
  everything. The floor, not the surface.
- **L1 — `WorkerPool`**: hides lifecycle and placement — lazy boot, warm
  reuse (qnlib compiled once per worker, not per job), idle reaping,
  crash-respawn with the in-flight job failing as a catchable error
  (supervision-lite; Erlang-style supervision trees are a later library, not
  runtime). `pool.run:` takes a portable block, returns a handle.
- **L2 — the handle IS a parked task.** The highest-leverage decision in the
  layer: `pool.run:` returns something that parks on the mailbox bridge
  exactly like an IO-parked task — so **every existing async combinator
  composes for free**: `Async.gather:` over worker jobs, `Async.timeout:do:`
  around one, cancellation propagating as a cooperative cancel-request to
  the worker-side task. Zero new control-flow vocabulary. Worker errors
  return as catchable errors (the wire's `CallReturnError` shape).
- **L3 — parallel combinators**, where pooling vanishes:

  ```quoin
  "* the 90% case - no pool, no worker, no placement visible anywhere
  var thumbs = images.parallelCollect:{ |img| renderThumb:img };
  var total = readings.parallelReduce:0 with:{ |acc x| acc + (score:x) };

  "* explicit pool only for isolation or sizing; composes with Async
  var pool = WorkerPool.size:4;
  var jobs = urls.collect:{ |u| pool.run:{ checksum:u } };
  var sums = Async.gather:jobs;
  ```

  Combinators run on an implicit default pool, auto-chunk (amortize
  per-message copy), preserve order, and — the numexpr lesson transferring
  directly — **gate on size**: `parallelCollect:` over ten cheap items runs
  serially. Same measured-crossover discipline, instrumented through a
  `VM.stats` `'workers'` section (jobs, bytes copied, per-job µs) so gates
  are tuned from data. `parallelReduce:` documents its associativity
  requirement (per-worker partials, then combine). `parallelEach:` is
  deliberately absent in v1: worker-side effects don't touch the caller's
  heap, so it only makes sense for IO and invites confusion.

  **SHIPPED (v1)** as `qnlib/core/10-parallel.qn` over a lazily-started warm
  pool of block-job workers (blocks now cross the lanes as portable-block
  MESSAGES, recursively — the user's per-item block rides as a capture of
  the chunk job). Measured honestly (profiling/parallel-combinators/):
  cheap per-item blocks NEVER pay (copy-bound at every count — the real
  eligibility knob is per-item work, which no gate can see; stated in the
  combinator docs); heavy blocks reach 2.7× at the measured pool sweet spot
  of 4 (after the shared-template false-sharing fix — shipped templates
  LOCALIZE per worker, profiling/worker-scaling/notes.md; the residual >4
  in-process ceiling is powermetrics-confirmed platform policy: extra
  same-process threads land on a sibling cluster clocked at ~1/3 frequency
  (1.5 vs 4.6 GHz) that macOS won't clock up, and aggregate throughput
  drops even under dynamic feed-on-completion chunking — which shipped
  anyway, as the robust design for variable per-item cost. Cap 4 stands,
  triple-confirmed; the escape hatch if it ever matters is process-backed
  pool workers). One flight
  at a time (no per-job lane addressing); concurrent calls fall back
  serial. Refusal semantics are UNIFORM: `Block.portable!` (the shape
  scan) runs on every path incl. serial fallbacks, so a write-capturing
  block refuses identically at any input size and inside workers.
- **L4 — `WorkerService`**, the stateful story — and the extension system
  pays off again: Phase-3 extension-backed classes already install proxy
  classes whose method sends dispatch over a wire with the
  data/resource/refused argument taxonomy. A service is the identical
  machinery with the socket swapped for a mailbox: host a class in a
  dedicated worker, get a proxy whose sends are RPC — sticky state,
  serialized access (an actor, effectively).

  ```quoin
  var index = WorkerService.host:'search/index.qn' class:SearchIndex;
  index.add:doc;
  var hits = index.query:'quoin';
  ```

  *Extensions : processes :: services : workers* — the proxying, argument
  classification, error transparency, and ownership/reap discipline are
  already designed and stress-tested; C2 reuses rather than invents.

  **SHIPPED (v1)**: `WorkerService.host:class:` (+ `backing:` — 'thread'
  now, 'process' reserved with a loud error). The proxy forwards through
  the dispatch MNU seam (lookup-miss branch only — the hot path never pays)
  in both the interpreter and the compiled outcall arm; callers serialize
  on a one-token internal lane (fair parking on existing machinery, no
  crossed replies); the hosted loop is synthesized guest code driving
  `perform:args:` (new Object reflection, MNU-correct), so hosted methods
  do real IO. Errors — throws, MNU, non-portable args — are transparent
  and catchable; boot failures surface from `host:` with the worker's own
  message; `serviceStop` waits for quiet, then joins.

  Services scale on a DIFFERENT AXIS than the pool, and the cluster
  ceiling (§ the scaling investigation) matters far less there. The pool's
  shape — N threads all CPU-saturated at once — is exactly what macOS
  punishes past the fast cluster. A service fleet under realistic load is
  MIXED: at any instant some workers are parked on lanes, some mid-request,
  few simultaneously compute-bound — parked threads are free, and the
  runnable set stays at or under cluster size most of the time. And where
  the pool scales one operation's throughput over stateless snapshots,
  services scale sustained concurrent load over SHARDED STATE (an index, a
  session store, a connection owner per shard) — the thing snapshots
  structurally cannot do. For the server shape that motivated C2, services
  are the real scaling surface; the pool is the special-purpose tool for
  data-parallel bursts.

  **Backing is a spawn-time choice, specified from day one.** The recorded
  escape hatch for the cluster ceiling is process-backed workers, and a
  service is the one worker shape where that's nearly free: long-lived,
  state-owning, message-only interface — precisely the extension model
  already in production. The proxy must not care what's behind the
  mailbox: a compute-heavy service escapes the DVFS policy with process
  backing; a chatty low-latency one stays in-process for the cheap lanes.
  Choosing per service beats anything the pool could do here — coarse RPC
  tolerates a process boundary; the pool's fine-grained chunk traffic
  would not. Retrofitting this later would mean two proxy kinds forever
  (the same argument that put handle-as-task into C2 v1 on day one).

### What stays visible, on purpose

Copy semantics leak deliberately: arguments and results are deep copies —
identity lost, mutations don't travel; user-class instances refuse until C3
gives them a story. The posture is the wire's posture: loud, early errors
("this block captures a mutable binding", "Fetcher is not defined in the
worker — preload its unit") over silent surprises. Scheduling is
nondeterministic; results of order-preserving combinators are not. And the
C1/C2 lanes stay distinct in v1 — `parallelCollect:` over a `[Num]` buffer
should eventually route to the offload pool instead of workers, but lane
unification behind one combinator is an optimization to earn with data, not
a founding requirement.

## 11. Build order

1. **C1 offload pool** — `IoRequest::Compute`, the Send bridge, 2-3 op
   families (Bytes hashing/codec, regex, msgpack encode), crossover
   measurement. Small, self-contained, immediately useful — and it builds
   the oneshot-bridge machinery C2's wakeups and the L2 handle reuse.
2. **`Rc→Arc`/`AtomicU8` for code objects** — PROMOTED onto the critical
   path (it was "decide by boot cost" until §10: portable blocks ship
   template references, so the whole ergonomic layer stands on `Send` code
   objects). Perf-verified per house rules — closure creation bumps template
   refcounts, so the atomic swap gets measured before anything builds on it.
3. **C2 v1** — `Worker` (L0, unit-path entry) + `WorkerPool` (L1), mailbox
   send/receive with `DataValue` copy, **the job handle designed as a parked
   task from day one** (L2 — retrofitting composition later would mean two
   handle kinds forever), deadlock-message honesty, `VM.stats` `'workers'`
   section, stress under `QN_SCHED_STRESS` per worker.
4. **Portable blocks** — the capture-snapshot spawn path + submit-time
   free-global check; `pool.run:{...}` and block-shaped `Worker.start:{...}`
   become real.
5. **L3 combinators** — `parallelCollect:`/`parallelReduce:with:` on the
   default pool, auto-chunking + size gates with measured crossovers.
6. **L4 `WorkerService`** — the ext-class proxy machinery over a mailbox.
7. **C3** — only when C2 traffic data demands it.

Parallel track, any time after (1): **`[Num]` on C1** — revisit the shelved
native backend WITH offload in its design (the evaluator becomes a
`ComputeOp`), so its big-array wins use cores from day one. It shares no
dependency with the C2/library line.

## 12. Decided vs open

**Decided**

- No shared mutable Quoin heap; gc_arena stays (C4 rejected, reopening
  criteria recorded).
- C1 rides the `IoBackend` seam as a request variant; offload eligibility =
  detachable owned inputs, no VM callbacks, plain-data result.
- Offloaded ops are not interruptible; cancel = abandon the wait.
- Compute requests are label+closure jobs (`ComputeJob`), not a central op
  enum; ops are open, result shapes (`ComputeOut`) are a small closed
  plain-data enum. No registration mechanism — in-tree families write
  closures; out-of-tree code is extensions, which are out-of-process and
  bring their own parallelism.
- C2 = arena-per-worker isolates; tasks pinned at spawn; messages deep-copy
  through the `DataValue` walkers; blocks refuse; worker entry is
  source-shaped, not closure-shaped.
- Post-boot definitions are worker-local; workers boot full qnlib.
- Per-worker extension processes; no cross-worker resource sharing in v1.
- Pool/worker tunables use `QN_*` naming (`QN_COMPUTE_THREADS`,
  `QN_WORKERS`, ...).
- **Portable blocks** are the block-crossing mechanism (read-only
  wire-representable captures + globals; write-captures/`^^`/data-`self`
  refuse at submit time); arbitrary blocks still refuse.
- **Worker/pool job handles are awaitable tasks** — they compose with
  `Async.gather:`/`timeout:do:`/cancellation with no new vocabulary.
- Parallel combinators auto-chunk, size-gate on measured crossovers, and
  preserve order; `parallelReduce:` requires associativity; no
  `parallelEach:` in v1.
- `WorkerService` reuses the extension-backed-class proxy machinery.
- Service BACKING (in-process thread vs child process) is a spawn-time
  option in the L4 design from day one; the proxy is backing-agnostic.
  Process backing is the sanctioned answer to the macOS cluster ceiling
  for compute-heavy services.
- `Rc→Arc` for code objects is on the C2 critical path (not optional).

**Open**

- Mailbox vs generalized cross-worker `Channel` API (lean: mailbox first —
  the channel type is arena-local by construction and shouldn't be
  overloaded; revisit once real programs exist).
- Whether C2 v1 ships before or after the `Rc→Arc` migration (workers can
  compile their own source; decide on measured boot cost).
- Resource handles across workers (socket handoff for an accept-and-dispatch
  server is the likely forcing function; the `DvResource` ownership pattern
  is the template).
- C3 freeze semantics (explicit `freeze` vs frozen-by-construction literals)
  — defer until C2 data exists.
- Snapshot/fork worker boot (only if fleet startup cost measures as real).
- Free-global provisioning for portable blocks: error + `pool.use:` preload
  (v1) vs auto-shipping the defining unit (the BEAM-style magic; needs units
  to carry provenance).
- Default-pool sizing and idle-reaping policy.
- Lane unification: routing `parallelCollect:` over offload-eligible data
  (`[Num]`/Bytes) to C1 instead of workers behind one combinator.
- Supervision beyond crash-respawn-with-catchable-error (restart strategies,
  linked workers) — a library concern, deliberately not runtime.
- Fire-and-forget service sends: ordering/delivery guarantees, backpressure
  on the service mailbox.
- Process-backed service transport: in-memory lanes don't cross a process
  boundary — likely the extension wire's UDS + msgpack verbatim, which
  would make a process-backed service nearly indistinguishable from an
  extension (worth unifying rather than paralleling).
