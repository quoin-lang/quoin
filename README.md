# Quoin

Quoin (extension `.qn`) is a small, dynamically-typed, object-oriented language in
the Smalltalk tradition: everything is an object, everything happens by sending
messages, and control flow is just blocks responding to messages. It runs on a
stack-based bytecode VM written in Rust, with a tracing garbage collector and
stackful coroutines ("fibers") that power generators, lazy iteration, and
cooperative concurrency.

```quoin
Person <- { |@name @age|
    init: -> { |name age|
        @name = name
        @age = age
    }

    name -> { @name }
    age  -> { @age }

    s --> { 'Mr %' % @name }
}

var damon = Person.new:{
    name = 'Damon'
    age  = 347
}

damon.s.print   "* Mr Damon
```

## Quickstart

There are no prebuilt binaries yet — build from source with a Rust toolchain:

```sh
cargo build --release        # binary lands at target/release/qn

qn -e "'hello'.print"        # hello, world
qn program.qn                # run a file
qn repl                      # start the interactive REPL
```

## Documentation

- **The language book** — [`docs/language/README.md`](docs/language/README.md):
  Parts I–IX take you from lexical structure through objects, concurrency,
  networking, the gradual type system, tooling, and the standard library. Every
  example in it is CI-checked against the VM.
- **The API reference** is generated: run `qn doc` (writes HTML to `qn-docs/`),
  or ask the REPL with `$doc Name` / `$doc Name.selector`.

---

## The Language

### Syntax basics

- **Comments** start with `"`. `"* ...` runs to end of line; `" ... "` is an
  inline comment delimited by a closing quote.
- **Declaration vs. assignment**: locals are declared with `var` (mutable) or
  `let` (immutable); plain `=` assigns to an *already-declared* binding, and
  assigning an undeclared name is a compile error. Identifiers prefixed with `@`
  are instance variables, declared in the class header.
- **Three special literals** — `true`, `false`, `nil` — are just read-only
  variables holding the singleton instances of `Boolean`, `Boolean`, and `Nil`.

```quoin
var name = 'Damon'      "* declare a mutable local
let year = 2026         "* declare an immutable one
name = 'Ada'            "* plain `=` assigns to an existing binding
true.class == Boolean   "* -> true
nil.class == Nil        "* -> true
```

### Literals

| Kind    | Syntax                          |
| ------- | ------------------------------- |
| Integer | `42`                            |
| Double  | `3.14`                          |
| String  | `'hello'`                       |
| Symbol  | `#name`, `#at:put:`, `#'+:'`    |
| List    | `#( 1 2 3 )`                    |
| Map     | `#{ 'a': 10 'b': 10 + 10 }`     |
| Set     | `#<1 2 3>`                      |
| Range   | `1..5` (end-exclusive)          |
| Regex   | `#/^[A-Z]+$/`                   |
| Block   | `{ 3 * 4 }` or `{ \|a b\| a > b }`|

### Everything is a message send

Method calls are Smalltalk-style keyword messages. Unary messages take no
argument; keyword messages name each argument with a trailing colon.

```quoin
'hello'.length          "* -> 5
var d = #{ 'a': 1 }
d.at:'k' put:'v'        "* two keyword arguments: the selector is at:put:
d.at:'k'                "* -> 'v'
```

Even operators are messages. `3 * 4` is `Send(3, '*:', 4)` and `4.abs` is
`Send(4, 'abs')`.

### Classes, subclasses, and extensions

Three operators shape the class graph:

- `Name <- { ... }` — define a class.
- `Parent <- Child <- { ... }` — define `Child` as a subclass of `Parent`.
- `Name <-- { ... }` — reopen / extend an existing class.

Instance variables are declared in the `| ... |` header. Methods are written as
`selector -> { block }`; `selector --> { block }` adds an additional
(overloaded) method for the same selector — see multi-methods below.

```quoin
Person <- { |@name|
    init: -> { |name| @name = name }
    name -> { @name }
}

Person <- Employee <- { |@title|
    title -> { @title }
}

String <-- {
    shout -> { self.upper + '!' }
}

var e = Employee.new:{ name = 'Ada'; title = 'Engineer' }
e.name + ' the ' + e.title   "* -> 'Ada the Engineer'
'hello'.shout                "* -> 'HELLO!'
```

The built-in value classes (`Integer`, `Double`, `Boolean`, `Nil`, `List`, `Map`,
`Set`, `NumberRange`) ship **sealed** — they can't be reopened or subclassed,
which is what lets the optimizer compile their operations directly; `String` and
user classes are open (the full story is in the book's
[Part VII](docs/language/07-types.md)).

Behavior can be mixed in with `.mix:`:

```quoin
MyCollection <- { |@items|
    .mix:Iterate;
    each: -> { |b| @items.each:b }
}

var c = MyCollection.new:{ items = #(3 1 2) }
c.collect:{ |n| n * 10 }   "* -> #(30 10 20)
```

### Multi-methods (dispatch by type and by guard)

A selector can have several definitions that dispatch on the **runtime type** of
the argument. The first matching, most specific definition wins.

```quoin
Multi <- {
    x: ->  { |x:Integer| 'Integer: %' % x }
    x: --> { |x:String|  'String: %'  % x }
    x: --> { |x:Object|  'Other: %'   % x }
}

Multi.new.x:77      "* -> 'Integer: 77'
Multi.new.x:'Hi'    "* -> 'String: Hi'
Multi.new.x:true    "* -> 'Other: true'
```

Operators can be overloaded the same way using the `#'selector:'` symbol form:

```quoin
FakeNumber <- {
    #'+:' ->  { |n:Object|     42 }
    #'+:' --> { |n:FakeNumber| 99 }
}

FakeNumber.new + 1                "* -> 42
FakeNumber.new + FakeNumber.new   "* -> 99
```

### Blocks (closures)

Blocks are first-class closures. You run a block by sending it a `value...`
message; arguments are declared in the `| ... |` header and may carry type
annotations.

```quoin
{ 3 * 4 }.value                                         "* -> 12
{ |a b| a > b }.valueWithArgs:#(10 20)                  "* -> false
{ |a:Double b:Integer| a + b }.valueWithArgs:#(2.3 10)  "* -> 12.3
```

Common entry points: `.value`, `.value:arg`, `.valueWithArgs:#(...)`,
`.valueWithSelfOrArg:x`. `^expr` returns from the current block; `^^expr` is a
non-local return from the enclosing method.

### Control flow

Conditionals and loops are just messages to booleans and blocks.

```quoin
(1 < 10).if:{ 'True'.print }
       else:{ 'False'.print }

var i = 1
{ i < 5 }.whileDo:{ i.print; i = i + 1 }
i   "* -> 5
```

### Case statements

`case:` pattern-matches against literals, regexes, ranges, predicate blocks, or any other object that implements the `~` match operator. Like everything, case statements are just methods and blocks, you can build your own.

```quoin
var name = 'Damon'
name.case:{
    .when:'Damon'              do:{ 'Hi Damon'.print          };
    .when:#/^[A-Z]+$/          do:{ 'NO NEED TO SHOUT'.print  };
    .when:{|x| x.length > 20 } do:{ 'YOUR NAME IS LONG'.print };
    .default:{ 'Who are you?'.print };
}
```

```quoin
5.case:{
    .when:1..10  do:'Low';
    .when:11..20 do:'High';
}   "* -> 'Low'
```

### Iteration

Iteration is a protocol: implement `each:` and the `Iterate` mixin gives you the
whole collections library for free.

```quoin
(1..4).collect:{ |n| n * 10 }     "* -> #(10 20 30)
(1..4).select:{ |n| n % 2 == 0 }  "* -> #(2)
(1..4).reduce:{ |sum n| sum + n } "* -> 6
(1..4).zip:(4..7)                 "* -> #(#(1 4) #(2 5) #(3 6))
```

```quoin
MyRange <- { |@start @end|
    .mix:Iterate;
    each: -> { |b|
        var i = @start
        { i < @end }.whileDo:{ b.valueWithSelfOrArg:i; i = i + 1 }
    }
}

var r = MyRange.new:{ start = 2; end = 5 }
r.collect:{ |n| n * n }   "* -> #(4 9 16)
```

An external pull-style iterator is also available:

```quoin
var it = #(10 20 30).iterator
it.hasNext?   "* -> true
it.next       "* -> 10
```

### Fibers, generators, and `^>`

Fibers are stackful coroutines with two-way communication. The `^>` operator is
sugar for `Fiber.yield:` and is an **expression** — it evaluates to the value
passed back in on the next `resume:`. Below, the first `resume:` passes `10` in
as `x` and gets the yielded `11` back; the second passes `5` in as `a`'s value
and gets the final `105`.

```quoin
var f = Fiber.new:{ |x|
    var a = ^> (x + 1)
    a + 100
}
f.resume:10   "* -> 11
f.resume:5    "* -> 105
```

Because fibers are stackful, `^>` works even from deep inside an iterator or a
native method. A `Generator` turns a yielding block into a re-runnable, lazy
iterable:

```quoin
var g = Generator.from:{ ^>1; ^>2; ^>3 }
g.collect:{ |x| x }   "* -> #(1 2 3)

var naturals = Generator.from:{ var n = 0; { true }.whileDo:{ ^>n; n = n + 1 } }
naturals.take:5                                   "* -> #(0 1 2 3 4)
(naturals.lazySelect:{ |x| x % 2 == 0 }).take:4   "* -> #(0 2 4 6)
```

