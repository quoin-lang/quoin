//! The per-process loaded-unit cache: parse + compile artifacts for `use`-loaded
//! units, reused across VM sessions in one process.
//!
//! Every fresh session (a doc-check example, a `qn -e`, a future `qn check
//! --daemon` request) boots by replaying the prelude's `use core/*`, and ~60% of
//! that boot is re-deriving pure functions of the source text: the pest parse
//! and the bytecode compile. This cache keys a unit's compiled [`StaticBlock`] +
//! checker diagnostics so sessions 2..N skip straight to execution.
//!
//! **Chained keys.** A unit's compile is NOT a pure function of its own source
//! alone: unit N compiles against the accumulated `seen_types`/class table of
//! units 1..N-1 (unknown-type warnings, devirt against earlier-declared sealed
//! classes). So the cache key is a hash CHAIN — unit N's key folds in every
//! previously loaded unit's identity and source. Editing qnlib unit 5 under a
//! long-lived process (the daemon case) therefore invalidates exactly the
//! suffix that could have seen it, automatically: the loader re-reads and
//! re-hashes per load, so a changed file simply misses.
//!
//! **Why this is sound without side-effect replay.** The accumulator tables the
//! compile writes into (`SeenTypes`, `ClassTable`) are `Rc`-shared handles that
//! every session cloned from the same runner options — session 1's fill
//! persists, so a hit has nothing to re-record. AOT likewise: jitted code lives
//! in the process-global registry keyed by template id (`codegen::registry`),
//! and dispatch consults that registry directly — a cached unit keeps its
//! template ids, so later sessions hit session 1's compiled entries with no
//! re-registration. (They also never *trigger* new lazy/spec compiles for
//! stdlib templates — pending maps are per-VM — which is fine: the registry is
//! a pure accelerator, never a semantic dependency.)
//!
//! **Why `thread_local`.** The shared accumulators are `Rc` (not `Send`), and a
//! worker isolate booting on another thread builds fresh, empty tables — a hit
//! against another thread's cache would skip the compile those tables need.
//! Per-thread caches make the sharing argument hold by construction.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use crate::compiler::Diagnostic;
use crate::instruction::StaticBlock;

/// One loaded unit's reusable compile artifacts.
pub struct CachedUnit {
    /// The compiled program — arena-independent (`build_block` stamps it into
    /// any session's GC arena). Shared, so template interior state (spec
    /// counters, inline-cache ids) carries across sessions like it carries
    /// across calls within one.
    pub program: Arc<StaticBlock>,
    /// The compile's checker diagnostics, replayed on every hit so a warning a
    /// unit produced under session 1 still prints under session N (stdlib units
    /// are warning-clean; project units loaded via `use self:` may not be).
    pub diagnostics: Vec<Diagnostic>,
}

thread_local! {
    static CACHE: RefCell<HashMap<u64, Rc<CachedUnit>>> = RefCell::new(HashMap::new());
    static HITS: Cell<u64> = const { Cell::new(0) };
}

/// Fold the next unit's identity and source into the load chain. The returned
/// value is both the unit's cache key and the chain state for whatever loads
/// after it.
pub fn advance(chain: u64, package: Option<&str>, path: &str, source: &str) -> u64 {
    let mut h = DefaultHasher::new();
    chain.hash(&mut h);
    package.hash(&mut h);
    path.hash(&mut h);
    source.hash(&mut h);
    h.finish()
}

pub fn get(key: u64) -> Option<Rc<CachedUnit>> {
    let hit = CACHE.with(|c| c.borrow().get(&key).cloned());
    if hit.is_some() {
        HITS.with(|h| h.set(h.get() + 1));
    }
    hit
}

pub fn insert(key: u64, unit: CachedUnit) {
    CACHE.with(|c| c.borrow_mut().insert(key, Rc::new(unit)));
}

/// Cache hits on this thread so far — test observability.
pub fn hits() -> u64 {
    HITS.with(|h| h.get())
}

#[cfg(test)]
#[path = "unit_cache_tests.rs"]
mod unit_cache_tests;
