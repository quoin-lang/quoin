//! Bytecode → Cranelift translation (docs/AOT_ARCH.md §4.2, v0.2).
//!
//! One JIT module per candidate *group* (one class body / `.meta` extension).
//! Members that fail translation are refused individually (a retry loop
//! rebuilds the module without them); anything not provably translatable
//! refuses — never guards, never silently diverges.
//!
//! Value model (v0.2): scalars live in SSA registers; every GC value lives in
//! the frame's *slot window* on `vm.stack` (rooted by construction) and is
//! carried as an absolute slot index — registers never hold object pointers,
//! so fuel-checkpoint suspends still need no rooting. Dynamic values
//! (`AV::Dyn`) are slot-resident; `BranchIfNotBool` narrows them to scalars
//! on the hot path. Sends leave the compiled world through the `outcall`
//! helper (`call_method` native re-entry: depth-guarded, suspension-safe,
//! thrown-value-transparent); only *scalar-pure* siblings (all-scalar
//! signatures whose bodies touch no slots, transitively) keep the direct
//! native-call fast path — fib-shaped recursion.
//!
//! Semantics are pinned to `devirt_ops`: wrapping i64 add/sub/mul, `/`/`%`
//! raising only on a zero divisor (`i64::MIN / -1` wraps — Cranelift's `sdiv`
//! would trap, hence the explicit −1 path), f64 ops that never raise, and
//! f64 `%` via an imported helper (Cranelift has no `frem`).

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block as CBlock, BlockArg, InstBuilder, MemFlagsData, Signature, StackSlotData,
    StackSlotKind, Type, Value as CVal, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::instruction::{Constant, Instruction, IntBinKind};
use crate::runtime::elem_tag::ElemTag;
use crate::symbol::{Symbol, self_symbol};

use super::helpers::{KIND_BOOL, KIND_DOUBLE, KIND_INT, KIND_NIL, KIND_SLOT};
use super::{
    AOT_MAX_CALL_DEPTH, AotCandidate, AotEntry, AotKind, AotParam, AotRawFn, AotRet, TAG_DEPTH,
    TAG_DIV_ZERO,
};

/// `%` on doubles: Rust's truncated remainder (what `devirt_ops::double_bin`
/// computes); Cranelift has no `frem`, so compiled code imports this.
unsafe extern "C" fn aot_fmod(a: f64, b: f64) -> f64 {
    a % b
}

/// Outcall arity cap: lane buffers are fixed-size native stack slots.
const MAX_OUTCALL_ARGS: usize = 8;

type SiblingMap = HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)>;

/// Refusal sentinel: the member used the slot window while marked
/// scalar-pure; `compile_all` demotes it and retries instead of refusing.
const PURITY_VIOLATION: &str = "__aot_purity__";

/// Compile every group; members are refused individually. Returns the
/// registered entries and refusals `(selector, reason)`.
pub(super) fn compile_all(
    cands: &[AotCandidate],
    siblings: &SiblingMap,
) -> (Vec<(u32, AotEntry)>, Vec<(String, String)>) {
    let mut groups: HashMap<u32, Vec<&AotCandidate>> = HashMap::new();
    for c in cands {
        groups.entry(c.group_id).or_default().push(c);
    }
    let mut compiled = Vec::new();
    let mut refused = Vec::new();
    for (_, mut active) in groups {
        // Per-member refusal: rebuild the module without the failed member and
        // retry (sends to it become outcalls). A purity violation demotes the
        // member from the direct-call set instead of refusing it. Groups are
        // small; worst-case quadratic compile cost is trivial.
        let mut demoted: HashSet<u32> = HashSet::new();
        loop {
            if active.is_empty() {
                break;
            }
            match compile_group(&active, siblings, &demoted) {
                Ok(mut entries) => {
                    compiled.append(&mut entries);
                    break;
                }
                Err((failed_tid, reason)) => {
                    if reason == PURITY_VIOLATION {
                        demoted.insert(failed_tid);
                        continue;
                    }
                    let i = active
                        .iter()
                        .position(|c| c.block.template_id == Some(failed_tid))
                        .expect("failed member is in the active set");
                    refused.push((active[i].selector.clone(), reason));
                    active.remove(i);
                }
            }
        }
    }
    (compiled, refused)
}

fn kind_type(k: AotKind) -> Type {
    match k {
        AotKind::Int => types::I64,
        AotKind::Double => types::F64,
        AotKind::Bool => types::I8,
    }
}

fn param_type(p: AotParam) -> Type {
    match p {
        AotParam::Scalar(k) => kind_type(k),
        AotParam::Obj => types::I64, // absolute slot index
    }
}

fn ret_type(r: AotRet) -> Type {
    match r {
        AotRet::Scalar(k) => kind_type(k),
        AotRet::Obj => types::I64, // absolute slot index
    }
}

/// The scalar-pure subset of a group: all-scalar signatures whose bodies stay
/// in the scalar instruction set and send only to other scalar-pure siblings.
/// These keep the direct native-call path; everything else outcalls.
fn scalar_pure_set(members: &[&AotCandidate], siblings: &SiblingMap) -> HashSet<u32> {
    let mut pure: HashSet<u32> = members
        .iter()
        .filter(|c| {
            c.params.iter().all(|p| matches!(p, AotParam::Scalar(_)))
                && matches!(c.ret, AotRet::Scalar(_))
        })
        .filter_map(|c| c.block.template_id)
        .collect();
    loop {
        let mut changed = false;
        for c in members {
            let Some(tid) = c.block.template_id else {
                continue;
            };
            if !pure.contains(&tid) {
                continue;
            }
            let ok = c.block.bytecode.0.iter().all(|inst| match inst {
                Instruction::Push(
                    Constant::Int(_) | Constant::Double(_) | Constant::Bool(_) | Constant::Nil,
                )
                | Instruction::LoadLocal(_)
                | Instruction::DefineLocal(_)
                | Instruction::StoreLocal(_)
                | Instruction::DefineLocalKeep(_)
                | Instruction::StoreLocalKeep(_)
                | Instruction::Dup
                | Instruction::Pop
                | Instruction::IntAdd
                | Instruction::IntSub
                | Instruction::IntMul
                | Instruction::IntDiv
                | Instruction::IntMod
                | Instruction::IntLt
                | Instruction::IntLe
                | Instruction::IntGt
                | Instruction::IntGe
                | Instruction::IntEq
                | Instruction::IntNe
                | Instruction::DoubleAdd
                | Instruction::DoubleSub
                | Instruction::DoubleMul
                | Instruction::DoubleDiv
                | Instruction::DoubleMod
                | Instruction::DoubleLt
                | Instruction::DoubleLe
                | Instruction::DoubleGt
                | Instruction::DoubleGe
                | Instruction::DoubleEq
                | Instruction::DoubleNe
                | Instruction::IntBinLL(..)
                | Instruction::IntBinLC(..)
                | Instruction::DoubleBinLL(..)
                | Instruction::DoubleBinLC(..)
                | Instruction::Jump(_)
                | Instruction::IfJump(_)
                | Instruction::ElseJump(_)
                | Instruction::Return
                | Instruction::BlockReturn => true,
                Instruction::Send(sel, _) | Instruction::SendLocal(_, sel, _) => siblings
                    .get(&(c.group_id, sel.as_str().to_string()))
                    .is_some_and(|(_, _, callee)| pure.contains(callee)),
                _ => false,
            });
            if !ok {
                pure.remove(&tid);
                changed = true;
            }
        }
        if !changed {
            return pure;
        }
    }
}

/// Imported helper function ids for one module.
struct Helpers {
    checkpoint: FuncId,
    fmod: FuncId,
    slot_set: FuncId,
    slot_peek: FuncId,
    list_new: FuncId,
    list_from: FuncId,
    list_push: FuncId,
    list_get: FuncId,
    list_len: FuncId,
    list_set: FuncId,
    string_const: FuncId,
    outcall: FuncId,
    narrow_error: FuncId,
    load_global: FuncId,
    tag_collection: FuncId,
    nil_mnu: FuncId,
}

