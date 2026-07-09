# Quoin bug-hunt findings

Worktree: `bughunt-wt` @ main `1535e7d`. Binary: `target/release/qn`.
Differential axis: `QN_AOT=0` (interp) vs default / `QN_AOT_WARM=1` (AOT). Suite baseline: 1684 passes / 0 fail.
Method: 6 parallel black-box hunters (differential interp-vs-AOT) + white-box review of the compiler/devirt/codegen. All findings below are reproduced and root-caused; nothing was fixed.

## Summary (severity-ordered)

| # | Severity | One-liner | Nature |
|---|---|---|---|
| 5 | High | `%{…}` interpolation reads locals as `nil` once the method is AOT-compiled | silent wrong output, **default config** |
| 9 | High | `Iterate#reduce:` (no seed) wrong for every fold but `+`/concat | silent wrong result, all configs |
| 10 | High | `List#each:` reuses one slot → closures capture the last element | silent wrong result, all configs |
| 13 | High | `^^` inside a `finally:` block returns a garbage stack value / `Stack underflow` | silent wrong result, all configs |
| 3 | High | Untyped local reassigned to a wider kind aborts under AOT | loud divergence, **default config** |
| 2 | High | `i64::MIN / -1` / `% -1`: interpreter panics uncatchably, AOT returns a value | uncatchable crash + divergence |
| 6 | High | Malformed/empty `%{…}` interpolation aborts the process uncatchably | uncatchable crash |
| 1 | Medium | Inlined `if:` on a comparison coerces truthiness (vs strict dispatched `if:`) | soundness gap, edge trigger |
| 4 | Medium | Integer literal ≥ 2^63 panics the parser uncatchably (also via `Runtime.eval:`) | uncatchable crash |
| 7 | Medium | `\uD800`–`\uDFFF` (surrogate) escape panics the parser uncatchably | uncatchable crash |
| 11 | Medium | Consecutive negative numbers in a list literal are silently subtracted | silent data loss |
| 12 | Medium | Class-structural errors are bare `String`s, uncatchable by `catch:{|e:Error|}` | error-handling gap |
| 14 | Medium | `whileDo:` with a non-Boolean condition loops forever instead of raising | infinite loop, all configs |
| 8 | Low | `\xXXXX` escape accepted by the grammar but never decoded | wrong result, all configs |

Reassuring negatives: the AOT/speculative tier was faithful across ~2200+ fuzzed class/dispatch/collection
programs and a broad AOT-divergence sweep (deopt, NLR, exceptions, fibers, channels) — the only interp-vs-AOT
divergences found are #2, #3, #5; every other finding is a shared interp+AOT bug in the compiler / stdlib /
parser / runtime. Findings #2 and #3 were each found independently by two hunters.

---

## Finding 1 — Inlined `if:`/`if:else:` on a comparison silently coerces truthiness instead of requiring a Boolean (inline vs. dispatch inconsistency)

**Severity:** Medium (soundness gap in an optimization; edge-case trigger; NOT AOT-specific — reproduces with `QN_AOT=0`).

**What:** The docs (`docs/language/02-blocks-and-control.md` §8) guarantee strict-boolean conditionals: "There is no truthiness coercion. A condition must be an actual boolean. Sending `if:` to a non-boolean — including `nil` — is a `MessageNotUnderstood`." This holds when `if:` is *dispatched*, but NOT when the compiler *inlines* it. `if:`/`if:else:` applied directly to a comparison expression (`a == b`, `a < b`, …) is inlined as an **unguarded** branch that treats any non-`false`/non-`nil` value as true — so if the comparison operator was overridden to return a non-Boolean, the wrong branch is taken silently instead of raising `MessageNotUnderstood`.

**Minimal repro:**
```quoin
Bad <- { #'==:' -> { |o| 7 } }
(Bad.new == Bad.new).if:{ 'TRUE'.print } else:{ 'FALSE'.print }
```
- **Actual:** prints `TRUE` (coerces `7` → truthy). Same under `QN_AOT=0`, default, and `QN_AOT_WARM=1`.
- **Expected:** `MessageNotUnderstood` (receiver `Integer` does not understand `if:else:`), matching the dispatched path.

