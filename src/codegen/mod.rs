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

/// The scalar kinds the v0 subset compiles. `Boolean` is 0/1 in an i64 lane.
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

/// A method the compiler proved eligible for native compilation: sealed owner,
/// all-scalar params and return, unguarded, single-variant selector. `group_id`
/// identifies the class-body (or `.meta` extension) context it was defined in, so
/// self-calls resolve only among true siblings (same table, same receiver shape).
pub struct AotCandidate {
    pub group_id: u32,
    pub selector: String,
    pub block: Rc<StaticBlock>,
    pub params: Vec<AotKind>,
    pub ret: AotKind,
}

/// Raw ABI of a compiled trampoline. `args`/`ret` carry bit patterns (`f64` via
/// `to_bits`, bool as 0/1). Returns a tag: 0 ok, 1 division by zero, 2 compiled
/// call depth exceeded, 3 cancelled. `vm` is the erased `*mut VmState` (used only
/// by the fuel checkpoint); `fuel`/`depth` point at the per-task counters.
pub type AotRawFn = unsafe extern "C" fn(
    vm: *mut c_void,
    fuel: *mut i64,
    depth: *mut i64,
    args: *const i64,
    ret: *mut i64,
) -> u8;

pub const TAG_OK: u8 = 0;
pub const TAG_DIV_ZERO: u8 = 1;
pub const TAG_DEPTH: u8 = 2;
pub const TAG_CANCELLED: u8 = 3;

/// Compiled recursion consumes the real coroutine stack (1 MiB) and bypasses
/// `MAX_NATIVE_REENTRY`, so every compiled prologue counts call depth and bails
/// with a *catchable* error at this cap — well before the machine stack faults.
pub const AOT_MAX_CALL_DEPTH: i64 = 2000;

/// A registered compiled method. Leaked (`&'static`) so the fn pointer and its
/// signature live for the process, like the code itself (the finalized JIT module
/// is intentionally never dropped — same append-only lifetime as the interner).
pub struct AotEntry {
    pub raw: AotRawFn,
    pub params: Box<[AotKind]>,
    pub ret: AotKind,
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
    let mut siblings: HashMap<(u32, String), (Vec<AotKind>, AotKind, u32)> = HashMap::new();
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

/// Unbox the (dispatch-guaranteed) scalar args, run the compiled body, box the
/// result. Fuel is reset at top-level entry only, so nested direct calls share
/// one budget like interpreted steps do.
pub fn invoke<'gc>(
    vm: &mut VmState<'gc>,
    entry: &'static AotEntry,
    args: &[Value<'gc>],
) -> AotOutcome<'gc> {
    if args.len() != entry.params.len() {
        return AotOutcome::Bail;
    }
    let mut raw: Vec<i64> = Vec::with_capacity(args.len());
    for (a, k) in args.iter().zip(entry.params.iter()) {
        let bits = match (k, a) {
            (AotKind::Int, Value::Int(i)) => *i,
            (AotKind::Double, Value::Double(d)) => d.to_bits() as i64,
            (AotKind::Bool, Value::Bool(b)) => *b as i64,
            _ => return AotOutcome::Bail,
        };
        raw.push(bits);
    }
    if vm.aot_depth == 0 {
        vm.aot_fuel = i64::from(crate::tuning::step_batch());
    }
    let fuel_ptr = &raw mut vm.aot_fuel;
    let depth_ptr = &raw mut vm.aot_depth;
    let vm_ptr = vm as *mut VmState<'gc> as *mut c_void;
    let mut ret: i64 = 0;
    let tag = unsafe { (entry.raw)(vm_ptr, fuel_ptr, depth_ptr, raw.as_ptr(), &mut ret) };
    match tag {
        TAG_OK => AotOutcome::Value(match entry.ret {
            AotKind::Int => Value::Int(ret),
            AotKind::Double => Value::Double(f64::from_bits(ret as u64)),
            AotKind::Bool => Value::Bool(ret != 0),
        }),
        TAG_DIV_ZERO => {
            AotOutcome::Err(QuoinError::ArithmeticError("Division by zero".to_string()))
        }
        TAG_DEPTH => AotOutcome::Err(QuoinError::Other(
            "Maximum compiled-call depth exceeded (recursion too deep for native code)".to_string(),
        )),
        TAG_CANCELLED => AotOutcome::Err(vm.take_cancellation()),
        other => AotOutcome::Err(QuoinError::Other(format!(
            "AOT: compiled method returned unknown tag {other}"
        ))),
    }
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

/// The checkpoint's address, for registration as a JIT symbol.
pub(crate) fn checkpoint_addr() -> *const u8 {
    aot_checkpoint as *const u8
}
