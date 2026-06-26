# Extension Architecture — out-of-process, polyglot, unix-socket extensions

Status: **Design capture — all §11 open questions resolved; not started.** This records the
reasoning and the decisions from a design discussion; no code exists yet and none should be written
without a fresh explain-then-pause. Next build per §11: **Tier 0 (`quoin-sdk`)**. Companion to `docs/ASYNC_ARCH.md` (the I/O waist, the scheduler, the
reap queue, and cancellation — all of which this design reuses).

---

## 1. Goal

Quoin is a general-purpose language first. Not everything belongs in core, and we will not
hand-implement every library. We want an **extension system** that lets third parties add
functionality **without source access to the VM** and **without recompiling it**. TLS is
ubiquitous enough to justify living in core; most other libraries are not.

The two motivating extension categories — everything we design must serve both:

- **(a) Bind functionality that already lives in a dylib.** libpq, sqlite, libpng, libcurl,
  … — countless existing native libraries. The extension is mostly *glue*: marshal values,
  call the C functions. The hard dependency is `dlopen` + raw syscalls.
- **(b) High-performance work that can't be done in QN** (NumPy-class). Native-speed numeric
  compute over large arrays — real BLAS/SIMD, big contiguous buffers shared with the VM
  without per-op copies. Note (b) is frequently (a) underneath (NumPy is a C extension over
  BLAS).

**Polyglot authoring is a hard requirement.** Being able to write an extension in the
language of your choice (not only C or Rust) is a large benefit, and very few languages
offer anything like it.

**Empirical grounding.** A 2024–25 survey of the most-downloaded native-extension libraries
in Ruby, Python, and Go (`scratchpad/native-ext-survey.md`) confirms this two-category model
and ranks demand: **crypto/TLS, database drivers, numeric/array compute, and serialization
are universal** across all three languages; image/media, compression, parsing, ML, and
hardware follow. The numeric/array/ML cluster (category b) is concentrated **almost entirely
in Python** and nearly absent as a native concern in Ruby and Go — so **(a) bind-a-dylib is
by far the more common case**, and (b) is real but narrower than its visibility suggests.

**Non-goal (explicit).** Standalone native *tools* — linters, formatters, build tools
(ruff/uv-style) — are **not** extensions and are out of scope. The Quoin toolchain is in Rust
and that is the default; because the parser is already an extracted crate (`quoin-syntax`),
anyone wanting Quoin tooling in another language can depend on it directly. That is orthogonal
to, and does not constrain, the extension system.

---

## 2. The crux is the GC boundary, not the ABI

"Rust has no stable ABI" is the *second* hardest problem. The first is `gc_arena`:

1. **All heap access requires the `&Mutation<'gc>` token**, which is lifetime-branded and
   only exists inside `mutate_root`. It can be handed to a statically-linked native method;
   it can **never** cross a dynamic boundary — the lifetime can't survive the trip.
2. **`Value<'gc>` / `Gc<'gc,T>` carry that brand** and are `#[repr(Rust)]`. Even with a
   perfect ABI they can't be passed out and back meaningfully.
3. **`no_gc_across_yield`** — bare `Gc` on the native stack across a suspend is unsound.