**Proof it's an inline-vs-dispatch inconsistency** — routing the identical value through a variable dispatches and correctly errors:
```quoin
Bad <- { #'==:' -> { |o| 7 } }
var r = (Bad.new == Bad.new)   "* r is Integer 7
r.if:{ 'TRUE'.print } else:{ 'FALSE'.print }   "* => MessageNotUnderstood (correct)
```

**Blast radius:** every comparison operator triggers it — `==`, `!=`, `<`, `<=`, `>`, `>=` (all confirmed). `nil` from the operator correctly takes the else-branch (so the falsy test is `false`/`nil`).

**Root cause:**
- `src/compiler/mod.rs:2257` — `binop_result_type` maps `Lt | LtEq | Gt | GtEq | Eq | NotEq => Type::Bool` **unconditionally**, regardless of operand types. A user class may override `#'==:'`/`#'<:'`/… to return a non-Boolean, so this static belief is unsound.
- `src/compiler/devirt.rs:76-80` — `try_compile_inlined_conditional` treats `static_type(subject) == Type::Bool` as a license to emit the **unguarded** inline (no `BranchIfNotBool` fallback). Its soundness comment (devirt.rs:42-44) argues "Boolean is sealed, so `if:` on a statically-Bool receiver always resolves to built-in True/False" — but that only holds if the *value* is really a Bool; a comparison's static Bool type does not guarantee it (user-overridable operators).
- For any other static type (`Any`), the code correctly emits the **guarded** form (`BranchIfNotBool` → cold real `if:else:` send), which is why the through-a-variable case errors correctly.

**Note (not a bug, related):** `whileDo:`, `&&`, `||`, and `!` all coerce truthiness generally (falsy = `false`/`nil`). Unlike `if:`, they are self-consistent (no inline-vs-dispatch split). `!` is a defined method (`Object#'!'`/`Nil#'!'`), and `&&`/`||` are short-circuit forms returning operand values. Whether the docs' blanket "no truthiness coercion" should be scoped to `if:` conditions is a docs question (see STALE_DOCS.md).

---

## Finding 2 — `i64::MIN / -1` and `i64::MIN % -1`: interpreter panics **uncatchably** while AOT returns a value (crash + divergence)

**Severity: High (uncatchable process abort + interp/AOT divergence).**

**Repro:**
```quoin
var min: Integer = 0 - 9223372036854775807 - 1   "* i64::MIN
(min / (0 - 1)).print
```
- `QN_AOT=0` (interp) and `gc_stress`: **Rust panic** `attempt to divide with overflow` at `src/devirt_ops.rs:48` (`:54` for `%`), exit 101. **Uncatchable** — `{…}.catch:{|e| …}` does not intercept it (the process aborts). Reproduces at top level, no loop needed.
- default / `QN_AOT_WARM=1` (AOT): runs, returns `-9223372036854775808` for `/`, `0` for `%`.

**Root cause:** `devirt_ops::int_bin` (`src/devirt_ops.rs:36-58`) computes `Div`/`Mod` with the plain `a / b` / `a % b`. Rust's integer division panics on `i64::MIN / -1` **unconditionally in every build** (it's LLVM UB otherwise, not gated by overflow-checks). The code comment there — *"`i64::MIN / -1` overflows and wraps like the plain `/` … so guard on `b == 0` rather than using `checked_div`"* — is factually wrong: plain `/` does not wrap, it panics. The AOT codegen (`src/codegen/translate.rs:3044-3065`) *does* special-case divisor `-1` (`ineg`/`0`), so the two paths diverge and the interpreter is the one that crashes. Fix should make `int_bin` match the AOT: treat `b == -1` as `Div → a.wrapping_neg()`, `Mod → 0`.

---