fn declare_helpers(module: &mut JITModule, ptr: Type) -> Result<Helpers, String> {
    let sig = |params: &[Type], rets: &[Type]| {
        let mut s = module.make_signature();
        for &p in params {
            s.params.push(AbiParam::new(p));
        }
        for &r in rets {
            s.returns.push(AbiParam::new(r));
        }
        s
    };
    let i = types::I64;
    let d = |m: &mut JITModule, name: &str, s: &Signature| {
        m.declare_function(name, Linkage::Import, s)
            .map_err(|e| e.to_string())
    };
    let cp = sig(&[ptr, ptr], &[types::I8]);
    let fm = sig(&[types::F64, types::F64], &[types::F64]);
    let s2 = sig(&[ptr, ptr, i, i, i], &[types::I8]); // slot_set / list_push(list,kind,bits) / list_get(list,idx,out)
    let peek = sig(&[ptr, ptr, i, ptr], &[i]);
    let l0 = sig(&[ptr, ptr, i], &[types::I8]);
    let lf = sig(&[ptr, ptr, i, i, ptr, ptr], &[types::I8]);
    let ls = sig(&[ptr, ptr, i, i, i, i], &[types::I8]);
    let sc = sig(&[ptr, ptr, ptr, i, i], &[types::I8]);
    let oc = sig(&[ptr, ptr, i, i, ptr, i, ptr, ptr, i], &[types::I8]);
    let ne = sig(&[ptr, ptr, i, i], &[types::I8]);
    let lg = sig(&[ptr, ptr, ptr, i], &[types::I8]);
    let tc = sig(&[ptr, ptr, i, i], &[types::I8]);
    let nm = sig(&[ptr, ptr, i, i, ptr, i], &[types::I8]);
    let ll = sig(&[ptr, ptr, i], &[i]);
    Ok(Helpers {
        checkpoint: d(module, "qn_aot_checkpoint", &cp)?,
        fmod: d(module, "qn_aot_fmod", &fm)?,
        slot_set: d(module, "qn_aot_slot_set", &s2)?,
        slot_peek: d(module, "qn_aot_slot_peek", &peek)?,
        list_new: d(module, "qn_aot_list_new", &l0)?,
        list_from: d(module, "qn_aot_list_from", &lf)?,
        list_push: d(module, "qn_aot_list_push", &s2)?,
        list_get: d(module, "qn_aot_list_get", &s2)?,
        list_len: d(module, "qn_aot_list_len", &ll)?,
        list_set: d(module, "qn_aot_list_set", &ls)?,
        string_const: d(module, "qn_aot_string_const", &sc)?,
        outcall: d(module, "qn_aot_outcall", &oc)?,
        narrow_error: d(module, "qn_aot_narrow_error", &ne)?,
        load_global: d(module, "qn_aot_load_global", &lg)?,
        tag_collection: d(module, "qn_aot_tag_collection", &tc)?,
        nil_mnu: d(module, "qn_aot_nil_mnu", &nm)?,
    })
}

/// Compile one attempt at a group. `Err((template_id, reason))` names the
/// member to refuse before retrying.
fn compile_group(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
    demoted: &HashSet<u32>,
) -> Result<Vec<(u32, AotEntry)>, (u32, String)> {
    let fail = |tid: u32, e: String| (tid, e);
    let any_tid = members[0].block.template_id.unwrap_or(0);

    let mut flags = settings::builder();
    flags
        .set("opt_level", "speed")
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let isa = cranelift_native::builder()
        .map_err(|e| fail(any_tid, e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, addr) in super::helpers::symbols() {
        jb.symbol(name, addr);
    }
    jb.symbol("qn_aot_fmod", aot_fmod as *const u8);
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let helpers = declare_helpers(&mut module, ptr).map_err(|e| fail(any_tid, e))?;
    let mut pure = scalar_pure_set(members, siblings);
    for d in demoted {
        pure.remove(d);
    }

    // Declare every member's inner fn first (mutual recursion among the pure
    // set), then define bodies and trampolines.
    let mut inner_ids: HashMap<u32, FuncId> = HashMap::new();
    for m in members {
        let tid = m
            .block
            .template_id
            .ok_or_else(|| fail(any_tid, "candidate without template id".into()))?;
        let sig = inner_sig(&mut module, ptr, m);
        let fid = module
            .declare_function(&format!("t{tid}"), Linkage::Local, &sig)
            .map_err(|e| fail(tid, e.to_string()))?;
        inner_ids.insert(tid, fid);
    }

    let mut fb_ctx = FunctionBuilderContext::new();
    let mut tramp_ids: Vec<(u32, FuncId, &AotCandidate, u32, bool)> = Vec::new();

    for m in members {
        let tid = m.block.template_id.unwrap();
        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig(&mut module, ptr, m);
        let n_scratch;
        let needs_list_self;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let mut tr = Translator {
                module: &mut module,
                cand: m,
                siblings,
                inner_ids: &inner_ids,
                pure: &pure,
                helpers: &helpers,
                is_pure: pure.contains(&tid),
                next_scratch: 0,
                proofs: HashMap::new(),
                sym_consts: Vec::new(),
                name_consts: Vec::new(),
                needs_list_self: false,
            };
            tr.build_inner(&mut b).map_err(|e| fail(tid, e))?;
            n_scratch = tr.next_scratch;
            needs_list_self = tr.needs_list_self;
            b.seal_all_blocks();
            b.finalize();
        }
        let fid = inner_ids[&tid];
        module
            .define_function(fid, &mut ctx)
            .map_err(|e| fail(tid, format!("{e:?}\nIR:\n{}", ctx.func.display())))?;

        let mut tctx = module.make_context();
        tctx.func.signature = tramp_sig(&mut module, ptr);
        let tramp_id = module
            .declare_function(
                &format!("t{tid}_tramp"),
                Linkage::Local,
                &tctx.func.signature,
            )
            .map_err(|e| fail(tid, e.to_string()))?;
        {
            let mut b = FunctionBuilder::new(&mut tctx.func, &mut fb_ctx);
            build_trampoline(&mut module, &mut b, m, fid);
            b.seal_all_blocks();
            b.finalize();
        }
        module
            .define_function(tramp_id, &mut tctx)
            .map_err(|e| fail(tid, e.to_string()))?;
        tramp_ids.push((tid, tramp_id, m, n_scratch, needs_list_self));
    }

    module
        .finalize_definitions()
        .map_err(|e| fail(any_tid, e.to_string()))?;
    let mut out = Vec::new();
    for (tid, tramp_id, m, n_scratch, needs_list_self) in tramp_ids {
        let addr = module.get_finalized_function(tramp_id);
        let raw: AotRawFn = unsafe { std::mem::transmute(addr) };
        out.push((
            tid,
            AotEntry {
                raw,
                params: m.params.clone().into_boxed_slice(),
                ret: m.ret,
                n_scratch,
                needs_list_self,
            },
        ));
    }
    // The code must live for the process (fn pointers are registered
    // globally): leak the module, same append-only lifetime as the interner.
    std::mem::forget(module);
    Ok(out)
}

fn inner_sig(module: &mut JITModule, ptr: Type, m: &AotCandidate) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..4 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth
    }
    sig.params.push(AbiParam::new(types::I64)); // slot_base
    for &p in &m.params {
        sig.params.push(AbiParam::new(param_type(p)));
    }
    sig.returns.push(AbiParam::new(types::I8)); // tag
    sig.returns.push(AbiParam::new(ret_type(m.ret)));
    sig
}

fn tramp_sig(module: &mut JITModule, ptr: Type) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..4 {
        sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth
    }
    sig.params.push(AbiParam::new(types::I64)); // slot_base
    sig.params.push(AbiParam::new(ptr)); // args
    sig.params.push(AbiParam::new(ptr)); // ret
    sig.returns.push(AbiParam::new(types::I8));
    sig
}

fn build_trampoline(
    module: &mut JITModule,
    b: &mut FunctionBuilder,
    m: &AotCandidate,
    inner: FuncId,
) {
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let (vm, mc, fuel, depth, slot_base, args, ret) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
    let mut call_args = vec![vm, mc, fuel, depth, slot_base];
    for (i, &k) in m.params.iter().enumerate() {
        let off = (i * 8) as i32;
        let v = match k {
            AotParam::Scalar(AotKind::Int) | AotParam::Obj => {
                b.ins().load(types::I64, MemFlagsData::trusted(), args, off)
            }
            AotParam::Scalar(AotKind::Double) => {
                b.ins().load(types::F64, MemFlagsData::trusted(), args, off)
            }
            AotParam::Scalar(AotKind::Bool) => {
                let w = b.ins().load(types::I64, MemFlagsData::trusted(), args, off);
                b.ins().ireduce(types::I8, w)
            }
        };
        call_args.push(v);
    }
    let callee = module.declare_func_in_func(inner, b.func);
    let call = b.ins().call(callee, &call_args);
    let results = b.inst_results(call).to_vec();
    let (tag, val) = (results[0], results[1]);
    match m.ret {
        AotRet::Scalar(AotKind::Bool) => {
            let w = b.ins().uextend(types::I64, val);
            b.ins().store(MemFlagsData::trusted(), w, ret, 0);
        }
        _ => {
            b.ins().store(MemFlagsData::trusted(), val, ret, 0);
        }
    }
    b.ins().return_(&[tag]);
}

