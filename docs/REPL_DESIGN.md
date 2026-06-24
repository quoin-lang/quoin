# Quoin REPL — design

An interactive read-eval-print loop, invoked as `qn repl`. Bootstrapped in Rust (a new
`VmRunnerMode::Repl`) and designed so the loop can later migrate into Quoin once its enabling
primitives land. Roadmap and priorities live in `QUOIN_TODO.md` (§ "REPL (`qn repl`)"); this
doc is the architecture.

## Settled decisions

1. **Persistent state model.** Globals (`Uppercase` consts, class defs) already persist across
   lines via `vm.globals`. Lowercase locals persist via a single long-lived **`repl_env`**
   (an `EnvFrame`) that is reused as the *frame env* of every evaluated line — so a top-level
   `x = 5` (which lowers to `DefineLocal`/`StoreLocal`) binds straight into it and is visible on
   the next line. The REPL does **not** promote locals to globals; it does not pollute the global
   namespace.
2. **Meta-command prefix `$`.** A line whose first non-space char is `$` is a REPL command, not
   Quoin (`$` is unused by the grammar, so pasted Quoin code never collides). E.g. `$help`,
   `$reset`, `$type <expr>`.
3. **Line editor: `rustyline`** (P1). Mature, synchronous, batteries-included; its `Validator`
   trait maps onto our incomplete-input check, and `Highlighter`/`Completer` onto the existing
   highlighter and future completion. (P0 uses plain stdin; rustyline lands in P1.)

## Building blocks (already in the tree)

- `try_parse_quoin_string_named(src, name) -> Result<Node, ParseError>` — non-panicking parse.
  Sidesteps the `Runtime.eval:` parse-panic bug *and* drives multiline detection.
- `VmState::execute_block(mc, block, args, self) -> Result<Value, QuoinError>` — the synchronous
  "run a top-level block, return its value" primitive (the core behind `eval:`/`use`).
- `fuse_bytecode` runs inside `compile_program`, so REPL lines get the same superinstructions.
- `prelude_asts()` — the `qnlib` core, loaded by running its ASTs.
- `VmState::annotate_error` — pretty errors with source snippets + color.
- `highlight_to_ansi` — input highlighting (P1).
- `[IO]Handle.stdin` — for the eventual Quoin-native loop (P3).

## Architecture

### Lifecycle
`qn repl` → `VmRunner::run_repl`. One `Arena<VmState>` is created and **kept alive across all
lines** (unlike file mode, which tears down after one run):

1. Register the native classes (the same ~30 `register_native_class` calls as file mode — extract
   to a shared `register_builtins(mc, vm)` to avoid duplication).
2. Run `prelude_asts()` to load `qnlib` core (synchronous; class defs).
3. Create `repl_env = EnvFrame::new(None)` and store it on `VmState` (see below).
4. Enter the loop.

### Persistent locals: `repl_env` on `VmState`
`start_block` always allocates a *fresh* child `EnvFrame`, so its top-level binds vanish when the
frame pops. To persist them we add:

```rust
// VmState
pub repl_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,   // traced; None outside the REPL
```

and a REPL entry point that runs a line's block in that env instead of a fresh one:

```rust
pub fn execute_repl_line(&mut self, mc, block) -> Result<Value, QuoinError>
```

It pushes a top-level `Frame` whose `env` is `repl_env` (reused, not a child), drives
`step_internal` to completion exactly like `execute_block`, and returns the final value. Because
the frame env *is* `repl_env`, `DefineLocal`/`StoreLocal` bind into it and `LoadLocal` reads from
it — locals persist line-to-line. `repl_env` lives on the GC root (`VmState`), so it survives
between `arena.mutate_root` calls.

### One iteration
1. **Read** a logical input: read a physical line; if it doesn't yet parse (see multiline), keep
   reading and appending until it does.
2. **Meta-command?** If the trimmed input starts with `$`, dispatch it (no VM call) and continue.
3. **Evaluate** inside `arena.mutate_root`:
   - `try_parse_quoin_string_named(input, "<repl>")`.
     - `Err(ParseError)` whose span is at EOF → **incomplete**: signal the reader to keep reading.
     - `Err(ParseError)` otherwise → **syntax error**: format and show, reset the buffer.
     - `Ok(node)` → compile (`compile_program`, which fuses) → `build_block` → `execute_repl_line`.
   - `Ok(value)` → **print** `=> <value.s>` (suppress a bare `nil`, optionally colorized).
   - `Err(e)` → `annotate_error(e)` and print; the REPL stays up.
4. **Loop.** Exit on EOF (Ctrl-D) or `$quit`/`$exit`.

### Multiline / continuation
Incomplete input is detected from the `try_parse` error: a pest error positioned at the end of
input (expecting more tokens — unbalanced `{`/`(`, trailing binary operator, open string)
indicates "keep typing." The reader accumulates lines and re-parses after each, switching the
prompt `qn> ` → `... ` while continuing. A genuine syntax error (positioned mid-input) is shown
immediately and the buffer is reset. P1's rustyline `Validator` implements exactly this contract
(`ValidationResult::Incomplete` vs `Invalid`/`Valid`).

### Error recovery
Three failure points, none of which may crash the loop:
- **Parse**: handled by `try_parse` returning `Err` (never panics).
- **Compile**: `compile_program` returns `Err(String)` → wrap as a `ParseError`, show, continue.
- **Runtime / uncaught throw**: `execute_repl_line` returns `Err(QuoinError)` (incl. the
  `Uncaught exception` case) → `annotate_error` + show, continue.

### Meta-commands (`$`)
A small dispatch on the word after `$`. P0 ships `$help`, `$quit`/`$exit`. P1 adds `$reset` (drop
and recreate `repl_env`; optionally clear non-builtin globals), `$type <expr>` (eval, print
`.class`), `$load <file>` (run a `.qn` file into the session), `$time <expr>`.

### Result printing
`=> ` + the value's `.s` (the in-language string form, so user `s` overrides are honored), color
optional (respect `supports_color`). A bare `nil` result (value-less statement) is suppressed or
dimmed. Huge collections may be truncated (P1).

## P0 scope and limitations
- **Synchronous eval only.** `execute_repl_line` runs on the current fiber; a line that awaits
  top-level I/O (`Async.gather:`, an HTTP call at the prompt) needs the async scheduler driver
  (the `block_on` + `FuturesUnordered` loop in `compile_and_run_asts`). Deferred: P0 covers the
  synchronous majority (defs, calls, arithmetic, inspection); top-level async is a follow-on that
  swaps `execute_repl_line` for a scheduler-driven variant.
- **Plain stdin** (no editing/history) until rustyline lands in P1.

## Path to a Quoin-native REPL (P3)
The Rust loop is the bootstrap. Migrating it into `qnlib` needs three primitives, each already
tracked: `eval:bindings:` (persistent locals as injected bindings — the in-language analogue of
`repl_env`), the `Runtime.eval:` parse-panic fix (so a Quoin loop can `catch:` bad input), and an
`[IO]Stdin` line-read helper. With those, the read/eval/print core becomes a short `.qn` program;
meta-commands and (if ever) editing follow. Keep the P0/P1 Rust pieces thin so each maps to one of
those primitives.
