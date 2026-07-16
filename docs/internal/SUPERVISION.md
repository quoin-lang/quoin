# Supervision — restart is a property of the hosting relationship

*Status: DESIGN, 2026-07-15, reviewed same day — every §10 question is decided;
ready to slice. Arc 3 of the concurrency road
(`CONCURRENCY_MODEL.md`). Grounded in an archaeology pass over the failure seams
(`extension.rs`, `worker_spawn.rs`, `worker_service.rs`, `claims.rs`,
`channel_relay.rs`) at main @ a378b39; file:line cites are from that pass. Companion
docs: `ACTOR_OBJECTS.md` (arc 2, whose §9 closes with "restart the isolate and re-host
is where proxy lifetime and supervision meet" — this document is that meeting),
`EXT_PACKAGING.md` (whose deferred auto-respawn this subsumes).*

## 0. The thesis

Stance guarantee 6 gives **containment**: a dying isolate yields catchable errors at
its boundary and never takes the VM down. Supervision adds **recovery**: the system
heals instead of merely explaining. The promise is *availability, not state* — a
restarted peer is a fresh incarnation, never a resurrected one.

Two facts make this arc smaller than it looks:

1. **Supervision reacts to death, never to errors.** In Erlang the two are one thing —
   an uncaught error *is* process death. Quoin already split them: a hosted method
   that raises delivers a catchable error **value** across the boundary
   (`ExtensionError` + `remoteStack`) and the isolate lives on. So the entire
   error-handling surface stays exactly as it is; supervision's trigger is only the
   peer *disappearing* (§2). This keeps the policy surface tiny — there is no
   "restart on which errors?" matrix, because the answer is *none*.

2. **The parent already holds a complete respawn recipe.** Since the `class:`-form
   removal (arc 2 finale), every hosting surface is a portable block + `args:` — and a
   portable block freezes its captures at ship time *by construction*, with args
   arity-checked and shipped at spawn. Re-running the recipe is therefore
   well-defined: same frozen captures, same args, fresh isolate. Extensions likewise:
   the recipe is the `quoin.toml` command line (or the explicit `Extension spawn:`
   arguments). Supervision is a *retained recipe plus a policy for when to re-run it*,
   not a new capability.

**Revision of an earlier stance, deliberate:** `CONCURRENCY_ARCH.md` (§L1, §16) called
restart strategies "a library concern, deliberately not runtime." Half survives: policy
*expression* stays a small qnlib value, and supervision *trees* stay a later library.
But the mechanisms — death detection without an in-flight call, proxy rebinding, claim
and mailbox semantics across a restart, reactor-integrated child watches — are runtime
seams a library cannot reach. The split this document draws: **runtime owns death
events + the respawn/rebind mechanics; the policy is data; trees are someone else's
loop over the events.** Sharpened in review (Damon, 2026-07-15): sane, predictable
defaults ship built in, and the exposed primitives must be sufficient for a Quoin
library to implement a robust supervision strategy of its own — see §10.1 for the
contract this implies.

## 1. Today: fail-fast, no restart (the archaeology)

What a dying peer looks like right now, per seam:

| seam | detection | caller sees |
|---|---|---|
| extension, mid-call | reply EOF + `note_if_exited` try_wait (`extension.rs:476`, `:1121`) | typed `IoError #closed` — "Extension process died (…)" (`:515`) |
| extension, idle | none — caught at next call via `ctx.dead` (`extension.rs:1368`) | same, "already exited" |
| extension, handshake | `read_reply_frame_timed`, `QN_EXT_HANDSHAKE_TIMEOUT_MS` (`:1508`) | typed `IoError #timedOut` |
| process worker, mid-call | conv pump breaks, reply sender drops (`worker_spawn.rs:545`) | **untyped** `Other` — "service call '…': the service exited mid-call" (`worker_service.rs:696`) |
| process worker, idle | mailbox-reader EOF synthesizes the done error (`worker_spawn.rs:895`) | at `join`: "worker process exited{status}"; at next call: "the service has exited" (untyped) |
| thread worker, panic | `catch_unwind` in the thread body (`worker_spawn.rs:45`) | done-lane `Err("worker panicked: …")`; in-flight calls as above |
| channel endpoint, link death | relay agent sees `ChanFrame(None)`, wakes all pending ops (`channel_relay.rs:398`) | send → `ValueError` "closed"; receive → nil |
| claims, holder dies | **no death broadcast** — the erroring holder's `end_call_and_wake` cascades, each promoted waiter fails fast on the dead peer (`worker_service.rs:459`, `QUOIN_TODO.md:413`) | each queued caller unwinds with the death error, FIFO |

Kill-on-cancel is the one place the *host* causes a death: a cancelled extension call
desyncs the framed conversation, so `finish_outcome` kills the peer
(`extension.rs:1176`). Worker conversations abandon cleanly instead and the service
survives — that asymmetry stays.

Gaps found, to fix regardless of policy (they become slice 0):

- **(a)** Parent-side `block_handles` are released only in `service_stop`
  (`worker_service.rs:1264`); an *unexpected* death leaks them until VM exit.
- **(b)** On link death the owner side of a shipped channel never retracts the dead
  remote waiter entries from its local channel, and a local task blocked on that
  channel waiting for the dead side is not woken. (The full "no sender will ever
  come" case is unsolvable locally — that residue is the arc-4 wait-graph stitching,
  documented in `ACTOR_OBJECTS.md` §6 — but the bookkeeping retraction is just a bug.)
- **(c)** No idle-time death detection for extensions (workers have it via the
  mailbox reader).
- **(d)** Worker death errors are untyped `Other` strings while extension deaths are
  typed `IoError` — callers cannot catch "peer died" as a kind, and supervision code
  will need to.

## 2. What death is (and is not)

**Death** := the isolate is gone or unusable, enumerated exactly:

- process exit (worker or extension child), detected by exit-watch, socket EOF, or
  `try_wait`;
- worker thread panic (caught; the thread ended);
- spawn/handshake failure of a *restart attempt* (exit-before-connect, version gate,
  handshake timeout) — counts as a death of the new incarnation, feeding backoff;
- kill-on-cancel of an extension (a real death, host-inflicted — restart-eligible);
- `terminate` — an explicit kill. **Not restart-eligible**: an explicit kill is an
  instruction, not a failure. Likewise `serviceStop`: stop means stop.

**Not death**, and never a supervision trigger: `ExtensionError` (the peer reported
and lives), any Quoin error raised by hosted code, deadlock errors from the claims
walk, `Cancelled` reaching a worker conversation (it abandons; the service survives),
a slow or unresponsive-but-alive peer. Hung-but-alive is the timeout/deadlock story —
`Async.timeout:` at call sites, `VM.claimsReport` for diagnosis — not supervision's.
No heartbeats, no health checks: death is OS-level truth, so detection is exact and
free of false positives (§9 for what we give up).

## 3. The supervisable unit, and where policy attaches

The unit is the **isolate** (worker or extension process); the **root proxy is its
name**. Hosted sub-objects, handles, and shipped channels are session state *within*
an incarnation, not units.

A sharp scope line: **supervision is for peers that serve, not jobs that finish.**
`with:`-hosted services and extensions are supervisable. Plain `spawn:`/`start:` jobs
are not — re-running a one-shot computation is a retry loop the caller writes in three
lines around `join` (which already reports the death, including a thread panic).
A user-level actor built on `spawn:` + `receive` loops can be supervised in user code
once lifecycle events exist (§7); the runtime does not guess which spawns are servers.

Surface (bikeshed open, §10): a `supervise:` option beside `lanes:`/`backing:` on the
hosting forms, and on `Extension spawn:`:

```quoin
var conn = Worker.host:{ |url| Db.connect:url }
           args:#( dbUrl ) backing:'process'
           supervise:(Supervise.always.backoff:100 cap:10000 max:5 within:60000);
```

Package extensions have no call site — `use pkg:*` spawns implicitly — so their policy
lives in `quoin.toml` under `[extension]` (`restart = "always"`, `backoff-ms`,
`max-restarts`, `window-ms`), read at `loadPackage:`. Default everywhere: `never` —
today's behavior, unchanged unless asked for.

The policy itself is **plain data** (a small immutable qnlib value, `Supervise.never`
/ `.always` with modifiers): it crosses as wire data, so the recipe + policy pair is
inspectable and loggable, and the runtime interprets it directly — no callback into
user code on the death path.

## 4. Restart semantics (the rules, candidate-frozen)

1. **Only death restarts** (§2's list, minus terminate/stop). Errors never do.
2. **Restart = re-run the retained recipe in a fresh isolate.** Worker: the shipped
   `PortableBlock` + args snapshot, re-shipped exactly as at spawn (captures frozen at
   the original ship — restart is deterministic with respect to the recipe; its side
   effects, e.g. `Db.connect:`, re-run — that is the point). Extension: re-spawn the
   command, re-handshake, re-gate.
3. **The root proxy rebinds in place.** Proxies and the installed service classes
   dispatch through the parent-side service context; restart swaps the context's link
   and root binding, and bumps an **incarnation counter**. Erlang precedent: a
   registered name reaching the fresh `gen_server` is not "hiding failure" — the
   callers who met the death got their error; later callers meet a healthy peer.
   Class-side sends flow through the same context and rebind with it.
4. **In-flight and queued sends at death time error** (the typed death error, §5's
   `PeerDied…` shape — today's cascade, better typed). Nothing is replayed:
   at-most-once stays the law; a queued send may assume state the death destroyed.
5. **Sends arriving during the restart window park** — ordinary cancellable,
   `Async.timeout:`-composable parks, labeled for `VM.ps` ("supervise restart wait").
   They dispatch to the new incarnation when ready. Give-up (rule 7) wakes them all
   with the death error. Parking (vs failing fast) is the availability supervision
   buys; failing here would make every caller write its own retry loop, which is the
   disease.
6. **Everything minted by an incarnation dies with it, permanently.** Sub-proxies,
   block handles, shipped channel endpoints carry the incarnation stamp; touching a
   stale one raises the death error naming the incarnation. Only the root rebinds —
   the recipe makes the root and nothing else. (This is what "availability, not
   state" means operationally.) Death also releases the incarnation's parent-side
   block handles (gap (a)) and closes its channel endpoints (already true endpoint-
   side).
7. **Backoff and give-up:** exponential backoff from `backoff` ms doubling to `cap`;
   more than `max` deaths inside `window` ms → **give-up**: the service enters a
   permanent dead state, waiters and all future sends get the death error annotated
   "gave up after N restarts", and a `gaveUp` lifecycle event fires. Give-up is
   per-process-permanent in v1 (a half-open circuit that retries later is a
   documented non-goal for now). For a crash-looping package extension this *is* the
   circuit breaker `EXT_PACKAGING.md` deferred: spawn storms are bounded by backoff,
   ended by give-up.
8. **Mailbox/claims across restart:** the claims registry rows persist (post-mortem
   property, unchanged); the new incarnation starts with empty mailboxes and a fresh
   lane pool. The waiter cascade needs no new machinery — rule 4 is exactly today's
   unwind, and rule 5's parkers re-enter the ordinary joint-atomic acquisition
   against the new incarnation.
9. **Manifest stability:** a restarted peer must present the same manifest (wire
   bytes are sorted, so equality is meaningful). The installed proxy classes were
   built from the old manifest; a differing one means the recipe is not deterministic
   (or the binary changed underfoot) — treat as a failed attempt with a clear error,
   feeding rule 7. Same rule for the worker ready-message's class manifest.

## 5. Detection (the runtime work)

- **Process children (workers and extensions): a reactor child-exit watch.** Today,
  idle deaths are invisible (extensions, gap (c)) or visible only to the mailbox
  reader (workers). A *supervised* child registers an exit watch with the reactor —
  `kqueue EVFILT_PROC` on macOS, `pidfd` on Linux — so the exit becomes an ordinary
  `Io` delivery to a supervision task. **Guarantee 8 applies in full:** the wake rides
  the logged `Io` path or it is a bug; the divergence test grows a supervised-death
  case. Unsupervised children keep today's lazy detection and pay nothing.
- **Thread workers:** the done-lane close *is* the event (already synthesized for
  panics via `catch_unwind`); the supervision task consumes it where `join` would.
- **Typed death error (gap (d), slice 0):** one error shape for every seam —
  **DECIDED (Damon, 2026-07-15): a new ROOT error class, distinct from `IoError`.**
  `IoError` is too broad and overlaps too much with ordinary user-catchable I/O
  failures — "the socket you were reading closed" and "the isolate hosting your
  object died" must not share a catch clause. Working name `PeerDiedError` (name
  still open, §10), carrying peer name/kind, incarnation, exit status or panic text,
  a `reason` symbol (`#exited` / `#panicked` / `#spawnFailed` / `#gaveUp` /
  `#staleIncarnation`), and `remoteStack` when one exists. The extension death path
  migrates off `IoError #closed` onto it (breaking, worth it before anyone depends
  on catching `IoError` for a dead peer); the worker paths' `Other` strings are
  replaced. Supervision code — ours and users' — catches death as a kind, not a
  substring.

## 6. Events and observability

- **Lifecycle events over a channel** — the Quoin-native monitor primitive, no new
  concepts: `service.events` (and an extension-handle equivalent) lazily creates a
  bounded channel of event records: `spawned(incarnation)`, `died(reason,
  incarnation)`, `restarting(delayMs, attempt)`, `gaveUp(reason)`, `stopped`. Emission
  must never park the runtime: bounded buffer, drop-oldest, with a `droppedCount` in
  the record stream. One consumer per channel (channel semantics); a fan-out
  supervisor-of-supervisors is user code. This is deliberately the *entire* tree
  story for v1: Erlang-style trees are a loop over these events, in qnlib or user
  code, later.
- **`VM.peers`** (renamed from the draft's `VM.services` in review — Damon,
  2026-07-15: "services" was stale vocabulary once WorkerService was removed, and
  the roster covers plain workers and extensions too; "peers" matches
  `PeerDiedError`/`PeerClaims` and the stance's "one protocol, three peer
  kinds"): per peer — status (`running` / `restarting` / `dead` / `gaveUp`),
  incarnation, restart count, last death reason, policy. `VM.claims` rows gain
  an explicit dead-peer marker (today a dead peer is only implicit in its
  unwinding waiters). Counters (restarts, give-ups) join the `ext_stats`-style
  registry; `VM.boundaryStats` rows already survive death per incarnation —
  they gain the incarnation in the key or a merged row, bikeshed.

## 7. Interaction with replay (guarantee 8)

Everything supervision adds is schedulable state: child-exit wakes are `Io`
deliveries (logged), backoff timers are deadline wakes (logged), and policy is pure
data — so given the log, restart decisions replay deterministically. What does not
replay is what never replayed: real spawn timing, pids, socket accept order — the
documented arc-4 injection boundary. The divergence test must cover a
record/replay run containing a supervised death + restart.

## 8. Slicing (proposed)

0. **Hygiene, lands first, valuable alone** (no policy machinery): the typed
   `PeerDiedError` unification (gap (d)); release parent-side block handles on
   unexpected death (gap (a)); retract dead remote channel waiters on link death
   (gap (b)'s fixable half); explicit dead-peer marker in `VM.claims`.
   **AS BUILT (2026-07-15).** `QuoinError::PeerDied { peer, reason, message }` →
   the `PeerDiedError` bootstrap class (`reason`/`peer` accessors); raised at
   every seam — `extension_dead_error` (off `IoError #closed`, the breaking
   change), the service mid-call/corpse/`serviceStop`-join paths, and `join`
   (the done lane's error side became `WorkerExit::{Failed, Died{reason,
   detail}}`, so body-reported failures stay ordinary errors without string
   matching; a thread panic is `Died(#panicked)`, pinned by unit test).
   `note_service_dead` (idempotent, every parent-side detection seam) drains
   the block handles and stamps `PeerClaims.gone = "died"` — `"stopped"` on the
   clean path — surfaced as the `gone` key in `VM.claims` rows and a
   `DIED/STOPPED (post-mortem)` marker in `claimsReport`. Channels went beyond
   the plan: `ChanLink.shipped` counts per-link endpoint refs at the one ship
   choke point, so `channel_link_died` both purges the dead link's parked
   remote RECEIVERS (the silent value-loss bug — a later send now reaches a
   live receiver, e2e-verified by SIGKILLing the child mid-park) and repays the
   ship refcounts, unrooting what only the dead peer referenced. Remote
   SENDERS deliberately survive: their values are already here and deliver
   like letters posted before death (the Ack drops harmlessly). Known residue:
   a child that dies *gracefully* mid-call (its serve fiber unwinds before the
   socket closes) answers with the synthesized "the hosted serve loop exited
   mid-call" `CallReturnError` — an ordinary error terminal the parent cannot
   distinguish from user code, so that race stays untyped until a slice-1
   death event can reclassify it.
1. **Death events:** reactor child-exit watch for process children (kqueue/pidfd),
   thread done-lane unification, lifecycle event records + per-service `events`
   channel, `VM.peers`, replay divergence coverage.
   **AS BUILT (2026-07-15).** One `Arc<LifeSink>` per spawned peer
   (`src/runtime/lifecycle.rs`, registered in `vm.io.lives`): status +
   bounded(64) staging of wire-data event records, emitted SINGLE-SOURCE at
   the done-lane producers (thread wrapper, process mailbox reader, extension
   death seams, the exit watch) with a first-terminal-wins flag, so racing
   observers collapse to one death. Consumers: `w.events` / `e.events` /
   `svc.serviceEvents` (the `serviceStop` naming precedent) answer a Quoin
   Channel pumped by a `LifecycleBoot` task parking on the ordinary
   `WorkerRecv` request — guarantee 8 with no new wake machinery; history from
   spawn time; the terminal closes the stream; one channel per peer
   (`vm.life_channels` is cache + GC root). The exit watch is
   `IoRequest::ChildExit{pid}` (macOS: a dedicated kqueue's
   `EVFILT_PROC`/`NOTE_EXIT`, the kqueue fd polled via `Async::new_nonblocking`
   — plain `Async::new` runs an `fcntl` the kqueue fd rejects, and treating
   that as "child gone" made the watch fire instantly on live children, caught
   by unit test; Linux: `pidfd_open`; elsewhere: peek-polling), guarded by a
   `waitid(WNOHANG|WNOWAIT)` peek before AND after registration (a kqueue
   `NOTE_EXIT` does not fire retroactively on a zombie) — pure observation,
   the owner's reap discipline untouched.
   *Race analysis (asked in review, recorded here).* The peek/arm/re-peek
   sandwich is sound for exit timing by interval walk: an exit before the
   first peek is seen (WNOWAIT sees a zombie; ECHILD = already reaped); an
   exit between peek and registration is the dangerous window (NOTE_EXIT
   never fires retroactively) and is exactly what the RE-peek — which runs
   strictly after the `kevent` registration returns — catches; an exit after
   the re-peek necessarily post-dates the armed knote, so it fires. There is
   no fourth interval. Two theoretical residues, both missed-wakeup-shaped
   (never false deaths): (1) the peek does not retry `EINTR` and reads it as
   "running" — practically unreachable, since `WNOHANG` never blocks and
   non-blocking syscalls have essentially no interruption window; (2) `waitid`
   keys on the PID, not the process, so an exit + reap + pid-recycle to
   another of our children could misdirect the peek — irrelevant on the armed
   paths (the knote and the Linux pidfd pin the PROCESS, and the exit event is
   already pending), and neutralized on the poll-only fallback by the reap
   discipline: the only reapers (`note_if_exited`, `kill_now`, `Drop`) all
   emit the sink's terminal before or at the reap, so a recycled pid implies
   the death/stop is already recorded and first-terminal-wins makes the
   watch's late-or-never firing harmless (worst case: a leaked parked watch
   task, which dies with the process like any relay agent). Armed once per extension on the
   first `events` ask ("arming = asking"; package extensions get roster rows
   but no watch until slice 3's policy). `terminate` and extension-handle drop
   quiet-stop the sink FIRST, so the supervision surface reads them as stops
   while `join` still raises the honest `PeerDiedError`. Deviations, recorded:
   staging drops NEWEST on overflow (+`eventsDropped`), not the drop-oldest
   above (`async_channel` cannot evict; events are rare); the events pump
   re-mints `reason` as a symbol at the boundary (wire data cannot carry
   symbols); the watch task marks only the sink — the claims `gone` marker
   still updates on lazy detection, not on watch fire (slice-2 material,
   where the watch consumer becomes the supervisor).
   *Postscript (2026-07-15): this slice's CI run wedged and led to the
   entombed-dispatch race — a send racing the pump's death-exit window parks
   forever on a reply entombed in the closed dispatch queue, invisible even
   to the deadlock detector (a live in-flight future). Caught live in the
   Lima VM by wake trace + gdb after ~100 CI-shaped iterations; fixed by the
   pumps draining their queues on exit. Pre-existing since hosted dispatch —
   load made it likely enough to see.*
2. **Respawn mechanics:** retained recipes (worker block+args are already held;
   extensions retain their spawn recipe), rebind-in-place with incarnation stamps,
   park-during-restart, give-up state, manifest-equality gate. Surface: `restart`
   manual trigger first (`service.restart` — supervision with a human in the loop),
   which proves the whole mechanism before any policy automates it — and stays as
   the library extension point (§10.1), not scaffolding.
   **AS BUILT, services half (2026-07-15; extensions are the slice's second
   commit).** `ServiceRecipe`, frozen at the original host: the `PortableBlock`
   (captures froze at first ship — rule 2 for free), path/lanes/backing, the
   spawn args as their classified `WorkerMsg`s re-sent verbatim — channels
   excepted, retained as VALUES in the traced `vm.recipe_chans` root and
   re-shipped against the new link (e2e: the fresh incarnation posts into the
   same parent channel) — plus the ready manifest, which IS the rule-9 gate
   (mismatch refuses to rebind with the full story and leaves the service
   dead-but-retryable; the orphan worker shuts down when its lanes drop).
   `serviceRestart` (the `serviceStop` naming family) lives only on the root
   (the recipe holder), refuses on running/stopped (§2), and on success
   rebinds the root state in place — fresh claims/convs/handles/stop flag,
   new chan link, new lifecycle sink — and bumps the shared incarnation cell;
   every proxy carries its mint stamp, checked BEFORE the state snapshot in
   dispatch, so the dead incarnation's sub-proxies raise `#staleIncarnation`
   forever. The rule-5 window is a `RestartGate` in the shared state: top-level
   sends park pre-snapshot (nested calls skip — their conversation belongs to
   the corpse and fails fast on its own lanes) and are woken all at once,
   re-snapshotting into the new incarnation or the typed death. Deterministic
   e2e for the window: the restart task sets the gate synchronously before its
   first park, so a send after `sleep:1` provably lands inside it.
   Bookkeeping per incarnation: fresh `PeerClaims` (label suffixed
   "(incarnation N)"), a fresh `LifeSink` (`VM.peers` rows carry
   `incarnation`; `serviceEvents` re-asked after a restart answers the new
   stream — the old one closed at its terminal, by slice-1 law); boundary
   rows deliberately MERGED across incarnations (§6's merged-row option).
   One slice-1 revision forced by construction: `note_service_dead` now also
   emits the death on the sink — the caller is about to CATCH the typed death,
   so `serviceRestart`/`VM.peers` must already agree it happened; the mailbox
   reader's own emission can lag on its thread, and first-terminal-wins
   collapses the double observation. Residues: thread-backed respawn shares
   every line but the spawn call, yet has no e2e (a thread service only dies
   by panic, which nothing can script); restart under record/replay sits
   behind the same external-timing boundary as deaths; the formal permanent
   `gaveUp` state waits for slice 3's attempt-counting.
   **AS BUILT, extensions half (2026-07-15).** `ExtRecipe` freezes the spawn
   inputs (command/args/cwd) plus the FIRST manifest — the rule-9 gate:
   `Extension.restart` re-runs `spawn_ext_process` (the spawn/connect/
   handshake front half, factored out of the original spawn) and refuses to
   rebind — abandoning the fresh child — unless the new manifest matches,
   class for class, lane for lane. On success the handle rebinds IN PLACE
   (fresh sockets/claims/sink/ext id, old lane fds reaped, old socket file
   removed, the dead incarnation's host-value handles released — the idle
   death path never had a failing call to do it) and the same installed
   classes keep working. Staleness rides the reap-queue identity — a restart
   swaps `resource_reap`, so an old instance's `Rc` no longer matches: the
   class-dispatch receiver path raises `#staleIncarnation` naming the
   incarnation, and the generic `args:` path refuses through the existing
   ownership check (message widened to name the dead-incarnation
   possibility — a typed error there is residue). Hardening forced by
   construction: a lane transport failure UNDER a call is now typed as the
   death directly in `finish_outcome` (kill_now + `PeerDiedError`), because
   the socket EOF races `try_wait` — the child can close its socket
   milliseconds before its exit is reap-visible, and slice 0's typed
   conversion silently lost that race (user/callback errors never travel
   that path, so `Io` there can only be the lane). Residues: extension
   restart-window sends fail fast typed rather than park (the service gate
   has no extension twin yet — slice 3 material if the policy wants it);
   `loadPackage:` extensions restart via the handle only (`init.qn` is NOT
   re-run — glue is installed classes, state is the process's, and
   availability-not-state says fresh).
3. **Policy:** the `Supervise` value + `supervise:` options + `quoin.toml`
   `[extension]` keys + backoff/intensity/give-up automation over slices 1+2.
4. *(adjacency, not this arc unless pulled)*: `WorkerPool` crash-respawn
   (`CONCURRENCY_ARCH.md` L1) becomes sugar over the same events + recipes.

Each slice lands green alone; slice 0 is worth shipping even if the arc pauses.

## 9. Rule-outs (recorded so future-us doesn't relitigate)

No state restoration or checkpointing (availability, not state). No send replay
(at-most-once is the law). No restart-on-error, ever — errors are values. No
heartbeats or liveness probes (death is OS truth; hung-alive is the timeout/deadlock
story — accepting this means a wedged-but-running peer is *not* supervision's to
catch, deliberately). No runtime supervision trees (events make them a library). No
half-open circuit retry after give-up (v1). No distributed supervision. Restart
never resurrects sub-proxies, handles, or channels.

## 10. Decisions (settled in review with Damon, 2026-07-15)

1. **Runtime/library split** — **DECIDED in direction (Damon, 2026-07-15): sane
   and predictable defaults built in, AND it must be possible to write a Quoin
   library implementing a robust supervision strategy of its own.** So: the runtime
   interprets the `Supervise` data policy (as drawn in §0), and the library-facing
   surface — lifecycle events (§6), the manual `restart` trigger (slice 2),
   give-up/incarnation introspection (`VM.services`), the typed death error — is a
   first-class contract, not an internal convenience. A library strategy is
   `Supervise.never` plus a loop over the events calling `restart`; the manual
   trigger re-enters the same rebind/park machinery, so a library-driven restart
   behaves identically from the moment it is requested (the gap between the death
   and the library's decision fails fast, which is honest).
2. **Rebind-in-place vs permanent-death-plus-new-proxy** (supervisor hands out fresh
   proxies via a registry). Rebinding keeps "objects are the only abstraction" — no
   registry concept — and matches Erlang's registered-name semantics.
   **DECIDED (Damon, 2026-07-15): rebind-in-place, root only (rules 3/6).**
3. **Park vs fail-fast during the restart window** (rule 5).
   **DECIDED (Damon, 2026-07-15): park** — cancellable, timeout-composable, labeled
   in `VM.ps`.
4. **Error shape** — **DECIDED (Damon, 2026-07-15): a new root error class,
   distinct from `IoError`** (too broad; overlaps user I/O errors). Residual
   bikeshed only: the name (`PeerDiedError` is the working name; `PeerError` if the
   root should also own non-death supervision failures someday), and whether
   give-up / stale-incarnation are `reason` symbols on the one class or subclasses.
   **Recommendation: one class, `reason` symbols** — the `IoError`-kind precedent,
   and catch clauses rarely want to split them.
5. **Package-extension policy override** — may a consumer override the package's
   `quoin.toml` policy at `use` time (no call site exists)? Project-level
   `quoin.toml` override table? **DECIDED (Damon, 2026-07-15): defer;
   package-declared only in v1.**
6. **Thread workers in v1?** The mechanism is shared (recipe re-run; detection is
   the done-lane) and threads are the *easier* backing.
   **DECIDED (Damon, 2026-07-15): yes, both backings from slice 2.**
