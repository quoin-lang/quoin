# Quoin Runtime & Library TODO List

This document outlines the language features, compiler updates, and VM modifications required to execute the Quoin standard library (`qnlib`) files and test suites.

## Misc
- [ ] Use a proper arg parsing library instead of the `VmRunnerMode` stuff in `runner.rs`.
- [ ] Design an installer.
  - [x] Named the language **Quoin** (extension `.qn`); rationale in `~/code/quoin/DECISIONS.md`.
  - [x] Binary name is `qn` (set via `[[bin]]` in `Cargo.toml`).
  - [ ] Support installing the binary and support files to `/usr/local/bin` or something.
  - [ ] Create a more general purpose way of determining what to load by default on start.
- [ ] Support importing files explicitly.
  - [ ] When the installer work is done, search for files in standard locations + wherever the binary is installed.
- [x] Change the file extension to `.qn` everywhere.
  - [x] Don't forget to update the plugin.
- [x] Get rid of `Value::Native`, it's only used by the global funcs and those are only used for testing.
  - In the Quoin language itself all methods are attached to a class.
- [x] Wire `assertMeetsRequirements:` into `mix:` so a mixin can declare requirements its host class must satisfy.
  - [x] Implemented `can?:` (`src/runtime/object.rs`), overloaded by argument: a Symbol/String selector asks "does the receiver implement that method?" (instance/class methods for instance/class receivers, class-side for metaclass); a Class asks "is-a / mixes in?". Removed the `.can:` alias for `.mix:` to disambiguate (`.can:` call sites converted; obsolete `can?: -> {|clz| clz == Iterate}` defs removed). To make `ClassName.meta.can?:` reachable, a metaclass (`ClassMeta`) receiver now falls through to `Object`'s instance methods in dispatch (`src/vm.rs`) — i.e. metaclasses act as if they subclass `Object` (gaining `can?:`, `s`, `==:`, …). Tests in `qnlib/tests/17-can.qn`.
  - [x] `mix:` enqueues the mixin's class-side `assertMeetsRequirements:host` (if defined) as a **deferred call** that runs at the end of the host's definition block — added a general frame-level defer mechanism (`DeferredCall`, `Frame.defers`, run on *normal* block completion in the Return handler, `src/vm.rs`). Defers run *before* the frame is popped, so the queue stays GC-rooted via `self.frames` even if a defer yields (a collection during the suspension would otherwise free Values reachable only through the defer). Regression tests: `test_deferred_call_values_survive_collection` (Rust) and `yieldFromDeferredMixinCheck` (`qnlib/tests/13-fibers.qn`). Deferring to block-end means required methods may be defined *after* the `.mix:` (the universal idiom). On failure the class is unregistered (`Frame.unregister_on_defer_failure`, seeded by `pending_class_def`) so a class with unmet requirements is never left registered. `test.qn` switched from the undefined `implements?:` to `can?:`. Tests: `qnlib/tests/05-classes.qn` (mixinRequirements). Subclassing needs no separate check — a subclass inherits a parent that already passed.
  - [ ] (Future) Expose the defer mechanism to Quoin source as a user-facing `defer` form.
- [ ] Implement the class-marker methods (currently parsed and callable but no-ops / missing):
  - `sealed!` — currently a no-op (`src/runtime/class.rs`). Should forbid further extension of the class or mixin (no more `<--`, `->`/`-->`, `.mix:`), raising a clear error on attempts.
  - `abstract!` — new marker, like `sealed!` but for construction: forbid instantiating the class itself via `new`/`new:` (a concrete subclass may still be instantiated). Raise a clear error, e.g. `Cannot instantiate abstract class X`.
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
- [x] Bring over AnsiColorizer.cs from the old repo.
  - [x] Switch to the colorized test suite runner.