## Finding 3 — Untyped local reassigned Int→wider-kind (Double/String) aborts under AOT (default config) but runs in the interpreter

**Severity: High (default-config divergence; breaks a legal, un-annotated program).**

**Repro:**
```quoin
M <- { .meta <-- { go -> { var x = 100; x = x + 0.5; ^x } } }
var k: Integer = 0
{ k < 5000 }.whileDo:{ (M.go).print; k = k + 1 }
```
- `QN_AOT=0` (interp): prints `100.5` every iteration (correct — an untyped local may legally hold any type).
- default / `QN_AOT_WARM=1` (AOT): prints `100.5` for the first **8** calls (the warm threshold), then — once `go` is compiled — raises `Type error: AOT-compiled method: a value did not match its declared scalar type`. Catchable as a `TypeError`, but the interpreter never raises it, so it's a semantics-breaking divergence on the default config.

**Natural, idiomatic repro (computing an average)** — this is why the bug matters:
```quoin
Stats <- { .meta <-- { normalize: -> { |vals|
  var total = 0
  var i = 0
  { i < vals.count }.whileDo:{ total = total + (vals.at: i); i = i + 1 }
  total = total / (vals.count * 1.0)   "* int sum ÷ double → double, reassigned to int-slotted `total`
  total
} } }
var data = #(2 4 6 8); var k = 0
{ k < 20 }.whileDo:{ Stats.normalize: data; k = k + 1 }
(Stats.normalize: data).print
```
- interp: `5`. default/warm AOT: `Type error: … a value did not match its declared scalar type`.

**Trigger precisely:** an untyped local, first-seen as `Int`/`Double` (so slotted as an unboxed scalar), later **reassigned** a different kind (`Double`, `String`). A local captured by a *real* block frame (e.g. `sum` accrued inside an `each:` block) is boxed and does NOT trigger it — but a `whileDo:` body is fused (not a real frame), so its loop locals stay unboxed and DO trigger (as in the average above). A *fresh* `var y = x + 0.5` (no reassignment) is fine. **Cold-path variant is worse:** if the wider-kind write is on a branch never taken during warmup, the first call that takes it *crashes* rather than deopting (found by the AOT hunter, `findings/aot/BUG1_scalar_slot_reassign_crash.qn`). This also violates the project's own SPECULATIVE_AOT doctrine ("Mid-body surprises remain refusals/demotions, never runtime type errors the interpreter wouldn't raise"). Independently found by both the arithmetic and AOT hunters.

**Root cause:** the AOT slots the untyped local as an unboxed Int and enforces a scalar-type check on write-back (`narrow_error`, `src/codegen/helpers.rs:799`). That comment says the divergence is deliberate *"at the annotation that lied"* — but here there is **no annotation**, so the deliberate-narrowing rationale is misapplied: an untyped local has no type contract to violate, and the AOT overlay must be behavior-preserving (per `docs/ENV_FLAGS.md`: disabling AOT "is always semantics-preserving"). The untyped-local slot needs a deopt/box path for a wider-kind write instead of a fatal error.

---

## Finding 4 — Integer literal ≥ 2^63 panics the parser uncatchably (and `i64::MIN` is unwritable)

**Severity: Medium (parse-time process abort on a finite program; consistent across configs).**

**Repro:** `(9223372036854775808).print` (or `-9223372036854775808`, or `(99999999999999999999999999).print`)
- All configs: **Rust panic** `called Result::unwrap() on an Err value: ParseIntError { kind: PosOverflow }` at `crates/quoin-syntax/src/pest/parser.rs:624`, exit 101.
- **Expected:** a graceful (catchable, for `Runtime.eval:`) parse error, or promotion to the existing `BigInteger` type. It's a hard abort, and it's the reason `i64::MIN` must be built as `0 - 9223372036854775807 - 1`.
- **Also uncatchable via `Runtime.eval:`** — `{ Runtime.eval:'99999999999999999999999' }.catch:{…}` still aborts the process, whereas an ordinary syntax error (`Runtime.eval:'1 +'`) is correctly caught as a `ParseError`. The `try_parse_quoin_string_named` "fallible" path (added by the QUOIN_TODO `Runtime.eval:` fix) does the `str → i64` parse inside its AST-building `.map()` closure with `.unwrap()`, so a `PosOverflow` there bypasses the `ParseError` conversion. This is the same root shape as Findings 6 and 7: **AST-building steps that run *after* a successful pest parse (int→i64, `\u` unescape, interpolation re-parse) `panic!`/`unwrap()` instead of returning a catchable `ParseError`.**


