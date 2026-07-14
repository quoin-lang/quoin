# Part V — Concurrency & iteration

Fibers (stackful coroutines), generators built on them, and the `Iterate` mixin
that turns a single `each:` into a full collection API — then the concurrency
system proper: detached `Task`s on one cooperative scheduler, the structured
`Async` helpers, CSP `Channel`s, and true parallelism with `Worker` isolates.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · **Concurrency & iteration** · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · [Packages](10-packages.md) · [Appendices](11-appendices.md)

---

## 16. Fibers & generators

> **Rules**
> - `Fiber.new:{ … }` creates an **unstarted** fiber. `f.resume` / `f.resume:arg` runs it to the next yield or to completion.
> - `Fiber.yield:v` (or the sugar `^> v`) suspends the running fiber, handing `v` back to whoever resumed it. The value passed to the next `resume:` becomes the *result* of that yield expression — communication is two-way.
> - Query: `done?`, `alive?`, `failed?`, `status` (`'created'`/`'suspended'`/`'running'`/`'done'`/`'failed'`), `result` (final return value), `error`. `Fiber.current` is the running fiber (or `nil` at top level).
> - Fibers are **stackful** (you can yield from deep inside a native method such as `each:`) and **nestable/re-entrant**.
> - `Generator.from:{ ^> … }` wraps a yielding block as a **lazy, re-runnable** iterable (it mixes in `Iterate`). `collection.iterator` returns an external pull iterator with `hasNext?` / `next`.

```quoin
var f = Fiber.new:{ Fiber.yield:1; Fiber.yield:2; 'done' }
f.resume        "* -> 1
f.resume        "* -> 2
f.resume        "* -> 'done'
f.done?         "* -> true
var evens = Generator.from:{ var n = 0; { true }.whileDo:{ ^> n; n = n + 2 } }
evens.take:4    "* -> #(0 2 4 6)
```

The first `resume` runs to the first yield; the third returns the block's final
value. `evens` is an infinite source, consumed lazily by `take:`.

> **⚠ Gotcha — resuming a finished or failed fiber throws.** Once `status` is
> `'done'` or `'failed'`, `resume` raises a `FiberError`. So does `Fiber.yield:`
> called outside any fiber, and a fiber attempting to resume itself. Check
> `alive?`/`done?` before resuming in a loop.

---

## 17. The iteration protocol

