# Extension packaging — delivering an out-of-process extension as a `use`-able folder

*Status (verified 2026-07-09 at `dbe188d`): **SHIPPED**. An extension is a folder with a
`quoin.toml`; `use <name>:*` finds it under a `quoin_packages/` root and synthesizes
`Extension loadPackage: '<dir>'` glue (`src/packages.rs`, `read_package_manifest` in
`src/runtime/extension.rs`). Two packages ship: `quoin_packages/adbc` and `quoin_packages/numpy`.
Tests: `src/packages_tests.rs`, `qnlib/tests/43-adbc.qn`. Package roots are resolved against the
CWD, not the script — see `FsResolver::package_roots`. The v1 scope below is what got built.*

*Update (2026-07-13, `feat/quoin-packages`): the manifest was **renamed `extension.toml` →
`quoin.toml`**, and a package is now any folder with one — `[extension]` (loaded by `use`),
`[lib]` (source units), and/or `[bin]` (executables). The first slice of §9's tooling
shipped: **`qn pkg install DIR` / `qn pkg list`** (`src/runner_pkg.rs`) copy a package into
the per-user home (`$QUOIN_HOME`, default `~/.quoin`) — `$QUOIN_HOME/packages` is now the
third built-in search root — and link `[bin]` entries into `$QUOIN_HOME/bin` for the user's
`PATH`. Tests: `tests/pkg_install.rs`; user docs: book Part X (`docs/language/10-packages.md`).*

