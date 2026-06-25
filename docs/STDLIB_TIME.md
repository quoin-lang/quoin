# Stdlib: Time — implementation outline

Status: **Outline, not started.** Plan for the `## Standard Library → Time` bullets in
`QUOIN_TODO.md`. Branch: `feat/stdlib-time`. **Phase 1 (Duration & monotonic clock) first**;
Phase 2 (DateTime / Timestamp / TimeZone) is sketched for a follow-up. Crate: **jiff**. The value
types follow the `BigDecimal`/`BigInteger` native-value-type pattern (`AnyCollect` +
`new_native_state` + `with_native_state`; see `src/runtime/big_decimal.rs`).

## The domain — three clocks + a bridge

Time is three different clocks; conflating them is the classic mistake.

- **Monotonic** — forward-only, unaffected by clock changes; for measuring *elapsed* time. → `Instant`.
- **Wall-clock instant** — an absolute point in time (UTC), can jump on a clock correction. → `Timestamp`.
- **Civil / calendar** — a date+time in a zone, with components and DST. → `DateTime` + `TimeZone`.

**`Duration`** (a fixed length of time) is the bridge between all three: `instantB - instantA`,
`dateTime + Duration`, and the `sleep:`/`timeout:` arguments.

## Crate: jiff

Pure Rust, **no C toolchain** (consistent with the repo's ring-over-aws-lc TLS choice). Provides
`SignedDuration` (absolute) + `Span` (calendar-aware), `Timestamp` (absolute UTC), `Zoned`
(zone-aware), `civil::{DateTime,Date,Time}`, the IANA tzdb, and RFC 3339/9557 parse+format.
Monotonic `Instant` is the one thing jiff doesn't cover (it's wall-clock by design) — that stays
`std::time::Instant`. Phase 1 uses **only `SignedDuration`**; Phase 2 pulls in `Timestamp`/`Zoned`/
`Span`/tzdb.

## Existing surface to build on / integrate

- `Timer.time:{block}` — monotonic elapsed micros via `std::time::Instant` (`src/runtime/timer.rs`).
  Keep as sugar over `Instant`.
- `Async.sleep:` — parks the running fiber via `IoRequest::Sleep { ms: u64 }` (`async_rt.rs`).
- `Async.timeout:do:` — `vm.await_timeout(mc, block, ms: i64, …)` (`async_rt.rs`).
- No wall-clock source yet (`SystemTime`/`UNIX_EPOCH` unused).

---

## Phase 1 — Duration & monotonic clock  (the immediate work)

### `Duration`  (native value type; jiff `SignedDuration`, **signed**)

- **Construct:** `Duration.seconds:` / `.milliseconds:` / `.microseconds:` / `.nanoseconds:` /
  `.minutes:` / `.hours:` (Integer args), `Duration.zero`.
- **Arithmetic:** `+:` (Dur+Dur), `-:` (Dur−Dur), `*:` (Dur × Integer scalar), `negate`, `abs`.
- **Compare:** `<:`, `==:`.
- **Accessors:** `.asSeconds` (Double, fractional), `.asMilliseconds` / `.asMicroseconds` /
  `.asNanoseconds` (Integer), `.s` (readable, e.g. jiff's "1h 30m" friendly form).
- Signed, so DateTime differences in Phase 2 can be negative.

### `Instant`  (monotonic; `std::time::Instant`)

- `Instant.now` → an `Instant`.
- `.elapsed` → `Duration` (now − self).
- `-:` (Instant − Instant → signed `Duration`; positive when the receiver is the later instant).
- `<:`, `==:`.
- Native state wraps `std::time::Instant` (Copy, no `Gc`, no reap — like the number types).

### Scheduler integration  (approved)

- `Async.sleep:` → typed variants: `sleep:&["Integer"]` (ms, existing) **+** `sleep:&["Duration"]`
  (Duration → total ms → `IoRequest::Sleep`).
- `Async.timeout:do:` → `timeout:&["Integer"] do:` (ms) **+** `timeout:&["Duration"] do:`
  (Duration → ms → `await_timeout`). Keeps the ms forms.

### Files

- `src/runtime/duration.rs` + `src/runtime/instant.rs` (new); `mod.rs` + `runner.rs` registration;
  `Cargo.toml` (jiff). Edit `async_rt.rs` (`sleep:` + `timeout:do:`) for the Duration overloads.
  Tests: `qnlib/tests/NN-duration.qn`.

---

## Phase 2 — DateTime  (sketch; separate PR)

- **`Timestamp`** (jiff `Timestamp`) — absolute UTC instant: `Timestamp.now`, epoch conversions,
  RFC 3339, `± Duration`.
- **`DateTime`** (jiff `Zoned`) — zone-aware, the primary calendar type: components
  (`year`/`month`/`day`/`hour`/`minute`/`second`/`nanosecond`, `weekday`), RFC 3339 parse/format,
  comparison, `± Duration` (absolute shift), and **calendar arithmetic via convenience methods**:
  `plusDays:` / `plusWeeks:` / `plusMonths:` / `plusYears:` (+ `minus…`), backed by jiff `Span`
  (end-of-month clamping, DST-correct "+1 day"), `.s`.
- **`TimeZone`** — IANA lookup; `DateTime.now` (system zone), `.nowUtc`, convert between zones.
- `dt2 - dt1` → `Duration` (absolute elapsed time).

**Deferred to its own TODO item:** civil `Date`/`Time` types, and a first-class `Span`/`Period`
value type (ISO 8601 duration parsing like `P1Y2M`, mixed-unit arithmetic, calendar *diffs*). The
`plus…`/`minus…` methods cover the common calendar-arithmetic case without that surface.

---

## Decisions

**Settled:** jiff; Phase 1 first; zone-aware `DateTime` (civil types deferred); calendar arithmetic
via `plus…`/`minus…` convenience methods (a `Span` value type deferred); `sleep:`/`timeout:` accept
a `Duration`.

**My call unless you object (small):**

- Unit constructors use **full words** (`milliseconds:`, not `millis:`) — Quoin favors readable
  selectors.
- `Duration.s` uses jiff's **friendly** format (`1h 30m`); an `.iso8601` (`PT1H30M`) can come later.
- `Duration *:` takes an **Integer** scalar for v1 (Double scaling deferred — avoids a rounding
  question).
- `Instant`/`now` are clock-based and so tested **loosely** (e.g. elapsed ≥ a slept duration within
  a tolerance) to avoid flaky tests; `Duration` arithmetic/conversions are pure and tested exactly.

## Order & testing

1. `Duration` → 2. `Instant` → 3. scheduler integration (`sleep:`/`timeout:`) → 4. tests.
   `cargo build` / `cargo test` + `qn qnlib/main.qn`. New dep: jiff.
