# Stdlib: Numbers & Math — implementation outline

Status: **ALL FOUR DONE** (Math → Statistics → BigDecimal → BigInteger; see §6 for the
per-item build record). This document is retained as the design record for the settled §5
decisions (BigInteger as a distinct type, no auto-promotion; the `closeTo:` assertion; etc.).

The four deliverables, mapped to the TODO bullets:

| # | TODO bullet | Native (Rust) | Quoin (`qnlib`) | New crate |
|---|---|---|---|---|
| 1 | ⭐ **Math** | methods on `Integer`/`Double` + `Math` namespace | optional `.meta` sugar | — |
| 2 | **Statistics** | — | `Iterate`-mixin methods | — |
| 3 | **BigDecimal** | `BigDecimal` native value type | constructors/sugar | `rust_decimal` |
| 4 | **BigInteger** | `BigInteger` native value type | constructors/sugar | `num-bigint` |

Math + Statistics are the ⭐ quick wins and pure additions; BigDecimal + BigInteger pull in a crate,
a new native value type, and the harder design questions — so they come last.

---

## 1. Where things live (grounded in the current code)

- **Number types** are unboxed immediates: `Value::Int(i64)`, `Value::Double(f64)`
  (`src/value.rs`). No per-instance fields; methods are per-type classes.
- **`Integer` methods**: `src/runtime/integer.rs` (`build_integer_class`, ~L42–84). Already has
  `sqrt` (→ Double) and the arithmetic/comparison operators.
- **`Double` methods**: `src/runtime/double.rs` (`build_double_class`, ~L29–51).
- **Builtin registration**: `src/runner.rs` `register_builtins()` (~L47–78) — every native class
  builder is registered here.
- **qnlib core**: `qnlib/core/NN-name.qn`, loaded by `qnlib/prelude.qn` via `use core/*` in
  zero-padded UTF-8 order (`00-bootstrap` … `06-io`). The `Iterate` mixin and the `List`/`Set`/
  `Map` extensions live in `qnlib/core/02-iterate.qn`.
- **Tests**: qnlib suites in `qnlib/tests/NN-name.qn` (`09-numbers.qn` already exists), run by
  `qn qnlib/main.qn`. Rust integration tests (if any) go in `tests/*.rs`.

## 2. Patterns to copy

**Native instance method on a number** (`integer.rs`/`double.rs`):
```rust
.instance_method("abs", |vm, mc, receiver, _args| {
    let v = recv!(receiver, Int);
    Ok(vm.new_int(mc, v.abs()))
})
// arg-taking + per-type dispatch:
.typed_instance_method("pow:", &["Double"], |vm, mc, receiver, args| {
    Ok(vm.new_double(mc, receiver.as_f64().unwrap().powf(args[0].as_f64().unwrap())))
})
```
Selectors are camelCase, trailing colon per argument (`pow:`, `log:base:`). Pull receiver with
`recv!`, args with `arg!`, build results with `vm.new_int/new_double/new_bool`.

**Namespace / static class** (`Math`), like `Async`/`Runtime`/`Timer`:
```rust
// src/runtime/math.rs
pub fn build_math_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Math", Some("Object"))
        .class_method("pi",   |vm, mc, _r, _a| Ok(vm.new_double(mc, std::f64::consts::PI)))
        .class_method("sin:", |vm, mc, _r, args| Ok(vm.new_double(mc, arg!(args, ..).sin())))
}
```
`.class_method` = receiver-is-the-class. Register in `runner.rs:register_builtins`. Optional
Quoin sugar via `Math <-- { .meta <-- { ... } }`.

**Native value type** (`BigDecimal`, `BigInteger`), like `NativeSocket`:
```rust
// src/runtime/big_decimal.rs
pub struct NativeBigDecimal(rust_decimal::Decimal);
impl AnyCollect for NativeBigDecimal {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}   // plain data, no Gc fields
}
```
Construct with `vm.new_native_state(mc, vm.get_or_create_builtin_class(mc, "BigDecimal"), state)`;
read inside methods with `receiver.with_native_state::<NativeBigDecimal, _, _>(|s| s.0)`. Unlike
`NativeSocket`, these own no OS resource, so **no reap-on-drop** is needed — `trace_gc` is empty.

