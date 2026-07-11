//! AOT native compilation of the typed subset (docs/AOT_ARCH.md).
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
pub mod spec;
mod translate;

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
    /// The owner class is OPEN (B2, docs/BLOCK_AOT_ARCH.md §3): the compiled
    /// form must contain no direct sibling calls — every send crosses a
    /// dispatch-equivalent seam, so a later reopen simply dispatches to its
    /// new template and the stale entry stops being reachable (the same
    /// per-dispatch minting argument as §6.2's no-deopt case).
    pub open_owner: bool,
    /// Speculative-AOT (S0, docs/SPECULATIVE_AOT_ARCH.md): `true` per param
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

/// A registered compiled method. Leaked (`&'static`) so the fn pointer and its
/// signature live for the process, like the code itself (the finalized JIT module
/// is intentionally never dropped — same append-only lifetime as the interner).
pub struct AotEntry {
    pub raw: AotRawFn,
    pub params: Box<[AotParam]>,
    pub ret: AotRet,
    /// Scratch slots (beyond receiver + object params) the frame needs.
    pub n_scratch: u32,
    /// Entry precondition (B2): the body contains a fused-`each:` loop over
    /// `self` compiled hot-path-only, so the receiver must be a native List —
    /// `invoke` Bails to the interpreted body (whose guarded loop handles any
    /// receiver exactly) when it isn't. Checked before any state changes.
    pub needs_list_self: bool,
    pub role: AotRole,
    /// The compiled template's id (the registry key), so the dispatch arm can
    /// tombstone a mispredicting speculation.
    pub template_id: u32,
    /// Speculated entry kind preconditions (S1): checked by the dispatch arm
    /// BEFORE `invoke` — a mismatching arg Bails to the interpreted body.
    /// Empty for classic annotated entries.
    pub param_preconditions: Box<[Option<AotKind>]>,
    /// Consecutive precondition Bails (reset on every pass); at
    /// `spec::BAIL_TOMBSTONE` the entry is tombstoned.
    pub spec_bails: std::sync::atomic::AtomicU32,
    /// The body contains DIRECT SELF-CALLS (S2 recursion fast path): valid
    /// only while `compile_epoch` matches the global redefinition epoch —
    /// `invoke` Bails otherwise.
    pub direct_self: bool,
    pub compile_epoch: u64,
    /// The body materializes at least one closure whose nest carries a `^^`
    /// (B3b/S5). Only such a frame can ever be a `^^` target — the compiled
    /// home id travels solely inside `^^`-carrying closures it materializes
    /// (`make_closure`'s `want_home`) — so `invoke` skips the S5 frame-mark
    /// and home-id bookkeeping entirely when this is false (the hot
    /// majority, including every `count:`-style write-back arm).
    pub materializes_nlr: bool,
    /// The body materializes ANY closure (superset of `materializes_nlr`).
    /// `vm.aot.enclosing_env` is consulted only by `make_closure`, so an
    /// entry that never materializes never reads it — `invoke` skips the
    /// env swap/restore entirely (D2.5a, docs/DIRECT_CALLS_ARCH.md §2).
    pub materializes: bool,
    /// The template is CLOSED (no captures/self/`^^`): its closures are
    /// cached per VM (constant-closure promotion), which makes baked
    /// identity guards DURABLE — a capture-bearing template materializes a
    /// fresh closure per call, so identity edges on it miss every element
    /// and their guard becomes pure tax (measured combinators +2.3%).
    pub is_closed: bool,
    /// Window-hoist: the body reads SLOT 0 (`self`) specifically. A baked
    /// block edge provides a real hoisted window whose self slot is never
    /// written per element — slot-0 readers are ineligible.
    pub uses_self_slot: bool,
    /// The body computes ANY absolute slot index (`abs_slot` — self reads,
    /// Dyn locals, field helpers, scratch). False = truly windowless: the
    /// entry never dereferences `slot_base`, so a baked W0 edge may pass a
    /// poison base (D3b, docs/DIRECT_CALLS_ARCH.md §3.2).
    pub uses_slot_base: bool,
    /// D2.5b marshaling plan, one i8 per param: for a verbatim-eligible
    /// scalar param (declared Scalar(K), S1 precondition absent or == K)
    /// this is the caller lane-kind constant (`helpers::KIND_*`) whose
    /// `bits` copy STRAIGHT into the raw lane — no `Value` decode, no
    /// re-encode, and the arg guard is one integer compare. `-1` = general
    /// lane (Obj params, precondition-narrowed params): full decode +
    /// cell guard + precondition, exactly the classic checks.
    pub lane_plan: Box<[i8]>,
}

/// W0 tier criteria (docs/DIRECT_CALLS_ARCH.md §3.2): a callee a baked
/// direct edge may call with NO window — all-scalar params, scalar ret, no
/// scratch, never touches its slot window, materializes nothing, and no
/// direct_self (its redef-epoch gate lives in `entry_gates`, which the
/// direct edge skips).
pub fn w0_eligible(entry: &AotEntry) -> bool {
    entry.role == AotRole::Method
        && matches!(entry.ret, AotRet::Scalar(_))
        && entry.n_scratch == 0
        && !entry.uses_slot_base
        && !entry.materializes
        && !entry.materializes_nlr
        && !entry.needs_list_self
        && !entry.direct_self
        && !entry.lane_plan.is_empty()
        && entry.lane_plan.iter().all(|&p| p >= 0)
}

/// W0-for-blocks (the window-hoist slice): a template a baked BLOCK edge
/// may call with a FRAME-HOISTED window — no scratch (nothing to re-nil
/// per element, the F2 invariant is vacuous), never touches its slot
/// window beyond what the caller provides (slots 0/2 provably unread via
/// `uses_slot_base`), materializes nothing. Blocks return via slot (`Obj`
/// eff-ret), so no ret-shape criterion.
pub fn block_w0_eligible(entry: &AotEntry) -> bool {
    entry.role == AotRole::BlockTemplate
        && entry.is_closed
        && !entry.materializes
        && !entry.materializes_nlr
        && !entry.uses_self_slot
}

/// One baked W0 site (D3b): the callee identity + the guard facts captured
/// from the D2 cell at bake time. `Copy` plain data — the entry is 'static.
#[derive(Clone, Copy)]
pub struct BakedW0 {
    pub entry: &'static AotEntry,
    /// `vm.dispatch_epoch` at bake time; the emitted guard compares the
    /// live value (through the ABI's epoch pointer) against this constant.
    pub epoch: u64,
    pub recv_kind: u8,
    pub recv_ptr: usize,
}

