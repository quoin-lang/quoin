# Part III — Objects

Classes, instances, methods, extension, the meta-object, mixins, and how a message
is dispatched to a method (including multimethods).

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · **Objects** · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Networking & the web](06-networking-and-web.md) · [Types](07-types.md) · [Tooling](08-tooling.md) · [Library & reference](09-library-and-reference.md) · [Appendices](10-appendices.md)

---

## 10. Classes, methods & extension

> **Rules**
> - `Name <- { |@x @y| … }` — define a class. Instance variables are declared in the header with `@`; they default to `nil`. Default parent is `Object`.
> - `Parent <- Child <- { … }` — define `Child` as a subclass of `Parent` (which must already exist as a class).
> - `Name <- expr` — define a **constant** (redefining it throws). Distinct from `name = value` (a local / mutable global).
> - `selector -> { … }` — add a method. `selector --> { … }` — add a method variant, but **error if the selector doesn't already exist** in the hierarchy.
> - `Class <-- { … }` — reopen a class to add instance methods. `value <-- { … }` — add singleton (eigenclass) methods to *one specific object*.
> - `.meta <-- { … }` — define class-side (static) methods. `obj.class` — an object's class; inside a method, `self` / leading `.` is the receiver.

```quoin
Point <- { |@x @y|
    .meta <-- {
        newX:y: -> { |x y| .new:{ x = x; y = y } }   "* a class-side factory
    }

    x -> { @x }
    y -> { @y }
    dist: -> { |other| (((@x - other.x) * (@x - other.x))
                      + ((@y - other.y) * (@y - other.y))).sqrt }
}

Point <- Point3D <- { |@z|                            "* subclass adds @z
    z -> { @z }

    "* Override dist: for 3D, reusing the parent's .x / .y accessors
    dist: --> { |other| (((.x - other.x) * (.x - other.x))
                       + ((.y - other.y) * (.y - other.y))
                       + ((@z - other.z) * (@z - other.z))).sqrt }
}

var p = Point.newX:3 y:4
p.x                                                   "* -> 3
var a = Point3D.new:{ x = 0; y = 0; z = 0 }
var b = Point3D.new:{ x = 1; y = 2; z = 2 }
a.dist:b                                              "* -> 3
```

Reopen a class with `<--` to add methods later; extend a single value with `<--`
to give just that object new behavior (a singleton/eigenclass, named `$Type`
internally). Note that a **sealed** class refuses both (§12) — and the value
built-ins (`Integer`, `Double`, `Boolean`, `Nil`, `List`, `Map`, `Set`,
`NumberRange`) ship sealed (the full list, and why sealing matters to the
optimizer: Part VII §34); `String` is open:

```quoin
String <-- { shout -> { .upper + '!' } }      "* every String gains shout
'code'.shout                                  "* -> 'CODE!'
var s = 'plain'
s <-- { fancy -> { '~' + self + '~' } }       "* only this one string gains fancy
s.fancy                                       "* -> '~plain~'
```

> **Note — redefining vs. adding a variant.** Within one class, a later definition
> with the **same signature** (same parameter types, no guard) *replaces* the
> earlier one: `bar -> {1}` then `bar --> {2}` makes `bar` return `2`. `-->`
> additionally requires the selector to already exist in the hierarchy. Definitions
> that differ by **parameter type** or carry a **guard** are instead kept as
> distinct *multimethod* variants (§13) and dispatched by argument, not replaced. (A
> subclass defining an inherited method — as `Point3D` does with `dist:` above —
> takes precedence for its own instances via method resolution; see §12.)

---

## 11. Construction & initialization

> **Rules**
> - `Class.new` — allocate an instance (all fields `nil`), then run the initializer chain.
> - `Class.new:{ field = … }` — allocate, run the block to populate fields, then run the initializer chain.
> - The chain runs **base → derived** (parents, then mixins, then the class itself). Each class contributes its `init:` (fed the block fields it names) if it has one, else its zero-arg `init`.
> - An **empty `new:{}` does not capture lexical scope** — fields stay at their `nil` default. Inside the block, an assignment's right-hand side resolves up the lexical chain, but the assignment binds the field in the block's own frame and never mutates the enclosing variable.

```quoin
Person <- { |@name @greeting|
    init: -> { |name| @name = name; @greeting = 'Hello, ' + name }
    greeting -> { @greeting }
}

(Person.new:{ name = 'Ada' }).greeting        "* 'Hello, Ada'
```