**Pure-Quoin collection method** (`Statistics`), extending the `Iterate` mixin so every
collection inherits it:
```quoin
"* qnlib/core/07-statistics.qn  (loads after 02-iterate via use core/*)
Iterate <-- {
    mean -> { (.count == 0).if:{ ^nil }; ^(.sum / .count) };
    "* median, mode, variance, stddev, percentile: ...
};
```
Built on existing `Iterate` primitives: `sum`, `count`, `min`, `max`, `reduce:`, `sort`, `at:`.

**Test idiom** (`qnlib/tests/`):
```quoin
(TestSuite.new:{ name = 'Math' }).add:{
    .test: abs -> { .is:{ -42.abs } equalTo:42; };
    .test: domain -> { .does:{ -1.0.sqrt } throw:ArithmeticError; };
};
```
Run: `cargo build --release && target/release/qn qnlib/main.qn`.

---

## 3. Per-deliverable surface

### 1 — Math  ⭐  (native; `integer.rs`, `double.rs`, new `math.rs`)

**Step 0: audit what already exists** in `integer.rs`/`double.rs`/`tests/09-numbers.qn`
(`sqrt` is already there) so we extend rather than duplicate.

Instance methods on `Integer` and `Double` (the "a number's own" operations):

| Selector | Integer | Double | Notes |
|---|---|---|---|
| `abs` | → Integer | → Double | |
| `floor` `ceil` `round` `truncate` | identity (→ self) | → Integer (f64→i64 guarded) | `round` = half-away-from-zero (`f64::round`) |
| `sqrt` | exists (→ Double) | add (→ Double) | negative → ArithmeticError |
| `pow:` | → Integer, checked (overflow → ArithmeticError); neg exp → Double | → Double | |
| `min:` `max:` | selection (→ winning operand) | selection | mixed Int/Double → returns the winner in its own type (§5.4) |
| `sign` | → −1/0/1 | → −1.0/0.0/1.0 | optional |

`Math` namespace — constants + transcendental/trig free functions (where `Math.sin: x` reads
better than `x.sin`):

- Constants: `pi`, `e`, `tau`.
- Trig: `sin:` `cos:` `tan:` `asin:` `acos:` `atan:` `atan:over:` (atan2).
- Exp/log: `exp:` `ln:` `log:` (base-10) `log2:` `log:base:`.
- Possibly mirror `sqrt:`/`pow:to:`/`hypot:over:` for a free-function style.

Files: `src/runtime/math.rs` (new), edits to `integer.rs`/`double.rs`, register in `runner.rs`.
Tests: extend `qnlib/tests/09-numbers.qn`, add a `Math` suite.

### 2 — Statistics  ✅ done (`qnlib/core/07-statistics.qn`)

Pure qnlib on the `Iterate` mixin (so `List`/`Set`/`NumberRange` all gain it). `sum`/`min`/`max`
already existed; added `mean`, `median`, `variance` + `populationVariance`, `stddev` +
`populationStddev`, `percentile:`, `mode`, `modes` (decisions in §5.7). `stddev` uses the Double
`sqrt` from deliverable 1. Gotcha for future qnlib: guards use `^^` (non-local return) — a plain
`^` exits only the enclosing `if:` block, not the method. Tests: `qnlib/tests/26-statistics.qn`.

### 3 — BigDecimal  ✅ done (`src/runtime/big_decimal.rs`; `rust_decimal`)

Exact base-10 arithmetic (money). First native value type — a Rust-state-backed `BigDecimal`
(`AnyCollect` + `new_native_state`, no `Gc`/no reap). Construct: `BigDecimal.of: '1.50'`
(string), `BigDecimal.of: 42` (Integer), `BigDecimal.of: 150 scale: 2`. Methods: `+:` `-:` `*:`
`/:` (checked, divide-by-zero guarded), `<:` and `==:` (value compare — native `==` is pointer
identity, so both are defined), `abs`, `scale`, `round:` (half-away-from-zero, matching
`Double.round`), `asDouble`/`asInteger`, `.s`. Mixing → **explicit only** (§5.6): a foreign
operand matches no typed variant and surfaces as `MessageNotUnderstood`. Tests:
`qnlib/tests/27-bigdecimal.qn`.

### 4 — BigInteger  ✅ done (`src/runtime/big_integer.rs`; `num-bigint`)

