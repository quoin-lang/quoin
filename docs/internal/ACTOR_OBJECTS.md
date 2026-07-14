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
  park FIFO with epoch identity, nested re-entry composes, depth-capped.
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

**b. Structured stacks.** Quoin-to-Quoin errors need not be opaque: the `remote_stack`
blob carries the worker's real rendered trace initially (day one, free via PR #11's
field), with a structured-trace upgrade (frames as data, uniformly steppable) as a
later, purely additive field.

**c. Cross-isolate channels.** See §5.

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
2. **Manifest + object table in the hosted-worker serve loop** — replaces
   `SERVICE_LOOP_QN`'s single-instance `perform:` loop.
3. **Retiring the `{t,v}` envelope** — process-worker pump speaks `Msg` frames;
   `worker-serve` decodes with `decode_frame`. The control lane's request-id machinery
   (`worker_spawn.rs:351-405`) collapses into the protocol's conversation shape.
4. **Claim machinery generalized** — `NativeExtension`'s owner/depth/waiters become a
   shared "peer connection" state used by extension and worker proxies alike.
5. **Plain workers unchanged** — `Worker.spawn:`/`start:`/`send:`/`receive`/`join` keep
   their surface; only the wire beneath process backing converges. `terminate` stays
   process-only; thread isolates still cannot be killed (documented).

## 5. Cross-isolate channels (CSP across the boundary)

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

## 6. Replay hooks (ride along with this arc — decided 2026-07-13)

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

## 7. Slicing (proposed)

1. **Replay hooks + divergence test** (small, first — everything after must stay logged).
2. **Peer-protocol convergence for process workers**: `worker-serve` speaks `Msg`
   (manifest, Call, host-ops, nesting); the pump/envelope retire; plain
   `send:`/`receive` ride `Call`-shaped frames.
3. **Hosted objects** (`Worker.host:`): object table + MNU proxy on the converged
   protocol, thread + process backings; claim machinery shared with extensions;
   lifetime via proxy-drop release.
4. **Portable-block arguments** for Quoin peers (thread first; process blocked on
   source-shipping and falls back to handles).
5. **Cross-isolate channels** (thread peers first — pure lane relay; process peers via
   the socket).
6. `WorkerService` reimplemented as sugar over `Worker.host:` (or deprecated into it).

Each slice lands green on its own; supervision (arc 3) starts once 3 is stable, since
"restart the isolate and re-host" is where proxy lifetime and supervision meet.

## 8. Open questions (to settle during, not before)

- Does `Worker.host:` take a portable block (evaluated remotely) or a class + init args
  (`WorkerService.host:class:` shape)? Block form is more general; class form survives
  process peers without source-shipping. Likely: both, block form thread-only at first.
- Proxy identity: `==` on two proxies to the same hosted object; `ps`/introspection
  rendering of hosted objects.
- Should plain `Worker.send:`/`receive` mailboxes be re-expressed as a cross-isolate
  channel pair once §5 lands (one concept fewer)?
- Structured (non-blob) Quoin-to-Quoin stack frames — format, and whether the debugger
  can step across the boundary.
