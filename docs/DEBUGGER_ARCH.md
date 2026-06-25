# Debugger Architecture — pausing, stepping, and inspecting the Quoin VM

Status: **Design capture, grounded in a code audit (June 2026).** No debugger code exists
yet; v0 (a CLI debugger, no wire protocol) is scoped at the end and about to begin. Companion
to `ASYNC_ARCH.md` (the scheduler this rides on) and `INTROSPECTION.md` (the read-only metadata
surface this extends). Like `FUTURE_EXT_ARCH.md`, treat the unbuilt parts as decisions to
revisit with a fresh explain-then-pause.

## The core insight

**A breakpoint pause is just another `YieldReason`, and the VM's existing driver ↔ coroutine
split *is* the debugger's controller ↔ target split.** The VM is single-threaded and
cooperative: `step_internal()` runs exactly one instruction per call (`src/vm.rs`), the
top-level `run_vm_loop` suspends with `YieldReason::CooperativeYield` after every instruction
(`src/fiber.rs`), and the scheduler/driver drives the coroutine and services I/O, gather, and
join *between* resumes. A debugger slots into that seam with almost no new concept:

- Pausing = a new `YieldReason::DebugBreak` that bubbles to the driver, exactly as `AwaitIo`/
  `Gather`/`Join` already do.
- The **driver** becomes the controller: on `DebugBreak` it talks to the frontend (a CLI
  prompt, later a DAP client), services inspect/evaluate/set-breakpoint requests via
  `mutate_root` — full, clean arena access with **no borrow held across the pause** — and
  resumes the coroutine on continue/step.
- Inspection and evaluation run *between* resumes, which is precisely where arena access is
  clean. Single-stepping is free: the program already yields after every instruction.

This is why the mechanism is low-risk: the hard parts (suspend/resume, controller separation,
per-instruction granularity) are machinery the async work already built and hardened.

## Three layers (keep them separate)

Most protocol confusion dissolves by separating:

1. **Mechanism** — how the VM pauses, steps, and exposes frames/vars. In-VM, unavoidable, the
   defining work. Covered below.
2. **Protocol** — the schema the frontend speaks (breakpoints, stack trace, evaluate, continue).
   v0 has none (a CLI command loop); v1 is **DAP** (Debug Adapter Protocol).
3. **Transport** — how bytes move (none in v0; stdio or a socket for DAP).

The layers are independent: the mechanism is fixed, the protocol/transport are pluggable. See
*Protocol & transport* for why this matters to the extension-system question.

## Mechanism — exact integration points

All line references are snapshots from the June 2026 audit; treat them as anchors.

### Pause / step hook

`step_internal()` (`src/vm.rs:~2461`) opens with the cancellation checkpoint:

```rust
if self.sched.cancel_current {
    return Err(self.take_cancellation());
}
```

This fires once per instruction, before dispatch, and is the proven template for "inject an
action at the next checkpoint" (`take_cancellation` clears its own flag so the ensuing unwind
isn't re-triggered — `src/vm_scheduler.rs`). The debug hook is the same shape, gated behind an
`Option` so it costs one well-predicted branch when disabled:

```rust
if let Some(dbg) = &self.debug {
    if dbg.should_pause(frame.block_id(), frame.ip) {   // breakpoint hit, or single-step armed
        return self.debug_break(mc);                    // suspend with YieldReason::DebugBreak
    }
}
```

`debug_break` suspends the coroutine with `YieldReason::DebugBreak` (no payload — the driver
reads the paused state directly off `vm`), mirroring `await_io`. The driver's resume match
gets a `DebugBreak` arm that hands control to the frontend command loop and resumes on the
next continue/step.

### PC and bytecode — breakpoints are a side-table

The program counter is `Frame.ip: usize` (`src/vm.rs:~43`); the bytecode is
`Frame.block.bytecode: SharedBytecode(Rc<Vec<Instruction>>)` (`src/instruction.rs`). The
bytecode is **`Rc`-shared and immutable** — no interior mutability — so **breakpoints cannot be
patched into the bytecode (no INT3-style opcode)**. They live in a **side-table** on
`DebugState`, e.g. `HashMap<BlockId, HashSet<usize>>` (block identity + PC offset), checked at
the hook. A `Break`-opcode optimization is possible later via copy-on-write per debugged block,
but the side-table is the v0 design and is plenty fast (the hook is already `Option`-gated).

### Source maps — PC↔line

