# Part IX — The standard library

A guided tour of the standard library: what each area is for, how the pieces fit
together, and the idioms that make them work well — concepts, not catalogues.

> **Where the API reference lives.** Every stdlib class is documented per-class
> and per-method, with verified examples, in the generated API reference: run
> `qn doc` to build it (it writes `qn-docs/`), or ask the running system
> directly in the REPL with `$doc Name` / `$doc Name.selector` — e.g. `$doc List`
> or `$doc [IO]File.create:`. This chapter names the classes; the reference
> documents them. Method lists are deliberately absent here.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · **The standard library** · [Packages](10-packages.md) · [Appendices](11-appendices.md)

---

## 44. The library by area

### Collections & the `Iterate` protocol

Four concrete collections — **List** `#(10 20 30)` (ordered, growable,
zero-based), **Map** `#{ 'a':1 'b':2 }` (insertion-ordered dictionary; iterating
yields **KeyValuePair** objects, read with `key`/`value`), **Set** `#<1 2 3>`
(insertion-ordered, unique, hash-indexed membership) and **NumberRange** `a..b`
(numeric, **end-exclusive**) — share one protocol. Each implements `each:`; the
**Iterate** mixin derives everything else from it: `collect:`, `select:`,
`reduce:`, `sort`, `join:`, `groupBy:`, lazy pipelines, external iterators, some
fifty combinators in all (Part V §17). Because the protocol is a mixin over a
single method, *anything* with an `each:` — a directory listing, a generator,
your own class — gets the whole vocabulary. For bulk numeric data there is also
**Array**, a typed contiguous column (`#int64`/`#float64` in Apache Arrow
layout) that extensions such as numpy read with zero conversion — a List holds
anything; an Array holds one dtype, packed.

```quoin
#(3 1 2).sort.collect:{ |n| n * 10 }    "* -> #(10 20 30)
(1..10).select:{ |n| (n % 3) == 0 }     "* -> #(3 6 9)
```

### Strings, regexes & symbols

**String** is immutable UTF-8 text, written `'single-quoted'` (a double quote
starts a *comment* in Quoin); position-based operations count characters, not
bytes. **Regex** is the type of `#/pattern/` literals (Rust regex syntax): test
with the match operator `~`, split with `split:`, substitute via
`replace:with:`, and capture with `match:` — it answers a **Match** (`nil` on a
miss) whose groups read by index or name via `at:`, list out via `captures`,
and destructure into a block via `bind:` (§14). **Symbol** is an interned
identifier — `#name`, or `#'quoted
form'` — compared by identity, so symbol comparison is a cheap pointer check;
selectors and other names in the reflective API are symbols, a distinct type
from String. Formatting and interpolation get their own section (§45).

```quoin
'quoin'.upper                       "* -> QUOIN
#/[0-9]+/ ~ 'abc 123'               "* -> true
(#/[0-9]+/.match:'abc 123').s       "* -> '123'
```

### Numbers

**Integer** is a 64-bit signed whole number (`/` truncates toward zero);
**Double** is IEEE-754, and mixed arithmetic yields a Double. There is *no
silent promotion* to arbitrary precision — when a value can outgrow 64 bits you
convert explicitly: **BigInteger** (`asBigInteger`, `BigInteger.of:`) for exact
whole numbers, **BigDecimal** (`BigDecimal.of:` a String or Integer —
deliberately not a Double) for exact base-10 quantities like money. **Math** is
a namespace of constants and free functions (`Math.pi`, `Math.sin:`) — the
operations that read naturally *on* a number (`abs`, `sqrt`, `pow:`, `floor`)
live on the number types themselves. Statistics are part of the collection
protocol, not a separate toolkit: any Iterate-able of numbers has `mean`,
`median`, `mode`, `variance`, `stddev`, `percentile:`.

```quoin
2.asBigInteger.pow:100    "* -> 1267650600228229401496703205376
#(1 2 3 4).mean           "* -> 2.5
```

### Time