/// Staging: the driver's drain captures baked sites (it has VM access; the
/// translator does not), keyed by caller tid; the retranslation's
/// Translator takes them. Cleared on take.
fn baked_staging() -> &'static Mutex<FxHashMap<u32, FxHashMap<usize, BakedW0>>> {
    static S: OnceLock<Mutex<FxHashMap<u32, FxHashMap<usize, BakedW0>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(FxHashMap::default()))
}

pub fn stage_baked(tid: u32, sites: FxHashMap<usize, BakedW0>) {
    baked_staging().lock().unwrap().insert(tid, sites);
}

pub(super) fn take_baked_for(tid: u32) -> FxHashMap<usize, BakedW0> {
    baked_staging()
        .lock()
        .unwrap()
        .remove(&tid)
        .unwrap_or_default()
}

/// Baked direct-edge sites emitted across all retranslations (stats/tests).
pub static TOTAL_DIRECT_SITES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Build the D2.5b plan (see `AotEntry::lane_plan`).
pub fn build_lane_plan(params: &[AotParam], pres: &[Option<AotKind>]) -> Box<[i8]> {
    params
        .iter()
        .enumerate()
        .map(|(i, p)| match p {
            AotParam::Scalar(k) => {
                let pre = pres.get(i).copied().flatten();
                if pre.is_none() || pre == Some(*k) {
                    match k {
                        AotKind::Int => helpers::KIND_INT as i8,
                        AotKind::Double => helpers::KIND_DOUBLE as i8,
                        AotKind::Bool => helpers::KIND_BOOL as i8,
                    }
                } else {
                    -1
                }
            }
            AotParam::Obj => -1,
        })
        .collect()
}

/// `Callable`-embeddable handle: `Copy`, no GC content.
#[derive(Clone, Copy)]
pub struct AotFnRef(pub &'static AotEntry);

// A leaked &'static to plain data: nothing to trace.
unsafe impl<'gc> gc_arena::Collect<'gc> for AotFnRef {
    const NEEDS_TRACE: bool = false;
}

impl std::fmt::Debug for AotFnRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AotFnRef({:?} -> {:?})", self.0.params, self.0.ret)
    }
}

fn registry() -> &'static RwLock<FxHashMap<u32, &'static AotEntry>> {
    static REGISTRY: OnceLock<RwLock<FxHashMap<u32, &'static AotEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(FxHashMap::default()))
}

/// The compiled entry for a template id, if any. Probed only on the cold
/// `lookup_method` path — the dispatch cache and inline cache memoize the minted
/// `Callable` exactly like any other.
/// The lazy-compilation warmth threshold (block templates and speculative
/// methods alike). Tunable for debugging/tests: `QN_AOT_WARM=1` compiles on
/// first use — the corpus's maximal-speculation stress mode.
pub fn warm_threshold() -> u32 {
    static WARM: OnceLock<u32> = OnceLock::new();
    *WARM.get_or_init(|| {
        std::env::var("QN_AOT_WARM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
    })
}

/// Class-redefinition epoch (S2): bumped whenever a method table mutates
/// (`DefineMethod`, extension class installs). A compiled entry that emits
/// DIRECT SELF-CALLS records the epoch at compile time; `invoke` Bails the
/// entry to the interpreted body when the epochs differ — a redefinition
/// anywhere may change what a self-send should dispatch to (an override in a
/// new subclass included), and the interpreted body re-dispatches per send.
/// Shared across VMs: cross-VM bumps only cost conservative Bails.
static REDEF_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn redef_epoch() -> u64 {
    REDEF_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn bump_redef_epoch() {
    REDEF_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Remove a promoted entry whose speculation keeps mispredicting (S1
/// tombstone): new dispatches stop minting `AotCall`; call sites whose
/// inline caches still hold the entry keep failing its precondition and
/// Bailing — correct, just interpreted.
/// Mint a D2 outcall-site id (docs/OUTCALL_ARCH.md): the index of this
/// compiled call site's cell in `VmState::aot_sites`. Monotonic and never
/// reused; retried translations waste a few — harmless.
/// D3a (docs/DIRECT_CALLS_ARCH.md §3.3): retained retranslation inputs —
/// the candidate (the re-translation source) and the outcall site ids its
/// first translation minted per bytecode ip. The SAME ids must be reused on
/// retranslation so the D2 cells and the generic fallback keep working.
pub struct Retained {
    pub cand: AotCandidate,
    pub sites: FxHashMap<usize, u32>,
}

fn retained() -> &'static RwLock<FxHashMap<u32, Retained>> {
    static RETAINED: OnceLock<RwLock<FxHashMap<u32, Retained>>> = OnceLock::new();
    RETAINED.get_or_init(|| RwLock::new(FxHashMap::default()))
}

pub(super) fn prior_sites_for(tid: u32) -> Option<FxHashMap<usize, u32>> {
    retained()
        .read()
        .unwrap()
        .get(&tid)
        .map(|r| r.sites.clone())
}

/// The driver's drain needs a caller's retained site map to bake guard
/// facts from the live cells (D3b).
pub fn retained_sites_for(tid: u32) -> Option<FxHashMap<usize, u32>> {
    prior_sites_for(tid)
}

/// D3b bisect hooks (the S1 discipline — they land WITH the feature):
/// `QN_DIRECT_ONLY=tid,tid` limits which callers bake direct edges;
/// `QN_DIRECT_MAX=n` caps how many callers may bake (process-wide).
pub fn direct_allows(tid: u32) -> bool {
    static ONLY: OnceLock<Option<Vec<u32>>> = OnceLock::new();
    let only = ONLY.get_or_init(|| {
        std::env::var("QN_DIRECT_ONLY")
            .ok()
            .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
    });
    match only {
        Some(list) => list.contains(&tid),
        None => true,
    }
}

/// Test hook: `QN_DIRECT_NULL=1` retranslates queued callers even with no
/// baked sites (the D3a null-retranslation contract). Production skips
/// empty bakes: recompiling without edges buys nothing and costs fresh
/// code placement — measured +2-3% on hot benches (notes.md).
pub fn direct_null_forced() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| std::env::var("QN_DIRECT_NULL").is_ok_and(|v| v == "1"))
}

pub fn direct_budget_allows() -> bool {
    static MAX: OnceLock<Option<usize>> = OnceLock::new();
    static USED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let max = MAX.get_or_init(|| {
        std::env::var("QN_DIRECT_MAX")
            .ok()
            .and_then(|v| v.parse().ok())
    });
    match max {
        Some(cap) => USED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < *cap,
        None => true,
    }
}

/// How many warm-site retranslations have run (D3a: null retranslations —
/// identical code, registry overwrite). Surfaced by `VM.stats`.
pub static TOTAL_RETRANSLATED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// `QN_DIRECT_WARM`: site-hit threshold that queues the CALLER for
/// retranslation. Unset/0 = the tier is off (the D3a default; D3b flips the
/// default once direct edges exist to justify the recompile).
static DIRECT_WARM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

pub fn direct_warm_threshold() -> Option<u32> {
    let mut v = DIRECT_WARM.load(std::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        v = std::env::var("QN_DIRECT_WARM")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&n| n > 0 && n < u32::MAX)
            .unwrap_or(0);
        DIRECT_WARM.store(v, std::sync::atomic::Ordering::Relaxed);
    }
    (v != 0).then_some(v)
}

