//! Bytecode → Cranelift translation for the v0 subset (docs/AOT_ARCH.md §4.2).
//!
//! One JIT module per candidate *group* (one class body / `.meta` extension),
//! all-or-nothing: self-calls are group-internal direct calls, so a member that
//! fails translation would leave dangling callees — the whole group is refused
//! instead (and stays interpreted). Groups are independent, so one odd class
//! never disables AOT elsewhere.
//!
//! The walker is a worklist over basic blocks with an abstract stack of typed
//! Cranelift values. Anything not provably in the subset refuses the group —
//! never guards, never falls back at runtime. Semantics are pinned to
//! `devirt_ops`: wrapping i64 add/sub/mul, `/`/`%` raising only on a zero
//! divisor (with `i64::MIN / -1` *wrapping*, which Cranelift's `sdiv` would
//! trap on — hence the explicit −1 path), f64 ops that never raise, and `%` on
//! doubles via Rust's `%` (an imported helper — Cranelift has no `frem`).

use std::collections::HashMap;
use std::rc::Rc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block as CBlock, InstBuilder, MemFlagsData, Signature, Type, Value as CVal, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::instruction::{Constant, Instruction, IntBinKind, StaticBlock};
use crate::symbol::{Symbol, self_symbol};

use super::{
    AOT_MAX_CALL_DEPTH, AotCandidate, AotEntry, AotKind, AotRawFn, TAG_DEPTH, TAG_DIV_ZERO,
};

/// `%` on doubles: Rust's truncated remainder (what `devirt_ops::double_bin`
/// computes); Cranelift has no `frem`, so compiled code imports this.
unsafe extern "C" fn aot_fmod(a: f64, b: f64) -> f64 {
    a % b
}

type SiblingMap = HashMap<(u32, String), (Vec<AotKind>, AotKind, u32)>;

