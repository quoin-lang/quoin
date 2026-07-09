# Quoin

Quoin (extension `.qn`) is a small, dynamically-typed, object-oriented language in
the Smalltalk tradition: everything is an object, everything happens by sending
messages, and control flow is just blocks responding to messages. It runs on a
stack-based bytecode VM written in Rust, with a tracing garbage collector and
stackful coroutines ("fibers") that power generators, lazy iteration, and
cooperative concurrency.

```b
Person <- { |@name @age|
    init: -> { |name age|
        @name = name
        @age = age
    }

    name -> { @name }
    age  -> { @age }

    s --> { %'Mr %{@name}' }
}

damon = Person.new:{
    name = 'Damon'
    age  = 347
}

damon.s.print   "* Mr Damon
```

---

## The Language

### Syntax basics

- **Comments** start with `"`. `"* ...` runs to end of line; `" ... "` is an
  inline comment delimited by a closing quote.
- **Assignment** uses `=`. Identifiers prefixed with `@` are instance variables.
- **Three special literals** — `true`, `false`, `nil` — are just read-only
  variables holding the singleton instances of `Boolean`, `Boolean`, and `Nil`.

```b
name = 'Damon'        "* local variable
@age = 347            "* instance variable
true.class == Boolean
nil.class  == Nil
```

### Literals

| Kind    | Syntax                          |
| ------- | ------------------------------- |
| Integer | `42`                            |
| Double  | `3.14`                          |
| String  | `'hello'`                       |
| List    | `#( 1 2 3 )`                    |
| Map     | `#{ 'a': 10 'b': 10 + 10 }`     |
| Regex   | `#/^[A-Z]+$/`                   |
| Block   | `{ 3 * 4 }` or `{ |a b| a > b }`|

### Everything is a message send

Method calls are Smalltalk-style keyword messages. Unary messages take no
argument; keyword messages name each argument with a trailing colon.

```b
'hello'.length              "* unary
person.name:'Damon'         "* one keyword argument
dictionary.at:'k' put:'v'   "* two keyword arguments: selector is at:put:
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

```b
Integer <-- {
    square -> { self * self }
}

Person <- Employee <- { |@title|
    init: --> { |title| @title = title }
    title -> { @title }
}
```

Behavior can be mixed in with `.mix:` / `.can:`:

```b
MyCollection <- { |@items|
    .mix:Iterate;
    each: -> { |b| @items.each:b }
}
```

### Multi-methods (dispatch by type and by guard)

A selector can have several definitions that dispatch on the **runtime type** of
the argument. The first matching, most specific definition wins.

```b
Multi <- {
    x: ->  { |x:Integer| 'Integer: %' % x }
    x: --> { |x:String|  'String: %'  % x }
    x: --> { |x:Object|  'Other: %'   % x }
}

Multi.new.x:77      "* 'Integer: 77'
Multi.new.x:'Hi'    "* 'String: Hi'
Multi.new.x:true    "* 'Other: true'
```

Operators can be overloaded the same way using the `#'selector:'` symbol form:

```b
FakeNumber <- {
    #'+:' ->  { |n:Object|     42 }
    #'+:' --> { |n:FakeNumber| 99 }
}
```

### Blocks (closures)

Blocks are first-class closures. You run a block by sending it a `value...`
message; arguments are declared in the `| ... |` header and may carry type
annotations.

```b
{ 3 * 4 }.value                                         "* 12
{ |a b| a > b }.valueWithArgs:#(10 20)                  "* false
{ |a:Double b:Integer| a + b }.valueWithArgs:#(2.3 10)  "* 12.3
```

Common entry points: `.value`, `.value:arg`, `.valueWithArgs:#(...)`,
value`.valueWithSelfOrArg:x`. `^expr` returns from the current block; `^^expr` is a
non-local return from the enclosing method.

### Control flow

Conditionals and loops are just messages to booleans and blocks.

```b
(1 < 10).if:{ 'True'.print }
       else:{ 'False'.print }

i = 1
{ i < 5 }.whileDo:{ i.print; i = i + 1 }
```

### Case statements

`case:` pattern-matches against literals, regexes, ranges, predicate blocks, or any other object that implements the `~` match operator. Like everything, case statements are just methods and blocks, you can build your own.

```b
name.case:{
    .when:'Damon'              do:{ 'Hi Damon'.print          };
    .when:#/^[A-Z]+$/          do:{ 'NO NEED TO SHOUT'.print  };
    .when:{|x| x.length > 20 } do:{ 'YOUR NAME IS LONG'.print };
    .default:{ 'Who are you?'.print };
}

5.case:{
    .when:1..10  do:'Low';
    .when:11..20 do:'High';
}
```

### Iteration

Iteration is a protocol: implement `each:` and the `Iterate` mixin gives you the
whole collections library for free.

```b
(1..4).collect:{ |n| n * 10 }     "* #( 10 20 30 )
(1..4).select:{ |n| n % 2 == 0 }  "* #( 2 )
(1..4).reduce:{ |sum n| sum + n } "* 6
(1..4).zip:(4..7)                 "* #( #(1 4) #(2 5) #(3 6) )
```

```b
MyRange <- { |@start @end|
    .mix:Iterate
    each: -> { |b|
        i = @start
        { i < @end }.whileDo:{ b.valueWithSelfOrArg:i; i = i + 1 }
    }
}
```

An external pull-style iterator is also available:

```b
it = #(10 20 30).iterator
it.hasNext?   "* true
it.next       "* 10
```

### Fibers, generators, and `^>`

Fibers are stackful coroutines with two-way communication. The `^>` operator is
sugar for `Fiber.yield:` and is an **expression** — it evaluates to the value
passed back in on the next `resume:`.