/// The outcall fast path's per-hit gate: one relaxed load + one branch when
/// the tier is off (measured: routing every hit through the accounting CALL
/// cost richards ~1.5%). `compile_candidates` resolves the env eagerly —
/// a hit requires a compiled entry, so the sentinel is never read hot; if
/// it somehow were, `true` merely routes into `aot_site_note_hit`, which
/// resolves and self-disables.
#[inline(always)]
/// Layout-pin accessors for value_layout_facts (helpers is pub(super)).
pub fn helpers_kind_int() -> i64 {
    helpers::KIND_INT
}
pub fn helpers_kind_nil() -> i64 {
    helpers::KIND_NIL
}

pub fn direct_warm_on() -> bool {
    DIRECT_WARM.load(std::sync::atomic::Ordering::Relaxed) != 0
}

/// Raw threshold for the seam's register-only warmth gate: 0 = off.
/// (`compile_candidates` eager-resolves the sentinel; see
/// [`direct_warm_threshold`].)
#[inline(always)]
pub fn direct_warm_raw() -> u32 {
    let v = DIRECT_WARM.load(std::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX { 0 } else { v }
}

/// Recompile a retained candidate and OVERWRITE its registry entry (§3.1:
/// in-flight invocations of the old leaked entry complete on their own
/// code). D3a emits IDENTICAL generic code — the null retranslation that
/// proves the queue, the site-id reuse, and the registry swap.
/// Wall-nanoseconds spent inside `retranslate` (attribution: on short
/// benches the Cranelift recompiles themselves are a visible slice).
pub static RETRANSLATE_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn retranslate(tid: u32) -> bool {
    let t0 = std::time::Instant::now();
    let out = retranslate_inner(tid);
    RETRANSLATE_NS.fetch_add(
        t0.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    out
}

fn retranslate_inner(tid: u32) -> bool {
    let (cand, group_cands) = {
        let r = retained().read().unwrap();
        let Some(ret) = r.get(&tid) else {
            return false;
        };
        let group = ret.cand.group_id;
        let group_cands: Vec<AotCandidate> = r
            .values()
            .filter(|x| x.cand.group_id == group)
            .map(|x| x.cand.clone())
            .collect();
        (ret.cand.clone(), group_cands)
    };
    // The sibling signature map exactly as the original group compile built
    // it — without it the retranslated body would lose its S2 direct
    // sibling calls and stop being "identical code".
    let mut siblings: HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)> = HashMap::new();
    for c in &group_cands {
        if let Some(id) = c.block.template_id {
            siblings.insert(
                (c.group_id, c.selector.clone()),
                (c.params.clone(), c.ret, id),
            );
        }
    }
    let cands = vec![cand];
    let (compiled, _refusals) = translate::compile_all(&cands, &siblings);
    let mut any = false;
    for (template_id, entry, sites) in compiled {
        registry()
            .write()
            .unwrap()
            .insert(template_id, Box::leak(Box::new(entry)));
        retained()
            .write()
            .unwrap()
            .entry(template_id)
            .and_modify(|r| r.sites = sites.iter().copied().collect());
        TOTAL_RETRANSLATED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        any = true;
    }
    any
}

pub fn next_outcall_site() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn tombstone(template_id: u32) {
    registry().write().unwrap().remove(&template_id);
}

/// Does a runtime value satisfy a speculated scalar-kind precondition?
pub fn scalar_matches(kind: AotKind, v: crate::value::Value<'_>) -> bool {
    use crate::value::Value;
    matches!(
        (kind, v),
        (AotKind::Int, Value::Int(_))
            | (AotKind::Double, Value::Double(_))
            | (AotKind::Bool, Value::Bool(_))
    )
}

/// Is a compiled entry registered for this template? (Promotion uses this to
/// distinguish a successful compile from a translator refusal.)
pub fn block_registered(template_id: u32) -> bool {
    registry().read().unwrap().contains_key(&template_id)
}

pub fn lookup(template_id: u32) -> Option<&'static AotEntry> {
    registry().read().unwrap().get(&template_id).copied()
}

/// Coarse buckets for WHY a member stayed interpreted — stable keys for the
/// `VM.stats` counters (the free-form `why` string carries the details, and
/// stays free-form precisely so these keys can be stable). The `Precheck*`
/// kinds are candidacy skips: the member never reached the translator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalKind {
    UnsupportedInstruction,
    UnsupportedConstant,
    /// `^^` inside a compiled block template (the recorded "template-^^" gap).
    NlrTemplate,
    /// `^^` meeting a catch-family send.
    NlrCatch,
    /// A `^^`-carrying closure escaping the compiled scope.
    NlrEscape,
    /// Per-iteration / guarded-nest materialization heuristics.
    MaterializationGate,
    /// Own-selector (recursive) materialization — the makeTree gate.
    RecursionGate,
    /// Write-capturing closure shapes (shared siblings, escapes, param/self writes).
    WriteCapture,
    /// A fused-combinator receiver or element shape that can't be proven.
    UnprovenReceiver,
    /// A value that must be slot-resident but isn't (self/nil at boundaries, cold stubs).
    SlotResidency,
    /// Local/return typing: unknown local, kind change, unprovable scalar.
    LocalTyping,
    /// The compiled ABI's 8-wide argument / list-literal caps.
    ArityCap,
    /// Structural bytecode limits (jump range, merges, underflow) — and the
    /// default for untagged helper errors.
    Structural,
    /// Candidacy skip: multi-variant (typed multimethod) selector.
    PrecheckMultiVariant,
    /// Candidacy skip: guard/decl-block member.
    PrecheckDeclBlock,
    /// Candidacy skip: a parameter/return shape with no scalar/Obj mapping.
    PrecheckSignature,
    /// Candidacy skip: block shape (>1 param, named, init-literal config,
    /// nested block literal, `^^` inside).
    PrecheckBlockShape,
}

