# Quoin Runtime & Library TODO List

This document outlines the language features, compiler updates, and VM modifications required to execute the Quoin standard library (`qnlib`) files and test suites.

## Misc
- [ ] `.is:`/`.isTrue:` assertions inside an `each:` block wedge the `qn test`
  harness (the suite stops at that test with no failure reported). No suite uses the
  pattern today — found writing `47-url.qn`, worked around by unrolling. Either
  support it or fail loudly.
- [ ] Harden the "value types have no instance variables" check. Today the compiler
  rejects `@x` in a value-type extension whose target is *statically* a value type
  (`Integer <-- …`, `5 <-- …`, `true <-- …`). A **computed** target slips through —
  e.g. `(1 + 2) <-- { |@x| test -> { @x } }` compiles (harmlessly: `@x` reads `nil`,
  `@x =` throws at runtime, so it's useless rather than wrong). Closing the gap needs
  a runtime check in `get_target_class_for_def`: when the receiver resolves to a value
  type, reject instance-variable declaration/use. See the note on
  `Compiler::is_value_type_target`.
- [ ] Investigate a latent GC root-coverage gap surfaced by ultra-aggressive collection.
  Forcing `arena.finish_cycle()` (or even `collect_debt()`) on *every* VM step instead of every
  10 (`src/runner.rs`) makes the bblib `test` run fail with `Message not understood:
  receiver=Nil, selector='add:'` — some value the test harness relies on is collected when GC
  runs that frequently. **Reproduces identically on a pre-`send-receiver-split` HEAD**, so it
  predates that change (not caused by the receiver/args rooting, which was stress-validated
  separately). The normal `% 10` debt-paced collection masks it. Worth tracking down: likely a
  temporary that's reachable only via the Rust stack across a step boundary in the `add:` /
  collection-builder path. See `profiling/send-receiver-split/notes.md`.
- [ ] Use a proper arg parsing library instead of the `VmRunnerMode` stuff in `runner.rs`.
- [ ] Add a Quoin builtin for exiting the process with a status code (like C's `exit(status)`) —
  e.g. `Runtime.exit:0` / `Runtime.exit:1` — threading a requested exit code out of the VM to
  `std::process::exit`. Once it exists, the `qn test` harness (`qnlib/main.qn`) can call it
  directly instead of the Rust driver inferring pass/fail from the program's final value (today
  `compile_and_run_asts` in `src/runner.rs` exits non-zero when a `qn test` run aborts on a VM
  error or its final result is falsy).
- [ ] Design an installer.
  - [x] Named the language **Quoin** (extension `.qn`); rationale in `~/code/quoin/DECISIONS.md`.
  - [x] Binary name is `qn` (set via `[[bin]]` in `Cargo.toml`).
  - [ ] Support installing the binary and support files to `/usr/local/bin` or something.
  - [x] Create a more general purpose way of determining what to load by default on start.
    - The prelude is now `qnlib/prelude.qn` (`use core/*`), loaded by the runner alongside one
      mode-entry file — instead of a hardcoded `glob("qnlib/*.qn")`.
- [x] Support importing files explicitly. `use (pkg:)? path;` — a soft keyword that loads a `.qn`
  file once (run-once, cycle-safe) through a host-swappable `PackageResolver` seam, so the VM never
  touches `std::fs` (works on WASM / embedded). Packages: bare or `std:` = stdlib (`$CWD/qnlib`),
  `self:` = the project (`$CWD`), other names are a reserved stub ("cannot resolve"). `dir/*` globs a
  directory in UTF-8-sorted order. The load *path* is decoupled from the `[Ns]` *namespace* a file
  registers under. Reference: `docs/language/` §21.
  - [ ] When the installer work is done, search for files in standard locations + wherever the binary
    is installed. (Today both roots are `$CWD`-relative; `self_root` can later anchor to the entry-point
    directory, and the stdlib can be embedded via `include_dir!`.)
- [x] Change the file extension to `.qn` everywhere.
  - [x] Don't forget to update the plugin.
- [x] Get rid of `Value::Native`, it's only used by the global funcs and those are only used for testing.
  - In the Quoin language itself all methods are attached to a class.
- [x] Wire `assertMeetsRequirements:` into `mix:` so a mixin can declare requirements its host class must satisfy.
  - [x] Implemented `can?:` (`src/runtime/object.rs`), overloaded by argument: a Symbol/String selector asks "does the receiver implement that method?" (instance/class methods for instance/class receivers, class-side for metaclass); a Class asks "is-a / mixes in?". Removed the `.can:` alias for `.mix:` to disambiguate (`.can:` call sites converted; obsolete `can?: -> {|clz| clz == Iterate}` defs removed). To make `ClassName.meta.can?:` reachable, a metaclass (`ClassMeta`) receiver now falls through to `Object`'s instance methods in dispatch (`src/vm.rs`) — i.e. metaclasses act as if they subclass `Object` (gaining `can?:`, `s`, `==:`, …). Tests in `qnlib/tests/17-can.qn`.
  - [x] `mix:` enqueues the mixin's class-side `assertMeetsRequirements:host` (if defined) as a **deferred call** that runs at the end of the host's definition block — added a general frame-level defer mechanism (`DeferredCall`, `Frame.defers`, run on *normal* block completion in the Return handler, `src/vm.rs`). Defers run *before* the frame is popped, so the queue stays GC-rooted via `self.frames` even if a defer yields (a collection during the suspension would otherwise free Values reachable only through the defer). Regression tests: `test_deferred_call_values_survive_collection` (Rust) and `yieldFromDeferredMixinCheck` (`qnlib/tests/13-fibers.qn`). Deferring to block-end means required methods may be defined *after* the `.mix:` (the universal idiom). On failure the class is unregistered (`Frame.unregister_on_defer_failure`, seeded by `pending_class_def`) so a class with unmet requirements is never left registered. `test.qn` switched from the undefined `implements?:` to `can?:`. Tests: `qnlib/tests/05-classes.qn` (mixinRequirements). Subclassing needs no separate check — a subclass inherits a parent that already passed.
- [x] Implement the class-marker methods.
  - [x] `sealed!` — sets `Class.is_sealed`; refuses extension (`<--`, `->`/`-->`, `.mix:`) **and**
    subclassing, on a class or an instance's eigenclass (`Object#sealed!`). Guards in
    `DefineMethod`/`OverrideMethod` (via `ensure_not_sealed` after `get_target_class_for_def`), the
    `DefineClass` parent resolution, and `Class#mix:`. Errors: "Cannot extend sealed …" / "Cannot
    subclass sealed class …". (Sealed is the intended future trigger for devirtualization — a leaf with
    a fixed method table.)
  - [x] `abstract!` — sets `Class.is_abstract`; refuses `new`/`new:` on the class itself via
    `ensure_instantiable` in `Callable::New`/`NewNoBlock`, while concrete subclasses still instantiate
    ("Cannot instantiate abstract class …"). Independent of `sealed!`. Tests in
    `qnlib/tests/20-markers.qn`.
