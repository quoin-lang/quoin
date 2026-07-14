# Part X — Quoin packages

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · **Packages** · [Appendices](11-appendices.md)

---

## 49. What a package is

> **Rules**
> - A **package is a folder with a `quoin.toml`** manifest. Two kinds share the format:
>   an **extension package** carries an `[extension]` launch spec and is loaded with
>   `use name:*`; a **program package** carries `[bin]` executables that install onto
>   your `PATH` (§50) — a package may be both.
> - `use name:*` finds the folder by searching, in order: the project-local
>   **`./quoin_packages/name/`**, each entry of **`$QUOIN_PATH`** (platform
>   path-separated), and the per-user install root **`$QUOIN_HOME/packages/name/`**
>   — project beats explicit path beats installed. The first root holding
>   `name/quoin.toml` wins.
> - Loading an extension package **spawns its process eagerly** and installs every
>   class it provides **under the package namespace** (`[Greet]Greeter`) — a package
>   can never claim a bare global. Loads are run-once: a second `use` is a no-op.
> - A program package's executables do not involve `use` at all — they are ordinary
>   commands once installed (§50).

The manifest is declarative TOML, so tooling can read it without spawning anything:

```text
greeter/
  quoin.toml          # identity + what the package provides
  bin/greeter         # an executable (an extension process, or a Quoin script)
  init.qn             # optional Quoin glue, run after an extension's classes install

[package]
name = "greeter"            # the `use` / install name (one plain path component)
version = "0.1.0"
description = "…"

[extension]                 # only for extension packages
command = "bin/greeter"     # how to launch the subprocess (relative to the folder)
namespace = "Greet"         # classes install as [Greet]…; defaults to PascalCase name

[bin]                       # only for program packages
greeter = "bin/greeter"     # PATH name -> package-relative executable
```

```quoin norun
use greeter:*;                       "* spawn + install the classes, namespaced
var g = [Greet]Greeter.named:'Ada';  "* a real class, dispatching over the socket
Greeter <- [Greet]Greeter;           "* want it short? alias it — an ordinary definition
```

> **⚠ Gotcha — package roots are CWD-anchored.** `./quoin_packages/` resolves against
> the process working directory, not the running script's folder — running a script
> from elsewhere changes which *project-local* packages it sees. The installed root
> (`$QUOIN_HOME/packages`) is absolute and always applies.

## 50. Installing packages — `qn pkg`

> **Rules**
> - **`qn pkg install DIR`** copies the folder into `$QUOIN_HOME/packages/<name>/`
>   (the name comes from the manifest) and links each `[bin]` entry into
>   `$QUOIN_HOME/bin/`. Installing again **replaces** the previous copy whole; the
>   copy is staged, so a failed install never leaves a half-package.
> - **`$QUOIN_HOME` defaults to `$HOME/.quoin`.** Installed packages resolve with no
>   `QUOIN_PATH` entry, from any directory.
> - **Put `$QUOIN_HOME/bin` on your `PATH` once** — every installed executable
>   arrives there as a link (made executable, so shebang scripts just run).
>   `qn pkg install` prints a reminder when the directory isn't on your `PATH` yet.
> - **`qn pkg list`** shows what's installed: name, version, executables, description.
> - No registry, no fetching, no version resolution yet: you install a folder you
>   have. Uninstall is manual for now — delete `$QUOIN_HOME/packages/<name>` and any
>   of its links in `$QUOIN_HOME/bin`.

```text
$ qn pkg install dist/greeter
installed greeter 0.1.0 -> /Users/you/.quoin/packages/greeter
$ qn -e "use greeter:*; ([Greet]Greeter.named:'World').greet.print"
Hello, World!

$ qn pkg install hello-tool
installed hello-tool 0.2.0 -> /Users/you/.quoin/packages/hello-tool
linked /Users/you/.quoin/bin/hello -> …/packages/hello-tool/bin/hello
$ hello
hello from an installed tool

$ qn pkg list
greeter 0.1.0  The quoin-ext greeter example…
hello-tool 0.2.0  [bin: hello]  A pure-Quoin program installed onto the PATH.
```

## 51. Writing packages

> **Rules**
> - **An extension package** is a separate process providing real Quoin classes over
>   a socket — write it against the Rust SDK (`quoin-ext`, see its README and
>   `examples/greeter.rs`) or the Python SDK (`sdk/python/quoin_ext`); the VM cannot
>   tell the two apart. The binary declares **simple** class names; the *package*
>   namespace is applied by the host at install.
> - An optional **`init.qn`** runs after the classes install — Quoin-side glue like
>   convenience methods reopened onto the installed classes (`[ADBC]Connection`'s
>   `transaction:` sugar is exactly this).
> - **A program package** is any folder whose `[bin]` entries point at executables —
>   compiled binaries or `#!/usr/bin/env qn` scripts. This is how a pure-Quoin
>   program ships: the [quern](https://github.com/quoin-lang/quern) task runner is
>   the model citizen.
> - The design record is `docs/internal/EXT_PACKAGING.md`; deliberately deferred:
>   registries/fetching, versions & lockfiles, signatures, per-platform binary
>   selection, auto-respawn of crashed extensions.

A package needs no build system, but a Quernfile makes the assemble step
repeatable — `crates/quoin-ext/examples/Quernfile.qn` builds the greeter binary and
lays out the installable folder in two mtime-skipped tasks; copy its shape for your
own extension crate.

---