Split by job so each type can be exactly right: **Instant** is the
monotonic clock — for measuring elapsed time, immune to wall-clock adjustments,
meaningless across processes; **Timestamp** is absolute UTC wall-clock time;
**DateTime** is a Timestamp plus its **TimeZone** (IANA zones), which is what
makes calendar arithmetic, DST, and offsets come out right; **Duration** is a
signed span with nanosecond precision. **Timer** is the one-selector stopwatch:
`Timer.time:aBlock` answers the block's elapsed whole microseconds.

The civil types drop what doesn't apply: **Date** is a calendar date with no
time and no zone (birthdays, deadlines — "March 3rd" everywhere), **Time** a
wall-clock time of day with no date (its `Duration` arithmetic *wraps* around
midnight). And **Span** is the calendar-aware duration — years, months, weeks,
days, and time units held as *separate fields*, so "1 month" stays 1 month
until it meets a date, where `+`/`-` apply it correctly (end-of-month clamping,
DST). `Span` parses ISO 8601 (`'P1Y2M'`) and the friendly form (`'1y 2mo'`);
its equality is *fieldwise* (`1h` ≠ `60m` — whether they match depends on a
calendar), and `asDuration` converts only pure-time spans, refusing calendar
units rather than guessing. Diffs go the other way with `until:` — a calendar
Span (`'2y 2mo 3d'`) on `Date` and `DateTime`, where subtraction answers an
absolute `Duration`. Civil and zone-aware meet through `date.atTime:zone:` /
`date.inZone:` (→ DateTime) and `DateTime#date` / `#time` (→ civil).

```quoin
(Duration.minutes:2) + (Duration.seconds:30)    "* -> 2m 30s
(Timestamp.parse:'2026-07-09T12:00:00Z').inZone:(TimeZone.of:'Asia/Tokyo')
"* -> 2026-07-09T21:00:00+09:00[Asia/Tokyo]
(Date.year:2024 month:1 day:31) + (Span.months:1)      "* -> 2024-02-29
(Date.parse:'2024-01-15').until:(Date.parse:'2026-03-18')    "* -> 2y 2mo 3d
((Time.hour:23 minute:30) + (Duration.hours:1)).s      "* -> '00:30:00'
```

### Data formats

One convention across the text formats — **JSON**, **YAML**, **TOML**, **CSV**:
`parse:` turns text into ordinary Quoin values (maps, lists, strings, numbers),
`generate:` turns values back into text. Because Map preserves insertion order,
a parse → generate round-trip doesn't reshuffle a document. **MessagePack** is
the binary equivalent (`pack:` to Bytes, `unpack:` back), and **Base64** /
**Hex** encode between Bytes and text — with conveniences hung directly on the
values (`'hi'.asBytes.toBase64`, `aString.fromBase64`).

Anything beyond the core value tree serializes through the **`asData`
protocol**: when a generator's walk reaches a value it has no representation
for, it calls the class's `asData` — which answers a core-tree value — and
recurses on the result. One method opts any class into *every* format, and
because classes are open, that includes classes you don't own. The stdlib
defaults are already wired: `DateTime` (RFC 9557), `Timestamp` (RFC 3339),
`Date`/`Time` (ISO), `Span`/`Duration` (ISO durations), `UUID`/`ULID`, `Set`
(→ List), `KeyValuePair` — each chosen to parse back via the type's own
`parse:`. Deliberately one-way and untagged (interop over magic); the reverse
convention is a class-side `fromData:`. `Symbol`, `Block`, and `Regex` stay
unserializable unless you opt them in — silently stringifying identifiers or
code is a trap. A self-referential `asData` spends the same 128-level depth
budget as any value, so it errors catchably instead of hanging.

```quoin
JSON.generate:(Date.parse:'2026-07-11')     "* -> '"2026-07-11"'
Point <- { |@x @y| asData -> { #{ 'x': @x 'y': @y } } };
JSON.generate:(Point.new:{ var x = 1; var y = 2 })    "* -> '{"x":1,"y":2}'
```

```quoin
JSON.parse:'[1, 2, 3]'      "* -> #(1 2 3)
JSON.generate:#{ 'a':1 }    "* -> {"a":1}
'hi'.asBytes.toBase64       "* -> aGk=
```

### Bytes & compression

