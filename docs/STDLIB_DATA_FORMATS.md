# Stdlib: Data formats — implementation outline

Status: **Outline, not started.** Plan for the `## Standard Library → Data formats &
serialization` bullets in `QUOIN_TODO.md`. Branch: `feat/stdlib-data-formats`. **Phase 1
(base64/hex + JSON) first**; Phase 2 (MessagePack / TOML / YAML) and Phase 3 (CSV) sketched.
Native classes follow the established pattern (`NativeClassBuilder`; see the number/time types).

## Three sub-families (the section isn't five equal items)

- **Structured** — JSON, MessagePack, TOML, YAML. All map a Quoin value tree
  (`Map`/`List`/`String`/`Integer`/`Double`/`Bool`/`Nil`, plus `BigInteger`/`BigDecimal`/`Bytes`)
  to/from the format. They share **one bridge**.
- **Byte codecs** — base64, hex. `Bytes ↔ String`. No value tree; independent and small.
- **Tabular** — CSV. Rows of string fields (`List` of `List`/`Map`); its own shape.

## Crate strategy: serde everywhere (decided)

serde is the pivot for the whole structured family — one bridge, every format plugs in:

- `serde` + `serde_json` (with **`arbitrary_precision`** — needed for number correctness, below),
  `base64`, `hex`. Later: `rmp-serde` (MessagePack), `toml`, a maintained YAML (`serde_yaml_ng`),
  `csv`.
- **The bridge — `DataValue`**: a GC-free enum that is the neutral tree and implements
  `serde::Serialize`/`Deserialize`:
  `Null | Bool | Int(i64) | BigInt | Float(f64) | Decimal | Str | Bytes | Array(Vec) | Object(Vec<(String, DataValue)>)`.
  Two conversions, both in native methods:
  - `Value<'gc> → DataValue` — walk the Quoin value into an owned tree (no `Gc`).
  - `DataValue → Value<'gc>` — build a Quoin value (needs `vm`/`mc` to allocate).
  Every structured format is then a thin native class: parse = `from_*::<DataValue>` → `Value`;
  generate = `Value → DataValue` → `to_*`.

---

## Phase 1a — base64 / hex  ⭐

Native namespace classes + qnlib helper methods (both, per the "really common use case" call).

- **`Base64`** (native; `base64` crate, standard alphabet + padding): `Base64.encode: aBytes → String`,
  `Base64.decode: aString → Bytes` (malformed input → `ValueError`).
- **`Hex`** (native; `hex` crate or std): `Hex.encode:` / `Hex.decode:`, same shape.
- **Helper methods** (qnlib sugar over the native classes, e.g. `qnlib/core/NN-codecs.qn`):
  - `Bytes#toBase64 → String`, `Bytes#toHex → String`.
  - `String#fromBase64 → Bytes`, `String#fromHex → Bytes` (interpret the string *as* encoded data).
  - `String#toBase64 → String` (encode the string's UTF-8 bytes) — convenience for the common case.
- Files: `src/runtime/base64.rs`, `src/runtime/hex.rs` (new) + registration; `qnlib/core/NN-codecs.qn`;
  `Cargo.toml`. Tests: `qnlib/tests/NN-codecs.qn`.

## Phase 1b — JSON

- Native **`JSON`**: `JSON.parse: aString → value`; `JSON.generate: value → String` (compact);
  `JSON.generatePretty: value → String` (indented — a separate method, not a flag). Malformed JSON
  → `ParseError`.
- **Structure mapping**: object → `Map` (JSON keys are strings — matches Quoin `Map`'s string-key
  requirement), array → `List`, string → `String`, `true`/`false` → `Bool`, `null` → `Nil`.
- **Number correctness (decided: never lose precision; fall back to the Big types):**
  - **Integer literal** (no `.`/`e`): `Integer` if it fits `i64`, else `BigInteger` (exact).
  - **Decimal literal**: `Double` **iff it round-trips exactly** through `f64` (parse → shortest
    round-trip string == the literal); otherwise `BigDecimal` (exact). So everyday decimals
    (`0.1`, `3.14`) stay `Double`, while a 25-significant-digit value becomes `BigDecimal` rather
    than silently truncating. Requires `serde_json`'s `arbitrary_precision` (numbers preserved as
    their original text so we can classify them).
  - **generate** emits exact digits for `BigInteger`/`BigDecimal` and the shortest round-trip for
    `Double` — so a parsed value re-serializes losslessly.
- **`Map` key order on generate**: Quoin `Map` iterates in sorted key order, so output keys are
  sorted (deterministic). Noted, not configurable for v1.
- **A `Bytes` value in `generate`**: JSON has no bytes type → **error** (`encode with Base64 first`),
  rather than silently base64-ing. (The Phase 1a helpers make explicit encoding a one-liner.)
- Files: `src/runtime/data_value.rs` (the bridge), `src/runtime/json.rs` (new) + registration;
  `Cargo.toml`. Tests: `qnlib/tests/NN-json.qn`.

---

## Phase 2 — MessagePack / TOML / YAML  (sketch)

All reuse the `DataValue` bridge via serde — each is a thin native class:

- **`MessagePack`** (`rmp-serde`): `pack: value → Bytes`, `unpack: Bytes → value`. Binary; the one
  format with a native `Bytes` type, so `DataValue::Bytes` round-trips here.
- **`TOML`** (`toml`): `parse:`/`generate:`. (TOML's top level must be a table → `Map`.)
- **`YAML`** (`serde_yaml_ng` or similar maintained fork): `parse:`/`generate:`.

## Phase 3 — CSV  (sketch)

Tabular, not a tree. `CSV.parse: aString → List of rows`, `CSV.generate: rows → String`, RFC 4180
quoting/escaping. Open: rows as `List` of `List` (positional) vs `List` of `Map` (header row) —
likely offer both (`parse:` and `parseWithHeader:`). The `csv` crate.

---

## Decisions

**Settled:** serde everywhere (one `DataValue` bridge); JSON numbers fall back to
`BigInteger`/`BigDecimal` for correctness (never lossy); `Base64` static API **and** `String`/`Bytes`
helper methods; Phase 1 (base64/hex + JSON) first.

**My call unless you object (small):**
- base64 uses the **standard** alphabet + padding for v1 (a `Base64Url` variant can come later).
- `hex` is lower-case on encode, case-insensitive on decode.
- `generate` of a value containing a non-serializable type (a `Block`, `DateTime`, socket, …) →
  a clear `TypeError` naming the offending type.
- Decimal round-trip classification uses f64 shortest-string equality; when it misclassifies it errs
  toward `BigDecimal` (exact, never lossy) — the safe direction.

## Order & testing

1. `Base64`/`Hex` + helpers → 2. `DataValue` bridge → 3. `JSON` parse/generate (+ number rules) →
   4. tests. `cargo build` / `cargo test` + `qn qnlib/main.qn`. New deps: `serde`, `serde_json`,
   `base64`, `hex`.