- Overhaul method dispatch with hierarchy-distance scoring, working toward fully unifying native and user methods under one scored multimethod model (the eventual goal: native methods carry type signatures and the hardcoded type-switching inside native fns is extracted into typed variants the scorer routes between).
  - [x] **Phase 1 — the scoring algorithm.** Replaced the pairwise `compare_specificity` (which returned `Equal` for incomparable types, so wasn't a total order — the fragile stable sort that the `-->` tie-break disaster came from) with per-candidate scoring in `lookup_method_in_class_hierarchy_rec` (`src/vm.rs`): `match_score` returns `None` if a variant doesn't apply (a typed param's arg isn't assignable, a guard fails, too few args) else `Σ` over params of `type_distance` (exact = 0, +1 per hop up the hierarchy; untyped param = a 1,000,000 sentinel so typed always wins). Lowest score wins; ties go to the first-defined (we only replace `best` on a *strictly* lower score), preserving ordered-guard dispatch. Written representation-agnostic — `param_types`/guard are read through `get_block_from_method`, and a legacy native method (no block) scores as `i64::MAX` (ranked last), so Phase 2 slots in without touching the scorer. Removed the now-dead `compare_specificity`, `method_matches_arguments`, `is_subclass_of` (string), `matches_type`. **Correction to the original plan:** scoring does *not* subsume `replace_or_append_method_in_chain` — guard-differentiated variants need first-defined-wins, which conflicts with most-recent-override, so replace-at-definition stays. Regression caught & fixed during this work: `type_distance` must treat a `Class`/`ClassMeta` *value* as being of type `Class` (the `val.type_name() == hint` fast path), else `|x:Class|`-typed methods (e.g. `assertMeetsRequirements:`) stop matching. Tests: `dispatchOnClassArg` (`qnlib/tests/06-methods.qn`); existing `dispatchByBlock`/`dispatchTypePriority`/`overridesSameSignature` still green.
  - [x] **Phase 2a — chainable native methods (no scoring change).** Generalized `NativeMethodState` to `{ selector, body: MethodBody, is_extension, next }` where `MethodBody = UserBlock(Value) | Native(NativeFunc)` (`src/runtime/method.rs`). `register_native_class` now wraps each native fn as a `Method` chain node (`new_native_method`, `src/vm.rs`) instead of a bare `ObjectPayload::Native`, so native methods are chainable, scored, override-able candidates. Invocation (`call_method_value` + the callable extraction) routes a native body to `NativeCallable`; `get_block_from_method` returns `None` for native bodies, so they still score `i64::MAX` (fallback) — **dispatch behavior is unchanged**, except that overriding a native method (e.g. `List <-- { count -> {…} }`) now works instead of crashing with "Invalid method object in chain". (Global operator funcs in `native.rs` stay bare `ObjectPayload::Native` — they aren't class methods.) Tests: `test_native_methods_are_chainable` (Rust); full suite green.
  - [x] **Phase 2b — typed native methods.** `MethodBody::Native` now carries `param_types: Option<Vec<Option<String>>>` (`src/runtime/method.rs`; `None` = untyped/legacy → `i64::MAX` fallback, `Some` = scored by type). The `NativeClass` trait returns `Vec<NativeMethodDef>` and the builder gained `.typed_instance_method`/`.typed_class_method` (`src/value.rs`); since several defs may now share a selector, `register_native_class` chains them into a multimethod (`src/vm.rs`). `match_score` reads a native variant's signature via `native_method_param_types` and scores it with the shared `score_param_types` helper (also used for user blocks). Existing native methods still register untyped (via `.instance_method`), so behavior is unchanged. (Minor semantic note: the builder's selector store became a `Vec`, so two `.instance_method` calls with the *same* selector now chain — first-defined wins on a tie — instead of the last silently overwriting; no current native class relies on that.) Tests: `test_typed_native_method_dispatches_by_type` (Rust). Phase 3 (extracting in-fn type-switches into typed variants) can now proceed per-method.
  - **Phase 3 — migrate native fns.** Extract a native fn's internal type-switching into typed variants routed by the scorer. Incremental, per-method. A pattern that matches no variant now raises `MessageNotUnderstood` (replacing the hand-written `TypeError` — accepted: MNU is the correct "no matching variant" error).
    - [x] `String#replace:with:` — the exemplar (a genuine *multi-type* switch). Split into `typed_instance_method("replace:with:", &["Regex","String"], …)` + `&["String","String"]` (`src/runtime/string.rs`). Tests: `replaceWith` in `qnlib/tests/08-strings.qn` (pinned before the refactor; covers both paths + the MNU case).
    - Survey: `split:` was already idiomatic (typed Quoin variants `|pat:String|`/`|p:Regex|` delegating to type-specific natives in `04-string.qn`) — no migration needed; it shows the target shape.
    - **Operators as methods (the big one).** Binary `a + b` already lowers to a method send; the receiver's class is consulted *first*, falling back to a global native fn (`native.rs`) that type-switches internally. Target (per the language's `+:` convention): the compiler lowers `a OP b` → `Send(a, "OP:", [b])` (the `:` keyword selector — `+:`, `-:`, `==:`, …; `+` with no colon stays for *unary* plus), operators become typed multimethods on the numeric/string classes, and the global fn is rekeyed to the `:` selector as a fallback (its internal `+:` delegation dropped — class-first dispatch resolves user `#'+:'` overrides). Coercion helpers `Value::as_i64`/`as_f64` (`value.rs`) keep the variants terse. Future compiler optimization: auto-coerce RHS to the LHS type in operator sends.
      - [x] Arithmetic + ordering done on the `:` convention: compiler lowers `+ - * / % < > <= >=` to their `:` selectors; `Integer` carries typed `[Integer]`/`[Double]` variants via the `int_binop!` macro (`integer.rs`, using `Value::as_i64`/`as_f64`; `/:`/`%:` guard Integer div-by-zero); the global fns are rekeyed to the `:` selectors with their delegations removed (`native.rs`); `String#<`/`>` renamed to `<:`/`>:`; `List#sort`'s internal `call_method(…, ">")` → `">:"`. Behavior-preserving (`09-numbers`/`08-strings`/`Iterate` pin it; user `#'+:'` override verified). Perf: within noise of the global-fn path. `Double`/`String`/mixed arithmetic still resolve via the rekeyed global fallback (which keeps type-switching) — fine until those classes get their own variants.
      - [x] `==`/`!=` done: compiler `Eq => "==:"`, `NotEq => "!=:"`; globals rekeyed `==`→`==:`/`!=`→`!=:` with `native_eq`'s `==:` delegation removed (`native.rs`). No new methods needed — `Object#==:`/`Object#!=:` already exist (the latter derived from `==:`), so every receiver resolves class-first (the global is effectively a dead fallback now). No internal code calls bare `"=="`/`"!="`. Verified: cross-type (`5==5.0`→true, `5=='a'`→false), class, nil equality all preserved.
      - [x] Done across three migrations. **(1) `~` (match):** the compiler now lowers `~` → the `~:` selector (like every other operator); `native_match` was decomposed into per-class `~:` methods — `Regex#~:` (native, regex engine), `Block#~:` (Quoin, `valueWithSelfOrArg:` predicate guard), `Class#~:` (Quoin, `{|x| x.can?:self}`) — with the existing `Object#~:` (`==:`) and `NumberRange#~:` as fallbacks. `~` is now **forward-only** (the matcher on the left — the case-statement convention); the `is:a:` test helper was flipped to `expected ~ actual` and two presentation docs corrected. `native_match`/`is_instance_of` deleted. **(2) `Double`/`String` + demote-to-Quoin:** `Double` got typed `[Integer]`/`[Double]` arithmetic + `<:` via a `double_binop!` macro; `String` got `+:` (String fast-path + a `.s`-coercing fallback) and `%:` (positional/named formatting, moved off the global); `<:`/`==:` are native per primitive type while `>:`/`<=:`/`>=:` derive as shared Quoin on `Object` (`>` ≡ `x < self`, etc.), and the booleans got `<:` on `true`/`false`. `Integer`'s native `>:`/`<=:`/`>=:` moved to Quoin. **All** the global fallbacks (`native_add`/`sub`/`mul`/`div`/`mod`/`lt`/`gt`/`le`/`ge`/`eq`/`ne`) were **deleted**. **(3) Unary `-`:** the compiler emits `Send("-", 0)` and `Integer#'-'`/`Double#'-'` are Quoin (`0 - self`); the `-`→`negated` and `+`→`posated` selector renames were removed entirely — the operator *is* the selector everywhere. Unary `+` (`Object#'+' -> { self }`) and `!` (`Object`/`Nil` Quoin) likewise. The whole `native.rs` (also `print:*`/`regex_match:`, refactored to `(x+y).print`/`Regex#~:`) was deleted and the bare `ObjectPayload::Native` variant removed — the global native-func table is now empty.
      - [x] **Demote natives to Quoin where possible.** Done for the operators (the main case): the *derived* comparisons (`>:` ≡ `x < self`, `<=:` ≡ `!(x < self)`, `>=:` ≡ `!(self < x)`) are shared Quoin methods on `Object`; `!` and unary `+`/`-` are Quoin; equality stays as `Object#==:`/`#!=:`. Native is kept only where it genuinely needs Rust (raw per-type arithmetic, string ops, regex, native state). (A broader pass — auditing *non-operator* natives that only compose other sends, e.g. in `list.rs`/`map.rs`, and moving them to qnlib — remains as optional future cleanup.)
    - [x] *Single-type checks* migrated to typed variants (wrong type → MNU instead of a hand-rolled `TypeError`): `List#at:`/`at:put:`/`sliceFrom:` (`&["Integer"]` — only the index is typed; `at:put:`'s value stays untyped) and `String#insert:at:` (`&["String", "Integer"]`). The index is then extracted with `arg!(…, Int, …)` (pure extraction — the scorer already guaranteed the type). Left as *not* this pattern: `Fiber.new:`/`KeyValuePair.new:` (class-side constructors entangled with `new:`/`NewCallable` dispatch — typing them would mis-route to the default constructor) and io.rs's internal String/ANSI coercion helper (not a dispatched method). Coverage: repointed `runtimeTypeErrorIsStructured` (`07-errors.qn`) to a still-`TypeError` op (`'abc'.contains?:5`, an `arg!`-based check) so it keeps demonstrating structured TypeErrors, and added a `typedArgDispatch` test pinning both the valid-dispatch and wrong-type→MNU paths for all four methods. The `at:put:` hot path (sieve benchmark) verified.
  - [x] Ambiguity detection (enabled by the total order). Scoring is now lexicographic — `(Σ type_distance, guarded?)` — where an untyped param counts as `:Object` (the universal supertype) so the `UNTYPED_PARAM_SCORE` sentinel is gone, and a guard *refines* specificity (a guarded variant outranks an otherwise-equal unguarded one). The lowest score wins; **two distinct candidates sharing the lowest score throw `AmbiguousMethodError`** — this covers both equal-distance unguarded *typed* variants (e.g. two mixin types at distance 1) and two *guarded* variants that both pass at the same type level. Definition order is no longer a tiebreaker (so overloaded methods can't rely on ordered overlapping guards — that's `case`/`~`'s job, which is sequential and unaffected). A guarded+unguarded pair never ties (the guard rank separates them), so the specific-guards-then-unguarded-catch-all idiom is unambiguous; `dispatchByBlock`'s catch-all changed from a `{.class==Object}` guard to a plain `|x|`. Signatureless native methods score `i64::MAX` and are exempt (a pure fallback, never ambiguous). New `AmbiguousMethodError` Quoin error type. Tests: `dispatchAmbiguityType`/`dispatchAmbiguityGuard` (`06-methods.qn`).
  - [ ] **Make `Class` and `ClassMeta` directly subclass `Object`** so the simulation hacks aren't needed: today a metaclass receiver *falls through* to `Object`'s instance methods in `lookup_method`, and `type_distance(_, "Object")` has a universal-supertype fallback (so untyped/`:Object` params still match metaclass values whose `parent` chain doesn't physically reach `Object`). Wiring `Class`/`ClassMeta`'s parent to `Object` for real would let both hacks be removed.
  - [x] When no method match is found but the _selector_ does exist, the filtered-out candidates are listed in the `MessageNotUnderstood` error (a hint that the method exists but the arguments were wrong). `MessageNotUnderstood`/`AmbiguousMethod` (`error.rs`) each carry a `candidates: Vec<String>` rendered one-per-line below the message and above the stack trace (`QuoinError` Display). Candidate signatures use the stack-trace style — selector keywords interleaved with each variant's *declared* param types, e.g. `bar:Integer`, `bar:String {x.length > 3}` — with a guarded variant's guard shown as its syntax-highlighted source (or a colorized `{...}` placeholder when source is unavailable), via `format_candidate_signature`/`collect_method_candidates` (`vm.rs`). Display-only (the caught path keeps the concise message). Tests: `dispatchNoMatchRaisesMNU` (`06-methods.qn`) + Display unit tests (`error.rs`).
  - [ ] **Per-argument guard blocks (multiple guards per method).** Intended design: a `{…}` guard block may follow *any* parameter (and several may appear in one param list), and each guard is evaluated against the argument it follows — `self` (`.`) and the guard's own first param are bound to *that* argument's value. A variant applies only if **all** its guards pass.
    - **Current state (single-guard only).** The representation keeps just one guard per method: `BlockNode.decl_block: Option<…>` and `BlockArgNode { identifier, type_hint }` has no per-arg guard slot. The parser (`parse…` in `parser/pest/parser.rs`) does `decl_block = Some(b)` for *each* `{…}` it sees in the param list, so multiple guards **collapse to the last one** (earlier guards silently dropped) and none is associated with a specific argument. `execute_validation_block` (`vm.rs`) binds **every method argument by its parameter name** (so `{ x > 100 }` / `{ a < b }` reach any arg directly), binds `self` to the **method's receiver** (the subject of the send — threaded through `lookup_method` → `lookup_method_in_class_hierarchy[_rec]` → `match_score`), so a guard can also use the class's other methods/instance vars, and doesn't re-declare its own params. (Earlier this bound `self` and the guard's own params to `args[0]`; both were dropped in favor of by-name args + receiver-`self`.) The grammar already *allows* writing interspersed guards; only the AST/parser/eval don't honor them (the guard isn't tied to its argument, and per-arg `self` isn't a thing yet).
    - **Implementation scope:** (1) AST — move the guard onto the argument, e.g. `BlockArgNode { identifier, type_hint, guard: Option<Arc<BlockNode>> }` (or a `Vec<(usize, guard)>` on `BlockNode`). (2) Parser — attach each `{…}` to the argument it follows instead of overwriting one slot. (3) Dispatch (`match_score`) — a variant applies iff *all* its per-arg guards pass; evaluate each with `self`/first-param = its own argument. (4) Error formatting — render each guard right after its argument (`foo:Integer {x>0} bar:String {y.len>3}`) in `format_candidate_signature`, replacing the single trailing-`{guard}` rendering.
    - **Open questions:** (a) **Scoring/specificity** — does *any* guard just make a variant "guarded" (today's single rank bit in the lexicographic `(Σ type_distance, guarded?)` score), or should *more* guards mean more-specific (so `{g1}{g2}` outranks `{g1}`)? This changes which guarded variants tie → throw `AmbiguousMethodError`. (b) A guard with no params and no `.` usage — is binding `self` to its argument enough, or do we also want positional access to *other* args inside a guard (currently all method params are in scope by name; keep that, or restrict a guard to only its own argument)? (c) Evaluation order / short-circuit — left-to-right, stop at first failing guard (matters only for guard side effects, which should be discouraged).
- [x] Implement the `#< … >` set literal. Added a native `Set` type (`src/runtime/set.rs`, `NativeSetState`) mirroring `List`/`Map`: insertion-ordered, unique by `==:`, with `count`/`add:`/`remove:`/`contains?:`/`each:`/`s`/`==:`; `Set` mixes in `Iterate` and gets `union:`/`intersection:`/`difference:`/`subset?:`/`superset?:` in `qnlib/02-iterate.qn`. Literal compiles via a new `NewSet(n)` instruction (deduped by `==:`). The closing `>` collided with the greater-than operator, so the grammar now excludes `>`/`>=` from set elements (`set_elem`/`set_infix_op` in `Quoin.pest`) — a bare `>` ends the set; parenthesize to use `>` in an element. Tests in `qnlib/tests/15-sets.qn`; docs updated.
- [ ] Find duplicate bits of code and refactor.
  - Spinning the VM while executing in a native method.
  - Object initialization/new:{} logic
- [x] **Extract the dispatch subsystem out of `vm.rs`** (which is ~5.5k lines). Move the method-dispatch
  machinery into its own module (e.g. `src/dispatch.rs` or `src/vm/dispatch.rs`): the `Callable` enum +
  `call`, `lookup_method`, `lookup_method_in_class_hierarchy[_rec]`, `match_score`/`score_param_types`/
  `type_distance`, `MethodCacheKey` + `method_cache_key` + `invalidate_method_cache`, and the
  candidate/ambiguity helpers (`collect_method_candidates`, `ambiguous_method_error`,
  `format_candidate_signature`, …). Behavior-neutral move (methods stay on `VmState` via an `impl` block in
  the new module, or become free fns taking `&mut VmState`). Do it as its own commit, separate from any
  perf change, so the diff is a pure relocation. ~600 lines out of `vm.rs`.
- [x] Bring over AnsiColorizer.cs from the old repo.
  - [x] Switch to the colorized test suite runner.
- [x] Bring over Highlighter from the old repo.
- [x] Improve stack trace output. (Similar to the C# output.)
  - [x] Show highlighted block snippets to the right.
- [x] Move to a better iterator design that doesn't require mutability.
  - Iterate now requires only `each:`; `next`/`reset` cursor removed. Re-entrant, nil-safe.
  - [x] Use generators now that the VM supports them.
    - Added `Generator` (yield-block as iterable) and a fiber-backed external `Iterator` (`hasNext?`/`next`) in `qnlib/02-iterate.qn`.
- [x] Rewrite the TestSuite so it doesn't mix the tests into itself, too many conflicts. Test suites
  now **self-register** into a global `[Test]Suites` list on construction (`TestSuite#init:`), so a test
  file just builds its suite — no return-value plumbing or explicit registration. `main.qn` loads the
  suites with `use std:tests/*` and runs the registry; the per-file loader `Runtime.evalFile:` was
  removed (its only caller). A deeper rewrite of the assertion/Test/Suite class graph wasn't needed.
- [ ] List, Regex and Map #bind:{}
  - [x] List#bind:{}
  - [ ] Regex#bind:{}
  - [ ] Map#bind:{}
  - See qnlib/presentation/20-method-destructuring.qn
- [ ] Think about a better destructuring protocol than assuming `#at:` exists.
  - use an Iterator?
- [x] Confirm `%'string%{eval}' is working.
  - [ ] Optimize it into string concatenation by the compiler. (Today it recompiles the interpolated
    expression at runtime with the caller's local *names* in scope — implicit local capture via the env
    chain, see `string.rs`.) Lowering `%{expr}` directly to `String` concatenation at compile time removes
    that runtime recapture — which also clears a blocker for the slot-based local plan (§9 "local-variable
    slots", Plan B): with no implicit local capture left in interpolation, `(depth, slot)` resolution has
    no cross-compilation-unit holes. If we go to B, insert this between steps A and B.
- [x] Make sure case statements are tested and working.
- [x] Make the `^>` yield operator usable in expression position.
  - Moved `yield_return` from `stmt` to `primary` in the pest grammar; it now works anywhere an expression does (e.g. `a = ^> v`), with greedy operand precedence matching `Fiber.yield:` (parenthesize to scope). ANTLR grammar (legacy/unused path) left as-is.
- [ ] Have the `LoadGlobal` instruction consult the `BuiltinCache`. Currently it always does a `HashMap<NamespacedName, Value>` lookup against `globals` (see `vm.rs` `Instruction::LoadGlobal`); builtin classes (`Fiber`, `List`, `Integer`, etc.) could be served from the cache to avoid hashing the name on every load (e.g. for the `^>` -> `Fiber.yield:` lowering). `BuiltinCache` may need to be keyed more generally by name to cover all builtins.
- [x] Formalize an interface for Quoin error types.
  - `Error` base (`message`/`payload`, class-side `throw:`/`throw:payload:`) + core subtypes (`TypeError`, `ArgumentError`, `MessageNotUnderstood`, `ArithmeticError`, `IndexError`) in `00-bootstrap.qn`. Catch-by-type via `case`/`~`.
  - Runtime now raises structured errors: `QuoinError::Thrown` marker (value rides in `active_exception`), and `vm.quoinerror_to_value` maps internal `QuoinError` variants to typed Quoin `Error` objects at the `catch:` boundary. `does:throw:` widened to match by value/type or message string.
  - [ ] Future: give the VM more fine-grained internal error variants and route more raise sites through typed Quoin errors.
- [x] Make sure #symbol types are working.
- [x] Language server (~/code/quoin-language-server/)
  - [x] VSCode plugin (~/code/quoin-language-server/editors/vscode/)
- [ ] Integrate fff into claude for non-Rust searches
  - https://github.com/dmtrKovalenko/fff#mcp-server
- [x] Write a document fully explaining the language semantics, including all corner cases.
  - Capture the subtle/surprising behaviors here as they surface so they can be folded into the doc.
  - **`new:{}` block initialization & lexical scope.** Instance variables are *not* pre-bound inside a `new:{}` block, so an empty `new:{}` leaves every field at its default (`nil`) — it does **not** silently capture a same-named variable from the surrounding scope. Only an explicit assignment binds a field. The right-hand side of such an assignment resolves up the lexical chain (so `{ x = x }` copies the enclosing `x` into the field), but the assignment itself binds in the block's own frame and never mutates the enclosing variable. Corollary: a plain-assignment `init:` like `init: -> {|a| @a = a }` is redundant — field population already sets `@a` from the block before `init:` runs — so it behaves identically to the default no-op `init`.
  - **`init`/`init:` run the whole chain.** `new`/`new:{}` invoke the initializer of every class in the hierarchy (ancestors and mixins included), base→derived, with `init:` preferred over `init` per class. A derived `init:` no longer shadows/skips an ancestor or mixin `init`.

## Syntax

New surface syntax — grammar/parser changes and the language forms that ride on them. (Syntax
that already shipped stays inline in its home section: the `#< … >` set literal and the `^>`
yield operator in `## Misc`, operators-as-`:`-selectors in the dispatch overhaul, etc. Related
but *not* parser changes — left in place: per-argument guard blocks and `#bind:{}` destructuring,
both under `## Misc`.)

- [ ] **Scope a namespace for definitions (`module`-like).** A way to open a namespace in code so
  that `Class` and constant definitions inside the scope implicitly register under it — instead of
  repeating the `[Ns]` prefix on every definition. Analogous to Ruby's `module Foo … end` (or
  C#/Rust `namespace`/`mod`). The load *path* is already decoupled from the `[Ns]` namespace a file
  registers under (see `use` in `## Misc` / `## 7. Namespaces`). Open design questions:
  - **Form:** a block `namespace Foo { … }` vs a file-level `namespace Foo;` header applying to the
    rest of the file; nesting; interaction with the existing `[Ns]` prefix.
  - **Does this imply import scopes?** If a scope sets the *current namespace* for definitions, does
    it also scope *resolution* — unqualified names inside resolving against the open namespace first
    (scoped imports / `using`)?
  - **`use` inside a namespace block:** does a `use` within the scope alias everything the imported
    unit registers *under that namespace* — so those imports are visible unqualified within the
    block — and is that aliasing confined to the block?
- [ ] **`#b'HEX'` byte literal.** A `#`-prefixed user-literal for `Bytes`, like `#(…)` / `#/…/` /
  `#< … >`; a parser change. (Companion to the `BytesBuilder` in `## Networking & Async I/O`.)
- [ ] **User-facing `defer` form.** Expose the frame-level defer mechanism — today internal, used by
  `mix:`'s deferred `assertMeetsRequirements:` — to Quoin source as a `defer` form (see the `mix:`
  item in `## Misc`).
- [x] **Wildcard selector dispatch.** Variadic keyword selectors: a definition marks a repeatable
  keyword component with `+` (`catch+:finally:`; grammar `selector_w_args = (ident ~ kw_var? ~ ":")+`,
  `parse_selector` bakes the marker into the canonical name). At a call site there is no marker — a run
  of the **same consecutive keyword** folds into one `List` argument, resolved entirely at **compile
  time** in `compile_method_call` (emits `NewList(k)` + a normal `Send` to the canonical `name+:`
  selector), so dispatch/inline-caching is unchanged. `n=1` stays non-variadic (a lone `catch:` routes
  to a separate `catch:`, not `catch+:`). A definition may **not** repeat a keyword without `+`
  (`dup:dup:` → compile error in `reconstruct_selector`, surfaced as a catchable `ParseError` via
  `Runtime.eval:`). Tests: `variadicSelector*` / `singleKeywordDoesNotFoldToVariadic` /
  `repeatedKeywordSelectorIsACompileError` (`qnlib/tests/06-methods.qn`). Done as the foundation for
  type-based multi-catch (`{x}.catch:{|e:IoError| …} catch:{|e:Error| …} finally:{…}`); the native
  `catch+:`/`catch+:finally:` handlers + break-on-uncaught are the follow-on (see exception-handling).
- [ ] Implement `...` / `???` / `!!!`.
- [ ] **Full Unicode identifiers.** Today `IDENT_PREFIX`/`IDENT_REST` are ASCII-closed
  (`[a-zA-Z_][a-zA-Z0-9?_]*`); eventually identifiers should support full Unicode (UAX #31
  `XID_Start`/`XID_Continue` or similar). **Coupling to watch:** the compiler's alpha-renaming
  for control-flow fusion (docs/MATERIALIZATION_ARCH.md, M1) mints *source-unspellable* local
  names by using a character outside the identifier charset (e.g. `·` U+00B7) — the
  collision-freedom/invisibility guarantee is pure grammar closure. U+00B7 is `XID_Continue`
  (Catalan), so naive Unicode identifiers would make the minted names spellable and break the
  guarantee. Any Unicode identifier design must preserve a reserved compiler namespace: either
  explicitly exclude one sigil from the identifier grammar forever, or switch the renamer to a
  scheme the parser structurally rejects (e.g. a reserved prefix the grammar refuses).

## Networking & Async I/O

The async-networking stack (Stages 0–7) shipped on `main` (PR #1; design + as-built notes in
`docs/ASYNC_ARCH.md`): cooperative tasks/cancellation, `TcpSocket`/`TlsSocket`,
`Async.timeout:`, the `[HTTP]` client (incl. chunked), `ByteStream`/`StringStream`, file
streams, and `TcpListener` servers. These are the deferred refinements — none blocks the
core, and each fits the existing narrow-waist seam (a thin backend op + a QN class, or pure
Quoin over the current sockets/streams).

- [ ] **HTTP client refinements.** Remaining: keep-alive / connection pooling (a *stateful*
  client as an instance of `[HTTP]Client` — the class-side facade was kept thin for exactly
  this), and cookies. All pure Quoin over the existing sockets/streams. See `qnlib/net/http.qn`.
  - [x] **`Content-Encoding`** — transparent on responses (gzip / x-gzip / deflate / zstd),
    with `Accept-Encoding: gzip, zstd` advertised by default and decode-on-drain. Backed by
    new `Bytes` methods (`decodeGz`/`encodeGz`, `decodeDeflate`/`encodeDeflate`, `decodeZstd`)
    over `src/runtime/compress.rs` (flate2 miniz_oxide + ruzstd; pure Rust, no C toolchain).
  - [x] **Redirects** (3xx + `Location`) — followed by default, opt out with
    `[HTTP]Request.followRedirects:false` (+ `maxRedirects:`). 307/308 preserve method+body;
    303 and 301/302-from-POST downgrade to GET. Absolute / root-relative / path-relative
    `Location` resolution. `resolveLocation:against:`, `[HTTP]Response.redirect?`/`location`.
- [x] **Streaming chunked HTTP responses (lazy generator).** `send` returns a stream-backed
  `[HTTP]Body` over a non-scoped `ByteStream` (the socket stays open, owned by the body).
  Drain with `.text`/`.json`/`.bytes`, or stream with `.chunks`/`.each:` — a lazy `Generator`
  yielding one `[HTTP]Body` chunk per pull (chunked / Content-Length / close framing), each
  carrying its chunk-extension metadata on `chunk.meta`. The socket closes on drain / full
  iteration / `.close` / GC. `qnlib/net/http.qn`.
  - [x] **Content-decoded streaming.** A content-encoded body can't be decoded
    chunk-by-chunk with the one-shot codecs (a transfer-chunk isn't a complete gzip/zstd
    frame), so `.chunks`/`.each:` drain+decode the whole entity and yield a single decoded
    chunk; a non-encoded body still streams its raw transfer-chunks with per-chunk metadata.
    The internal wire-framing generator is `[HTTP]Body.rawChunks`.
  - [ ] Expose **trailer headers** (after the terminating `0\r\n`) — currently read and
    discarded.
  - [ ] **True per-chunk streaming decode** — incremental content-decode as chunks arrive,
    via a streaming (`ByteStream`) decompressor (see the `gzip / zstd` streaming follow-up
    below); today an encoded streamed body buffers the whole entity before decoding.
- [x] **Unified `[HTTP]Body` + JSON.** One value object (bytes- or stream-backed) backs both
  request and response bodies: `.bytes`/`.text`/`.json`/`.mediaType`/`.meta`. The polymorphic
  `[HTTP]Request.body:` auto-encodes a Map/List to JSON (`Content-Type: application/json`) and
  a String to bytes; responses auto-decode via `resp.body.json`. `qnlib/net/http.qn`.
- [ ] **TLS server-side.** Pair with `TcpListener`: accept a `TcpSocket`, then upgrade it
  with a *server* handshake (a rustls `ServerConfig` built from a cert/key) — the mirror of
  `TlsSocket.wrap:host:`. Needs a config-loading surface plus a backend op (e.g.
  `TlsAccept { id, config }`). Enables QN HTTPS servers.
- [ ] **Servers: serial `acceptLoop:` vs concurrent `TcpServer` — document the seam (and
  reconsider `acceptLoop:`).** `acceptLoop:` (native, `sockets.rs`) is **serial**: it runs the
  block to completion and *closes the accepted socket* before accepting the next, breaking only
  on a non-local exit (`^^`) from the block. That's great for tests/fixtures (see
  `qnlib/tests/24-server.qn`) and simple request-response, and its win is ergonomic (automatic
  socket cleanup on return/throw/cancel + a clean `^^` break). But it's a **footgun for real
  servers**: the natural "spawn a Task per connection" instinct silently breaks under it (the
  socket is closed the instant the spawning block returns, so the deferred handler writes to a
  dead socket, and a per-task `^^`/throw never reaches the loop). Concurrent serving belongs one
  layer up: `TcpServer` (`qnlib/core/tcp_server.qn`) built on manual `accept` + `Task.spawn:`
  (each task owns/closes its socket), with **external** termination via cancelling the
  accept-loop task (cancel aborts even a parked `accept`). To capture in the eventual docs: the
  layering (`accept`/`acceptOnce:`/`acceptLoop:` = thin native primitives; `TcpServer` = the
  blessed concurrent server), and a decision on whether `acceptLoop:` should stay once `TcpServer`
  is standard or at least cross-reference it. A *native* concurrent-accept variant is probably not
  worth it — the pure-Quoin `TcpServer` already nails it, and the native surface is better kept thin.
- [ ] **Write-mode file streams + `seek:`.** `[IO]File` streams are read-only today
  (`OpenFile` opens read-only). Add a write/append mode (a `mode` on `OpenFile`, or a
  separate selector) and `seek:` on a file-backed `ByteStream` — a file-only op (sockets
  aren't seekable; the first real reason to consider a file-stream subclass).
- [ ] **IPv6 `[host]:port` parsing.** `parse_host_port` (`src/runtime/sockets.rs`) splits on
  the last `:`, so a bracketed IPv6 literal (`[::1]:8080`) mis-parses. Handle the bracket form.
- [x] **Structured `IoError` class.** Socket/stream/file errors now throw a typed Quoin
  `IoError` (an `Error` subtype carrying a `kind` symbol + `message`), so `catch:`-by-type and
  `e.kind == #connectionRefused` work. New `QuoinError::Io { kind, message }` + `IoErrorKind`
  (`src/error.rs`) maps the backend `IoError { kind, message }` (and closed-handle / unexpected-EOF
  cases) to the class at the `catch:` boundary (`vm.quoinerror_to_value` / `make_io_error`); the
  per-module string `raise_io` helpers are retired in favour of `return Err(QuoinError::io*(..))`.
  From Quoin: `IoError.throw:msg kind:k`. `quoinerror_to_value` is now exhaustive over domain
  variants so a future typed error can't silently fall through to a string. *Remaining string I/O
  sites, deferred to later error tranches:* the `parse_host_port` bad-host/port and the
  `ByteStream`/`StringStream` UTF-8 / empty-delimiter cases (a `ValueError` / `ParseError` tranche),
  and the `unexpected I/O result` internal-invariant guards.
- [ ] **`Bytes` extras.** A mutable `BytesBuilder` (if concat churn shows up — body assembly
  is `bytes + chunk` today). (The `#b'HEX'` byte literal moved to `## Syntax`.)
- [x] **(Separate, larger track) Polyglot extension system.** Out-of-process extensions over a
  unix-domain socket — design in `docs/FUTURE_EXT_ARCH.md`. Shipped: Tier 0 (gc-free `Host`
  trait) + Tier 1 transport, structured values, host-reach, crash/timeout isolation, and
  extension-backed classes — INCLUDING the Python-SDK class-registration parity slice (3b:
  `sdk/python/quoin_ext`, `ext_vector.py` example), so Rust + Python SDKs are at parity
  (`crates/quoin-ext`, `crates/quoin-ext-proto`, `src/runtime/extension.rs`). The optional
  Phase-3 residue — a fuller Arrow C Data Interface for columnar interchange — is tracked
  where its forcing function lives: `crates/adbc/DESIGN.md` §7 (the columnar `Table` value).
  Remaining refinements below.
  - [ ] **Extension calls: fair queuing instead of a busy error.** *(audit follow-up, PR #48.)*
    A connection serves one top-level call at a time (a single request/response socket, no
    request ids); a second *concurrent* call now fails fast with a catchable "extension busy"
    error (`in_flight` guard, `src/runtime/extension.rs`) instead of interleaving frames and
    desyncing the socket. The nicer behavior is to **queue** a waiting caller — park it on the
    connection, wake it when the in-flight call finishes — so `Async.gather:` over one
    long-lived `[ADBC]Connection` just works. Needs a per-extension waiter queue + park/wake
    (mirrors the channel park model in `src/runtime/channel.rs`). Same-task re-entry (the
    extension re-entering itself through a host block) must still error — only cross-task
    contention queues.
  - [ ] **Extension socket files leak on abnormal *host* exit.** *(audit follow-up, PR #48.)* An
    extension's socket file (`/tmp/quoin-ext-*.sock`) and child are torn down by
    `NativeExtension::drop` (and promptly on a timeout via `kill_now`), but if the *host* crashes
    or is killed, `Drop` never runs and the socket file persists. Add a startup sweep of stale
    `quoin-ext-*.sock` files (own-pid-tagged so a concurrently-running host isn't clobbered), or
    place them under a process-scoped temp dir removed on a best-effort atexit/panic hook.

## REPL (`qn repl`)

An interactive read-eval-print loop. Bootstrap it in Rust (a new `VmRunnerMode::Repl`), but
design toward eventually re-implementing the loop in Quoin once the enabling primitives land
(`eval:bindings:`, the `eval:` parse-panic fix, stdin line reading). Existing building blocks:
`try_parse_quoin_string_named` (`Result`, not panic — distinguishes *incomplete* from *invalid*
input), `compile_and_run_asts` (execute + capture `VmStatus::Finished(val)`), `highlight_to_ansi`
(input highlighting), `annotate_error` (pretty errors w/ source snippets), persistent
`vm.globals`, `[IO]Handle.stdin`.

**Key design decisions to settle first:**
- **Persistent locals.** `vm.globals` already persists across lines (class defs, `Uppercase`
  consts), but `eval`/run uses `parent_env: None`, so lowercase `x = 5` would not carry. Pick:
  (a) keep a persistent REPL `EnvFrame` injected as each line's parent env and accumulate
  bindings (the "right" model, and what [[eval-bindings]] in §8 enables), vs (b) auto-promote
  top-level bindings to globals (simpler, but bends the uppercase=global/lowercase=local rule).
- **Meta-command prefix.** `:` collides with keyword-message selectors. Need a leading sigil that
  can't begin a valid Quoin expression (e.g. `\`, `%`, or a `:` only honored as the first char).
- **Line-editor crate.** No readline dep today. `rustyline` (mature, batteries-included:
  history/Highlighter/Completer traits) vs `reedline` (nicer multiline + live highlighting,
  heavier). Needed for P1; choose before then.

**P0 — MVP (a genuinely usable REPL): DONE** (branch `feat/repl`; design in `docs/REPL_DESIGN.md`).
- [x] `qn repl` wiring: `VmRunnerMode::Repl` + dispatch in `runner.rs`. Boots one VM with the
  prelude loaded, kept alive across the loop; native-class registration extracted to a shared
  `register_builtins`.
- [x] The loop: read a line → `try_parse` → compile → run sharing persistent globals → print the
  result (`=> <value>`), prompt `qn> `.
- [x] **Persistent state** across lines: a `repl_env` on `VmState` (GC-rooted), reused as each
  line's frame env via `VmState::execute_repl_line`; each line's compiler is seeded with the
  session's binding names (`Compiler::new_with_locals`) so references resolve as locals, not globals.
- [x] **Graceful recovery**: parse (`try_parse`), compile, and runtime/`throw` failures are shown
  (`annotate_error`) and the loop continues; `execute_repl_line` resets frames/stack/active-exception
  to the baseline after an error.
- [x] **Multiline continuation**: incomplete input re-prompts (`... `), detected by the `try_parse`
  error being positioned at end-of-input. (Abandoning an in-progress multiline buffer is deferred to
  a P1 readline keybinding; for now exit/complete the input or Ctrl-C.)
- [x] Exit on `Ctrl-D` (EOF) and `$quit`/`$exit`; bonus `$help`, `$reset`.
- Known P0 limitations (see design doc): synchronous eval only (top-level async needs the scheduler
  driver); plain stdin (editing/history is P1); result uses `Display`, not `.s` (P1).

**P1 — ergonomics:**
- [x] Line editing via `rustyline` (cursor movement, kill/yank, multiline via its `Validator`).
  Interactive tty only; piped/redirected stdin falls back to a promptless accumulation loop (which
  also covers the P2 `qn repl < script` case). Helper traits are hand-impl'd (no `derive` feature).
- [x] **Abandon an in-progress multiline buffer** with Ctrl-C (rustyline `Interrupted` → drop the
  input, fresh prompt; Ctrl-D exits).
- [x] **History** with up/down recall, persisted to `~/.quoin_history` (load on start, save on exit).
- [x] **Input syntax highlighting** via the rustyline `Highlighter` (reuses `highlight_to_ansi`).
  Guarded by `try_parse`: the highlighter's parser panics on partial input, so an incomplete line
  shows uncolored and colors in once it parses. (Color-while-typing is the refinement below.)
- [x] **Highlight incomplete input by predictive completion.** Done in `quoin-syntax`
  (`complete::complete_source` + `highlight::highlight_resilient`), reusable by the REPL and the
  language server. A context-aware mini-lexer closes open strings/regex/brackets; a placeholder
  operand (`{}` then `0`) completes a trailing operator / keyword selector / `=` / definition op;
  every candidate is verified by re-parsing (first that parses wins — the suffix is cropped away, so
  the choice is irrelevant to the result). `highlight_resilient` parses-or-completes, then returns
  only the spans within the original length. The REPL now colors partial lines as you type.
  - **Caveat (tied to the `Runtime.eval:` parse-panic bug below):** `try_parse` only guards the pest
    step — the AST builder still `unreachable!`s on some pest-valid shapes (e.g. `Foo <-- 0`), so a
    `parse_or_none` wrapper `catch_unwind`s the parse. It recovers (no crash) but the caught panic
    prints its message; a clean fix needs the parser to return a `Result` through AST building.
  - [x] **Trailing postfix dot `a.` heuristic.** `complete_source` now appends a selector
    placeholder (` x`) so input ending in a method-send dot (`a.`/`@x.`/`Foo.`) parses and stays
    colored while the completion popup is open. Shipped with `.`-completion (P2 below).
  - [x] **Trailing range `..` heuristic.** A trailing `..` (range with the RHS not yet typed) is
    already covered by the existing ` 0`/` x` placeholder operands (`1..` → `1 .. 0`); ditto a
    float-in-progress `1.` (→ `1. x`). Confirmed by `trailing_range_operator` in `complete.rs`; no
    new candidate needed (the placeholder set the dot heuristic added already supplies the bound).
- [x] Result pretty-printing: render the result via its `.s` method (honors user `s` overrides;
  e.g. a custom `Point` prints `Point(3, 4)`), falling back to `Display` if `.s` errors. (`=>`
  prefix, nil suppression. Color/truncation still open.)
- [x] `$`-commands: `$type <expr>` (result's class), `$time <expr>` (eval + wall-clock), `$load
  <file.qn>` (run a file into the session), plus the P0 `$help`/`$reset`/`$quit`.

**P2 — power features:**
- [x] Tab completion (`src/repl_complete.rs`). A `CompletionIndex` snapshots the surface metadata
  (globals/locals/namespaces + per-class class-side & instance selectors) once per line from the
  live VM via the `introspect` API — owned/`'static`, so no arena access in the completer; refreshed
  through `editor.helper_mut()` before each `readline` (the VM is frozen during editing, so it's
  never stale). `complete_input(line, pos, &index)` is a pure, unit-tested function of lexical
  context: inside `[ … ]` → namespaces; `recv.` where the receiver's class is statically known →
  class-side selectors (class-name receiver), instance selectors (session local), or instance
  selectors of a syntactically-typed literal — string / integer / `true`/`false` / `nil`, plus the
  `#`-sigil collections & regex (`#(…)`→List, `#{…}`→Map, `#<…>`→Set, `#/…/`→Regex), symbols
  (`#sym`/`#'sym'`), and bare blocks (`{…}`→Block), detected by a string/nesting-aware delimiter
  match. A closed `[ns]` completes the fully-qualified name (`[IO]Fi`→`[IO]File`); else a bare word.
  The rustyline `Completer` (`CompletionType::List`) is a thin adapter. **v1 limit:** only receivers
  whose class genuinely needs evaluation (`@ivars`, `(expr)` groupings, chained sends) yield nothing.
- [ ] **VM introspection API** (`src/introspect.rs`; design in `docs/INTROSPECTION.md`). Read-only
  surface metadata as plain owned structs (no `'gc`), owning the VM-internal walking so the REPL /
  completion / a future Quoin `Mirror` stay ignorant of internals. Exact: `globals` /
  `describe_class` / `describe_value` / `session_locals`; prefix finds: `find_globals` /
  `find_namespaces` / `find_selectors`. Consumed by the `$`-commands and tab completion above.
  - [x] **`$`-introspection commands.** `$globals [prefix]` (→ `introspect::globals`, classes flowed
    + values as `name: Class`), `$class <Name>` (→ `describe_class`: header `Parent <- Name (mixins…)
    [flags]`, ivars, own + class-method signatures via `introspect::signature`), `$inspect <expr>` (→
    eval then `describe_value`: value repr + `(class X)` + `@field: Class` lines). Built on a generic
    `eval_value(arena, input, render)` (HRTB `render` closure) with `eval_repl_input`/`_type`/`_inspect`
    as the three renders; `format_globals`/`format_class`/`format_inspect`/`flow_names` helpers.
  - [ ] Later: the Quoin `Mirror` wrapper (native reflection class converting the structs to Quoin
    objects) — a layer over this API, not part of it.
- [x] Startup file: `~/.quoinrc` is run into the session on interactive REPL boot (shell-style:
  not for piped scripts or `qn -e`); errors are reported but non-fatal. Banner suppressible with
  `QN_NO_BANNER`; prompt overridable with `QN_PROMPT` (default `qn> `). (`QN_*` to match the binary
  and the `tuning` stress knobs.) The shared arena setup is factored into `build_repl_arena`.
  - [ ] **Evaluate `~/.quoinrc` against a `QuoinRepl` object** (`self` bound to a fresh `QuoinRepl`
    instance) so the rc can both define helpers *and* act as a config file — calling setter methods
    (`.prompt: 'λ> '`, `.banner: false`, …) on `self` to configure the session, plus overriding
    `QuoinRepl` methods to customize behavior. This generalizes the env-var knobs above into a
    first-class, scriptable config surface. Needs a native `QuoinRepl` class exposing the REPL's
    settings (a sibling/consumer of the introspection API and the future `Mirror`).
    - [ ] Once `QuoinRepl` exists, **drive the prompt from it** (`self.prompt:`), superseding
      `QN_PROMPT` — initially a read-once-at-startup value is fine (a live/dynamic prompt can come
      later). Same for the banner and any other knobs that graduate from `QN_*` env vars.
- [x] One-shot eval `qn -e '<expr>'`: evaluates one expression in a fresh prelude-loaded session and
  prints its `.s` result (a `nil` result prints nothing); parse/compile/runtime errors go to stderr
  with a non-zero exit, so it composes in pipelines. Non-interactive `qn repl < script` (pipe mode)
  already works via the promptless accumulation loop.

**P3 — "REPL in Quoin" (the eventual goal):** migrate the loop into `qnlib` once its primitives
exist — depends on [[eval-bindings]] (`eval:bindings:`, §8), the `Runtime.eval:` parse-panic fix
(Bugs below), and an `[IO]Stdin` line-read helper. The Rust REPL is the bootstrap; move pieces over
incrementally (read+eval+print loop first, then meta-commands, then editing if a Quoin-side line
editor ever exists).

## Standard Library

A modular stdlib backlog — mostly self-contained native classes (often a thin wrapper over a Rust
crate) plus `qnlib` glue, sized to pick up between larger changes. **⭐ = small, self-contained,
foundational** (good first picks). Suggested crates are noted where one fits; for small formats a
hand-rolled parser may be preferable to taking on a dependency's surface. Cross-refs: the async/net
primitives live under `## Networking & Async I/O`, and reflection-over-the-*running*-program is the
deferred `Mirror` in `## REPL`.

**Numbers & math** — all four shipped; see `docs/STDLIB_NUMBERS.md` for the build record.
- [x] ⭐ **Math** — number methods + `Math` namespace + `closeTo:` test assertion.
- [x] **Decimal** — `BigDecimal` (`src/runtime/big_decimal.rs`, `rust_decimal`).
- [x] **BigInt** — `BigInteger` (`src/runtime/big_integer.rs`, `num-bigint`); distinct type,
  no auto-promotion (the settled §5 decision).
- [x] **Statistics** — `qnlib/core/07-statistics.qn` (pure qnlib over the collection protocol).

**Data formats & serialization** — parsers/generators all shipped; see
`docs/STDLIB_DATA_FORMATS.md` for the build record.
- [x] **JSON** — parse/generate with pretty option.
- [x] ⭐ **base64 / hex** — `Bytes` ↔ `String` codecs.
- [x] **CSV** — read/write with quoting/escaping.
- [x] **TOML**/**YAML** (config)
- [x] **MessagePack** (binary, pairs with `Bytes`).
- [ ] **Custom serialization** — a configurable way for users to serialize non-trivial types
  (`DateTime`, `BigDecimal`, custom classes, …) into the structured formats, rather than the
  Phase-1 behavior of erroring on anything outside the core value tree. Design a serialization
  protocol/hook — e.g. an opt-in `toData`/`asJson`-style method, or a registry mapping a class to
  a `DataValue` shape — so a `DateTime` serializes as an RFC 3339 string, a user class as a chosen
  Map, etc., with control over the round-trip. (Cross-ref `docs/STDLIB_DATA_FORMATS.md` — the
  `DataValue` bridge is the natural seam for this.)

**Text & presentation**
- [x] **Pretty-printer** — structural, width-aware rendering of nested collections/objects
  (Wadler/Leijen-style groups + line breaks). Wire into the REPL result display for large values;
  console width is already plumbed (`VmOptions.console_width`).
- [ ] ⭐ **ANSI / color** — public terminal styling API (`[Term]`/`Color`): colors, bold/underline,
  `NO_COLOR`-aware. (The internal `ansi_colorizer` is for highlighting; this is the user-facing one.)
- [ ] **Logging** — leveled logger (`debug`/`info`/`warn`/`error`), formatting, pluggable sinks,
  defaulting to `[IO]Stderr`.

**Time** — Phases 1+2 shipped (`docs/STDLIB_TIME.md`); the civil-types residual remains.
- [x] **DateTime** — zone-aware instants, RFC 3339, components, arithmetic (`jiff`).
- [x] **Duration & monotonic clock** — `Duration` value type + `Instant.now`/elapsed.
- [ ] **Civil `Date`/`Time` + first-class `Span`/`Period`** — ISO-8601 duration parsing
  (`P1Y2M…`), mixed-unit arithmetic, calendar diffs (the `plus…`/`minus…` methods cover the
  common case today; see `docs/STDLIB_TIME.md` deferred).

**Crypto & hashing**
- [ ] ⭐ **Digests** — `sha256`/`sha1`/`md5`/`blake3` + HMAC over `Bytes`/`String` (`sha2`, `blake3`).
- [ ] ⭐ **UUID** — v4 (random) and v7 (time-ordered) (`uuid`).
- [ ] **Secure random** — CSPRNG bytes (pairs with Random/UUID). Later: symmetric (AES-GCM) and
  signatures (Ed25519).

**Compression & archives**
- [x] ⭐ **gzip / zlib / deflate** — one-shot (de)compression as `Bytes` methods
  (`decodeGz`/`encodeGz`, `decodeDeflate`/`encodeDeflate`) via flate2's miniz_oxide backend
  (pure Rust). `src/runtime/compress.rs`. Remaining: streaming (incremental) over a `ByteStream`.
- [x] ⭐ **zstd** — decode via `ruzstd` (pure Rust) as `Bytes.decodeZstd`. Encode deferred (no
  pure-Rust zstd compressor — would need the C `zstd` crate). `src/runtime/compress.rs`.
- [ ] **tar** (`tar`) and **zip** (`zip`) — archive read/write.

**System & process**
- [ ] ⭐ **Environment** — read/iterate/set process env vars (`[OS]Env`).
- [ ] ⭐ **Path** — `join:`/`dirname`/`basename`/`extension`/`normalize`/`isAbsolute?` (string-level
  path manipulation, separate from `[IO]File`).
- [ ] **Process / subprocess** — spawn a command, capture stdout/stderr/exit; async-aware (parks on
  the scheduler like socket I/O).
- [ ] ⭐ **`[IO]Stdin`** — line/byte reading. Also unblocks P3 "REPL in Quoin" (see `## REPL`).
- [ ] **CLI argument parsing** — options/flags/positionals/subcommands on top of
  `VmOptions.arguments`.

**Networking** (built on the async arc — see `## Networking & Async I/O`)
- [x] **HTTP client (high-level)** — `[HTTP]Client.get:`/`post:`/`request:` builder over
  sockets + parser + TLS (`qnlib/net/http.qn`, PR #14: content-encoding, unified body/JSON,
  streaming, redirects). Remaining refinements (keep-alive pooling, cookies) tracked under
  `## Networking & Async I/O`.
- [x] ⭐ **URL** — `[Web]Url` parse/build + percent/query/form codecs (`qnlib/web/`,
  tests/47-url.qn).
- [ ] **DNS** resolution (async) and **WebSocket** (over the HTTP upgrade) — later.

**Metaprogramming**
- [ ] **Parser / AST to Quoin** — expose the parser and a visitable AST as Quoin objects so Quoin
  code can read/transform source. Foundation for macros; companion to the deferred REPL `Mirror`
  (reflection over the running program) and `Runtime.eval:`.

**Concurrency** (on the async scheduler)
- [x] **Channels** — buffered/unbuffered async queues with scheduler park/wake
  (`src/runtime/channel.rs`, PR #10; the extension fair-queuing item above mirrors its park
  model). Remaining in this family: the structured-concurrency API itself (nurseries,
  deadlines, detached spawn+join — `docs/ASYNC_ARCH.md` Stage 2b).

## Bugs/Odd Behavior
- [x] **`List.new` / `Map.new` / `Set.new` produce broken shells.** FIXED for the
  collections (+ `Bytes.new`): native class methods now win the hierarchy lookup before the
  generic fallback — `new` constructs the real empty native value; `new:` raises a clear
  catchable error ("List has no instance fields — construct with `#()` …"). Tests:
  `classNewConstructs` in 41-list/40-maps/15-sets/22-bytes. **Residual (general trap):** any
  OTHER native-payload builtin reachable via the `NewNoBlock`/`New` fallbacks still mints a
  shell (e.g. namespace-ish classes); a general fix needs a per-class "has native constructor"
  marker consulted by the fallbacks — deferred. Original report: The generic `Callable::New`
  instantiation path builds a plain `Object` of the class *without* the `NativeState` payload, so
  the very first native method call on it fails with the internal `"Not a native state of the
  requested type"` error (`value.rs` `with_native_state`): `List.new.add:1` errors; so do
  `Map.new.at:put:` and `Set.new.add:`. Never noticed because qnlib and every bench construct
  collections via literals (`#()`, `#{}`, `#< >`), which build the native payload. Found writing
  the control-flow-inline v2 tests (2026-07). Fix direction to decide: make class-side `new` on
  the native collection classes construct the real native value (nice UX, matches `List.of:`), or
  refuse instantiation with a clear "use `#()`" error — silently minting a poison object is the
  one wrong behavior. Repro: `var l = List.new; l.add:1`.
- [x] **Operator precedence was inverted for arithmetic.** In the pest Pratt parser (`src/parser/pest/parser.rs`), `+`/`-` bound *tighter* than `*`/`/`/`%`, and `..` bound tighter than all arithmetic (`2 + 3 * 4 == 20`; `2 .. 3 + 1` errored as `(2..3) + 1`). Fixed by reordering the `.op(...)` levels to the conventional ordering — loosest→tightest: `||` · `&&` · `== !=` · comparison · `~` · `..` · `+ -` · `* / %`, with postfix `.method` tighter than any infix and prefix tightest. Now `2 + 3 * 4 == 14` and `2 .. n + 1` is `2 .. (n + 1)`. Full `qnlib` test suite passes (0 regressions); docs updated (`docs/language/01-foundations.md` §6 and appendices A/C).
- [x] **`-->` / `->` didn't override a same-signature method.** Both appended a variant to the selector's multimethod chain; equal-specificity ties resolved to the *first-defined*, so a plain redefinition (`Foo <- { bar -> { 1 } }; Foo <-- { bar --> { 2 } }`) was dead code and `bar` returned `1`. The originally-planned fix (reverse the equal-specificity tie-break) turned out **wrong** — it breaks ordered guard dispatch (the `dispatchByBlock` test relies on first-defined guards winning over a later `.class==Object` catch-all). Fixed instead by **replace-at-definition**: a new *unguarded* variant whose `param_types` match an existing unguarded variant replaces that variant's block in place (`replace_or_append_method_in_chain`, `src/vm.rs`); guard- and type-differentiated variants still append and dispatch by specificity. `Foo.new.bar` now returns `2`; full suite passes; regression test `overridesSameSignature` added; docs updated (`docs/language/03-objects.md` §10/§13, appendix C). **Known limitation:** overriding a *guarded* variant with an identical guard does not replace (guards aren't compared for equality) — subsumed by the scoring overhaul below.
- [x] **`Runtime.eval:` panics on a syntax error instead of throwing.** A syntactically-invalid source string panicked the whole VM in the pest parser (`parse_quoin_string_named` → `crates/quoin-syntax/src/pest/parser.rs`, which unwraps the `PestError`) rather than surfacing a catchable error, so `{ Runtime.eval:'1 +' }.catch:{…}` aborted the process instead of recovering. Found during the structured-error work (Tranche 4b): the typed `ParseError` raised by `compile_and_execute_source` (`runtime.rs`) only covered *semantic* compile failures of already-parseable source; the parse step upstream still panicked. **Fixed:** `compile_and_execute_source` now calls the already-existing fallible `try_parse_quoin_string_named` and maps its `ParseError` to `QuoinError::ParseError` — sidestepping the feared `quoin-syntax` signature ripple (the panicking entry stays for the main-program path, which fails the process by design). One call site changed; `eval:`/`eval:self:`/`use` all recover now. Test: `evalSyntaxErrorIsCatchable` (`qnlib/tests/07-errors.qn`). Done as the debugger-v0 prerequisite (evaluate-in-frame must not crash on a malformed watch expression).
- [x] **`Runtime.eval:self:` did not expose `self` (or `@ivars`) to the eval'd code.** Despite the name,
  the `self_val` argument was *not* bound as the eval'd unit's `self`: `Runtime.eval:'self' self:obj` returned
  `nil`, and `Runtime.eval:'@total' self:obj` returned `nil` (not the receiver's field). Found during the
  debugger's eval-in-frame work (Slice 3c). **Root cause:** `compile_program` (`compiler.rs`) emitted a top-level
  `Push(Nil); DefineLocal(self)` for *every* program unit — correct for a real top-level program, but it clobbered
  the receiver that `start_block_as_method` had bound when the unit ran as a method (`eval:self:`). **Fixed:**
  added `Compiler::compile_program_with(program, define_self)`; `compile_and_execute_source` passes
  `define_self = self_val.is_none()`, so an eval-with-receiver skips the `self = nil` init and `self`/`@x`/
  `self.method` resolve (`self` still compiles as a local via `is_local`'s special case). Plain `eval:` / `use`
  keep `self == nil` at top level — byte-for-byte unchanged. The debugger's compound `$print @total + 1` /
  `$print self.method` now work with no debugger-side change. Test: `evalSelf` (`qnlib/tests/99-misc.qn`). The
  remaining *locals* gap is the separate [[eval-bindings]] (`eval:bindings:`) item.
- [x] **A multibyte char in a string literal leaked a stray `'` into the value.** Any non-ASCII glyph in a
  string literal (e.g. `µ`, U+00B5) appended the closing quote to the parsed value — `'µ'.length` was 2,
  `'café'` became `café'`, and `#ANSI'…µs…'` rendered a spurious apostrophe. Root cause was *not* the
  colorizer (`src/ansi_colorizer.rs` is UTF-8-safe) but the lexer: `parse_primary_expr`
  (`crates/quoin-syntax/src/pest/parser.rs`) extracted literal bodies with `raw.substring(1, raw.len() - 1)`
  — the `substring` crate is **char**-indexed while `raw.len()` is **byte** length, so on multibyte content
  the end index overshot and (clamped) swallowed the closing `'`. Affected both `string_expr` and
  `user_string_expr` (`#ident'…'`). Fixed by byte-slicing between the single-byte `'`/`#` delimiters
  (`&raw[1..raw.len() - 1]`), which is char-boundary-safe. The `us`→`µs` workaround in `qnlib/test.qn` was
  reverted. Regression test: `multibyteLiterals` (`qnlib/tests/08-strings.qn`).
- [ ] **Native re-entry through `execute_block` can still overflow the machine stack
  (uncatchable SIGBUS).** *(audit follow-up, PR #48.)* `call_method`/`call_method_value` now
  cap native→Quoin re-entry depth (`VmState.native_reentry_depth`, per-task, saved/restored
  across task switches), so a self-referential `==:`/`hash`/comparator that re-enters a native
  op raises a catchable error instead of aborting the process (`src/vm.rs`,
  `tests/native_recursion.rs`). `execute_block` is **deliberately not** capped: lazy generator
  pipelines legitimately compose blocks many levels deep on the native stack (the Generators
  suite nests past any machine-stack-safe fixed cap), so a low cap there would break real
  programs. A block-based infinite re-entry (a sort comparator that re-sorts its own receiver,
  an `each:` body that re-iterates it) can therefore still SIGBUS. A real fix needs a
  stack-remaining check (stacker-style `maybe_grow`) or a larger/growable coroutine stack,
  rather than a fixed depth counter that conflates pathological recursion with deep-but-finite
  legitimate nesting.
- [ ] **Extension `DataValue` depth cap is decode-side only — the host encode path is still
  unbounded.** *(audit follow-up, PR #48.)* Decoding a deeply nested `DataValue` *from* an
  extension is capped (`MAX_DV_DEPTH = 64`, catchable `DecodeError` —
  `crates/quoin-ext-proto/src/lib.rs`), which stops a buggy/hostile extension from overflowing
  the host stack. The **symmetric** path — the host serializing a deeply nested Quoin value to
  *send* (`value_to_data` + `encode_dv`, via `call:…data:` or a `MakeValue`/`ReadHandleReturn`
  reply) — still recurses without a bound, so a deep value built in Quoin can overflow the host
  on the way out. Cap it the same way: a depth check in `value_to_data` (or a fallible
  depth-bounded `encode_dv`) raising a catchable error.

## 1. Class & Method Definition Semantics
- [x] **Class Creation (`<-` operator)**:
  - Implement AST compilation for `IDENTIFIER <- BLOCK` expressions. This should define a new `Value::Class` and store it in `globals`.
  - The block body must be executed with the new Class object as the default receiver (`self`).
  - Declare instance variables using the block's parameters (e.g. `| @x @y |` inside the class definition block).
- [x] **Class/Instance Extension (`<--` operator)**:
  - Implement `IDENTIFIER <-- BLOCK` behavior. This adds new methods to either a Class meta-object or a specific object instance (singleton/eigenclass methods).
- [x] **Method Definitions (`->`) and Overrides (`-->`)**:
  - `SELECTOR -> BLOCK`: Define a new method on the current subject. Raise an error if it already exists.
  - `SELECTOR --> BLOCK`: Override an existing method. Raise an error if it does not exist.
  - Support normalize selectors for operator symbols (e.g., mapping `#'-'` to `-`, `#'+:'` to `+:`).
- [x] **Class Meta-object (`.meta`)**:
  - Implement a `.meta` method on `Class` to retrieve/define class-side (static/constructor) methods.

## 2. Object Instantiation & Instance Variables
- [x] **Instantiation Block Syntax (`.new:`)**:
  - Support `Class.new: { ... }`.
  - The block must run in the context of the newly created instance. Instance variable names (without the `@` prefix) are bound as local variables or directly assignable inside the block to initialize fields.
- [x] **Instance Variables (`@variable`)**:
  - Support reading/writing instance variables via the `@` prefix in method definitions.
  - Map field names to their storage on the `Object` struct.

## 3. Mixins & Multiple Inheritance
- [x] **Mixin Registration (`.mix:`)**:
  - Implement `.mix:CLASS` to copy or link behaviors from a mixed-in class.
- [x] **Mixin Method Resolution**:
  - Update `lookup_method` in the VM to search through mixed-in classes (depth-first or breadth-first) before checking parent classes.

## 4. Advanced Method Dispatch (Multimethods / Argument Types)
- [x] **Typed Block Arguments**:
  - Support parameter type checking inside block headers: `| name:Type |`.
- [x] **Method Overloading**:
  - Resolve messages by matching both the selector name *and* matching the types of the arguments passed at runtime.
  - E.g., `split: -> { |pat:String| ... }` vs `split: --> { |p:Regex| ... }` must dispatch correctly depending on whether the argument is a `String` or a `Regex`.

## 5. Non-Local Returns (`^^` operator)
- [x] **Method-level returns (`^^`)**:
  - Implement the `^^` return operator.
  - When a block executes `^^ value`, it must return from the enclosing method that created the block.
  - This requires closures (`Block`) to hold a reference to their creator's stack frame, and the VM to unwind frames up to that context.

## 6. Exception Handling & unwinding (`catch:` and `throw`)
- [x] **Throwing Exceptions**:
  - Support `.throw` and `.throw:` on objects.
- [x] **Catches**:
  - Support `.catch:{ ... }` blocks.
  - The VM must unwind execution frames back to the nearest enclosing catch block when an exception is thrown.

## 7. Namespaces
- [x] **Namespaced Globals**:
  - Support namespaced identifiers like `[IO]Stdout` or `[IO]Folder`.
  - The compiler and VM must parse, store, and look up namespaced globals.

## 8. Built-in Core Library Extensions
- [x] **Boolean & Nil Logic**:
  - Implement `if:`, `else:`, `if:else:`, and `not` purely as methods on the `true`, `false`, and `nil` objects in `bootstrap.qn`, rather than using VM-level jump instructions.
- [x] **IO Library**:
  - Implement native classes under `[IO]` namespace: `[IO]Stdout`, `[IO]Stderr`, `[IO]Handle`, and `[IO]Folder`.
- [x] **System Utilities**:
  - `Timer.time: { ... }`: Computes elapsed time in milliseconds.
  - `Runtime.evalFile: filename`: Loads, compiles, and evaluates a file.
  - `Object.s` overrides: Overriding `s` string representation when converting objects to strings for printing.
  - [x] **`eval:bindings:` (eval with an explicit environment).** `Runtime.eval:'expr' bindings:#{…}`
    seeds the map's entries as locals in the eval'd frame, so the expression can reference them by name.
    Deliberately *explicit* (not implicit lexical capture). **Implemented:** the binding names are passed to
    `Compiler::new_with_locals` so references compile to `LoadLocal` (not `LoadGlobal`), and the values are
    bound into a parent `EnvFrame` attached to the eval block (`build_block_with_env`) that `LoadLocal` walks
    into — names interned to the same `Symbol`s the compiler emits. Composes with the `eval:self:` fix: the
    debugger's `debug_eval` passes the focus frame's `self` *and* its locals as bindings, so `$print @total + n`
    (mixing an `@ivar` and a local) resolves. Test: `evalBindings` (`qnlib/tests/99-misc.qn`); the debugger
    end-to-end. (`compile_and_execute_source` / `eval_string` now thread `bindings: &[(Symbol, Value)]`.)
- [x] **Native State Support**:
  - Implement native classes holding arbitrary Rust state inside VM objects.

## 9. Performance Tuning
- [x] **Alternative Parser Architecture Evaluation**:
  - Evaluate replacing ANTLR with Tree-sitter for faster full-file compiles using its compiled C engine.
  - Assess native Rust parser generators (e.g., LALRPOP or Pest) or hand-writing a recursive-descent parser for optimal compiler performance.
- [ ] **Method-dispatch optimization** — live rollup in `profiling/status.md` (the authoritative
  before/after + next-options doc). Original baseline (`profiling/dispatch-cache/notes.md`): the Send path
  was malloc-dominated (37.8% self), `lookup_method` 21.5% inclusive (13.6% walk + 2.7% scoring). Since
  then the bounded wins (caching, hashing, allocator, per-step/per-send allocs) **and** the first
  structural swing (superinstructions) have landed — cumulatively ~65-78% faster than the start-of-session
  baseline. The resolution-side levers are spent (IC ruled out); remaining headroom is structural.
  - [x] **Selector interning.** Replaced `Instruction::Send(String, …)` with an interned
    `Symbol(&'static str)` (Eq/Hash by pointer, lock-free `as_str()`; global leak-forever interner in
    `src/symbol.rs`). Kills the per-Send `selector.clone()` (~8.8% `String::clone`) and gives the dispatch
    cache a `Copy`, collision-free, pointer-stable key. Scoped to selectors in the dispatch path; method
    tables / globals stay `String`-keyed for now (the walk resolves via `as_str()`).
  - [x] **mimalloc** as the default global allocator (separate session) — absorbed much of the
    allocation cost; the biggest single help on the alloc-bound benchmark (Binary Trees).
  - [x] **FxHash** on the dispatch `method_cache`, replacing SipHash on the hot resolution key.
  - **Inline call-site cache — BUILT, MEASURED, RULED OUT (reverted).** A recv-class-monomorphic IC on
    top of FxHash was a net regression (flat Fib/Sieve, +4.6% Binary Trees): post-FxHash the global cache
    is already cheap, and the probe is paid on *every* send while only single-untyped guard-free *user*
    sends qualify (the hot typed-multimethod arithmetic/indexing pays it for nothing). Parked on branch
    `experiment/inline-cache`; full reasoning in `profiling/inline-cache/notes.md`. **Don't rebuild** unless
    dispatch becomes hash-bound again.
  - [x] **Superinstructions.** Fuse the hot `<operand-load>; Send` instruction pairs
    (`Push`/`LoadLocal`/`LoadField` → `SendConst`/`SendLocal`/`SendField`) so one dispatch-loop step does
    the operand-load + send. Pairs chosen from a dynamic histogram of executed bytecode (~3.9M of the hot
    sends). A `fuse_bytecode` peephole pass (`src/compiler.rs`) runs per block at compile time: never fuses
    across a jump target, recomputes relative jump offsets + keeps the source map index-aligned; the `Send`
    body is extracted to `exec_send` (shared by the fused handlers), and the stack-trace formatter reads
    the selector from the fused forms. **~8-10%** (release best-of-4: Fib 21→19, Sieve 52→47, Binary Trees
    769→707). See `profiling/superinstructions/notes.md`. Cheap follow-ons: 3-instruction sends (fuse the
    receiver load too) and the `Dup → StoreField`/`StoreLocal` assignment pairs.
  - [x] **Method-resolution cache** (the headline). Global `HashMap<MethodCacheKey → Option<Value>>`
    where the key is `(searched-class ptr, selector: Symbol, class_side, n_args, arg_class_ptrs,
    arg_kinds)`. Guard-free resolutions only — `match_score` sets `VmState.dispatch_uncacheable` when a
    guarded candidate is examined; saved/restored around each lookup for re-entrancy (a guard's nested
    sends run their own lookups). Pointer keys are safe because named classes are globals-rooted;
    eigenclasses carry `Class.is_eigenclass` and are excluded (receiver or any arg). The per-arg
    `Value`-variant **kind** byte is required so a `Class` value and an instance of that class (same
    `get_class_for_lookup` pointer, different `type_name`-based dispatch) don't collide. Invalidated by
    `invalidate_method_cache()` (`clear()`) at the method-table mutation sites: both method-def handlers,
    `register_native_class`, and the class-unregister path. Cache is traced (holds `Gc` method `Value`s).
    **Result: Fib −28% / Sieve −22% / Binary Trees −17%; walk+scoring collapse into hits.** Validated
    incl. `QN_GC_STRESS=1`; invalidation pinned by `dispatchCacheInvalidatesOnNewMethod`. See
    `profiling/dispatch-cache/notes.md`.
  - [x] **`Callable` enum (kill the per-Send `Box<dyn Callable>`).** Replaced the `Callable` trait + 5
    structs + 5 impls with one `#[derive(Copy, Clone)] enum Callable<'gc>` (5 variants of `Gc`/`NativeFunc`)
    and an inherent `call(self, …)` that `match`es; `lookup_method`/`call_method_value` return/build
    `Option<Callable>` instead of `Option<Box<dyn Callable>>`. Dispatch now resolves+invokes with no heap
    alloc. No `Collect` (transient, like the `Box`); `no_gc_across_yield` did not fire. Measured: malloc
    self-time 34.3% → 32.6%, locks 7.3% → 6.2%; wall-clock within noise (the `Box` was a cheap short-lived
    alloc — the real allocation volume is the per-call `EnvFrame` HashMap + per-step instruction clone).
    Net code simplification; unblocks caching a `Copy` `Callable` (vs the method `Value`) later. See
    `profiling/callable-enum/notes.md`.
  - [x] **Local variables — Step A: Symbol-keyed env.** `EnvFrame.vars: HashMap<String,Value>` →
    `Vec<(Symbol, Value)>` (linear scan, pointer-compared `Symbol`s); `LoadLocal/StoreLocal/DefineLocal`
    carry `Symbol`. Killed the per-frame HashMap alloc/drop (the ~3.6% hashbrown teardown is gone), the
    var-name `String` clones, and the per-access local SipHash (~2.6% → 1.1%). Capture model unchanged
    (closures still capture via `EnvFrame.parent`). **Result: malloc 32.6% → 27.7%; Fib −13% / Sieve
    −22% / Binary Trees −18%.** All green incl. `QN_GC_STRESS=1`. See `profiling/local-var-symbols/notes.md`.
  - [ ] **Local variables — Step B (optional): full `(depth, slot)` resolution.** Replace the per-frame
    `Vec<(Symbol,Value)>` with a `Box<[Value]>` sized to the slot count; compiler resolves every reference
    to `(depth, slot)` (incl. a reserved `self` slot + capture depth + frame sizing). Buys: smaller env
    allocs (no `Symbol` storage) + O(1) index access + the shared `Slots` primitive with instance vars.
    Step A already captured the big win, so B's remaining upside is modest (same per-frame alloc *count*,
    Vec→Box). **Only pursue if a later profile shows `EnvFrame::get`/the env alloc still hot.** Blocker for
    B: the `%{}` string interpolation captures caller locals by name (`string.rs`) — see the "lower it to
    String concat" item in §8; do that between A and B if we proceed.
  - [ ] **Follow-up: migrate method tables to `Symbol` keys.** After the cache lands, rekey
    `Class.instance_methods`/`class_methods` (and ideally `globals`/`NamespacedName`) from `String` to
    `Symbol`, turning the per-class `methods.get(selector)` SipHash into an integer hash. Ripples through
    the ~150 native `instance_method("name", …)` builder sites and `register_native_class`; do it as its
    own pass once dispatch caching is in.
  - [ ] **Follow-up: unify the symbol caches.** Reconcile the new compile-time selector interner
    (`Symbol(&'static str)`) with the existing runtime `symbol_table` (`VmState`, interned `#foo` Quoin
    symbol *values*) so there's a single canonical interning mechanism rather than two parallel ones.
  - [x] **Per-step whole-instruction clone.** `step_internal` used to deep-clone the whole `Instruction`
    every step (`frame.block.bytecode.get(ip).cloned()`), allocating for every heap-carrying variant.
    Fixed by cloning only the bytecode `Rc` (refcount bump, no alloc) into a local and taking a
    `&Instruction` into it — `inst` borrows the local `Rc`, not `self`, so handlers keep `&mut self`.
    GC-safe (`Instruction` is `'static`/no `Gc`, bytecode immutable). Match arms use reference patterns
    (Copy operands get a `let x = *x;` deref-shadow; the few class-def-time owned sites `.clone()`).
    **Result: malloc 30.6% → 27.3%, `String::clone` 4.3% → 1.9%; Fib −11% / Sieve −8% / Binary Trees
    −13%.** All green incl. `QN_GC_STRESS=1`. See `profiling/instr-borrow/notes.md`.
  - [x] **Phase 3 — per-Send allocation cleanups.** (A) `lookup_method`'s `selector_key`
    (`NamespacedName::new(.., selector)`, a `String` alloc every send) made lazy via
    `lookup_selector_in_globals` — only the globals-fallback path (method not found) allocates it now.
    (B) the per-send `last_send_args = args.clone()` removed; args are captured only on the in-place-error
    branches (lookup-`Err`/MNU in the Send handler, `New`/`NewNoBlock`/`Native` in `Callable::call` — the
    native case reuses the existing `active_native_args` rooting snapshot, so the hot path adds nothing).
    Removed the dead write-only `last_send_receiver`. Stack traces diffed byte-identical before/after.
    **Result: malloc 30.4% → 22.9% (−7.4pts, the largest single drop); Fib −6% / Sieve −5% / Binary
    Trees −4%.** All green incl. `QN_GC_STRESS=1`. See `profiling/phase3-send-allocs/notes.md`.
  - [ ] **Make stack-trace/error formatting unit-testable (data, not direct print).** `annotate_error`
    (`vm.rs`) builds the stack trace by formatting strings inline (selector + arg types + source location
    per frame). Extract the *structured data* — a `Vec` of per-frame records (selector, arg class names,
    location) — from the string rendering, so the data can be asserted in Rust unit tests. This gives
    Phase 3 (B) a real regression test: that an error raised from a 1+-arg send still reports its args
    correctly (today only the `.qn` `07-errors` suite covers this indirectly, by matching printed output).

## 10. Test Coverage
- [ ] **Increase Code Coverage**:
  - Add more integration tests under `qnlib/tests/` to target uncovered parts of the compiler, runtime, and VM.
- [x] **Track Quoin-level (`.qn`) coverage, not just Rust-level.** `cargo cov` / `cargo cov-test`
  (llvm-cov) only measure which *Rust* paths in the VM/runtime are exercised, not whether every
  `qnlib` method is actually called by the suite — a pure-Quoin method can sit untested while Rust
  coverage looks healthy. Implemented in `src/coverage.rs`: `qn test --coverage[=lcov|cobertura]
  [--coverage-out=PATH]` (also on `qn <file>`) records `.qn` line + function coverage by reusing
  the debugger's line-map seam (one bool-load on the hot path when off), attributing hits per
  executing block so defining a method doesn't count its body as run. Emits LCOV or Cobertura XML.
  The first run flagged ~86 of 330 stdlib methods never exercised by the suite. Remaining:
  - [ ] **Branch coverage** — harder than the usual "tag the conditional jumps." In Quoin only
    `&&`/`||` lower to `IfJump`/`ElseJump` (`compiler.rs` `compile_binary_operator`); *every other*
    conditional — `if:`, `if:else:`, `else:`, `whileTrue:`, `whileFalse:`, `ifNil:`, `ifNotNil:`,
    `caseOf:` — is a **message send to a Boolean/nil receiver with block arguments** (the generic
    keyword-send path, `Send(selector, n)`). The branch is the polymorphic dispatch to `True#if:`
    vs `False#if:` deciding whether to run a block. So tagging jumps would catch almost nothing;
    the right model is *a branch is a conditional send where the receiver decides which block-arm
    runs.*
    - **Leverage:** the arms are blocks and we already track per-block hit counts (block-span
      keying), so "did each arm run?" is already measured — branch coverage largely reduces to
      **arm-block coverage**, nearly free.
    - **Gap:** the *implicit not-taken* side of one-armed / short-circuit forms is not a block
      (a bare `cond.if:{a}` runs nothing on false; a loop may never enter; `&&`'s RHS may never
      eval), so block hits alone can't see it. We tick line-starts, not every send, so there is no
      exact per-site execution count to subtract.
    - **(a) Arm coverage — cheap, recommended next:** at the denominator walk, recognize
      branching-selector send sites (`Send` / `SendConst` / fused `SendLocalConst`) and pair them
      with their arm blocks; report each arm covered or not. Also instrument the `&&`/`||`
      `IfJump`/`ElseJump` the classic way. Honest and surfaces dead arms (the high-value finding),
      but reports *arms taken*, not full two-way coverage (the implicit-else stays a blind spot).
      Fiddly part: matching arm blocks through the fusion superinstructions.
    - **(b) Condition-polarity coverage — accurate, later:** a gated send-site hook over a fixed
      table of branching selectors that records the receiver's true/false/nil per site; a branch
      is covered iff both polarities were seen there. Classic branch coverage, uniform across
      one-armed/loops/short-circuit — at the cost of a second hot-path hook, a selector table, and
      per-site identity.
    - Emit as LCOV `BRDA` (and Cobertura `branch` attrs) once a model is chosen.
  - [ ] The denominator is class methods only; file-level/top-level code and test-body blocks (not
    reachable from the class registry) aren't enumerated. Walk loaded program blocks too.
  - [ ] Per-tick filename hashing is fine for opt-in runs but could be interned if coverage ever
    runs by default.