/// An abstract stack slot.
#[derive(Clone, Copy)]
enum AV {
    /// A scalar in SSA.
    C(CVal, AotKind),
    /// A slot-resident dynamic value: the SSA value is its *absolute index*
    /// into `vm.stack` (i64).
    Dyn(CVal),
    /// The method receiver (`self`): encoded as slot 0 when a value is needed.
    SelfRef,
    /// The `nil` a local-declaration prologue pushes; also a plain nil value
    /// (boxed into a slot when it must travel).
    Nil,
}

/// Block-boundary shape of one stack slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BKind {
    S(AotKind),
    Dyn,
}

fn bkind_type(k: BKind) -> Type {
    match k {
        BKind::S(k) => kind_type(k),
        BKind::Dyn => types::I64,
    }
}

/// A named local's storage.
#[derive(Clone, Copy)]
enum VarSlot {
    Scalar(Variable, AotKind),
    /// Scratch-slot number in the frame window, plus what the translator can
    /// PROVE about the value it holds (tag-backed; docs/GENERICS_ARCH.md §8).
    Obj(u32, Option<DynProof>),
}

/// A guarantee the translator carries for a slot-resident dynamic value —
/// only from sources the runtime enforces (a `TagCollection` it emitted, or
/// an element read from such a collection). Never from checker beliefs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DynProof {
    /// A collection whose element tag is enforced. In compiled code this is
    /// always a native List: `TagCollection` is only reachable after a list
    /// literal (`NewMap`/`NewSet` don't translate), and the only param source
    /// is a `List`-hinted Obj param (B1 seeding below).
    CollectionOf(ElemTag),
    /// An element read from such a collection: proven tag-or-nil.
    ElemOrNil(ElemTag),
    /// A native List with no (or unknown) element tag — a bare `List`-hinted
    /// Obj param (dispatch guarantees the class) or a fresh list literal.
    /// Enough for the fused `each:` guard (B1); mints no element proofs.
    NativeList,
}

struct Translator<'a> {
    module: &'a mut JITModule,
    cand: &'a AotCandidate,
    siblings: &'a SiblingMap,
    inner_ids: &'a HashMap<u32, FuncId>,
    pure: &'a HashSet<u32>,
    helpers: &'a Helpers,
    /// This member is in the scalar-pure set (direct-callable): it must not
    /// touch the slot window, because direct callers pass their own base.
    is_pure: bool,
    next_scratch: u32,
    /// Proofs for in-flight `AV::Dyn` values, keyed by their SSA index value.
    /// Values that cross a control-flow join (block params) drop their proofs
    /// — a sound degradation; the load-bearing flows (element read → inlined
    /// conditional) stay within one block, and locals carry proofs in
    /// `VarSlot::Obj` across blocks.
    proofs: HashMap<CVal, DynProof>,
    /// Leaked `Symbol`/`NamespacedName` boxes for outcall selectors and
    /// global references (live for the process, like the code).
    sym_consts: Vec<&'static Symbol>,
    name_consts: Vec<&'static crate::value::NamespacedName>,
    /// Set when a fused-`each:` guard on `self` compiled hot-path-only (B2):
    /// becomes the entry's `needs_list_self` precondition.
    needs_list_self: bool,
}

struct FnCtx {
    vm: CVal,
    mc: CVal,
    fuel: CVal,
    depth: CVal,
    slot_base: CVal,
    exit: CBlock,
    ret: AotRet,
    /// Native-stack lane buffers for helper calls (kinds, bits) and the
    /// peek out-parameter.
    kinds_buf: cranelift_codegen::ir::StackSlot,
    bits_buf: cranelift_codegen::ir::StackSlot,
    peek_out: cranelift_codegen::ir::StackSlot,
}

impl<'a> Translator<'a> {
    fn alloc_scratch(&mut self) -> Result<u32, String> {
        if self.is_pure {
            // Translation-verified purity: the syntactic pure-set scan missed a
            // slot use (e.g. a sibling-selector send on a non-self receiver).
            // The caller demotes this member from the pure set and retries.
            return Err(PURITY_VIOLATION.to_string());
        }
        let n_obj = self
            .cand
            .params
            .iter()
            .filter(|p| matches!(p, AotParam::Obj))
            .count() as u32;
        let k = 1 + n_obj + self.next_scratch; // 0 = receiver, then obj params
        self.next_scratch += 1;
        Ok(k)
    }

    fn abs_slot(&self, b: &mut FunctionBuilder, fx: &FnCtx, window_idx: u32) -> CVal {
        b.ins().iadd_imm(fx.slot_base, i64::from(window_idx))
    }