impl RefusalKind {
    /// The stable camelCase key this bucket counts under in `VM.stats`.
    pub fn name(self) -> &'static str {
        match self {
            RefusalKind::UnsupportedInstruction => "unsupportedInstruction",
            RefusalKind::UnsupportedConstant => "unsupportedConstant",
            RefusalKind::NlrTemplate => "nlrTemplate",
            RefusalKind::NlrCatch => "nlrCatch",
            RefusalKind::NlrEscape => "nlrEscape",
            RefusalKind::MaterializationGate => "materializationGate",
            RefusalKind::RecursionGate => "recursionGate",
            RefusalKind::WriteCapture => "writeCapture",
            RefusalKind::UnprovenReceiver => "unprovenReceiver",
            RefusalKind::SlotResidency => "slotResidency",
            RefusalKind::LocalTyping => "localTyping",
            RefusalKind::ArityCap => "arityCap",
            RefusalKind::Structural => "structural",
            RefusalKind::PrecheckMultiVariant => "precheckMultiVariant",
            RefusalKind::PrecheckDeclBlock => "precheckDeclBlock",
            RefusalKind::PrecheckSignature => "precheckSignature",
            RefusalKind::PrecheckBlockShape => "precheckBlockShape",
        }
    }

    /// True for candidacy skips (`VM.stats` counts them as 'skipped', not 'refused').
    pub fn is_precheck(self) -> bool {
        matches!(
            self,
            RefusalKind::PrecheckMultiVariant
                | RefusalKind::PrecheckDeclBlock
                | RefusalKind::PrecheckSignature
                | RefusalKind::PrecheckBlockShape
        )
    }
}

/// A translation refusal traveling out of the bytecode walk: the coarse bucket
/// plus the human-readable detail. `From<String>`/`From<&str>` default to
/// [`RefusalKind::Structural`] so incidental helper errors (`ok_or("stack
/// underflow")?`) keep composing; every deliberate refusal site tags its kind.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub why: String,
}

impl From<String> for Refusal {
    fn from(why: String) -> Self {
        Refusal {
            kind: RefusalKind::Structural,
            why,
        }
    }
}

impl From<&str> for Refusal {
    fn from(why: &str) -> Self {
        Refusal {
            kind: RefusalKind::Structural,
            why: why.to_string(),
        }
    }
}

/// One recorded refusal or candidacy skip, for `VM.stats` / `VM.aotRefusals`.
#[derive(Debug, Clone)]
pub struct RefusalRecord {
    pub selector: String,
    pub kind: RefusalKind,
    pub why: String,
}

/// The process-lifetime refusal/skip log behind `VM.stats`. Bounded (a
/// pathological compile loop must not grow it without limit); appended on
/// final outcomes only — demote-retries that eventually compile never land
/// here. Reads dedup by (selector, kind, why): units recompile (REPL lines,
/// speculative re-attempts), and "distinct members refused" is the honest
/// statistic.
static REFUSAL_LOG: std::sync::Mutex<Vec<RefusalRecord>> = std::sync::Mutex::new(Vec::new());
const REFUSAL_LOG_CAP: usize = 4096;

/// Record one refusal (translator) or skip (candidacy pre-check).
pub fn record_refusal(selector: &str, kind: RefusalKind, why: &str) {
    let mut log = REFUSAL_LOG.lock().unwrap();
    if log.len() < REFUSAL_LOG_CAP {
        log.push(RefusalRecord {
            selector: selector.to_string(),
            kind,
            why: why.to_string(),
        });
    }
}

/// A deduplicated snapshot of the refusal/skip log (see [`REFUSAL_LOG`]).
pub fn refusal_snapshot() -> Vec<RefusalRecord> {
    let log = REFUSAL_LOG.lock().unwrap();
    let mut seen = HashSet::new();
    log.iter()
        .filter(|r| seen.insert((r.selector.clone(), r.kind, r.why.clone())))
        .cloned()
        .collect()
}

/// How `compile_candidates` fared, for logs/tests (`VM.stats` reads the
/// process-lifetime aggregates instead: `compile_totals` + `refusal_snapshot`).
#[derive(Default, Debug)]
pub struct CompileStats {
    pub compiled: usize,
    pub refused: Vec<RefusalRecord>,
}

/// Process-lifetime compile/refusal counters. Every `compile_candidates`
/// caller used to drop its `CompileStats`, so the ONLY record that a
/// candidate silently fell out of compilation was an env-gated eprintln —
/// no way to notice a coverage regression. `QN_AOT_STATS=1` surfaces these.
static TOTAL_COMPILED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TOTAL_REFUSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `(compiled, refused)` across the process so far.
pub fn compile_totals() -> (usize, usize) {
    use std::sync::atomic::Ordering;
    (
        TOTAL_COMPILED.load(Ordering::Relaxed),
        TOTAL_REFUSED.load(Ordering::Relaxed),
    )
}

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

/// Outcome of invoking a compiled method from dispatch.
pub enum AotOutcome<'gc> {
    Value(Value<'gc>),
    /// Argument shapes didn't match (shouldn't happen — dispatch selected the
    /// typed variant — but the interpreter path is always a safe answer).
    Bail,
    Err(QuoinError),
}