**Bytes** is immutable binary data. Text crosses the boundary *explicitly* —
`'…'.asBytes` encodes, `asString` decodes (UTF-8) — so there is never a
question of which encoding applied. Compression is built in as Bytes codecs:
gzip and deflate both ways (`encodeGz`/`decodeGz`, `encodeDeflate` — HTTP's
zlib-wrapped form — plus `encodeDeflateRaw` for the bare RFC 1951 stream zip
carries; `decodeDeflate` reads both), zstandard decode (`decodeZstd`);
malformed input raises a catchable `ParseError`. `crc32` is the matching
integrity stamp (zip's per-entry checksum). Compression also *streams*, both ways: `gunzip` on an
unread `ByteStream`/`StringStream` wraps it in place, so a `.gz` file — or the
`.gz` half of a `.tar.gz` — reads incrementally through the ordinary stream
methods (`readAll`, `readLine`, `eachLine:`), nothing materialized; `gzip` on
an unwritten file write stream is the encoder twin, so a `.log.gz` writes line
by line. Concatenated gzip members decode end to end; corrupt input is a
catchable `IoError` on read. One discipline on the write side: **`close`
finishes the encoder** — the trailer that makes the file valid is written
there. Close deliberately; a stream the program leaks or leaves open at exit
is still finished for it, best-effort, on the way out.

**[Archive]Tar** treats tar as a *stream* in both directions — pure Quoin over
any ByteStream, so `.tar.gz` is just composition: `[Archive]Tar.over:(handle
.byteStream.gunzip)` to read, `[Archive]Tar.writeTo:(([IO]File.create:'x.tar.gz')
.gzip)` to write. Reading: entries arrive in order and are consumed once
(`each:` carries the whole Iterate vocabulary; a passed entry's content is
skipped in chunks, never materialized); ustar prefixes, GNU long names, and
pax `path=` headers all resolve, and header checksums are verified.
`extractTo:` writes files and directories under a target — with member paths
normalized and **confined**: an absolute or `../`-escaping path throws before
anything lands outside. Writing: the `writeTo:` writer takes `add:text:` /
`add:bytes:`, `addFile:as:` (streamed from disk, size and mtime from the
metadata snapshot), and `addFolder:`; names over 100 bytes ride a pax `path=`
header, and `close` writes the end blocks and closes the stream through the
codec — system tar reads the result directly.

**[Archive]Zip** is the random-access archive, and its reader is shaped by the
format: a zip's truth is the *central directory at the end of the file*
(trusting the local headers a streaming reader meets first is a classic
correctness and security trap), so `Zip.open:` reads through a
`RandomAccessFile` — the directory once, each member lazily by offset, nothing
loaded whole — and `Zip.of:` runs the same reader over in-memory `Bytes` (a
downloaded archive). Entries are a real `List`: the whole Iterate vocabulary,
`at:'name'` addressing, contents re-readable in any order — no one-pass
constraint. Every content read is CRC-32-verified; stored and deflated members
both decode; encrypted, multi-disk, and zip64 archives refuse with a
`ParseError` naming the reason; timestamps come back as civil `date` / `time`
(that is what DOS times are — local wall clock, no zone). `extractTo:`
confines exactly like tar's. Writing streams naturally (data first, directory
at `close`), so `Zip.writeTo:` takes any write stream; each member is stored
or deflated, whichever is smaller — what real zip tools do.

```quoin
'hello'.asBytes.encodeGz.decodeGz.asString    "* -> hello
```

### Files, I/O & streams

