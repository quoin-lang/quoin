# Stdlib: Data formats — implementation outline

Status: **All phases done** (base64/hex + JSON + MessagePack + TOML + YAML + CSV). Plan for the
`## Standard Library → Data formats & serialization` bullets in `QUOIN_TODO.md`. Branch:
`feat/stdlib-data-formats`. **Phase 1** (base64/hex + JSON), **Phase 2** (`DataValue` bridge +
MessagePack / TOML / YAML), and **Phase 3** (CSV) are all done.
Native classes follow the established pattern (`NativeClassBuilder`; see the number/time types).

Phase-1 note on the bridge: JSON uses `serde_json::Value` (with `arbitrary_precision`) as the
serde tree directly, with `Value ↔ serde_json::Value` converters in `src/runtime/json.rs` — clean
and correct for one format. The generic `DataValue` enum below is the Phase-2 generalization, where
MessagePack (which has a native bytes type) and TOML/YAML need a shared tree across formats.

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

## Phase 1a — base64 / hex  ⭐  ✅ done

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

## Phase 1b — JSON  ✅ done

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

## Phase 2 — DataValue + MessagePack / TOML / YAML  ✅ done

The `DataValue` bridge (`src/runtime/data_value.rs`) — a GC-free tree with `Value ↔ DataValue`
conversions and hand-written serde `Serialize`/`Deserialize` — lets each format be a thin native
class. Out-of-range `BigInt`/`Decimal` serialize as their exact digits in a **string** (serde's
data model caps at i128/u128/f64); on the way back, that string stays a `String`. JSON keeps its
own `serde_json::Value` path (still the fully-lossless format for *numbers*).

- **`MessagePack`** (`rmp-serde`, `src/runtime/msgpack.rs`): `pack: value → Bytes`,
  `unpack: Bytes → value`. The one format with a native `Bytes` type, so `Bytes` round-trips.
- **`TOML`** (`toml`, `src/runtime/toml_fmt.rs`): `parse:` / `generate:`. Top level must be a `Map`
  (a TOML table) and TOML has no null, so `generate:` of a non-Map or of a value containing `nil`
  errors clearly.
- **`YAML`** (`serde_yaml_ng`, `src/runtime/yaml.rs`): `parse:` / `generate:`. Allows any top-level
  value and has a native null (no extra constraints).

Tests: `qnlib/tests/33-msgpack.qn`, `34-toml.qn`, `35-yaml.qn`; Rust unit tests for `DataValue`'s
serialize side in `src/runtime/data_value_tests.rs`.

## Phase 3 — CSV  ✅ done (`src/runtime/csv_fmt.rs`; `csv` crate)

Tabular, not a tree (so it doesn't use `DataValue`). RFC 4180 quoting/escaping. CSV is untyped, so
`parse` yields **strings**; `generate` stringifies each field via its `.s`. Both row shapes are
offered:

- `CSV.parse: str` → **List of List of String** (positional); `CSV.generate: rows` ← List of Lists.
- `CSV.parseWithHeaders: str` → **List of Map** (first row = headers; column order preserved, since
  Maps are insertion-ordered); `CSV.generateWithHeaders: rows` ← List of Maps (header from the
  first row's keys; a missing key → an empty field).

Tests: `qnlib/tests/36-csv.qn`.

---

## Decisions

**Settled:** serde everywhere (one `DataValue` bridge); JSON numbers fall back to
`BigInteger`/`BigDecimal` for correctness (never lossy); `Base64` static API **and** `String`/`Bytes`
helper methods; Phase 1 (base64/hex + JSON) first.

**My call unless you object (small):**
- base64 uses the **standard** alphabet + padding for v1 (a `Base64Url` variant can come later).

  > **Tracked as #111** — Add a Base64Url codec variant.

- `hex` is lower-case on encode, case-insensitive on decode.
- `generate` of a value containing a non-serializable type (a `Block`, `DateTime`, socket, …) →
  a clear `TypeError` naming the offending type.
- Decimal round-trip classification uses f64 shortest-string equality; when it misclassifies it errs
  toward `BigDecimal` (exact, never lossy) — the safe direction.

## Order & testing

1. `Base64`/`Hex` + helpers → 2. `DataValue` bridge → 3. `JSON` parse/generate (+ number rules) →
   4. tests. `cargo build` / `cargo test` + `qn qnlib/main.qn`. New deps: `serde`, `serde_json`,
   `base64`, `hex`.


## Custom serialization — the `asData` protocol (shipped 2026-07-11)

The Phase-1 "error on anything outside the core tree" behavior grew the planned hook: a
method protocol, not a registry (classes are open — `DateTime <-- { asData -> {…} }`
covers "a class you don't own", so a registry would duplicate class extension with worse
discoverability).

- **Seam**: `data_value.rs::as_data_of` — if the value's class understands `asData`
  (`lookup_in_class_hierarchy`), call it and recurse on the answer, spending the same
  `MAX_SERIALIZE_DEPTH` budget (a self-referential `asData` errors catchably). Fires from
  both walks (`value_to_json`, `value_to_data`) at every no-representation point: the
  Instance/Symbol/Block/Bytes(JSON) arms and the unknown-native fallthrough. Both walks
  now take `vm, mc` (their five call sites are native class methods).
- **The extension wire and worker frames keep their strict walkers deliberately** — that
  boundary's contract is explicit core data; an extension silently receiving a
  stringified DateTime would be a trap.
- **Stdlib defaults** in `qnlib/core/16-serialize.qn`, each chosen to round-trip via the
  type's own `parse:`: DateTime → RFC 9557 (zone kept; serialize `.timestamp` for bare
  RFC 3339), Timestamp → RFC 3339, Date/Time → ISO, Span/Duration → iso8601, UUID/ULID →
  canonical strings, Set → List (insertion order), KeyValuePair → `#{'key':… 'value':…}`.
  Symbol/Block/Regex stay errors by default; the error message names the protocol.
- **One-way, untagged** by design. The reverse convention is class-side `fromData:`
  (documented, not enforced); auto-tagging could layer on later without breaking this.

Tests: `qnlib/tests/74-serialize.qn`.
