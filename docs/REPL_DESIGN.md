# Quoin REPL — design

*Status (verified 2026-07-09 at `dbe188d`): **SHIPPED (P0–P2)**. `qn repl` lives in
`src/runner_repl.rs` on rustyline — editing, history, multiline, syntax highlighting,
`$`-commands (`$help`/`$reset`/`$type`/`$quit`) and tab completion (`src/repl_complete.rs`), plus
`qn -e` and `~/.quoinrc`. The note below that "P0 uses plain stdin; rustyline lands in P1" is
stale — it landed. **Not built:** P3, migrating the loop itself into Quoin, and the `Mirror` API.*

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
pub fn begin_repl_line(&mut self, block) -> (usize, usize)   // start the line as task #0
pub fn end_repl_line(&mut self, mc, base_frames, base_stack, succeeded) -> Value
```

`begin_repl_line` pushes a top-level `Frame` whose `env` is `repl_env` (reused, not a child) and
returns the frame/stack baseline. The caller (`run_scheduled_line` in `runner.rs`) installs it as
scheduler **task #0** (`install_main_task`) and drives it to completion through the *shared*
scheduler (`drive_main_task` — the same `block_on` + `FuturesUnordered` loop the file runner uses),
then `end_repl_line` takes the result and restores the baseline. Because the frame env *is*
`repl_env`, `DefineLocal`/`StoreLocal` bind into it and `LoadLocal` reads from it — locals persist
line-to-line. `repl_env` lives on the GC root (`VmState`), so it survives between
`arena.mutate_root` calls. Running under the scheduler is what lets a REPL line do async I/O,
`Async.sleep`, spawn `Task`s, and resume fibers (iterators included).

### One iteration
1. **Read** a logical input: read a physical line; if it doesn't yet parse (see multiline), keep
   reading and appending until it does.
2. **Meta-command?** If the trimmed input starts with `$`, dispatch it (no VM call) and continue.
3. **Evaluate** inside `arena.mutate_root`:
   - `try_parse_quoin_string_named(input, "<repl>")`.
     - `Err(ParseError)` whose span is at EOF → **incomplete**: signal the reader to keep reading.
     - `Err(ParseError)` otherwise → **syntax error**: format and show, reset the buffer.
     - `Ok(node)` → compile (`compile_program`, which fuses) → `build_block` → `run_scheduled_line`.
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
- **Runtime / uncaught throw**: the scheduler drive returns `Err(QuoinError)` (incl. the
  `Uncaught exception` case), already source-annotated by `step` → show, continue. `begin_repl_line`
  resets transient scheduler state, so an error mid-fiber can't corrupt the next line.

### Meta-commands (`$`)
A small dispatch on the word after `$`. P0 ships `$help`, `$quit`/`$exit`. P1 adds `$reset` (drop
and recreate `repl_env`; optionally clear non-builtin globals), `$type <expr>` (eval, print
`.class`), `$load <file>` (run a `.qn` file into the session), `$time <expr>`.

### Result printing
`=> ` + the value's `.s` (the in-language string form, so user `s` overrides are honored), color
optional (respect `supports_color`). A bare `nil` result (value-less statement) is suppressed or
dimmed. Huge collections may be truncated (P1).

## Scope and limitations
- **Async at the prompt: done.** Every REPL line (and `qn -e` / `~/.quoinrc`) runs through the
  same scheduler the file runner uses, so top-level I/O (`Async.gather:`, an HTTP call), a
  `Async.sleep`, a spawned `Task`, or a fiber/iterator resume all work at the prompt. The
  scheduler driver was extracted from `compile_and_run_asts` into the shared `install_main_task` +
  `drive_main_task`; the REPL seam is `run_scheduled_line` (replacing the old synchronous
  `execute_repl_line`). Each line is its own task #0, run to completion.
- **Plain stdin** (no editing/history) until rustyline lands in P1.

## Path to a Quoin-native REPL (P3)
The Rust loop is the bootstrap. Migrating it into `qnlib` needs three primitives, each already
tracked: `eval:bindings:` (persistent locals as injected bindings — the in-language analogue of
`repl_env`), the `Runtime.eval:` parse-panic fix (so a Quoin loop can `catch:` bad input), and an
`[IO]Stdin` line-read helper. With those, the read/eval/print core becomes a short `.qn` program;
meta-commands and (if ever) editing follow. Keep the P0/P1 Rust pieces thin so each maps to one of
those primitives.
