# Actor-objects — hosted objects, one peer protocol, cross-isolate channels

*Status: DESIGN, 2026-07-13 — arc 2 of the concurrency road (`CONCURRENCY_MODEL.md`).
Grounded in an archaeology pass over `worker.rs`/`worker_spawn.rs`/`worker_service.rs`
vs `extension.rs`, and over the scheduler/channel machinery; file:line cites are from
that pass (main @ v0.1.1 + PR #11). Companion docs: `CONCURRENCY_ARCH.md` (mechanism
survey; its §13 already flags this convergence), `quoin-ext-proto/PROTOCOL.md`.*

## 0. The thesis

An **actor is an object hosted in another isolate**, addressed by ordinary message
sends. Quoin already ships this twice, incompatibly:

- **Extension-backed classes**: a real protocol — manifest handshake, `Call` dispatch to
  receivers, SDK-owned object tables with drop-driven reaping, fair-queued connection
  claims, LIFO-nested re-entrancy, cross-process error blobs (`extension.rs`,
  `PROTOCOL.md`). Peers: Rust and Python processes.
- **`WorkerService`**: a proxy actor by hand — one hosted instance, a synthesized Quoin
  serve loop (`SERVICE_LOOP_QN`, `worker_spawn.rs:74-88`), MNU-seam RPC forwarding
  (`worker_service.rs:75-94`), a one-token bounded(1) channel as the mailbox serializer,
  a bespoke `{t,v}` `DataValue` frame envelope with **no manifest, no version handshake,
  no re-entrancy, no object table** (`worker_spawn.rs:259-274`).

`CONCURRENCY_ARCH.md` §13 already says it: *"extensions : processes :: services :
workers … the extension wire … worth unifying rather than paralleling."* This document
is that unification: **one peer protocol, three peer kinds** (Rust process, Python
process, Quoin isolate), with the Quoin peer gaining three upgrades foreign peers cannot
have — portable blocks, structured stacks, and cross-isolate channels.

## 1. One protocol, pluggable transports

The peer protocol is the extension protocol's message set (`Msg`: manifest, `Call`,
terminals, host-ops, `remote_stack`). Transports by locality:

| peer | carrier | value form | notes |
|---|---|---|---|
| Rust/Python process (extension) | UDS, u32-LE + msgpack | bytes | unchanged, today's path |
| Quoin process worker | UDS, u32-LE + msgpack | bytes | replaces the bespoke `{t,v}` envelope |
| Quoin thread worker | in-memory lanes | **owned `Msg` values** | no encode/decode, no syscalls |

The thread-worker row is the "same protocol, cheaper carrier" case: the lanes
(`async_channel`, `worker.rs:73-107`) carry the protocol's *typed messages* directly —
today they already move `WireData` trees rather than bytes, so this is a re-typing, not
a redesign. This satisfies the stance's one-encoding rule: `Msg`/`DataValue` **is** the
protocol form; byte-encoding is a per-transport detail, and there is still exactly one
value-crossing data model. Bulk `ArrowArray` buffers move without copy same-process.

Manifest handshake becomes universal: a hosting worker answers `GetManifest` with the
classes it hosts (a plain `Worker.spawn:`/`start:` peer answers empty, exactly as a
generic extension does — back-compat by construction). This also gives worker peers the
version gate they currently lack.

## 2. The user surface

Evolve `WorkerService` rather than invent beside it:

```quoin
var counter = Worker.host:{ Counter.new:{ var start = 0 } };        "* thread isolate
var conn    = Worker.host:{ Db.connect:url } backing:'process';
counter.increment;               "* an ordinary send; parks; crosses the boundary
counter.value                    "* -> 1
```

- `Worker.host:` evaluates the block **in the worker** (it is a portable block — the
  submit-time scan applies) and hosts the resulting object; the parent receives a proxy
  whose unknown selectors forward as `Call`s — the MNU seam `WorkerService` already uses.
- Sends park; concurrency is blocks and channels (stance guarantee 3). The mailbox is
  the fair-queued claim machinery from PR #11, replacing the one-token channel — waiters
  park FIFO with epoch identity, nested re-entry composes, depth-capped. The claim is
  keyed **per hosted object**, not per connection (§5) — sends to different objects in
  one isolate may overlap.
