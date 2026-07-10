# Part VIII — Tooling

Everything ships in one binary: `qn` runs programs, and its subcommands are the
REPL, the test runner, the static checker, the formatter, the documentation
generator, the debugger, and a syntax highlighter. This part is a tour of each,
with real sessions. Shell transcripts are shown as plain fences (they are
terminal text, not Quoin); runnable Quoin examples are tagged and verified like
everywhere else in this book.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · **Tooling** · [Library & reference](09-library-and-reference.md) · [Appendices](10-appendices.md)

---

## 36. Running programs — `qn`

> **Rules**
> - `qn app.qn [args…]` runs a program. Everything after the file lands in
>   `Runtime.arguments` (a List of Strings). Arguments that *look like flags*
>   must sit after a `--` separator, or `qn` tries to parse them itself.
> - `qn -e EXPR` evaluates one expression and prints its value (a `nil` result
>   prints nothing) — the same rendering the REPL uses.
> - **Exit codes**: `0` on normal completion; an **uncaught error** prints its
>   annotated report to stderr and exits `1`; `Runtime.exit:N` exits with `N`
>   (it is uncatchable, and teardown still runs).
> - **Environment**: `QUOIN_STDLIB=DIR` loads the stdlib from `DIR` instead of
>   the copy embedded in the binary; `QUOIN_PATH=DIRS` adds extra roots to the
>   extension-package search. Internal `QN_*` tuning knobs are catalogued in
>   `docs/ENV_FLAGS.md` and are not part of the language surface.

A program is just a `.qn` file; there is no required entry point — top-level
statements run in order (Part I). Command-line arguments arrive as a list of
strings:

```quoin
Runtime.arguments.print
```

```
$ qn args.qn -- --verbose input.txt
#(--verbose input.txt)
```

The `--` matters: without it, `qn` claims dashed arguments as its own and
refuses the ones it doesn't know.

```
$ qn args.qn --verbose
error: unexpected argument '--verbose' found

  tip: a similar argument exists: '--version'
  tip: to pass '--verbose' as a value, use '-- --verbose'
...
```

`-e` is the one-liner form — handy in shell pipelines, and it prints the
expression's value exactly as the REPL would:

```
$ qn -e '(1..5).collect:{ |n| n * n }'
#(1 4 9 16)
$ qn -e 'nil.foo'
Message not understood: receiver=Nil, selector='foo', args=[]
  at <eval>:1:1
  |
  | nil.foo
  |
  at (top) in <eval>:1:0                                < nil.foo
$ echo $?
1
```

That second run shows the exit-code contract: an error nobody catches is
reported on stderr and the process exits `1`, so shell scripts and CI can gate
on any `qn` invocation. To choose the status yourself, `Runtime.exit:3` ends
the process with code 3 — it cannot be caught, but sockets, extensions, and
other resources are still torn down normally.

---

## 37. The REPL — `qn repl`

> **Rules**
> - `qn repl` opens an interactive session; the prompt is `qn> `. Results echo
>   as `=> value`; a `nil` result is suppressed.
> - **State persists across lines**: class definitions and constants become
>   ordinary globals; lowercase `var`/`let` locals live for the whole session.
> - Line editing with history (`~/.quoin_history`), **syntax highlighting as
>   you type**, multiline continuation (an incomplete line keeps reading), and
>   **tab completion** — global and session-local names, namespace names inside
>   `[…]`, and `receiver.` selectors when the receiver's class is knowable (a
>   class name, a session local, or a literal).
> - A line whose first non-space character is `$` is a REPL command, not Quoin:
>
>   | Command | Does |
>   |---|---|
>   | `$type <expr>` | show the class of an expression's result |
>   | `$inspect <expr>` | evaluate and show the value's class + fields |
>   | `$time <expr>` | evaluate and report wall-clock time |
>   | `$globals [pre]` | list defined classes and values (optional prefix) |
>   | `$class <Name>` | show a class: parent, mixins, ivars, methods |
>   | `$doc <Name>[.sel]` | show a class's or method's reference doc |
>   | `$load <file.qn>` | run a `.qn` file into the session |
>   | `$ps` (or `$ps tree`) | show tasks, fibers, workers, and waits |
>   | `$reset` | clear session locals (definitions stay) |
>   | `$help`, `$quit`/`$exit` | help; leave (also Ctrl-D) |
> - `~/.quoinrc`, if present, runs into the session before the first prompt —
>   **interactive sessions only** (piped input and `qn -e` skip it, like a
>   shell rc file).

