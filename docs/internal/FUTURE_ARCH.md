# Future Architecture — where the VM performance work goes from here

*Status (verified 2026-07-09 at `dbe188d`): **SUPERSEDED**, kept for lineage. Its central bet —
"execute natively (compilation)" — shipped as the AOT tier (`docs/internal/AOT_ARCH.md`) and speculative
AOT (`docs/internal/SPECULATIVE_AOT_ARCH.md`). The live ranked roadmap is `docs/internal/PERF_ROADMAP.md`, which
explicitly synthesizes this document. Do not plan from this file.*

Long-term performance roadmap, at `main` @ `fc362a6` (after the typed-devirt tier and the
cheap-dispatch work merged). Companion to `docs/internal/TYPED_DEVIRT_ARCH.md`. Grounded in the profiling
under `profiling/post-cheap-dispatch/`.

## Lineage — the previous bet shipped

The prior version of this doc (Jun 2026) argued that "types don't make a VM fast — *unboxing* and
*devirtualization* do," and bet on typed locals + `sealed!` + unboxed/devirtualized arithmetic (for
fib/sieve) and unboxed struct nodes (for trees), de-risked by a ceiling-screen first. **That bet was
taken and largely shipped:**

- The **typed-devirt tier** (PR #31) — sealed value types, compile-time type propagation, devirtualized
  `Int`/`List` ops, control-flow inlining, step-batching. See `docs/internal/TYPED_DEVIRT_ARCH.md`.
- The **cheap-dispatch work** (PR #32, slices a1/b1/b2) — fused `Int` superinstructions, a flat inner
  dispatch loop, and an `ip`-register hoist.

Net effect: cross-language parity went from ~9–44× CPython (that doc's starting point) to **~2.4–5.8×
CPython** today. The unboxed-struct-node piece for trees is the one part of that bet still open (see
Tier 0). This document picks up from there.

## Where we are: the interpreter floor

The post-cheap-dispatch profile is decisive about the *shape* of what's left:

| bucket | fib(20) | sieve(10000) | trees(10) |
|---|--:|--:|--:|
| dispatch / interpreter loop | **73.5%** | **83.7%** | 25.1% |
| allocation + GC | 11.8% | 0.7% | 22.2% |

The two call/loop-bound benchmarks are **73–84% interpreter dispatch**. a1/b1/b2 made each bytecode
cheap to *execute*, but you are still fetching → dispatching → executing one operation at a time.
That is the **interpreter floor**. Optimizing the loop further (Tier 0) hits a ceiling, because the
cost being paid is the interpretation itself.

Past the floor, going meaningfully faster requires one of two things:

1. **Execute fewer operations** (a smarter compiler / IR) — Tier 1.
2. **Execute them natively** (compilation) — Tier 2.

Everything below is that map.

## Tier 0 — remaining interpreter crumbs (near-term, bounded)

Small, isolated, worth doing but each capped:

- **SipHash → FxHash** — a default-hasher `HashMap` is probed per node in the object/field path
  (visible in the trees profile as `hash_one` + `sip::Hasher::write`, ~1.4%). Cheap freebie.
- **Tree / struct allocation** — trees is the one allocation-bound benchmark (alloc+GC ~22% + object
  construction). Unboxed struct nodes / a per-type arena is the lever; bounded ~1.2× (earlier estimate).
  This is the unfinished part of the previous bet.

Ruled-out/bounded levers (do not revisit — see `profiling/inline-cache`, `profiling/dispatch-cache`,
and the `cheap-dispatch-progress` memory): per-call-site inline cache (regressed trees), frame/env
pooling (~1.1×), `Frame` memcpy slimming and `Callable::call` skip (marginal).

## Tier 1 — best-in-class interpreter (execute fewer ops)

Real headroom without leaving the interpreter, in rough ROI order:

1. **Method inlining — the big one.** Extend the control-flow inlining already shipped (Slice 2d
   inlines `if:else:`/`whileDo:` into native jumps) to hot *user* methods. For fib the recursive calls
   *are* the floor — inline a small monomorphic callee into its caller and the entire per-call cost
   (frame alloc + dispatch + `exec_send`) evaporates. **Composes with typed devirt**: an inlined typed
   method's body fully devirtualizes to native `Int`/`List` ops. Plausibly 2×+ on call-heavy code.
2. **Register bytecode (Lua-style).** The current *stack* VM pays constant push/pop shuffling; a
   register VM cuts instruction count ~30–50%. A larger compiler change.
3. **Threaded dispatch (computed-goto).** The ~15–20% dispatch-*mechanism* win CPython gets. Rust-hard:
   needs guaranteed tail calls (`become`, unstable) or an fn-pointer jump table. Deliberately skipped so
   far because b1/b2 captured the structural dispatch wins without it.

Together these reach **LuaJIT-interpreter / CPython-3.13 territory** — likely Python-parity-or-better
across the board. Ceiling: still an interpreter.

## Tier 2 — the leap: execute natively

The only way to fundamentally break the floor. Two roads:

- **Traditional JIT** — baseline (copy-and-patch, like CPython 3.13's) → optimizing/tracing JIT with
  *runtime type feedback* (LuaJIT / PyPy / V8). The route dynamic languages are forced onto. Enormous:
  a compiler backend + deopt + guards + the GC interaction.
- **AOT-native compilation of the *typed subset* — Quoin's distinctive road** (below).

### Why Quoin can take a road Python/Ruby cannot

Quoin has a real static type system — sealed types + **compile-time type proof** (that is what
devirtualization is underneath). Python and Ruby *cannot* AOT-compile: fully dynamic, they **need** a
speculative JIT + type feedback to discover types at runtime. Quoin **proves** types at compile time.

So typed methods can be **AOT-compiled to native code** (e.g. via Cranelift), with the interpreter kept
only for genuinely-dynamic code: typed hot path → native, dynamic code → interpreted. And Tier 1 feeds
straight in — a fully-inlined, fully-devirtualized typed method is already **a flat, straight-line
sequence of native-typed ops**, which is exactly the input a native codegen backend wants.

Open questions for the AOT path (to be worked out when it's on the table):
- **GC interaction** — native frames holding `Gc` pointers vs the `gc_arena` yield model; likely forces
  a rooting/stack-map scheme (and possibly revisits the collector — only warranted once execution is
  native and allocation dominates).
- **The typed/dynamic boundary** — calling conventions between native typed code and the interpreter,
  and deopt when a "sealed" assumption is violated at a boundary (the same guard-on-entry/trust-inside
  contract the typed tier already uses).
- **Backend choice** — Cranelift (lean, JIT-and-AOT capable) vs LLVM (heavier, better optimizer).

## The arc

The type system was always the long-term bet:

> **Devirtualization was the down-payment** — it made the interpreter faster and paid rent immediately.
> **AOT native compilation is the jackpot** — proving types at compile time lets Quoin skip the entire
> speculative-JIT machinery that dynamic languages are stuck building.

Trajectory:

```
cheap interpreter (done) → method inlining → register VM → AOT-compile the typed subset to native
```

## Recommendation — highest-leverage next bet

After the Tier-0 crumbs, **method inlining**. It is the rare lever that pays at *both* tiers: it
extends proven work (2d control-flow inlining), it kills fib's actual floor (the recursive-call cost),
and every bit of it de-risks the eventual AOT step by producing exactly the flat typed code a compiler
backend consumes.

## The honest part

Each tier is a real step up in effort (inlining: weeks; register VM: weeks–months; a native backend:
months) and there is a *lot* of runway — CPython-no-JIT is itself ~50× off C, so "beat Python" and
"approach native" are very different destinations. How far to go is a product decision: a *fine*
interpreter (≈ where we are), a *great* interpreter (Tier 1), or a *compiled language* (Tier 2 AOT).
