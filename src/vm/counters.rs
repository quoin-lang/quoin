//! Always-on engine counters — the `'dispatch'` and `'gc'` sections of `VM.stats`
//! (docs/internal/AOT_ARCH.md §9.7, issue #70).
//!
//! Plain `u64` bumps on the relevant paths, no sampling and no env gate: the
//! point is that a perf arc can read them from any run, including one that was
//! not started with profiling in mind.
//!
//! They are not free. Measured at ~0.94% on dispatch-heavy microbenchmarks, of
//! which ~0.58% is the per-send IC bumps below and ~0.36% the collector pump
//! (profiling/vm-stats-counters/notes.md). That was accepted rather than gating
//! the dispatch section behind an env flag — a gated counter only answers
//! questions from runs somebody already decided to instrument. Keep that price
//! in mind before adding a *fourth* bump to `exec_send`.
//!
//! **Both are per-VM**, and deliberately so — `VM.stats` reports them next to
//! each other, and two counters in one Map measured against different
//! denominators is a trap. (`'aot'` and `'compile'` next door genuinely are
//! process-wide: their work is cached and shared across VM sessions, so a
//! per-VM reading would be near-zero for every session after the first. Each
//! section carries a `scope` key saying which it is.)
//!
//! - **Dispatch** ([`DispatchCounters`]) hangs off `VmState` directly. Bump
//!   sites already hold `&self`/`&mut self` on the way through `exec_send` /
//!   `lookup_method_in_class_hierarchy`, so the field is free to reach. `Cell`
//!   rather than a plain `u64` because `ic_probe` takes `&self` — a `Cell::set`
//!   compiles to the same store.
//! - **GC** ([`GcCounters`]) hangs off `VmState` behind an `Rc`, because the
//!   collector is pumped from the driver loops (`runner.rs`,
//!   `runner_driver.rs`, `direct_runner.rs`) which hold `&mut Arena` *outside*
//!   `mutate_root`, where the root is unreachable. Each driver pulls a handle
//!   once via the cheap shared `Arena::mutate` and passes it to
//!   [`pump_collector`]; reaching through `mutate_root` per pump would put a
//!   root write barrier on the collector's hot path.
//!
//! The live heap numbers (`liveObjects` / `liveBytes` / `debt`) are not counted
//! here at all — `gc_arena`'s own `Metrics` already tracks them per arena, and
//! [`GcCounters::snapshot`] reads them straight off it.

use std::cell::Cell;

use gc_arena::metrics::Metrics;

/// Live-shape flips at one call site before it counts as megamorphic.
///
/// The IC is strictly monomorphic — a new receiver shape simply overwrites the
/// one slot ([`crate::vm::ICSlot`]), so "megamorphic" is not a state the VM
/// otherwise has. A site that flips this many times is one the single-entry
/// cache can never serve, which is exactly what the metric is for. Chosen so a
/// one-off phase change (a site that settles on a second shape and stays) does
/// not register; a genuinely polymorphic site crosses it almost immediately.
pub const IC_MEGAMORPHIC_FLIPS: u8 = 8;

/// Per-VM method-dispatch counters. Monotonic for the VM's lifetime.
#[derive(Default)]
pub struct DispatchCounters {
    /// Sends served straight from the per-site inline cache.
    pub ic_hits: Cell<u64>,
    /// Sends whose IC probe missed and fell through to `lookup_method`.
    pub ic_misses: Cell<u64>,
    /// IC slots actually written (a miss only fills for a guard-free resolution).
    pub ic_fills: Cell<u64>,
    /// Fills that overwrote a still-live entry holding a DIFFERENT type shape.
    pub ic_flips: Cell<u64>,
    /// Distinct sites whose flip count crossed [`IC_MEGAMORPHIC_FLIPS`].
    pub megamorphic: Cell<u64>,
    /// Resolutions served by the global (class+selector) dispatch cache.
    pub cache_hits: Cell<u64>,
    /// Resolutions that fell through to the full class-hierarchy walk.
    pub cache_misses: Cell<u64>,
}

impl DispatchCounters {
    #[inline]
    pub fn note_ic_hit(&self) {
        self.ic_hits.set(self.ic_hits.get() + 1);
    }

    #[inline]
    pub fn note_ic_miss(&self) {
        self.ic_misses.set(self.ic_misses.get() + 1);
    }

    #[inline]
    pub fn note_ic_fill(&self) {
        self.ic_fills.set(self.ic_fills.get() + 1);
    }

    /// Record a live-shape flip at a site that has now flipped `flips` times.
    /// `megamorphic` counts the site once, on the crossing, so it stays a count
    /// of *sites* rather than of flips.
    #[inline]
    pub fn note_ic_flip(&self, flips: u8) {
        self.ic_flips.set(self.ic_flips.get() + 1);
        if flips == IC_MEGAMORPHIC_FLIPS {
            self.megamorphic.set(self.megamorphic.get() + 1);
        }
    }

    #[inline]
    pub fn note_cache_hit(&self) {
        self.cache_hits.set(self.cache_hits.get() + 1);
    }

