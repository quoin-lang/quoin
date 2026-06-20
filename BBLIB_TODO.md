# BuildingBlocks Runtime & Library TODO List

This document outlines the language features, compiler updates, and VM modifications required to execute the BuildingBlocks standard library (`bblib`) files and test suites.

## Misc
- [x] Change the file extension to `.bub` everywhere.
  - Don't forget to update the plugin.
- [ ] Get rid of `Value::Native`, it's only used by the global funcs and those are only used for testing.
  - In the BB language itself all methods are attached to a class.
- [ ] Support checking `assertMeetsRequirements:` in calls to `mix:`/`can:`.
  - Implement `Class#can?:SELECTOR`. Now that `Symbol` is a real interned type (`#sym`),
    `can?:` should accept either a symbol or a string and normalize (e.g. via `.s`) to a
    selector name before lookup.
- [ ] Implement the class-marker methods (currently parsed and callable but no-ops / missing):
  - `sealed!` — currently a no-op (`src/runtime/class.rs`). Should forbid further extension of the class or mixin (no more `<--`, `->`/`-->`, `.mix:`), raising a clear error on attempts.
  - `abstract!` — new marker, like `sealed!` but for construction: forbid instantiating the class itself via `new`/`new:` (a concrete subclass may still be instantiated). Raise a clear error, e.g. `Cannot instantiate abstract class X`.
- [ ] Overhaul method dispatch with hierarchy-distance scoring. Replace the pairwise `compare_specificity` (`src/vm.rs`) — which returns `Equal` for incomparable types (e.g. `Integer` vs `String`) and so isn't a total order, making the stable sort fragile — with a per-candidate score: for each argument, the class-hierarchy distance from the actual class to the declared param type (exact = 0, each hop up the hierarchy = +1, untyped = a max sentinel); sum across args; among guard-passing candidates, lowest score wins. More robust, enables ambiguity detection, and would let same-guard variant overrides be handled uniformly (the current `replace_or_append_method_in_chain` only covers *unguarded* redefinitions).
- [x] Implement the `#< … >` set literal. Added a native `Set` type (`src/runtime/set.rs`, `NativeSetState`) mirroring `List`/`Map`: insertion-ordered, unique by `==:`, with `count`/`add:`/`remove:`/`contains?:`/`each:`/`s`/`==:`; `Set` mixes in `Iterate` and gets `union:`/`intersection:`/`difference:`/`subset?:`/`superset?:` in `bblib/02-iterate.bub`. Literal compiles via a new `NewSet(n)` instruction (deduped by `==:`). The closing `>` collided with the greater-than operator, so the grammar now excludes `>`/`>=` from set elements (`set_elem`/`set_infix_op` in `BuildingBlocks.pest`) — a bare `>` ends the set; parenthesize to use `>` in an element. Tests in `bblib/tests/15-sets.bub`; docs updated.
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
    - Added `Generator` (yield-block as iterable) and a fiber-backed external `Iterator` (`hasNext?`/`next`) in `bblib/02-iterate.bub`.
- [ ] Rewrite the TestSuite so it doesn't mix the tests into itself, too many conflicts.
- [ ] List, Regex and Map #bind:{}
  - [x] List#bind:{}
  - [ ] Regex#bind:{}
  - [ ] Map#bind:{}
  - See bblib/presentation/20-method-destructuring.bub