A session, verbatim:

```
$ qn repl
qn> var nums = #(3 1 4 1 5)
=> #(3 1 4 1 5)
qn> nums.sort
=> #(1 1 3 4 5)
qn> $type nums.first
class Integer
qn> $inspect 1..5
NumberRange(1..5)  (class NumberRange)
  @start: Integer
  @end: Integer
  @n: Integer
qn> $time nums.sort
=> #(1 1 3 4 5)
   (1.873 ms)
qn> $globals Str
Classes (2):
  String  StringStream
qn> $reset
Session locals cleared.
```

Every line runs under the same scheduler a program uses, so top-level I/O, an
`Async.sleep:`, a spawned `Task`, or a fiber resume all work at the prompt —
and `$ps` shows what is running or parked while they do.

`$doc` is the reference at your fingertips: it answers with the same
documentation `qn doc` publishes (see below), for stdlib and your own loaded
classes alike:

```
qn> $doc List.sort
Sort in place, ascending by the elements' `>:`, and answer the receiver; nils sort last.
...
```

---

## 38. Tests — `qn test`

> **Rules**
> - `qn test [DIR]` (default `tests`) loads the test framework, then every
>   `.qn` file in `DIR` in sorted order, then runs every suite that registered.
> - A test file *builds* suites; **constructing a `TestSuite` registers it**
>   (into the `[Test]Suites` registry) — no export or main needed.
> - Structure: `(TestSuite.new:{ var name = '…' }).add:{ … }`; inside the
>   `add:` block, each `.test: name -> { … }` declares one test.
> - Assertions take the value under test as a **block**. The vocabulary:
>   `isTrue:` / `isFalse:`; `is:equalTo:` / `is:notEqualTo:` (the workhorse);
>   `is:a:` / `is:an:`; `is:closeTo:` (`…within:`); `is:lessThan:` /
>   `is:greaterThan:` (and the `…OrEqualTo:` pair); `does:match:` /
>   `does:notMatch:` (the `~` pattern protocol, §14); `does:resultIn:`
>   (effects); `does:throw:`; `elementsOf:areEqualTo:`. `.skip:'reason'` in an
>   `add:` block skips the suite without failing the run.
> - **Exit code**: `0` when everything passes, `1` on any failure — the CI gate.
> - `--coverage[=lcov|cobertura]` collects Quoin-level coverage: the report
>   goes to stdout (or `--coverage-out PATH`), a one-line summary to stderr.
>   The same flags work on a plain `qn program.qn` run.

A complete test file:

```quoin
(TestSuite.new:{ var name = 'Math' }).add:{
    .test:
    addition -> {
        .is:{ 2 + 2 } equalTo:4;
    };

    .test:
    division -> {
        .is:{ 10 / 4 } equalTo:2;
        .does:{ 1 / 0 } throw:ArithmeticError;
    };
}
```

```
$ qn test tests
[Math] Running 2 tests
[Math]   Test addition . 7µs (119µs) 1 passed
[Math]   Test division .. 10µs (1.5ms) 2 passed
[Math] Finished in 17µs (11.4ms) : 3 passes / 0 failures
All suites finished in 17µs (38.6ms) : 3 passes / 0 failures / 0 skipped
$ echo $?
0
```

A failure names the assertion's expected/actual and its source location, and
flips the exit code:

```
[Failing]   Test arithmetic ! 8µs (138µs) 1 of 1 assertions failed:
[Failing]     5 != 4 at self:failing/fail_test.qn:4:13
[Failing] Finished in 8µs (10.5ms) : 0 passes / 1 failures
All suites finished in 8µs (15.6ms) : 0 passes / 1 failures / 0 skipped
$ echo $?
1
```

The framework itself is ordinary, documented Quoin — for the full assertion
reference (including how to build custom assertions on
`recordResult:evidence:block:`), see the `TestSuite`, `BuiltinAssertions`, and
`IterateAssertions` pages of the generated API reference (`qn doc`, below), or
ask the REPL directly with `$doc TestSuite`.

> **⚠ Gotcha — the test directory's name is spliced into a `use` path.**
> `qn test DIR` synthesizes `use self:DIR/*`, so `DIR` must be spellable as a
> Quoin load path: `qn test my-tests` fails with a parse error (the `-` reads
> as an operator). Prefer plain names like `tests`.

Coverage, for CI dashboards:

```
$ qn test --coverage tests > coverage.lcov
coverage: 188/868 lines (21.7%)
```

The summary lands on stderr; the redirected stdout is a standard LCOV report
(`--coverage=cobertura` for Cobertura XML instead).

---

## 39. Static checking — `qn check`

> **Rules**
> - `qn check PATH…` parses and type-checks each file (directories recurse)
>   **without running anything**.
> - It reports parse errors, plus the gradual type checker's diagnostics:
>   unknown types, type mismatches (declarations, reassignments, returns),
>   messages a typed receiver does not understand, possibly-`nil` receivers and
>   operands, return-type covariance violations, and generics misuse.
> - Diagnostics are **warnings**: the checker is best-effort and never blocks
>   `qn` from running the program. `qn check` is where they become a gate.
> - **Exit codes**: `0` and silent when clean; `1` if any diagnostic or parse
>   error was reported.

```
$ qn check typo.qn
typo.qn:1:22: warning: type mismatch: expected `Integer`, found `String`
    |
  1 | var count: Integer = 'three'
    |                      ^^^^^^^
typo.qn:3:1: warning: `NumberRange` does not respond to `nopeMethod`
    |
  3 | r.nopeMethod
    | ^
$ echo $?
1
```

Quoin's types are gradual (annotations are optional, checking is best-effort),
so a clean `qn check` is not a soundness proof — but it catches the classic
mistakes before they run, and it is cheap enough to sit in a pre-commit hook
next to `qn fmt --check`.

---

## 40. Formatting — `qn fmt`

> **Rules**
> - `qn fmt PATH…` formats files **in place** (directories recurse; `-` reads
>   stdin and writes the result to stdout). Changed files are named; untouched
>   ones are silent.
> - **Opinionated and zero-config**: one canonical style (4-space indent,
>   width-driven line breaks, `;` separators normalized, `{ |x| … }` delimiter
>   padding), nothing to tune.
> - **Self-verifying**: the formatter re-parses its own output and refuses to
>   write if the AST changed or a comment was lost — a bug can produce an
>   error, never a meaning-changing write.
> - `--check` exits `1` and lists any file not already formatted (the CI
>   gate); `--dry-run` prints the would-be result without writing; `--diff`
>   prints a unified diff (and also exits `1` when anything would change).

```
$ qn fmt --diff messy.qn
--- messy.qn	2026-07-09 23:30:36
+++ messy.qn (formatted)	2026-07-09 23:30:36
@@ -1,3 +1,3 @@
-var total=0
-(1..4).each:{|n| total = total+n}
+var total=0;
+(1..4).each:{ |n| total = total+n };
 total.print
$ qn fmt messy.qn
formatted messy.qn
$ qn fmt --check messy.qn
$ echo $?
0
```

Note what the diff fixed silently: the missing `;` after the first two
statements. Part I's separator rule (a line starting with `.` or an operator
continues the previous statement) makes a dropped `;` a real hazard, and the
canonical style always writes them between statements — one of several ways
the formatter removes a whole class of surprises.