    #[inline]
    pub fn note_cache_miss(&self) {
        self.cache_misses.set(self.cache_misses.get() + 1);
    }
}

/// Phase codes stored in [`GcCounters::phase`]. `Sleeping` is 0, so a VM that has
/// never been pumped reads correctly.
const PHASE_SLEEPING: u8 = 0;
const PHASE_MARKING: u8 = 1;
const PHASE_MARKED: u8 = 2;
const PHASE_SWEEPING: u8 = 3;

fn phase_code(phase: gc_arena::arena::CollectionPhase) -> u8 {
    use gc_arena::arena::CollectionPhase as P;
    match phase {
        P::Sleeping => PHASE_SLEEPING,
        P::Marking => PHASE_MARKING,
        P::Marked => PHASE_MARKED,
        P::Sweeping => PHASE_SWEEPING,
    }
}

fn phase_name(code: u8) -> &'static str {
    match code {
        PHASE_MARKING => "marking",
        PHASE_MARKED => "marked",
        PHASE_SWEEPING => "sweeping",
        _ => "sleeping",
    }
}

/// Per-VM collector counters. Held behind an `Rc` so the driver loop — which
/// pumps the collector from *outside* `mutate_root`, where the root is
/// unreachable — can bump the same counters the VM reports.
#[derive(Default)]
pub struct GcCounters {
    /// Collection cycles observed to have started. A floor; see [`Self::note_pump`].
    cycles: Cell<u64>,
    /// Calls into the collector (`Arena::collect_debt`) — exact.
    pumps: Cell<u64>,
    /// The collection phase as of the last pump ([`phase_code`]).
    phase: Cell<u8>,
}

impl GcCounters {
    /// Record one pump of the collector, given the phase it came out in.
    ///
    /// A cycle is counted when the collector was asleep going in and is not
    /// coming out. The "going in" phase is the one recorded on the LAST pump
    /// rather than a fresh sample: collection only advances inside the
    /// collector, so between two pumps the phase is whatever the previous pump
    /// left, which makes a second sample per pump redundant. (The cold one-shot
    /// `Arena::finish_cycle` calls in `runner.rs` move the phase without coming
    /// through here, so the stored phase can go stale across one of those; it
    /// costs at most an uncounted cycle, and the count is already a floor.)
    ///
    /// `Arena::collect_debt` runs until the allocation debt is zero with no
    /// guaranteed stop at a phase boundary, so a cycle that both starts AND
    /// finishes inside one call is invisible here (both readings say
    /// `Sleeping`) — hence a floor, in the same spirit as the bounded refusal
    /// log behind the `'aot'` section. Making it exact would mean driving the
    /// collector with `Arena::cycle_debt`, which always returns at a cycle
    /// boundary but changes pacing; that is a perf change, deliberately not
    /// bundled into an observability one.
    #[inline]
    pub fn note_pump(&self, after: gc_arena::arena::CollectionPhase) {
        let after = phase_code(after);
        self.pumps.set(self.pumps.get() + 1);
        if self.phase.get() == PHASE_SLEEPING && after != PHASE_SLEEPING {
            self.cycles.set(self.cycles.get() + 1);
        }
        self.phase.set(after);
    }

    /// This VM's counters plus the live heap numbers read straight off its
    /// arena's metrics (`mc.metrics()` from inside the `mutate_root` closure).
    pub fn snapshot(&self, metrics: &Metrics) -> GcSnapshot {
        GcSnapshot {
            collections: self.cycles.get(),
            pumps: self.pumps.get(),
            phase: phase_name(self.phase.get()),
            live_objects: metrics.total_gc_count() as u64,
            live_bytes: metrics.total_gc_allocation() as u64,
            // Debt is a byte figure carried as an f64 for the pacing
            // arithmetic; it is never negative (`allocation_debt` clamps at 0).
            debt: metrics.allocation_debt() as u64,
        }
    }
}

/// What [`GcCounters::snapshot`] answers — the `'gc'` section's raw numbers.
pub struct GcSnapshot {
    pub collections: u64,
    pub pumps: u64,
    pub phase: &'static str,
    pub live_objects: u64,
    pub live_bytes: u64,
    pub debt: u64,
}

/// Drive the collector one step and account for it — the counted replacement for
/// a bare `arena.collect_debt()`. One phase read per pump, against a call that is
/// already doing tracing work.
///
/// `counters` is the handle the driver pulled once from its arena's root (see
/// [`GcCounters`]); re-reading it from the root per pump would mean a
/// `mutate_root` on the collector's hot path, whose write barrier can re-mark
/// the root mid-cycle.
#[inline]
pub fn pump_collector<R>(arena: &mut gc_arena::Arena<R>, counters: &GcCounters)
where
    R: for<'a> gc_arena::Rootable<'a>,
    for<'a> gc_arena::arena::Root<'a, R>: gc_arena::Collect<'a>,
{
    arena.collect_debt();
    counters.note_pump(arena.collection_phase());
}