`SharedSourceMap(Rc<Vec<Option<SourceInfo>>>)` (`src/instruction.rs:~40`) is **dense and
per-instruction**, parallel to the bytecode. `SourceInfo`
(`crates/quoin-syntax/src/source_info.rs:~12`) carries `filename`, `line` (1-indexed), `column`,
and byte `start..end` — a full span.

- **PC → line ("where am I"): ready.** O(1) index with block-level fallback
  (`src/vm.rs:~1737`).
- **line → PC (breakpoint placement): build a reverse index at load.** No reverse map exists;
  scan the dense source map to find PCs on a target line. The peephole-fusion pass keeps the
  *semantically significant* entry (the `Send`/`Store`) and drops helper loads
  (`src/compiler.rs`), so breakpoints land on meaningful instructions.
- **Step granularity: detect line changes.** Statement boundaries aren't explicitly marked, but
  the dense map lets step-over/into watch for `SourceInfo.line` changes between consecutive PCs.

### Block / line registry

Source maps are **per-block**; there is no global registry. To set "file X line 42" before
knowing which block holds line 42, enumerate blocks via the introspection API
(`get_block_from_method`, `src/dispatch.rs:~835`) over `globals()`/`describe_class()`, and walk
nested blocks via `Block.decl_block`. Build this once at load into the same structure as the
line→PC reverse index: `HashMap<(file, line), Vec<(BlockId, pc)>>`.

### Stepping semantics

All computable from `Frame.ip → SourceInfo.line` plus frame depth (`self.frames.len()`):

- **step (instruction)** — resume one instruction (the program yields anyway).
- **step-over / next** — resume until the source line changes *and* depth ≤ current (don't stop
  inside deeper calls).
- **step-into** — resume until the source line changes (stop in deeper frames too).
- **step-out / finish** — resume until depth < current.

Native (Rust) methods are opaque: they have no Quoin frame, so the debugger treats them as
step-over only.

## Inspection — ~95% ready today

A paused frame exposes everything needed (`src/vm.rs:~43`, `src/value.rs:~553`):

- **Locals** — `Frame.env` is an `EnvFrame.vars: Vec<(Symbol, Value)>` chain; iterate it.
- **`self`** — bound in the env under `self_symbol()` (also on `Frame.receiver`).
- **Instance vars (`@x`)** — via `self`'s `Object.fields` indexed by `Class.field_slots`
  (`load_field`, `src/vm.rs:~2275`); enumerate from the class's `instance_vars`.
- **Closure captures** — walk `EnvFrame.parent`.
- **Globals** — `vm.globals`, surfaced by the introspection API.