- [ ] Think about a better destructuring protocol than assuming `#at:` exists.
- [x] Confirm `%'string%{eval}' is working.
  - [ ] Make sure it's optimized into string concatenation by the compiler.
- [x] Make sure case statements are tested and working.
- [x] Make the `^>` yield operator usable in expression position.
  - Moved `yield_return` from `stmt` to `primary` in the pest grammar; it now works anywhere an expression does (e.g. `a = ^> v`), with greedy operand precedence matching `Fiber.yield:` (parenthesize to scope). ANTLR grammar (legacy/unused path) left as-is.
- [ ] Have the `LoadGlobal` instruction consult the `BuiltinCache`. Currently it always does a `HashMap<NamespacedName, Value>` lookup against `globals` (see `vm.rs` `Instruction::LoadGlobal`); builtin classes (`Fiber`, `List`, `Integer`, etc.) could be served from the cache to avoid hashing the name on every load (e.g. for the `^>` -> `Fiber.yield:` lowering). `BuiltinCache` may need to be keyed more generally by name to cover all builtins.
- [ ] Repurpose the Yeet instruction and make sure .../???/!!! are all working.
- [x] Formalize an interface for BB error types.
  - `Error` base (`message`/`payload`, class-side `throw:`/`throw:payload:`) + core subtypes (`TypeError`, `ArgumentError`, `MessageNotUnderstood`, `ArithmeticError`, `IndexError`) in `00-bootstrap.bub`. Catch-by-type via `case`/`~`.
  - Runtime now raises structured errors: `BBError::Thrown` marker (value rides in `active_exception`), and `vm.buberror_to_value` maps internal `BBError` variants to typed BB `Error` objects at the `catch:` boundary. `does:throw:` widened to match by value/type or message string.
  - [ ] Future: give the VM more fine-grained internal error variants and route more raise sites through typed BB errors.
- [ ] Implement DateTime.
- [ ] Implement Decimal.
  - rust_decimal crate
- [x] Make sure #symbol types are working.
- [ ] Language server
  - [ ] VSCode plugin
- [x] Integrate fff into agy
  - https://github.com/dmtrKovalenko/fff#mcp-server
- [ ] Write a document fully explaining the language semantics, including all corner cases.
  - Capture the subtle/surprising behaviors here as they surface so they can be folded into the doc.
  - **`new:{}` block initialization & lexical scope.** Instance variables are *not* pre-bound inside a `new:{}` block, so an empty `new:{}` leaves every field at its default (`nil`) — it does **not** silently capture a same-named variable from the surrounding scope. Only an explicit assignment binds a field. The right-hand side of such an assignment resolves up the lexical chain (so `{ x = x }` copies the enclosing `x` into the field), but the assignment itself binds in the block's own frame and never mutates the enclosing variable. Corollary: a plain-assignment `init:` like `init: -> {|a| @a = a }` is redundant — field population already sets `@a` from the block before `init:` runs — so it behaves identically to the default no-op `init`.
  - **`init`/`init:` run the whole chain.** `new`/`new:{}` invoke the initializer of every class in the hierarchy (ancestors and mixins included), base→derived, with `init:` preferred over `init` per class. A derived `init:` no longer shadows/skips an ancestor or mixin `init`.

## Bugs/Odd Behavior
- [x] **Operator precedence was inverted for arithmetic.** In the pest Pratt parser (`src/parser/pest/parser.rs`), `+`/`-` bound *tighter* than `*`/`/`/`%`, and `..` bound tighter than all arithmetic (`2 + 3 * 4 == 20`; `2 .. 3 + 1` errored as `(2..3) + 1`). Fixed by reordering the `.op(...)` levels to the conventional ordering — loosest→tightest: `||` · `&&` · `== !=` · comparison · `~` · `..` · `+ -` · `* / %`, with postfix `.method` tighter than any infix and prefix tightest. Now `2 + 3 * 4 == 14` and `2 .. n + 1` is `2 .. (n + 1)`. Full `bblib` test suite passes (0 regressions); docs updated (`docs/language/01-foundations.md` §6 and appendices A/C).
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
  - Implement `if:`, `else:`, `if:else:`, and `not` purely as methods on the `true`, `false`, and `nil` objects in `bootstrap.bub`, rather than using VM-level jump instructions.
- [x] **IO Library**:
  - Implement native classes under `[IO]` namespace: `[IO]Stdout`, `[IO]Stderr`, `[IO]Handle`, and `[IO]Folder`.
- [x] **System Utilities**:
  - `Timer.time: { ... }`: Computes elapsed time in milliseconds.
  - `Runtime.evalFile: filename`: Loads, compiles, and evaluates a file.
  - `Object.s` overrides: Overriding `s` string representation when converting objects to strings for printing.
- [x] **Native State Support**:
  - Implement native classes holding arbitrary Rust state inside VM objects, following [native_rust_state_plan.md](file:///Users/damon/code/building_blocks_vm/native_rust_state_plan.md).

## 9. Performance Tuning
- [x] **Alternative Parser Architecture Evaluation**:
  - Evaluate replacing ANTLR with Tree-sitter for faster full-file compiles using its compiled C engine.
  - Assess native Rust parser generators (e.g., LALRPOP or Pest) or hand-writing a recursive-descent parser for optimal compiler performance.

## 10. Test Coverage
- [ ] **Increase Code Coverage**:
  - Add more integration tests under `bblib/tests/` to target uncovered parts of the compiler, runtime, and VM.