File I/O has two levels. For the common case there are one-shot class methods
on **[IO]File**: `read:` (the whole file as a String), `write:to:` (replace a
file's contents), `append:to:` — each opens, writes, and closes, so nothing is
left unflushed. For incremental work, `open:`, `create:` (truncate/create) and
`append:` answer a buffered **ByteStream** — writes accumulate and drain in
chunks; `close` flushes, `flush!` forces the buffer out early, and a stream
never closed is flushed when the program ends. `stringStream` wraps any byte
stream as a **StringStream**, the UTF-8 text view (`readLine`, `eachLine:`,
`writeln:`). These two stream classes are the *single* reading/writing surface
over every conduit — files, sockets, and standard input all hand you the same
streams, so code written against a stream doesn't care what's underneath.
An `[IO]File` value from `open:` also carries a metadata snapshot: `size` in
bytes and `modified` as a Timestamp — what a tar header or a Content-Length
needs before any content is read.
For random-access *formats*, `randomAccess` opens the file for positioned
reads instead: a **RandomAccessFile** answers `readAt:offset count:` and
`size` — pread-style, no cursor, reads independent and repeatable. That pair
is the informal random-access read protocol; `Bytes` speaks it too, which is
how `[Archive]Zip` reads a file on disk and a downloaded buffer with the same
code.
`[IO]Stdout` / `[IO]Stderr` are writable handles, `[IO]Stdin` reads without
blocking other tasks, and **[IO]Folder** is an Iterate-able directory listing.

```quoin
[IO]File.write:'saved' to:'/tmp/qn-book-io.txt';
[IO]File.read:'/tmp/qn-book-io.txt'    "* -> saved
[IO]File.delete:'/tmp/qn-book-io.txt'
```

### The operating system

**[OS]Env** is *read-only* access to the process environment (`at:`,
`at:ifAbsent:`, `asMap`, iteration) — mutation is deliberately absent, because
the C environment is process-global state other threads may be reading.
**[OS]Path** is purely *lexical* path manipulation over Strings — `join:`,
`dirname:`, `basename:`, `extension:`, `normalize:` — it never touches the
filesystem, which is exactly what makes it safe on a path you are about to
create. Filesystem truth (existence, deletion, renames) lives on `[IO]File` and
`[IO]Folder`.

**[OS]Process** runs subprocesses *on the scheduler*: `run:` parks the calling
task for the child's whole lifecycle — other tasks keep running — and answers a
**ProcessResult** (`stdout`/`stderr`, `exitCode`, `ok?`, and `check`, which
turns failure into a typed `ProcessError` carrying the result). The command is
a List (program + arguments): there is **no shell**, so nothing splits, globs,
or injects — and `run:env:` sets variables for the *child*, which is why
`[OS]Env` itself can stay read-only. `start:` spawns for streaming: the handle's
`stdout`/`stderrText` read like sockets, `writeStdin:`/`closeStdin` feed it,
`wait`/`kill`/`terminate` manage it. Lifecycle is owned, not leaked: a
cancelled `run:` (an `Async.timeout:` firing) kills its child, an undetached
handle's child dies with it, and `detach` is the explicit opt-out.

```quoin
[OS]Path.join:#('etc' 'app' 'conf.toml')                "* -> etc/app/conf.toml
[OS]Env.at:'QN_SURELY_UNSET' ifAbsent:{ 'fallback' }    "* -> fallback
([OS]Process.run:#( 'echo' 'hi' )).stdout               "* -> 'hi\n'
([OS]Process.run:#( 'cat' ) input:'meow').stdout        "* -> 'meow'
([OS]Process.run:#( 'false' )).ok?                      "* -> false
```

### Terminal & logging

**Term** answers the terminal facts — `color?` (whether styled output is on:
the *same* detection the std-stream writers use, so true means an `#ANSI'…'`
write will actually render), `tty?`, `width`/`height` (nil when piped) — and
treats markup as an operation: `render:` to escape codes unconditionally,
`strip:` to the plain text (also the honest way to measure styled width).
`'FAIL'.styled:'bold red'` styles a computed String programmatically (markup-
escaped, so its own brackets stay literal), and an ANSI value answers `plain`
and `renderedLength`.

**Log** is leveled logging over one replaceable sink. `Log.debug:` / `info:` /
`warn:` / `error:` take a String, an ANSI value, or a **Block** producing one —
below the `Log.level` threshold the block is *never evaluated*, so a debug
entry costs a comparison. `Log.level:#debug in:{ … }` changes the threshold
temporarily (restored even on a throw). Every emitted entry carries its
caller's `'file:line:col'`, passed to the sink as a separate argument
(`Log.sink:{ |level message location| … }`); the default sink writes
`HH:MM:SS LEVEL file:line:col: message` to `[IO]Stderr`, colored on a terminal
and plain everywhere else. The `???` placeholder statement is a plain
`Log.warn:` — level and sink govern it like any other warning.

```quoin
Term.strip:'[red]hot[/]'                     "* -> 'hot'
('FAIL'.styled:'bold red').plain             "* -> 'FAIL'
Log.level:#error in:{ Log.info:{ !!! } }    "* -> nil
```

### Command-line tools

**[CLI]Spec** turns `Runtime.arguments` into a declared interface: flags,
value-taking options (defaults, `required:`, `values:` enums), positionals, a
trailing `rest:` splat, or subcommands (each with its own sub-spec). `parse` is
the production entry — `-h`/`--help` prints the *generated* help and exits 0,
misuse prints the message plus usage to stderr and exits 2 — while `parseFrom:`
throws a typed **UsageError** instead, which is how you test a tool in-process.
GNU conventions: `--name value`, `--name=value`, short `-x`, `--` ends option
parsing; values stay Strings (convert with `to_integer` and friends). Reading a
name the spec never declared is a ValueError — a typo can't masquerade as an
absent option. With a shebang and the execute bit (§36), the result is a real
command: `./greet --help` is your tool's help, not qn's.

```quoin
var cli = [CLI]Spec.new:'greet' about:'says hello'
cli.flag:'shout' short:'s' help:'LOUDLY'
cli.positional:'name' help:'whom to greet'
var args = cli.parseFrom:#( '-s' 'quoin' )
#( (args.at:'name') (args.flag?:'shout') )    "* -> #(quoin true)
```

### Unique identifiers

**UUID** generates the standard 128-bit identifiers — `generateV4` (random) or
`generateV7` (time-ordered, so fresh IDs sort by creation time) — and parses
the hyphenated form. **ULID** is the sortable alternative: a millisecond
timestamp plus randomness, rendered as 26 characters of Crockford base32, where
string order equals creation order. Both convert to `Bytes` and compare with
the ordinary operators.

```quoin
ULID.generate.s.length    "* -> 26
```

### Hashes, MACs & secure random — `[Crypto]`

**`[Crypto]Digest`** computes one-shot digests — `sha256:` / `sha512:` /
`sha1:` / `blake3:`, plus `md5:` (not cryptography anymore; it lives here to
keep the hashes together). Each takes a String (hashed as its UTF-8 bytes) or
a Bytes value and answers the raw digest as **Bytes**, composing with the
codecs above: `toHex` for the usual text form, `Base64.encode:` for wire
formats. **`[Crypto]Hmac`** is the keyed counterpart (`sha256:key:`,
`sha512:key:`, `sha1:key:`) — and checking a received MAC goes through
`verifySha256:message:key:`, which compares in **constant time**; `==` on the
recomputed Bytes bails at the first differing byte and leaks how much of a
guess was right. **`[Crypto]Random`** answers bytes from the operating
system's CSPRNG for keys, tokens, and salts — the seedable `Random` class is
for simulations, this one is for secrets.

```quoin
([Crypto]Digest.sha256:'abc').toHex          "* -> 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
var mac = [Crypto]Hmac.sha256:'msg' key:'secret'
[Crypto]Hmac.verifySha256:mac message:'msg' key:'secret'    "* -> true
([Crypto]Random.bytes:16).count              "* -> 16
```

### Metaprogramming: the parser & AST

`use std:lang/ast` exposes the compiler's front half: **[Lang]Parser** parses
source (`parse:`, `parse:named:`, `parseFile:`) into a tree of **[Lang]Node**
— one class for every node, not thirty. A node answers `kind` (a Symbol:
`#send`, `#classDefinition`, `#stringLiteral`, …), `children` (structural, in
source order), and `at:#field` for kind-specific parts (`#selector` /
`#receiver` / `#arguments` on a send; `#name` / `#parent` / `#body` on a class
definition; `#value` on literals — nil for a field the kind doesn't have).
Source fidelity is total: every node carries `file` (threaded from
`parseFile:` / `parse:named:`), `span` (byte offsets + line/column), and
`text`, the exact source slice. `walk:` visits a subtree pre-order and
`allNodes` flattens it, so the whole Iterate vocabulary works on programs —
a lint rule is a `select:` one-liner.

Transformation is deliberately *source rewriting*, not tree mutation:
**[Lang]Rewrite** collects span edits (`replace:with:`, `insertBefore:text:`,
`delete:`) and `apply` splices them byte-exactly into new source — parse it
again to verify, `Runtime.eval:` it to use it. Overlapping edits throw. (A
synthetic-node unparser is future work; until then the AST never has to
invent text, only point at it.)

```quoin
use std:lang/ast;
var src = 'var total = 2 + 3';
var rw = [Lang]Rewrite.over:src;
([Lang]Parser.parse:src).walk:{ |n|
    (n.kind == #integerLiteral).if:{ rw.replace:n with:((n.at:#value) * 10).s }
};
rw.apply    "* -> var total = 20 + 30
```

### Covered elsewhere

- **Concurrency** — `Task`, `Fiber`, `Channel`, `Async`, the `parallelCollect:`
  combinators, and `Plan` are Part V's subject.
- **Networking & the web** — sockets (`TcpSocket`, `TlsSocket`, `TcpListener`,
  `TcpServer`), the `[HTTP]Client` / `[HTTP]Server` pair, and the `[Web]App`
  framework are covered in Part VI.
- **Types** — annotations, generics, and the gradual checker are Part VII.
- **Tooling** — the `qn` CLI (`test`, `fmt`, `doc`, `check`, `debug`, the
  REPL), plus the program-facing side: `Runtime` (command-line `arguments`,
  `exit:`, `eval:`) and `VM` (runtime introspection: `stats`, `ps`) — Part VIII.

---

## 45. Value rendering & string formatting

> **Rules**
> - Every value renders two ways: **`s`** is the *human* rendering (a String —
>   what `writeln:` and interpolation use), **`pp`** is the *structural* dump
>   for debugging and inspection — escaped strings, instance variables,
>   width-aware wrapping (`pp:` takes an explicit width). `s` falls back to the
>   `pp` rendering for values with no intrinsic human form.
> - **`%:` (binary `%`)** — `'fmt' % arg` substitutes into placeholders:
>   - a bare `%` consumes the next argument value;
>   - `%1`, `%2`, … index (1-based) into a **list** argument;
>   - `%a`, `%b`, … (single letters) key into a **map** argument.
> - **Prefix `%`** — `%'…%{expr}…'` is inline interpolation: the compiler lowers the literal to string concatenation, so each `%{expr}` compiles in the **enclosing scope** — locals, parameters, `self`, leading-dot sends (`%{.name}`), and instance fields (`%{@name}`) all work — and is stringified with `.s`. A malformed fragment is a compile error. Prefix `%` on a *computed* string interpolates reflectively at runtime instead, in the **caller's scope** (locals, `self`, and `@ivars` alike); a malformed fragment there raises a catchable `ParseError`.
> - Values are converted with `.s` before insertion.
> - ANSI strings are the `#ANSI'…'` literal (a user string mixing in `ActAsUserString`); `%`-formatting works on them too — interpolated values are markup-escaped automatically, so they can't inject styling.
> - The markup inside `#ANSI'…'` is Rich-style: `[red bold]text[/]` opens/closes a styled span (named colors use the terminal palette; `#rrggbb` is exact; `on <color>` sets the background; styles: `bold dim italic underline strike reverse blink`). Spans **nest** — `[/]` restores the *enclosing* style. A bracket run that isn't a tag is literal text (`[IO]Stdout` needs no escaping); `[[` writes a literal `[`. On a terminal the markup renders as color; anywhere else it strips.

```quoin
#(1 'two' #three).s                  "* -> #(1 two three)
#(1 'two' #three).pp                 "* -> #(1 'two' #three)
```

```quoin
'hello %' % 'world'                  "* -> 'hello world'
'%1 then %2' % #('a' 'b')            "* -> 'a then b'
'%h-%w' % #{ 'h':'hi' 'w':'world' }  "* -> 'hi-world'
var a = 'foo'; var b = 'bar';        "* the ; matters: the next line starts with an operator
%'value is %{a + b}!'                "* -> 'value is foobar!'
```

> **⚠ Gotcha — two different `%`.** Binary `%` (between a string and an argument)
> is `printf`-style substitution; prefix `%` (in front of a string literal) is
> `%{…}` interpolation. They are distinct operators with distinct selectors
> (`%:` vs `mod`). And recall `%` as an *infix arithmetic* operator is modulo —
> three roles for one glyph, disambiguated by position.

---

## 46. Namespaces

> **Rules**
> - `var name = value` declares a **reassignable local** (§4). `Name <- value` defines a **constant** global — redefining it throws (`"Global […]Name is already defined in this scope"`).
> - Namespaced names: `[NS]Name` (e.g. `[IO]File`), multi-segment `[A/B]Name`, and root `[/]Name`. A bare `Name` and `[/]Name` both refer to the **root** namespace.
> - Globals are stored by full namespace + name; namespaces are a lookup/organization mechanism, not modules with their own scope.

```quoin
Pi <- 3.14159           "* constant; a second `Pi <- …` throws
var radius = 2          "* local; reassignable

var out = [IO]Stdout    "* namespaced global
var root = [/]Object    "* explicit root; same as bare `Object`
```

> **⚠ Gotcha — constants can't be reassigned, locals can't be `<-`.** Use `<-` for
> things defined once (classes, constants) and `var` for mutable locals. Trying to
> redefine a `<-` constant is a runtime throw, not a silent overwrite.

---

## 47. File loading & packages (`use`)

> **Rules**
> - `use (pkg:)? path;` loads a `.qn` file **once** — a repeat `use` (or a cyclic one) is a no-op. It's a statement that runs when reached and evaluates to `nil`. `use` is a **soft keyword**: special only here, an ordinary identifier everywhere else.
> - **Path is the load address** (with `.qn` implied, `/`-separated); the **`[Ns]` namespace is the logical name** a file's definitions register under. The two are independent — a file may define classes, extend existing ones, add mixins, anything.
> - **Package qualifier** (`pkg:`): bare or **`std:`** = the standard library; **`self:`** = the current project; any other name is a **package** — a folder with a `quoin.toml`, found on the package search roots and installed with `qn pkg` (Part X, [§49–50](10-packages.md)).
> - **`dir/*`** globs a directory, loading every `.qn` in it in **UTF-8-sorted** order.
> - Loading is filesystem-**agnostic**: resolution goes through a host-supplied resolver (disk on the CLI; host-provided units on WASM / embedded). There is no way to load an arbitrary OS path.

These forms are illustrative — `self:` paths resolve against *your* project
(`self:helpers` names a `helpers.qn` this document doesn't ship), so the block
isn't runnable as pasted:

```quoin norun
use core/*;             "* every .qn in the stdlib's core/ dir, in sorted order
use self:helpers;       "* the current project's helpers.qn
use std:net/http;       "* explicit stdlib; `std:` and bare are the same package

MyFile <- [IO]File;     "* aliasing is just an ordinary definition — not a `use` concern
```

> **⚠ Gotcha — `use` loads, the namespace names.** `use` does not pull symbols into a
> local scope (there isn't one). It runs a file, whose `<-`/`<--` definitions register
> as ordinary namespaced globals — so you reference what a file defined by its
> `[Ns]Name`, exactly as if it had always been loaded. A second `use` of the same unit
> does nothing (definitions aren't re-run), so it can't trigger a "redefine" error.

---

## 48. Stdlib map

> **Rules**
> - The **core library is `qnlib/core/`** and loads automatically as the prelude
>   (`qnlib/prelude.qn` does `use core/*`, then seals the immediate value types).
>   Every runner mode loads it; user scripts never `use` it themselves.
> - **`net/` and `web/` are opt-in**: `use std:net/http;`, `use std:web/*;`, etc.
> - Native code (Rust, exposed as sealed built-in classes) supplies primitive
>   payloads and operations; Quoin code (`qnlib/`) supplies the abstractions on
>   top. Which is which is an implementation detail — `$doc` covers both sides
>   the same way.

| Unit | Provides |
|---|---|
| `prelude.qn` | The prelude entry — `use core/*` (sorted == numeric), then seals Integer/Double so their arithmetic can be optimized. |
| `core/00-bootstrap.qn` | `true`/`false`/`nil` behavior, `Object`, `Mixin`, the `Error` hierarchy, `Block` loops (`whileDo:`, `whileDefinedDo:`), numeric helpers, the `ANSI` class. |
| `core/01-case.qn` | `Case` and `Object#case:` pattern matching (built on the `~` match operator). |
| `core/02-iterate.qn` | The `Iterate` mixin and every combinator, plus `Generator`, the external `Iterator`, and `Set` algebra (`union:`/`intersection:`/…). |
| `core/03-number_range.qn` | `NumberRange` (`a..b`, end-exclusive): `each:`, `~` containment. |
| `core/04-string.qn` | Splitting and padding conveniences over the native String primitives. |
| `core/05-async.qn` | `Async` helpers (`joinAll:`) over the native `Async`/`Task` scheduler primitives. |
| `core/06-io.qn` | `[IO]Stdout`/`[IO]Stderr` constants, `[IO]Stdin` delegators, the one-shot `[IO]File.read:`/`write:to:`/`append:to:`, and the `Iterate`-able `[IO]Folder`. |
| `core/07-statistics.qn` | Statistics on `Iterate` — `mean`, `median`, `mode`, `variance`, `stddev`, `percentile:` for any collection of numbers. |
| `core/08-bignum.qn` | `Integer.asBigInteger` conversion (BigInteger/BigDecimal themselves are native). |
| `core/09-codecs.qn` | `toBase64`/`fromBase64`/`toHex`/`fromHex` conveniences on Bytes and String, over the native `Base64`/`Hex` codecs. |
| `core/10-parallel.qn` | The `parallelCollect:`/`parallelReduce:` combinators — a lazily-started pool of worker isolates behind the plain List API. |
| `core/11-plan.qn` | `Plan` — the lazy join graph: compose `task:`/`thread:`/`process:` leaves with `all:`/`any:`, then `await`. |
| `core/12-os.qn` | Conveniences over the native `[OS]` namespace (`[OS]Env.at:ifAbsent:`, iteration). |
| `core/tcp_server.qn` | `TcpServer` — a minimal concurrent accept-loop server (`start:`/`stop`/`join`). |
| `net/http.qn` | `[HTTP]Client` — an HTTP/1.1 client in pure Quoin over `TcpSocket`/`TlsSocket` (so HTTPS falls out for free). |
| `net/websocket.qn` | `WebSocket` — an RFC 6455 client in pure Quoin over the same sockets (`wss://` included); masked frames, reassembled fragments, auto-pong, the close handshake. |
| `lang/ast.qn` | `[Lang]Parser`/`[Lang]Node`/`[Lang]Rewrite` — the parser and a walkable AST as Quoin objects, plus span-based source rewriting. |
| `net/http_server.qn` | `[HTTP]Server` — the HTTP/1.1 server protocol machine, pure Quoin over `TcpListener`. |
| `web/00-url.qn` | `[Web]Url` — the percent codec: `encode:`/`decode:`, `queryParse:`, `formDecode:`. |
| `web/01-error.qn` | `HttpError` — throw a status (and optional body) from anywhere under a `[Web]App`. |
| `web/02-route.qn` | `[Web]Route`/`[Web]Router` — most-specific-wins path routing (`:param`, `*splat`). |
| `web/03-response.qn` | `[Web]Response` — response builders (`json:`, `text:`, …) over `[HTTP]ServerResponse`. |
| `web/04-app.qn` | `[Web]App` — the framework core: routing DSL, middleware onion, render conventions, error mapping. |
| `web/05-pool.qn` | `[Web]Pool` — multi-core request execution over worker isolates (`serve:workers:`). |
| `test.qn` | The test framework — `TestSuite`/`TestRunner`/reporters/assertions; suites self-register as files load, and `qn test DIR` runs everything collected. |

---

Next: **[Packages](10-packages.md) · [Appendices](11-appendices.md)** — cheat-sheets, the consolidated gotchas
list, and a glossary.
