# Future architecture — toward a dramatically faster VM

Captured from a big-picture discussion. The language is "fast enough for now"; this is the direction
to revisit if/when raw speed becomes a goal. It deliberately preserves Quoin's dynamic feel as the
*default* and makes speed *opt-in*. See `profiling/status.md` for the current perf state.

## Why this, why now
The bounded, low-risk perf wins are **spent**: method-resolution cache, FxHash, mimalloc, borrow-not-
clone in `step_internal`, per-Send allocation cleanups. The inline-cache experiment was *built and
measured and ruled out* (parked on `experiment/inline-cache`) — confirming dispatch-resolution caching
is exhausted. What remains is **structural/architectural**: the interpreter dispatch loop
(`step_internal` ~30% inclusive) and per-call frame setup. Closing the next big chunk needs a different
kind of project. Current standing: ~20-50× slower than Ruby 2.6 / ~9-44× slower than Python 3.9
(interpreter-to-interpreter), worst on call/loop-bound code.

## The core insight: types don't make a VM fast — *unboxing* and *devirtualization* do
Static type annotations are necessary-but-not-sufficient. Adding `var x: Integer` today would change
nothing — the VM would still store it as `Value::Int`. The win comes only from an architecture that
*exploits* the types to do two things:

1. **Unbox.** `Value` is a tagged enum (`Int(i64)`, `Double(f64)`, `Object(Gc<…>)`, …); every op pays
   a tag check + match. If `x: Integer` and `y: Integer` are known, `x + y` compiles to a **raw `i64`
   add** — no enum, no tag, no match. For numeric code (fib/sieve) this kills boxing *and* per-op
   dispatch at once. Plausibly **5-20×** there.
2. **Devirtualize.** If the receiver type is known, `x + y` resolves to `Integer#+:` *at compile time*
   — a direct call, no `lookup_method`, no cache, no `Callable`. That's the entire ~27% dispatch cost
   gone for typed code.

## The blocker is exactly the dynamism we like
Devirtualization needs to know `Integer#+:` **can't change**. But `Integer <-- { … }` can redefine it
at runtime, and eigenclasses let instances diverge. **Runtime class extension is precisely what
defeats static dispatch.** Two ways out:

- **Speculate + deopt** (V8 / LuaJIT / PyPy). Stay fully dynamic; assume the common method, guard on a
  per-class version counter, de-optimize if extended. *No language change* — but it's a JIT, a bigger
  project than the entire current VM.
- **Seal — opt in to giving up dynamism locally.** `sealed!` (already a stubbed marker in QUOIN_TODO)
  → the class's method table is frozen → calls devirtualize and arithmetic specializes. Default stays
  dynamic; speed is opt-in where you don't need the dynamism. **This is the sweet spot** — "gradual
  performance."

## The feature set to bet on (dynamism-preserving, opt-in)
1. **Typed locals + typed ivars.** `var x: Integer = …` and types on instance vars. Enables unboxed
   storage and is the input to specialization. (Method args already carry types — this completes the
   picture.)
2. **`sealed!` with teeth.** The escape hatch from runtime mutability that makes devirtualization
   sound — the single most pivotal feature, because it's the thing currently blocking static dispatch.
   (`abstract!`/`final` compose with it.)
3. **Unboxed value/struct types.** A user-defined "struct": immutable, no eigenclass, flat typed
   fields, **not GC-allocated**. The Binary-Trees lever: today every `TreeNode` is a
   `Gc<RefLock<Object>>`; an unboxed struct node could live inline and skip the allocator + GC.

What each targets: typed+sealed numerics → fib/sieve (unboxed arithmetic); unboxed structs → trees
(no per-node allocation). I.e. exactly the slow benchmarks.

## Foundations already in place
- **Slot-based instance vars** (`Class.field_slots`) = the hidden-class/shape foundation. Typed slots
  make them unboxed + smaller.
- **Typed method arguments** already exist (dispatch scores by type-distance).
- So two of the hard prerequisites most dynamic languages lack are already built.

## Other doors worth considering
- **`let` vs `var` (immutable bindings).** Cheap; enables registerization / hoisting / no-reload, and
  good for the language regardless of speed.
- **`abstract!` / `final`.** Compose with sealing for devirtualization (both stubbed in QUOIN_TODO).

## Honest cost
Even the seal path (no JIT) is a **rearchitecture, not a session**: a typed/unboxed value
representation, a typed-bytecode or specializing compiler tier that emits unboxed ops for typed+sealed
regions, and *sound type-checks at the typed/untyped boundary* (guard on entry, trust inside — like
Typed Racket's contracts). Untyped/unsealed code keeps running on today's interpreter. The
no-language-change alternative (a speculative JIT) is even bigger.

## How to de-risk before committing: prototype-and-measure the ceiling
Same discipline that cleanly ruled out the inline cache — measure before building the real thing.

**Tier 1 — standalone ceiling screen (~half-day, ~4-6h focused).** A minimal *standalone* unboxed
interpreter that runs `fib`: tiny instruction set (`PushInt`, `LoadIntLocal`, `IAdd`/`ISub`/`ILt`,
conditional jump, recursive call), fib hand-assembled, a dispatch loop over raw `i64` with no `Value`
enum and no method lookup. Time it like `qn benchmark`, compare to the current ~20ms.
- Answers the go/no-go: a 2× ceiling kills it, a 15× ceiling justifies the rearchitecture.
- **Caveat:** it *overstates* the ceiling (also drops the real VM's frame/call overhead a true tier
  would keep) → treat the result as an **upper bound**. If even the upper bound is unimpressive, stop.
- **Main risk / where it overruns:** the recursive unboxed call (needs a call stack of return-point +
  unboxed locals). Timebox it; report the number or the blocker rather than grinding.

**Tier 2 — integrated slice (~1.5-3 days, hold loosely).** Unboxed `Integer` local slots +
devirtualized arithmetic in the *actual* VM (keeping real frames), through the fib path — the honest
target-architecture number. Touches the value representation, new bytecode, the compiler, and the
typed/untyped boundary. Only worth doing if Tier 1's ceiling is compelling. Unfamiliar-territory
estimate → optimistic; expect surprises.

## Decision when we return
Run **Tier 1** first (cheap upper-bound number). If compelling → **Tier 2** for the realistic number →
then decide on the full specializing tier. If not → the language is fast enough; the dynamic
interpreter stays, and perf work yields to features / code-health.
