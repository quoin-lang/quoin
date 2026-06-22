# Part III — Objects

Classes, instances, methods, extension, the meta-object, mixins, and how a message
is dispatched to a method (including multimethods).

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · **Objects** · [Patterns & errors](04-patterns-and-errors.md) · [Concurrency & iteration](05-concurrency-and-iteration.md) · [Library & reference](06-library-and-reference.md) · [Appendices](07-appendices.md)

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

p = Point.newX:3 y:4
p.x                                                   "* 3

a = Point3D.new:{ x = 0; y = 0; z = 0 }
b = Point3D.new:{ x = 1; y = 2; z = 2 }
a.dist:b                                              "* 3
```

Reopen a class with `<--` to add methods later; extend a single value with `<--`
to give just that object new behavior (a singleton/eigenclass, named `$Type`
internally):

```quoin
Integer <-- { doubled -> { self + self } }    "* every Integer gains doubled
21.doubled                                     "* 42

n = 42
n <-- { greet -> { 'hi from this 42' } }       "* only this object gains greet
n.greet                                        "* 'hi from this 42'
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
> plain copies. See `QUOIN_TODO.md` → *Bugs/Odd Behavior* for the full write-up of
> the `new:{}` scoping rules and the (intentionally absent) lexical capture.

> **Note — there is no `super`.** A subclass `init:` cannot call its parent's
> initializer with computed arguments; the parent runs first off the raw block
> fields. If a child needs to set a parent's field, it assigns `@field` directly.

---

## 12. Inheritance & mixins

> **Rules**
> - **Method lookup order**: the receiver's own class → its mixins (in the order added) → its parent, recursing upward. The most-derived definition wins.
> - `.mix:M` mixes class `M` into the current class; its methods and instance vars are included. (There is no `.can:` alias — use `.mix:`.)
> - A mixin may declare requirements via a class-side `assertMeetsRequirements: -> { |class| … }` (typically using `class.can?:#someMethod`). It runs at the **end of the host's definition block**, so the host may define the required methods *after* the `.mix:`. If it throws, the host class is not registered.
> - **Initializer order is the dual of lookup**: base → derived (parent, then mixins, then self), so ancestors initialize first (§11).
> - **`.sealed!`** freezes a class (or, on an instance, its eigenclass): no further `<--` / `->` / `-->` / `.mix:` and no subclassing — any attempt throws *"Cannot extend sealed …"* / *"Cannot subclass sealed class …"*. **`.abstract!`** forbids instantiating the class itself (`new` / `new:` throw *"Cannot instantiate abstract class …"*), though concrete subclasses still instantiate. The two are independent. Call `.sealed!` **last** in a body — defs *after* it are rejected.
> - `obj.can?:X` is **overloaded**: a `Symbol`/`String` selector asks *"does it implement that method?"*; a `Class` asks *"is it an instance of / does it mix in that class?"* — e.g. `list.can?:#each:`, `list.can?:'each:'`, `list.can?:Iterate`. Works on instance, class, and metaclass receivers.
> - The built-in `ActAsUserList` / `ActAsUserString` mixins are what enable the `#Name( … )` and `#Name'…'` custom-literal forms.

```quoin
Greeter <- { hello -> { 'hi from ' + .class.name.s } }

Widget <- {
    .mix:Greeter
    name -> { 'widget' }
}

Widget.new.hello       "* 'hi from Widget'   (found via the mixin)
```

> **⚠ Gotcha — seal last.** `.sealed!` takes effect immediately, so any `->` / `-->`
> / `.mix:` *after* it in the same class body is rejected. Put `.sealed!` at the end of
> the body (or call `Foo.sealed!` after the definition). `.abstract!` doesn't have this
> issue — it only blocks instantiation, not extension.

---

## 13. Multimethod dispatch

> **Rules**
> - A selector can have several definitions distinguished by argument **type** and/or a **guard block**; dispatch picks the most specific matching one at call time.
> - Candidates are ranked by **specificity**: a more specific parameter type wins (`Integer` beats `Object`, and an unannotated parameter counts as `:Object`), and a **guard refines** specificity — a guarded variant outranks an otherwise-equal *unguarded* one.
> - **Definition order is not a tiebreaker.** Two distinct candidates that are equally specific *and* both match are ambiguous → `AmbiguousMethodError` (which lists the tied candidates). Keep guards mutually exclusive or distinguish by type; for *ordered* "first match wins" semantics use `case`/`~` instead.
> - Typed parameter: `|x:Integer|`. Guard block: `|x { x > 5 }|` or `|x:Type { … }|` — the guard must return a truthy value to match. Inside a guard the arguments are bound **by name** and `self` is the **receiver**, so a guard can also use the class's instance variables and other methods.
> - No matching variant → `MessageNotUnderstood` — and if the selector *does* exist with non-matching argument types, the error lists those filtered-out variants as a hint.

```quoin
describe: -> { |n:Integer| 'int ' + n.s }
describe: --> { |s:String|  'str ' + s }
describe: --> { |n:Integer { n > 100 }| 'big number' }   "* a guard refines :Integer

describe:5         "* 'int 5'        (the guard fails, so plain :Integer matches)
describe:'hi'      "* 'str hi'
describe:150       "* 'big number'   (the guarded :Integer beats the unguarded one)
```

Type-based variants are the right tool when you want different behavior per
argument type; the dispatcher chooses the most specific match.

> **⚠ Gotcha — equal-specificity matches are ambiguous, not ordered.** If two
> variants match the same argument with the **same** type-specificity *and* the same
> guard status (e.g. two overlapping guards on the same type that both pass), neither
> is preferred — dispatch raises `AmbiguousMethodError` rather than picking by
> definition order. A *guarded* variant always beats an equal-typed *unguarded* one,
> so the usual idiom is specific guarded variants plus one unguarded catch-all
> (`|x|`), which is unambiguous. (Two variants with the **same signature and no
> guard** don't coexist at all: the later one *replaces* the earlier — §10.)

---

Next: **[Part IV — Patterns & errors](04-patterns-and-errors.md)**.