/// Entry bails since process start (`VM.stats` 'aot' section). Incremented
/// only on the rare bail path, never on successful entries.
static ENTRY_BAILS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Compiled-entry bails so far (stale-epoch, or a malformed fiber value).
pub fn entry_bails() -> usize {
    ENTRY_BAILS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The entry gates every compiled invocation shares — checked BEFORE any
/// state changes, so a `false` (Bail to the interpreted body, identical
/// semantics) is always safe.
///
/// - COMPILED FRAMES INSIDE USER FIBERS are allowed (the historical blanket
///   bail is gone): the fiber context swap carries a per-fiber
///   `AotTaskState`, and entry MARKS the fiber (`Fiber::ran_compiled`) so an
///   abandoned-suspended drop leaks its stack instead of force-unwinding
///   across Cranelift frames — see `Fiber::drop` for the invariant argument.
/// - S2: direct self-recursion bakes in "dispatch(self, sel) reaches this
///   template"; any later method-table mutation could change that, so a
///   stale epoch runs the interpreted body (which re-dispatches per send).
///   Templates never carry `direct_self`, so the check is a no-op for them.
#[inline(always)]
fn entry_gates(vm: &VmState<'_>, entry: &AotEntry) -> bool {
    if let Some(f) = vm.sched.current_fiber {
        let marked = f
            .with_native_state::<crate::runtime::fiber::NativeFiberState, _, _>(|s| {
                s.coro().ran_compiled.set(true);
            })
            .is_ok();
        if !marked {
            // Not a real fiber value (should not happen) — stay interpreted.
            ENTRY_BAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
    }
    if entry.direct_self && entry.compile_epoch != redef_epoch() {
        ENTRY_BAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return false;
    }
    true
}

/// The `^^`-home context a compiled invocation runs under (S5).
enum HomeCtx {
    /// A method frame that materializes a `^^`-carrying nest: mint a frame
    /// id, push its mark, publish it as the home.
    Mint,
    /// A block template that materializes a `^^`-carrying nest: propagate
    /// the invoked closure's own home (possibly `None` — a homeless block's
    /// `^^` errors exactly as interpreted).
    Propagate(Option<usize>),
    /// The body materializes no `^^` nest — the home is never consulted;
    /// skip all bookkeeping (the hot majority).
    Untracked,
}

/// Run one compiled body inside the frame context it needs — fuel reset at
/// top-level entry (nested direct calls share one budget like interpreted
/// steps), a clean error channel, the lexical parent for cold-path
/// materializations, and the `^^` home/mark per [`HomeCtx`] — with the
/// setup/teardown pairing enforced BY SCOPE: the closure is the only thing
/// that runs between them, so no future early return can leak a stale env,
/// home, or mark into the caller (the by-convention balance this replaces
/// is the exact shape of two shipped bugs). Returns the body's tag plus the
/// minted frame id, if any.
#[inline(always)]
fn run_in_frame_ctx<'gc>(
    vm: &mut VmState<'gc>,
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
    home: HomeCtx,
    base: usize,
    // D2.5a: the callee never materializes a closure, so `enclosing_env` is
    // never read during this body — skip the swap/restore pair. Nested
    // calls that DO materialize install their own env first.
    env_blind: bool,
    body: impl FnOnce(&mut VmState<'gc>) -> u8,
) -> (u8, Option<usize>) {
    if vm.aot.depth == 0 {
        vm.aot.fuel = i64::from(crate::tuning::step_batch());
    }
    vm.aot_pending_error = None;
    let saved_env = if env_blind {
        None
    } else {
        Some(std::mem::replace(&mut vm.aot.enclosing_env, enclosing_env))
    };
    let (minted, saved_home) = match home {
        HomeCtx::Mint => {
            let id = vm.next_frame_id;
            vm.next_frame_id += 1;
            vm.aot.frame_marks.push(crate::vm::AotFrameMark {
                id,
                frames_len: vm.frames.len(),
                stack_base: base,
            });
            (
                Some(id),
                Some(std::mem::replace(&mut vm.aot.home_frame_id, Some(id))),
            )
        }
        HomeCtx::Propagate(h) => (None, Some(std::mem::replace(&mut vm.aot.home_frame_id, h))),
        HomeCtx::Untracked => (None, None),
    };
    let tag = body(vm);
    if let Some(env) = saved_env {
        vm.aot.enclosing_env = env;
    }
    if let Some(h) = saved_home {
        vm.aot.home_frame_id = h;
    }
    if let Some(id) = minted {
        let popped = vm.aot.frame_marks.pop();
        debug_assert_eq!(popped.map(|m| m.id), Some(id));
    }
    (tag, minted)
}

/// The one tag → [`AotOutcome`] translation both entry points share; `ok`
/// supplies the TAG_OK value (scalar-from-lane for methods, slot-read for
/// blocks — the only shape difference). A new `TAG_*` is handled here or
/// nowhere.
#[inline(always)]
fn outcome_from_tag<'gc>(
    vm: &mut VmState<'gc>,
    tag: u8,
    // `Option`, not `Result`: this runs per compiled invocation (per ELEMENT
    // on combinator seams), and a Drop-glued `Result<_, QuoinError>` here is
    // real per-call cost; `None` = the checked return-slot read failed.
    ok: impl FnOnce(&mut VmState<'gc>) -> Option<Value<'gc>>,
) -> AotOutcome<'gc> {
    match tag {
        TAG_OK => match ok(vm) {
            Some(v) => AotOutcome::Value(v),
            // An OK tag with the ret slot truncated away means a
            // delivery/absorb protocol bug upstream — a catchable error
            // beats a panic that would abort across the Cranelift frames.
            None => AotOutcome::Err(QuoinError::Other(
                "AOT invariant violated: return slot past stack top".to_string(),
            )),
        },
        TAG_DIV_ZERO => {
            AotOutcome::Err(QuoinError::ArithmeticError("Division by zero".to_string()))
        }
        TAG_INT_OVERFLOW => {
            AotOutcome::Err(QuoinError::ArithmeticError("Integer overflow".to_string()))
        }
        TAG_DEPTH => AotOutcome::Err(QuoinError::Other(
            "Maximum compiled-call depth exceeded (recursion too deep for native code)".to_string(),
        )),
        TAG_CANCELLED => AotOutcome::Err(vm.take_cancellation()),
        TAG_ERR => AotOutcome::Err(vm.aot_pending_error.take().unwrap_or_else(|| {
            QuoinError::Other("AOT: TAG_ERR with no pending error".to_string())
        })),
        other => AotOutcome::Err(QuoinError::Other(format!(
            "AOT: compiled code returned unknown tag {other}"
        ))),
    }
}

/// The shared exit protocol: consume a `^^` that came home to THIS
/// invocation (the unwind already popped the outcall frames to our mark,
/// truncated the stack to our window base, and pushed the delivered value
/// there — to the caller, `^^v` from a cold arm IS the method returning
/// `v`), then tear down the slot window — EXCEPT when a non-local return
/// escaped through this frame: the `^^` unwind already truncated past the
/// window and pushed the delivered value at its target's stack base, which
/// can sit AT `base` (a caller whose operand stack was empty at the send);
/// truncating then would chop the delivered value off the stack (found by a
/// promoted `False#else:` whose arm block did `^^` — S1).
#[inline(always)]
fn finish_frame<'gc>(
    vm: &mut VmState<'gc>,
    outcome: AotOutcome<'gc>,
    base: usize,
    minted: Option<usize>,
) -> AotOutcome<'gc> {
    if let Some(frame_id) = minted
        && vm.aot.nlr_target == Some(frame_id)
    {
        vm.aot.nlr_target = None;
        debug_assert!(matches!(
            &outcome,
            AotOutcome::Err(QuoinError::NonLocalReturn)
        ));
        debug_assert_eq!(vm.stack.len(), base + 1);
        if let AotOutcome::Err(QuoinError::NonLocalReturn) = &outcome
            && let Some(v) = vm.stack.pop()
        {
            return AotOutcome::Value(v);
        }
    }
    if !matches!(&outcome, AotOutcome::Err(QuoinError::NonLocalReturn)) {
        vm.stack.truncate(base);
    }
    outcome
}