`src/introspect.rs` already returns **owned `'static` snapshots** (`GlobalInfo`/`ClassInfo`/
`ValueInfo`/`BindingInfo` — no `'gc`), safe to hand to an out-of-process client. The one gap:
it returns value *structure*, not rendered text — so the debugger renders each value in-arena
via `.pp` (the width-aware pretty-printer's `pp_shape` yields the DAP-style expandable children)
or `.s`, before serializing. References to the live `Value`s stay valid because **paused frames
are GC roots**.

## Evaluate-in-frame — fully working (both prerequisites since fixed)

"Evaluate this expression in frame N's scope" now resolves `self`, `@ivars`, *and* locals — so
`$print @total + n` works. It rests on two fixes that the debugger forced, both since landed:

- ✅ **The `Runtime.eval:` parse-panic** — `compile_and_execute_source` uses the fallible
  `try_parse_quoin_string_named` and maps to a catchable `ParseError`, so a malformed expression
  can't crash the VM.
- ✅ **`eval:self:` + `eval:bindings:`** — `eval:self:` now actually binds the receiver as the
  eval'd code's `self` (the top-level `self = nil` init was clobbering it), and `eval:bindings:`
  seeds a name→value map as locals (compiler told they're locals; values bound into a parent
  env the eval'd frame walks into). `debug_eval` passes the focus frame's `self` *and* its locals
  together, closing the gap.

A bare local / `@ivar` still takes a side-effect-free **direct read** fast path; everything else
evaluates with self + the frame's locals seeded.

## Async / scheduler interaction

- **Pause the world (v1).** Stopping at a breakpoint stops the *whole scheduler*, not one task.
  In a single-threaded cooperative VM this is trivial — the driver simply stops advancing any
  task. Per-task / per-fiber debugging is a later luxury.
- **The pause is *above* the scheduler, not an `AwaitIo` park.** A parked I/O op cooperatively
  yields so other tasks run; a breakpoint must *not* let program tasks run. So the paused
  command loop is a driver-level stop (a blocking read on the frontend), distinct from the
  program's `IoRequest` reactor. (An "async-break" — interrupt a running program — *can* poll
  the frontend cooperatively; that's the one place the reactor model applies.)
- **Step-over an `await`** — let the task park, let the I/O complete, re-pause when it resumes.
  The dense source map + per-instruction yield make the "did we reach the next line" test work
  across the suspension.

## Exception breakpoints — break on throw

A flag to drop into the debugger when an exception is thrown: `qn debug --break-on-throw=Type[,Type…]`.

**The type filter is mandatory** — there is deliberately no bare "break on *every* throw" form.
The suite throws constantly (every `does:throw:` assertion), so an unfiltered mode would be
unusable; naming a type ("stop whenever a `TypeError` is thrown") is both well-defined and
useful. (`--break-on-throw=Object` is the explicit, deliberately-typed "everything" escape
hatch.) Matching is hierarchy-aware: `--break-on-throw=Error` catches every built-in structured
error (all are `Error` subclasses) but not a bare `'x'.throw` (a `String`).

**Source-agnostic by design.** The user names a *type*, never a *source* — `TypeError` (raised
in Rust as a `QuoinError` variant) and `MyError` (a user `Error.throw:`) behave identically.
This is the key requirement and it dictates *where* we match, because the two have different
shapes mid-flight:
- a **user throw** sets the thrown value in `active_exception` (a Quoin `Value`) at
  `Object#throw`;
- a **structured error** is an `Err(QuoinError::TypeError{…})` raised at one of hundreds of
  scattered native sites — it only becomes a Quoin `Error` value lazily (`quoinerror_to_value`).

There is no single *throw site* that sees both. But the VM's **lazy frame-popping** (errors
propagate without unwinding; `catch:` pops only when it actually catches — see the audit) gives
two downstream chokepoints that see *both* kinds with **frames still live**:
1. **`catch:`'s `Err` arm**, before its `while frames.len() > initial { pop }` — every *caught*
   error (the throwing stack is intact);
2. **`run_vm_loop`'s uncaught-`Err` arm** (per task) — every *uncaught* error, frames intact
   because nothing popped them on the way up.

At both, the innermost frame's `ip` has already advanced *past* the failing send (`exec_send`
bumps the caller `ip` before dispatching), so the debugger displays the throw frame at `ip - 1` —
the failing instruction — via `frame_display_ip`, matching `annotate_error`'s stack trace. A
transient `DebugState.at_throw` flag (set in `debug_check_throw`, cleared on resume) selects this
post-dispatch line for the throw pause; a normal breakpoint pause is pre-dispatch and uses `ip`.

So the match runs at those two points, against the error's type — the value's class for a user
throw, or the variant's mapped `Error` class for a structured one — and on a hit fires the
existing `DebugBreak` pause (live frames, full inspect/eval). "Continue" resumes normal
handling/propagation. This is effectively **first-chance** (we break at the nearest `catch`
boundary or the top, an instant after the literal throw, but with the throw site still on the
stack), and it is **uniform** across Rust- and Quoin-raised errors — exactly the requirement.

No distinct "resume from a throw" action is needed: the pause sits *on* the in-flight `Err`
(the checkpoint suspends without consuming it), so a plain `$continue` simply returns from the
checkpoint and lets the error keep propagating/handling exactly as it would have. The transient
`at_throw` flag is reset on resume so the next (non-throw) pause renders normally.

**Not "uncaught-only."** This is type-filtered *first-chance*, which sidesteps caught-vs-uncaught
prediction entirely. A true "break only on *uncaught*" mode is genuinely hard and deferred (see
*Deferred / open*): you cannot reliably know at throw time whether a `catch:` will keep an
exception — a typed/declining handler (or a re-raise in the handler body) passes it through, and
the declining `catch:` pops the throw-site frames before re-raising, so by the time you *know*
it is uncaught the frames are gone. Doing it right needs two-phase exception handling.

## Protocol & transport — and the extension-system question

**Verdict: the debugger gets its own protocol (DAP) and its own transport. It does not ride the
polyglot-extension interface.** Grounded in the audit of `FUTURE_EXT_ARCH.md`:

- **The extension system is unbuilt** (pure design, zero code), so there is nothing to reuse
  *from it* today. What *is* built and reusable is one level lower: the **async I/O waist** —
  `IoBackend`/`AsyncStream` (`src/io_backend.rs`), fiber-parking via `AwaitIo`, the handle
  reap-queue + GC rooting, and the cancellation/timeout machinery. Extensions would ride that
  waist; a debugger could share its low-level socket/framing code — but that's a future
  `quoin-ipc` library nicety, not a dependency.
- **Codec: not shared.** The extension protocol commits to FlatBuffers/Cap'n Proto for control
  + Arrow C Data Interface for bulk data — *not* the `DataValue` serde bridge. (DataValue
  remains a fine *internal* value-tree for the debugger; it's just not common ground with
  extensions.)
- **Semantics: different.** Extensions are *Quoin-calls-out, request/response RPC* between
  *trusted* peers (out-of-process there is for polyglot + crash-isolation + parallelism, not
  sandboxing). A debugger is the inverse: an external *controller drives the VM*, event-driven.
  Forcing that through the extension call model fits neither side.

So the debugger is its own thing at the protocol and transport layers; the only legitimately
shared substrate is the already-built async I/O waist (and, if it ever exists, a low-level IPC
framing crate). The debugger must not couple to, or block on, the extension track — if
anything it's the better first customer for extracting shared framing, being smaller and
self-contained. DAP itself commonly runs over stdio, sidestepping sockets entirely for v1.

## Hard constraints

- **Zero hot-path cost when off.** The hook is `debug: Option<DebugState>` — one predictable
  branch per step in normal runs. Profile if a `Break`-opcode COW is ever needed.
- **No `Gc`/`Value` held across the pause.** The pause is a `YieldReason` suspension; like every
  other await point, only plain data may cross it (the `no_gc_across_yield` lint enforces this).
  Inspected values are re-fetched from the (rooted) frames in the driver, never carried on the
  native stack across the suspend.
- **Bytecode is immutable** — breakpoints are a side-table, not in-place patches (above).
- **Determinism** — a debug build with no breakpoints set and the hook disabled must execute
  identically to a normal run.

## Staged plan

**Prerequisites (independent, also wanted by the REPL):**
- ✅ **Fix the `Runtime.eval:` parse-panic** — `compile_and_execute_source` now uses the fallible
  `try_parse_quoin_string_named` and maps to a catchable `ParseError` (Slice 0). Unblocks
  "evaluate" / watch expressions.
- ✅ **`eval:self:` fix + `eval:bindings:`** — `eval:self:` now binds the receiver as the eval'd
  code's `self`, and `eval:bindings:` seeds a name→value map as locals. `debug_eval` passes the
  frame's self + locals, so `$print` over arbitrary frame state works.

**v0 — CLI debugger, no wire protocol (the mechanism proof).** No socket, no DAP, no codec.
- ✅ **Slice 1 — pause/step core.** `DebugState` (`Option`-gated on `VmState`) + the
  `step_internal` hook + `YieldReason::DebugBreak` + the driver's `DebugBreak` handler (a stub in
  Slice 1/2; the real command loop is Slice 3). The pause is a direct `yielder.suspend` deep in
  the step loop, so it works at any call depth.
- ✅ **Slice 2 — stepping.** `DebugAction` (continue / step into·over·out) + `apply_debug_action`,
  driven by a `DebugState.script` queue in v0. Stops fire on a *line start* (`is_line_start`, a
  static bytecode property), so a breakpoint fires once per arrival (each loop iteration) but not
  on a mid-line call-return.