The self-verification is why formatting is safe to run blind over a tree: the
guarantee is checked *at write time* on your actual file, not merely promised
by the formatter's own test suite.

---

## 41. The API reference — `qn doc`

> **Rules**
> - `qn doc [PATH…]` generates the API reference for the stdlib **plus any of
>   your own units** into `qn-docs/` (`--out DIR` to change): one HTML page per
>   class and a namespace-grouped index. `--json` also writes the raw doc
>   model as `model.json` (`{"version": 1, …}`) for other renderers.
> - **Docs are comments**: a contiguous run of `"*` lines *immediately* above
>   a definition — method, class, or class extension — is that definition's
>   documentation. A blank line detaches it. The first line is the summary.
> - The same docs answer `$doc` in the REPL and `doc`/`docFor:` in code.
> - `--coverage` lists every public class/selector with no doc and the overall
>   percentage. It is a report, not a gate — exit `0` either way.
> - `--check` runs the documentation's **examples** instead of generating:
>   with PATHs, fenced blocks tagged `quoin` in markdown files/dirs; without,
>   the annotated examples inside the stdlib's own doc comments. Exit `1` if
>   any fail.

Documenting your own code is just commenting it (Part I's `"*` line comment),
directly above the thing described:

```quoin
"* A circle with a radius, in whatever unit you like.
Circle <- { |@radius|
    "* The enclosed area.
    area -> { @radius * @radius * 3.14159 };
    diameter -> { @radius * 2 };
}

(Circle.new:{ var radius = 2 }).area     "* -> 12.56636
```

```
$ qn doc shapes.qn --out shapedocs
qn doc: 111 classes -> shapedocs
$ qn doc --coverage shapes.qn
undocumented: Circle diameter
doc coverage: 1077/1078 (99.9%)
```

The coverage report is the safety net for the adjacency rule: a blank line
sneaking in between a doc block and its definition silently detaches the doc,
and `--coverage` is how that shows up.

### The doc-example harness — `qn doc --check`

Documentation rots unless something executes it. `qn doc --check PATH…` finds
every fenced code block tagged `quoin` in the given markdown files and **runs
each one in a fresh session**, statement by statement like the REPL. A block
tagged `quoin norun` is display-only; an untagged fence (shell transcripts,
program output) never runs. Within a running block, every `"* -> value`
annotation is asserted: the statement's rendered result must match. So this
fence is a *test*, executed on every change to this book:

```quoin
(1..5).collect:{ |n| n * n }    "* -> #(1 4 9 16)
```

```
$ qn doc --check docs/language/08-tooling.md
qn doc --check: 5 examples, 2 annotations checked, 0 failed
```

The chapter you are reading — all of `docs/language/` — is checked exactly this
way in CI, which is why its examples can promise their annotations are true.
Run bare (`qn doc --check`, no paths), the same engine executes the annotated
examples inside the stdlib's own doc comments, so the generated API reference
is held to the same standard.

---

## 42. The debugger — `qn debug`

> **Rules**
> - `qn debug app.qn [args…]` runs the program under the debugger, **paused at
>   the first line**. The prompt is `$ `; a bare expression evaluates in the
>   focus frame (`self`, `@ivars`, and locals all resolve).
>
>   | Command | Does |
>   |---|---|
>   | `$continue`, `$c` | resume execution |
>   | `$step`/`$s` · `$next`/`$n` · `$finish`/`$fin` | step into · over · out |
>   | `$break FILE:LINE` (`$b`; `$break LINE` = current file) | set a breakpoint |
>   | `$delete FILE:LINE` (`$d`) | clear a breakpoint |
>   | `$frames`/`$bt` · `$up` · `$down` | backtrace; move the focus frame |
>   | `$locals`/`$l` | locals, `self`, and `self`'s `@ivars` |
>   | `$list` · `$source on\|off` | source around the focus; auto-show toggle |
>   | `$print EXPR`, `$p` | evaluate in the focus frame (or just type it) |
>   | `$quit`/`$q` · `$help` | leave; help |
> - **Exception breakpoints**: `--break-on-throw TYPES` pauses when a matching
>   exception is thrown, *even one that would be caught* (first-chance);
>   `--break-on-uncaught TYPES` pauses only when it will escape. Both take a
>   comma-separated type list — matching is hierarchy-aware (`Error` catches
>   every structured error; `Object` is the explicit catch-everything), and a
>   Rust-raised `TypeError` and your own `Error.throw:` behave identically.
> - `qn debug --dap` speaks the **Debug Adapter Protocol** on stdio instead of
>   the `$`-command loop — point VS Code, `nvim-dap`, or any DAP client at it
>   for editor breakpoints and stepping.
> - A breakpoint pauses the whole scheduler, so the paused world holds still
>   while you inspect it.

A session against the `Tally` class from earlier chapters' mold:

```quoin
Tally <- { |@total|
    init -> { @total = 0 };
    add: -> { |n|
        @total = @total + n;
        @total
    };
    total -> { @total };
}

var tally = Tally.new
#(3 4 5).each:{ |n| tally.add:n }
('total: ' + tally.total.s).print
```

```
$ qn debug tally.qn
Quoin debugger — $help for commands, $continue to run, $quit to exit.
→ paused at tally.qn:1  (in <block>)
→    1 │ Tally <- { |@total|
     2 │     init -> { @total = 0 };
     3 │     add: -> { |n|
$ $break 4
breakpoint set at tally.qn:4
$ $continue
→ paused at tally.qn:4  (in add:)
     2 │     init -> { @total = 0 };
     3 │     add: -> { |n|
→    4 │         @total = @total + n;
     5 │         @total
     6 │     };
$ $locals
  n = 3
  self = Tally{@total: 0}
  @total = 0
$ @total + n
3
$ $continue
→ paused at tally.qn:4  (in add:)
...
$ $delete tally.qn:4
breakpoint cleared at tally.qn:4
$ $continue
total: 12
```

Note the second pause: a breakpoint fires on every arrival at its line — each
`each:` iteration — until deleted. The `@total + n` line is the
expression-first design: anything that isn't a `$`-command is evaluated in the
focus frame, so inspecting state is just writing Quoin.

Exception breakpoints pause with the throwing stack still live, before any
unwinding, so `$frames`, `$locals`, and eval-in-frame see the world exactly as
the `throw` left it:

```quoin norun
var half = { |n|
    (n % 2 == 0).if:{ n / 2 } else:{ ValueError.throw:('odd: ' + n.s) }
}
#(4 6 7).each:{ |n| (half.value:n).print }
```

```
$ qn debug --break-on-throw ValueError oops.qn
...
$ $continue
2
3
→ broke on throw: ValueError{@message: 'odd: 7' @payload: nil}
→ paused at core/00-bootstrap.qn:115  (in throw:)
$ $frames
→ #2  core/00-bootstrap.qn:115  throw:
  #1  oops.qn:2  value:
  #0  oops.qn:4  <block>
```

`$continue` from a throw pause simply lets the exception keep propagating (or
get caught) exactly as it would have. Had this run used `--break-on-uncaught`
and the `ValueError` been caught somewhere, no pause would fire at all — that
mode is for the error that actually escapes.

---

## 43. Syntax highlighting — `qn highlight`

> **Rules**
> - `qn highlight FILE` prints the source ANSI-colorized to the terminal —
>   the same highlighting the REPL applies as you type.
> - `qn highlight --html FILE` emits a standalone HTML page instead, using the
>   same code styles as the pages `qn doc` generates (one style table, two
>   consumers).

```
$ qn highlight --html greet.qn | head -3
<!doctype html>
<html><head><meta charset="utf-8">
<title>greet.qn</title>
```

---

Next: **[Part IX — The standard library](09-library-and-reference.md)** — the
core types, string formatting, namespaces, and the stdlib map.