- **Multiple objects per isolate**: hosting returns instances backed by an object table
  in the worker (the SDK `ObjectTable` pattern), so a hosted object's methods can return
  further live instances (`makes`/resources-in-data semantics carry over verbatim), all
  sharing the isolate. Lifetime: proxy drop → batched release (`ExtResource` reap
  pattern) frees the hosted object; **isolate lifetime is separate** and belongs to
  supervision (arc 3) — `Worker.host:` answers `(proxy, handle)` or the proxy exposes
  `worker` for stop/join.

## 3. What the Quoin peer adds over foreign peers

**a. Portable blocks as arguments.** A block argument to a foreign peer is a host-value
handle driven by `invoke_block` round-trips. For a Quoin peer, when the block passes the
portability scan, ship it (`PortableBlock` — template by `Arc` for threads with
`localize_template`, by source/bytecode for processes once source-shipping exists;
until then process peers fall back to the handle path). An unportable block also falls
back to the handle path — same semantics, more round trips, never an error. Decision
rule: *portable + Quoin peer → ship; otherwise → handle.*

**As built (slice 6 v1, 2026-07-14) — ship path only; the fallback is an error, not
handles yet.** A block argument to a THREAD-backed service snapshots at the encode
seam (`service_call`, before the token — a refused argument never occupies the
service) and rides the dispatch request as an out-of-band sidecar:
`DispatchReq.blocks: Vec<(position, PortableBlock)>`, with a Null placeholder holding
the frame's `method_args` position. Deliberately NOT an `Arg` variant: `Arg` is the
wire protocol, and a `PortableBlock` (an `Arc` template) is only meaningful
in-process — when source-shipping lands, the wire form will be source/bytecode
inline, a different shape. The sidecar is the same richer-than-wire-taxonomy
allowance the plain lane already uses for `WorkerMsg::Block`. Worker-side,
`dispatch_hosted` rebuilds each sidecar entry via `rebuild_portable_value` (global
references verified against the worker — a missing user global is a clear catchable
error) into a live closure the hosted method invokes locally, N runs per one
crossing, storable across dispatches (rooted via the hosted table while reachable).
**Completed by worker host-ops (slice 4.5, 2026-07-14) — the decision rule is now the
doc's "never an error":** *portable + thread peer → ship; otherwise → handle.* The
handle fallback needs no `InvokeBlock` frame: a conversation is symmetric `Call` /
`CallReturn*` — a worker→parent `Call` whose `recv` is a parent-held handle IS the
host-op (one protocol, both directions; extensions keep their bespoke host-op frames).
As built:

- **Transport**: `DispatchReq` carries the conversation's two lanes (`reply`
  worker→parent, `hostops` parent→worker). A conversation is strictly LIFO; `Call`
  opens a level, `CallReturn*` closes one. Thread backing holds the lanes directly;
  the process pumps run a depth-counting relay of the same frames over the socket
  (both sides in `worker_spawn.rs`).
- **Parent side**: `service_call` pumps a conversation loop — worker host-ops are
  serviced ON THE CALLER'S FIBER (`service_parent_hostop` runs the handle's block via
  ordinary dispatch), which is what makes claims and cycle detection composable
  (§5.1). A send the serviced code makes back into the same worker is a NESTED call
  riding the open conversation — the worker-wide `active` record is §5.1 rule 3 at
  N=1, absorbed by the claim machinery in slice 5. Depth-capped at 16 both sides.
- **Worker side**: `Arg::Handle` wraps as a `HostBlock` instance
  (`value`/`value:`/`valueWithArgs:` forward as host-ops); `invoke_parent_block`
  pumps the mirror loop, servicing nested parent→worker calls while it waits.
- **Handle table**: the parent's own `vm.hosted` table roots handed-out blocks (the
  same table a worker uses for hosted objects — one id space, symmetric roles);
  handles minted for a service release at `serviceStop` (a worker-stored `HostBlock`
  may be invoked by any later call, so per-call release would be wrong).
- **Semantics, honest version**: a PORTABLE block freezes its captures at send time
  regardless of path — shipping snapshots by construction, and the handle path for a
  portable block (process backing, nested frames) wraps a local snapshot-rebuild so
  backing never changes meaning. An UNPORTABLE block runs in the parent against LIVE
  state — that is what write-captures are for, and why it could not ship.