/// Unbox the (dispatch-guaranteed) args, establish this frame's slot window
/// on `vm.stack` (slot 0 = receiver, then one slot per param, then
/// nil-initialized scratch — all GC-rooted by construction), run the compiled
/// body, and box the result.
///
/// D1 (docs/OUTCALL_ARCH.md): when the caller already pushed the
/// `[receiver, args…]` rooting window (`window = Some(recv_start)` — every
/// outcall and interpreted send does, A2c/A2d), that window IS the frame's
/// slot window: nothing is re-pushed but the scratch. The translator's slot
/// layout reserves one slot per param (scalars waste theirs) precisely so the
/// two layouts coincide. All Bail paths fire BEFORE the scratch pushes, so a
/// bailing windowed call leaves the caller's window exactly as it found it.
pub fn invoke<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
    window: Option<usize>,
) -> AotOutcome<'gc> {
    if !entry_gates(vm, entry) {
        return AotOutcome::Bail;
    }
    if args.len() != entry.params.len() {
        return AotOutcome::Bail;
    }
    if entry.needs_list_self
        && receiver
            .with_native_state::<crate::runtime::list::NativeListState, _, _>(|_| ())
            .is_err()
    {
        // B2 entry precondition: the compiled body's fused loop assumes a
        // native-List self. Any other receiver runs the interpreted body,
        // whose guarded loop dispatches the real `each:` — exact semantics.
        return AotOutcome::Bail;
    }
    // Raw lanes without a heap allocation for the common arity (compiled
    // call sites are capped at 8 args; wider entries reach here only from
    // interpreted seams).
    let mut raw_buf = [0i64; 16];
    let mut raw_vec: Vec<i64>;
    let raw: &mut [i64] = if args.len() <= raw_buf.len() {
        &mut raw_buf[..args.len()]
    } else {
        raw_vec = vec![0; args.len()];
        &mut raw_vec
    };
    let base = window.unwrap_or(vm.stack.len());
    for (i, (a, k)) in args.iter().zip(entry.params.iter()).enumerate() {
        let bits = match (k, a) {
            (AotParam::Scalar(AotKind::Int), Value::Int(v)) => *v,
            (AotParam::Scalar(AotKind::Double), Value::Double(d)) => d.to_bits() as i64,
            (AotParam::Scalar(AotKind::Bool), Value::Bool(b)) => *b as i64,
            // The arg's window slot (existing or about-to-be-pushed below).
            (AotParam::Obj, Value::Object(_)) => (base + 1 + i) as i64,
            _ => return AotOutcome::Bail,
        };
        raw[i] = bits;
    }
    invoke_tail(vm, mc, entry, receiver, args, raw, enclosing_env, window)
}

/// D2.5b: the helper fast path enters here with lanes ALREADY marshaled
/// straight from the caller's `(kind,bits)` per the entry's `lane_plan` —
/// no `Value` decode/re-encode round trip. Gates and the list-self
/// precondition still apply; the arity was matched against the plan.
#[allow(clippy::too_many_arguments)]
pub fn invoke_prebuilt<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    raw: &[i64],
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
    window: Option<usize>,
) -> AotOutcome<'gc> {
    if !entry_gates(vm, entry) {
        return AotOutcome::Bail;
    }
    if entry.needs_list_self
        && receiver
            .with_native_state::<crate::runtime::list::NativeListState, _, _>(|_| ())
            .is_err()
    {
        return AotOutcome::Bail;
    }
    invoke_tail(vm, mc, entry, receiver, args, raw, enclosing_env, window)
}

/// The post-ladder body of [`invoke`]: window/scratch pushes, frame ctx, the
/// raw call, outcome. `raw` must already hold the lane bits per the entry's
/// param shapes (the D2.5b helper fast path builds them straight from the
/// caller's lanes and enters here — `invoke_prebuilt`).
#[allow(clippy::too_many_arguments)]
fn invoke_tail<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    raw: &[i64],
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
    window: Option<usize>,
) -> AotOutcome<'gc> {
    let base = window.unwrap_or(vm.stack.len());
    if window.is_none() {
        vm.stack.push(receiver); // slot 0
        for &a in args {
            vm.stack.push(a); // one slot per param, scalar or not
        }
    }
    for _ in 0..entry.n_scratch {
        vm.stack.push(Value::Nil);
    }
    // S5: a frame that materializes a `^^`-carrying nest is a potential `^^`
    // target (the compiled home id travels only inside such closures) —
    // frames that don't skip all bookkeeping.
    let home = if entry.materializes_nlr {
        HomeCtx::Mint
    } else {
        HomeCtx::Untracked
    };
    let mut ret: i64 = 0;
    // Window-arena: refresh the slot head the compiled code will read
    // (lazy discipline — see SlotStack::sync_head).
    vm.stack.sync_head();
    let env_blind = !entry.materializes && matches!(home, HomeCtx::Untracked);
    let (tag, minted) = run_in_frame_ctx(vm, enclosing_env, home, base, env_blind, |vm| {
        let fuel_ptr = &raw mut vm.aot.fuel;
        let depth_ptr = &raw mut vm.aot.depth;
        let epoch_ptr = &raw const vm.dispatch_epoch;
        let slots_ptr = vm.stack.head_addr();
        let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
        let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
        unsafe {
            (entry.raw)(
                vm_ptr,
                mc_ptr,
                fuel_ptr,
                depth_ptr,
                epoch_ptr,
                slots_ptr,
                base as i64,
                raw.as_ptr(),
                &mut ret,
            )
        }
    });
    let outcome = outcome_from_tag(vm, tag, |vm| match entry.ret {
        AotRet::Scalar(AotKind::Int) => Some(Value::Int(ret)),
        AotRet::Scalar(AotKind::Double) => Some(Value::Double(f64::from_bits(ret as u64))),
        AotRet::Scalar(AotKind::Bool) => Some(Value::Bool(ret != 0)),
        AotRet::Obj => vm.stack.get(ret as usize).copied(),
    });
    finish_frame(vm, outcome, base, minted)
}

