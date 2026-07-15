//! AOT native compilation of the typed subset (docs/internal/AOT_ARCH.md).
//!
//! The compiler collects [`AotCandidate`]s — methods of in-unit `sealed!` classes
//! whose params and return are all scalar (`Integer`/`Double`/`Boolean`) — and the
//! runner, when `QN_AOT=1`, hands them to [`compile_candidates`], which translates
//! each post-inlining bytecode body to native code via Cranelift and registers the
//! result in a process-global, append-only registry keyed by the block literal's
//! `template_id`. Dispatch (`lookup_method`) mints [`Callable::AotCall`] for a
//! registered template; the interpreter path is untouched and remains the
//! authoritative fallback (`QN_AOT=0` is the kill switch — the registry is a pure
//! overlay over unchanged bytecode).
//!
//! Soundness posture: no speculation, no deopt. Candidacy is refused (never
//! guarded) when anything can't be proven; runtime semantics are pinned to
//! `devirt_ops` (wrapping i64 arithmetic, zero-divisor-only `ArithmeticError`,
//! never-raising f64). Scheduling and cancellation use fuel checkpoints through the
//! same `yielder.suspend` mechanism natives already use (`await_io`); compiled
//! frames hold only scalars, so suspending needs no rooting (the resume-segment GC
//! rule — gc-arena collects only between coroutine resumes).

mod helpers;
mod invoke;
mod refusal;
mod registry;
pub mod spec;

// The flat `crate::codegen::<Name>` surface predates the split; the glob re-exports
// keep every consumer (and tests' `use super::*`) unchanged.
pub use invoke::*;
// Non-pub items cross the glob only explicitly (used from translate/).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use invoke::aot_checkpoint;
pub use refusal::*;
pub use registry::*;
#[cfg(not(target_arch = "wasm32"))]
mod translate;

/// wasm32: no executable memory, no JIT — translation refuses everything by compiling
/// nothing. Everything else in this module (candidate collection, the registry, spec
/// state, stats) is plain data and compiles unchanged; the registry just stays empty,
/// so dispatch never mints an `AotCall` and the interpreter (the authoritative
/// fallback, same as `QN_AOT=0`) runs everything.
#[cfg(target_arch = "wasm32")]
mod translate {
    use super::{AotCandidate, AotEntry, AotParam, AotRet, Refusal};
    use std::collections::HashMap;

    #[allow(clippy::type_complexity)]
    pub(super) fn compile_all(
        _cands: &[AotCandidate],
        _siblings: &HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)>,
    ) -> (
        Vec<(u32, AotEntry, Vec<(usize, u32)>)>,
        Vec<(String, Refusal)>,
    ) {
        (Vec::new(), Vec::new())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock, RwLock};

use rustc_hash::FxHashMap;

use crate::error::QuoinError;
use crate::fiber::YieldReason;
use crate::instruction::StaticBlock;
use crate::value::Value;
use crate::vm::VmState;

/// The scalar kinds the compiled subset carries in registers. `Boolean` is
/// 0/1 in an i64 lane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AotKind {
    Int,
    Double,
    Bool,
}

impl AotKind {
    pub fn from_annotation(name: &str) -> Option<AotKind> {
        match name {
            "Integer" => Some(AotKind::Int),
            "Double" => Some(AotKind::Double),
            "Boolean" => Some(AotKind::Bool),
            _ => None,
        }
    }
}

/// A compiled method's parameter shape: scalars ride in registers; objects
/// (List/Map/String — any dispatch-guaranteed class annotation) live in the
/// frame's slot window on `vm.stack` and are addressed by index (v0.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AotParam {
    Scalar(AotKind),
    Obj,
}

impl AotParam {
    /// Scalar names ride in registers; EVERY other annotation is a boxed value
    /// in a slot (`Obj` assumes nothing, so any class name — `Block`, a user
    /// class, an erased type variable's `Object`, a nullable `Integer?` — is
    /// sound). Nullable scalars land here too, never on `Scalar`: the name
    /// keeps its `?`, so it can't match the scalar arm, and a nil-carrying
    /// param must not compile into a register lane.
    pub fn from_annotation(name: &str) -> AotParam {
        match AotKind::from_annotation(name) {
            Some(k) => AotParam::Scalar(k),
            None => AotParam::Obj,
        }
    }
}