- [x] Bring over Highlighter from the old repo.
- [x] Improve stack trace output. (Similar to the C# output.)
  - [x] Show highlighted block snippets to the right.
- [x] Move to a better iterator design that doesn't require mutability.
  - Iterate now requires only `each:`; `next`/`reset` cursor removed. Re-entrant, nil-safe.
  - [x] Use generators now that the VM supports them.
    - Added `Generator` (yield-block as iterable) and a fiber-backed external `Iterator` (`hasNext?`/`next`) in `qnlib/02-iterate.qn`.
- [ ] Rewrite the TestSuite so it doesn't mix the tests into itself, too many conflicts.
- [ ] List, Regex and Map #bind:{}
  - [x] List#bind:{}
  - [ ] Regex#bind:{}
  - [ ] Map#bind:{}
  - See qnlib/presentation/20-method-destructuring.qn
- [ ] Think about a better destructuring protocol than assuming `#at:` exists.
  - use an Iterator?
- [x] Confirm `%'string%{eval}' is working.
  - [ ] Optimize it into string concatenation by the compiler.
- [x] Make sure case statements are tested and working.
- [x] Make the `^>` yield operator usable in expression position.
  - Moved `yield_return` from `stmt` to `primary` in the pest grammar; it now works anywhere an expression does (e.g. `a = ^> v`), with greedy operand precedence matching `Fiber.yield:` (parenthesize to scope). ANTLR grammar (legacy/unused path) left as-is.
- [ ] Have the `LoadGlobal` instruction consult the `BuiltinCache`. Currently it always does a `HashMap<NamespacedName, Value>` lookup against `globals` (see `vm.rs` `Instruction::LoadGlobal`); builtin classes (`Fiber`, `List`, `Integer`, etc.) could be served from the cache to avoid hashing the name on every load (e.g. for the `^>` -> `Fiber.yield:` lowering). `BuiltinCache` may need to be keyed more generally by name to cover all builtins.
- [ ] Repurpose the Yeet instruction and make sure .../???/!!! are all working.
- [x] Formalize an interface for Quoin error types.
  - `Error` base (`message`/`payload`, class-side `throw:`/`throw:payload:`) + core subtypes (`TypeError`, `ArgumentError`, `MessageNotUnderstood`, `ArithmeticError`, `IndexError`) in `00-bootstrap.qn`. Catch-by-type via `case`/`~`.
  - Runtime now raises structured errors: `QuoinError::Thrown` marker (value rides in `active_exception`), and `vm.quoinerror_to_value` maps internal `QuoinError` variants to typed Quoin `Error` objects at the `catch:` boundary. `does:throw:` widened to match by value/type or message string.
  - [ ] Future: give the VM more fine-grained internal error variants and route more raise sites through typed Quoin errors.
- [ ] Implement DateTime.
- [ ] Implement Decimal.
  - rust_decimal crate
- [x] Make sure #symbol types are working.
- [ ] Language server
  - [ ] VSCode plugin
- [ ] Integrate fff into claude for non-Rust searches
  - https://github.com/dmtrKovalenko/fff#mcp-server
- [x] Write a document fully explaining the language semantics, including all corner cases.
  - Capture the subtle/surprising behaviors here as they surface so they can be folded into the doc.
  - **`new:{}` block initialization & lexical scope.** Instance variables are *not* pre-bound inside a `new:{}` block, so an empty `new:{}` leaves every field at its default (`nil`) — it does **not** silently capture a same-named variable from the surrounding scope. Only an explicit assignment binds a field. The right-hand side of such an assignment resolves up the lexical chain (so `{ x = x }` copies the enclosing `x` into the field), but the assignment itself binds in the block's own frame and never mutates the enclosing variable. Corollary: a plain-assignment `init:` like `init: -> {|a| @a = a }` is redundant — field population already sets `@a` from the block before `init:` runs — so it behaves identically to the default no-op `init`.
  - **`init`/`init:` run the whole chain.** `new`/`new:{}` invoke the initializer of every class in the hierarchy (ancestors and mixins included), base→derived, with `init:` preferred over `init` per class. A derived `init:` no longer shadows/skips an ancestor or mixin `init`.

## Bugs/Odd Behavior
- [x] **Operator precedence was inverted for arithmetic.** In the pest Pratt parser (`src/parser/pest/parser.rs`), `+`/`-` bound *tighter* than `*`/`/`/`%`, and `..` bound tighter than all arithmetic (`2 + 3 * 4 == 20`; `2 .. 3 + 1` errored as `(2..3) + 1`). Fixed by reordering the `.op(...)` levels to the conventional ordering — loosest→tightest: `||` · `&&` · `== !=` · comparison · `~` · `..` · `+ -` · `* / %`, with postfix `.method` tighter than any infix and prefix tightest. Now `2 + 3 * 4 == 14` and `2 .. n + 1` is `2 .. (n + 1)`. Full `qnlib` test suite passes (0 regressions); docs updated (`docs/language/01-foundations.md` §6 and appendices A/C).
- [x] **`-->` / `->` didn't override a same-signature method.** Both appended a variant to the selector's multimethod chain; equal-specificity ties resolved to the *first-defined*, so a plain redefinition (`Foo <- { bar -> { 1 } }; Foo <-- { bar --> { 2 } }`) was dead code and `bar` returned `1`. The originally-planned fix (reverse the equal-specificity tie-break) turned out **wrong** — it breaks ordered guard dispatch (the `dispatchByBlock` test relies on first-defined guards winning over a later `.class==Object` catch-all). Fixed instead by **replace-at-definition**: a new *unguarded* variant whose `param_types` match an existing unguarded variant replaces that variant's block in place (`replace_or_append_method_in_chain`, `src/vm.rs`); guard- and type-differentiated variants still append and dispatch by specificity. `Foo.new.bar` now returns `2`; full suite passes; regression test `overridesSameSignature` added; docs updated (`docs/language/03-objects.md` §10/§13, appendix C). **Known limitation:** overriding a *guarded* variant with an identical guard does not replace (guards aren't compared for equality) — subsumed by the scoring overhaul below.

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
- [ ] Wildcard selector dispatch.
  - Grab examples from old repo.

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
- [x] **Native State Support**:
  - Implement native classes holding arbitrary Rust state inside VM objects.

## 9. Performance Tuning
- [x] **Alternative Parser Architecture Evaluation**:
  - Evaluate replacing ANTLR with Tree-sitter for faster full-file compiles using its compiled C engine.
  - Assess native Rust parser generators (e.g., LALRPOP or Pest) or hand-writing a recursive-descent parser for optimal compiler performance.

## 10. Test Coverage
- [ ] **Increase Code Coverage**:
  - Add more integration tests under `qnlib/tests/` to target uncovered parts of the compiler, runtime, and VM.

