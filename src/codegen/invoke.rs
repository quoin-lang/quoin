//! Running compiled bodies from dispatch: entry gating, frame contexts, tag ->
//! outcome mapping, the invoke family, and the lazy block-template runtime.

use super::*;

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
            (Some(id), Some(vm.aot.home_frame_id.replace(id)))
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
/// D1 (docs/internal/OUTCALL_ARCH.md): when the caller already pushed the
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
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // native-only caller is compiled out
pub(crate) unsafe extern "C" fn aot_checkpoint(vm: *mut c_void, fuel: *mut i64) -> u8 {
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
