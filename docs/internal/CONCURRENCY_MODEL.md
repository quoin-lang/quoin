# The Quoin concurrency model — the stance

*Status: adopted 2026-07-13. This is the referee document: every future API, protocol,
and scheduler change is measured against these guarantees. The mechanism-level survey
lives in `CONCURRENCY_ARCH.md`; the extension protocol in `quoin-ext-proto/PROTOCOL.md`;
the actor-objects design in `ACTOR_OBJECTS.md`. Rationale was settled in the 2026-07-13
long-term planning discussion.*

## The model in one sentence

**Cooperative within, isolated across**: inside one VM, single-threaded stackful fibers
with park-don't-block I/O — no data races exist, and invariants hold between yield
points; across VMs, share-nothing isolates — workers, extensions, pools — where a crash
is a contained, catchable event and nothing is ever shared, only sent.

The north star beyond it: **objects are the only abstraction; locality is a property.**
An "actor" is not a new concept — it is an object that happens to live in another
isolate, addressed by ordinary message sends. Extension-backed classes already prove
this: `conn.query:sql` is a plain send whose receiver lives in another process, in
another language.

## The guarantees

These are promises, not implementation notes. Breaking one is a breaking change to the
language, whatever the version number says.

1. **No function coloring — ever.** Any code can park: stackful fibers mean
   `Async.sleep:`, a channel receive, or a remote send suspends the fiber wherever it
   is, however deep the ordinary call stack. No Quoin API may require an `async`
   annotation, a special calling form, or a split sync/async surface. (Corollary: no
   future-proxy invocation form — see guarantee 3.)

2. **No data races, by construction.** One VM runs one fiber at a time; a task's
   invariants hold from one yield point to the next. Parallelism exists only across
   isolates, which share nothing. There is no plan — and there will be no plan — for
   shared-memory threading inside a VM.

3. **A send is a send, and it parks.** Message sends — local or remote — are synchronous
   from the caller's fiber. Concurrency is expressed by *composing blocks*
   (`Task.spawn:`, `Async.gather:`, channels), never by decorating call sites.
   Throughput to a single peer is an **API-design problem** (batch the call — the
   `evalGraph:`/`apply_block` lesson), not a transport problem; per-send futures would
   be coloring through the back door and would subsidize chatty remote APIs that the
   ~10µs-per-round-trip syscall floor punishes anyway.

4. **The boundary law.** *Values* cross isolate boundaries by the portable-value rules —
   a submit-time scan (no write-captures, no `^^`, no `self`/`@fields`, snapshots of
   free reads) that fails loudly before anything runs. *Objects* never cross: they are
   **addressed**, by sends to a remote receiver, with host-driven lifetime (dropping the
   last local reference releases the remote object, batched — the `ExtResource` reap
   pattern). The **wire data model** is the only form in which a value exists on a
   boundary: byte-encoded when crossing a process, the tree itself moved in memory
   between same-process isolates (as the worker lanes already do). Either way a GC
   reference can never leak across, by construction.

5. **The cost gradient is visible and is not a bug.** Same-VM sends cost nanoseconds;
   same-process isolates microseconds (in-memory frames, no syscalls); other processes
   tens of microseconds (UDS); other machines, if that day comes, milliseconds. The
   *semantics* are uniform across the gradient; the *cost* is controlled by placement.
   We never hide the gradient — pretending remote is local is the CORBA sin. If an
   object is too chatty for its tier, move the object or batch the API.

6. **Failure is contained and explains itself.** A dying isolate yields catchable errors
   at its boundary, never takes the VM down, and callers queued behind it fail fast.
   Errors that cross a boundary carry the other side's story (`ex.remoteStack` — opaque,
   appended in unwind order, displayed fenced, never parsed).

7. **One protocol, pluggable transports.** Every remote-object peer — Rust extension,
   Python extension, Quoin isolate — speaks the same message shapes (manifest, dispatch,
   object tables, fair-queued claims, LIFO-nested re-entrancy, error blobs). Locality
   picks the carrier: UDS frames cross-process, the same frames over an in-memory queue
   with scheduler-native wake for same-process isolates, bulk buffers moved zero-copy
   where physics allows. There is exactly **one** value-crossing encoding (the wire-v2
   lesson: dual encodings diverge); transport optimizations may skip work, never fork
   the format.

8. **Scheduling decisions are recordable.** The scheduler's behavior is fully determined
   by its wake decisions and I/O results — a property we keep deliberately, because it
   makes deterministic replay of concurrent executions possible. Every new wake source
   must flow through the logged choke point (see `ACTOR_OBJECTS.md`, replay hooks); an
   unlogged wake path is a bug even before the replayer exists.

## What this stance rules out

Recorded so future-us doesn't relitigate casually: shared-memory threads in one VM;
`async`-annotated functions or call forms; per-send future proxies; transparent
distribution that hides the cost gradient; a second value-crossing encoding; unlogged
wake paths.

## The road (order of work)

1. This document, and its user-facing reflection in the book (Part V).
2. **Actor-objects**: workers that *host* objects and speak the extension protocol —
   plus cross-isolate channels (CSP across the boundary) and the replay-log hooks
   (`ACTOR_OBJECTS.md`).
3. **Supervision**: uniform restart policy over everything child-process-shaped
   (workers, pools, extensions) — subsumes the deferred extension auto-respawn.
4. **Deterministic replay**: the recorder exists by then (hooks land with arc 2); build
   the replayer and its debugger integration.
5. Distribution stays a gleam: arcs 2–3 walk toward it without promising it.