- **Cancellation ABANDONS a conversation cleanly**: a `Cancelled` raised in serviced
  code re-raises unchanged (never becomes a wire error — the extension precedent);
  the dropped lanes tell the worker/pump to answer pending host-ops with errors and
  unwind to the terminal. The service SURVIVES — unlike a cancelled extension call,
  which desyncs the framed socket and kills the peer.

*Still open after 4.5:*
1. **Process shipping** is blocked on source/bytecode shipping (its wire form
   replaces the sidecar for that backing).
2. **Blocks nested inside data-structure arguments** still refuse (the wire walkers
   own that taxonomy — same rule as plain lane messages).
3. **Block RETURNS** from hosted methods currently fall into the non-portable-object
   path and come back as sub-proxies (semantics untested — `value:` on such a proxy
   dispatches remotely); the symmetric ship-back needs a reply-side sidecar.
4. **`Worker.host:{...}` block form** still waits — remote evaluation, not just
   transport.
5. **Sends to arbitrary parent OBJECTS** (the `CallMethodOnHandle` analogue) — the
   frame shape already supports it (`Call` on a handle); minting object handles and
   deciding their lifetime does not exist yet.

**b. Structured stacks.** Quoin-to-Quoin errors need not be opaque: the `remote_stack`
blob carries the worker's real rendered trace initially (day one, free via PR #11's
field), with a structured-trace upgrade (frames as data, uniformly steppable) as a
later, purely additive field.

**c. Cross-isolate channels.** See §6.

**d. Re-entrancy both ways.** The worker peer services nested `Call`s while awaiting
host-ops (as SDK peers now do), and — unlike foreign peers — hosted Quoin code calling
`parent`-owned objects is symmetric in the far future; v1 keeps the extension asymmetry
(host drives, peer host-ops back).

## 4. The gap list (from the archaeology, honestly)

What convergence must build, in rough order of weight:

1. **Re-entrancy/host-ops for worker peers** — the largest structural divergence: worker
   lanes today are one-shot message passing with no host-op analogue (`ControlKind` has
   one variant, `PsTree`). The child-side serve loop must become the SDK-style
   conversation loop (service nested requests while awaiting replies). For thread peers
   this is lane discipline, not sockets.
   **DONE (slice 4.5, as built — see §3a):** conversations are symmetric
   `Call`/`CallReturn*` both directions (no new frames): worker→parent `Call` on a
   parent-held handle = host-op, serviced on the caller's fiber; parent→worker `Call`
   mid-conversation = nested call, serviced by the parked worker fiber. Strict LIFO,
   depth-capped 16, relayed over the process socket by depth-counting pumps.
   Deliberately minimal: block handles only — no object handles, no
   `MakeString`-style host reach.
2. **Manifest + object table in the hosted-worker serve loop** — replaces
   `SERVICE_LOOP_QN`'s single-instance `perform:` loop.
