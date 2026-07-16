# `use` — explicit file loading & packages

Quoin's explicit file-loading construct, which replaced the hardcoded qnlib startup loads
(QUOIN_TODO: "Support importing files explicitly").

**Status: shipped.** Implemented and tested across four commits:
`6d9883a` (stage 1: single-file loads), `ee26f2e` (stage 2: `self:`/named/globs/cycles/`std`-norm),
`69f14b8` (stage 3: prelude via `use core/*`), `377485b` (stage 4: test harness + `evalFile:` removal +
docs). User-facing reference: `docs/language/09-library-and-reference.md` §21. This doc is the
implementation/design record.

> **Context for the plugin update:** the plugin needs the *syntax* — see **Syntax reference** below.
> The short version: `use` is a **soft keyword** (only special at statement start before a path; an
> ordinary identifier everywhere else), followed by an optional `pkg:` qualifier and a `/`-separated
> path of identifier segments, optionally ending in `/*`, optionally terminated by `;`.

---

## TL;DR
`use (pkg:)? path ;` loads a `.qn` file **once**, on demand. `path` is a logical address (`.qn`
implied); the `pkg:` qualifier picks a root (bare/`std:` = stdlib, `self:` = the project, other names =
reserved). `dir/*` globs a directory (UTF-8-sorted). Loading goes through a host-swappable resolver, so
the VM never touches `std::fs` (works on WASM/embedded). The load *path* is decoupled from the `[Ns]`
*namespace* a file's definitions register under.

---

## Syntax reference (for tooling / the plugin)

### Grammar (actual pest rules, `src/parser/pest/Quoin.pest`)
```
stmt = { use_stmt | method_return | block_return | assignment | bang3 | dot3 | huh3 | expr }

use_stmt   = { use_kw ~ use_target }
use_kw     = @{ "use" ~ !IDENT_REST }                 // soft keyword: word boundary
use_target = ${ (use_pkg ~ ":")? ~ use_path }          // compound-atomic: no internal whitespace
use_pkg    = @{ IDENT }
use_path   = @{ IDENT ~ ("/" ~ IDENT)* ~ ("/" ~ "*")? }
```
where (existing rules) `IDENT = @{ IDENT_PREFIX ~ IDENT_REST* }`, `IDENT_PREFIX = [a-zA-Z_]`,
`IDENT_REST = [a-zA-Z0-9?_]`.

### What that means for highlighting/parsing
- **`use` is a soft keyword, not reserved.** It's the `use` keyword only when it begins a statement and
  is immediately followed by a word boundary (`!IDENT_REST`) and a path. So `useThing`, `used`, `use_x`
  are plain identifiers; `use = 5` (assign a variable named `use`) and `x.use` are still valid code.
  A highlighter should treat `use` as a keyword only in the `use <target>` position.
- **Whitespace:** allowed between `use` and the target; **not** allowed *inside* the target
  (`use_target` is compound-atomic). So `use http:io/file` is valid; `use http : io / file` is not.
