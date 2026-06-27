# `adbc` — Quoin's ADBC database extension (design)

Status: **Design capture — approved scope, building.** `adbc` is an out-of-process Quoin extension
(`docs/FUTURE_EXT_ARCH.md`) that exposes [Apache Arrow Database
Connectivity](https://arrow.apache.org/adbc/current/) as Quoin classes — the default database-access
library. It is **out-of-core**: its only Quoin dependency is the extension SDK (`quoin-ext`); it
never links the VM. Companion to `docs/EXT_PACKAGING.md` (the producer-side model this realizes).

## 1. What ADBC is (and why it fits)

ADBC is a vendor-neutral, **Arrow-native** database API (a JDBC/ODBC analog whose results are columnar
Arrow record batches, not row-by-row). Its object model is a lifecycle chain:

**Driver → Database → Connection → Statement → result (an Arrow `RecordBatchReader`)**

That chain maps almost 1:1 onto Phase-3 **extension-backed classes**: the live ADBC handles live in
the SDK's object table, and `Database → connect → Connection → prepare: → Statement → execute →
ResultSet` are all **cross-class returns**, with SQL + bind params as **richer args** — exactly the
features the SDK gained in PR #24.

## 2. Crate & dependencies

A new **binary** crate `crates/adbc` (workspace member) producing the `adbc` extension binary.

```toml
[dependencies]
quoin-ext           = { path = "../quoin-ext" }   # the ONLY Quoin dep (out-of-core)
adbc_core           = "0.23"                       # Driver/Database/Connection/Statement traits
adbc_driver_manager = "0.23"                       # dlopens the C driver, resolves manifests
arrow-array         = ">=53, <59"                  # read result batches / build bind batches
arrow-schema        = ">=53, <59"
```

No dependency on `quoin` or (by name) `quoin-ext-proto`; the SDK re-exports everything the extension
touches (`DataValue`, `Arg`, …). This is `EXT_PACKAGING.md §13`, in-workspace.

## 3. Drivers — loading via the ADBC manifest

The SQLite and PostgreSQL ADBC drivers are C/C++ shared libraries, loaded at runtime by the driver
manager. They are registered on this machine via the **ADBC driver-manifest** convention —
`~/Library/Application Support/ADBC/Drivers/{sqlite,postgresql}.toml` (manifest v1, ASF 1.11.0), each
pointing at a per-platform `.dylib`.

- **v1 loads by name:** `ManagedDriver::load_dynamic_from_name("sqlite" | "postgresql", None,
  AdbcVersion::V100)` — the manager resolves the manifest in the platform search paths (incl. the
  macOS `Application Support` dir above) to the right `.dylib`. Default entrypoint `"AdbcDriverInit"`.
- **Fallback (robustness):** if the Rust binding does not auto-resolve manifests, the extension reads
  the manifest TOML itself — `[Driver.shared].<platform>` → the `.dylib` path — and calls
  `load_dynamic_from_filename(path, None, AdbcVersion::V100)`. Either way, **no hardcoded paths in
  code**, and an env override (`QUOIN_ADBC_<DRIVER>_PATH`) is available for non-manifest installs.

Drivers are loaded lazily and cached: the first `Database` opened against a driver loads it; later
ones reuse it. (Driver/library install — pip/conda/manifest — is a setup prerequisite, not a `cargo`
artifact; the eventual `adbc` *package* declares it, per `EXT_PACKAGING.md`.)

## 4. The `[ADBC]` class API

Namespaced `[ADBC]` (uppercase acronym). Faithful to ADBC's Database/Connection split (so connection
pooling and shared config land naturally later).

### `[ADBC]Database` — the data source (reusable)
- class-side constructors: `sqlite: path` · `sqliteMemory` · `postgres: connString` ·
  `driver: name uri: s` (generic)
- instance: `connect` → `[ADBC]Connection`

### `[ADBC]Connection` — a session
- `query: sql` · `query: sql params: aList` → **`[ADBC]ResultSet`**   *(SELECT)*
- `execute: sql` · `execute: sql params: aList` → rows-affected (`Integer`)   *(INSERT/UPDATE/DDL)*
- `prepare: sql` → **`[ADBC]Statement`**
- `commit` · `rollback` · `autocommit: aBool` (set) · `autocommit` (get) · `transaction: aBlock`   *(sugar: save mode → autocommit off → block → commit / rollback-on-raise → restore mode)*
- `tables` → `List` of table names · `tableColumns: name` → column metadata   *(get_objects / get_table_schema)*
- `close`

### `[ADBC]Statement` — a prepared statement (reusable across binds)
- `bind: aList` · `query` → ResultSet · `execute` → rows-affected

### `[ADBC]ResultSet` — a streaming result (holds the Arrow `RecordBatchReader`)
- `next` → a row `Map` (column → value), or `nil` at end   *(lazy)*
- `each: aBlock` · `toList` → `List` of row `Map`s
- `columns` → `List` of column names · `close`

```quoin
use adbc:*;

db := [ADBC]Database sqlite: 'app.db'.
c  := db connect.
c execute: 'create table users (id integer, name text, age integer)'.
c execute: 'insert into users values (?, ?, ?)' params: #( 1 'Ada' 36 ).
(c query: 'select name, age from users where age > ?' params: #( 18 ))
  each: { |r| Console log: (r at: 'name') }.

pg := ([ADBC]Database postgres: 'host=/tmp dbname=damon user=damon') connect.
(pg query: 'select count(*) as n from pg_class') toList.   "#( #{ 'n': 397 } )"
```

## 5. Result model — streaming rows as `Map`

The ADBC `RecordBatchReader` stays alive in the extension's object table; Quoin pulls rows **lazily**
(`next`) or drains (`each:` / `toList`). Each Arrow row becomes a Quoin `Map` (column → value),
crossing the socket as a structured `DataValue`. Large results never fully materialize host-side. The
extension buffers one record batch and walks its rows; `next` advances, fetching the next batch from
the reader when the current one is exhausted.

Per-cell **value mapping (Arrow → DataValue), v1:**

| Arrow type | DataValue |
|---|---|
| int8 … int64 | `Int` (`BigInt` if it overflows i64) |
| float32 / float64 | `Float` |
| utf8 / large_utf8 | `Str` |
| boolean | `Bool` |
| null / SQL NULL | `Null` |
| decimal128 / decimal256 | `Decimal` |
| binary / large_binary | `Bytes` |
| date32/64, timestamp, time | `Str` (ISO-8601) — a real `DateTime` mapping is a follow-up |
| list / struct / map / other | `Str` (debug rendering) fallback — noted, not silently wrong |

**Bind direction (`params:`):** a Quoin `List` of values → a **single-row Arrow `RecordBatch`** (one
column per param, type inferred from each value via the inverse table) → `statement.bind(batch)`.
`Int`→int64, `Float`→float64, `Str`→utf8, `Bool`→boolean, `Bytes`→binary, `Null`→null,
`Decimal`→decimal128, `BigInt`→decimal/utf8. Typed-`?`-placeholder dialects (PostgreSQL `$1`) are
handled by the driver; the extension just supplies the bound batch positionally.

Full-Arrow **columnar** results (a host-side `Table` value backed by the Arrow C Data Interface) are
**deferred** — `adbc` is the canonical forcing function for that data-plane work, kept decoupled from
v1.

## 6. Lifecycle, errors, concurrency

- **Lifecycle / teardown order.** ADBC handles are RAII; the SDK object table drops them when the
  Quoin-side wrapper is dropped (reaped via the existing release batch). ADBC requires children to
  outlive parents to be dropped first (a reader pins its statement/connection); the extension keeps a
  parent reference inside each child handle so a `ResultSet` keeps its `Connection` alive until the
  `ResultSet` is dropped/drained, regardless of Quoin-side drop order.
- **Errors.** An ADBC/SQL error → the handler returns `Err` → the Quoin send **raises a catchable
  error** carrying the driver message (and SQLSTATE where the driver provides it). A structured
  `[ADBC]Error` class with `.sqlState` / `.vendorCode` is a follow-up.
- **Concurrency.** The ADBC sync API blocks the *extension* thread; the host fiber merely parks on the
  socket reply, so the VM stays responsive while a query runs. One `adbc` process serializes its calls
  (ADBC connections are single-threaded); multi-connection parallelism (threads / multiple processes)
  is later.

## 7. v1 scope

**In:** SQLite + PostgreSQL via the driver manager (manifest-resolved); `Database` / `Connection` /
`Statement` / `ResultSet`; `query` / `execute` / `prepare` / `bind`; streaming rows-as-`Map` with a
typed `schema`; the value-mapping table (both directions); `commit` / `rollback` / `autocommit:`;
catchable errors.

**`transaction:` block sugar** ships in the package's `pkg/init.qn` (a Quoin `[ADBC]Connection`
reopening, loaded by `Extension loadPackage:`), *not* in the extension binary — so it's ordinary
VM-side control flow over the `autocommit:`/`commit`/`rollback` primitives, and the block never
re-enters its own connection mid-call (which the in-binary approach couldn't do). It runs the block,
commits on success or rolls back and re-raises on a throw, and always restores autocommit.

**Deferred:** a hierarchical **schema-introspection API** (catalogs / schemas / tables / columns — a
flat `tables` string list wasn't worth shipping); full-Arrow columnar `Table` (Arrow C Data
Interface); `DateTime` / temporal mapping; structured `[ADBC]Error`; bulk ingest; connection pooling;
parallelism; additional drivers; driver-library bundling (the packaging story).

## 8. Build slices

1. **Skeleton + driver load** — the crate, `serve`, and `[ADBC]Database sqlite:` / `sqliteMemory` /
   `postgres:` → load the driver (manifest), open a Database, `connect` → `Connection`. Prove a
   round-trip with `select 1`.
2. **Queries → `ResultSet`** — `query:` returning a streaming `ResultSet`; Arrow→`DataValue` row
   mapping; `next` / `each:` / `toList` / `columns`.
3. **DML + params** — `execute:` (rows-affected); `params:` binding (the inverse Arrow batch);
   `prepare:` → `Statement` with `bind:` / `query` / `execute`.
4. **Transactions** — `autocommit:` / `commit` / `rollback`. (The `transaction:` block sugar lands
   later as Quoin glue in `pkg/init.qn` via the packaging path; schema introspection deferred — §7.)
5. **Tests** — an integration test against SQLite (`:memory:`, no external deps) for CI, and a
   PostgreSQL test gated on the local server (passwordless `damon` via the `/tmp` socket).

Each slice its own commit, pausing for review per the usual rhythm.