`new` (no block) runs the `init` of every class in the hierarchy. `new:{…}` runs
the block first (binding the fields you assign), then runs the chain, with each
class's `init:` receiving the block fields whose names match its parameters.

> **⚠ Gotcha — a plain-assignment `init:` is redundant.** Fields named in the
> `new:{…}` block are copied into the object *before* any `init:` runs, so
> `init: -> { |a| @a = a }` just re-does work already done — it behaves identically
> to having no `init:` at all. Use `init:` for *derived* or *validated* state, not
> plain copies.

> **Note — there is no `super`.** A subclass `init:` cannot call its parent's
> initializer with computed arguments; the parent runs first off the raw block
> fields. If a child needs to set a parent's field, it assigns `@field` directly.

---

## 12. Inheritance & mixins

> **Rules**
> - **Method lookup order**: the receiver's own class → its mixins (in the order added) → its parent, recursing upward. The most-derived definition wins.
> - `.mix:M` mixes class `M` into the current class; its methods and instance vars are included.
> - A mixin may declare requirements via a class-side `assertMeetsRequirements: -> { |class| … }` (typically using `class.can?:#someMethod`). It runs at the **end of the host's definition block**, so the host may define the required methods *after* the `.mix:`. If it throws, the host class is not registered.
> - **Initializer order is the dual of lookup**: base → derived (parent, then mixins, then self), so ancestors initialize first (§11).
> - **`.sealed!`** freezes a class (or, on an instance, its eigenclass): no further `<--` / `->` / `-->` / `.mix:` and no subclassing — any attempt throws *"Cannot extend sealed …"* / *"Cannot subclass sealed class …"*. **`.abstract!`** forbids instantiating the class itself (`new` / `new:` throw *"Cannot instantiate abstract class …"*), though concrete subclasses still instantiate. The two are independent. Call `.sealed!` **last** in a body — defs *after* it are rejected.
> - `obj.can?:X` is **overloaded**: a `Symbol`/`String` selector asks *"does it implement that method?"*; a `Class` asks *"is it an instance of / does it mix in that class?"* — e.g. `list.can?:#each:`, `list.can?:'each:'`, `list.can?:Iterate`. Works on instance, class, and metaclass receivers.

```quoin
Greeter <- { hello -> { 'hi from ' + .class.name.s } }

Widget <- {
    .mix:Greeter
    name -> { 'widget' }
}

Widget.new.hello       "* 'hi from Widget'   (found via the mixin)
```

---

## 13. Multimethod dispatch

> **Rules**
> - A selector can have several definitions distinguished by argument **type** and/or a **guard block**; dispatch picks the most specific matching one at call time.
> - Candidates are ranked by **specificity**: a more specific parameter type wins (`Integer` beats `Object`, and an unannotated parameter counts as `:Object`), and a **guard refines** specificity — a guarded variant outranks an otherwise-equal *unguarded* one.
> - **Definition order is not a tiebreaker.** Two distinct candidates that are equally specific *and* both match are ambiguous → `AmbiguousMethodError` (which lists the tied candidates). Keep guards mutually exclusive or distinguish by type; for *ordered* "first match wins" semantics use `case`/`~` instead.
> - Typed parameter: `|x:Integer|` — namespaced classes work too (`|e:[Web]Halt|`), and a bare name always means the **root** namespace (it never matches some `[X]Name` by leaf name). Guard block: `|x { x > 5 }|` or `|x:Type { … }|` — the guard must return a truthy value to match. Inside a guard the arguments are bound **by name** and `self` is the **receiver**, so a guard can also use the class's instance variables and other methods.
> - No matching variant → `MessageNotUnderstood` — and if the selector *does* exist with non-matching argument types, the error lists those filtered-out variants as a hint.

```quoin
Describer <- {
    describe: -> { |n:Integer| 'int ' + n.s }
    describe: --> { |s:String|  'str ' + s }
    describe: --> { |n:Integer { n > 100 }| 'big number' }   "* a guard refines :Integer
}

var d = Describer.new
d.describe:5         "* -> 'int 5'
d.describe:'hi'      "* -> 'str hi'
d.describe:150       "* -> 'big number'
```

Type-based variants are the right tool when you want different behavior per
argument type; the dispatcher chooses the most specific match: `5` fails the
guard, so the plain `:Integer` variant handles it, while `150` passes it and the
guarded variant outranks the unguarded one.

---

Next: **[Part IV — Patterns & errors](04-patterns-and-errors.md)**.