---

## Finding 5 — `%{…}` string interpolation reads locals/params as `nil` when the enclosing method is AOT-compiled (silent wrong output on default config)

**Severity: High (silent wrong answer under the shipping default; no error raised).**

**Repro:**
```quoin
M <- { .meta <-- { render: -> { |n| var a = n; %'x=%{a}' } } }
var k = 0
{ k < 20 }.whileDo:{ (M.render:199).print; k = k + 1 }
```
- `QN_AOT=0` (interp): `x=199` (correct).
- default / `QN_AOT_WARM=1` (AOT): `x=` — the local `a` is seen as `nil`. `%{a > 5}` even throws `MessageNotUnderstood receiver=Integer selector='<:' args=[Nil]` (comparing against the nil-read `a`).

**Scope:** affects both parameters and `let`/`var` locals referenced inside `%{…}`. Literal / pure-expression interpolations (`%{7+8}`) are fine. The `%:` binary-`%` formatter is unaffected — only `%{}` (which re-compiles the inner expression) diverges.

**Root cause:** `String#mod` (`src/runtime/string.rs:285-320`) recovers the caller's locals by walking `caller_frame.env` and binding those names before re-compiling the `%{}` expression. An AOT-compiled frame does not materialize its locals into `env`, so every caller local resolves as an unknown global → `nil`. The silent-wrong-output nature (no error) makes this the most insidious of the interpolation bugs.

---

## Finding 6 — Malformed or empty `%{…}` interpolation aborts the process uncatchably

**Severity: High (uncatchable process abort from ordinary code).**

**Repro:** `(%'oops %{ 1 + }').print`  or  `(%'%{}').print`
- All configs: **Rust panic** at `crates/quoin-syntax/src/pest/parser.rs:91`, exit 101. Wrapping in `{…}.catch:{…}` does **not** save it — the process aborts before `finally`/`catch` runs (`'AFTER'` never prints).
- **Expected:** a catchable error (e.g. `ParseError`), since `%{}` re-compiles user-controlled expression text at runtime — exactly the case the `Runtime.eval:` catchability fix was meant to cover.

**Root cause:** `String#mod` re-parses the inner interpolation expression with the **infallible** `parse_quoin_string` (`parser.rs:82/91`), which `panic!`s on any pest error rather than returning a `ParseError`. (Same theme as Finding 4.)

---

## Finding 7 — `\uXXXX` escape in the UTF-16 surrogate range (`\uD800`–`\uDFFF`) panics the parser uncatchably

**Severity: Medium (uncatchable abort; also fires via `Runtime.eval:`).**

**Repro (file):** `'\uD800'.length.print`
- Panic at `crates/quoin-syntax/src/pest/parser.rs:1296`: `Invalid unicode escape sequence \uuD800` (exit 101). All of `\uD800`..`\uDFFF` reproduce. Also uncatchable when reached via `Runtime.eval:`.
- **Expected:** a clean, catchable parse error (an unpaired surrogate is not a valid `char`).

**Root cause:** `unicode_from_hex` → `char::from_u32(0xD800)` returns `None`, and the caller `panic!`s instead of surfacing a `ParseError`. **Cosmetic sub-bug:** the message doubles the `u` (`\uuD800`).

---

## Finding 8 — `\xHHHH` string escape is accepted by the grammar but silently not decoded (dead code)

**Severity: Low (wrong result; all configs agree).**