/// The compiled entry for a block template, compiling LAZILY once warm
/// (B3a): registry hit → done; else a pending candidate stashed at unit load
/// compiles at the warmth threshold (once — a refusal tombstones). `None` =
/// run interpreted.
///
/// The warmth window doubles as S1-style ARGUMENT observation (`arg` is the
/// `valueWithSelfOrArg:` item): a one-param block whose observed args
/// saturate to one scalar kind compiles that param into a register lane with
/// an entry precondition — `invoke_block` checks it and Bails to the
/// interpreted body on mismatch, tombstoning after `BAIL_TOMBSTONE`
/// consecutive misses, exactly like a speculated method. This is what lets
/// `(x * 3) + 1` inside `collect:{ |x| … }` devirt to native arithmetic
/// instead of paying two classic outcalls per element.
pub fn block_entry_for<'gc>(
    vm: &mut VmState<'gc>,
    template_id: u32,
    arg: Value<'gc>,
) -> Option<&'static AotEntry> {
    if let Some(entry) = lookup(template_id) {
        return (entry.role == AotRole::BlockTemplate).then_some(entry);
    }
    if vm.aot_refused_blocks.contains(&template_id) {
        return None;
    }
    warm_pending_block(vm, template_id, 1, Some(spec::kind_of(arg)))
}

/// The interpreted `BranchIfNotList` guard's routing question: should this
/// fused `each:` site take the COLD path (the real send) instead of the
/// interpreted splice? Yes exactly when the argument block compiled WITH a
/// speculated scalar param — its body devirts, and the send path reaches it
/// per element via `invoke_block`, beating the splice ~2x (measured,
/// bench/micro). An Obj-param compiled block stays spliced: the send would
/// only wrap the same per-element outcalls in dispatch + entry shell
/// (measured +23% on maps). While the template is still pending, the guard
/// FEEDS its tiering instead: the list's elements are the very args the send
/// path would deliver, so warmth advances by the element count (one hot loop
/// crosses the threshold at its first call) and the observation lattice
/// merges the first element's kind (homogeneous lists dominate; a wrong
/// sample only costs precondition Bails, never wrong answers). A refused or
/// tombstoned template answers `false` forever — the splice remains the best
/// available tier.
pub fn fused_site_prefers_send<'gc>(
    vm: &mut VmState<'gc>,
    template_id: u32,
    len: usize,
    first: Option<Value<'gc>>,
) -> bool {
    let speculated = |entry: &AotEntry| {
        entry.role == AotRole::BlockTemplate
            && entry.param_preconditions.iter().any(|p| p.is_some())
    };
    if let Some(entry) = lookup(template_id) {
        return speculated(entry);
    }
    if vm.aot_refused_blocks.contains(&template_id) {
        return false;
    }
    let n = u32::try_from(len).unwrap_or(u32::MAX);
    warm_pending_block(vm, template_id, n, first.map(spec::kind_of)).is_some_and(speculated)
}

/// Advance a pending block template's warmth/observation by `n` invocations
/// (kind `Some(k)` merges into the argument lattice) and compile at the
/// threshold; a refusal tombstones. Returns the entry once compiled.
///
/// Tiering rationale: a once-invoked block would pay Cranelift more than it
/// saves — compile only once a template proves warm. A combinator loop
/// crosses the threshold in its first handful of elements.
fn warm_pending_block<'gc>(
    vm: &mut VmState<'gc>,
    template_id: u32,
    n: u32,
    kind: Option<u8>,
) -> Option<&'static AotEntry> {
    let warm = warm_threshold();
    {
        let (count, arg_kind, _) = vm.aot_pending_blocks.get_mut(&template_id)?;
        *count = count.saturating_add(n);
        if let Some(k) = kind {
            *arg_kind = spec::merge(*arg_kind, k);
        }
        if *count < warm {
            return None;
        }
    }
    let (_, arg_kind, mut cand) = vm.aot_pending_blocks.remove(&template_id)?;
    // Zero-param blocks receive the item as `self`, not as the param — only
    // a real one-param block speculates its (single) argument lane. And only
    // when the body has something a scalar lane can devirt: the lane costs a
    // per-invocation precondition branch plus a return re-boxing (~+12% on an
    // identity block, measured), so a body with no scalar-op sends stays a
    // slot-resident Obj.
    if cand.block.param_syms.len() == 1
        && block_body_has_scalar_ops(&cand.block)
        && let Some(kind) = spec::scalar_kind(arg_kind)
    {
        cand.params = vec![AotParam::Scalar(kind)];
        cand.spec_preconditions = vec![Some(kind)];
    }
    compile_candidates(vec![cand]);
    match lookup(template_id) {
        Some(entry) if entry.role == AotRole::BlockTemplate => Some(entry),
        _ => {
            vm.aot_refused_blocks.insert(template_id);
            None
        }
    }
}

/// Does this block body contain a send a scalar param could devirt — any
/// selector `IntBinKind` recognizes (the same table the translator's
/// scalar-op devirt keys on, so the heuristic can't drift from the payoff)?
fn block_body_has_scalar_ops(block: &crate::instruction::StaticBlock) -> bool {
    use crate::instruction::{Instruction as I, IntBinKind};
    block.bytecode.0.iter().any(|inst| {
        let sel = match inst {
            I::Send(s, _)
            | I::SendLocal(_, s, _)
            | I::SendConst(_, s, _)
            | I::SendField(_, s, _)
            | I::SendLocalLocal(_, _, s, _)
            | I::SendLocalConst(_, _, s, _) => *s,
            _ => return false,
        };
        IntBinKind::from_selector(sel.as_str()).is_some()
    })
}