*Update (2026-07-13, same branch): **source packages**. `[lib]` (root defaults to the package
dir) declares `.qn` units the resolver serves like `std:`/`self:` files: `use name:*` loads
the synthetic extension unit `*` first (when present) then the lib units sorted; `init.qn`
is never listed (it is `loadPackage:`'s hook). **Decisions:** (A) inside a package's unit
`self:` = that package — a load-context stack in `Modules` rewrites it before resolution, so
run-once keys agree however a unit is reached; (B) a named package's unit that DEFINES a
bare-global class errors at load (reopening allowed) — the §4 no-pollution rule for source;
(C) no versioning — first root wins. Tests: `tests/pkg_source.rs`. Still deferred:
uninstall, registry/fetch/versions, signatures, per-platform binaries.*

Companion to `docs/internal/FUTURE_EXT_ARCH.md` (the extension protocol / runtime) and `docs/internal/USE_ARCH.md`
(the `use` / package-resolution machinery this hooks into).

## 1. Goal

An out-of-process extension (`docs/internal/FUTURE_EXT_ARCH.md`) is currently loadable only by a manual
runtime call — `Extension spawn: '<binary>'` — which spawns the subprocess and installs the classes
it provides. **Packaging** makes a third-party extension a **folder** that loads like any other Quoin
package:

```quoin
use vectors:*;            "spawns the extension, installs its classes (namespaced)"
Vec <- [Vectors]Vector;   "local alias (see §4)"
v = Vec ofFloats: #( 1.0 2.0 3.0 );
```

The author ships a directory; the consumer drops it on a search path and `use`s it. No manual
`spawn`, no knowledge of the wire protocol.

## 2. Why this fits cleanly

The `use` system already does the hard parts (`docs/internal/USE_ARCH.md`, `src/packages.rs`,
`src/runtime/runtime.rs`):

- **`use` executes code on import.** A unit is resolved to source text and *run* in a nested frame.
  So if `use vectors:*` resolves to glue that spawns the extension, the spawn is a normal side effect
  of import — no new "on-import hook" is needed.
- **Resolution is a pluggable seam.** `PackageResolver` (`src/packages.rs`) maps `(package, path)` to
  source; the VM never touches `std::fs`. **Named packages (`vectors:`) return `None` today** — the
  explicit, reserved hook for exactly this (`docs/internal/USE_ARCH.md`: "manifest/fetch/versions slot in
  *behind* resolution without touching syntax").
- **Run-once** is keyed on `(package, path)` → loading a package twice is idempotent (no double-spawn).
- **Class installation already exists.** `install_ext_class` (`src/vm.rs`) installs an extension's
  provided classes as globals at spawn, with names parsed as `NamespacedName` — so namespacing the
  classes is a matter of *which name* the host installs them under (§4).

So the new surface is small: a manifest format, a search path, a `loadPackage:` primitive, and a
package registry.

## 3. The folder + manifest

```
vectors/                 # directory name == the `use` package name (§5)
  quoin.toml         # the manifest
  main.py                # the extension entry (a script or a prebuilt binary)
  README.md              # human docs
```

`quoin.toml` — declarative TOML, so tooling can read it without spawning:

```toml
[package]
name = "vectors"          # canonical/metadata; the directory name is what `use` resolves (§5)
version = "0.1.0"
license = "MIT"
description = "Vector / matrix operations"

[extension]
command = "python3"       # how to launch the subprocess (§6) — v1 is a single command…
args    = ["main.py"]     # …and its args; no per-platform binary matrix yet
namespace = "Vectors"     # optional; defaults to PascalCase of the directory name (§4)
```

The launch follows the existing transport: the host appends the unix-socket path as the child's
final argv (as `Extension spawn:` does today), the child binds it, and the manifest-at-spawn protocol
(`GetManifest` → `ManifestReturn`) reports the provided classes — plus the extension's declared
`lanes` count, after which the host opens that many connections to the same path
(`quoin-ext-proto/PROTOCOL.md` ‡; `ACTOR_OBJECTS.md` §5.1 as adopted for extensions).

## 4. Namespacing — extensions cannot pollute the globals

**Firm rule: every class an extension provides lives under that extension's namespace; a package can
never register a bare global.**

- The namespace comes from the **manifest** (`namespace = "Vectors"`), defaulting to **PascalCase of
  the directory name** if omitted. The *package* owns the namespace, not the extension binary.
- The extension's `ClassDecl`s declare **simple** names (`Vector`, `Matrix`); `loadPackage:` installs
  them as `[Vectors]Vector`, `[Vectors]Matrix`. The extension never knows its package namespace (no
  coupling to the package name), and pollution is **structurally impossible** — the host always
  prefixes.
- A package that declares an **already-namespaced** class name is an **error**: it does not get to
  choose its namespace.
- `use` has **no aliasing** (deliberately skipped in the language). Consumers make a local alias at
  the use site: `Vec <- [Vectors]Vector`.
- **The raw `Extension spawn:` primitive stays the unmanaged escape hatch** — it keeps today's
  behavior (installs `ClassDecl` names verbatim, possibly bare) for dev/testing. The safe,
  namespace-enforcing path is `loadPackage:` / `use`. (So existing `ext_vector` fixtures/tests, which
  install bare `Vector`/`Matrix` via `spawn:`, are unaffected.)

This changes `install_ext_class` **only on the package path**: it takes the package namespace and the
simple `ClassDecl` names and installs `[Ns]Name`.

## 5. Loading — `use name:*` → synthesized `loadPackage:`

The chosen v1 mechanism (over a shipped `init.qn`, see §9):

1. The resolver, extended to handle named packages, finds `vectors/` on the search path (§6) and
   reads `quoin.toml`.
2. It **synthesizes one line of glue as the unit's "source"**:
   `Extension loadPackage: '<absolute-package-dir>';`
   The resolver knows the absolute directory, so it bakes it in — there is no "where am I on disk?"
   problem (§9).
3. The existing resolve → compile → execute → run-once pipeline runs that line.
4. `Extension loadPackage:` (a new class method) reads the manifest, spawns the subprocess per the
   `[extension]` launch spec, installs the provided classes under the package namespace (§4), and
   stores the live `Extension` in the **package registry** (§7).

The directory name is the `use` name (`use vectors:*` ↔ `vectors/`); the manifest `name` is canonical
metadata. Version/compatibility enforcement is deferred.

## 6. Discovery — drop a folder on a search path

v1 as first shipped was **drop-a-folder** — no install step, no registry, no fetch. (An install
step exists now: `qn pkg install` copies a folder to `$QUOIN_HOME/packages`, the third search
root — see the 2026-07-13 status note. Registry/fetch remain deferred.)

- The resolver looks up a named package in **project-local `./quoin_packages/<name>/` first, then each
  dir in `$QUOIN_PATH`** (colon-separated). `FsResolver` (which today knows only `qnlib/` for `std`
  and the cwd for `self`) gains these roots for named packages.
- The launch spec is a **single `command` + `args`** for v1. Per-platform prebuilt-binary selection
  is deferred until distribution exists — it only matters once packages are fetched rather than placed
  by hand.

## 7. Lifecycle — eager spawn + a package registry

- **Eager spawn:** `use vectors:*` spawns the extension immediately. (Lazy-on-first-class-touch is a
  later optimization; eager is simpler and makes extensions far easier to debug — the process exists
  the moment you import it.)
- **Package registry on `VmState`:** keyed by package name, holding the live `Extension`. This roots
  it (kept alive for the session), makes load idempotent alongside `use`'s run-once, gives a teardown
  handle (the child is killed via `NativeExtension::Drop` / the reap queue on exit), and a place for
  `$packages`-style REPL introspection.
- **No auto-respawn** if a package's extension has crashed and you `use` it again — doing that safely
  needs a circuit breaker (avoid spawn-storms against a broken extension). Deferred.

> Note: this depends on the I/O backend persisting across REPL evaluations (an extension spawned by
> `use` on one line must survive to the next) — fixed by the session-persistent `VmState.io_backend`
> (`fix/repl-persistent-io-backend`).

## 8. Documentation

**Deferred for v1**, captured here as a placeholder for the eventual design.

A `README.md` in the package covers human docs for now. The intended direction is a real **documentation
format that lives in a folder inside the package** (e.g. `doc/`), **not** embedded in the manifest —
the manifest stays a thin launch/identity descriptor. A structured doc format (and how it surfaces —
e.g. flowing per-class/per-selector doc strings through the manifest-at-spawn `ClassDecl` into the
REPL's `$class` / `describe_class` introspection, `docs/internal/INTROSPECTION.md`) is its own project, taken
up separately.

## 9. Deferred / out of scope (v1)

- **Distribution:** a registry, fetch/download, version resolution, lockfiles. (`docs/internal/USE_ARCH.md`
  notes these slot in behind the resolver later, no syntax change.)
- **Per-platform prebuilt binaries** in the launch spec — only meaningful once packages are
  distributed, not hand-placed.
- **Publishing the SDK crates** (`quoin-ext` + `quoin-ext-proto`) to crates.io, so a third-party
  author writes `quoin-ext = "0.1"` rather than git-depending this repo — the prerequisite for
  building extensions out-of-tree (§13; Tier 0.5 in `docs/internal/FUTURE_EXT_ARCH.md` §9).
- **Version / protocol-compat enforcement** — the manifest may carry a version; the host doesn't gate
  on it yet.
- **Auto-respawn / circuit breaker** for a crashed package extension (§7).
- **Signatures / supply-chain trust** (§10) — the scheme depends on how distribution works.
- **`qn pkg` tooling** — `new` (scaffold), `build` (package the binary, emit glue), `install`, `list`,
  `info`. The manifest is kept rich enough that these are trivial later.
- **Shipped `init.qn` instead of synthesized glue (the followup).** A real, tool-generated `init.qn`
  as the `use` entry is more inspectable and the long-term preference, but it hits the **"where am I
  on disk?"** problem: a loaded unit runs as bare source with no `__file__`/`__dir__`, so relative
  paths resolve against the process CWD, not the package dir — `init.qn` can't find its own sibling
  binary/manifest. Fixing it needs either an injected `__dir__` binding (general-purpose; requires the
  resolver to surface a unit's directory) or an implicit "currently-loading package" the loader tracks
  (it already keeps an in-progress load stack for cycle detection). There are further subtle
  complexities here to work through before adopting it.
- **`use` aliasing** — not supported in the language (skipped); consumers use `Vec <- [Vectors]Vector`.

## 10. Trust

Extensions are a **single trust domain** (`docs/internal/FUTURE_EXT_ARCH.md` §4): no per-extension sandbox.
**Installing a third-party extension runs their native code** — `cargo install`-grade trust. v1 makes
that explicit (and a one-time "this package runs native code, continue?" confirmation is a reasonable
low-cost guardrail). Cryptographic signatures / supply-chain verification are deferred until the
distribution story exists (§9), since the scheme depends on it.

## 11. Build sketch (when we start)

Concrete pieces, smallest-first:

1. **Generalize the spawn primitive** — `Extension spawn:` (one binary path today, `extension.rs`)
   gains a command + args + cwd form; the package path uses it.
2. **`Extension loadPackage:` (host primitive)** — read `quoin.toml` (the TOML stdlib already
   exists), launch via the spec, install the provided classes under the package namespace (§4), store
   the `Extension` in the registry (§7). Idempotent against the registry.
3. **Package registry on `VmState`** — `HashMap<String, Extension-value>` (a `require_static` /
   GC-traced field as appropriate), with a `$packages` introspection hook.
4. **Resolver extension** — teach the named-package path of `FsResolver` (`src/packages.rs`) the
   search roots (§6), detect an extension package (`quoin.toml` present), and return the
   synthesized `loadPackage:` glue as the unit source (§5).

## 12. Decided vs deferred

**Decided (v1):** synthesized-`loadPackage:` load path; `quoin.toml` with a single `command`/`args`
launch spec; directory name == `use` name; **all classes namespaced under the package namespace, no
bare-global pollution**, host-applied to simple `ClassDecl` names; drop-a-folder search path
(`./quoin_packages/` + `$QUOIN_PATH`); eager spawn + a `VmState` package registry; raw `Extension
spawn:` remains the unmanaged escape hatch.

**Since shipped (2026-07-13):** the first `qn pkg` slice — `install` (per-user home
`$QUOIN_HOME`, `[bin]` PATH links) + `list`; the manifest generalized to `quoin.toml`.

**Deferred:** distribution/registry/fetch/versions/lockfiles; per-platform binaries; version/compat
enforcement; auto-respawn + circuit breaker; documentation format (folder-based, §8); signatures;
the remaining `qn pkg` verbs (`new`/`build`/`info`/`uninstall`); shipped `init.qn` (+ the "where am
I on disk?" complexities); `use` aliasing; publishing the SDK crates (§13).

## 13. Producing an extension (the author side)

§3–§7 are what a *package* is and how it *loads*; this is what an *author* builds to go inside one. An
extension is an ordinary program that links the extension-side SDK and runs a serve loop — **it needs
no part of the VM.** The same SDK is used in-tree (the `ext_vector` fixtures in `tests/fixtures/`) and
out-of-tree; the only difference is whether the SDK is a path dependency or a published one.

**Rust** — a plain binary crate depending only on the `quoin-ext` SDK. The author never names the wire
crate: `quoin-ext` re-exports everything they touch (`DataValue` / `ArrowArray` / `Arg`), and
`quoin-ext-proto` is a hidden transitive dep whose MessagePack codec is hand-rolled, so there is
no codegen and no serialization dependency at build time.

```toml
# Cargo.toml
[package]
name = "vectors-ext"
version = "0.1.0"
edition = "2024"

[dependencies]
quoin-ext = "0.1"        # once published; until then a git dep on this repo (see "The gap")
```

```rust
// src/main.rs
use quoin_ext::{DataValue, Extension};

struct Vector { data: Vec<f64> }

fn main() {
    let path = std::env::args().nth(1).expect("usage: vectors-ext <socket-path>");
    let mut ext = Extension::new();
    ext.class::<Vector>("Vector", |c| {
        c.constructor("ofFloats:", |_h, args| Vector { data: floats(&args[0]) });
        c.method("sum",   |v, _h, _a| DataValue::Float(v.data.iter().sum::<f64>()));
        c.makes("scale:", |v, _h, a| v.scaled(as_f64(&a[0])));
    });
    ext.serve(&path).expect("serve loop");
}
```

`cargo build --release` produces a standalone binary; the package's `quoin.toml` points `command`
at it (§3). The host execs it with the socket path as the final argv and the serve loop answers
`GetManifest` and dispatches — identical to an in-tree fixture. Other languages produce an equivalent
binary against their own SDK (`sdk/python/quoin_ext`, …); to the host it is just a `command`.

**The gap — publishing the SDK.** `quoin-ext` and `quoin-ext-proto` are **in-tree path crates** today
(`quoin-ext-proto = { path = "../quoin-ext-proto" }`), so a third party can currently only git-depend
the whole repo. They are self-contained — they depend on the `quoin` VM crate **not at all** — and on
nothing else (`quoin-ext-proto` is dependency-free) — so making them external is mechanical: publish both to
crates.io (turning the path dep into a version dep) so an author can write `quoin-ext = "0.1"`. This is
the **Tier 0.5 "extract / publish the SDK crates"** item (`docs/internal/FUTURE_EXT_ARCH.md` §9 / build-order
note) — the missing link that makes "out-of-crate" real, and a prerequisite for third-party authoring
even though the packaging machinery (§3–§7) does not otherwise depend on it.