- **Package qualifier** is `IDENT ":"` — optional. Examples: `std:`, `self:`, `http:`.
- **Path** is `/`-separated `IDENT` segments. Segments are identifier-shaped: start with a letter/`_`,
  then letters/digits/`?`/`_`. **No leading digit, no `-`.** (That's exactly why the core stdlib files
  `00-bootstrap.qn` etc. can't be `use`d by name and are loaded via `use core/*` instead.)
- **Glob:** an optional trailing `/*` (e.g. `use core/*`). Recursive `**` is not implemented.
- **`.qn` is implied** — never written in a `use`.
- **Terminator `;` is optional** — `stmt` is wrapped by `(stmt ~ ";"?)+` in the program/block rules.
  Convention is to write it (Quoin uses `;` heavily to disambiguate line continuation).
- **Value:** a `use` evaluates to `nil` (it's a statement; the VM pushes nil so it nets +1 on the stack).

### Valid / invalid examples
```quoin
use core/*;            "* glob the stdlib core/ dir (sorted)
use io/file;           "* stdlib (default package)
use std:io/file;       "* same unit — std: == bare
use self:helpers;      "* the current project
use self:io/file/*;    "* glob within the project
use http:client;       "* a named package (currently resolves to nothing)
use io/file            "* the `;` is optional

use = 5                "* NOT a use-stmt — `use` is an ordinary identifier here
x = use                "* NOT a use-stmt — reading a variable named `use`
useThing               "* identifier (no word boundary after `use`)
```

---

## Goal & rationale: filesystem-agnostic
Quoin must run where there is no filesystem — **WASM** (only what the runtime provides) and **embedded
in a host app** (the host hands over code units, no FS access). So a load target can't be a filesystem
path; it's a **logical address `(package, path)`** that a host-supplied resolver maps to bytes. Logical
addressing is the only thing that works on every target; it also makes loads location-independent,
sandboxable, and portable.

## Packages (the `pkg:` qualifier)
- **bare** (no qualifier) / **`std:`** → the **stdlib** (the default package). The two are the same
  package — canonicalized to one run-once key so mixing them never double-loads.
- **`self:`** → the **current project**. Replaces filesystem-relative paths (no `./`/`../`):
  project-root-anchored, so the same `self:` path means the same unit from anywhere.
- **`<name>:`** → a named/third-party package. The syntax slot is reserved; resolution is a stub
  (currently → "cannot resolve"). No package manager yet.

## Path vs. namespace are decoupled
- **Path** = the *load address* (where the code lives): `io/file`.
- **`[Namespace]`** = the *logical name* the code registers under (`[IO]File`), declared **inside** the
  file via the existing namespace system.

Independent, conventionally parallel (`io/…` files tend to register `[IO]…`). A file may contain
**anything** — class definitions, extensions (`Object <-- {…}`), mixins, helpers — because `use` only
*loads* it; naming is the namespace system's job. (This is why a class-name target like `[IO]File` was
rejected: no clean class→file mapping for extension/mixin/helper files.)

## Semantics
- **Statement, evaluates to `nil`.** Runs when reached (conditional/late `use` works; most sit at file
  top).
- **Run-once / idempotent**, keyed on the canonical `(package, path)`. The registry is an **ordered
  `Vec`** (`VmState.loaded`), not a set — run order *is* load order (reproducibility + a possible
  `Runtime.loadedUnits` reporting hook). Linear-scan membership is fine (`use` runs at load frequency,
  never in a hot loop).
- **Cycles** are handled by an **in-progress marker**: a unit is appended `InProgress` when its load
  *begins*; a cyclic `use` finds the entry and skips, seeing the partial definitions instead of
  recursing. Marked `Loaded` on completion.
- **Glob loads UTF-8-lexicographically sorted, always** (`FsResolver::list` sorts). Stable, reproducible
  registration order (e.g. for the test harness).
- **Aliasing is not a `use` concern** — it's ordinary assignment of the namespaced globals the file
  registered: `MyFile = [IO]File;`.
- **No per-symbol import/export, no visibility rules.** File-focused by design.

## The load-bearing piece: a package resolver seam
The VM **never touches `std::fs`**. It asks an injected resolver:
```
resolve(package, path) -> Option<source>          // a single unit
list(package, dir)      -> Option<Vec<unit_path>> // a directory, sorted (for globs)
```
Hosts plug in their own:
- **Native CLI** (`FsResolver`) → filesystem-backed. `std`/bare → **`$CWD/qnlib/`**; `self` → **`$CWD`**;
  `<name>` → unknown (stub). (Both roots CWD-relative in dev; an installer relocates the stdlib later,
  and `self_root` can anchor to the entry-point dir.)
- **WASM** → an in-memory map / host `fetch`.
- **Embedded** → packages the host registers programmatically.

`include_dir!`-embedding the stdlib into the binary is deferred; the resolver may later return
precompiled units instead of source.

---

## Implementation map (by file / symbol — line numbers omitted, they drift)
- **Grammar** — `src/parser/pest/Quoin.pest`: `use_stmt`/`use_kw`/`use_target`/`use_pkg`/`use_path`
  (and `use_stmt` is the first alternative of `stmt`).
- **AST** — `src/parser/ast.rs`: `UseNode { package: Option<String>, path: String, glob: bool }` and
  `NodeValue::Use(UseNode)`. The parser arm (`src/parser/pest/parser.rs`, in `parse_stmt`) builds it,
  **stripping the trailing `/*`** into `glob = true`. `parse_quoin_string_named` (new) gives loaded
  units a display name for errors.
- **Instruction** — `src/instruction.rs`: `Instruction::Use { package: Option<String>, path: String,
  glob: bool }`.
- **Compiler** — `src/compiler.rs`: the `NodeValue::Use` arm in `compile_node_internal` pushes
  `Instruction::Use`.
- **VM** — `src/vm.rs`: `VmState.resolver: Box<dyn PackageResolver>` + `VmState.loaded: Vec<LoadedUnit>`
  (constructed in `VmState::new`). The `Instruction::Use` handler in `step_internal` advances ip, calls
  `load_glob` (glob) or `load_unit`, then pushes `nil`.
- **Loading** — `src/runtime/runtime.rs`: `load_unit` (canonicalize pkg → run-once check → resolve →
  append `InProgress` → `compile_and_execute_source` → mark `Loaded`) and `load_glob` (list → sorted
  `load_unit` each). `compile_and_execute_source` + `build_block` are shared with `eval:`.
- **Resolver / packages** — `src/packages.rs`: the `PackageResolver` trait (`resolve` + `list`),
  `FsResolver` (`stdlib_root` = `qnlib`, `self_root` = `.`, `root_for`), `canonical_package`
  (`Some("std")` → `None`), `LoadedUnit { package, path, status }`, `LoadStatus { InProgress, Loaded }`.
- **Module wiring** — `src/lib.rs` declares `pub mod packages;`.

## Highlighting (CLI + plugin parity)
`use` statements are syntax-highlighted by the CLI highlighter (`src/highlighter.rs`); the editor plugin
should match. Colors:

| Element | Light (QuoinDefault) | Dark (QuoinDarcula) |
|---|---|---|
| `use` keyword | `#b5651d` amber, **bold** | `#e0a45a` amber, **bold** |
| package (`std:` / `self:` / `name:`) | `#9a0047` (the namespace hue) | `#d53b82` |
| path / `*` | `#3b6ea5` steel-blue | `#6aa9e0` |

The Rust CLI highlighter implements the **dark** theme: `HighlightType::Keyword` (`#e0a45a;bw`), the
package reuses `HighlightType::Namespace` (`#d53b82`), and `HighlightType::Path` (`#6aa9e0`). Sub-spans
are computed from the statement span (the target is contiguous, the keyword is the first 3 bytes), so no
AST/parser change was needed.

## What shipped, by stage
- **Stage 1** (`6d9883a`) — grammar/AST/instruction/compiler/VM/resolver for single-file bare loads;
  run-once; the resolver seam; `eval_string`/`eval_file` refactored to share `compile_and_execute_source`.
- **Stage 2** (`ee26f2e`) — `self:` + named-package stub; `canonical_package` (bare≡`std`); `dir/*`
  globs (`PackageResolver::list`, sorted); cycle handling verified.
- **Stage 3** (`69f14b8`) — prelude composed in Quoin: the 6 core files moved to `qnlib/core/`,
  `qnlib/prelude.qn` is just `use core/*`, and the runner loads `prelude.qn` + one mode-entry file
  (Test/Benchmark/Run all "prelude + entry"); dropped the `glob` crate use in the runner.
- **Stage 4** (`377485b`) — test harness via registration: `[Test]Suites` global in `qnlib/test.qn`,
  `TestSuite#init:` self-registers, `qnlib/main.qn` = `use test;` + `use std:tests/*;` + run the
  registry. Removed `Runtime.evalFile:`/`evalFile:self:` and `eval_file` (the harness was the only
  caller) — closing the last arbitrary-OS-path read. `eval:` (string eval) stays. Docs + QUOIN_TODO
  updated.

## Resolved decisions
- Stdlib spellable both ways (bare and `std:`), canonicalized to one key.
- `self` root = `$CWD` in dev (parallel to the stdlib at `$CWD/qnlib`); entry-point anchoring is a later
  refinement.
- Glob order = UTF-8 lexicographic, unconditional.
- Cycles handled by the in-progress registry entry (no separate machinery).
- The 19-vs-20 test-suite count change is expected: the old harness double-added `01-iterate`;
  registration + run-once loads each file exactly once.

## Open / deferred

> **Tracked as #108** — Extend use: recursive ** glob, loadedUnits, precompiled units.

- Recursive glob `**`.
- Real package management for `<name>:` (manifest, fetch, versions, lockfiles) — slots in *behind*
  resolution without touching syntax.
- Embedding qnlib via `include_dir!`; precompiled units from the resolver.
- `Runtime.loadedUnits` reporting hook (the ordered registry already supports it).
- Anchoring `self_root` to the entry-point directory (needs the runner to thread the entry path).