### Error handling

Any object can be thrown. Typed `catch:` handlers dispatch on the thrown
object's type — chain further `catch:` keywords for more handlers (first match
wins) and end with `finally:` to run code on every path.

```quoin
Error <- CustomError <- {}

{ CustomError.new.throw }
    .catch:{ |ex:CustomError| 'Logic for CustomError'  }
     catch:{ |ex:Error|       'Logic for other errors' }
   finally:{ 'Always runs'.print }   "* -> 'Logic for CustomError'
```

### Operators, ranges, formatting, destructuring

Ranges are **end-exclusive**. The match operator `~` puts the *matcher* on the
left (a regex, range, class, or predicate block). `%` formats positionally, and
`%'…%{expr}…'` interpolates over the surrounding locals.

```quoin
"* Operators: + - * / == != > >= < <= .. % ~   (unary: + - ! %)
#/llo/ ~ 'Hello'                             "* -> true
(1..5) ~ 3                                   "* -> true
(1..5).list                                  "* -> #(1 2 3 4)

'foo%baz' % 'bar'                            "* -> 'foobarbaz'
var a = 'b'; var b = 'a'; var c = 'r';
%'foo%{a+b+c}baz'                            "* -> 'foobarbaz'

var first *rest = #(1 2 3 4)                 "* first=1, rest=#(2 3 4)
('1/2/3'.split:'/').bind:{ |x y z| x + z }   "* -> '13'
```

---

## VM Design

The implementation lives in `src/` (with the parser in its own crate,
`crates/quoin-syntax`) and is a classic compile-to-bytecode VM, extended with a
GC-aware execution model and stackful fibers.

```
source (.qn)  →  parse  →  compile  →  bytecode  →  VM step loop  →  result
                                          │
                              gc-arena heap · corosensei fibers
```

Optimization is layered over that pipeline: the compiler devirtualizes and
inlines message sends using type annotations and sealed classes, and at runtime
inline caches and observed-type speculation feed an AOT tier that translates
hot methods and blocks into compiled frames — so untyped code reaches typed
speed once its concrete types stabilize.

### Parser