- **Slice 3 — the `qn debug <file.qn>` CLI.** A `VmRunnerMode::Debug` + the interactive
  **`$`-command loop**, expression-first (bare expr → eval-in-frame). Sub-sliced:
  - ✅ **3a — wiring + control.** `RunStep::DebugPaused` bubbles the pause to the driver loop;
    `DebugFrontend` (rustyline + history) reads outside the arena, `exec_command` runs inside it.
    Control + breakpoints: `$continue`/`$c`, `$step`/`$s`, `$next`/`$n`, `$finish`/`$fin`,
    `$break`/`$b`, `$delete`/`$d`, `$quit`/`$q`, `$help`. Stop-at-entry; `~/.quoin_debug_history`.
  - **3b — inspection + source display.** `$frames`/`$bt`, `$up`, `$down`, `$locals`/`$l`,
    `$list` (no aliases for `$up`/`$down`), plus the load-time block/line registry (line→PC
    reverse index + nested-block walk) for breakpoint placement. **Display source as you step:**
    at every pause (breakpoint or step), auto-print the current line with a few lines of context
    and a marker on it — gdb/pdb-style — replacing the bare `→ paused at file:line`. Reuses the
    highlighted-snippet machinery (`get_highlighted_snippet`, already used by stack traces) shared
    with `$list`; a toggle (`$source on|off`, default on) for terse stepping.
  - ✅ **3c — eval-in-frame.** Bare expr / `$print`/`$p`: a bare local / `@ivar` is read directly
    (side-effect-free fast path); any other expression is evaluated with the frame's `self` bound
    and its locals seeded as bindings, so `self`/`@ivars`/locals all resolve (`@total + n`). Needed
    the `eval:self:` fix + `eval:bindings:` (both since landed; see *Evaluate-in-frame*).
- ✅ **Slice 4 — exception breakpoints.** `qn debug --break-on-throw=Type[,…]` (mandatory type).
  `debug_check_throw` matches the propagating error's type — hierarchy-aware, via the class chain
  — at the two live-frame chokepoints (`catch:`/`catch:finally:`'s `Err` arm and `run_vm_loop`'s
  uncaught arm); on a hit, the same `DebugBreak` pause (banner + full inspect/eval). Plain
  `$continue` resumes the error's normal handling/propagation — no distinct resume action is
  needed, since the pause sits *on* the in-flight `Err` and returning from the checkpoint lets it
  keep flowing. Uniform across user throws (`active_exception`) and structured errors
  (`quoinerror_to_value`). (See *Exception breakpoints* above.)
- *Test:* `.qn` fixtures driven by scripted command sequences — breakpoints + stepping (done in
  Slices 1–2), and the exception-break path (`break_on_throw_pauses_at_the_throw_site` /
  `_ignores_a_non_matching_type`, plus the `--break-on-throw` flag parse).

**v1 — DAP adapter.** A Debug Adapter (in the language-server repo or as `qn debug --dap`)
translating DAP ⟷ the v0 control API: `setBreakpoints`, `stackTrace`, `scopes`/`variables`
(over the inspection snapshots + `pp_shape` children), `evaluate` (over `eval:bindings:`),
`continue`/`next`/`stepIn`/`stepOut`, exception breakpoints (the Slice 4 filter). VSCode/
JetBrains/nvim-dap integration rides on the existing language server + VSCode plugin.

## Deferred / open

- **Break on *uncaught* exception (true last-chance)** — Slice 4 is type-filtered *first-chance*,
  which deliberately avoids predicting caught-vs-uncaught. A real "uncaught-only" mode is hard:
  whether a `catch:` keeps an exception is dynamic (a typed/declining handler, or a re-raise in
  the body, passes it through), and a declining `catch:` pops the throw-site frames before
  re-raising — so by the time you *know* it is uncaught, the frames are gone. Doing it right
  needs **two-phase exception handling**: a handler stack searched at throw time (match the
  exception against each `catch:`'s *declared* type filter, without running handler bodies) to
  decide caught-vs-uncaught *before* unwinding — then break with frames intact. That is a
  language-level change, coupled to the item below.
- **Typed `catch:` (and its coupling to the above)** — `catch:{|ex:SomeError| …}` should catch
  selectively (an untyped param defaults to `:Object`, so it stays backward-compatible — no
  migration). Implementable single-phase (check the type on `Err`, re-raise on mismatch). The
  *declared* type filter is exactly what a two-phase search needs, so typed `catch:` is the
  natural precursor to the uncaught mode. **Guard-block catch** (`catch:{|ex {ex.code==123}| …}`)
  is intentionally *not* planned: a guard must run arbitrary code to decide, which a frames-intact
  pre-unwind search can't do cleanly — types stay declarative, guards don't.
- **Per-task / per-fiber debugging** — v1 pauses the world; debugging one task while others run
  is a later model (needs a per-task stop and a "threads" view).
- **Data breakpoints / watchpoints** (break when a variable changes) — needs write interception,
  not just PC checks.
- **Conditional & hit-count breakpoints** — fall out of `eval:bindings:` once it lands.
- **Time-travel / reverse debugging** — out of scope.
- **`Break`-opcode COW** — only if the side-table hook ever shows up in a profile.
