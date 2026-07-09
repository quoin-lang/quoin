# VM correctness-audit repro corpus (2026-07-04)

Minimal repro programs for the bugs found by the async/fiber/scheduler +
`quoin-ext` audit of main @ `f752f61`. Each file's header comment describes the
buggy vs. correct behavior; run any of them from the repo root:

    target/debug/qn qnlib/stress/audit/sched/stale_deadline.qn

These are *repros*, not tests — several demonstrate bugs that are still open, so
this directory is not part of `qn test`. The fixed ones are additionally wired
into `tests/park_identity.rs` and `tests/fiber_ownership.rs` as self-checking
cargo regression tests.

## sched/ — scheduler & task-identity

| file | bug | status |
|---|---|---|
| `chan_ghost_misdeliver.qn` | ghost channel waiter + task-slot reuse delivers a value sent on ch1 to a task parked on ch2 | **FIXED** (park-identity epoch) |
| `channel_misroute.qn` | cancelled receiver's ghost entry misroutes channel A's value into the same task's later channel-B receive | **FIXED** (park-identity epoch) |
| `stale_deadline.qn` | disarmed `JoinTimed` deadline fires on reused task slots with a matching epoch — a 60 s timeout throws instantly | **FIXED** (global `park_seq`) |
| `stale_deadline_control.qn` | control for the above: a dummy task offsets slot allocation, so pre-fix it passed — proof of the slot-reuse mechanism | n/a (control) |
| `nested_timeout_min.qn` | minimal 2-statement form: nested `Async.timeout:` blames an outer deadline (`ms=22222`) instead of the inner `ms=2` | **FIXED** (global `park_seq`) |
| `nested_timeout_matrix.qn` | depth matrix 0–7 enclosing timeouts, bare and `onCancel:` forms (pre-fix failed at n=3,5,6) | **FIXED** (global `park_seq`) |
| `cancel_woken_joiner.qn` | cancelling a joiner already woken by the joined task's completion double-enqueues it → "task slot is empty" panic | **FIXED** (`wake.is_none()` guard) |
| `cross_task_fiber.qn` | resuming a fiber that is suspended inside another *parked* task corrupts it, then aborts the process ("attempt to resume a completed coroutine") | **FIXED** (fiber owner guard) |
| `resume_while_parked.qn` | same abort, minimal 2-task form, 100% without stress | **FIXED** (fiber owner guard) |
| `shared_fiber_resume.qn` | same abort, yield-suspended variant (~29/50 stress seeds; 0/50 post-fix) | **FIXED** (fiber owner guard) |
| `empty_gather.qn` | `Async.gather:#()` parks the caller forever; program exits 0 with the rest unexecuted | **FIXED** (immediate empty delivery) |
| `deadlock_exit.qn` | a globally deadlocked program exits silently with status 0 — indistinguishable from success | **FIXED** (loud deadlock diagnostic; exit status stays 0 by run-mode convention) |
| `lost_value_on_cancel.qn` | a committed channel handoff is dropped when the receiver is cancelled before running (send already reported success) | **FIXED** (re-delivered to next receiver / buffer front) |

## aio/ — async I/O

| file | bug | status |
|---|---|---|
| `close_while_read_parked.qn` | closing a socket while another task is parked reading it: reader never woken, fd never closed (lease re-inserts it) | **FIXED** (lease tombstone + op abort) |
| `listener_close_leak.qn` | `TcpListener.close` never closes the OS fd — port stays bound, backlog keeps accepting | **FIXED** (reap close drops listeners too) |
| `http_truncated_body.qn` | Content-Length body truncated by early EOF returns status 200 with a short body, no error | **FIXED** (IoError kind `#unexpectedEof`) |
| `http_chunked_eof.qn` | chunked body truncated at a chunk boundary surfaces as a hex `ValueError` instead of an unexpected-EOF `IoError` | **FIXED** (IoError kind `#unexpectedEof`) |
| `folder_open_panic.qn` | `[IO]Folder.open:` on a missing directory panics the VM (`unwrap`) instead of throwing a catchable `IoError` | **FIXED** (typed IoError, kind `#notFound`) |

## ext/ — extension mechanism (python fixtures alongside)

| file | bug | status |
|---|---|---|
| `concurrent.qn` / `concurrent_catch.qn` | two tasks calling one extension concurrently desync the connection ("unknown stream id"; a host-reach yield can kill the extension's serve loop) | **FIXED** (in-flight guard → catchable busy error) |
| `spawn_silent.qn` | handshake has no timeout: an extension that accepts but never answers `GetManifest` hangs the VM forever | **FIXED** (10s handshake read timeout, `QN_EXT_HANDSHAKE_TIMEOUT_MS`) |
| `deep.qn` | deeply nested `DataValue` reply overflows the host stack — uncatchable `Bus error: 10`, defeating crash isolation | **FIXED** (decode depth cap = 64) |

## top level

| file | bug | status |
|---|---|---|
| `native_reentry_recursion.qn` | unbounded native re-entrant recursion (`set_add → == : → set_add …`) overflows the real C stack — uncatchable SIGBUS (pure-Quoin recursion is fine) | **FIXED** (per-task re-entry depth cap on method dispatch; 2026-07-09 its bare-String error became a typed `StackError`) |
| `each_reenter.qn` | an `each:` body re-iterating its receiver compounds native frames through the `valueWithSelfOrArg:`/`execute_block` seam — uncatchable SIGBUS | **FIXED** (2026-07-09: `execute_block` stack-watermark — each coroutine's `Stack::limit()`, refused within 2 MiB of the 16 MiB → catchable `StackError`) |
| `catch_reenter.qn` | a self-nesting `catch:` protected block compounds native frames the same way — uncatchable SIGBUS | **FIXED** (same watermark) |
| `serialize_cycle.qn` | a CYCLIC (or ~500k-deep) value SIGBUSes every serializer: `JSON.generate:`, `MessagePack.pack:`, TOML/YAML, extension `call:…data:` — widens the encode-side audit finding to a no-extension, two-line user crash | **FIXED** (2026-07-09: `MAX_SERIALIZE_DEPTH = 128` threaded through `value_to_data` + `value_to_json` → catchable `ValueError`; proto `write_dv` stays infallible, guarded by the producer cap) |

Verified-safe contrasts from the same dig (no repro files needed): a plain
`b = { b.value }; b.value` self-call runs as FLAT interpreted frames (an ordinary
infinite loop, not a crash), and a sort comparator that re-sorts its own receiver
is already caught by the >12 native-reentry cap.