The parser lives in its own crate,
[`crates/quoin-syntax`](crates/quoin-syntax), written with
[`Pest`](https://github.com/pest-parser/pest): a PEG grammar with a Pratt parser
for operator precedence. It emits a `Node` tree carrying optional `SourceInfo`
(file, line, column) so that runtime errors can show highlighted source
snippets; the same crate powers `qn highlight` and the REPL's tab completion.

### Compiler and bytecode

`src/compiler/` is a tree-walking compiler that lowers the AST to a
**stack-based** instruction set defined in `src/instruction.rs`, with
devirtualization and inlining passes that exploit type annotations and sealed
classes. Representative instructions:

| Category    | Instructions                                                               |
| ----------- |----------------------------------------------------------------------------|
| Stack       | `Push`, `Pop`, `Dup`                                                       |
| Variables   | `Load/StoreLocal`, `DefineLocal`, `Load/StoreGlobal`, `Load/StoreField`    |
| Messages    | `Send(selector, num_args)`, `Return`                                       |
| Control     | `Jump`, `IfJump`, `ElseJump`                                               |
| Collections | `NewList`, `NewMap`, `NewRegex`                                            |
| OOP         | `DefineClass`, `DefineMethod`, `OverrideMethod`, `ExecuteBlockWithSelf`    |
| Errors      | `Yeet` (throw)                                                             |

### Values and the object model

Every runtime value is a `Value`. Small scalars — integers, doubles, booleans,
`nil` — are **unboxed immediates** whose class is derived from the variant, so
"everything is an object" still holds without a heap allocation. Everything else
is a GC pointer to an `Object`, a `Class`, or a class's metaclass; an object
carries a unique id, its class, a field map, and a payload identifying its
built-in kind (`src/value.rs`, abridged):

```rust
enum Value<'gc> {
    Int(i64), Double(f64), Bool(bool), Nil,          // unboxed immediates
    Object(...), Class(...), ClassMeta(...),          // GC pointers
}

enum ObjectPayload<'gc> {
    String(...), Symbol(...), Bytes(...),
    Block(...),
    Instance,
    NativeState(...),   // wrapped native Rust state
}
```

Classes hold their parent, instance-variable list, instance/class method tables,
and mixins. Method dispatch walks the receiver's class, its mixins, then its
parent chain; dispatch targets are `Callable` trait objects, which lets guest
blocks, native functions, and class constructors all be invoked uniformly.
Built-in classes such as `Object`, `Boolean`, `Integer`, `Double`, `String`,
`Bytes`, `List`, `Map`, `Set`, `Regex`, `Block`, `Fiber`, `Channel`, and `Timer`
are defined in Rust under `src/runtime/` and registered at startup — together
with the Quoin-level stdlib the API reference documents 110 classes (run
`qn doc`). Native classes that wrap Rust data (e.g. `List`, `Map`) keep it in a
`NativeState` payload whose `AnyCollect::trace_gc` traces any `Value`s it
contains.

### Garbage collection

Memory is managed by the [`gc-arena`](https://crates.io/crates/gc-arena) crate.
All heap references share the arena lifetime `'gc`; interior mutability goes
through `RefLock`, and mutating a GC object requires a `Mutation<'gc>` token. The
runner drives execution in bounded batches of steps and lets the collector run
between them, outside the arena borrow (simplified):

```rust
loop {
    arena.mutate_root(|mc, vm| vm.step(mc));  // execute a batch of instructions
    arena.collect_debt();                     // incremental GC outside the borrow
}
```

### Execution loop

`VmState` (`src/vm.rs`) holds the evaluation stack, a stack of call `Frame`s
(each with its own instruction pointer, environment, and receiver), the global
table, and fiber-scheduling state. `step` fetches the instruction at the current
frame's `ip`, executes it, and returns a status:

```rust
enum VmStatus {
    Running,
    Finished(Value),  // top-level block returned
    Yeeted(Value),    // uncaught exception
}
```

Thrown exceptions are parked in a GC-rooted `active_exception` slot rather than
being carried inside the Rust error type, so the exception object stays visible
to the collector while it propagates to a matching `catch:`.

### Fibers

Fibers are real stackful coroutines built on
[`corosensei`](https://crates.io/crates/corosensei) (~10–20 ns context switches,
stable-Rust inline assembly). Each fiber runs the same VM step loop; when guest
code yields, the coroutine suspends with a `YieldReason` describing what it
wants (call a block, cooperatively yield, resume/yield another fiber, return).
The runner owns the fiber pointers and performs context switches by saving and
restoring each fiber's stack and frames.

### The yield-safety lints

A subtle hazard: when a fiber suspends, its native Rust stack is frozen, and any
`Gc<'gc, T>`/`Value<'gc>` living only in a local on that frozen stack is
invisible to the collector and can be swept out from under you. The rule for
native methods is therefore **flush GC values into GC-rooted storage before
yielding**, then re-read them after resuming.

To enforce this mechanically, the workspace ships a custom
[Dylint](https://crates.io/crates/cargo-dylint) lint at
`lint/no_gc_across_yield/`. It runs as a `LateLintPass`, finds yield points, and
flags any `'gc`-lifetime local that is held live across a suspend — turning a
memory-safety invariant into a compile-time check.

A companion lint, `lint/no_borrow_across_yield/`, guards the other suspend
hazard: a `RefCell`/`RefLock` borrow guard still alive at a yield point. Any
other borrow of the same cell — from another task, or re-entrantly from the
Quoin code the suspended call was running — panics "already borrowed", and the
guard doesn't have to be *used* after the yield for that to happen, so this
lint reasons about drop scopes rather than uses. Both lints run under
`cargo lint` and in CI; see `docs/internal/LINTER_DESIGN.md`.

### The `qn` CLI

| Command               | What it does                                                                 |
| --------------------- | ---------------------------------------------------------------------------- |
| `qn program.qn [args]`| Run a program; arguments after the file are passed to it (`Runtime.arguments`). |
| `qn -e "<expr>"`      | Evaluate one expression and print its result.                                 |
| `qn repl`             | Interactive read-eval-print loop (`$help` lists the `$`-commands).            |
| `qn test DIR`         | Run the test suites in a directory (`--coverage` emits lcov/cobertura).       |
| `qn check PATH...`    | Type-check files and report diagnostics without running them.                 |
| `qn fmt PATH...`      | Format Quoin source in place (`--check`, `--diff`, `--dry-run`).              |
| `qn debug FILE`       | Run under the interactive debugger; `--dap` speaks the Debug Adapter Protocol on stdio. |
| `qn doc [PATH...]`    | Generate the HTML API reference; `--check` runs the documentation's fenced examples. |
| `qn highlight FILE`   | Print syntax-highlighted source (`--html` for a standalone page).             |
| `qn benchmark`        | Run the built-in benchmarks.                                                  |

The standard library lives in `qnlib/` and is embedded in the binary at build
time: the core units (`qnlib/core/00-bootstrap.qn` … `12-os.qn`, plus
`tcp_server.qn`), the `net/` and `web/` units loaded by `use std:net/*` /
`use std:web/*`, the `test.qn` harness, and `prelude.qn`. A guided language tour
lives under `qnlib/presentation/`, and the test suite under `qnlib/tests/`.

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
