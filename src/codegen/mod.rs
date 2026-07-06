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

pub const TAG_OK: u8 = 0;
pub const TAG_DIV_ZERO: u8 = 1;
pub const TAG_DEPTH: u8 = 2;
pub const TAG_CANCELLED: u8 = 3;
/// The helper stored a full `QuoinError` in `VmState::aot_pending_error`
/// (outcall errors, IndexError, thrown values via `QuoinError::Thrown`, …).
pub const TAG_ERR: u8 = 4;

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
pub fn lookup(template_id: u32) -> Option<&'static AotEntry> {
    registry().read().unwrap().get(&template_id).copied()
}

/// How `compile_candidates` fared, for logs/tests (and, later, `VM.stats`).
#[derive(Default, Debug)]
pub struct CompileStats {
    pub compiled: usize,
    pub refused: Vec<(String, String)>,
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

/// Unbox the (dispatch-guaranteed) args, reserve this frame's slot window on
/// `vm.stack` (slot 0 = receiver, then object params, then nil-initialized
/// scratch — all GC-rooted by construction), run the compiled body, and box
/// the result. Fuel is reset at top-level entry only, so nested direct calls
/// share one budget like interpreted steps do.
pub fn invoke<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    entry: &'static AotEntry,
    receiver: Value<'gc>,
    args: &[Value<'gc>],
    enclosing_env: Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
) -> AotOutcome<'gc> {
    // NO COMPILED FRAMES INSIDE USER FIBERS: an abandoned fiber (a generator
    // dropped mid-iteration by `take:`) is torn down by corosensei's FORCED
    // UNWIND, which cannot cross Cranelift frames (no unwind info) — the
    // process aborts. A compiled body may suspend at any outcall or fuel
    // checkpoint, so the only sound rule is entry-level: inside a fiber,
    // Bail to the interpreted body (identical semantics, unwindable frames).
    // The main task and spawned tasks are torn down gracefully (cancellation
    // errors / process exit), so they keep compiled execution.
    if vm.sched.current_fiber.is_some() {
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
    if vm.aot_depth == 0 {
        vm.aot_fuel = i64::from(crate::tuning::step_batch());
    }
    vm.aot_pending_error = None;
    let saved_env = std::mem::replace(&mut vm.aot_enclosing_env, enclosing_env);
    let fuel_ptr = &raw mut vm.aot_fuel;
    let depth_ptr = &raw mut vm.aot_depth;
    let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
    let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
    let mut ret: i64 = 0;
    let tag = unsafe {
        (entry.raw)(
            vm_ptr,
            mc_ptr,
            fuel_ptr,
            depth_ptr,
            base as i64,
            raw.as_ptr(),
            &mut ret,
        )
    };
    vm.aot_enclosing_env = saved_env;
    let outcome = match tag {
        TAG_OK => AotOutcome::Value(match entry.ret {
            AotRet::Scalar(AotKind::Int) => Value::Int(ret),
            AotRet::Scalar(AotKind::Double) => Value::Double(f64::from_bits(ret as u64)),
            AotRet::Scalar(AotKind::Bool) => Value::Bool(ret != 0),
            AotRet::Obj => vm.stack[ret as usize],
        }),
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
            "AOT: compiled method returned unknown tag {other}"
        ))),
    };
    vm.stack.truncate(base);
    outcome
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
    // Tunable for debugging/tests (`QN_AOT_WARM=1` compiles on first use).
    static WARM: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let warm = *WARM.get_or_init(|| {
        std::env::var("QN_AOT_WARM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
    });
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
    // Same fiber gate as `invoke` — see the comment there.
    if vm.sched.current_fiber.is_some() {
        return AotOutcome::Bail;
    }
    let enclosing_env = match block_val {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::Block(b) => b.parent_env,
            _ => None,
        },
        _ => None,
    };
    let base = vm.stack.len();
    vm.stack.push(arg); // slot 0: self (vWSOA binds the arg)
    vm.stack.push(arg); // slot 1: the parameter (its own cell)
    vm.stack.push(block_val); // slot 2: the block object (env access)
    for _ in 0..entry.n_scratch {
        vm.stack.push(Value::Nil);
    }
    if vm.aot_depth == 0 {
        vm.aot_fuel = i64::from(crate::tuning::step_batch());
    }
    vm.aot_pending_error = None;
    let saved_env = std::mem::replace(&mut vm.aot_enclosing_env, enclosing_env);
    let fuel_ptr = &raw mut vm.aot_fuel;
    let depth_ptr = &raw mut vm.aot_depth;
    let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
    let mc_ptr = mc as *const gc_arena::Mutation<'gc> as *const c_void;
    let raw_args: [i64; 1] = [base as i64 + 1];
    let mut ret: i64 = 0;
    let tag = unsafe {
        (entry.raw)(
            vm_ptr,
            mc_ptr,
            fuel_ptr,
            depth_ptr,
            base as i64,
            raw_args.as_ptr(),
            &mut ret,
        )
    };
    vm.aot_enclosing_env = saved_env;
    let outcome = match tag {
        TAG_OK => AotOutcome::Value(vm.stack[ret as usize]),
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
            "AOT: compiled block returned unknown tag {other}"
        ))),
    };
    vm.stack.truncate(base);
    outcome
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