> **Rules**
> - The `Iterate` mixin provides the entire collection API on top of **one required primitive: `each:`**. Implement `each:` and mix in `Iterate`, and every combinator below works.
> - Custom iterable recipe:
>   ```quoin norun
>   MyThing <- { …
>       .mix:Iterate
>       each: -> { |b| … b.valueWithSelfOrArg:element … }   "* call b once per element
>   }
>   ```
> - Iteration is **re-entrant** (no stored cursor — nested/concurrent passes don't interfere) and **nil-safe** (`nil` is a valid element, never a terminator).
> - `lazyCollect:` / `lazySelect:` return **Generators** (lazy), so they compose and work over infinite sources when capped with `take:`.

### Combinators provided by `Iterate`

Transform: `collect:`, `select:`, `reject:`, `flatten`, `groupBy:`, `partition:`,
`uniq`, `zip:`, `reverse`, `sort` / `sort:`.
Reduce/query: `reduce:`, `reduce:into:`, `detect:`, `all?:`, `any?` / `any?:`,
`none?:`, `count` / `count:`, `contains?:`, `sum` / `sum:`, `min` / `max` /
`min:` / `max:`.
Access: `first`…`fifth`, `last`, `nth:`, `take:`, `drop:`, `list`, `join:`,
`iterator`.
Lazy: `lazyCollect:`, `lazySelect:`.

```quoin
MyRange <- { |@start @end|
    .mix:Iterate
    each: -> { |b| var i = @start; { i < @end }.whileDo:{ b.valueWithSelfOrArg:i; i = i + 1 } }
}

var r = MyRange.new:{ start = 0; end = 5 }
r.collect:{ |n| n * n }        "* -> #(0 1 4 9 16)
r.select:{ |n| n > 2 }         "* -> #(3 4)
```

Every combinator here — `collect:`, `select:`, and the rest — comes from that one
`each:`.

> **⚠ Gotcha — combinators return materialized lists; use the `lazy*` forms for
> infinite or expensive sources.** `collect:`/`select:` walk the whole collection
> eagerly. Over an infinite `Generator`, use `lazyCollect:`/`lazySelect:` and finish
> with `take:`; otherwise iteration never terminates.

---

## 18. Tasks & the cooperative scheduler

A fiber (§16) is a coroutine *you* drive with `resume`. A **task** is scheduled
for you: the VM interleaves every spawned task (the top level of the program is
itself a task) on one cooperative scheduler, overlapping their waits.

> **Rules**
> - `Task.spawn:{ … }` starts a **detached task** from a zero-parameter block and answers a **handle** immediately — the spawner keeps running. On the handle: `join` (park until finished; answers the task's result), `cancel`, `status` (`'running'`/`'done'`/`'failed'`/`'cancelled'`), `done?`. `Task.running` is a snapshot List of the handles of all still-running tasks.
> - **One thread, cooperative round-robin.** A task runs until it **parks** — an I/O wait, `Async.sleep:`, a channel operation, a `join` — finishes, or reaches a **scheduler boundary** (every few hundred instructions), where a runnable sibling gets a turn. One thread means no data races: a single method send always completes before any other task runs, so plain data needs no locks — but a *sequence* of statements can interleave with other tasks at any parking point or scheduler boundary.
> - **Parking parks the task, not the VM.** A blocking read (`[IO]Stdin.readLine`, a socket receive) suspends only the task that issued it; every other task keeps running, and the process truly sleeps only when *all* tasks are parked.
> - `join` re-raises a failed task's error **catchably** at the join site (and raises if the task was cancelled). `cancel` is **cooperative**: the task raises an *uncatchable* cancellation at its next checkpoint — within a few hundred instructions, parked or not — `finally:` blocks run on the way out, but no `catch:` handler fires.
> - **The process exits when the main program finishes.** Detached tasks still running — even ones that never got a turn — are abandoned, not awaited: `join` what must complete. `Runtime.exit:code` is likewise uncatchable: the raising task unwinds through its `finally:` blocks, every other task stops, and the process exits (with that status) after normal teardown.

```quoin
var t = Task.spawn:{ 21 * 2 };
t.join    "* -> 42
```

A failed task holds its error until someone joins it:

```quoin
var t = Task.spawn:{ ValueError.throw:'boom' };
{ t.join }.catch:{ |e:ValueError| 'caught: ' + e.message }    "* -> caught: boom
```

Cancellation is a request, honored at the task's next parking point. Its
`finally:` blocks run; its `catch:` handlers do not see the cancellation:

```quoin
var log = #();
var t = Task.spawn:{ { Async.sleep:50 }.catch:{ |e| log.add:'caught' } finally:{ log.add:'cleanup' } };
Async.sleep:5;        "* let the task start and park inside its sleep
t.cancel;
Async.sleep:100;      "* give the cancellation time to land
#( t.status log )     "* -> #(cancelled #(cleanup))
```

### Parking parks the task, not the VM

This is the load-bearing semantic of the whole system. A read that would
"block" — stdin, a socket, a sleep — hands control back to the scheduler, which
runs whatever else is ready. Here the main task waits on stdin while a spawned
ticker keeps ticking (illustrative — it needs a terminal):

```quoin norun
"* tick.qn — the ticker runs while the main task waits on stdin
var ticker = Task.spawn:{
    (0..3).each:{ |i| ('tick ' + i.s).print; Async.sleep:200 }
};
var line = [IO]Stdin.readLine;    "* parks THIS task only
ticker.join;
('read: ' + line).print
```

```
$ qn tick.qn        (typing "hello" about half a second in)
tick 0
tick 1
tick 2
read: hello
```

The same holds for every parking point. A deterministic, runnable version —
`join` is the park, and the spawned task runs to completion while the main task
waits:

```quoin
var log = #();
var t = Task.spawn:{ (0..3).each:{ |i| log.add:i } };
t.join;    "* main parks here; the spawned task runs
log        "* -> #(0 1 2)
```

> **⚠ Gotcha — spawning queues; only joining guarantees.** `Task.spawn:` queues the
> task; it gets its first turn at the spawner's next parking point or scheduler
> boundary (near-immediately, even if the spawner is compute-bound — CPU-bound
> tasks round-robin rather than starving each other). But **the process exits when
> the main program finishes**, abandoning whatever is still queued or running —
> `join`, `Async.gather:`, or a channel handshake is what guarantees completion.
> The one thing that still monopolizes the thread is a single long-running *native*
> call: nothing preempts inside one send.

---

## 19. Async — sleep, gather, timeout

Structured helpers over the task scheduler: `Task.spawn:` is fire-and-forget;
these wait for their work.

> **Rules**
> - `Async.sleep:ms` parks the running task for the given milliseconds (a `Duration` is also accepted) without blocking other tasks; answers `nil`.
> - `Async.gather:#( {…} {…} … )` runs a List of zero-parameter blocks as concurrent tasks — their waits overlap — and answers their results as a List **in input order** once all complete. The first error propagates out of the gather.
> - `Async.timeout:ms do:{ … }` answers the block's value if it finishes in time. If the deadline fires first, the block is cancelled (its `finally:` runs, in-flight I/O aborts) and a catchable `TimeoutError` raises.
> - `Async.timeout:ms do:{ … } onCancel:{ … }`: on the deadline, run the handler and answer **its** value instead of throwing (`onCancel:{ nil }` is the non-throwing form). The handler covers only *this* deadline — an outer cancellation still propagates and the handler does not run.
> - `Async.joinAll:tasks` joins a List of **already-started** `Task` handles, answering their values in the list's order; the total wait is ~the slowest task, not the sum. (`gather:` spawns the work itself from blocks; `joinAll:` is for tasks you spawned yourself.)

```quoin
Async.gather:#( { Async.sleep:2; 'a' } { 'b' } )    "* -> #(a b)
```

`'b'` finishes first, but results keep the input order — gather is about
overlapping waits, not racing.

```quoin
Async.timeout:50 do:{ 42 }                                       "* -> 42
Async.timeout:5 do:{ Async.sleep:200; 1 } onCancel:{ 'late' }    "* -> late
{ Async.timeout:5 do:{ Async.sleep:200 } }
    .catch:{ |e:TimeoutError| e.message }    "* -> operation timed out after 5ms
```

```quoin
var ts = #( { Async.sleep:1; 1 } { 2 } ).collect:{ |b| Task.spawn:b };
Async.joinAll:ts    "* -> #(1 2)
```

> **⚠ Gotcha — `gather:` takes blocks, `joinAll:` takes handles.** Passing task
> handles to `gather:` (or blocks to `joinAll:`) is a type error. And because a
> deadline *cancels* the block it wraps, don't put must-complete side effects
> inside `timeout:do:` without a `finally:`.

---

## 20. Channels

CSP-style message passing between tasks: instead of sharing a structure and
coordinating around parking points, hand values from task to task.

> **Rules**
> - `Channel.new` is an **unbuffered rendezvous**: every `send:` waits for its receiver. `Channel.buffered:n` queues up to `n` values before sends park. Query: `count` (values sent but not yet received), `capacity`, `closed?`.
> - `send:value` hands the value to a waiting receiver, else buffers it if there is room, else **parks** until a receiver takes it. `receive` answers the next value (FIFO — buffered values first, then directly from a parked sender), parking until one is available.
> - `close` (idempotent) ends the conversation: parked and future `send:`s raise a `ValueError` (`'send on a closed channel'`); buffered values remain receivable; a drained `receive` answers `nil`; `each:` ends.
> - `ch.each:{ |v| … }` runs the block on each received value until the channel is closed and drained, parking between values — the standard consumer loop.
> - **Channels cross isolate boundaries.** Send a channel to a thread-backed worker (`Worker.send:`, or as a hosted-service argument or return) and the far side gets a **live endpoint** — its `send:`/`receive`/`close`/`each:` relay to the owning isolate with the same semantics: values deep-copy (and must be portable — a non-portable send raises immediately), backpressure crosses (a full buffer parks remote senders), `close` propagates both ways, and several workers can hold endpoints on one channel — fan a jobs channel out to a worker pool and fan the results back in. Endpoint introspection (`count`/`closed?`/`capacity`) stays with the owner, and process-backed workers can't receive channels yet; both refuse with clear errors.
> - **Deadlock is an error, not a hang** — within one isolate. When every task is parked and no I/O is in flight, the program stops (exit 1) with `deadlock: every task is parked with no I/O in flight (e.g. a receive with no sender, or a join cycle); the program cannot make progress`. A wait cycle *through channels across isolates* is the exception: each side looks I/O-live to its own scheduler, so it hangs rather than reports — `VM.ps` shows the parked shape (`relay channel send`/`receive`).

```quoin
var ch = Channel.new;
Task.spawn:{ ch.send:42 };
ch.receive    "* -> 42
```

The main task parks in `receive`; that yields to the spawned sender, whose
`send:` completes the rendezvous. A buffered channel decouples the two sides:

```quoin
var ch = Channel.buffered:2;
ch.send:1; ch.send:2; ch.close;
#( ch.receive ch.receive )    "* -> #(1 2)
```

Producer/consumer, with `close` as the end-of-stream signal:

```quoin
var ch = Channel.buffered:8;
Task.spawn:{ (0..5).each:{ |i| ch.send:(i * i) }; ch.close };
var got = #();
ch.each:{ |v| got.add:v };
got    "* -> #(0 1 4 9 16)
```

> **⚠ Gotcha — the deadlock error usually means "nobody on the other end".**
> A `receive` with no task that will ever send (or an unbuffered/full `send:`
> with no task that will ever receive) can never be woken; once *every* task is
> in that state the scheduler reports it rather than hanging silently:
>
> ```
> $ qn -e 'Channel.new.receive'
> deadlock: every task is parked with no I/O in flight (e.g. a receive with no sender, or a join cycle); the program cannot make progress
> ```
>
> Spawn the other side before parking (as in the examples above), buffer the
> channel, or `close` it when the producer is done.

---

## 21. Workers, Parallel & Plan — true parallelism

Everything so far shares one OS thread and one heap. For real parallelism,
Quoin uses **isolates**: a `Worker` is a fresh VM on its own OS thread (or
child process) connected to its parent by message lanes — no shared state,
parallelism by message passing. This section is an overview; run `qn doc` for
the generated per-class API reference.

> **Rules**
> - `Worker.spawn:'unit.qn'` boots a fresh VM running that unit file on its own OS thread and answers a handle immediately. `Worker.start:{ … }` spawns from a **portable block** instead — its free reads ship as a deep-copied snapshot, and `join` answers the block's value. `Worker.spawn:'unit.qn' backing:'process'` runs a child `qn` process (data-only lanes; blocks cannot cross a process boundary).
> - On the handle: `send:` / `receive` exchange **deep-copied data** — numbers, strings, booleans, `nil`, `Bytes`, Lists, Maps; symbols, instances, and resources refuse loudly. `join` parks until the worker finishes, re-raising its error catchably (a handle joins once — a second `join` raises); `label:` names its row in `VM.ps` / `VM.psTree`; `terminate` kills a process-backed worker (a thread-backed one can't be killed).
> - Inside the worker, class-side `Worker.receive` / `Worker.send:` are the mirror lanes, and `Worker.worker?` tells a unit which side it is running on.
> - **Portable blocks refuse at submit time**: writes to captured bindings, `self` / `@fields`, and `^^` are all rejected — the worker gets a snapshot, so none of those could mean the same thing there.
> - A worker `receive` / `join` parks like any I/O wait, so it composes with `Async.gather:` / `timeout:do:` / cancellation unchanged.
> - The layers above workers:
>   - **`Parallel`** — `list.parallelCollect:` / `list.parallelReduce:` map/fold across a warm pool of worker isolates, preserving order. Inputs shorter than `Parallel.minItems` (2048) run serially with the same result; the reduce block must be **associative** (each worker folds a chunk, then the partials fold).
>   - **`Plan`** — a lazy task graph. Leaves say *where* code runs: `Plan.task:{ … }` (in-VM task; the only leaf that may capture freely), `Plan.thread:{ … }` (isolate, portable block), `Plan.process:'unit.qn'` (child process). Composites say *shape*: `Plan.all:#( … )` (structural gather) and `Plan.any:#( … )` (race — first success wins, the rest are cancelled). Nothing runs until `await`. Failure policy on a gather: `all:…onError:'cancelRest'` (default — the first error cancels the rest and re-raises) or `'collect'` (every slot resolves to `#{'ok': v}` or `#{'err': msg}`). Compose deadlines with `Async.timeout:ms do:{ plan.await }`.
>   - **`WorkerService`** — `WorkerService.host:'unit.qn' class:'Name'` hosts a class in a dedicated isolate and answers a **proxy**: ordinary method sends become calls into the worker, state stays there, and calls serialize — an actor, effectively. Portable returns deep-copy back; a method that returns a **non-portable object hosts it** — the answer is a *sub-proxy* addressing that object, usable like any receiver, including as an argument to further calls on the same service (it travels as a live reference). **Block arguments always cross.** A *portable* block ships to a thread-backed service and runs *inside* the worker on a snapshot of its captures (as with `Worker.start:`) — one boundary crossing however many times the method invokes it. Any other block — one that captures live state, or any block to a process-backed service — crosses as a **handle**: the worker invokes it back in the parent, one round trip per invocation, and write-captures see the parent's live state. Portable blocks freeze their captures at send time on *either* path, so the backing never changes meaning. Code running this way may call back into the service (the nested call rides the open conversation), and a timeout mid-call abandons the conversation cleanly — the service stays usable. **Concurrency is per object, not per worker**: `WorkerService.host:'unit.qn' class:'Pool' lanes:4` lets calls to *different* objects of the service run concurrently (up to the lane count), while calls to any *one* object still serialize in arrival order — its mailbox. Lanes work on either backing (`host:class:backing:lanes:`): a thread-backed service runs one cooperative fiber per lane; a process-backed one opens one conversation socket per lane — same semantics, different transport. Two calls that synchronously wait on each other's objects raise a catchable deadlock error naming the cycle instead of hanging, and `VM.claims` / `VM.claimsReport` show the live lock shapes — holders, queues, and wait chains — before a deadlock ever happens. A hosted method's error raises catchably at the call site with the worker's stack as `ex.remoteStack`. `serviceStop` ends the service (worker-wide — every proxy of it refuses afterwards).

Round trip through a block worker's lanes (the block is the whole worker
program; `receive` parks the parent task, like any wait):

```quoin
var w = Worker.start:{ var n = Worker.receive; Worker.send:(n * 2); 'done' };
w.send:21;
w.receive    "* -> 42
```

The parallel combinators keep `collect:`'s contract — this input is far below
`Parallel.minItems`, so it runs serially, with the same result either way:

```quoin
#(1 2 3).parallelCollect:{ |x| x * 10 }    "* -> #(10 20 30)
```

A `Plan` mixes in-VM tasks and isolates in one awaited shape:

```quoin
(Plan.all:#( (Plan.task:{ 1 + 1 }) (Plan.thread:{ 2 + 2 }) )).await    "* -> #(2 4)
```

```quoin
(Plan.all:#( (Plan.task:{ 1 }) (Plan.task:{ 'boom'.throw }) ) onError:'collect').await
    "* -> #(#{'ok': 1} #{'err': 'boom'})
```

Hosting a class as a service (illustrative — it needs the unit file):

```quoin norun
var index = WorkerService.host:'search/index.qn' class:'SearchIndex';
index.add:doc;                     "* an ordinary send — runs inside the isolate
var hits = index.query:'quoin';
index.serviceStop
```

> **⚠ Gotcha — messages are copies.** A List sent to a worker (or received from
> one) is deep-copied at the boundary: mutating it on one side never affects the
> other. Isolation is the point — design worker protocols around values passed
> through the lanes, not shared structures.

---

Next: **[Part VI — Networking & the web](06-networking-and-web.md)**.