/// Compile every group; per group all-or-nothing. Returns the successfully
/// registered entries and the refusals `(selector, reason)`.
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
    for (_, members) in groups {
        match compile_group(&members, siblings) {
            Ok(mut entries) => compiled.append(&mut entries),
            Err(reason) => {
                for m in &members {
                    refused.push((m.selector.clone(), reason.clone()));
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

fn compile_group(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
) -> Result<Vec<(u32, AotEntry)>, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    jb.symbol("qn_aot_checkpoint", super::checkpoint_addr());
    jb.symbol("qn_aot_fmod", aot_fmod as *const u8);
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();

    // Imported helpers.
    let mut cp_sig = module.make_signature();
    cp_sig.params.push(AbiParam::new(ptr)); // vm
    cp_sig.params.push(AbiParam::new(ptr)); // fuel
    cp_sig.returns.push(AbiParam::new(types::I8));
    let cp_id = module
        .declare_function("qn_aot_checkpoint", Linkage::Import, &cp_sig)
        .map_err(|e| e.to_string())?;
    let mut fmod_sig = module.make_signature();
    fmod_sig.params.push(AbiParam::new(types::F64));
    fmod_sig.params.push(AbiParam::new(types::F64));
    fmod_sig.returns.push(AbiParam::new(types::F64));
    let fmod_id = module
        .declare_function("qn_aot_fmod", Linkage::Import, &fmod_sig)
        .map_err(|e| e.to_string())?;

    // Declare every member's inner fn first (mutual recursion), then trampolines.
    let mut inner_ids: HashMap<u32, FuncId> = HashMap::new();
    for m in members {
        let tid = m.block.template_id.ok_or("candidate without template id")?;
        let sig = inner_sig(&mut module, ptr, m);
        let fid = module
            .declare_function(&format!("t{tid}"), Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        inner_ids.insert(tid, fid);
    }

    let mut fb_ctx = FunctionBuilderContext::new();
    let mut tramp_ids: Vec<(u32, FuncId, &AotCandidate)> = Vec::new();

    for m in members {
        let tid = m.block.template_id.unwrap();
        // Inner fn body.
        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig(&mut module, ptr, m);
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let mut tr = Translator {
                module: &mut module,
                cand: m,
                siblings,
                inner_ids: &inner_ids,
                cp_id,
                fmod_id,
            };
            tr.build_inner(&mut b)?;
            b.seal_all_blocks();
            b.finalize();
        }
        let fid = inner_ids[&tid];
        module
            .define_function(fid, &mut ctx)
            .map_err(|e| format!("{e:?}\nIR:\n{}", ctx.func.display()))?;

        // Trampoline (the uniform raw ABI).
        let mut tctx = module.make_context();
        tctx.func.signature = tramp_sig(&mut module, ptr);
        let tramp_id = module
            .declare_function(
                &format!("t{tid}_tramp"),
                Linkage::Local,
                &tctx.func.signature,
            )
            .map_err(|e| e.to_string())?;
        {
            let mut b = FunctionBuilder::new(&mut tctx.func, &mut fb_ctx);
            build_trampoline(&mut module, &mut b, m, fid, ptr);
            b.seal_all_blocks();
            b.finalize();
        }
        module
            .define_function(tramp_id, &mut tctx)
            .map_err(|e| e.to_string())?;
        tramp_ids.push((tid, tramp_id, m));
    }

    module.finalize_definitions().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (tid, tramp_id, m) in tramp_ids {
        let addr = module.get_finalized_function(tramp_id);
        let raw: AotRawFn = unsafe { std::mem::transmute(addr) };
        out.push((
            tid,
            AotEntry {
                raw,
                params: m.params.clone().into_boxed_slice(),
                ret: m.ret,
            },
        ));
    }
    // The code must live for the process (fn pointers are registered globally):
    // leak the module, same append-only lifetime as the interner.
    std::mem::forget(module);
    Ok(out)
}

fn inner_sig(module: &mut JITModule, ptr: Type, m: &AotCandidate) -> Signature {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(ptr)); // vm
    sig.params.push(AbiParam::new(ptr)); // fuel
    sig.params.push(AbiParam::new(ptr)); // depth
    for &k in &m.params {
        sig.params.push(AbiParam::new(kind_type(k)));
    }
    sig.returns.push(AbiParam::new(types::I8)); // tag
    sig.returns.push(AbiParam::new(kind_type(m.ret)));
    sig
}

fn tramp_sig(module: &mut JITModule, ptr: Type) -> Signature {
    let mut sig = module.make_signature();
    for _ in 0..5 {
        sig.params.push(AbiParam::new(ptr)); // vm, fuel, depth, args, ret
    }
    sig.returns.push(AbiParam::new(types::I8));
    sig
}

fn build_trampoline(
    module: &mut JITModule,
    b: &mut FunctionBuilder,
    m: &AotCandidate,
    inner: FuncId,
    ptr: Type,
) {
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let (vm, fuel, depth, args, ret) = (p[0], p[1], p[2], p[3], p[4]);
    let mut call_args = vec![vm, fuel, depth];
    for (i, &k) in m.params.iter().enumerate() {
        let off = (i * 8) as i32;
        let v = match k {
            AotKind::Int => b.ins().load(types::I64, MemFlagsData::trusted(), args, off),
            AotKind::Double => b.ins().load(types::F64, MemFlagsData::trusted(), args, off),
            AotKind::Bool => {
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
        AotKind::Int => {
            b.ins().store(MemFlagsData::trusted(), val, ret, 0);
        }
        AotKind::Double => {
            b.ins().store(MemFlagsData::trusted(), val, ret, 0);
        }
        AotKind::Bool => {
            let w = b.ins().uextend(types::I64, val);
            b.ins().store(MemFlagsData::trusted(), w, ret, 0);
        }
    }
    b.ins().return_(&[tag]);
    let _ = ptr;
}

/// An abstract stack slot: a concrete typed value, the method receiver (`self`,
/// which only a self-call may consume), or the `nil` a local-declaration
/// prologue pushes (which only `DefineLocal` may consume).
#[derive(Clone, Copy)]
enum AV {
    C(CVal, AotKind),
    SelfRef,
    Nil,
}

struct Translator<'a> {
    module: &'a mut JITModule,
    cand: &'a AotCandidate,
    siblings: &'a SiblingMap,
    inner_ids: &'a HashMap<u32, FuncId>,
    cp_id: FuncId,
    fmod_id: FuncId,
}

struct FnCtx {
    vm: CVal,
    fuel: CVal,
    depth: CVal,
    exit: CBlock,
    ret_kind: AotKind,
}

impl<'a> Translator<'a> {
    fn build_inner(&mut self, b: &mut FunctionBuilder) -> Result<(), String> {
        let code: &Rc<StaticBlock> = &self.cand.block;
        let insts = &code.bytecode.0;

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let (vm, fuel, depth) = (p[0], p[1], p[2]);

        // Named locals: params first (their types are the signature's), the rest
        // declared lazily at first store.
        let mut vars: HashMap<Symbol, (Variable, AotKind)> = HashMap::new();
        let mut declared_uninit: HashMap<Symbol, ()> = HashMap::new();
        for (i, (&sym, &k)) in code
            .param_syms
            .iter()
            .zip(self.cand.params.iter())
            .enumerate()
        {
            let var = b.declare_var(kind_type(k));
            b.def_var(var, p[3 + i]);
            vars.insert(sym, (var, k));
        }

        // Exit block: single point that undoes the depth increment and returns.
        let exit = b.create_block();
        b.append_block_param(exit, types::I8);
        b.append_block_param(exit, kind_type(self.cand.ret));
        let fx = FnCtx {
            vm,
            fuel,
            depth,
            exit,
            ret_kind: self.cand.ret,
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

        // Basic-block map over the bytecode: leaders = jump targets.
        let mut leaders: Vec<usize> = Vec::new();
        for (ip, inst) in insts.iter().enumerate() {
            let off = match inst {
                Instruction::Jump(o)
                | Instruction::IfJump(o)
                | Instruction::ElseJump(o)
                | Instruction::BranchIfNotBool(o) => *o,
                _ => continue,
            };
            let target = ip as isize + off;
            if target < 0 || target as usize >= insts.len() {
                return Err(format!("jump out of range at ip {ip}"));
            }
            leaders.push(target as usize);
            if !matches!(inst, Instruction::Jump(_)) {
                leaders.push(ip + 1); // conditional fallthrough starts a block
            }
        }
        leaders.sort_unstable();
        leaders.dedup();

        let mut blocks: HashMap<usize, (CBlock, Vec<AotKind>)> = HashMap::new();
        let mut done: HashMap<usize, ()> = HashMap::new();
        let mut work: Vec<usize> = Vec::new();

        // Walk from the entry continuation with an empty stack, then drain the
        // worklist of jump-target blocks (each filled exactly once).
        let mut cursor = Some((0usize, Vec::<AV>::new()));
        loop {
            let (start_ip, mut stack) = match cursor.take() {
                Some(s) => s,
                None => match work.pop() {
                    Some(ip) => {
                        if done.contains_key(&ip) {
                            continue;
                        }
                        done.insert(ip, ());
                        let (bl, kinds) = blocks[&ip].clone();
                        b.switch_to_block(bl);
                        let params = b.block_params(bl).to_vec();
                        let stack = params
                            .iter()
                            .zip(kinds.iter())
                            .map(|(&v, &k)| AV::C(v, k))
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
                // Fallthrough into a leader: close this block with a jump.
                if ip != start_ip && leaders.binary_search(&ip).is_ok() {
                    let (bl, _) = self.block_for(b, &mut blocks, &mut work, ip, &stack)?;
                    let args = Self::stack_args(&stack)?;
                    b.ins().jump(bl, &args);
                    break 'block;
                }
                match &insts[ip] {
                    Instruction::Push(c) => match c {
                        Constant::Int(i) => {
                            let v = b.ins().iconst(types::I64, *i);
                            stack.push(AV::C(v, AotKind::Int));
                        }
                        Constant::Double(d) => {
                            let v = b.ins().f64const(*d);
                            stack.push(AV::C(v, AotKind::Double));
                        }
                        Constant::Bool(x) => {
                            let v = b.ins().iconst(types::I8, *x as i64);
                            stack.push(AV::C(v, AotKind::Bool));
                        }
                        Constant::Nil => stack.push(AV::Nil),
                        _ => return Err(format!("unsupported constant at ip {ip}")),
                    },
                    Instruction::LoadLocal(sym) => {
                        if *sym == self_symbol() {
                            stack.push(AV::SelfRef);
                        } else if let Some(&(var, k)) = vars.get(sym) {
                            let v = b.use_var(var);
                            stack.push(AV::C(v, k));
                        } else {
                            return Err(format!("read of unknown/uninitialized local at ip {ip}"));
                        }
                    }
                    Instruction::DefineLocal(sym) | Instruction::StoreLocal(sym) => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        match v {
                            AV::Nil if matches!(insts[ip], Instruction::DefineLocal(_)) => {
                                declared_uninit.insert(*sym, ());
                            }
                            AV::C(cv, k) => self.store_local(b, &mut vars, *sym, cv, k)?,
                            _ => return Err(format!("unsupported store at ip {ip}")),
                        }
                    }
                    Instruction::DefineLocalKeep(sym) | Instruction::StoreLocalKeep(sym) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        match v {
                            AV::C(cv, k) => self.store_local(b, &mut vars, *sym, cv, k)?,
                            _ => return Err(format!("unsupported store at ip {ip}")),
                        }
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
                        let ra = Self::local_scalar(b, &vars, *a, AotKind::Int)?;
                        let rb = Self::local_scalar(b, &vars, *bb, AotKind::Int)?;
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLC(a, c, kind) => {
                        let ra = Self::local_scalar(b, &vars, *a, AotKind::Int)?;
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
                        let ra = Self::local_scalar(b, &vars, *a, AotKind::Double)?;
                        let rb = Self::local_scalar(b, &vars, *bb, AotKind::Double)?;
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::DoubleBinLC(a, c, kind) => {
                        let ra = Self::local_scalar(b, &vars, *a, AotKind::Double)?;
                        let cd = match c {
                            Constant::Double(d) => *d,
                            Constant::Int(i) => *i as f64,
                            _ => return Err("DoubleBinLC without numeric constant".into()),
                        };
                        let rb = b.ins().f64const(cd);
                        let out = self.emit_double_bin(b, *kind, ra, rb);
                        stack.push(out);
                    }
                    Instruction::Jump(off) => {
                        let target = (ip as isize + off) as usize;
                        if target <= ip {
                            stack = self.emit_fuel_tick(b, &fx, &stack)?;
                        }
                        let (bl, _) = self.block_for(b, &mut blocks, &mut work, target, &stack)?;
                        let args = Self::stack_args(&stack)?;
                        b.ins().jump(bl, &args);
                        break 'block;
                    }
                    Instruction::IfJump(off) | Instruction::ElseJump(off) => {
                        let cond = match stack.pop().ok_or("stack underflow")? {
                            AV::C(v, AotKind::Bool) => v,
                            _ => return Err(format!("non-Bool condition at ip {ip}")),
                        };
                        let target = (ip as isize + off) as usize;
                        if target <= ip {
                            // Conditional back-edge: tick before the branch (both
                            // successors pay one decrement — loops are the shape
                            // that must stay preemptible and cancellable).
                            stack = self.emit_fuel_tick(b, &fx, &stack)?;
                        }
                        let (tbl, _) = self.block_for(b, &mut blocks, &mut work, target, &stack)?;
                        let (fbl, _) = self.block_for(b, &mut blocks, &mut work, ip + 1, &stack)?;
                        let args = Self::stack_args(&stack)?;
                        if matches!(insts[ip], Instruction::IfJump(_)) {
                            b.ins().brif(cond, tbl, &args, fbl, &args);
                        } else {
                            b.ins().brif(cond, fbl, &args, tbl, &args);
                        }
                        break 'block;
                    }
                    Instruction::BranchIfNotBool(_) => {
                        // The peeked receiver is statically Bool in this subset, so
                        // the guard always falls through; the cold real-send path is
                        // unreachable and never translated.
                        match stack.last() {
                            Some(AV::C(_, AotKind::Bool)) => {}
                            _ => return Err(format!("BranchIfNotBool over non-Bool at ip {ip}")),
                        }
                    }
                    Instruction::Send(sel, n) | Instruction::SendLocal(_, sel, n) => {
                        let n = *n;
                        let explicit_recv = matches!(insts[ip], Instruction::Send(..));
                        if let Instruction::SendLocal(recv, ..) = &insts[ip]
                            && *recv != self_symbol()
                        {
                            return Err(format!("non-self send receiver at ip {ip}"));
                        }
                        let key = (self.cand.group_id, sel.as_str().to_string());
                        let Some((psig, pret, ptid)) = self.siblings.get(&key) else {
                            return Err(format!(
                                "send to non-sibling selector '{}' at ip {ip}",
                                sel.as_str()
                            ));
                        };
                        if psig.len() != n {
                            return Err(format!("arity mismatch calling '{}'", sel.as_str()));
                        }
                        let mut args_v = Vec::with_capacity(n);
                        for i in (0..n).rev() {
                            let want = psig[i];
                            args_v.push(Self::pop_kind(&mut stack, want)?);
                        }
                        args_v.reverse();
                        if explicit_recv {
                            match stack.pop() {
                                Some(AV::SelfRef) => {}
                                _ => return Err(format!("non-self receiver at ip {ip}")),
                            }
                        }
                        let callee_fid = self.inner_ids[ptid];
                        let callee = self.module.declare_func_in_func(callee_fid, b.func);
                        let mut call_args = vec![fx.vm, fx.fuel, fx.depth];
                        call_args.extend(args_v);
                        let call = b.ins().call(callee, &call_args);
                        let res = b.inst_results(call).to_vec();
                        let (tag, val) = (res[0], res[1]);
                        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
                        let bad_bl = b.create_block();
                        let ok_bl = b.create_block();
                        b.ins().brif(bad, bad_bl, &[], ok_bl, &[]);
                        b.switch_to_block(bad_bl);
                        let zero = self.zero_of(b, fx.ret_kind);
                        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
                        b.switch_to_block(ok_bl);
                        stack.push(AV::C(val, *pret));
                    }
                    Instruction::Return | Instruction::BlockReturn => {
                        let v = match stack.pop().ok_or("stack underflow")? {
                            AV::C(v, k) if k == fx.ret_kind => v,
                            AV::C(_, k) => {
                                return Err(format!(
                                    "return kind {k:?} != declared {:?}",
                                    fx.ret_kind
                                ));
                            }
                            _ => return Err("returning non-scalar".to_string()),
                        };
                        let tag = b.ins().iconst(types::I8, 0);
                        b.ins().jump(fx.exit, &[tag.into(), v.into()]);
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

    /// Fuel decrement + (rarely) checkpoint, carrying the live abstract stack
    /// through the checkpoint's control flow as block params so values stay in
    /// SSA across the call; returns the rebuilt stack. Emitted in every
    /// prologue (covers recursion) and at loop back-edges (covers loops) — the
    /// two shapes that must stay preemptible and cancellable.
    fn emit_fuel_tick(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, String> {
        let keep = Self::stack_args(stack)?;
        let kinds: Vec<AotKind> = stack
            .iter()
            .map(|v| match v {
                AV::C(_, k) => Ok(*k),
                _ => Err("self/nil live at a fuel checkpoint".to_string()),
            })
            .collect::<Result<_, String>>()?;
        let f0 = b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), fx.fuel, 0);
        let f1 = b.ins().iadd_imm(f0, -1);
        b.ins().store(MemFlagsData::trusted(), f1, fx.fuel, 0);
        let spent = b.ins().icmp_imm(IntCC::SignedLessThanOrEqual, f1, 0);
        let cp_bl = b.create_block();
        let cont = b.create_block();
        for &k in &kinds {
            b.append_block_param(cont, kind_type(k));
        }
        b.ins().brif(spent, cp_bl, &[], cont, &keep);
        b.switch_to_block(cp_bl);
        let cp = self.module.declare_func_in_func(self.cp_id, b.func);
        let call = b.ins().call(cp, &[fx.vm, fx.fuel]);
        let tag = b.inst_results(call)[0];
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let cp_bad = b.create_block();
        b.ins().brif(bad, cp_bad, &[], cont, &keep);
        b.switch_to_block(cp_bad);
        let zero = self.zero_of(b, fx.ret_kind);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(cont);
        let params = b.block_params(cont).to_vec();
        Ok(params
            .iter()
            .zip(kinds.iter())
            .map(|(&v, &k)| AV::C(v, k))
            .collect())
    }

    fn emit_fuel_tick_empty(&mut self, b: &mut FunctionBuilder, fx: &FnCtx) {
        self.emit_fuel_tick(b, fx, &[])
            .expect("empty-stack tick cannot fail");
    }

    fn store_local(
        &mut self,
        b: &mut FunctionBuilder,
        vars: &mut HashMap<Symbol, (Variable, AotKind)>,
        sym: Symbol,
        v: CVal,
        k: AotKind,
    ) -> Result<(), String> {
        match vars.get(&sym) {
            Some(&(var, vk)) => {
                if vk != k {
                    return Err(format!("local '{}' changes kind", sym.as_str()));
                }
                b.def_var(var, v);
            }
            None => {
                let var = b.declare_var(kind_type(k));
                b.def_var(var, v);
                vars.insert(sym, (var, k));
            }
        }
        Ok(())
    }

    fn local_scalar(
        b: &mut FunctionBuilder,
        vars: &HashMap<Symbol, (Variable, AotKind)>,
        sym: Symbol,
        want: AotKind,
    ) -> Result<CVal, String> {
        match vars.get(&sym) {
            Some(&(var, k)) if k == want => Ok(b.use_var(var)),
            Some(_) => Err(format!("local '{}' has wrong kind", sym.as_str())),
            None => Err(format!("read of unknown local '{}'", sym.as_str())),
        }
    }

    fn pop_kind(stack: &mut Vec<AV>, want: AotKind) -> Result<CVal, String> {
        match stack.pop() {
            Some(AV::C(v, k)) if k == want => Ok(v),
            Some(AV::C(_, k)) => Err(format!("operand kind {k:?}, wanted {want:?}")),
            Some(_) => Err("non-scalar operand".to_string()),
            None => Err("stack underflow".to_string()),
        }
    }

    fn stack_args(stack: &[AV]) -> Result<Vec<cranelift_codegen::ir::BlockArg>, String> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(cv, _) => Ok((*cv).into()),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect()
    }

    /// The Cranelift block for bytecode leader `ip`, creating it (with one
    /// parameter per stack slot) and queueing it on first sight; on later
    /// sights the incoming stack kinds must match.
    fn block_for(
        &mut self,
        b: &mut FunctionBuilder,
        blocks: &mut HashMap<usize, (CBlock, Vec<AotKind>)>,
        work: &mut Vec<usize>,
        ip: usize,
        stack: &[AV],
    ) -> Result<(CBlock, Vec<AotKind>), String> {
        let kinds: Vec<AotKind> = stack
            .iter()
            .map(|v| match v {
                AV::C(_, k) => Ok(*k),
                _ => Err("self/nil live at block boundary".to_string()),
            })
            .collect::<Result<_, String>>()?;
        if let Some((bl, expect)) = blocks.get(&ip) {
            if *expect != kinds {
                return Err(format!("stack shape mismatch at merge ip {ip}"));
            }
            return Ok((*bl, expect.clone()));
        }
        let bl = b.create_block();
        for &k in &kinds {
            b.append_block_param(bl, kind_type(k));
        }
        blocks.insert(ip, (bl, kinds.clone()));
        work.push(ip);
        Ok((bl, kinds))
    }

    fn zero_of(&self, b: &mut FunctionBuilder, k: AotKind) -> CVal {
        match k {
            AotKind::Int => b.ins().iconst(types::I64, 0),
            AotKind::Double => b.ins().f64const(0.0),
            AotKind::Bool => b.ins().iconst(types::I8, 0),
        }
    }

    fn bail(&self, b: &mut FunctionBuilder, fx: &FnCtx, tag: u8) {
        let t = b.ins().iconst(types::I8, tag as i64);
        let zero = self.zero_of(b, fx.ret_kind);
        b.ins().jump(fx.exit, &[t.into(), zero.into()]);
    }

    /// Integer ops with `devirt_ops::int_bin` semantics: wrapping add/sub/mul;
    /// `/`/`%` bail on a zero divisor, and take an explicit `-1` path (negate /
    /// zero) because `i64::MIN / -1` must wrap where `sdiv` would trap.
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

    /// f64 ops with `devirt_ops::double_bin` semantics: never raise; `/` gives
    /// inf/NaN; `%` is Rust's truncated remainder (imported helper).
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
                let f = self.module.declare_func_in_func(self.fmod_id, b.func);
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
