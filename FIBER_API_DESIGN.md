# BuildingBlocks VM: Guest-Level Fibers (Continuations with Yield)

This document is the design reference and implementation record for exposing the
VM's stackful fibers to the BuildingBlocks (BB) language as a first-class
`Fiber` type. It builds directly on the runtime described in
[`FIBER_REDESIGN.md`](FIBER_REDESIGN.md), which migrated the interpreter onto
`corosensei` stackful coroutines so the GC could run during long native calls.
That work gave the *host* a single fiber; this feature gives the *guest* many.

Status: **Phases 1–3 implemented and shipped** (core fibers, the `^>` operator,
the iteration redesign + Generator/Iterator bridge, and the Phase 2
introspection/diagnostics layer). All BB test suites and Rust unit tests pass;
see [Testing](#9-testing).

---

## 1. Executive Summary

The VM already runs guest code inside a `corosensei` coroutine that suspends
cooperatively so the host can collect garbage. We expose that same capability to
BB programs as asymmetric, resumable coroutines — Ruby `Fiber` / Lua coroutine
style:

```
counter = Fiber.new:{ |start|
    n = start;
    { true }.whileDo:{
        Fiber.yield:n;     "* suspend, hand n back to the resumer"
        n = n + 1;
    }
};

counter.resume:10    "* => 10  (start bound to 10)"
counter.resume       "* => 11"
counter.resume       "* => 12"
```

The central design choice is to turn the host driver loop into a **fiber
scheduler**. Every fiber — including the main program (fiber #0) — is a sibling
coroutine resumed directly by the driver. `resume` and `yield` are expressed as
new `YieldReason` variants that bubble up to the scheduler, which performs the
context switch and re-enters the appropriate coroutine. Because every suspension
still goes exactly one hop to the driver (the only place that holds the
`mutate_root` borrow), the existing GC checkpoint mechanism keeps working
untouched and no "bubbling" through nested coroutines is required.

---

## 2. Goals and Non-Goals

### Goals
- A first-class guest `Fiber` type created from a block.
- Asymmetric `resume` / `yield` with two-way value passing.
- `yield` legal from anywhere in guest code, **including from inside native
  methods that call back into a block** (e.g. `List#each`). This is the payoff
  of stackful fibers over a frame-swapping scheme.
- Correct interaction with the tracing GC: a fiber may stay suspended across
  arbitrarily many collections.
- Zero behavioral change for existing programs; all existing tests stay green.

### Non-Goals (explicitly out of scope)
- **Full `call/cc` / re-entrant, multi-shot continuations.** `corosensei` can
  suspend and resume a stack but cannot *copy* one, so a captured continuation
  can be resumed to completion but not re-invoked from a saved point multiple
  times. We provide coroutines, not copyable continuations.
- **Preemption / scheduling fairness.** Fibers are cooperative; control moves
  only on explicit `resume`/`yield`.
- **Cross-thread fibers.** The VM is single-threaded; fibers are too.

---

## 3. Surface API

### Class side
| Selector | Behavior |
|---|---|
| `Fiber.new:aBlock` | Construct an unstarted fiber wrapping `aBlock`. Status `created`. Does not run. |
| `Fiber.yield:value` | Suspend the running fiber; `value` becomes the result of the resumer's `resume`. Returns the value passed to the next `resume:`. |
| `Fiber.yield` | `Fiber.yield:nil`. |
| `Fiber.current` | The fiber currently running, or `nil` in the main program. |

`Fiber.yield:` is **dynamic** — it acts on whichever fiber is currently running,
so the block keeps its natural parameter list. Calling it when no fiber is
running (i.e. from the main program) raises an error.

#### The `^>` yield operator
`^> expr` is sugar for `Fiber.yield:expr`. The compiler lowers it to exactly
that send (`LoadGlobal(Fiber)`, evaluate `expr`, `Send "yield:"`), so it has
identical behavior — including the "yield outside a Fiber" error and the
two-way value pass. It is usable in **expression position** (it lives in the
grammar's `primary`), so its resume value can be captured anywhere, e.g.
`a = ^> v`. Its operand binds greedily like `Fiber.yield:` (`^> a + b` yields
`a + b`); parenthesize to scope it, e.g. `(^> a) + b`.

```
f = Fiber.new:{ |start|
    n = start;
    { true }.whileDo:{ ^> n; n = n + 1 }
};
f.resume:10   "=> 10"   f.resume   "=> 11"
```

### Instance side
| Selector | Behavior |
|---|---|
| `f.resume` / `f.resume:value` | Start or continue `f`. On the **first** resume, `value` is bound to the block's parameter(s). On **subsequent** resumes, `value` becomes the result of the in-fiber `Fiber.yield:` expression. Returns the next yielded value, or the block's final return value on completion. |
| `f.done?` | `true` once the block has returned normally. |
| `f.failed?` | `true` once the block has raised an uncaught error. |
| `f.alive?` | `true` until the fiber terminates (done or failed). |
| `f.result` | The block's final return value once `done` (else `nil`). |
| `f.error` | The error value once `failed` (else `nil`). |
| `f.status` | `'created'` \| `'suspended'` \| `'running'` \| `'done'` \| `'failed'`. |

### Semantics and edge cases
- **First resume binds parameters.** `Fiber.new:{ |x| ... }` then `f.resume:42`
  binds `x = 42`. `f.resume` with no argument binds `nil`.
- **Completion.** When the block returns, that value comes out of the final
  `resume` and the fiber transitions to `done`.
- **Misuse raises a typed `FiberError`** (a subclass of `Error`, catchable by
  type): resuming a finished/failed fiber, a fiber resuming itself, resuming a
  fiber currently resuming this one, or `yield` outside a fiber. Each carries a
  distinct message.
- **An uncaught error in the block** marks the fiber `failed`, re-raises to the
  resumer's `resume` call, and is retained in `f.error`.
- **Re-entrant resume.** Resuming a fiber that is currently running, or that is
  an ancestor on the resume chain, raises `"cannot resume a Fiber that is
  already running"` (prevents cycles).
- **Errors propagate to the resumer.** An uncaught throw inside the block
  surfaces as a throw out of the resumer's `resume` call; the fiber becomes
  `done`. These errors are ordinary catchable BB exceptions.
- **Nesting.** Fibers may resume other fibers to arbitrary depth; `yield` always
  returns control to the *immediate* resumer.

---

## 4. Execution Model: Driver-as-Scheduler

### 4.1 Why not nest coroutines, and why not frame-swap

Two rejected alternatives:

1. **Nested native coroutines** (the resumer's `resume` drives the child
   coroutine inline). A child's `CooperativeYield` would then suspend only to its
   parent, not to the host driver that owns the GC borrow, forcing every level to
   re-bubble GC checkpoints upward. Complex, and it scatters GC-rooting concerns
   across the native stack.

2. **Pure frame-swapping** (swap `VmState.frames`/`stack`, no per-fiber
   coroutine). This cannot capture a continuation that is partly on the *native*
   Rust stack — e.g. a `yield` issued from inside `List#each`, which reaches the
   block via `execute_block`'s nested step-loop. Capturing that requires a real
   stackful coroutine.

### 4.2 The chosen model

Make the driver a scheduler over **sibling** coroutines:

- Every fiber, including main (#0), is its own `corosensei` coroutine running the
  shared step-loop (`run_vm_loop`). All of them are resumed *directly* by the
  driver, so every suspension is one hop to the driver.
- `resume` and `yield` are native `Fiber` methods that, deep inside `step`,
  suspend the current coroutine with a new `YieldReason`:
  - `YieldReason::ResumeFiber { fiber, arg }`
  - `YieldReason::YieldFiber { value }`
- The scheduler (in the driver loop) interprets these, swaps execution contexts,
  and resumes the target coroutine on the next iteration.
- These new suspends are **transparent to the existing nested step-loops**
  (`execute_block`, `call_method`, …). Just like `CooperativeYield`, a
  `yielder.suspend(...)` simply returns later; the native code that issued it
  resumes in place.

### 4.3 Control-flow walkthrough

`main` resumes fiber `A`, which yields a value back:

1. `main`'s step-loop executes `A.resume:x`. The native `fiber_resume` validates
   `A`, then `yielder.suspend(ResumeFiber{A, x})` — suspending `main`'s coroutine
   to the driver.
2. Driver: saves `main`'s context, pushes `main` on the resume stack, sets
   `current_fiber = A`, loads `A`'s context (first time: pushes `A`'s initial
   frame, binding `x` to the block params), and on the next iteration resumes
   `A`'s coroutine.
3. `A` runs and executes `Fiber.yield:v`. `fiber_yield` does
   `yielder.suspend(YieldFiber{v})`.
4. Driver: saves `A`'s context, pops the resume stack (`main`), sets
   `current_fiber = None`, loads `main`'s context, places `v` in the transfer
   slot, and resumes `main`.
5. `main`'s `fiber_resume` returns from its `suspend`, reads `v` from the
   transfer slot, and returns it as the result of `A.resume:x`.

Completion is detected without a dedicated variant: when a fiber's block
finishes, its step-loop returns `VmStatus::Finished`, the coroutine *returns*
`Ok(val)` (`CoroutineResult::Return`), and the driver — seeing
`current_fiber.is_some()` — treats it as "fiber done", delivering `val` to the
resumer and marking the fiber `done`.

---

## 5. Data Model

### 5.1 `YieldReason` (in `src/fiber.rs`)
Extended with two variants carrying GC values (traced by the existing `Collect`
derive):

```rust
pub enum YieldReason<'gc> {
    CallBlock { block: Gc<'gc, Block<'gc>>, args: Vec<Value<'gc>> },
    CooperativeYield,
    Return(Value<'gc>),
    ResumeFiber { fiber: Value<'gc>, arg: Value<'gc> }, // NEW
    YieldFiber  { value: Value<'gc> },                  // NEW
}
```

### 5.2 Shared step-loop (in `src/fiber.rs`)
`run_vm_loop(yielder, ctx)` is the standard driver body — step the VM over the
*current* context, suspend `CooperativeYield` between steps, return on
finish/error. It is now used by both the main program and every guest fiber
(previously this body was inlined in the driver). Fiber resume/yield happen
deeper in `step`, so they are invisible to this loop.

### 5.3 `NativeFiberState` (in `src/runtime/fiber.rs`)
A guest `Fiber` is a `NativeState` object whose backing state follows the same
transmute-to-`'static` + hand-written `trace_gc` idiom as `NativeListState` and
`NativeMethodState`:

```rust
pub enum FiberStatus { Created, Suspended, Running, Done }

pub struct NativeFiberState {
    coro: Gc<'static, Fiber<'static>>,   // the coroutine wrapper (NEEDS_TRACE = false)
    block: Value<'static>,               // the block to run
    pub status: FiberStatus,
    pub started: bool,
    stack: Vec<Value<'static>>,          // saved guest operand stack
    frames: Vec<Frame<'static>>,         // saved guest call frames
    native_args: Vec<Vec<Value<'static>>>, // saved active_native_args
}
```

The coroutine itself is held in the existing `Gc<'gc, Fiber<'gc>>` wrapper
(`NEEDS_TRACE = false`), which is how a non-`Collect` `corosensei` coroutine is
legally carried in the GC graph. `trace_gc` dyn-traces the coroutine handle, the
block, and every value in the saved context so the collector keeps them alive
while the fiber is suspended.

### 5.4 `VmState` scheduler fields (in `src/vm.rs`)
```rust
current_fiber: Option<Value<'gc>>,        // running guest fiber, or None = main
resume_stack: Vec<Option<Value<'gc>>>,    // resumer chain (None == main)
fiber_transfer: Option<Value<'gc>>,       // one-slot value mailbox across a switch
main_saved_stack / main_saved_frames / main_saved_native_args, // main's saved context
fiber_error: Option<BBError>,             // (require_static) error delivered to a resumer
```

All `Value`-bearing fields are GC-traced; `fiber_error` is `require_static`
(`BBError` contains no `Gc`).

---

## 6. Key Operations

### Native `Fiber` methods (`src/runtime/fiber.rs`)
`new:`, `yield:`, `yield`, `resume`, `resume:`, `done?`, `alive?`, `status`.
`new:` constructs the coroutine eagerly (`Fiber::new(run_vm_loop)`) and wraps it
plus the block in a `NativeFiberState`.

### VM helpers (`src/vm.rs`)
- `fiber_resume(mc, fiber, arg)` / `fiber_yield(mc, value)` — run *inside* the
  native methods; they save/restore `vm.yielder`, `suspend` the appropriate
  `YieldReason`, and on return read the transfer slot (or `fiber_error`). Both
  carry the audited `#[allow(no_gc_across_yield)]`.
- `do_resume_switch` / `do_yield_switch` / `do_fiber_done` — run in the **driver**
  (never across a suspend); they perform the context swap, update statuses and the
  resume stack, and set the transfer/error slot.
- `save_fiber_context` / `load_fiber_context` — move `stack`/`frames`/
  `active_native_args` between `VmState` and a fiber's slot (main uses the
  `main_saved_*` fields; guests use `NativeFiberState::{take,set}_context`).

### Driver (`src/runner.rs`)
The `compile_and_run_asts` loop now selects the current coroutine (the guest
`current_fiber`'s coroutine, else the main `active_fiber`), resumes it, and routes
`ResumeFiber`/`YieldFiber`/guest-`Return` through the switch helpers. Benchmark
mode does not support guest fibers and `panic!`s if one is used there.

---

## 7. GC Safety

The invariant from `FIBER_REDESIGN.md` still holds: **no `Gc` value may be live
only on a suspended fiber's native stack across a `yielder.suspend()`.** This
design upholds it:

- A suspended fiber's full guest context (`stack`/`frames`/`native_args`) lives
  in its `NativeFiberState`, which is GC-reachable (the fiber object is held by
  guest variables) and is traced by `trace_gc`. Any `Gc` copies sitting on the
  fiber's suspended native stack are therefore also reachable through the traced
  context — the same discipline native methods already follow.
- The cross-switch transfer value is stored in the traced `VmState.fiber_transfer`
  slot, never held only as a Rust local across the suspend.
- The `no_gc_across_yield` lint described in [`LINTER_DESIGN.md`](LINTER_DESIGN.md)
  governs the new suspend sites; `fiber_resume`/`fiber_yield` get the same
  audited `#[allow]` as `call_method`.

### The `vm.yielder` correctness subtlety
`vm.yielder` must always point to the *currently running* coroutine's yielder.
After a fiber switch and a switch back, a coroutine resumes deep inside its
native `fiber_resume`/`fiber_yield` — not at the closure top that normally sets
`vm.yielder`. The helpers therefore **save `vm.yielder` to a local before
`suspend` and restore it after**. Since the local lives on that coroutine's own
native stack, it survives the suspension, guaranteeing `vm.yielder` is correct
whenever any coroutine runs (including for nested `CooperativeYield`s in
`execute_block`).

### The `vm`/`mc` pointer subtlety
As in the existing nested step-loops, native code continues to use its `&mut
VmState`/`&Mutation` across a `suspend`. This relies on the arena root (and the
mutation token) being stable across `mutate_root` calls — the same assumption the
pre-existing `execute_block`/`call_method` cooperative yields already make.

---

## 8. Worked Examples

**Two-way communication** — `resume:` feeds a value *into* the fiber as the
result of `yield`:
```
f = Fiber.new:{ |x|
    a = Fiber.yield:(x + 1);
    b = Fiber.yield:(a + 1);
    a + b
};
f.resume:10     "* x=10  -> yields 11"
f.resume:100    "* a=100 -> yields 101"
f.resume:1000   "* b=1000 -> returns 1100"
```

**Yield from inside a native iterator** — the stackful payoff:
```
f = Fiber.new:{
    #(10 20 30).each:{ |x| Fiber.yield:x };
    'done'
};
f.resume  "=> 10"   f.resume  "=> 20"   f.resume  "=> 30"   f.resume  "=> 'done'"
```

**Nested fibers** — `yield` returns to the immediate resumer:
```
inner = Fiber.new:{ Fiber.yield:'a'; Fiber.yield:'b'; 'inner-done' };
outer = Fiber.new:{
    Fiber.yield:(inner.resume);
    Fiber.yield:(inner.resume);
    inner.resume
};
outer.resume  "=> 'a'"   outer.resume  "=> 'b'"   outer.resume  "=> 'inner-done'"
```

---

## 9. Testing

`bblib/tests/13-fibers.b` — 8 tests / 25 assertions:
- `yieldsValuesInOrder` — first-resume parameter binding + ordered yields.
- `returnsFinalValueThenIsDone` — final return value + `done?` transition.
- `twoWayCommunication` — values passed both directions.
- `statusTransitions` — `created → suspended → done`, `alive?`.
- `nestedFibers` — a fiber driving another; yield returns to the immediate resumer.
- `yieldsFromInsideEach` — yielding across native `execute_block` frames.
- `resumingFinishedFiberThrows`, `yieldOutsideFiberThrows` — catchable error paths.

Additional validation performed during development:
- **GC stress**: a fiber kept suspended across ~20k allocations returned the
  correct result, exercising `trace_gc` on suspended contexts.
- **No regressions**: all BB suites pass (0 failures); all Rust unit tests pass;
  warning-free build.

---

## 10. Phasing & Future Work

- **Phase 1 (done):** `new:`, `resume`/`resume:`, `yield`/`yield:`, `done?`/
  `alive?`/`status`; scheduler refactor; GC rooting; tests.
- **`^>` yield operator (done):** `^> expr` is sugar for `Fiber.yield:expr`,
  lowered to that send by the compiler (see [§3](#3-surface-api)).
- **Phase 3 — iteration redesign + Generator/Iterator bridge (done):** The
  `Iterate` protocol was simplified to a single required method, `each:`
  (dropping the `next`/`reset` mutable cursor), which made iteration re-entrant
  and `nil`-safe. The fiber bridges iteration in both directions, in
  `bblib/02-iterate.b`:
  - **`Generator`** — a `^>`-yielding block as an iterable (`Generator.from:`);
    its `each:` runs the block in a fiber and forwards each yield to the
    consumer. Consumed lazily through an `Iterator` (e.g. `.take:`), so even
    infinite generators work.
  - **`Iterator`** — external pull iteration (`hasNext?` / `next`), produced by
    `someIterable.iterator`, backed by a fiber running `each:` with one element
    of look-ahead.

  Plus lazy combinators `lazyCollect:` / `lazySelect:` that return `Generator`s,
  so pipelines stay lazy and compose over infinite sources.

  Covered by `bblib/tests/14-generators.b` (custom `each:`-only collection,
  `nil` elements, re-entrancy, `Generator`, infinite generator + `take:`,
  external `Iterator`, `zip:`/`drop:`, lazy combinators).
- **Phase 2 (done):** `Fiber.current`; richer status/result surface
  (`failed?`/`result`/`error`, `status` gains `'failed'`); typed `FiberError`
  with distinct double-resume / yield-outside diagnostics. Tests in
  `bblib/tests/13-fibers.b`.

### Known limitations
- Guest fibers are unavailable in benchmark mode (that driver bypasses the
  scheduler and `panic!`s on fiber use).
- No `call/cc`-style copyable continuations (see [§2](#2-goals-and-non-goals)).
- A fiber that runs a long stretch *between* yields defers GC for that stretch,
  the same way a long native method does; cooperative GC resumes at the next
  step boundary / fiber switch.
