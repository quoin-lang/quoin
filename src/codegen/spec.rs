//! Speculative-AOT type feedback (S0, docs/SPECULATIVE_AOT_ARCH.md §3).
//!
//! Unannotated methods of AOT-eligible units are collected as *speculative
//! pending*: the interpreter merges the kinds it actually sees at method
//! entry (args) and method return into a small per-template lattice, only
//! while the template stays pending. S1 consumes a saturated profile as the
//! compile-time kinds + entry preconditions.

use rustc_hash::FxHashMap;

use crate::codegen::AotCandidate;

/// Kind lattice: `Unknown` rises to one scalar kind, any conflict (or any
/// non-scalar) lands on `Obj`. Nil observes as `Obj` — a nil-carrying param
/// can never scalar-speculate.
pub const K_UNKNOWN: u8 = 0;
pub const K_INT: u8 = 1;
pub const K_DOUBLE: u8 = 2;
pub const K_BOOL: u8 = 3;
pub const K_OBJ: u8 = 4;

pub fn merge(lat: u8, observed: u8) -> u8 {
    match (lat, observed) {
        (K_UNKNOWN, o) => o,
        (l, o) if l == o => l,
        _ => K_OBJ,
    }
}

/// A runtime value's lattice kind. Nil is `Obj` deliberately — a nil-carrying
/// value can never scalar-speculate (see the module doc).
pub fn kind_of(v: crate::value::Value<'_>) -> u8 {
    use crate::value::Value;
    match v {
        Value::Int(_) => K_INT,
        Value::Double(_) => K_DOUBLE,
        Value::Bool(_) => K_BOOL,
        _ => K_OBJ,
    }
}

/// Per-template state riding in `VmState.aot_spec_state`, indexed by
/// template id (dense: ids come from one global counter). Everything not
/// registered stays `NOT_SPECULATIVE`, so the interpreter's gate is one
/// bounds-checked byte load.
pub const NOT_SPECULATIVE: u8 = 0;
pub const OBSERVING: u8 = 1;
// (2 was a per-template SATURATED state that no code ever set — observation
// stops via the process-wide budget instead; the value stays unused so
// RESOLVED needn't renumber.)
/// Promoted (compiled, or refused by the translator) — never observed again.
pub const RESOLVED: u8 = 3;

/// Observations after which a profile stops merging (S0 has no compile
/// trigger yet; the cap bounds the observation cost on hot methods to a
/// prefix of their calls). S1's warmth threshold is expected to be lower.
pub const OBSERVE_CAP: u32 = 64;

/// Process-wide observation budget: once this many entry observations have
/// been merged, observation stops for good and the per-call cost collapses
/// to one predicted branch on a hot `VmState` field. Bounds S0's total cost
/// to well under a millisecond regardless of program length. Partial
/// profiles are fine — S1's entry guards never TRUST a profile, they CHECK
/// it, so an incomplete profile is merely conservative (more Bails), never
/// wrong.
pub const OBSERVE_BUDGET: u32 = 8192;

/// Entry-precondition Bails before a promoted entry is TOMBSTONED (registry
/// removal; the method runs interpreted from then on). Counted per entry in
/// `AotEntry::spec_bails`, reset on every passing entry — so only CONSECUTIVE
/// mispredictions kill a speculation.
pub const BAIL_TOMBSTONE: u32 = 8;

/// The scalar `AotKind` a saturated lattice slot speculates as (`None` =
/// ride as Obj, no precondition).
pub fn scalar_kind(lat: u8) -> Option<crate::codegen::AotKind> {
    match lat {
        K_INT => Some(crate::codegen::AotKind::Int),
        K_DOUBLE => Some(crate::codegen::AotKind::Double),
        K_BOOL => Some(crate::codegen::AotKind::Bool),
        _ => None,
    }
}

/// Compiled-call Rust-stack nesting cap: each compiled->interpreted
/// alternation (outcall -> call_method_cached -> ... -> AotCall) nests real
/// Rust frames on a fixed-size coroutine stack, so past this depth dispatch
/// runs the INTERPRETED body instead (flat frames; deep untyped recursion
/// degrades to the interpreter rather than overflowing or erroring).
pub const MAX_OUTCALL_NESTING: u32 = 48;

/// A speculative method waiting on its profile.
pub struct SpecPending {
    pub count: u32,
    /// One lattice slot per parameter (same order as `cand.params`).
    pub param_kinds: Vec<u8>,
    pub ret_kind: u8,
    pub cand: AotCandidate,
}

pub type SpecPendingMap = FxHashMap<u32, SpecPending>;

/// Human-readable one-liner for stats output.
pub fn kind_name(k: u8) -> &'static str {
    match k {
        K_UNKNOWN => "?",
        K_INT => "Int",
        K_DOUBLE => "Double",
        K_BOOL => "Bool",
        _ => "Obj",
    }
}