```b
f = Fiber.new:{ |x|
    a = ^> (x + 1)
    a + 100
}
f.resume:10   "* 11   (yields x + 1)
f.resume:5    "* 105  (a = 5, returns a + 100)
```

Because fibers are stackful, `^>` works even from deep inside an iterator or a
native method. A `Generator` turns a yielding block into a re-runnable, lazy
iterable:

```b
g = Generator.from:{ ^>1; ^>2; ^>3 }
g.collect:{ |x| x }   "* #(1 2 3)

naturals = Generator.from:{ n = 0; { true }.whileDo:{ ^>n; n = n + 1 } }
naturals.take:5                                   "* #(0 1 2 3 4)
(naturals.lazySelect:{ |x| x % 2 == 0 }).take:4   "* #(0 2 4 6)
```

### Error handling

Any object can be thrown; `catch:` blocks dispatch on the thrown object's type,
and chains may end in `finally:`.

```b
Error <- CustomError <- {}

{ CustomError.new.throw }
    .catch:  { |ex:CustomError| 'Logic for CustomError'   }
    .catch:  { |ex:Error|       'Logic for other errors'  }
    .finally:{ 'Always runs'                              };
```

### Operators, ranges, formatting, destructuring

```b
"* Operators: + - * / == != > >= < <= .. % ~   (unary: + - ! %)
'Hello' ~ #/llo/         "* regex match → true

"* Ranges (end-exclusive)
(1..5).list              "* #(1 2 3 4)
(1..5) ~ 3               "* true (membership)

"* String formatting: positional % and %{ } interpolation
'foo%baz' % 'bar'        "* 'foobarbaz'
a = 'b'; b = 'a'; c = 'r'
%'foo%{a+b+c}baz'        "* 'foobarbaz'

"* Destructuring in assignment and via bind:
a *b c = #(1 2 3 4 5 6)  "* a=1, b=#(2 3 4 5), c=6
('1/2/3'.split:'/').bind:{ |a b c| ... }
```

---

## VM Design

The implementation lives in `src/` and is a classic compile-to-bytecode VM,
extended with a GC-aware execution model and stackful fibers.

```
source (.qn)  →  parse  →  compile  →  bytecode  →  VM step loop  →  result
                                          │
                              gc-arena heap · corosensei fibers
```

### Parser

The parser is written using [`Pest`](https://github.com/pest-parser/pest) in `src/parser/pest/`.
It's a PEG grammar with a Pratt parser for operator precedence.
It emits a `Node` tree carrying optional `SourceInfo` (file, line,
column) so that runtime errors can show highlighted source snippets.

### Compiler and bytecode

`src/compiler.rs` is a single-pass, tree-walking compiler that lowers the AST to
a **stack-based** instruction set defined in `src/instruction.rs`. Representative
instructions:

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

Every runtime value is a `Value` — a GC pointer to an `Object`, a `Class`,
or a class's metaclass. An object carries a unique id, its class, a field map,
and a payload identifying its built-in kind.

```rust
enum ObjectPayload {
    Nil, Bool(bool), Int(i64), Double(f64),
    String(String),
    Block(Block),
    Native(NativeFunc),
    Instance,
    NativeState(Opaque),
}
```

Classes hold their parent, instance-variable list, instance/class method tables,
and mixins. Method dispatch walks the receiver's class, its mixins, then its
parent chain; dispatch targets are `Callable` trait objects, which lets guest
blocks, native functions, and class constructors all be invoked uniformly. The
built-in classes (`Object`, `Nil`, `Boolean`, `Integer`, `Double`, `String`,
`List`, `Map`, `Regex`, `Block`, `Class`, `Fiber`, `Method`, `Timer`) are
defined in Rust under `src/runtime/` and registered at startup. Native classes
that wrap Rust data (e.g. `List`, `Map`) keep it in a `NativeState` payload whose
`AnyCollect::trace_gc` traces any `Value`s it contains.

### Garbage collection

Memory is managed by the [`gc-arena`](https://crates.io/crates/gc-arena) crate.
All heap references share the arena lifetime `'gc`; interior mutability goes
through `RefLock`, and mutating a GC object requires a `Mutation<'gc>` token. The
runner drives execution one step at a time and lets the collector run between
steps:

```rust
loop {
    arena.mutate_root(|mc, vm| vm.step(mc));  // execute one instruction
    arena.collect_debt();                     // incremental GC outside the borrow
}
```

### Execution loop

`VmState` (`src/vm.rs`) holds the evaluation stack, a stack of call `Frame`s
(each with its own instruction pointer, environment, receiver, and arguments),
the global table, and fiber-scheduling state. `step` fetches the instruction at
the current frame's `ip`, executes it, and returns a status:

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

### The `no_gc_across_yield` lint

A subtle hazard: when a fiber suspends, its native Rust stack is frozen, and any
`Gc<'gc, T>`/`Value<'gc>` living only in a local on that frozen stack is
invisible to the collector and can be swept out from under you. The rule for
native methods is therefore **flush GC values into GC-rooted storage before
yielding**, then re-read them after resuming.

To enforce this mechanically, the workspace ships a custom
[Dylint](https://crates.io/crates/cargo-dylint) lint at
`lint/no_gc_across_yield/`. It runs as a `LateLintPass`, finds yield points, and
flags any `'gc`-lifetime local that is held live across a suspend — turning a
memory-safety invariant into a compile-time check. See `docs/LINTER_DESIGN.md`.

### Running it

```sh
cargo run            # run the VM
cargo run -- ...     # see src/runner.rs for modes: run / test / benchmark / highlight
```

The `qnlib/` directory holds the standard library and bootstrap code
(`00-bootstrap.qn` … `06-io.qn`), a guided language tour under
`qnlib/presentation/`, and the test suite under `qnlib/tests/`.

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