Arbitrary-precision integers — a Rust-state-backed `BigInteger` (`BigInt` is `Clone`, extracted
by cloning). Construct `BigInteger.of: '123…'` / `BigInteger.of: 42` / `42.asBigInteger`
(`qnlib/core/08-bignum.qn`). Methods: `+:` `-:` `*:` `/:` (truncating) `%:` (zero-guarded), `<:`
`==:`, `abs`, `pow:` (non-negative exponent; negative errors — no Double escape), `asInteger`
(range-checked) / `asDouble`, `.s`. **Distinct opt-in type, no auto-promotion** — `Integer` stays
an unboxed i64; mixing is explicit only. Tests: `qnlib/tests/28-biginteger.qn`.

---

## 4. Error handling

Domain errors raise `ArithmeticError` (matches the existing `1/0` behavior in
`tests/09-numbers.qn`): negative `sqrt`, `ln`/`log` of ≤ 0, Integer `pow:` with negative
exponent, divide-by-zero. Parse failures (`BigDecimal.of:`/`BigInteger.of:` on bad input) raise a
value/parse error. Align the exact classes with the in-flight `structured-errors` work rather
than inventing new string errors.

---

## 5. Decisions

**Settled:**

1. **Math surface split** — number-centric ops (`abs`/`floor`/`ceil`/`round`/`truncate`/`sqrt`/
   `pow:`/`min:`/`max:`/`sign`) are instance methods on `Integer`/`Double`; the `Math` namespace
   owns constants (`pi`/`e`/`tau`) + trig/transcendental free functions. **Mirror a few where it
   reads well** (e.g. `Math.sqrt: x` alongside `x.sqrt`, `Math.pow:to:` alongside `x.pow: y`); keep
   the long tail of trig/transcendental in `Math` only.
2. **`floor`/`ceil`/`round`/`truncate` on a Double → Integer** (with an f64→i64 range guard;
   out-of-range magnitude → ArithmeticError). `round` is half-away-from-zero (`f64::round`).
3. **`pow:`** — `Integer.pow:` with a non-negative exponent stays Integer but is **checked**
   (overflow → ArithmeticError, since there is no auto-promotion); a **negative** exponent returns
   a Double (`2.pow: -1` → `0.5`). `Double.pow:` is always Double. (My call, per your deferral —
   consistent with promoting to Double exactly when the integer domain can't represent the result.)
5. **`BigInteger`/`BigDecimal` are distinct opt-in types — no auto-promotion.** `Integer` stays an
   unboxed i64; overflow is an error, never a silent promotion. Big types are reached explicitly.

4. **`min:`/`max:` on mixed Integer/Double — selection semantics** (approved). Return the winning
   operand in its own type (`5.max: 3.0` → `5` the Integer; `5.max: 7.0` → `7.0`), as Python
   (`max(5, 3.0) == 5`) and Ruby do — min/max *select* an existing value rather than compute a new
   one. Arithmetic ops (`+:`, `pow:`) still promote because they genuinely compute a new value.
7. **Statistics specifics** (approved). `variance`/`stddev` default to **sample (n−1)** with
   `populationVariance`/`populationStddev` for the population (n) case; `percentile:` takes 0..100
   and uses **linear interpolation** (so `percentile:50 == median`); **`mode`** returns one value
   (first-encountered on a tie) and **`modes`** returns every maximally-frequent value; empty
   collections → `nil` (`modes` → `#()`).

6. **BigDecimal mixing — explicit conversion** (approved). No implicit `BigDecimal + Integer/Double`:
   a foreign operand matches no typed variant and raises `MessageNotUnderstood`, so an exact value
   can't be silently contaminated by a float. (Applies to `BigInteger` too.)

All §5 decisions are now settled.

## 6. Suggested order

1. **Math** ✅ done — number methods + `Math` namespace + `closeTo:` test assertion.
2. **Statistics** ✅ done — `qnlib/core/07-statistics.qn` + `qnlib/tests/26-statistics.qn`.
3. **BigDecimal** ✅ done — `src/runtime/big_decimal.rs` + `qnlib/tests/27-bigdecimal.qn`.
4. **BigInteger** ✅ done — `src/runtime/big_integer.rs` + `qnlib/tests/28-biginteger.qn`.

All four "Numbers & math" deliverables are complete.

## 7. Build & test

- Rust changes: `cargo build --release` (or `cargo build`), `cargo test`, plus
  `target/release/qn qnlib/main.qn` for the qnlib suites.
- Pure-qnlib changes (Statistics): no rebuild — `qnlib/*.qn` is read from disk at runtime; just
  re-run `qn qnlib/main.qn`.
- Keep selectors camelCase; follow the repo's rustfmt-clean / import conventions.
