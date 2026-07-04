# no_borrow_across_yield

### What it does

Flags `RefCell`/`RefLock` borrow guards that are still alive when execution can
cooperatively yield — a literal `yielder.suspend(...)`, or any call that
transitively reaches one (`execute_block`, `call_method*`, `await_io`,
`fiber_resume`, …).

### Why is this bad?

A yield suspends the coroutine with the guard alive on its Rust stack. Any other
borrow of the same cell — from another task, or re-entrantly from the Quoin code
the suspended call was running — panics `RefCell already borrowed`.

This is a different hazard from `no_gc_across_yield` (its sibling): a GC value is
dangerous if *used after* the yield, but a guard is dangerous merely by being
*not yet dropped* at the yield. So this lint reasons about drop scopes, not uses,
and it is `deny` by default (the codebase is expected to stay clean).

Three shapes are detected:

1. **Named guard live across a yield** — its scope (or explicit `drop`) ends
   after a yield point:

   ```rust
   let g = class.borrow();
   vm.call_method(mc, v, "s", vec![])?;   // guard still alive here
   drop(g);
   ```

2. **Scrutinee temporary** — an `if let`/`while let`/`match` scrutinee temporary
   lives through the success branch / all arms (the edition-2024 rescope only
   shortened the `else` path). This is how the VM's `finalize_instantiation` held
   a class borrow across `init` execution:

   ```rust
   if let Some(m) = clz.borrow().instance_methods.get("init").copied() {
       self.call_method_value(mc, receiver, m, "init", Vec::new())?;  // borrow live!
   }
   ```

3. **Guard temporary inside a yield-capable call** — alive for the whole call:

   ```rust
   vm.call_method(mc, cell.borrow().val, "s", vec![])?;
   ```

Use instead: copy what you need out of the borrow so the temporary dies at the
end of its own statement, or `drop(guard)` before anything that can yield:

```rust
let m = clz.borrow().instance_methods.get("init").copied();  // temp dies here
if let Some(m) = m {
    self.call_method_value(mc, receiver, m, "init", Vec::new())?;
}
```

### Running

`cargo lint` at the workspace root runs every dylint library (this one is
registered in the root `Cargo.toml`'s `[workspace.metadata.dylint]`); violations
fail the command. `cargo test` inside this directory runs the `ui/` test.

If you alternate between `cargo lint` and single-library invocations
(`cargo dylint --lib no_borrow_across_yield`), run `rm -rf target/dylint/target`
in between: the incremental cache is keyed on the code but not on the registered
lint set, and rustc ICEs ("unstable fingerprints") on the mismatch.

### Known problems

Span-based and per-crate. Closure bodies are analyzed as their own functions,
but calls *through* closures/function pointers are not edges in the yield graph
(in this codebase the yielding paths are all reachable through direct calls). A
guard moved into a struct or returned ends liveness tracking silently, and a
guard created and yielded-over inside a single non-scrutinee statement
(`let x = (cell.borrow().v, yielding());`) is missed.
