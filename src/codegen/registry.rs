//! The AOT registry and tiering state: registered entries (`AotEntry`), the
//! global tables, redefinition epochs, warmth/direct-warm thresholds, retained
//! prior-site facts, baked-W0 staging, and retranslation.

use super::*;

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
    /// The candidate's selector (`block@<tid>` for block templates) — the
    /// handle `VM.aotCompiled` and the tier-shape pins identify entries by.
    pub selector: String,
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
    /// env swap/restore entirely (D2.5a, docs/internal/DIRECT_CALLS_ARCH.md §2).
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
    /// poison base (D3b, docs/internal/DIRECT_CALLS_ARCH.md §3.2).
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

/// W0 tier criteria (docs/internal/DIRECT_CALLS_ARCH.md §3.2): a callee a baked
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

#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // native-only caller is compiled out
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

pub(crate) fn registry() -> &'static RwLock<FxHashMap<u32, &'static AotEntry>> {
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
/// Mint a D2 outcall-site id (docs/internal/OUTCALL_ARCH.md): the index of this
/// compiled call site's cell in `VmState::aot_sites`. Monotonic and never
/// reused; retried translations waste a few — harmless.
/// D3a (docs/internal/DIRECT_CALLS_ARCH.md §3.3): retained retranslation inputs —
/// the candidate (the re-translation source) and the outcall site ids its
/// first translation minted per bytecode ip. The SAME ids must be reused on
/// retranslation so the D2 cells and the generic fallback keep working.
pub struct Retained {
    pub cand: AotCandidate,
    pub sites: FxHashMap<usize, u32>,
}

pub(crate) fn retained() -> &'static RwLock<FxHashMap<u32, Retained>> {
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