/// A compiled method's return shape. `Obj` returns the value via a slot (the
/// raw `ret` lane carries the absolute slot index).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AotRet {
    Scalar(AotKind),
    Obj,
}

impl AotRet {
    /// Same widening as [`AotParam::from_annotation`]: scalars by name, all
    /// else `Obj`. This is what lets `detect: -> { … ^T? }` be a candidate at
    /// all — its return erases to `Object`, which used to end candidacy
    /// (`precheckSignature`) and silently kept the whole `^T`/`^T?`/`^Object`
    /// family interpreted.
    pub fn from_annotation(name: &str) -> AotRet {
        match AotKind::from_annotation(name) {
            Some(k) => AotRet::Scalar(k),
            None => AotRet::Obj,
        }
    }
}

/// A method the compiler proved eligible for native compilation: sealed owner,
/// all-scalar params and return, unguarded, single-variant selector. `group_id`
/// identifies the class-body (or `.meta` extension) context it was defined in, so
/// self-calls resolve only among true siblings (same table, same receiver shape).
/// What kind of unit a candidate/entry compiles (B3a). A METHOD's params are
/// dispatch-guaranteed and its `^^` is its own return; a BLOCK TEMPLATE is a
/// literal invoked via `valueWithSelfOrArg:` — its param is an arbitrary
/// value (slot-resident `Obj`), its free names resolve through the closure's
/// real `EnvFrame` chain (`env_get`/`env_set` helpers — exact shared-cell
/// semantics), and a `^^` refuses (no frame to unwind to).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AotRole {
    Method,
    BlockTemplate,
}

#[derive(Clone)]
pub struct AotCandidate {
    pub group_id: u32,
    pub selector: String,
    pub block: Arc<StaticBlock>,
    pub params: Vec<AotParam>,
    pub ret: AotRet,
    pub role: AotRole,
    /// The owner class is OPEN (B2, docs/internal/BLOCK_AOT_ARCH.md §3): the compiled
    /// form must contain no direct sibling calls — every send crosses a
    /// dispatch-equivalent seam, so a later reopen simply dispatches to its
    /// new template and the stale entry stops being reachable (the same
    /// per-dispatch minting argument as §6.2's no-deopt case).
    pub open_owner: bool,
    /// Speculative-AOT (S0, docs/internal/SPECULATIVE_AOT_ARCH.md): `true` per param
    /// whose kind is UNANNOTATED — `params[i]` holds an Obj placeholder and
    /// the runtime profile supplies the real kind at compile time. All-false
    /// for classic annotated candidates.
    pub spec_params: Vec<bool>,
    /// The return annotation is absent; `ret` is an Obj placeholder.
    pub spec_ret: bool,
    /// Entry kind preconditions minted at PROMOTION (S1): `Some(kind)` per
    /// param whose scalar kind is speculated from the runtime profile rather
    /// than guaranteed by dispatch. Empty until promotion; always empty for
    /// classic annotated candidates.
    pub spec_preconditions: Vec<Option<AotKind>>,
}

impl AotCandidate {
    /// A candidate that must wait for a runtime type profile (any observed
    /// slot) rather than compiling at unit load.
    pub fn speculative(&self) -> bool {
        self.spec_ret || self.spec_params.iter().any(|&b| b)
    }
}

/// Raw ABI of a compiled trampoline. `args`/`ret` carry bit patterns (`f64` via
/// `to_bits`, bool as 0/1). Returns a tag: 0 ok, 1 division by zero, 2 compiled
/// call depth exceeded, 3 cancelled. `vm` is the erased `*mut VmState` (used only
/// by the fuel checkpoint); `fuel`/`depth` point at the per-task counters.
/// `slot_base` is the absolute index of this frame's slot window on
/// `vm.stack` (slot 0 = receiver, then object params, then scratch); `args`
/// carries one lane per declared parameter — scalar bits, or the absolute
/// slot index for `Obj` params.
pub type AotRawFn = unsafe extern "C" fn(
    vm: *mut c_void,
    mc: *const c_void,
    fuel: *mut i64,
    depth: *mut i64,
    // D3a: pointer to the VM's `dispatch_epoch`, rides beside fuel/depth so
    // D3b's baked-guard sites can compare epochs without raw VmState field
    // offsets.
    epoch: *const u64,
    // A3 (window arena): the VM's SlotStack head — compiled code re-loads
    // (ptr, len) through it per slot access and does native bounds-checked
    // loads/stores against Value's fixed layout. Passed per call (NOT baked:
    // portable-block localization preserves template ids, so an entry can
    // run on a VM other than its compiler). The 9th param spills one slot
    // past the ARM64 register budget — measured under the A3 gate.
    slots: *mut crate::value::SlotHead,
    slot_base: i64,
    args: *const i64,
    ret: *mut i64,
) -> u8;

