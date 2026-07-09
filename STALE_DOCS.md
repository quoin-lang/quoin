# Stale docs (implementation ≠ documentation)

Mismatches between `docs/language/` and the current implementation found during a bug-hunt
session (worktree @ main `1535e7d`). These are *documentation* lag, not necessarily bugs —
though a couple are borderline and cross-referenced to the bug report. Fix the doc (or decide
the impl should change).

## 1. "No truthiness coercion" is stated too broadly (§8, `02-blocks-and-control.md:58`)

> "There is **no truthiness coercion**. A condition must be an actual boolean. Sending `if:` to a
> non-boolean — including `nil` — is a `MessageNotUnderstood`."

True for **dispatched `if:`/`if:else:`**, but the language *does* have a truthiness notion elsewhere
(falsy = `false` or `nil`; everything else truthy):
- `&&` / `||` accept any operand and short-circuit on truthiness, **returning the operand value**
  (`7 || false` → `7`, `nil && x` → `nil`, `'x' || 9` → `'x'`).
- `!` coerces truthiness via `Object#'!'` / `Nil#'!'` (`!5` → `false`, `!nil` → `true`).
- `whileDo:` treats a non-boolean condition as truthy (`{7}.whileDo:{…}` loops).

Suggest scoping the sentence to `if:`/conditional *receivers* and documenting `&&`/`||`/`!`/`whileDo:`
truthiness explicitly. (Related bug: the *inlined* `if:` on a comparison also coerces truthiness,
diverging from the dispatched strict-boolean path — that IS a bug, see `findings/BUGS.md` Finding 1.)

## 2. `%{…}` interpolation is documented as full lexical scope, but `self`/`@fields` are invisible (§19, `06-library-and-reference.md:69`)

> "each `%{expr}` is evaluated **in the surrounding lexical scope** and stringified with `.s`."

Only ordinary **locals/params** resolve inside `%{…}`. `self`, a leading-dot send (`%{.name}`), and
instance fields (`%{@name}`) all resolve as `nil`/`MessageNotUnderstood` — even in the pure
interpreter, single call:
```quoin
Widget <- { |@name| … m -> { %'%{@name}' } }   "* @name reads as nil inside %{}
```
Either the doc should carve out `self`/fields, or (more likely the intent) `String#mod` should bind
the caller's `self` so fields work. Borderline — could be treated as an interpolation bug.
(Separately, under AOT even *locals* vanish — that's a real bug, `findings/BUGS.md` Finding 5.)

## 3. `\xXXXX` escape is documented but not implemented (§3, `01-foundations.md:121-122`)

> "Escapes: `\t \n \r \" \' \\`, plus `\uXXXX` and `\xXXXX` (four hex digits)."

`\uXXXX` works; `\xXXXX` does not — the grammar accepts it but `unescape` never decodes it, so
`'\x0041'` yields the 6 literal characters `\x0041` instead of `'A'`. Because it silently produces a
wrong value (not an error), it's filed as a real defect too (`findings/BUGS.md` Finding 8). The doc
should match whatever the fix decides (implement `\x`, or drop it from the escape list and grammar).

## 4. "Sets are unique by `==:`" is imprecise (§3, `01-foundations.md`; §12 `can?:`)

Set membership/dedup is by **`hash` + `==:`**, and the default `Object#hash` is identity-based — so two
distinct instances that a user's `#'==:'` considers equal are **not** deduped unless the class also
overrides `hash` (mirrors the any-key Map contract in `qnlib/tests/52-map-keys.qn`). The "deduplicated
by `==:`" phrasing suggests `==:` alone suffices. Behavior is correct/expected; only the wording lags.