/// Invoke a compiled BLOCK TEMPLATE (B3a) with `valueWithSelfOrArg:`
/// semantics: the argument is bound as BOTH `self` (slot 0) and the block's
/// parameter (slot 1 — a SEPARATE cell, so a param reassignment doesn't move
/// `self`, matching the interpreter's two env bindings); slot 2 roots the
/// block object itself, through which the `env_get`/`env_set` helpers reach
/// the closure's captured `EnvFrame` chain. Same fuel/depth regime as
/// `invoke` (nested compiled calls share one budget).
/// `self` for a self-or-arg invocation: the item for a parameterless block
/// (the `{ .name }` shorthand), the LEXICAL self (resolved through the
/// closure's parent env chain) for a parameterized one. Compiled and
/// interpreted tiers must agree — the interpreted fix lives in
/// `valueWithSelfOrArg:` (block.rs); this is the shared answer for the
/// compiled frames, whose templates read `self` from slot 0.
pub fn self_or_arg_self<'gc>(block: &crate::value::Block<'gc>, arg: Value<'gc>) -> Value<'gc> {
    if block.template.param_syms.is_empty() {
        return arg; // implicit-self shorthand: the item IS self
    }
    // Parameterized: lexical self — but the env walk only pays off for
    // blocks that actually reference self/@fields; for the common no-self
    // block the slot is dead and the item is as good a filler as any.
    if !crate::instruction::template_uses_self(&block.template) {
        return arg;
    }
    block
        .parent_env
        .and_then(|env| crate::value::EnvFrame::get(env, crate::symbol::self_symbol()))
        .unwrap_or(Value::Nil)
}

pub fn invoke_block<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    block_val: Value<'gc>,
    block: gc_arena::Gc<'gc, crate::value::Block<'gc>>,
    arg: Value<'gc>,
    self_val: Value<'gc>,
) -> AotOutcome<'gc> {
    debug_assert!(entry.role == AotRole::BlockTemplate);
    if !entry_gates(vm, entry) {
        return AotOutcome::Bail;
    }
    // Speculated-argument precondition (the block-side S1 gate): a scalar
    // lane was compiled from the warmth window's observations — check the
    // live argument BEFORE any stack effect, Bail to the interpreted body on
    // mismatch, and tombstone after BAIL_TOMBSTONE consecutive misses (the
    // speculation was wrong about this program).
    let mut arg_lane: Option<i64> = None;
    if let Some(&Some(kind)) = entry.param_preconditions.first() {
        use std::sync::atomic::Ordering;
        arg_lane = match (kind, arg) {
            (AotKind::Int, Value::Int(v)) => Some(v),
            (AotKind::Double, Value::Double(d)) => Some(d.to_bits() as i64),
            (AotKind::Bool, Value::Bool(b)) => Some(b as i64),
            _ => None,
        };
        if arg_lane.is_none() {
            let bails = entry.spec_bails.fetch_add(1, Ordering::Relaxed) + 1;
            if bails >= spec::BAIL_TOMBSTONE {
                tombstone(entry.template_id);
            }
            return AotOutcome::Bail;
        }
        entry.spec_bails.store(0, Ordering::Relaxed);
    }
    // The invoked closure's lexical parent AND its `^^` home: a closure the
    // template's cold path materializes belongs to the same home method this
    // closure does (S5) — including `None` (a homeless block's `^^` errors,
    // exactly as interpreted). A template is never a `^^` TARGET itself
    // (`HomeCtx::Propagate`, not `Mint` — so `finish_frame` never consumes).
    // The caller already holds the block payload — no second object borrow.
    let (enclosing_env, home_id) = (block.parent_env, block.enclosing_method_id);
    let base = vm.stack.len();
    vm.stack.push(self_val); // slot 0: self (self-or-arg, resolved by the caller)
    vm.stack.push(arg); // slot 1: the parameter (its own cell)
    vm.stack.push(block_val); // slot 2: the block object (env access)
    for _ in 0..entry.n_scratch {
        vm.stack.push(Value::Nil);
    }
    let home = if entry.materializes_nlr {
        HomeCtx::Propagate(home_id)
    } else {
        HomeCtx::Untracked
    };
    let mut ret: i64 = 0;
    // Window-arena: refresh the slot head the compiled code will read
    // (lazy discipline — see SlotStack::sync_head).
    vm.stack.sync_head();
    let env_blind = !entry.materializes && matches!(home, HomeCtx::Untracked);
    let (tag, minted) = run_in_frame_ctx(vm, enclosing_env, home, base, env_blind, |vm| {
        let fuel_ptr = &raw mut vm.aot.fuel;
        let depth_ptr = &raw mut vm.aot.depth;
        let epoch_ptr = &raw const vm.dispatch_epoch;
        let slots_ptr = vm.stack.head_addr();
        let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
        let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
        // A speculated scalar rides its lane; an Obj param takes its window
        // slot's index (slot 1), exactly like a method Obj param.
        let raw_args: [i64; 1] = [arg_lane.unwrap_or(base as i64 + 1)];
        unsafe {
            (entry.raw)(
                vm_ptr,
                mc_ptr,
                fuel_ptr,
                depth_ptr,
                epoch_ptr,
                slots_ptr,
                base as i64,
                raw_args.as_ptr(),
                &mut ret,
            )
        }
    });
    let outcome = outcome_from_tag(vm, tag, |vm| vm.stack.get(ret as usize).copied());
    finish_frame(vm, outcome, base, minted)
}

/// Fuel checkpoint, called from compiled code when the fuel counter hits zero.
/// Mirrors `run_dispatch`'s per-instruction contract: check cancellation, then
/// cooperatively yield (the same `yielder.suspend` mechanism `await_io` uses —
/// compiled frames hold only scalars, so suspending here needs no rooting), check
/// cancellation again after resume (one may have arrived while parked), then
/// refill the fuel budget.
///
/// # Safety
/// `vm` must be the live `*mut VmState` passed by [`invoke`] for the current
/// resume segment; the `'gc` erasure matches the `VMContext`/`Scheduler.yielder`
/// precedent (no `'gc` value is created or held here — `CooperativeYield` carries
/// none).
unsafe extern "C" fn aot_checkpoint(vm: *mut c_void, fuel: *mut i64) -> u8 {
    let vm = unsafe { &mut *(vm as *mut VmState<'_>) };
    if vm.sched.cancel_current {
        return TAG_CANCELLED;
    }
    if let Some(yielder) = unsafe { vm.get_yielder() } {
        yielder.suspend(YieldReason::CooperativeYield);
    }
    if vm.sched.cancel_current {
        return TAG_CANCELLED;
    }
    unsafe { *fuel = i64::from(crate::tuning::step_batch()) };
    TAG_OK
}