/// Status tags returned by every compiled body and helper. `TAG_OK` must be
/// 0 (compiled code branches on nonzero = error). The error tags start at
/// 0x11 so the TAG range is DISJOINT from the value-lane KIND range
/// (`helpers::KIND_*`, 0–4) that shares the same integer ABI — a mis-wired
/// value channel fed to a tag check then fails loudly as "unknown tag"
/// instead of aliasing a valid KIND into a spurious cancellation or
/// phantom error.
pub const TAG_OK: u8 = 0;
pub const TAG_DIV_ZERO: u8 = 0x11;
pub const TAG_DEPTH: u8 = 0x12;
pub const TAG_CANCELLED: u8 = 0x13;
/// The helper stored a full `QuoinError` in `VmState::aot_pending_error`
/// (outcall errors, IndexError, thrown values via `QuoinError::Thrown`, …).
pub const TAG_ERR: u8 = 0x14;
pub const TAG_INT_OVERFLOW: u8 = 0x15;

/// Compiled recursion consumes the real coroutine stack (1 MiB) and bypasses
/// `MAX_NATIVE_REENTRY`, so every compiled prologue counts call depth and bails
/// with a *catchable* error at this cap — well before the machine stack faults.
pub const AOT_MAX_CALL_DEPTH: i64 = 2000;

/// Translate and register every candidate that survives the authoritative
/// bytecode walk. Refusal is silent and safe (the method stays interpreted);
/// `QN_AOT_VERBOSE=1` prints per-method outcomes to stderr.
pub fn compile_candidates(cands: Vec<AotCandidate>) -> CompileStats {
    let _ = direct_warm_threshold(); // eager-resolve the fast path's gate
    let mut stats = CompileStats::default();
    if cands.is_empty() {
        return stats;
    }
    // Sibling signature map for direct self-calls: (group, selector) -> sig.
    let mut siblings: HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)> = HashMap::new();
    for c in &cands {
        if let Some(id) = c.block.template_id {
            siblings.insert(
                (c.group_id, c.selector.clone()),
                (c.params.clone(), c.ret, id),
            );
        }
    }
    let verbose = std::env::var("QN_AOT_VERBOSE").is_ok_and(|v| v == "1");
    let by_tid: HashMap<u32, &AotCandidate> = cands
        .iter()
        .filter_map(|c| c.block.template_id.map(|id| (id, c)))
        .collect();
    let (compiled, refusals) = translate::compile_all(&cands, &siblings);
    {
        let mut reg = registry().write().unwrap();
        let mut ret = retained().write().unwrap();
        for (template_id, entry, sites) in compiled {
            if verbose {
                eprintln!("qn aot: compiled template {template_id}");
            }
            reg.insert(template_id, Box::leak(Box::new(entry)));
            // D3a: retain the retranslation inputs (candidate + site ids).
            if let Some(c) = by_tid.get(&template_id) {
                ret.insert(
                    template_id,
                    Retained {
                        cand: (*c).clone(),
                        sites: sites.iter().copied().collect(),
                    },
                );
            }
            stats.compiled += 1;
        }
    }
    for (sel, refusal) in refusals {
        if verbose {
            eprintln!(
                "qn aot: refused {sel} [{}]: {}",
                refusal.kind.name(),
                refusal.why
            );
        }
        record_refusal(&sel, refusal.kind, &refusal.why);
        stats.refused.push(RefusalRecord {
            selector: sel,
            kind: refusal.kind,
            why: refusal.why,
        });
    }
    use std::sync::atomic::Ordering;
    TOTAL_COMPILED.fetch_add(stats.compiled, Ordering::Relaxed);
    TOTAL_REFUSED.fetch_add(stats.refused.len(), Ordering::Relaxed);
    stats
}