Consequence: **an extension cannot touch the heap directly. It can only hold opaque handles
and call back into the host to do anything GC-related** ("make a string → handle", "get
field x of H → handle", "call method m on H with [H2,H3]"). The host owns `mc`; the
extension is a pure consumer of a host API.

### The handle indirection solves three problems at once

Represent every value the extension holds as an **opaque handle** (a `u64` indexing a
host-side table). Then:

- **ABI** — a handle is an integer; integers have a trivially stable ABI.
- **Rooting** — the handle table **is a GC root set** (gc_arena traces it). Handles stay
  alive exactly as long as the extension holds them. This is JNI local/global refs exactly.
- **no-gc-across-yield** — a rooted handle held across a yield is *fine*; the lint exists
  only because bare stack `Gc` isn't rooted. Handles are.

Every successful embedding API is handle/stack-based for this reason (Lua's stack, CPython
`PyObject*`, JNI `jobject`, Ruby `VALUE`). Design the handle-based host API once; the
**transport becomes a swappable detail**. (Direct codebase analogy: `AnyCollect::trace_gc`
is Ruby's `RTypedData` mark function — the hook a native object uses to keep embedded host
refs alive. The handle table generalizes it to the dynamic case.)

### Handles flow both ways

The survey sharpens the model: for category (a), the dominant pattern is an **extension-side
resource handle** — a connection, prepared statement, cursor, or image context that *lives in
the extension process* and is referenced by the host through an opaque token (the mirror of
the host→extension Value handles above). So the handle table is **bidirectional**: host Values
the extension holds, *and* extension resources the host holds. Both are opaque integers; both
have an owner that reaps on drop (the host's reap queue handles extension resources — see §7).
This is how libpq/sqlite bindings already work conceptually: the C library owns the
connection, the caller holds only a handle to it.

---

## 3. Transport decision: out-of-process native extensions over unix domain sockets

The two motivating categories collapse the option space in a slightly counterintuitive
direction.

Options considered:

| Option | ABI-proof | Polyglot | Native dylibs | Native speed | Isolation | Notes |
|---|---|---|---|---|---|---|
| Static `quoin-sdk` (source/compile-in) | n/a | no | yes | yes | no | First-party only; needs toolchain |
| In-process C-ABI plugin (`libloading`) | via C ABI | **C-ABI langs only** | yes | yes | no | `unsafe`; GC-entangled; forces a cdylib |
| `abi_stable` (Rust↔Rust) | yes | Rust only | yes | yes | no | Still can't carry `Gc`; niche |
| **WASM component** | yes | yes (to-sandbox) | **no** | **no** | yes | Sandboxed; **fails (a) and (b)** |
| **Out-of-process + unix domain socket** | **yes** | **yes (any lang)** | **yes** | **yes** | **yes (crash)** | Per-call latency; socket setup |

### Why out-of-process wins for our categories

Both (a) and (b) are about *reaching the native world*. WASM — the supposed "polyglot" tier —
serves **neither**: it can't `dlopen` a native `.so` (the lib would need a WASM rebuild that
mostly doesn't exist), can't match native BLAS, and can't cheaply share a 100 MB array with
the host (the host can't hand the sandbox a pointer; it copies into linear memory). A sandbox
is for *not* reaching native code — the opposite of what we need.

A separate native **process**, by contrast, can `dlopen` anything, run real syscalls, spin
its own threads, and is **the most polyglot option of all**: any language that runs as a
process, mmaps a region, and can call C qualifies (Python, Go, Ruby, Node, Rust, C, Zig).

The polyglot requirement *itself* rules out in-process C-ABI as the public mechanism: an
in-process shim must be a C-ABI `cdylib`, so it's "polyglot among AOT-native languages" only
(Rust/C/Zig/C++) — excluding exactly the dynamic languages people most want to write glue
in. Out-of-process is the only option that is *truly* any-language.

So: **out-of-process native extensions, unix-domain-socket transport.** Justified — once the
trust decision below removes security from the list — by **polyglot + crash isolation +
parallelism** (a buggy-but-trusted video codec must not take down the VM, and it gets its
own threads/runtime, escaping the VM's single-threaded `!Send` arena). The transport itself is
now a detail, not a headline — which the profiling below earns.

### Why a socket, not shared memory

Shared memory's *only* advantage over a kernel-mediated transport is skipping the per-message
syscall — and you realize that **only by busy-polling** a memory location (io_uring's
SQPOLL-style spin). That is incompatible with the cooperative scheduler: a parked fiber must
yield its core to other tasks, not burn it spinning. Once an extension call parks and takes a
wakeup syscall — which it must, under async — the shared-memory read is no cheaper than reading
the same bytes off a socket.

A proof-of-concept (`../scratch/shm-vs-uds/`, Apple M4) confirms it. With the spin variant
excluded (off the table by the async constraint), **blocking** shared memory is equal-or-worse
than a UDS `socketpair` on exactly the traffic this transport carries:

| metric (blocking shm vs UDS) | UDS | shm + semaphore |
|---|---|---|
| latency p50, 64 B RTT | **2.3 µs** | 4.0 µs |
| one-way throughput, 1 KiB | **2.3 GB/s** | 2.0 GB/s |
| bidir aggregate, 1 KiB | **1.5M msg/s** | 1.3M msg/s |

Shared memory keeps an edge only on **large one-way streaming** (~2× at 1 MiB) — and that edge
is a copy *through* a ring, not the in-place zero-copy that alone would justify a shared mapping
(which the bench never measured). So the SQ/CQ-rings-in-shm transport costs complexity and buys
nothing we can use under async: **unix domain sockets.** The socket fd also drops into the
existing reactor unchanged (§7) — shared-memory rings would need a new wakeup primitive.

### The real cost of native binding is friction, not latency

The survey reframes *why* native extensions are painful, and it validates this transport. The
dominant cost authors cite is **deployment friction**, not call overhead: a C-ABI dependency
loses cross-compilation, demands a C toolchain at install time, and breaks language tooling
(profilers, race detectors). A cgo call is ~40 ns — irrelevant once the C does real work.
Go's documented **cgo-avoidance** follows: where an open wire protocol exists Go reimplements
in pure Go (Postgres, MySQL, Kafka, TLS in stdlib), and binds natively only where **the native
library *is* the product** (SQLite/DuckDB engines, librdkafka, libvips, TF/ONNX) — a ~7:1
preference for the pure-Go SQLite driver where one exists.

Implication: out-of-process **trades link/ABI/toolchain friction for process-lifecycle
friction** (spawn, health-check, restart). For the heavy cases that genuinely need native
code, that is the better trade — the extension ships as its own artifact in its own language,
with no compile-into-the-VM step and no ABI coupling. (It also tells us what belongs in *core*
instead: §9.)

---

## 4. Trust model: a single trust domain

**Extensions are trusted, like the Quoin process itself.** Per-extension sandboxing is a
niche product; "I don't trust this whole program" is already served by whole-process
sandboxing (containers, seccomp, jails) at a different layer.

Consequences:

- The **untrusted-WASM tier is dropped** from the roadmap. (It could return later as a
  separate, deliberately limited product for untrusted drop-in plugins — a different need,
  not a competitor to this.)
- Out-of-process is **not** for security here; it's for polyglot + crash isolation +
  parallelism.
- The cross-process boundary **validation pass is no longer mandatory** — against a trusted
  peer the FlatBuffers/Arrow verifier becomes a debug-only bug-catcher, so "access the received
  frame in place, no decode, no verify" is genuinely free on release builds. **Trust buys the
  real zero-copy** (in the no-deserialize sense; §6).

---

## 5. The decomposition that makes it tractable

**Quoin owns only the Quoin↔extension boundary. Each language's existing FFI owns the
extension↔dylib boundary.**

```
Quoin VM  ⟷  [UDS: framed records / Arrow / handles]  ⟷  extension process (lang X)
                                                                   ⟷ [X's native FFI] ⟷ dylib
```

We never write a libpq binding or ship an FFI layer for native libs. A Python extension
reaches libpq with `ctypes`/`cffi`; Go with cgo; Rust with `bindgen`; Node with N-API. We
provide one thing — a language-agnostic wire protocol — and the entire native-library
binding problem is solved by ecosystems that already exist. The "countless dylibs" category
becomes "not our problem to bind," only "our problem to talk to a process."

### Polyglot is delivered as: protocol + per-language SDKs

Polyglot is never magic — it's a **stable wire protocol + a thin SDK per language**, exactly
like gRPC / Thrift / Cap'n Proto RPC / dbus and, most relevantly, **LSP/DAP** (a JSON-RPC
spec + servers in every language — the proof that "protocol + any-language implementers"
beats a native plugin ABI for ecosystem breadth). Deliverable: the socket framing +
handle semantics, a schema with codegen, and reference SDKs in two or three languages to
seed it. The community writes the rest because the contract is the wire, not a linkage.

**SDK priority (from the survey).** The Rust/PyO3 wave is the most important recent shift in
native extensions: ~¼–⅓ of *new* native code on PyPI is now Rust, and it already owns the
most-downloaded native package (`cryptography`), the dominant validator (`pydantic-core`), and
the dataframe leader (`polars`) — winning on memory-safety across the FFI boundary and
prebuilt artifacts (no compiler at install). So the first reference SDKs should be **Rust and
Python** (where the energy and the hardest perf cases are); Go is a distant third (it mostly
reimplements rather than binds).

---

## 6. Format strategy: separate transport from payload

The transport is **format-agnostic — it moves bytes.** On top of it we offer *format
conventions*, chosen **per message**, not per extension. There are three data shapes with
genuinely different needs:

1. **Control / RPC messages** — "open connection", "run query with params", "here's an
   error", "call handle H". Small, heterogeneous, request/response, latency-sensitive.
   → **Record format** (**FlatBuffers**, decided §11). *Not* Arrow — a one-row columnar table
   is all overhead. This is the bulk of category-(a) glue.
2. **Opaque blobs** — an image, a gzip stream, a file chunk. Just `bytes`: a length-framed
   byte run the extension interprets itself. Arrow adds nothing for a single blob.
3. **Bulk tabular / array data** — many homogeneous rows, numeric arrays, dataframes, result
   sets. → **Apache Arrow**, specifically its **C Data Interface** (decided). The C Data
   Interface is a *tiny stable ABI* (a couple of structs) for zero-copy columnar handoff that
   pyarrow / polars / DuckDB all converge on — so we inherit cross-language interop instead of
   inventing a numeric format.

**Columnar is a data-plane format, not a message format.** Its real constraint is *schema
regularity + homogeneity + bulk processing*; it's a poor fit for irregular/nested/document
data or one-record-at-a-time access. Scoping it to the data plane is what keeps it from
being limiting — a regex or image extension that has no tables never touches Arrow.

This split is per-message, not per-extension: a Postgres driver wants **records for control +
Arrow for result rows** (cf. **ADBC**, Arrow's DB-connectivity standard — query results are a
perfect columnar fit); an image lib wants **records + raw**.

**Decision: adopt Arrow's _C Data Interface_ as the blessed data-plane format _over_ a
format-agnostic transport.** It's the small stable contract — not the whole Arrow library, and
not Arrow Flight as the transport — that buys zero-copy columnar interop for free. Offer it as
a payload codec where array/tabular data flows.

### "Zero-copy" means "no serialize/deserialize step", not "into a Value"

The Cap'n Proto / FlatBuffers / Arrow / rkyv sense: the in-memory layout *is* the wire
layout, so "access a field" replaces "parse a message". Precisions that bound the win:

- It eliminates the **wire codec + intermediate buffer**, but **not the projection** out of
  the arena. A Quoin object/map/list is a GC graph, not already in the flat layout, so the
  host still walks it to write the record. Truly free only for **bytes and scalars**;
  structured graphs still pay a projection walk.
- True zero-copy for **bulk** (no copy at all) would need host and extension to *share* the
  buffer's backing store — which a socket does not do; bytes are copied through the kernel.
  Start with **copy-through-the-socket** (one `write`/`read`, usually noise next to the native
  work) and treat in-place sharing as **deferred**: if a *measured* high-bandwidth path (video,
  large-array streaming) ever demands it, the extension can hand the host a buffer fd over the
  socket (`SCM_RIGHTS` + `mmap`) without touching the control protocol. Not in the baseline —
  profiling already showed shared memory buys nothing for messaging (§3), and the in-place bulk
  case is itself unmeasured.
- rkyv is **out** — Rust-only, defeats polyglot.

---

## 7. Reuse of existing VM machinery

This design is a **transport for the plain-data waist** (`IoRequest`/`IoResult`), not a new
API. It drops into the existing machinery almost 1:1:

- **Async call = a socket read already in the waist.** The extension's UDS fd registers with
  the `async-io` reactor exactly as `NativeSocket` already does, so an **extension call becomes
  just another `IoRequest`**: write the request frame, park the fiber on the readable socket,
  resume when the reply frame arrives. The calling fiber parks, other tasks run, the child
  writes its reply, the fiber resumes. Async extension calls for free; a slow extension never
  stalls the VM; `no_gc_across_yield` already holds (handles/plain data across the await, not
  `Gc`). This is a *tighter* reuse than a bespoke ring — no new wakeup primitive, no eventfd to
  register, just the socket the reactor already waits on.
- **Resource lifecycle reuses the reap queue.** An extension resource handle (a libpq conn,
  a prepared statement, a cursor) drops → host reaps, exactly like `NativeSocket`'s `StreamId`.
  The socket reap queue is the prototype for all extension resources.
- **Crash/timeout reuses cancellation.** Child crash mid-call must not deadlock the parked
  fiber → timeout + park-cancellation, i.e. the 2b-ii cancellation machinery + the deferred
  timeout combinator. Dead child → release the handle-table roots it held (else they never
  free). Host dies → child notices its peer fd closed and exits.
- **Handle table = GC root set** (gc_arena traces it; §2).

### Quoin-side consequence: a bulk array / dataframe Value type

Arrow pairs with a **bulk array/dataframe Value type** held as a handle over one contiguous
buffer, **distinct from ordinary objects** — you must *not* explode a million-element column
into a million `Value`s. Ordinary control data maps to normal Quoin objects/maps (small,
projected, copy is fine). The columnar data plane and the "array Value type for NumPy-in-QN"
are the **same design decision seen twice**. (Baseline: that buffer is filled by copying the
column off the socket once; the deferred §6 in-place path would back it with a shared buffer
fd instead.)

---

## 8. Selection criteria — what this tier is (and isn't) for

Good fit: **trusted native code, OS access, parallelism, byte/array-heavy, coarse-grained,
low re-entrancy.** Both motivating categories qualify — a DB query returns rows; a vectorized
array op runs over millions of elements; neither calls *back* into Quoin per element, so the
µs cross-process round-trip amortizes and the "chatty callback thrash" failure mode doesn't
bite. The rule that makes (b) work: **keep arrays resident extension-side as handles and ops
whole-array** so successive operations never re-copy them across the boundary.

Poor fit (use a different tier): **fine-grained value juggling and frequent callbacks into
Quoin** (a host call per row/field). Cross-process host calls are µs, not ns; chatty APIs
thrash. If an extension needs frequent re-entry, it belongs in-process (the static SDK tier).

---

## 9. Tiering

- **Tier 0 — `quoin-sdk` (static / source plugins).** Publish the safe registration API
  (`NativeClassBuilder`, native-state + `trace_gc`, the I/O-request waist) as a semver crate,
  ship the dylint with it. Extensions compiled *into* a custom VM via Cargo. No ABI problem,
  full speed/safety. Real value: **forces stabilization of the host API surface** that the
  out-of-process protocol then mirrors, while we control both sides. **Decided (§11): this is the
  first build** — and the home for first-party / perf-critical / chatty extensions.
- **Primary public tier — out-of-process native over unix domain sockets** (this document).
  Polyglot, trusted, crash-isolated, native-speed. The flagship third-party story.
- **Deferred — WASM** for untrusted drop-in plugins, only if that need ever materializes.
  Separate product; not a competitor to the above.

### Core vs. extension boundary

Core stays small — **ubiquitous + perf-critical + security-sensitive** primitives. The
survey's top, stable, security-sensitive categories argue for a concrete list:

- **In core:** TLS/crypto, regex, JSON, compression, hashing — plus the scheduler. Used
  everywhere, change slowly, and (for crypto) dangerous to leave to a long tail of extensions.
  Go's pure-TLS-in-core shows this beats recreating OpenSSL build-pain.
- **As extensions:** databases, ML/numeric, imaging, niche-format parsing, hardware —
  anything whose value is a specific native library or a narrow audience.

**Counter-pressure (note it):** core inclusion inherits the VM's release cadence and a
*forever* platform-support burden — exactly why CPython keeps even Rust strictly optional
(Gaynor / the Language Summit). The bar for core is high; the default is extension.

Deno is a reasonable north star for the overall shape (small core + a guarded native path).

---

## 10. Prior art / design templates

- **io_uring** — SQ/CQ rings in shared memory + eventfd wakeups. Considered as the transport
  and **dropped**: its edge needs busy-polling, which the async scheduler can't spend (§3). A
  reference for the submit/complete *shape*, not the mechanism.
- **FlatBuffers** — schema-driven, in-place access, verifier, schema evolution. The **decided**
  control-plane record format (§11), chosen over Cap'n Proto for polyglot binding breadth.
- **Apache Arrow** — the data-plane format; zero-copy columnar interchange across languages
  and processes. The **C Data Interface** (a tiny stable ABI) is the decided contract; **ADBC**
  (DB connectivity), **Plasma** (shared-memory object store), and **Flight** (RPC) are
  reference points, not dependencies adopted wholesale.
- **LSP / DAP** — protocol + any-language servers; the proof that polyglot-via-protocol beats
  a native plugin ABI.
- **JNI / Lua stack / CPython `Py_LIMITED_API` (abi3) / Ruby `RTypedData` mark** — handle &
  rooting models; CPython's lesson: keep the stable surface *small*.
- **Erlang ports (out-of-process) vs NIFs/dirty schedulers** — the isolation-vs-speed split.
- **Terraform / HashiCorp go-plugin** — subprocess + gRPC over a socket; the closest mainstream
  analog to our shape (subprocess + framed RPC over a UDS), modulo our handle/Arrow payload layer.
- **Deno FFI / ops** — core + WASM + a guarded `dlopen` escape hatch.

---

## 11. Decided vs open

**Decided**

- Extension system is **out-of-process native processes over unix domain sockets** (polyglot,
  trusted, crash-isolated). Accepted trade: **link/ABI/toolchain friction → process-lifecycle
  friction**, which the survey's "friction, not latency" finding supports for heavy cases.
- **Transport is a unix domain socket, not shared memory** (`../scratch/shm-vs-uds/`). With
  busy-polling off the table under async, blocking shared memory is equal-or-worse than a UDS
  socketpair on control-plane traffic; its only edge (large one-way streaming) is a
  copy-through-a-ring, not in-place sharing. The socket fd reuses the reactor unchanged (§7).
  In-place bulk sharing (`SCM_RIGHTS` fd-passing) is deferred until a measured workload needs it.
- **Single trust domain** — extensions are trusted; no per-extension sandbox; untrusted-WASM
  tier dropped.
- **Quoin owns the Quoin↔extension protocol; each language's FFI owns extension↔dylib.**
- Polyglot delivered as **wire protocol + per-language SDKs**; **first SDKs: Rust + Python**
  (where the energy and hardest perf cases are; Go a distant third).
- **Transport is format-agnostic** (framed socket messages + handles); **payload format is
  per-message**: records (control) + raw bytes (blobs) + **Arrow C Data Interface (columnar)**.
- **Control-plane record format: FlatBuffers.** In-place access + schema evolution + a verifier
  (debug-only against a trusted peer, §4), chosen for the **broadest, most-uniform polyglot
  bindings** — the "protocol + per-language SDKs" strategy (§5) needs a low bar for community SDKs.
  Cap'n Proto's built-in RPC/promise-pipelining is its main edge and we don't use it (we own the
  transport, §7); its non-C++/Rust bindings are also less mature.
- **Bulk data transfer: copy-through-the-socket baseline; `SCM_RIGHTS` fd-passing deferred.** One
  `write`/`read` per bulk payload (noise next to the native work). True in-place sharing (a shared
  buffer fd + `mmap`) is added only if a *measured* high-bandwidth workload (video, large-array
  streaming) demands it — and it slots in over the socket **without changing the control protocol**
  (§6). Start simple, measure.
- **Handles are bidirectional** — host Values held by the extension, and extension-side
  resources (connection/statement/context) held by the host; both opaque, both reaped on drop.
- **Handle rooting/lifetime: JNI-style local + global.** The handle table is a traced GC root set
  (§2). Handles default to **call-scoped (local)** — auto-released when the originating call
  completes, so the transients in a call generate no release traffic; an extension **promotes** a
  handle to **retained (global)** to hold it across calls, then releases it explicitly (batched onto
  a later call). Alloc is implicit (any host op returning a value mints a handle). Peer crash →
  bulk-release the dead peer's handles (host Values drop their roots; ext resources reap via the
  reap queue, §7). This is what enables the callback protocol (#3 — an ext retains a host block).
- **Callback / re-entrancy protocol: batched.** An extension invokes a retained (global) host-block
  handle via a request on the socket *during* an in-flight call — re-entrant request/response
  interleaving the host services while parked on the reply (LSP-style). The hot-path primitive is
  **invoke H over a batch `[a₁..aₙ] → [r₁..rₙ]`** in one round-trip (host runs the block N times
  locally); a single call is n=1. This amortizes the boundary crossing so per-element callback
  thrash (§8) can't bite; genuinely fine-grained / high-frequency callbacks stay a Tier-0 concern.
- **Bulk array / dataframe Value type: a native-state (`AnyCollect`) handle type.** A native object
  (like `List`/`Map`/`Socket`) holding one contiguous Arrow-layout buffer behind a handle —
  GC-integrated via `trace_gc`, reaped via the reap queue, **no `Value`-enum change**. Copy-backed
  at baseline (shared-buffer-backed only if #2's deferred fd-passing lands); whole-array ops keep it
  resident extension-side (§8). The columnar data plane and the "array type for NumPy-in-QN" are the
  same type (§7) — distinct from ordinary objects, never exploded into per-element `Value`s.
- Reuse the **plain-data waist, the reactor (extension-call-as-parked-fiber on the socket fd),
  the reap queue, and 2b-ii cancellation/timeout**. Handle table = GC root set.
- "Zero-copy" = **no serialize/deserialize step**, not zero-copy into a `Value`.
- **Core vs extension:** TLS/crypto, regex, JSON, compression, hashing in core; DBs,
  ML/numeric, imaging, niche parsing, hardware as extensions (high bar for core; §9).
- **Build order: Tier 0 (`quoin-sdk`) first.** Stabilize the static host API —
  `NativeClassBuilder`, native-state + `trace_gc`, the I/O-request waist, the `no_gc_across_yield`
  dylint — as a semver crate *before* the out-of-process protocol, since the wire protocol
  re-expresses that operation surface (§2/§9). It is mostly **extract-and-harden** of the existing
  internal surface, a permanent tier for chatty / first-party / perf-critical extensions (§8), and
  lets the GC-object model settle in-process before any transport is added. **Guardrail:** design
  the surface as *operations an extension requests of the host* (handle-*projectable*) so the
  protocol mirrors it rather than diverging. Scope note: this settles the **foundation order
  only** — the handle/rooting protocol (decided below) and the transport are inherently
  protocol-tier and aren't exercised by the Tier-0 build itself.
- **Non-goal:** standalone native tools (linters/formatters/build tools) are out of scope;
  the Rust toolchain + the extracted `quoin-syntax` crate cover that need separately.
- **rkyv rejected** (Rust-only).

**Open**

The design questions captured in this document are now **all resolved** (this pass). What remains is
**implementation-level** and will surface during the Tier-0 (`quoin-sdk`) build — e.g. the exact host
operation / op-code list, the FlatBuffers control schema, and the wire shape of batched handle
release and batched callbacks. Capture new questions here as they arise.
