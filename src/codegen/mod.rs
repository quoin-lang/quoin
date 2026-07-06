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

use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};

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
    pub fn from_annotation(name: &str) -> Option<AotParam> {
        if let Some(k) = AotKind::from_annotation(name) {
            return Some(AotParam::Scalar(k));
        }
        match name {
            // `Block`: any block value is an ordinary heap object in a slot;
            // compiled code interacts with it through outcalls only (B2).
            "List" | "Map" | "String" | "Block" => Some(AotParam::Obj),
            _ => None,
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
    pub fn from_annotation(name: &str) -> Option<AotRet> {
        if let Some(k) = AotKind::from_annotation(name) {
            return Some(AotRet::Scalar(k));
        }
        match name {
            "List" | "Map" | "String" => Some(AotRet::Obj),
            _ => None,
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

pub struct AotCandidate {
    pub group_id: u32,
    pub selector: String,
    pub block: Rc<StaticBlock>,
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

/// How `compile_candidates` fared, for logs/tests (and, later, `VM.stats`).
#[derive(Default, Debug)]
pub struct CompileStats {
    pub compiled: usize,
    pub refused: Vec<(String, String)>,
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
    let (compiled, refusals) = translate::compile_all(&cands, &siblings);
    {
        let mut reg = registry().write().unwrap();
        for (template_id, entry) in compiled {
            if verbose {
                eprintln!("qn aot: compiled template {template_id}");
            }
            reg.insert(template_id, Box::leak(Box::new(entry)));
            stats.compiled += 1;
        }
    }
    for (sel, why) in refusals {
        if verbose {
            eprintln!("qn aot: refused {sel}: {why}");
        }
        stats.refused.push((sel, why));
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

/// The entry gates every compiled invocation shares — checked BEFORE any
/// state changes, so a `false` (Bail to the interpreted body, identical
/// semantics) is always safe.
///
/// - NO COMPILED FRAMES INSIDE USER FIBERS: an abandoned fiber (a generator
///   dropped mid-iteration by `take:`) is torn down by corosensei's FORCED
///   UNWIND, which cannot cross Cranelift frames (no unwind info) — the
///   process aborts. A compiled body may suspend at any outcall or fuel
///   checkpoint, so the only sound rule is entry-level. The main task and
///   spawned tasks are torn down gracefully (cancellation errors / process
///   exit), so they keep compiled execution.
/// - S2: direct self-recursion bakes in "dispatch(self, sel) reaches this
///   template"; any later method-table mutation could change that, so a
///   stale epoch runs the interpreted body (which re-dispatches per send).
///   Templates never carry `direct_self`, so the check is a no-op for them.
#[inline(always)]
fn entry_gates(vm: &VmState<'_>, entry: &AotEntry) -> bool {
    if vm.sched.current_fiber.is_some() {
        return false;
    }
    if entry.direct_self && entry.compile_epoch != redef_epoch() {
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
    body: impl FnOnce(&mut VmState<'gc>) -> u8,
) -> (u8, Option<usize>) {
    if vm.aot.depth == 0 {
        vm.aot.fuel = i64::from(crate::tuning::step_batch());
    }
    vm.aot_pending_error = None;
    let saved_env = std::mem::replace(&mut vm.aot.enclosing_env, enclosing_env);
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
    vm.aot.enclosing_env = saved_env;
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

/// Unbox the (dispatch-guaranteed) args, reserve this frame's slot window on
/// `vm.stack` (slot 0 = receiver, then object params, then nil-initialized
/// scratch — all GC-rooted by construction), run the compiled body, and box
/// the result.
pub fn invoke<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
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
    let base = vm.stack.len();
    vm.stack.push(receiver); // slot 0
    let mut raw: Vec<i64> = Vec::with_capacity(args.len());
    for (a, k) in args.iter().zip(entry.params.iter()) {
        let bits = match (k, a) {
            (AotParam::Scalar(AotKind::Int), Value::Int(i)) => *i,
            (AotParam::Scalar(AotKind::Double), Value::Double(d)) => d.to_bits() as i64,
            (AotParam::Scalar(AotKind::Bool), Value::Bool(b)) => *b as i64,
            (AotParam::Obj, v @ Value::Object(_)) => {
                let idx = vm.stack.len() as i64;
                vm.stack.push(*v);
                idx
            }
            _ => {
                vm.stack.truncate(base);
                return AotOutcome::Bail;
            }
        };
        raw.push(bits);
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
    let (tag, minted) = run_in_frame_ctx(vm, enclosing_env, home, base, |vm| {
        let fuel_ptr = &raw mut vm.aot.fuel;
        let depth_ptr = &raw mut vm.aot.depth;
        let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
        let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
        unsafe {
            (entry.raw)(
                vm_ptr,
                mc_ptr,
                fuel_ptr,
                depth_ptr,
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

/// The compiled entry for a block template, compiling LAZILY on first use
/// (B3a): registry hit → done; else a pending candidate stashed at unit load
/// compiles now (once — a refusal tombstones). `None` = run interpreted.
pub fn block_entry_for<'gc>(vm: &mut VmState<'gc>, template_id: u32) -> Option<&'static AotEntry> {
    if let Some(entry) = lookup(template_id) {
        return (entry.role == AotRole::BlockTemplate).then_some(entry);
    }
    if vm.aot_refused_blocks.contains(&template_id) {
        return None;
    }
    // Tiering: a once-invoked block would pay Cranelift more than it saves —
    // compile only once a template proves warm. A combinator loop crosses the
    // threshold in its first handful of elements.
    let warm = warm_threshold();
    {
        let (count, _) = vm.aot_pending_blocks.get_mut(&template_id)?;
        *count += 1;
        if *count < warm {
            return None;
        }
    }
    let (_, cand) = vm.aot_pending_blocks.remove(&template_id)?;
    compile_candidates(vec![cand]);
    match lookup(template_id) {
        Some(entry) if entry.role == AotRole::BlockTemplate => Some(entry),
        _ => {
            vm.aot_refused_blocks.insert(template_id);
            None
        }
    }
}

/// Invoke a compiled BLOCK TEMPLATE (B3a) with `valueWithSelfOrArg:`
/// semantics: the argument is bound as BOTH `self` (slot 0) and the block's
/// parameter (slot 1 — a SEPARATE cell, so a param reassignment doesn't move
/// `self`, matching the interpreter's two env bindings); slot 2 roots the
/// block object itself, through which the `env_get`/`env_set` helpers reach
/// the closure's captured `EnvFrame` chain. Same fuel/depth regime as
/// `invoke` (nested compiled calls share one budget).
pub fn invoke_block<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    block_val: Value<'gc>,
    arg: Value<'gc>,
) -> AotOutcome<'gc> {
    debug_assert!(entry.role == AotRole::BlockTemplate);
    if !entry_gates(vm, entry) {
        return AotOutcome::Bail;
    }
    // The invoked closure's lexical parent AND its `^^` home: a closure the
    // template's cold path materializes belongs to the same home method this
    // closure does (S5) — including `None` (a homeless block's `^^` errors,
    // exactly as interpreted). A template is never a `^^` TARGET itself
    // (`HomeCtx::Propagate`, not `Mint` — so `finish_frame` never consumes).
    let (enclosing_env, home_id) = match block_val {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::Block(b) => (b.parent_env, b.enclosing_method_id),
            _ => (None, None),
        },
        _ => (None, None),
    };
    let base = vm.stack.len();
    vm.stack.push(arg); // slot 0: self (vWSOA binds the arg)
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
    let (tag, minted) = run_in_frame_ctx(vm, enclosing_env, home, base, |vm| {
        let fuel_ptr = &raw mut vm.aot.fuel;
        let depth_ptr = &raw mut vm.aot.depth;
        let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
        let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
        let raw_args: [i64; 1] = [base as i64 + 1];
        unsafe {
            (entry.raw)(
                vm_ptr,
                mc_ptr,
                fuel_ptr,
                depth_ptr,
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