3. **Retiring the `{t,v}` envelope** — process-worker pump speaks `Msg` frames;
   `worker-serve` decodes with `decode_frame`. The control lane's request-id machinery
   (`worker_spawn.rs:351-405`) collapses into the protocol's conversation shape.
   **DONE (slice 3, as built):** TWO sockets per process worker — lanes, never
   frame-multiplexing (§5's rule). The *conversation* socket carries the
   `GetManifest`/`ManifestReturn` handshake (parent enforces the version gate, killing
   a mismatched child — the gate workers previously lacked) and one-at-a-time
   conversations (`Call{op:"psTree"}` → `CallReturnData`); both sides' id-correlation
   machinery is deleted, and hosted-object dispatch lands here next. The *mailbox*
   socket is one long-lived implicit conversation: `Worker.send:` either direction is
   an intermediate `Call{op:"send", data}` frame (fire-and-forget by design — real
   backpressure is the §6 channel-relay work), and the done report is its TERMINAL —
   `CallReturnData{value}` or `CallReturnError{message, remote_stack}` (blob empty
   until structured stacks). The child answers the handshake synchronously before
   anything fallible runs, so a fast-failing unit still gets its done terminal read.
   Thread workers untouched (item 5's rule).
4. **Claim machinery generalized** — `NativeExtension`'s owner/depth/waiters become a
   shared "peer connection" state used by extension and worker proxies alike, and the
   claim key moves from the connection to the hosted object (§5).
5. **Plain workers unchanged** — `Worker.spawn:`/`start:`/`send:`/`receive`/`join` keep
   their surface; only the wire beneath process backing converges. `terminate` stays
   process-only; thread isolates still cannot be killed (documented).

## 5. Per-object mailboxes, connection lanes (multi-in-flight sends)

*Added 2026-07-13 after review: the database-connection example demands it. Queries are
slow; funneling every send to a peer through one serialized conversation makes a hosted
`Db` pool useless.*

PR #11's fair-queued claim serializes at the wrong granularity: it is **per
connection**, which conflates every object in a peer into one actor. The actor
guarantee is only *one message at a time per object* — cross-object parallelism inside
one peer is semantically fine and, for slow-op peers, essential.

- **The claim moves to the hosted object** (keyed by resource id; class-side calls have
  no object and claim nothing but a lane). Same machinery — FIFO waiters with epoch
  identity, depth-capped nesting — different key. Sends to one object serialize (the
  mailbox); sends to different objects may overlap.
- **Overlap rides N connections ("lanes") to the same peer**, each speaking today's
  protocol *unchanged* — LIFO nesting, host-ops, fairness are all per-lane properties.
  Explicitly **not** frame multiplexing with correlation ids: that rewrites protocol
  v2's conversation shape, and buys nothing — the peer still needs threads to overlap
  blocking handlers, so the concurrency has to exist peer-side either way.
- **The manifest declares the lane count** (`lanes`, append-only field; absent = 1 =
  today's behavior, back-compat by construction). A GIL-bound compute peer stays at 1;
  a database extension declares 8 and its SDK serves each lane on its own thread. The
  shared object table needs only structural locking — instance exclusivity is already
  guaranteed host-side by the per-object claim, plus the re-entrancy work's
  take-instance-out-of-the-table discipline.
- **Quoin peers get the better version free**: N lanes = N concurrent conversations =
  N fibers in the worker VM, interleaved cooperatively — a hosted object parked on I/O
  doesn't block its isolate-mates, with no threads at all.
- The database story then reads correctly: host a `DbPool`; its `connection`s are
  distinct hosted objects; queries on different connections run in parallel, queries on
  one connection serialize.

**Care point (scary, settle before that slice lands — see §10):** lock ordering
between object claims and lane claims under nested re-entrancy. A nested call must
ride its conversation's existing lane; the acquisition discipline for (object claim,
lane claim) must be provably deadlock-free against lane waiters, and tested as such.

### §5.1 The acquisition discipline (SETTLED 2026-07-14, frozen before any code)

Two facts defuse the OS-lock intuition. Claims here are *cooperative fiber claims* —
`owner: (TaskId, epoch)`, FIFO waiters, depth for same-owner re-entry — plain
parent-side data mutated only between yields (the `ext_prelude` machinery). So we get
two powers OS locks never have: **atomic multi-acquire** (take several resources in
one scheduler step, or none) and a **complete waits-for graph** (host-op callbacks run
on the *caller's own fiber* — `service_host_op` runs inside `extension_call`'s frame
loop — so every wait in a re-entrant call web is an ordinary task parked on a claim
with a known owner, all in one VM).

Disciplines considered and rejected: *object-then-lane* deadlocks (A holds O waiting
for a lane; all lanes held by calls whose callbacks nested-send to O — cycle);
*lane-then-object* (strict hierarchy) is provably free of cross-kind cycles but pins a
lane per queue-sitter, so one slow hot object starves every other object on the peer —
head-of-line blocking is the disease lanes exist to cure.

**The frozen rules:**

1. Resources: per-object claims (FIFO mailbox, owner = (task, epoch), depth-capped
   re-entry) and per-peer lane pools. All state parent-side, mutated between yields.
2. A top-level send acquires **(object, lane) jointly and atomically** — it parks with
   a want and is granted both or neither; it never holds one kind while waiting for
   the other. Object FIFO is primary (mailbox order); a freed object is **reserved for
   its head waiter** (no barging); freed lanes go to reserved heads in per-peer FIFO
   order.
3. A **nested** send (the task already has a bound conversation on the peer — it is
   executing a host-op callback) **rides the bound lane** and acquires the object
   claim only. A nested send never waits for a lane. (If nested calls took fresh
   lanes, N concurrent calls whose handlers all call back would exhaust N lanes and
   deadlock at full load.)
4. Same-owner re-entry on an object nests `depth++`, capped at 16 (as extensions).
5. Class-side calls and `serviceStop` claim a lane only, never an object.
6. At every object-claim park, walk the waits-for graph
   (`waits_for(task) = owner of the claim the task is parked on`; a joint waiter's
   edge is its wanted object's owner). If the walk closes on the parking task,
   **raise a catchable deadlock error naming the cycle instead of parking**.
   Decision: the error lands at whichever task closes the cycle (timing-dependent);
   it is catchable and names every participant.
7. The actor guarantee is a **boundary-mailbox** guarantee: worker-side, a hosted
   object passed as a live `Arg::Resource` is called directly — ordinary cooperative
   local code, interleaving only at park points, as anywhere in a single VM.

**Why detection is complete for the resource layer:** joint waiters hold nothing
while waiting; a stuck lane always traces to its holder task being parked on an
object claim (callbacks run on the holder's own fiber); therefore every true
claims-layer deadlock contains an object-claim cycle, and every object-claim cycle
is caught at park time. The irreducible residue — object↔object cycles from mutual
synchronous re-entrant calls — is application-level (two gen_servers calling each
other), detected and raised, never a silent hang. A worker whose handler simply
never answers hangs exactly as it can today with one lane; not a slice-5 regression.

**Deadlock tests (land with the machinery, before/with the wiring):**
- Shape-1 regression: all N lanes busy with calls whose callbacks nested-send to an
  object held by an (N+1)th slow call — drains.
- Lane exhaustion under nesting: N lanes, N+k callers, every handler calls back —
  completes (rule 3).
- No head-of-line blocking: one slow hot object saturated; calls to a second object
  on the same peer proceed at lane speed (joint-atomic beats lane-first).
- Mutual-call cycles: two-party, three-party ring, and cross-peer (the waits-for
  graph is parent-side regardless) → catchable error naming the cycle, no hang.
- Re-entry to the depth cap and past it.
- FIFO fairness per object under contention; reserved head not barged.
- Per-object totals stay exact with lanes > 1 (the serialization test, generalized).
Scoping note (superseded 2026-07-14): worker peers now HAVE callbacks — slice 4.5
landed host-ops before the claim slice, by decision (building claims onto the
one-park-for-terminal loop and then swapping in the conversation loop would have
built the trickiest integration twice, and left cycle detection with no trigger
path until 5c). The full deadlock list is therefore end-to-end testable against
thread workers in 5a itself, on top of the claim module's own unit tests.

**Observability (decided 2026-07-14, lands with 5a):** the claim system exports its
shapes — `VM.claims` (live structured snapshot: per peer/object owner + depth +
queue + each waiter's park time, lane pools, and the waits-for edges themselves) and
`VM.claimsReport` (rendered, contention-sorted, longest live wait-chain called out —
the pre-deadlock warning), plus accumulated counters in the `ext_stats`-style
registry (acquisitions, contended, total/max wait, queue high-water, max nesting,
deadlocks detected). Hosted services also start feeding `VM.boundaryStats` rows
(claim-wait in the existing column — one diagnosis surface), and the driver's
global-deadlock report dumps the claim graph beside the wake-log ring.

**Sequencing:** 5a = generalize the claim machinery (shared module + exhaustive unit
tests) and adopt it for thread workers (per-object claims + in-memory lanes; the
one-token serializer dies; `WorkerService.host:class:lanes:` — decided surface).
5b = process workers (N conversation sockets at spawn). 5c = extensions: manifest
`lanes` field (append-only, absent = 1), SDKs serve lanes on threads, claim key
moves from connection to resource id — the DB story cashes out here.
`serviceStop` decision: stop-flag + drain — refuse new top-level sends immediately,
wait for in-flight conversations to finish, then stop each lane and join.

## 6. Cross-isolate channels (CSP across the boundary)

A channel endpoint becomes portable to a Quoin peer. Design, per the seam analysis:

- **A relay endpoint is a native state**, not a shared channel: it parks local
  senders/receivers with the existing machinery (`ChannelPark`, `ParkOutcome`,
  `Wake::Channel*` — the resume vocabulary transfers verbatim; `wake_channel_task` is
  the single local wake choke point) and forwards operations to the counterpart isolate
  as protocol messages over the peer's transport.
- **Values serialize at the endpoint** (wire data model; `Gc` values cannot cross —
  `channel.rs` values are arena-bound). The worker-lane plumbing is the precedent.
- **Correlation ids replace `(TaskId, epoch)`** across the boundary — those are VM-local
  (waiter entries are meaningless remotely); the relay maps remote correlation ids to
  locally parked tasks.
- **The deadlock detector must keep seeing life**: the driver declares global deadlock
  when `ready` and the reactor are both empty (`runner_driver.rs:719-741`). A relay MUST
  hold a live reactor future whenever a local task is parked on a remote counterpart —
  which is also simply how it receives the reply. (Thread-peer lanes already register
  reactor futures via `IoRequest::WorkerRecv`.)
- **Semantics preserved**: bounded caps give cross-isolate backpressure (a full remote
  buffer parks the local sender); `close` propagates as a message and wakes both sides'
  waiters with the standard `ChannelClosed`; the cancelled-receiver redelivery rule
  (`channel_redeliver`) applies on whichever side holds the value.
- **Scope**: Quoin peers only. Foreign extensions keep request/response — a Rust/Python
  process cannot host Quoin channel semantics, and shouldn't pretend to.

## 7. Boundary profiling (diagnosing chattiness)

*Added 2026-07-13 after review: the cost gradient (stance guarantee 5) is only usable
if developers can **measure** it — placement decisions need data, not vibes.*

Every crossing already funnels through one choke point (the proxy send), so the data
is nearly free to collect:

- **Host-side counters, always on**: per `(peer, class, selector)` — call count, bytes
  out/in, total wall µs, and **claim-wait µs separately** (mailbox contention is its
  own diagnosis, distinct from transport or remote work). Two `Instant` reads against
  a ≥10µs round-trip floor is noise.
- **Remote decomposition**: peers report `handler_micros` in reply frames (append-only
  protocol field, all three SDKs). A round trip then splits into claim-wait +
  transport/encode + remote handler — precisely the chatty-vs-slow distinction:
  - `vec.at: — 40,000 calls, 480ms total, 91% transport` → *batch the API / move the
    object*;
  - `conn.query: — 14 calls, 210ms total, 93% remote handler` → *the work is slow;
    placement is fine.*
- **Surface**: `VM.boundaryStats` (structured rows, precedent `VM.stats`/
  `aotRefusals`) plus a rendered report sorted by total time with a chattiness
  callout. Per-object breakdown (hot-object hunts) behind a flag.
- **The gleam, named not promised**: the replay event log (§8) is the natural
  substrate for real distributed traces — chrome-trace/samply-style spans across VMs,
  with Quoin peers eventually contributing full span trees. Arc-4 adjacency; the
  counters come now.

**As built (slice 2):** `BoundaryStats`/`BoundaryRow` on each `NativeExtension`,
registered in `vm.io.ext_stats` (rows survive a dead/dropped peer — the post-mortem);
recording at the two call sites in `extension.rs`, with claim-wait measured in
`ext_prelude`'s queued path and nested host-op traffic metered through
`service_host_op`. `handler_micros` rides every `CallReturn*` terminal as an
append-only field, carried out-of-band of `Msg` as `ReplyMeta`
(`encode_with_meta`/`decode_frame_with_meta`) so the 50-odd `Msg` construction sites
stayed untouched; both SDKs stamp it at their serve/nested-dispatch write sites.
Surface: `VM.boundaryStats` (sorted rows) + `VM.boundaryReport` (rendered, sorted by
total cost, chattiness callout at calls ≥ 100 and transport ≥ 60%). One decomposition
caveat, documented on the field: a handler that calls back into the host (apply_block)
counts that nested time as *handler* — from the host's view it is still time the peer
held the call.

## 8. Replay hooks (ride along with this arc — decided 2026-07-13)

The archaeology reduced deterministic replay to a startlingly small surface. Everything
in the scheduler is already deterministic given two streams — all queues are FIFO
`VecDeque`s, no hash iteration anywhere scheduler-visible, task/stream ids and park
epochs are monotonic:

1. **The scheduling decision** — one site, the ready-pop (`runner_driver.rs:704-713`,
   where `QN_SCHED_STRESS` already hooks): record the chosen `TaskId`; replay forces it.
2. **External payloads** — one site, the driver `deliver` closure's `Io` arm
   (`runner_driver.rs:639-654`): record `(TaskId, IoResult)`; replay short-circuits the
   reactor with the logged result (a record/replay `IoBackend` wrapper behind the
   existing `perform` trait is the natural shape). Deadline wakeups need only their
   win/lose outcome.

Deliverables with this arc: a ring-buffer event log behind an env flag (nanoseconds when
off), the two hooks, and the **divergence test** — record a stressed run, replay it,
assert identical event streams — which doubles as the enforcement that *every new wake
path added by this very arc* (peer transports, channel relays) flows through the logged
points. Known out-of-log inputs to document, not chase: wall-clock reads
(`Timestamp.now` etc.), `[OS]Env`, the `ps`-collection deadline — replay either stubs
them from the log later (full replayer, arc 4) or names them as divergence points.

**As built (slice 1, `src/replay.rs` + `tests/wake_replay.rs`):** three streams, not
two — the yield-boundary *preempt decision* (`Rotate`) is logged at every cooperative
yield as well, which is what lets replay know where the yields fell without consulting
the stress rng, and makes the log self-delimiting (a pick with no preceding rotate =
the previous task parked). Two learnings: (a) a process drives the scheduler more than
once (the stdlib load drives before the program; the REPL drives per line), so the log
carries one `RUN` section per driver run, paired up in process order; (b) the yield
cadence (`QN_BATCH`, forced to 1 by the stress modes) determines where boundaries fall,
so the header records it and replay validates the match. Slice-1 replay re-performs
real I/O and forces delivery *order* (payloads content-hashed so divergence is
reported); injecting logged results is the arc-4 replayer. Env surface:
`QN_WAKE_RECORD=<path>`, `QN_WAKE_REPLAY=<path>`, `QN_WAKE_LOG=1` (diagnostic ring,
dumped on global deadlock), `QN_WAKE_DEBUG=1` (delivery trace). Worker VMs stay
unlogged until convergence names them.

**Scope, field-tested (2026-07-13):** replaying `qn test qnlib/tests` diverges, and
that is the expected boundary, not a hook bug. Slice-1 replay pins *the schedule*;
it cannot pin *external timing*: the suite spawns extensions (socket-path probe
retries vary with child startup speed), reads sockets (chunking varies), and its
recorded event streams differ run-to-run even without replay. Two findings came out
of the field test: (a) a genuine determinism bug — the Rust SDK serialized manifest
selector lists in `HashMap` order, so manifest bytes differed per process (fixed:
both SDKs emit sorted selector lists — wire bytes must never depend on hash order);
(b) the rule of thumb — programs whose external inputs are deterministic (timers,
channels, plain file reads, schedule races) replay end-to-end today; programs with
timing-dependent externals (extensions, sockets, subprocesses) need the arc-4
injection wrapper, which feeds recorded results instead of re-performing.

## 9. Slicing (proposed)

1. **Replay hooks + divergence test** (small, first — everything after must stay
   logged). DONE.
2. **Boundary profiling** (§7): host-side counters + `VM.boundaryStats` +
   `handler_micros` — early because it is valuable against extensions *today*. DONE.
3. **Peer-protocol convergence for process workers**: `worker-serve` speaks `Msg`
   (manifest, Call, host-ops, nesting); the pump/envelope retire; plain
   `send:`/`receive` ride `Call`-shaped frames. DONE (see the gap list's as-built
   note: two sockets — conversation + mailbox).
4. **Hosted objects** (`Worker.host:`): object table + MNU proxy on the converged
   protocol, thread + process backings; claim machinery shared with extensions;
   lifetime via proxy-drop release. DONE (as built, 2026-07-14): `WorkerService`
   upgraded IN PLACE (§2's "evolve, don't invent beside") — class form only; the
   block form waits for portable-block work. Dispatch = `Call{class_name, op, recv,
   method_args}` → `CallReturn*`, over the dispatch lane (thread: owned `Msg` values,
   no pump; process: via the conversation socket). The worker's serve loop is native
   (`Worker.hostServe:`, replacing the synthesized Quoin `perform:` loop) over a
   rooted `vm.hosted` table (the `handle_table` rooting pattern). THE RULE SHIPPED:
   a method's non-portable object return is HOSTED (`CallReturnResource` → parent
   mints a sub-proxy); same-worker proxies pass back as `Arg::Resource` live
   references; proxy drop reaps into `Call.releases`; errors carry the worker's
   rendered trace as `remote_stack` (labeled "(worker)"), surfacing as
   `ex.remoteStack`. One discovery worth keeping: the native `call_method` answers
   NIL on a lookup miss (hook semantics) — remote dispatch needs send semantics, so
   `call_method_mnu` exists now and hosted dispatch uses it; and a `Thrown` error's
   message lives in `vm.exceptions.active`, not the error value. The one-token
   serializer remains until per-object claims (slice 5); the MNU-seam proxy hook
   remains until hosted manifests (§10).
4b. **Worker host-ops** (slice 4.5, inserted 2026-07-14 ahead of slice 5 by
   decision — claims land on their final substrate, and cycle detection gets a real
   trigger path): the conversation loop both directions + block handles, completing
   slice 6's "never an error" rule. DONE — as-built in §3a and gap-list item 1.
5. **Per-object mailboxes + lanes** (§5) — after hosted objects, when the claim
   machinery is already generalized; the lock-ordering discipline gets settled and
   tested here. Discipline SETTLED as §5.1 (2026-07-14) before any code; sub-slices:
   5a = shared claim module + unit-tested discipline + thread workers
   (`host:class:lanes:`) + claim observability (`VM.claims`/`VM.claimsReport`),
   5b = process workers (N sockets), 5c = extensions (manifest `lanes`, SDK
   threading, per-resource claims).
6. **Portable-block arguments** for Quoin peers (thread first; process blocked on
   source-shipping and falls back to handles). v1 DONE (2026-07-14): the ship path
   for thread backing — snapshot at the encode seam, `DispatchReq.blocks` sidecar,
   worker-side rebuild. COMPLETED by slice 4.5 (same day): the handle fallback
   landed, so blocks never refuse — see §3a's as-built note for the full rule.
7. **Cross-isolate channels** (thread peers first — pure lane relay; process peers via
   the socket).
8. `WorkerService` reimplemented as sugar over `Worker.host:` (or deprecated into it).

Each slice lands green on its own; supervision (arc 3) starts once 4 is stable, since
"restart the isolate and re-host" is where proxy lifetime and supervision meet.

## 10. Open questions (to settle during, not before)

- Does `Worker.host:` take a portable block (evaluated remotely) or a class + init args
  (`WorkerService.host:class:` shape)? Block form is more general; class form survives
  process peers without source-shipping. Likely: both, block form thread-only at first.
- Proxy identity: `==` on two proxies to the same hosted object; `ps`/introspection
  rendering of hosted objects.
- **The lock-ordering discipline** for (object claim, lane claim) under nested
  re-entrancy (§5's care point) — flagged as the scariest part of the design; must be
  written down and deadlock-tested before slice 5, not discovered during it.
  **SETTLED 2026-07-14 — frozen as §5.1** (joint-atomic top-level acquisition,
  nested rides the bound lane and waits only for objects, park-time cycle detection
  raising catchably). Decisions: `host:class:lanes:` surface; the error lands at the
  cycle-closing task; `serviceStop` = stop-flag + drain.
- **Decoupling proxy dispatch from the VM miss path** (raised in review, 2026-07-14):
  the service proxy's MNU-seam hook (`try_service_call` in `vm.rs`) is tolerated for
  now but must not be permanent. The exit is the extension pattern, already in-tree:
  `install_ext_class` puts a dispatch node in an ordinary method table — no hook —
  because the manifest enumerates selectors. Once hosted classes declare theirs (the
  ready message carries the selector list; the handshake itself runs before the unit
  compiles), the parent installs a real class and the hook is deleted. Full dynamism
  (`doesNotUnderstand:` as a language protocol, Smalltalk-style) is a separate,
  independent language feature — QUOIN_TODO — not the load-bearing mechanism here.
- Should plain `Worker.send:`/`receive` mailboxes be re-expressed as a cross-isolate
  channel pair once §6 lands (one concept fewer)?
- Block RETURNS from hosted methods (noticed during slice 6): today they take the
  hosted-resource path and come back as sub-proxies — is remote `value:` dispatch on
  a block sub-proxy the semantics we want, or should a portable block ship back
  symmetrically (needs a reply-side sidecar)?
- Structured (non-blob) Quoin-to-Quoin stack frames — format, and whether the debugger
  can step across the boundary.
