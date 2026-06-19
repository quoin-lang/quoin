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

```buildingblocks
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

```buildingblocks
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

```buildingblocks
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
> plain copies. See `BBLIB_TODO.md` → *Bugs/Odd Behavior* for the full write-up of
> the `new:{}` scoping rules and the (intentionally absent) lexical capture.

> **Note — there is no `super`.** A subclass `init:` cannot call its parent's
> initializer with computed arguments; the parent runs first off the raw block
> fields. If a child needs to set a parent's field, it assigns `@field` directly.

---

## 12. Inheritance & mixins

> **Rules**
> - **Method lookup order**: the receiver's own class → its mixins (in the order added) → its parent, recursing upward. The most-derived definition wins.
> - `.mix:M` mixes class `M` into the current class; `.can:M` is an exact alias. Mixed-in methods and instance vars are included.
> - **Initializer order is the dual of lookup**: base → derived (parent, then mixins, then self), so ancestors initialize first (§11).
> - `.sealed!` is currently a **no-op** (intended to forbid further extension; not yet enforced).
> - `.can?:` is **not implemented**.
> - The built-in `ActAsUserList` / `ActAsUserString` mixins are what enable the `#Name( … )` and `#Name'…'` custom-literal forms.

```buildingblocks
Greeter <- { hello -> { 'hi from ' + .class.name.s } }

Widget <- {
    .mix:Greeter
    name -> { 'widget' }
}

Widget.new.hello       "* 'hi from Widget'   (found via the mixin)
```

> **⚠ Gotcha — `.sealed!` and `.can?:` don't do anything yet.** `.sealed!` parses
> and runs but does not actually seal the class, and `.can?:` is absent (calling it
> is a `MessageNotUnderstood`). Don't rely on either for correctness.

---

## 13. Multimethod dispatch

> **Rules**
> - A selector can have several definitions distinguished by argument **type** or a **guard block**; dispatch picks a matching one at call time.
> - Candidates are ordered by **specificity** (a more specific type like `Integer` beats `Object`); ties are broken by **definition order — first wins**.
> - Typed parameter: `|x:Integer|`. Guard block: `|x { x > 5 }|` or `|x:Type { … }|` — the guard must return a truthy value to match.
> - No matching variant → `MessageNotUnderstood { receiver, selector, args }`.

```buildingblocks
describe: -> { |n:Integer| 'int ' + n.s }
describe: --> { |s:String|  'str ' + s }
describe: --> { |n { n > 100 }| 'big number' }   "* guard block

describe:5         "* 'int 5'
describe:'hi'      "* 'str hi'
```

Type-based variants are the right tool when you want different behavior per
argument type; the dispatcher chooses the most specific match.

> **⚠ Gotcha — order matters for equal-specificity guarded variants.** Among
> variants that match the same argument with equal type-specificity, **guarded**
> variants are tried in *definition order* (first match wins) — so define the
> specific guards before a catch-all. (Two variants with the **same signature and no
> guard** don't coexist: the later one *replaces* the earlier — §10.)

---

Next: **[Part IV — Patterns & errors](04-patterns-and-errors.md)**.