**Repro:** `'\x0041'.length` → **6** (the literal characters `\ x 0 0 4 1`) instead of **1** (`'A'`); `'A'` correctly yields `'A'`.
- The pest grammar (`crates/quoin-syntax/.../Quoin.pest:23`) allows `"\\" ~ ("u" | "x") ~ ASCII_HEX_DIGIT{4}`, and the parser has a `"x" =>` unescape arm (`parser.rs:1299-1305`), but the `unescape` driver regex (`parser.rs:1281`) only matches `\u…`, so the `\x` arm is dead code and `\x…` passes through verbatim. Either implement `\x` or reject it in the grammar. (Docs §3 list `\xXXXX` as a supported escape, so this is impl-lags-docs — but since the grammar accepts it and produces a wrong value rather than an error, it's a real defect, not merely stale docs.)

---

## Finding 9 — `Iterate#reduce:` (no seed) is broken for every fold except addition/concatenation

**Severity: High (silent wrong result from a core stdlib combinator; all configs).**

**Repro:**
```quoin
(#(2 3 4).reduce:{ |a b| a * b }).print       "* => 0    (expected 24)
(#(100 10 1).reduce:{ |a b| a - b }).print    "* => -111 (expected 89)
(#(5).reduce:{ |a b| a * b }).print           "* => 0    (expected 5)
(#(2 3 4).reduce:{ |a b| a + b }).print       "* => 9    (correct — by luck)
```

**Root cause:** `qnlib/core/02-iterate.qn` `reduce:` seeds the accumulator with the block's *class default* and then immediately folds the first element:
```quoin
reduce: -> { |block: Block(T T ^T) ^T?|
    var sum = nil;
    .each:{ |x|
        sum.defined?.else:{ sum = (block.valueWithArgs:#(x x)).class.default };   "* seed = 0 / '' …
        sum = block.valueWithArgs:#( sum x )                                       "* folds first elt against the seed
    };
    ^sum
};
```
So the result is `block(block(block(default, a), b), c)` — a phantom leading `default` operand. Correct only when `default` is a left identity of the operator: `+` (default `0`) and string concat (default `''`) survive; `*`, `-`, `/`, and any `min`/`max`-style fold via `reduce:` return garbage (`*` poisons to `0`; `-` negates). The fix is the standard seedless fold: `sum = x` on the first element, fold from the second.

**Why it passed CI:** every `reduce:` test in the suite (`qnlib/tests/01-iterate.qn`, `14-generators.qn`, `50-aot-parity.qn`) uses either `+` or the *seeded* `reduce:into:` (which is correct). The non-additive cases are untested. `sum`/`min`/`max`/`sum:` combinators don't route through `reduce:`, so they're fine.

---

## Finding 10 — `List#each:` fused loop reuses one slot for the block parameter; escaping closures all capture the last element

**Severity: High (silent data corruption in the idiomatic "build closures inside `each:`" pattern; all configs).**

**Repro:**
```quoin
var b = #()
#(0 1 2).each:{ |x| b.add:{ x } }        "* stash a closure over x each iteration
(b.collect:{ |f| f.value }).print         "* => #(2 2 2)   (expected #(0 1 2))
```

**Scope / proof it's a bug, not a design choice:**
- Only `List.each:` is affected. `Set.each:`, `Range#each:`, `Map#each:`, `Generator#each:` all capture correctly → `#(0 1 2)`.
- Binding the *same* block to a var first (`var blk = {|x| b.add:{x}}; #(0 1 2).each:blk`) captures correctly → referential transparency is violated by the literal-block fast path.
- Capturing a block-*local* instead of the param (`each:{|x - y| y = x; b.add:{y}}`) → correct `#(0 1 2)`.

**Root cause:** the `List.each:{literal-block}` fusion in `src/compiler/devirt.rs` (~lines 845-895) hoists the parameter's `DefineLocal($x)` *before* the loop and re-`StoreLocal($x)` each iteration, so every closure created in the body aliases the one shared cell (classic loop-variable capture, à la JS `var`-in-loop). A per-iteration fresh binding is needed. Shared by interp and AOT (bytecode-level), hence config-consistent.

---

## Finding 11 — Consecutive negative numbers in a list/collection literal are silently merged by subtraction

**Severity: Medium (silent data loss; a natural way to write a list of negatives).**

**Repro:**
```quoin
(#(-1 -2)).print              "* => #(-3)          (expected #(-1 -2))
(#(-1 -2).count).print        "* => 1              (expected 2)
(#(5 -10)).print              "* => #(-5)          (expected #(5 -10))
(#(-3 -1 -2 0 5 -10)).print   "* => #(-6 0 -5)     (expected 6 elements)
```

**Cause:** list-literal elements are parsed as greedy binary expressions (which is what lets `#(a + b  c)` be a two-element list). A `-N` immediately following another element is therefore consumed as infix subtraction, not a new element. `#(-1 2)` is fine (the `2` has no leading `-`); the first element may be negative. Workaround: parenthesize — `#((-1) (-2))`. This is the same family of sharp edge as the documented `#< … >` `>`-terminator gotcha, but it silently loses data rather than erroring, and writing a literal list of negative numbers is common. At minimum it deserves a documented gotcha; arguably the list-literal grammar should treat a whitespace-then-`-digits` as a new element.

---

## Finding 12 — Class-structural errors are thrown as bare `String`s, uncatchable by a typed `catch:{|e:Error|}`

**Severity: Medium (well-formed error handling silently fails to catch; all configs).**

**Repro:**
```quoin
Foo <- { n -> { 1 } }
Foo.sealed!
{ Foo <-- { m -> { 2 } } }.catch:{ |e:Error| 'handled'.print }   "* never runs → error escapes uncaught
```
- The `Cannot extend sealed class` error is a raw `String`, so the typed handler `|e:Error|` doesn't match and **re-raises** — the program dies with an uncaught "VM execution error". A bare `catch:{|e| …}` catches it, but `e.class.name` is `String`, not an `Error` subtype.

**Affected operations (all throw Strings):** `.sealed!` extend/subclass, `.abstract!` instantiation, constant redefinition (`K <- 2` when `K` exists), undefined/non-class parent in `Parent <- Child <- {…}`, `-->` on a selector absent from the hierarchy, assignment to an undeclared `@ivar`. By contrast `MessageNotUnderstood` / `AmbiguousMethodError` / mixin-requirement failures are proper `Error` instances.

**Impact:** contradicts the structured-error design (docs §15, "the VM maps its internal errors to these Quoin `Error` objects"). A program that correctly writes `catch:{|e:Error| …}` around class manipulation gets no protection. (Flagged as a known gap in a `00-bootstrap.qn` comment, but with real user-facing impact — worth fixing, or at least documenting the exceptions.)

---

## Finding 13 — `^^` (non-local return) inside a `finally:` block returns a garbage stack value or `Stack underflow`

**Severity: High (silent wrong result / stack corruption; all configs).**

**Repro:**
```quoin
Foo <- { }
Foo <-- {
  m -> {
    { 'body-ok' }.catch:{ |e| 'c' } finally:{ ^^ 'RET-FROM-FINALLY' }
    'normal-after'
  }
}
Foo.new.m.print
```
- **Actual:** prints `class Set` — a leaked/garbage stack value. A slightly different shape (`{ 'body' }.catch:{|e| 'c'} finally:{ ^^ 'X' }; 'after'` at `-e` top level) yields `Stack underflow` instead.
- **Expected:** the `^^` overrides the method result, so `m` returns `'RET-FROM-FINALLY'`.
- Consistent across `QN_AOT=0` and AOT — a shared runtime bug, not a divergence. Reproduces whether the protected block succeeded or the `catch:` handled a throw.

**Root cause:** `do_catch_finally` (`src/runtime/block.rs:353-359`), success arm:
```rust
Ok(val) => {
    vm.push(val);                                     // stash protected result across `finally`
    let finally_res = vm.execute_block(mc, finally, …);
    let val = vm.pop()?;                              // ← reads a corrupted stack
    finally_res.map(|_| val)
}
```
When `finally` performs a `^^`, its `MethodReturn` (`src/vm.rs:~5459`) truncates the operand stack to the method frame's base and pushes its own return value — which discards the `vm.push(val)` stashed here. The following `vm.pop()?` then pops the wrong slot (a leaked value → `class Set`) or underflows (`Stack underflow`), and the intended `^^` value is lost. The success/handled arm needs to detect a non-local return / throw out of `finally` (as the `Cancelled` and compiled-NLR arms below it already do) and let it win, rather than unconditionally `push`/`pop`-ing around it.

---

## Finding 14 — `whileDo:` with a non-Boolean condition loops forever instead of raising (vs. `if:` which raises `MessageNotUnderstood`)

**Severity: Medium (infinite loop where an error is expected; `if:`/`whileDo:` inconsistency; all configs).**

**Repro:**
```quoin
{ 5 }.whileDo:{ … }     "* loops FOREVER (5 treated as truthy)
5.if:{ … } else:{ … }   "* MessageNotUnderstood (Integer doesn't understand if:else:)
```
`whileDo:` continues unless the condition value is exactly `false` or `nil` (truthy loop test), so a non-Boolean condition (`Integer`, `String`, `0`, …) never terminates — whereas the documented contract (§8) is strict Boolean, and `if:` enforces it by raising `MessageNotUnderstood`. Root: the loop lowers to an `is_truthy` test (`src/codegen/translate.rs:~1510` and the interpreter's `whileDo:`), not a Bool check. Same truthiness family as Finding 1 and the `&&`/`||`/`!` note; the distinctive hazard here is a **silent infinite loop** rather than a wrong branch. Either enforce a Bool condition (raise on non-Bool) or document `whileDo:`/`&&`/`||`/`!` truthiness explicitly (see STALE_DOCS.md).

---

## Secondary notes (lower-priority / partially-known)

- **File-mode compile/parse errors `panic!` (exit 101) while `qn -e` reports them cleanly (exit 1).** `src/runner.rs:1013` `panic!("Compilation error: …")` and `crates/quoin-syntax/.../parser.rs:160` `panic!("Pest parsing error…")` on the file path; the identical source via `qn -e` prints a clean `Compile error:`/`Parse error at …`. The missing-file panic (`parser.rs:144`) is the same family. The main-program parse-panic is "by design" per QUOIN_TODO, but the **file-vs-eval inconsistency for the same error** is a real wart, and a plain missing file / unknown flag (`qn --help`) aborting with a Rust backtrace is poor CLI hygiene.
- **Redefining a guarded variant with a byte-identical guard doesn't replace** (two `m: -> {|x { x > 5 }| …}` coexist → `AmbiguousMethodError`), unlike same-typed-signature redefinition which replaces. Explicitly a known limitation in QUOIN_TODO ("guards aren't compared for equality"). Low.
- **Bad comparator to `sort:` (returns a non-boolean) silently produces an unsorted result** rather than erroring (`#(3 1 2).sort:{|a b| 'yes'}` → `#(2 1 3)`). Garbage-in; minor.
- **Dangling `^^` (non-local return whose home method already returned) silently unwinds to the top-level program frame and terminates it early**, rather than raising a "cannot return" error. Repro: `Maker <- { .meta <-- { make -> { { ^^ 'RET' } } } }; var blk = Maker.make; blk.value; 55` prints `RET` (the escaped value) and never evaluates `55` — exit 0. Consistent interp↔AOT. When wrapped in a `catch:`, it surfaces as a bare `String` `'Non-local return'` (so `catch:{|e:Error| …}` can't catch it — same family as Finding 12; a typed non-matching handler lets it unwind to top instead of re-raising). Most languages (Ruby `LocalJumpError`, Smalltalk `cannotReturn:`) raise here. Borderline design-vs-bug; consistent across tiers.
