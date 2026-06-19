# Part V — Concurrency & iteration

Fibers (stackful coroutines), generators built on them, and the `Iterate` mixin
that turns a single `each:` into a full collection API.

Nav: [Foundations](01-foundations.md) · [Blocks & control](02-blocks-and-control.md) · [Objects](03-objects.md) · [Patterns & errors](04-patterns-and-errors.md) · **Concurrency & iteration** · [Library & reference](06-library-and-reference.md) · [Appendices](07-appendices.md)

---

## 16. Fibers & generators

> **Rules**
> - `Fiber.new:{ … }` creates an **unstarted** fiber. `f.resume` / `f.resume:arg` runs it to the next yield or to completion.
> - `Fiber.yield:v` (or the sugar `^> v`) suspends the running fiber, handing `v` back to whoever resumed it. The value passed to the next `resume:` becomes the *result* of that yield expression — communication is two-way.
> - Query: `done?`, `alive?`, `failed?`, `status` (`'created'`/`'suspended'`/`'running'`/`'done'`/`'failed'`), `result` (final return value), `error`. `Fiber.current` is the running fiber (or `nil` at top level).
> - Fibers are **stackful** (you can yield from deep inside a native method such as `each:`) and **nestable/re-entrant**.
> - `Generator.from:{ ^> … }` wraps a yielding block as a **lazy, re-runnable** iterable (it mixes in `Iterate`). `collection.iterator` returns an external pull iterator with `hasNext?` / `next`.

```buildingblocks
f = Fiber.new:{ Fiber.yield:1; Fiber.yield:2; 'done' }
f.resume        "* 1     (runs to first yield)
f.resume        "* 2
f.resume        "* 'done'   (block's final value)
f.done?         "* true

evens = Generator.from:{ n = 0; { true }.whileDo:{ ^> n; n = n + 2 } }
evens.take:4    "* #(0 2 4 6)   (infinite source, consumed lazily)
```

> **⚠ Gotcha — resuming a finished or failed fiber throws.** Once `status` is
> `'done'` or `'failed'`, `resume` raises a `FiberError`. So does `Fiber.yield:`
> called outside any fiber, and a fiber attempting to resume itself. Check
> `alive?`/`done?` before resuming in a loop.

---

## 17. The iteration protocol

> **Rules**
> - The `Iterate` mixin provides the entire collection API on top of **one required primitive: `each:`**. Implement `each:` and mix in `Iterate`, and every combinator below works.
> - Custom iterable recipe:
>   ```buildingblocks
>   MyThing <- { …
>       .mix:Iterate
>       each: -> { |b| … b.valueWithSelfOrArg:element … }   "* call b once per element
>   }
>   ```
> - Iteration is **re-entrant** (no stored cursor — nested/concurrent passes don't interfere) and **nil-safe** (`nil` is a valid element, never a terminator).
> - `lazyCollect:` / `lazySelect:` return **Generators** (lazy), so they compose and work over infinite sources when capped with `take:`.

### Combinators provided by `Iterate`

Transform: `collect:`, `select:`, `reject:`, `flatten`, `groupBy:`, `partition:`,
`uniq`, `zip:`, `reverse`, `sort` / `sort:`.
Reduce/query: `reduce:`, `reduce:into:`, `detect:`, `all?:`, `any?` / `any?:`,
`none?:`, `count` / `count:`, `contains?:`, `sum` / `sum:`, `min` / `max` /
`min:` / `max:`.
Access: `first`…`fifth`, `last`, `nth:`, `take:`, `drop:`, `list`, `join:`,
`iterator`.
Lazy: `lazyCollect:`, `lazySelect:`.

(See `bblib/02-iterate.bub` for the authoritative list and exact semantics.)

```buildingblocks
MyRange <- { |@start @end|
    .mix:Iterate
    each: -> { |b| i = @start; { i < @end }.whileDo:{ b.valueWithSelfOrArg:i; i = i + 1 } }
}

r = MyRange.new:{ start = 0; end = 5 }
r.collect:{ |n| n * n }        "* #(0 1 4 9 16)   — all from one each:
r.select:{ |n| n > 2 }.list    "* #(3 4)
```

> **⚠ Gotcha — combinators return materialized lists; use the `lazy*` forms for
> infinite or expensive sources.** `collect:`/`select:` walk the whole collection
> eagerly. Over an infinite `Generator`, use `lazyCollect:`/`lazySelect:` and finish
> with `take:`; otherwise iteration never terminates.

---

Next: **[Part VI — Library & reference](06-library-and-reference.md)**.
