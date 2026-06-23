# Extension Architecture — out-of-process, polyglot, shared-memory extensions

Status: **Design capture, not started.** This records the reasoning and the decisions
from a design discussion; no code exists yet and none should be written without a fresh
explain-then-pause. Companion to `docs/ASYNC_ARCH.md` (the I/O waist, the scheduler, the
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

## 3. Transport decision: out-of-process native extensions over shared memory

The two motivating categories collapse the option space in a slightly counterintuitive
direction.

Options considered:

| Option | ABI-proof | Polyglot | Native dylibs | Native speed | Isolation | Notes |
|---|---|---|---|---|---|---|
| Static `quoin-sdk` (source/compile-in) | n/a | no | yes | yes | no | First-party only; needs toolchain |
| In-process C-ABI plugin (`libloading`) | via C ABI | **C-ABI langs only** | yes | yes | no | `unsafe`; GC-entangled; forces a cdylib |
| `abi_stable` (Rust↔Rust) | yes | Rust only | yes | yes | no | Still can't carry `Gc`; niche |
| **WASM component** | yes | yes (to-sandbox) | **no** | **no** | yes | Sandboxed; **fails (a) and (b)** |
| **Out-of-process + shared memory** | **yes** | **yes (any lang)** | **yes** | **yes** | **yes (crash)** | Per-call latency; shm setup |

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

So: **out-of-process native extensions, shared-memory transport.** Justified — once the
trust decision below removes security from the list — by **polyglot + crash isolation +
parallelism** (a buggy-but-trusted video codec must not take down the VM, and it gets its
own threads/runtime, escaping the VM's single-threaded `!Send` arena).

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
- The shared-memory boundary **validation pass is no longer mandatory** — against a trusted
  peer it becomes a debug-only bug-catcher, so "access in place, no decode, no verify" is
  genuinely free on release builds. **Trust buys the real zero-copy.**

---

## 5. The decomposition that makes it tractable

**Quoin owns only the Quoin↔extension boundary. Each language's existing FFI owns the
extension↔dylib boundary.**

```
Quoin VM  ⟷  [shm protocol: rings + records/Arrow + handles]  ⟷  extension process (lang X)
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
beats a native plugin ABI for ecosystem breadth). Deliverable: the ring/shm protocol +
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
   → **Record format** (FlatBuffers / Cap'n Proto). *Not* Arrow — a one-row columnar table
   is all overhead. This is the bulk of category-(a) glue.
2. **Opaque blobs** — an image, a gzip stream, a file chunk. Just `bytes`: a raw
   `(offset, len)` into the region the extension interprets itself. Arrow adds nothing for a
   single blob.
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
- True zero-copy for **bulk** (no copy at all, not even into the ring) requires backing the
  buffer storage with the shm region — e.g. a Quoin array type whose store is a shm-resident
  Arrow buffer. Start with **copy-once-into-ring** (one memcpy, usually noise) and reserve
  shm-backed storage for a *measured* high-bandwidth path (video, large-array streaming).
- rkyv is **out** — Rust-only, defeats polyglot.

---

## 7. Reuse of existing VM machinery

This design is a **transport for the plain-data waist** (`IoRequest`/`IoResult`), not a new
API. It drops into the existing machinery almost 1:1:

- **Sync = the io_uring pattern.** SQ/CQ ring buffers in shm + an **eventfd** (or futex) for
  wakeups — io_uring's submission/completion design with VM↔extension instead of app↔kernel.
  Register the completion eventfd with the `async-io` reactor and an **extension call becomes
  just another `IoRequest` variant**: "write to the SQ, park the fiber on the CQ eventfd".
  The calling fiber parks, other tasks run, the child signals completion, the fiber resumes.
  Async extension calls for free; a slow extension never stalls the VM; `no_gc_across_yield`
  already holds (handles/plain data across the await, not `Gc`).
- **Offsets, not pointers**, inside the region (different base addresses across processes).
- **Resource lifecycle reuses the reap queue.** An extension resource handle (a libpq conn,
  a shm slab) drops → host reaps, exactly like `NativeSocket`'s `StreamId`. The socket reap
  queue is the prototype for all extension resources.
- **Crash/timeout reuses cancellation.** Child crash mid-call must not deadlock the parked
  fiber → timeout + park-cancellation, i.e. the 2b-ii cancellation machinery + the deferred
  timeout combinator. Dead child → release the handle-table roots it held (else they never
  free). Host dies → child notices its peer fd closed and exits.
- **Handle table = GC root set** (gc_arena traces it; §2).

### Quoin-side consequence: a bulk array / dataframe Value type

Arrow pairs with a **bulk array/dataframe Value type** whose backing store is the shm-
resident Arrow buffer (a handle), **distinct from ordinary objects** — you must *not* explode
a million-element column into a million `Value`s. Ordinary control data maps to normal Quoin
objects/maps (small, projected, copy is fine). The columnar data plane and the "array Value
type for NumPy-in-QN" are the **same design decision seen twice**.

---

## 8. Selection criteria — what this tier is (and isn't) for

Good fit: **trusted native code, OS access, parallelism, byte/array-heavy, coarse-grained,
low re-entrancy.** Both motivating categories qualify — a DB query returns rows; a vectorized
array op runs over millions of elements; neither calls *back* into Quoin per element, so the
µs cross-process round-trip amortizes and the "chatty callback thrash" failure mode doesn't
bite. The rule that makes (b) work: **keep arrays shm-resident as handles and ops whole-array**
so successive operations never re-copy.

Poor fit (use a different tier): **fine-grained value juggling and frequent callbacks into
Quoin** (a host call per row/field). Cross-process host calls are µs, not ns; chatty APIs
thrash. If an extension needs frequent re-entry, it belongs in-process (the static SDK tier).

---

## 9. Tiering

- **Tier 0 — `quoin-sdk` (static / source plugins).** Publish the safe registration API
  (`NativeClassBuilder`, native-state + `trace_gc`, the I/O-request waist) as a semver crate,
  ship the dylint with it. Extensions compiled *into* a custom VM via Cargo. No ABI problem,
  full speed/safety. Real value: **forces stabilization of the host API surface** that the
  out-of-process protocol then mirrors, while we control both sides. This is the natural
  *first* build and the home for first-party / perf-critical / chatty extensions.
- **Primary public tier — out-of-process native over shared memory** (this document).
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

- **io_uring** — SQ/CQ rings in shared memory + eventfd wakeups; offsets not pointers. The
  transport template.
- **FlatBuffers / Cap'n Proto** — schema-driven, in-place access, verifier, schema evolution.
  The control-plane record format (choice pending).
- **Apache Arrow** — the data-plane format; zero-copy columnar interchange across languages
  and processes. The **C Data Interface** (a tiny stable ABI) is the decided contract; **ADBC**
  (DB connectivity), **Plasma** (shared-memory object store), and **Flight** (RPC) are
  reference points, not dependencies adopted wholesale.
- **LSP / DAP** — protocol + any-language servers; the proof that polyglot-via-protocol beats
  a native plugin ABI.
- **JNI / Lua stack / CPython `Py_LIMITED_API` (abi3) / Ruby `RTypedData` mark** — handle &
  rooting models; CPython's lesson: keep the stable surface *small*.
- **Erlang ports (out-of-process) vs NIFs/dirty schedulers** — the isolation-vs-speed split.
- **Terraform go-plugin** — subprocess + gRPC (the pipe-based, no-zero-copy contrast).
- **Deno FFI / ops** — core + WASM + a guarded `dlopen` escape hatch.

---

## 11. Decided vs open

**Decided**

- Extension system is **out-of-process native processes over shared memory** (polyglot,
  trusted, crash-isolated). Accepted trade: **link/ABI/toolchain friction → process-lifecycle
  friction**, which the survey's "friction, not latency" finding supports for heavy cases.
- **Single trust domain** — extensions are trusted; no per-extension sandbox; untrusted-WASM
  tier dropped.
- **Quoin owns the Quoin↔extension protocol; each language's FFI owns extension↔dylib.**
- Polyglot delivered as **wire protocol + per-language SDKs**; **first SDKs: Rust + Python**
  (where the energy and hardest perf cases are; Go a distant third).
- **Transport is format-agnostic** (shm rings + handles); **payload format is per-message**:
  records (control) + raw bytes (blobs) + **Arrow C Data Interface (columnar)**.
- **Handles are bidirectional** — host Values held by the extension, and extension-side
  resources (connection/statement/context) held by the host; both opaque, both reaped on drop.
- Reuse the **plain-data waist, the reactor (extension-call-as-parked-fiber on an eventfd),
  the reap queue, and 2b-ii cancellation/timeout**. Handle table = GC root set.
- "Zero-copy" = **no serialize/deserialize step**, not zero-copy into a `Value`.
- **Core vs extension:** TLS/crypto, regex, JSON, compression, hashing in core; DBs,
  ML/numeric, imaging, niche parsing, hardware as extensions (high bar for core; §9).
- **Non-goal:** standalone native tools (linters/formatters/build tools) are out of scope;
  the Rust toolchain + the extracted `quoin-syntax` crate cover that need separately.
- **rkyv rejected** (Rust-only).

**Open**

- Control-plane record format: **FlatBuffers vs Cap'n Proto**.
- Bulk data: **copy-into-ring first vs commit to shm-backed storage** (start simple, measure).
- **Re-entrancy / callback protocol** (how an extension invokes a Quoin block — batched?).
- The Quoin-side **bulk array / dataframe Value type** (shm-backed, handle-based).
- **Handle table / rooting protocol** specifics (alloc, release, lifetime on crash).
- Whether **Tier 0 (`quoin-sdk`)** is built first to harden the host API before the protocol.