    /// Encode an AV as `(kind, bits)` lanes for a helper call. May allocate a
    /// scratch slot (boxing `Nil` never needs one; scalars pass by value).
    fn encode(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> (CVal, CVal) {
        match v {
            AV::C(cv, AotKind::Int) => (b.ins().iconst(types::I64, KIND_INT), cv),
            AV::C(cv, AotKind::Double) => {
                let bits = b.ins().bitcast(types::I64, MemFlagsData::new(), cv);
                (b.ins().iconst(types::I64, KIND_DOUBLE), bits)
            }
            AV::C(cv, AotKind::Bool) => {
                let w = b.ins().uextend(types::I64, cv);
                (b.ins().iconst(types::I64, KIND_BOOL), w)
            }
            AV::Dyn(idx) => (b.ins().iconst(types::I64, KIND_SLOT), idx),
            AV::SelfRef => {
                let idx = self.abs_slot(b, fx, 0);
                (b.ins().iconst(types::I64, KIND_SLOT), idx)
            }
            AV::Nil => {
                let z = b.ins().iconst(types::I64, 0);
                (b.ins().iconst(types::I64, KIND_NIL), z)
            }
        }
    }

    /// Call a helper whose return is a status tag; branch to exit on non-zero.
    fn tag_check(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, tag: CVal) {
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let bad_bl = b.create_block();
        let ok_bl = b.create_block();
        b.ins().brif(bad, bad_bl, &[], ok_bl, &[]);
        b.switch_to_block(bad_bl);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(ok_bl);
    }

    fn build_inner(&mut self, b: &mut FunctionBuilder) -> Result<(), String> {
        let insts = &self.cand.block.bytecode.0.clone();

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let (vm, mc, fuel, depth, slot_base) = (p[0], p[1], p[2], p[3], p[4]);

        // Named locals: params first. Object params occupy window slots 1..;
        // their SSA param value already carries the absolute index.
        let mut vars: HashMap<Symbol, VarSlot> = HashMap::new();
        let mut obj_param_avs: HashMap<Symbol, CVal> = HashMap::new();
        for (i, (&sym, &pk)) in self
            .cand
            .block
            .param_syms
            .iter()
            .zip(self.cand.params.iter())
            .enumerate()
        {
            match pk {
                AotParam::Scalar(k) => {
                    let var = b.declare_var(kind_type(k));
                    b.def_var(var, p[5 + i]);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                }
                AotParam::Obj => {
                    obj_param_avs.insert(sym, p[5 + i]);
                    // B1: a `List`-hinted param is a dispatch-GUARANTEED native
                    // List (List is sealed; the hint only matches the native
                    // class) — and a tag-required param is guaranteed tagged,
                    // since tag requirements gate dispatch too (G1). These
                    // proofs are what let a fused `each:` guard fall away.
                    if self.cand.block.param_types.get(i).map(String::as_str) == Some("List") {
                        let proof = match self.cand.block.param_elem_tags.get(i).copied().flatten()
                        {
                            Some(tag) => DynProof::CollectionOf(tag),
                            None => DynProof::NativeList,
                        };
                        self.proofs.insert(p[5 + i], proof);
                    }
                }
            }
        }

        let exit = b.create_block();
        b.append_block_param(exit, types::I8);
        b.append_block_param(exit, ret_type(self.cand.ret));
        let kinds_buf = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (MAX_OUTCALL_ARGS * 8) as u32,
            3,
        ));
        let bits_buf = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (MAX_OUTCALL_ARGS * 8) as u32,
            3,
        ));
        let peek_out =
            b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let fx = FnCtx {
            vm,
            mc,
            fuel,
            depth,
            slot_base,
            exit,
            ret: self.cand.ret,
            kinds_buf,
            bits_buf,
            peek_out,
        };

        // Prologue: depth guard, then fuel tick (checkpoint on exhaustion).
        let d0 = b.ins().load(types::I64, MemFlagsData::trusted(), depth, 0);
        let d1 = b.ins().iadd_imm(d0, 1);
        b.ins().store(MemFlagsData::trusted(), d1, depth, 0);
        let too_deep = b
            .ins()
            .icmp_imm(IntCC::SignedGreaterThan, d1, AOT_MAX_CALL_DEPTH);
        let deep_bl = b.create_block();
        let cont1 = b.create_block();
        b.ins().brif(too_deep, deep_bl, &[], cont1, &[]);
        b.switch_to_block(deep_bl);
        self.bail(b, &fx, TAG_DEPTH);
        b.switch_to_block(cont1);
        self.emit_fuel_tick_empty(b, &fx);

        // Fill the exit block now (parameters flow straight to the return).
        {
            let saved = b.current_block().unwrap();
            b.switch_to_block(exit);
            let ep = b.block_params(exit).to_vec();
            let d = b.ins().load(types::I64, MemFlagsData::trusted(), depth, 0);
            let d2 = b.ins().iadd_imm(d, -1);
            b.ins().store(MemFlagsData::trusted(), d2, depth, 0);
            b.ins().return_(&[ep[0], ep[1]]);
            b.switch_to_block(saved);
        }

        // Basic-block map over the bytecode: leaders = jump targets +
        // conditional fallthroughs.
        let mut leaders: Vec<usize> = Vec::new();
        for (ip, inst) in insts.iter().enumerate() {
            let off = match inst {
                Instruction::Jump(o)
                | Instruction::IfJump(o)
                | Instruction::ElseJump(o)
                | Instruction::BranchIfNotBool(o)
                | Instruction::BranchIfNotList(o) => *o,
                _ => continue,
            };
            let target = ip as isize + off;
            if target < 0 || target as usize >= insts.len() {
                return Err(format!("jump out of range at ip {ip}"));
            }
            leaders.push(target as usize);
            if !matches!(inst, Instruction::Jump(_)) {
                leaders.push(ip + 1);
            }
        }
        leaders.sort_unstable();
        leaders.dedup();

        let mut blocks: HashMap<usize, (CBlock, Vec<BKind>)> = HashMap::new();
        let mut done: HashSet<usize> = HashSet::new();
        let mut work: Vec<usize> = Vec::new();

        let mut cursor = Some((0usize, Vec::<AV>::new()));
        loop {
            let (start_ip, mut stack) = match cursor.take() {
                Some(s) => s,
                None => match work.pop() {
                    Some(ip) => {
                        if done.contains(&ip) {
                            continue;
                        }
                        done.insert(ip);
                        let (bl, kinds) = blocks[&ip].clone();
                        b.switch_to_block(bl);
                        let params = b.block_params(bl).to_vec();
                        let stack = params
                            .iter()
                            .zip(kinds.iter())
                            .map(|(&v, &k)| match k {
                                BKind::S(sk) => AV::C(v, sk),
                                BKind::Dyn => AV::Dyn(v),
                            })
                            .collect();
                        (ip, stack)
                    }
                    None => break,
                },
            };

            let mut ip = start_ip;
            'block: loop {
                if ip >= insts.len() {
                    return Err("fell off the end of bytecode".to_string());
                }
                if ip != start_ip && leaders.binary_search(&ip).is_ok() {
                    let nstack = self.norm_stack(b, &fx, &stack)?;
                    let (bl, _) = self.block_for(b, &mut blocks, &mut work, ip, &nstack)?;
                    let args = Self::stack_args(&nstack)?;
                    b.ins().jump(bl, &args);
                    break 'block;
                }
                match &insts[ip] {
                    Instruction::Push(c) => {
                        let av = self.const_av(b, &fx, c, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadLocal(sym) => {
                        let av = self.local_av(b, &fx, &vars, &obj_param_avs, *sym, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadGlobal(name) => {
                        let leaked: &'static crate::value::NamespacedName =
                            Box::leak(Box::new(name.clone()));
                        self.name_consts.push(leaked);
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        let np = b.ins().iconst(types::I64, leaked as *const _ as i64);
                        let f = self.func_ref(b, self.helpers.load_global);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, np, out_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::DefineLocal(sym) | Instruction::StoreLocal(sym) => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        if matches!((v, &insts[ip]), (AV::Nil, Instruction::DefineLocal(_)))
                            && !vars.contains_key(sym)
                            && !obj_param_avs.contains_key(sym)
                        {
                            // declaration prologue: type decided at first store
                        } else {
                            self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
                        }
                    }
                    Instruction::DefineLocalKeep(sym) | Instruction::StoreLocalKeep(sym) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
                    }
                    Instruction::Dup => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        stack.push(v);
                    }
                    Instruction::Pop => {
                        stack.pop().ok_or("stack underflow")?;
                    }
                    Instruction::IntAdd
                    | Instruction::IntSub
                    | Instruction::IntMul
                    | Instruction::IntDiv
                    | Instruction::IntMod
                    | Instruction::IntLt
                    | Instruction::IntLe
                    | Instruction::IntGt
                    | Instruction::IntGe
                    | Instruction::IntEq
                    | Instruction::IntNe => {
                        let kind = int_inst_kind(&insts[ip]);
                        let rb = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let ra = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let out = self.emit_int_bin(b, &fx, kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLL(a, bb, kind) => {
                        let ra =
                            self.local_scalar(b, &fx, &vars, &obj_param_avs, *a, AotKind::Int, ip)?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *bb,
                            AotKind::Int,
                            ip,
                        )?;
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLC(a, c, kind) => {
                        let ra =
                            self.local_scalar(b, &fx, &vars, &obj_param_avs, *a, AotKind::Int, ip)?;
                        let ci = c.as_int().ok_or("IntBinLC without int constant")?;
                        let rb = b.ins().iconst(types::I64, ci);
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::DoubleAdd
                    | Instruction::DoubleSub
                    | Instruction::DoubleMul
                    | Instruction::DoubleDiv
                    | Instruction::DoubleMod
                    | Instruction::DoubleLt
                    | Instruction::DoubleLe
                    | Instruction::DoubleGt
                    | Instruction::DoubleGe
                    | Instruction::DoubleEq
                    | Instruction::DoubleNe => {
                        let kind = double_inst_kind(&insts[ip]);
                        let rb = Self::pop_kind(&mut stack, AotKind::Double)?;
                        let ra = Self::pop_kind(&mut stack, AotKind::Double)?;
                        let out = self.emit_double_bin(b, kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::DoubleBinLL(a, bb, kind) => {
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *bb,
                            AotKind::Double,
                            ip,
                        )?;
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::DoubleBinLC(a, c, kind) => {
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let cd = match c {
                            Constant::Double(d) => *d,
                            Constant::Int(i) => *i as f64,
                            _ => return Err("DoubleBinLC without numeric constant".into()),
                        };
                        let rb = b.ins().f64const(cd);
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::NewList(n) => {
                        let n = *n;
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        if n == 0 {
                            let f = self.func_ref(b, self.helpers.list_new);
                            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx]);
                            let tag = b.inst_results(call)[0];
                            self.tag_check(b, &fx, tag);
                        } else {
                            if n > MAX_OUTCALL_ARGS {
                                return Err("list literal too long for v0.2".into());
                            }
                            let elems: Vec<AV> =
                                stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                            self.fill_lanes(b, &fx, &elems)?;
                            let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
                            let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
                            let nn = b.ins().iconst(types::I64, n as i64);
                            let f = self.func_ref(b, self.helpers.list_from);
                            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx, nn, ka, ba]);
                            let tag = b.inst_results(call)[0];
                            self.tag_check(b, &fx, tag);
                        }
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::ListPush => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListPush receiver")?;
                        let (k, bits) = self.encode(b, &fx, val);
                        let f = self.func_ref(b, self.helpers.list_push);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(recv_idx));
                    }
                    Instruction::BranchIfNotList(_) => {
                        // The fused-`each:` guard (B1, docs/BLOCK_AOT_ARCH.md §3). A
                        // PROVEN native-List receiver takes the hot path
                        // unconditionally — no branch is emitted, so nothing ever
                        // jumps to the cold path (the literal re-materialization +
                        // real send) and it is never translated: the same
                        // reachability discipline that deleted the sieve refusal
                        // (G3). An unproven receiver refuses the member — the
                        // interpreter's guarded loop still runs it.
                        let proven = match stack.last() {
                            Some(AV::Dyn(cv)) => matches!(
                                self.proofs.get(cv),
                                Some(DynProof::NativeList) | Some(DynProof::CollectionOf(_))
                            ),
                            // A guard on `self` (an open-owner combinator body, B2)
                            // becomes the ENTRY's precondition: `invoke` Bails to
                            // the interpreted body for non-List receivers, so the
                            // hot path is proven-by-entry.
                            Some(AV::SelfRef) => {
                                self.needs_list_self = true;
                                true
                            }
                            _ => false,
                        };
                        if !proven {
                            return Err(format!(
                                "fused each: on an unproven receiver at ip {ip} — a \
                                 `List`-annotated param, a fresh/checked list, or `self` \
                                 (entry-gated) compiles"
                            ));
                        }
                    }
                    Instruction::ListLen => {
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListLen receiver")?;
                        let f = self.func_ref(b, self.helpers.list_len);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx]);
                        let len = b.inst_results(call)[0];
                        stack.push(AV::C(len, AotKind::Int));
                    }
                    Instruction::ListGet => {
                        let idx = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_proof = match &recv {
                            AV::Dyn(ri) => self.proofs.get(ri).copied(),
                            _ => None,
                        };
                        let recv_idx = self.obj_index(b, &fx, recv, "ListGet receiver")?;
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        let f = self.func_ref(b, self.helpers.list_get);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, idx, out_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        if let Some(DynProof::CollectionOf(t)) = recv_proof {
                            // Tag-enforced source: the element is PROVEN t-or-nil.
                            self.proofs.insert(out_idx, DynProof::ElemOrNil(t));
                        }
                        stack.push(AV::Dyn(out_idx));
                    }
                    Instruction::TagCollection(tag) => {
                        // Verify + stamp the fresh literal on top of the stack
                        // (same helper contract as the interpreter arm), and
                        // record the PROOF — this is a tag the runtime enforces.
                        let AV::Dyn(idx) = *stack.last().ok_or("stack underflow")? else {
                            return Err("TagCollection on a non-slot value".into());
                        };
                        let Some(code) = tag.code() else {
                            return Err("user-class element tags in compiled literals are not \
                                 supported yet"
                                .into());
                        };
                        let code_v = b.ins().iconst(types::I64, code);
                        let f = self.func_ref(b, self.helpers.tag_collection);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, idx, code_v]);
                        let t = b.inst_results(call)[0];
                        self.tag_check(b, &fx, t);
                        self.proofs.insert(idx, DynProof::CollectionOf(*tag));
                    }
                    Instruction::ListSet => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let idx = Self::pop_kind(&mut stack, AotKind::Int)?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListSet receiver")?;
                        let (k, bits) = self.encode(b, &fx, val);
                        let f = self.func_ref(b, self.helpers.list_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, idx, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(recv_idx));
                    }
                    // Map devirt ops reissue as outcalls: the native methods do
                    // the same lookup, so behavior is identical, and it keeps
                    // string keys out of the compiled ABI.
                    Instruction::MapGet => {
                        let key = stack.pop().ok_or("stack underflow")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_outcall(b, &fx, recv, "at:", &[key])?;
                        stack.push(out);
                    }
                    Instruction::MapSet => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let key = stack.pop().ok_or("stack underflow")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_outcall(b, &fx, recv, "at:put:", &[key, val])?;
                        stack.push(out);
                    }
                    Instruction::Jump(off) => {
                        let target = (ip as isize + off) as usize;
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        if target <= ip {
                            nstack = self.emit_fuel_tick(b, &fx, &nstack)?;
                        }
                        let (bl, _) = self.block_for(b, &mut blocks, &mut work, target, &nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        b.ins().jump(bl, &args);
                        break 'block;
                    }
                    Instruction::IfJump(off) | Instruction::ElseJump(off) => {
                        // `is_truthy` lowering: everything but `false` and `nil`
                        // is truthy (the interpreter's exact contract). Statically
                        // known scalars fold; dynamic values peek their kind.
                        let cond = match stack.pop().ok_or("stack underflow")? {
                            AV::C(v, AotKind::Bool) => v,
                            AV::C(..) | AV::SelfRef => b.ins().iconst(types::I8, 1),
                            AV::Nil => b.ins().iconst(types::I8, 0),
                            AV::Dyn(idx) => {
                                let f = self.func_ref(b, self.helpers.slot_peek);
                                let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
                                let kind = b.inst_results(call)[0];
                                let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
                                let is_nil = b.ins().icmp_imm(IntCC::Equal, kind, KIND_NIL);
                                let is_bool = b.ins().icmp_imm(IntCC::Equal, kind, KIND_BOOL);
                                let is_zero = b.ins().icmp_imm(IntCC::Equal, bits, 0);
                                let is_false = b.ins().band(is_bool, is_zero);
                                let not_truthy = b.ins().bor(is_nil, is_false);
                                b.ins().icmp_imm(IntCC::Equal, not_truthy, 0)
                            }
                        };
                        let target = (ip as isize + off) as usize;
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        if target <= ip {
                            // Conditional back-edge: tick before the branch
                            // (loops must stay preemptible and cancellable).
                            nstack = self.emit_fuel_tick(b, &fx, &nstack)?;
                        }
                        let (tbl, _) =
                            self.block_for(b, &mut blocks, &mut work, target, &nstack)?;
                        let (fbl, _) =
                            self.block_for(b, &mut blocks, &mut work, ip + 1, &nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        if matches!(insts[ip], Instruction::IfJump(_)) {
                            b.ins().brif(cond, tbl, &args, fbl, &args);
                        } else {
                            b.ins().brif(cond, fbl, &args, tbl, &args);
                        }
                        break 'block;
                    }
                    Instruction::BranchIfNotBool(off) => {
                        let target = (ip as isize + off) as usize;
                        match *stack.last().ok_or("stack underflow")? {
                            AV::C(_, AotKind::Bool) => {} // statically Bool: fall through
                            AV::C(..) | AV::Nil | AV::SelfRef => {
                                // Statically not a Bool: always the cold path.
                                let nstack = self.norm_stack(b, &fx, &stack)?;
                                let (bl, _) =
                                    self.block_for(b, &mut blocks, &mut work, target, &nstack)?;
                                let args = Self::stack_args(&nstack)?;
                                b.ins().jump(bl, &args);
                                break 'block;
                            }
                            AV::Dyn(idx) => {
                                let f = self.func_ref(b, self.helpers.slot_peek);
                                let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
                                let kind = b.inst_results(call)[0];
                                let is_bool = b.ins().icmp_imm(IntCC::Equal, kind, KIND_BOOL);
                                let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
                                let as_bool = b.ins().icmp_imm(IntCC::NotEqual, bits, 0);
                                let mut hot_stack = self.norm_stack(b, &fx, &stack)?;
                                hot_stack.pop();
                                hot_stack.push(AV::C(as_bool, AotKind::Bool));
                                let (hot, _) =
                                    self.block_for(b, &mut blocks, &mut work, ip + 1, &hot_stack)?;
                                let hot_args = Self::stack_args(&hot_stack)?;
                                if self.proofs.get(&idx).copied()
                                    == Some(DynProof::ElemOrNil(ElemTag::Bool))
                                {
                                    // PROVEN Boolean-or-nil (a checked List(Boolean)
                                    // element read): the only non-Bool is nil, whose
                                    // sealed class has no `if:` — raise the exact
                                    // interpreter MNU instead of jumping to the cold
                                    // path. Nothing jumps there, so its capturing
                                    // block re-materialization is never translated:
                                    // this deletes the sieve refusal
                                    // (GENERICS_ARCH.md §7, AOT_ARCH.md §9).
                                    let (sel, argc) = Self::cold_send(insts, target);
                                    let mnu_bl = b.create_block();
                                    b.ins().brif(is_bool, hot, &hot_args, mnu_bl, &[]);
                                    b.switch_to_block(mnu_bl);
                                    let sel_ptr = b.ins().iconst(
                                        types::I64,
                                        Box::leak(Box::new(sel)) as *const Symbol as i64,
                                    );
                                    let argc_v = b.ins().iconst(types::I64, argc);
                                    let nf = self.func_ref(b, self.helpers.nil_mnu);
                                    let call = b
                                        .ins()
                                        .call(nf, &[fx.vm, fx.mc, kind, bits, sel_ptr, argc_v]);
                                    let etag = b.inst_results(call)[0];
                                    let zero = self.zero_of(b, fx.ret);
                                    b.ins().jump(fx.exit, &[etag.into(), zero.into()]);
                                    break 'block;
                                }
                                // Unproven: Bool → hot; anything else → the cold
                                // path's real send.
                                let nstack = self.norm_stack(b, &fx, &stack)?;
                                let (cold, _) =
                                    self.block_for(b, &mut blocks, &mut work, target, &nstack)?;
                                let cold_args = Self::stack_args(&nstack)?;
                                b.ins().brif(is_bool, hot, &hot_args, cold, &cold_args);
                                break 'block;
                            }
                        }
                    }
                    // Fused sends push their folded operand(s), then share the
                    // generic path: `exec_send` semantics pop n args, then the
                    // receiver (the fused operand is the receiver only for
                    // 0-arg sends — for n≥1 it is the LAST ARGUMENT).
                    Instruction::Send(sel, n)
                    | Instruction::SendLocal(_, sel, n)
                    | Instruction::SendConst(_, sel, n)
                    | Instruction::SendLocalLocal(_, _, sel, n)
                    | Instruction::SendLocalConst(_, _, sel, n) => {
                        let (sel, n) = (*sel, *n);
                        match &insts[ip] {
                            Instruction::SendLocal(a, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendConst(c, ..) => {
                                let v = self.const_av(b, &fx, c, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalLocal(a, bb, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *bb, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalConst(a, c, ..) => {
                                let v = self.local_av(b, &fx, &vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v = self.const_av(b, &fx, c, ip)?;
                                stack.push(v);
                            }
                            _ => {}
                        }
                        let args: Vec<AV> =
                            stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_send(b, &fx, recv, sel, &args, ip)?;
                        stack.push(out);
                    }
                    // Within one method's bytecode, `MethodReturn` (`^^`) always
                    // targets THIS method's frame — a real nested block is a separate
                    // `StaticBlock` never translated inline, and a fused-`each:` body
                    // (B1) is spliced into this very frame. So all three return forms
                    // are the compiled function's return.
                    Instruction::Return | Instruction::BlockReturn | Instruction::MethodReturn => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.emit_return(b, &fx, v)?;
                        break 'block;
                    }
                    other => {
                        return Err(format!("unsupported instruction at ip {ip}: {other:?}"));
                    }
                }
                ip += 1;
            }
        }
        Ok(())
    }

    fn func_ref(&mut self, b: &mut FunctionBuilder, id: FuncId) -> cranelift_codegen::ir::FuncRef {
        self.module.declare_func_in_func(id, b.func)
    }

    /// The absolute slot index of an object-shaped AV (helpers take indices).
    fn obj_index(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        v: AV,
        what: &str,
    ) -> Result<CVal, String> {
        match v {
            AV::Dyn(idx) => Ok(idx),
            AV::SelfRef => Ok(self.abs_slot(b, fx, 0)),
            _ => Err(format!("{what} is not slot-resident")),
        }
    }

    fn const_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        c: &Constant,
        ip: usize,
    ) -> Result<AV, String> {
        Ok(match c {
            Constant::Int(i) => AV::C(b.ins().iconst(types::I64, *i), AotKind::Int),
            Constant::Double(d) => AV::C(b.ins().f64const(*d), AotKind::Double),
            Constant::Bool(x) => AV::C(b.ins().iconst(types::I8, *x as i64), AotKind::Bool),
            Constant::Nil => AV::Nil,
            Constant::String(s) => {
                // Leak once per site; the code referencing it is process-lived.
                let leaked: &'static str = Box::leak(s.clone().into_boxed_str());
                let out = self.alloc_scratch()?;
                let out_idx = self.abs_slot(b, fx, out);
                let ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
                let len = b.ins().iconst(types::I64, leaked.len() as i64);
                let f = self.func_ref(b, self.helpers.string_const);
                let call = b.ins().call(f, &[fx.vm, fx.mc, ptr, len, out_idx]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                AV::Dyn(out_idx)
            }
            Constant::Block(_) => {
                return Err(format!(
                    "capturing block materialization at ip {ip} (an inlined-if cold \
                     path or a block argument) — not compilable until checked \
                     generics remove the dynamic branch (AOT_ARCH.md §9)"
                ));
            }
            _ => return Err(format!("unsupported constant at ip {ip}")),
        })
    }

    fn local_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        ip: usize,
    ) -> Result<AV, String> {
        if sym == self_symbol() {
            return Ok(AV::SelfRef);
        }
        if let Some(&idx) = obj_params.get(&sym) {
            return Ok(AV::Dyn(idx));
        }
        match vars.get(&sym) {
            Some(&VarSlot::Scalar(var, k)) => Ok(AV::C(b.use_var(var), k)),
            Some(&VarSlot::Obj(slot, proof)) => {
                let idx = self.abs_slot(b, fx, slot);
                if let Some(pr) = proof {
                    self.proofs.insert(idx, pr);
                }
                Ok(AV::Dyn(idx))
            }
            None => Err(format!(
                "read of unknown/uninitialized local '{}' at ip {ip}",
                sym.as_str()
            )),
        }
    }

    fn local_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        want: AotKind,
        ip: usize,
    ) -> Result<CVal, String> {
        match self.local_av(b, fx, vars, obj_params, sym, ip)? {
            AV::C(v, k) if k == want => Ok(v),
            _ => Err(format!(
                "local '{}' is not a {want:?} at ip {ip}",
                sym.as_str()
            )),
        }
    }

    fn store_local(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        v: AV,
    ) -> Result<(), String> {
        if obj_params.contains_key(&sym) || sym == self_symbol() {
            return Err(format!("store to parameter/self '{}'", sym.as_str()));
        }
        match v {
            AV::C(cv, k) => match vars.get(&sym) {
                Some(&VarSlot::Scalar(var, vk)) => {
                    if vk != k {
                        return Err(format!("local '{}' changes kind", sym.as_str()));
                    }
                    b.def_var(var, cv);
                    Ok(())
                }
                Some(VarSlot::Obj(..)) => Err(format!("local '{}' changes kind", sym.as_str())),
                None => {
                    let var = b.declare_var(kind_type(k));
                    b.def_var(var, cv);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                    Ok(())
                }
            },
            AV::Dyn(idx) if matches!(vars.get(&sym), Some(VarSlot::Scalar(..))) => {
                // Accumulator pattern: `total = total + (dynamic)` — narrow the
                // dynamic value back into the scalar local, checked.
                let Some(&VarSlot::Scalar(var, k)) = vars.get(&sym) else {
                    unreachable!()
                };
                let val = self.narrow_to_scalar(b, fx, idx, k);
                b.def_var(var, val);
                return Ok(());
            }
            AV::Dyn(_) | AV::Nil | AV::SelfRef => {
                let vproof = match &v {
                    AV::Dyn(idx) => self.proofs.get(idx).copied(),
                    _ => None,
                };
                let slot = match vars.get(&sym) {
                    Some(&VarSlot::Obj(slot, _)) => {
                        // Reassignment: the slot's proof becomes the new value's.
                        vars.insert(sym, VarSlot::Obj(slot, vproof));
                        slot
                    }
                    Some(VarSlot::Scalar(..)) => {
                        return Err(format!("local '{}' changes kind", sym.as_str()));
                    }
                    None => {
                        let slot = self.alloc_scratch()?;
                        vars.insert(sym, VarSlot::Obj(slot, vproof));
                        slot
                    }
                };
                let dst = self.abs_slot(b, fx, slot);
                let (k, bits) = self.encode(b, fx, v);
                let f = self.func_ref(b, self.helpers.slot_set);
                let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                Ok(())
            }
        }
    }

    /// The selector + block-arg count of the cold path's re-materialized send
    /// (the real `if:`/`if:else:` an inlined conditional falls back to) — what
    /// the proven-nil MNU stub must name to match the interpreter exactly.
    fn cold_send(insts: &[Instruction], target: usize) -> (Symbol, i64) {
        for inst in insts.iter().skip(target).take(8) {
            match inst {
                Instruction::Send(sel, n)
                | Instruction::SendLocal(_, sel, n)
                | Instruction::SendConst(_, sel, n)
                | Instruction::SendLocalLocal(_, _, sel, n)
                | Instruction::SendLocalConst(_, _, sel, n) => {
                    return (*sel, *n as i64);
                }
                _ => {}
            }
        }
        (Symbol::intern("if:"), 1)
    }

    /// Fill the lane buffers with encoded AVs.
    fn fill_lanes(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vals: &[AV],
    ) -> Result<(), String> {
        if vals.len() > MAX_OUTCALL_ARGS {
            return Err("too many arguments for the compiled ABI".into());
        }
        for (i, &v) in vals.iter().enumerate() {
            let (k, bits) = self.encode(b, fx, v);
            let off = (i * 8) as i32;
            b.ins().stack_store(k, fx.kinds_buf, off);
            b.ins().stack_store(bits, fx.bits_buf, off);
        }
        Ok(())
    }

    /// A dynamic send: the general exit from the compiled world.
    fn emit_outcall(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        selector: &str,
        args: &[AV],
    ) -> Result<AV, String> {
        let (rk, rb) = self.encode(b, fx, recv);
        self.fill_lanes(b, fx, args)?;
        let sym: &'static Symbol = Box::leak(Box::new(Symbol::intern(selector)));
        self.sym_consts.push(sym);
        let sel = b.ins().iconst(types::I64, sym as *const Symbol as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
        let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
        let argc = b.ins().iconst(types::I64, args.len() as i64);
        let f = self.func_ref(b, self.helpers.outcall);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, rk, rb, sel, argc, ka, ba, out_idx]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// A send site: direct native call when the callee is a scalar-pure
    /// sibling and the receiver is `self`; otherwise an outcall.
    fn emit_send(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        sel: Symbol,
        args: &[AV],
        ip: usize,
    ) -> Result<AV, String> {
        let key = (self.cand.group_id, sel.as_str().to_string());
        // An OPEN owner (B2) never emits direct calls: the frozen-callee-set
        // assumption behind them is exactly what a reopen would violate. Every
        // send goes through the outcall (dispatch-equivalent) seam instead.
        if !self.cand.open_owner
            && matches!(recv, AV::SelfRef)
            && let Some((psig, pret, callee_tid)) = self.siblings.get(&key)
            && self.pure.contains(callee_tid)
            && psig.len() == args.len()
        {
            // Direct call. Scalar-pure callee: args must be exact scalars.
            let mut ok = true;
            let mut call_args = vec![fx.vm, fx.mc, fx.fuel, fx.depth, fx.slot_base];
            for (a, pk) in args.iter().zip(psig.iter()) {
                match (a, pk) {
                    (AV::C(v, k), AotParam::Scalar(want)) if k == want => call_args.push(*v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let callee_fid = self.inner_ids[callee_tid];
                let callee = self.func_ref(b, callee_fid);
                let call = b.ins().call(callee, &call_args);
                let res = b.inst_results(call).to_vec();
                let (tag, val) = (res[0], res[1]);
                self.tag_check(b, fx, tag);
                let AotRet::Scalar(rk) = pret else {
                    return Err(format!("pure sibling with non-scalar ret at ip {ip}"));
                };
                return Ok(AV::C(val, *rk));
            }
        }
        self.emit_outcall(b, fx, recv, sel.as_str(), args)
    }

    /// Checked narrow of a slot-resident value to a scalar kind: peek the
    /// kind, extract on match, raise a clear TypeError otherwise (the one
    /// deliberate divergence — a wrong dynamic type surfaces at the annotation
    /// or accumulator that expected the scalar, instead of corrupting later).
    fn narrow_to_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        idx: CVal,
        want: AotKind,
    ) -> CVal {
        let f = self.func_ref(b, self.helpers.slot_peek);
        let oa = b.ins().stack_addr(types::I64, fx.peek_out, 0);
        let call = b.ins().call(f, &[fx.vm, fx.mc, idx, oa]);
        let kind = b.inst_results(call)[0];
        let want_code = match want {
            AotKind::Int => KIND_INT,
            AotKind::Double => KIND_DOUBLE,
            AotKind::Bool => KIND_BOOL,
        };
        let is_ok = b.ins().icmp_imm(IntCC::Equal, kind, want_code);
        let ok_bl = b.create_block();
        let err_bl = b.create_block();
        b.ins().brif(is_ok, ok_bl, &[], err_bl, &[]);
        b.switch_to_block(err_bl);
        let wc = b.ins().iconst(types::I64, want_code);
        let nf = self.func_ref(b, self.helpers.narrow_error);
        let ecall = b.ins().call(nf, &[fx.vm, fx.mc, wc, kind]);
        let etag = b.inst_results(ecall)[0];
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[etag.into(), zero.into()]);
        b.switch_to_block(ok_bl);
        let bits = b.ins().stack_load(types::I64, fx.peek_out, 0);
        match want {
            AotKind::Int => bits,
            AotKind::Double => b.ins().bitcast(types::F64, MemFlagsData::new(), bits),
            AotKind::Bool => b.ins().icmp_imm(IntCC::NotEqual, bits, 0),
        }
    }

    /// Return: narrow to the declared shape. A `Dyn` flowing into a scalar
    /// return is runtime-checked (the one deliberate divergence: a lying
    /// annotation raises a clear TypeError instead of corrupting callers).
    fn emit_return(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> Result<(), String> {
        let tag0 = b.ins().iconst(types::I8, 0);
        match (fx.ret, v) {
            (AotRet::Scalar(want), AV::C(cv, k)) if k == want => {
                b.ins().jump(fx.exit, &[tag0.into(), cv.into()]);
            }
            (AotRet::Scalar(want), AV::Dyn(idx)) => {
                let val = self.narrow_to_scalar(b, fx, idx, want);
                b.ins().jump(fx.exit, &[tag0.into(), val.into()]);
            }
            (AotRet::Scalar(_), _) => {
                return Err("return value does not match the declared scalar type".into());
            }
            (AotRet::Obj, v) => {
                let idx = match v {
                    AV::Dyn(idx) => idx,
                    AV::SelfRef => self.abs_slot(b, fx, 0),
                    other => {
                        // Box a scalar/nil into a scratch slot (a lying `^List`
                        // etc. returns the honest value, as the interpreter's
                        // trusted-return contract does).
                        let out = self.alloc_scratch()?;
                        let dst = self.abs_slot(b, fx, out);
                        let (k, bits) = self.encode(b, fx, other);
                        let f = self.func_ref(b, self.helpers.slot_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, fx, tag);
                        dst
                    }
                };
                b.ins().jump(fx.exit, &[tag0.into(), idx.into()]);
            }
        }
        Ok(())
    }

    fn pop_kind(stack: &mut Vec<AV>, want: AotKind) -> Result<CVal, String> {
        match stack.pop() {
            Some(AV::C(v, k)) if k == want => Ok(v),
            Some(AV::C(_, k)) => Err(format!("operand kind {k:?}, wanted {want:?}")),
            Some(_) => Err("non-scalar operand where a scalar was proven".to_string()),
            None => Err("stack underflow".to_string()),
        }
    }

    /// Box `Nil`/`SelfRef` stack entries into slots so they can cross a block
    /// boundary as jump arguments (a statement-position inlined `if:` joins an
    /// arm value with the nil of the not-taken path). Scalars and slot values
    /// pass through untouched.
    fn norm_stack(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, String> {
        let mut out = Vec::with_capacity(stack.len());
        for v in stack {
            match v {
                AV::C(..) | AV::Dyn(_) => out.push(*v),
                AV::SelfRef => out.push(AV::Dyn(self.abs_slot(b, fx, 0))),
                AV::Nil => {
                    let slot = self.alloc_scratch()?;
                    let dst = self.abs_slot(b, fx, slot);
                    let (k, bits) = self.encode(b, fx, AV::Nil);
                    let f = self.func_ref(b, self.helpers.slot_set);
                    let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
                    let tag = b.inst_results(call)[0];
                    self.tag_check(b, fx, tag);
                    out.push(AV::Dyn(dst));
                }
            }
        }
        Ok(out)
    }

    fn stack_args(stack: &[AV]) -> Result<Vec<BlockArg>, String> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(cv, _) => Ok((*cv).into()),
                AV::Dyn(idx) => Ok((*idx).into()),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect()
    }

    fn stack_bkinds(stack: &[AV]) -> Result<Vec<BKind>, String> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(_, k) => Ok(BKind::S(*k)),
                AV::Dyn(_) => Ok(BKind::Dyn),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect()
    }

    fn block_for(
        &mut self,
        b: &mut FunctionBuilder,
        blocks: &mut HashMap<usize, (CBlock, Vec<BKind>)>,
        work: &mut Vec<usize>,
        ip: usize,
        stack: &[AV],
    ) -> Result<(CBlock, Vec<BKind>), String> {
        let kinds = Self::stack_bkinds(stack)?;
        if let Some((bl, expect)) = blocks.get(&ip) {
            if *expect != kinds {
                return Err(format!("stack shape mismatch at merge ip {ip}"));
            }
            return Ok((*bl, expect.clone()));
        }
        let bl = b.create_block();
        for &k in &kinds {
            b.append_block_param(bl, bkind_type(k));
        }
        blocks.insert(ip, (bl, kinds.clone()));
        work.push(ip);
        Ok((bl, kinds))
    }

    fn zero_of(&self, b: &mut FunctionBuilder, r: AotRet) -> CVal {
        match r {
            AotRet::Scalar(AotKind::Int) | AotRet::Obj => b.ins().iconst(types::I64, 0),
            AotRet::Scalar(AotKind::Double) => b.ins().f64const(0.0),
            AotRet::Scalar(AotKind::Bool) => b.ins().iconst(types::I8, 0),
        }
    }

    fn bail(&self, b: &mut FunctionBuilder, fx: &FnCtx, tag: u8) {
        let t = b.ins().iconst(types::I8, tag as i64);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[t.into(), zero.into()]);
    }

    /// Fuel decrement + (rarely) checkpoint, carrying the live abstract stack
    /// through the checkpoint's control flow as block params. Emitted in
    /// every prologue (covers recursion) and at loop back-edges (covers
    /// loops) — the two shapes that must stay preemptible and cancellable.
    /// `Dyn` values pass through as slot indices (their contents are rooted
    /// in the window, so suspending needs no extra work).
    fn emit_fuel_tick(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, String> {
        let keep = Self::stack_args(stack)?;
        let kinds = Self::stack_bkinds(stack)?;
        let f0 = b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), fx.fuel, 0);
        let f1 = b.ins().iadd_imm(f0, -1);
        b.ins().store(MemFlagsData::trusted(), f1, fx.fuel, 0);
        let spent = b.ins().icmp_imm(IntCC::SignedLessThanOrEqual, f1, 0);
        let cp_bl = b.create_block();
        let cont = b.create_block();
        for &k in &kinds {
            b.append_block_param(cont, bkind_type(k));
        }
        b.ins().brif(spent, cp_bl, &[], cont, &keep);
        b.switch_to_block(cp_bl);
        let cp = self.func_ref(b, self.helpers.checkpoint);
        let call = b.ins().call(cp, &[fx.vm, fx.fuel]);
        let tag = b.inst_results(call)[0];
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let cp_bad = b.create_block();
        b.ins().brif(bad, cp_bad, &[], cont, &keep);
        b.switch_to_block(cp_bad);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(cont);
        let params = b.block_params(cont).to_vec();
        Ok(params
            .iter()
            .zip(kinds.iter())
            .map(|(&v, &k)| match k {
                BKind::S(sk) => AV::C(v, sk),
                BKind::Dyn => AV::Dyn(v),
            })
            .collect())
    }

    fn emit_fuel_tick_empty(&mut self, b: &mut FunctionBuilder, fx: &FnCtx) {
        self.emit_fuel_tick(b, fx, &[])
            .expect("empty-stack tick cannot fail");
    }

    /// Integer ops with `devirt_ops::int_bin` semantics.
    fn emit_int_bin(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        kind: IntBinKind,
        a: CVal,
        rb: CVal,
    ) -> Result<AV, String> {
        use IntBinKind::*;
        let out = match kind {
            Add => AV::C(b.ins().iadd(a, rb), AotKind::Int),
            Sub => AV::C(b.ins().isub(a, rb), AotKind::Int),
            Mul => AV::C(b.ins().imul(a, rb), AotKind::Int),
            Div | Mod => {
                let is_zero = b.ins().icmp_imm(IntCC::Equal, rb, 0);
                let zero_bl = b.create_block();
                let cont = b.create_block();
                b.ins().brif(is_zero, zero_bl, &[], cont, &[]);
                b.switch_to_block(zero_bl);
                self.bail(b, fx, TAG_DIV_ZERO);
                b.switch_to_block(cont);
                let is_m1 = b.ins().icmp_imm(IntCC::Equal, rb, -1);
                let m1_bl = b.create_block();
                let norm_bl = b.create_block();
                let join = b.create_block();
                b.append_block_param(join, types::I64);
                b.ins().brif(is_m1, m1_bl, &[], norm_bl, &[]);
                b.switch_to_block(m1_bl);
                let m1v = if matches!(kind, Div) {
                    b.ins().ineg(a)
                } else {
                    b.ins().iconst(types::I64, 0)
                };
                b.ins().jump(join, &[m1v.into()]);
                b.switch_to_block(norm_bl);
                let nv = if matches!(kind, Div) {
                    b.ins().sdiv(a, rb)
                } else {
                    b.ins().srem(a, rb)
                };
                b.ins().jump(join, &[nv.into()]);
                b.switch_to_block(join);
                AV::C(b.block_params(join)[0], AotKind::Int)
            }
            Lt => AV::C(b.ins().icmp(IntCC::SignedLessThan, a, rb), AotKind::Bool),
            Le => AV::C(
                b.ins().icmp(IntCC::SignedLessThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Gt => AV::C(b.ins().icmp(IntCC::SignedGreaterThan, a, rb), AotKind::Bool),
            Ge => AV::C(
                b.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Eq => AV::C(b.ins().icmp(IntCC::Equal, a, rb), AotKind::Bool),
            Ne => AV::C(b.ins().icmp(IntCC::NotEqual, a, rb), AotKind::Bool),
        };
        Ok(out)
    }

    /// f64 ops with `devirt_ops::double_bin` semantics.
    fn emit_double_bin(
        &mut self,
        b: &mut FunctionBuilder,
        kind: IntBinKind,
        a: CVal,
        rb: CVal,
    ) -> AV {
        use IntBinKind::*;
        match kind {
            Add => AV::C(b.ins().fadd(a, rb), AotKind::Double),
            Sub => AV::C(b.ins().fsub(a, rb), AotKind::Double),
            Mul => AV::C(b.ins().fmul(a, rb), AotKind::Double),
            Div => AV::C(b.ins().fdiv(a, rb), AotKind::Double),
            Mod => {
                let f = self.func_ref(b, self.helpers.fmod);
                let call = b.ins().call(f, &[a, rb]);
                AV::C(b.inst_results(call)[0], AotKind::Double)
            }
            Lt => AV::C(b.ins().fcmp(FloatCC::LessThan, a, rb), AotKind::Bool),
            Le => AV::C(b.ins().fcmp(FloatCC::LessThanOrEqual, a, rb), AotKind::Bool),
            Gt => AV::C(b.ins().fcmp(FloatCC::GreaterThan, a, rb), AotKind::Bool),
            Ge => AV::C(
                b.ins().fcmp(FloatCC::GreaterThanOrEqual, a, rb),
                AotKind::Bool,
            ),
            Eq => AV::C(b.ins().fcmp(FloatCC::Equal, a, rb), AotKind::Bool),
            Ne => AV::C(b.ins().fcmp(FloatCC::NotEqual, a, rb), AotKind::Bool),
        }
    }
}

fn int_inst_kind(i: &Instruction) -> IntBinKind {
    match i {
        Instruction::IntAdd => IntBinKind::Add,
        Instruction::IntSub => IntBinKind::Sub,
        Instruction::IntMul => IntBinKind::Mul,
        Instruction::IntDiv => IntBinKind::Div,
        Instruction::IntMod => IntBinKind::Mod,
        Instruction::IntLt => IntBinKind::Lt,
        Instruction::IntLe => IntBinKind::Le,
        Instruction::IntGt => IntBinKind::Gt,
        Instruction::IntGe => IntBinKind::Ge,
        Instruction::IntEq => IntBinKind::Eq,
        Instruction::IntNe => IntBinKind::Ne,
        _ => unreachable!(),
    }
}

fn double_inst_kind(i: &Instruction) -> IntBinKind {
    match i {
        Instruction::DoubleAdd => IntBinKind::Add,
        Instruction::DoubleSub => IntBinKind::Sub,
        Instruction::DoubleMul => IntBinKind::Mul,
        Instruction::DoubleDiv => IntBinKind::Div,
        Instruction::DoubleMod => IntBinKind::Mod,
        Instruction::DoubleLt => IntBinKind::Lt,
        Instruction::DoubleLe => IntBinKind::Le,
        Instruction::DoubleGt => IntBinKind::Gt,
        Instruction::DoubleGe => IntBinKind::Ge,
        Instruction::DoubleEq => IntBinKind::Eq,
        Instruction::DoubleNe => IntBinKind::Ne,
        _ => unreachable!(),
    }
}
