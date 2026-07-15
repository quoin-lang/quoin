//! The bytecode walk on `Translator`: `build_inner`'s per-instruction lowering,
//! plus closure/env/field lowering and the loop-span/write-back gating it leans on.

use super::*;

impl<'a> Translator<'a> {
    pub(super) fn build_inner(&mut self, b: &mut FunctionBuilder) -> Result<(), Refusal> {
        let insts = &self.cand.block.bytecode.0.clone();

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let (vm, mc, fuel, depth, epoch, slots, slot_base) =
            (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);

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
                    b.def_var(var, p[7 + i]);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                }
                AotParam::Obj => {
                    obj_param_avs.insert(sym, p[7 + i]);
                    // B1: a `List`-hinted param is a dispatch-GUARANTEED native
                    // List (List is sealed; the hint only matches the native
                    // class) — and a tag-required param is guaranteed tagged,
                    // since tag requirements gate dispatch too (G1). These
                    // proofs are what let a fused `each:` guard fall away.
                    // METHOD role only: a block's annotations are beliefs
                    // (`value:` checks nothing) — never proofs.
                    if self.cand.role == AotRole::Method
                        && self.cand.block.param_types.get(i).map(String::as_str) == Some("List")
                    {
                        let proof = match self.cand.block.param_elem_tags.get(i).copied().flatten()
                        {
                            Some(tag) => DynProof::CollectionOf(tag),
                            None => DynProof::NativeList,
                        };
                        self.proofs.insert(p[7 + i], proof);
                    }
                }
            }
        }

        let exit = b.create_block();
        b.append_block_param(exit, types::I8);
        b.append_block_param(exit, ret_type(self.eff_ret));
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
        let direct_ret =
            b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let fx = FnCtx {
            vm,
            mc,
            fuel,
            depth,
            epoch,
            slots,
            slot_base,
            exit,
            ret: self.eff_ret,
            direct_ret,
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
                | Instruction::BranchIfNotList(o, _)
                | Instruction::BranchIfNotPlainNew(o) => *o,
                _ => continue,
            };
            let target = ip as isize + off;
            if target < 0 || target as usize >= insts.len() {
                return Err(format!("jump out of range at ip {ip}").into());
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
                    return Err("fell off the end of bytecode".into());
                }
                if ip != start_ip && leaders.binary_search(&ip).is_ok() {
                    let mut nstack = self.norm_stack(b, &fx, &stack)?;
                    let (bl, _) =
                        self.block_for(b, &fx, &mut blocks, &mut work, ip, &mut nstack)?;
                    let args = Self::stack_args(&nstack)?;
                    b.ins().jump(bl, &args);
                    break 'block;
                }
                match &insts[ip] {
                    Instruction::Push(c) => {
                        let av = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadLocal(sym) => {
                        let av = self.local_av(b, &fx, &mut vars, &obj_param_avs, *sym, ip)?;
                        stack.push(av);
                    }
                    Instruction::LoadGlobal(name) => {
                        let leaked: &'static crate::value::NamespacedName =
                            Box::leak(Box::new(name.clone()));
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
                        self.refuse_tracked_escape(v, ip, "a local")?;
                        if matches!((v, &insts[ip]), (AV::Nil, Instruction::DefineLocal(_)))
                            && !vars.contains_key(sym)
                            && !obj_param_avs.contains_key(sym)
                        {
                            if Self::in_loop_span(insts, ip) {
                                // F2: an IN-LOOP `var x = nil` re-executes per
                                // iteration — give it a real slot and re-nil it
                                // HERE, the interpreter's fresh-binding-per-
                                // execution semantics. Deferral would leave
                                // iteration 2 reading iteration 1's value
                                // (unreachable today only because M1's survival
                                // walk refuses the conditional-store shapes
                                // that would expose it — this makes the
                                // semantics right by construction, not by
                                // shield).
                                self.nil_deferred.remove(sym);
                                let w = self.alloc_scratch()?;
                                let idx = self.abs_slot(b, &fx, w);
                                let (k, bits) = self.encode(b, &fx, AV::Nil);
                                let f = self.func_ref(b, self.helpers.slot_set);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx, k, bits]);
                                let tag = b.inst_results(call)[0];
                                self.tag_check(b, &fx, tag);
                                vars.insert(*sym, VarSlot::Obj(w, None));
                            } else {
                                // declaration prologue: type decided at first
                                // store — TRACKED: a read forces a slot (F2,
                                // entry-nil), and a closure materialization
                                // must force still-deferred vars too (a
                                // write-captured block stores them OUT-OF-BAND
                                // through its snapshot env, invisible to
                                // "first store"; the S1 recordResult
                                // nil-capture bug).
                                self.nil_deferred.insert(*sym);
                            }
                        } else if self.free_in_block(&vars, &obj_param_avs, *sym)
                            && matches!(&insts[ip], Instruction::StoreLocal(_))
                        {
                            // B3a: a captured-variable write goes through the
                            // closure's real EnvFrame cell — exact shared-cell
                            // semantics (`sum = sum + x` mutates the caller's
                            // binding, as interpreted).
                            self.emit_env_set(b, &fx, *sym, v)?;
                        } else {
                            self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
                        }
                    }
                    Instruction::DefineLocalKeep(sym) | Instruction::StoreLocalKeep(sym) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a local")?;
                        if self.free_in_block(&vars, &obj_param_avs, *sym)
                            && matches!(&insts[ip], Instruction::StoreLocalKeep(_))
                        {
                            self.emit_env_set(b, &fx, *sym, v)?;
                        } else {
                            self.store_local(b, &fx, &mut vars, &obj_param_avs, *sym, v)?;
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
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &mut vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Int,
                            ip,
                        )?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &mut vars,
                            &obj_param_avs,
                            *bb,
                            AotKind::Int,
                            ip,
                        )?;
                        let out = self.emit_int_bin(b, &fx, *kind, ra, rb)?;
                        stack.push(out);
                    }
                    Instruction::IntBinLC(a, c, kind) => {
                        let ra = self.local_scalar(
                            b,
                            &fx,
                            &mut vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Int,
                            ip,
                        )?;
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
                            &mut vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let rb = self.local_scalar(
                            b,
                            &fx,
                            &mut vars,
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
                            &mut vars,
                            &obj_param_avs,
                            *a,
                            AotKind::Double,
                            ip,
                        )?;
                        let cd = match c {
                            Constant::Double(d) => *d,
                            Constant::Int(i) => *i as f64,
                            _ => {
                                return Err(refuse(
                                    RefusalKind::UnsupportedConstant,
                                    "DoubleBinLC without numeric constant".to_string(),
                                ));
                            }
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
                                return Err(refuse(
                                    RefusalKind::ArityCap,
                                    "list literal too long for v0.2".to_string(),
                                ));
                            }
                            let elems: Vec<AV> =
                                stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                            for e in &elems {
                                self.refuse_tracked_escape(*e, ip, "a list literal")?;
                            }
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
                        self.refuse_tracked_escape(val, ip, "a list")?;
                        let recv_idx = self.obj_index(b, &fx, recv, "ListPush receiver")?;
                        let (k, bits) = self.encode(b, &fx, val);
                        let f = self.func_ref(b, self.helpers.list_push);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, recv_idx, k, bits]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(recv_idx));
                    }
                    Instruction::BranchIfNotList(..) => {
                        // The fused-`each:` guard (B1, docs/internal/BLOCK_AOT_ARCH.md §3). A
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
                            return Err(refuse(
                                RefusalKind::UnprovenReceiver,
                                format!(
                                    "fused each: on an unproven receiver at ip {ip} — a \
                                     `List`-annotated param, a fresh/checked list, or `self` \
                                     (entry-gated) compiles"
                                ),
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
                            return Err(refuse(
                                RefusalKind::UnprovenReceiver,
                                "TagCollection on a non-slot value".to_string(),
                            ));
                        };
                        let Some(code) = tag.code() else {
                            return Err(refuse(
                                RefusalKind::UnprovenReceiver,
                                "user-class element tags in compiled literals are not \
                                 supported yet"
                                    .to_string(),
                            ));
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
                        self.refuse_tracked_escape(val, ip, "a list")?;
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
                        self.refuse_tracked_escape(key, ip, "a map key")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out = self.emit_outcall_nosite(b, &fx, recv, "at:", &[key], ip)?;
                        stack.push(out);
                    }
                    Instruction::MapSet => {
                        let val = stack.pop().ok_or("stack underflow")?;
                        let key = stack.pop().ok_or("stack underflow")?;
                        // These reissue as outcalls WITHOUT the send head's
                        // post-send write-back flush — a tracked closure here
                        // would orphan its obligations.
                        self.refuse_tracked_escape(val, ip, "a map")?;
                        self.refuse_tracked_escape(key, ip, "a map key")?;
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let out =
                            self.emit_outcall_nosite(b, &fx, recv, "at:put:", &[key, val], ip)?;
                        stack.push(out);
                    }
                    Instruction::Jump(off) => {
                        let target = (ip as isize + off) as usize;
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        if target <= ip {
                            nstack = self.emit_fuel_tick(b, &fx, &nstack)?;
                        }
                        let (bl, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, target, &mut nstack)?;
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
                            self.block_for(b, &fx, &mut blocks, &mut work, target, &mut nstack)?;
                        let (fbl, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, ip + 1, &mut nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        if matches!(insts[ip], Instruction::IfJump(_)) {
                            b.ins().brif(cond, tbl, &args, fbl, &args);
                        } else {
                            b.ins().brif(cond, fbl, &args, tbl, &args);
                        }
                        break 'block;
                    }
                    Instruction::RequireBool => {
                        // Statically Bool → no-op. Otherwise materialize the
                        // top to a slot and let the helper raise on a
                        // non-Bool (BUGS.md Finding 14). The value stays on
                        // the stack for the following ElseJump.
                        match *stack.last().ok_or("stack underflow")? {
                            AV::C(_, AotKind::Bool) => {}
                            _ => {
                                let nstack = self.norm_stack(b, &fx, &stack)?;
                                let idx = match nstack.last() {
                                    Some(AV::Dyn(i)) => *i,
                                    _ => return Err("RequireBool: top not slot-resident".into()),
                                };
                                let f = self.func_ref(b, self.helpers.require_bool);
                                let call = b.ins().call(f, &[fx.vm, fx.mc, idx]);
                                let tag = b.inst_results(call)[0];
                                self.tag_check(b, &fx, tag);
                                stack = nstack;
                            }
                        }
                    }
                    Instruction::BranchIfNotBool(off) => {
                        let target = (ip as isize + off) as usize;
                        match *stack.last().ok_or("stack underflow")? {
                            AV::C(_, AotKind::Bool) => {} // statically Bool: fall through
                            AV::C(..) | AV::Nil | AV::SelfRef => {
                                // Statically not a Bool: always the cold path.
                                let mut nstack = self.norm_stack(b, &fx, &stack)?;
                                let (bl, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    target,
                                    &mut nstack,
                                )?;
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
                                let (hot, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    ip + 1,
                                    &mut hot_stack,
                                )?;
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
                                    let (sel, argc) = Self::cold_send(insts, target)?;
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
                                let mut nstack = self.norm_stack(b, &fx, &stack)?;
                                let (cold, _) = self.block_for(
                                    b,
                                    &fx,
                                    &mut blocks,
                                    &mut work,
                                    target,
                                    &mut nstack,
                                )?;
                                let cold_args = Self::stack_args(&nstack)?;
                                b.ins().brif(is_bool, hot, &hot_args, cold, &cold_args);
                                break 'block;
                            }
                        }
                    }
                    Instruction::BranchIfNotPlainNew(off) => {
                        let target = (ip as isize + off) as usize;
                        let recv = *stack.last().ok_or("stack underflow")?;
                        if !matches!(recv, AV::Dyn(_)) {
                            // Statically never a plain class value (scalar/nil —
                            // and conservatively self): always the cold path,
                            // which performs the real send with exact semantics.
                            let mut nstack = self.norm_stack(b, &fx, &stack)?;
                            let (bl, _) = self.block_for(
                                b,
                                &fx,
                                &mut blocks,
                                &mut work,
                                target,
                                &mut nstack,
                            )?;
                            let args = Self::stack_args(&nstack)?;
                            b.ins().jump(bl, &args);
                            break 'block;
                        }
                        // The verdict helper shares the interpreted site's
                        // (template, ip) cache cell; the receiver stays on the
                        // stack for BOTH paths.
                        let (rk, rb) = self.encode(b, &fx, recv);
                        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
                        let f = self.func_ref(b, self.helpers.plain_new_check);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, tid_v, ip_v, len_v, rk, rb]);
                        let verdict = b.inst_results(call)[0];
                        let mut nstack = self.norm_stack(b, &fx, &stack)?;
                        let (hot, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, ip + 1, &mut nstack)?;
                        let (cold, _) =
                            self.block_for(b, &fx, &mut blocks, &mut work, target, &mut nstack)?;
                        let args = Self::stack_args(&nstack)?;
                        b.ins().brif(verdict, hot, &args, cold, &args);
                        break 'block;
                    }
                    Instruction::NewWithFields(names) => {
                        let n = names.len();
                        let args: Vec<AV> =
                            stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                        let recv = stack.pop().ok_or("stack underflow")?;
                        let (rk, rb) = self.encode(b, &fx, recv);
                        self.fill_lanes(b, &fx, &args)?;
                        let leaked: &'static [Symbol] =
                            Box::leak(names.iter().copied().collect::<Vec<_>>().into_boxed_slice());
                        let names_ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
                        let n_v = b.ins().iconst(types::I64, n as i64);
                        let out = self.alloc_scratch()?;
                        let out_idx = self.abs_slot(b, &fx, out);
                        let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
                        let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
                        let f = self.func_ref(b, self.helpers.new_with_fields);
                        let call = b
                            .ins()
                            .call(f, &[fx.vm, fx.mc, names_ptr, n_v, rk, rb, ka, ba, out_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, &fx, tag);
                        stack.push(AV::Dyn(out_idx));
                    }
                    // Fused sends push their folded operand(s), then share the
                    // generic path: `exec_send` semantics pop n args, then the
                    // receiver (the fused operand is the receiver only for
                    // 0-arg sends — for n≥1 it is the LAST ARGUMENT).
                    Instruction::Send(sel, n)
                    | Instruction::SendLocal(_, sel, n)
                    | Instruction::SendConst(_, sel, n)
                    | Instruction::SendField(_, sel, n)
                    | Instruction::SendLocalLocal(_, _, sel, n)
                    | Instruction::SendLocalConst(_, _, sel, n) => {
                        let (sel, n) = (*sel, *n);
                        match &insts[ip] {
                            Instruction::SendLocal(a, ..) => {
                                let v = self.local_av(b, &fx, &mut vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendField(field, ..) => {
                                // Interpreter parity: `SendField` loads the
                                // field UNCACHED (its single ip belongs to the
                                // send IC), then shares the send tail.
                                let v = self.emit_field_get_uncached(b, &fx, field)?;
                                stack.push(v);
                            }
                            Instruction::SendConst(c, ..) => {
                                let v = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalLocal(a, bb, ..) => {
                                let v = self.local_av(b, &fx, &mut vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v =
                                    self.local_av(b, &fx, &mut vars, &obj_param_avs, *bb, ip)?;
                                stack.push(v);
                            }
                            Instruction::SendLocalConst(a, c, ..) => {
                                let v = self.local_av(b, &fx, &mut vars, &obj_param_avs, *a, ip)?;
                                stack.push(v);
                                let v = self.const_av(b, &fx, &mut vars, &obj_param_avs, c, ip)?;
                                stack.push(v);
                            }
                            _ => {}
                        }
                        let args: Vec<AV> =
                            stack.split_off(stack.len().checked_sub(n).ok_or("underflow")?);
                        let recv = stack.pop().ok_or("stack underflow")?;
                        // B3a: the combinator seam — `valueWithSelfOrArg:` routes
                        // through the block-call helper, invoking a COMPILED block
                        // template directly on a registry hit (else the interpreted
                        // body, else the full send for non-block receivers).
                        // Sibling-closure interference: 2+ materialized
                        // closures consumed together, any of which WRITES a
                        // capture, cannot keep exact shared-cell semantics
                        // across independent snapshots — refuse (unfused
                        // `whileDo:`-shaped methods run interpreted).
                        let closure_args: Vec<CVal> = std::iter::once(&recv)
                            .chain(args.iter())
                            .filter_map(|v| match v {
                                AV::Dyn(idx) if self.materialized.contains(idx) => Some(*idx),
                                _ => None,
                            })
                            .collect();
                        if closure_args.len() >= 2
                            && closure_args
                                .iter()
                                .any(|idx| self.pending_writebacks.contains_key(idx))
                        {
                            return Err(refuse(
                                RefusalKind::WriteCapture,
                                format!("sibling closures share written captures at ip {ip}"),
                            ));
                        }
                        // A `catch`-family send consuming a `^^`-carrying
                        // closure: interpreted, a catch-all can CATCH the
                        // `^^` crossing it; a compiled home cannot reproduce
                        // that (in-flight compiled-target `^^` is
                        // uncatchable) — the method runs interpreted. The
                        // predicate lives next to the runtime registrations
                        // it must mirror.
                        if crate::runtime::block::is_catch_family(sel.as_str())
                            && closure_args
                                .iter()
                                .any(|idx| self.materialized_nlr.contains(idx))
                        {
                            return Err(refuse(
                                RefusalKind::NlrCatch,
                                format!("non-local return (^^) under a catch at ip {ip}"),
                            ));
                        }
                        let out = if sel.as_str() == "valueWithSelfOrArg:" && args.len() == 1 {
                            self.emit_block_call(b, &fx, recv, args[0], ip)?
                        } else {
                            self.emit_send(b, &fx, recv, sel, &args, ip)?
                        };
                        let mut consumed = args.clone();
                        consumed.push(recv);
                        self.flush_writebacks(b, &fx, &consumed)?;
                        stack.push(out);
                    }
                    // Within one method's bytecode, `MethodReturn` (`^^`) always
                    // targets THIS method's frame — a real nested block is a separate
                    // `StaticBlock` never translated inline, and a fused-`each:` body
                    // (B1) is spliced into this very frame. So all three return forms
                    // are the compiled function's return.
                    // In a BLOCK TEMPLATE (B3a) a `^^` must unwind interpreter
                    // frames the compiled world doesn't have — refused.
                    Instruction::MethodReturn if self.cand.role == AotRole::BlockTemplate => {
                        return Err(refuse(
                            RefusalKind::NlrTemplate,
                            format!("non-local return (^^) from a compiled block at ip {ip}"),
                        ));
                    }
                    Instruction::LoadField(name) => {
                        let out = self.emit_field_get(b, &fx, name, ip)?;
                        stack.push(out);
                    }
                    Instruction::StoreField(name) => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a field")?;
                        self.emit_field_set(b, &fx, name, v, ip)?;
                    }
                    Instruction::StoreFieldKeep(name) => {
                        let v = *stack.last().ok_or("stack underflow")?;
                        self.refuse_tracked_escape(v, ip, "a field")?;
                        self.emit_field_set(b, &fx, name, v, ip)?;
                    }
                    Instruction::Return | Instruction::BlockReturn | Instruction::MethodReturn => {
                        let v = stack.pop().ok_or("stack underflow")?;
                        self.emit_return(b, &fx, v)?;
                        break 'block;
                    }
                    other => {
                        return Err(refuse(
                            RefusalKind::UnsupportedInstruction,
                            format!("unsupported instruction at ip {ip}: {other:?}"),
                        ));
                    }
                }
                ip += 1;
            }
        }
        Ok(())
    }

    pub(super) fn func_ref(
        &mut self,
        b: &mut FunctionBuilder,
        id: FuncId,
    ) -> cranelift_codegen::ir::FuncRef {
        self.module.declare_func_in_func(id, b.func)
    }

    /// The absolute slot index of an object-shaped AV (helpers take indices).
    fn obj_index(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        v: AV,
        what: &str,
    ) -> Result<CVal, Refusal> {
        match v {
            AV::Dyn(idx) => Ok(idx),
            AV::SelfRef => {
                self.uses_self_slot.set(true);
                Ok(self.abs_slot(b, fx, 0))
            }
            _ => Err(refuse(
                RefusalKind::SlotResidency,
                format!("{what} is not slot-resident"),
            )),
        }
    }

    /// Is `sym` a FREE variable of a block template — not a param, not a
    /// block-own local, not `self`? (Method role: never — unknown names there
    /// are compile errors, as before.)
    fn free_in_block(
        &self,
        vars: &HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
    ) -> bool {
        self.cand.role == AotRole::BlockTemplate
            && sym != self_symbol()
            && !vars.contains_key(&sym)
            && !obj_params.contains_key(&sym)
    }

    /// Read a captured variable through the closure's EnvFrame chain (B3a).
    pub(super) fn emit_env_get(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        sym: Symbol,
    ) -> Result<AV, Refusal> {
        let block_idx = self.abs_slot(b, fx, 2);
        let leaked: &'static Symbol = Box::leak(Box::new(sym));
        let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let f = self.func_ref(b, self.helpers.env_get);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, block_idx, sym_ptr, out_idx]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// Write a captured variable through the closure's EnvFrame chain (B3a) —
    /// the same shared cell the enclosing frame reads.
    fn emit_env_set(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        sym: Symbol,
        v: AV,
    ) -> Result<(), Refusal> {
        let block_idx = self.abs_slot(b, fx, 2);
        let leaked: &'static Symbol = Box::leak(Box::new(sym));
        let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
        let (k, bits) = self.encode(b, fx, v);
        let f = self.func_ref(b, self.helpers.env_set);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, block_idx, sym_ptr, k, bits]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(())
    }

    /// Per-element block invocation (B3a): registry hit → the compiled block
    /// template directly; miss → the interpreted body; non-block receiver →
    /// the full `valueWithSelfOrArg:` send. One helper call either way.
    fn emit_block_call(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        arg: AV,
        ip: usize,
    ) -> Result<AV, Refusal> {
        let (rk, rbits) = self.encode(b, fx, recv);
        let (ak, abits) = self.encode(b, fx, arg);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let (tid_v, _ip_v, len_v) = self.site_consts(b, ip);
        // Block-call sites get D2-style cells too (guarded by TEMPLATE id —
        // all blocks share one class, so the receiver-class guard the
        // method cells use would alias every closure). The cell caches the
        // template's entry, killing the per-element registry RwLock the
        // combinator loops paid. Site id rides the ip lane's high bits,
        // same packing as the outcall helper.
        let site = {
            let s = self
                .prior_sites
                .as_ref()
                .and_then(|m| m.get(&ip).copied())
                .unwrap_or_else(crate::codegen::next_outcall_site);
            self.site_log.push((ip, s));
            s
        };
        let ip_site = b
            .ins()
            .iconst(types::I64, (ip as i64) | ((i64::from(site)) << 32));

        // Window-hoist (the block-edge slice): a baked BLOCK site calls the
        // template directly through a FRAME-HOISTED window — the callee's
        // 3+scratch slots are this caller's own scratch, pushed once at
        // frame entry and torn down at frame exit, so the per-element cost
        // is: the native identity guard + param slot_set (+ scratch re-nil
        // per F2) + one
        // call_indirect + the result copy. The generic helper call is the
        // guard-miss path, exactly as before.
        let baked = self
            .baked
            .get(&ip)
            .copied()
            .filter(|bk| bk.entry.role == crate::codegen::AotRole::BlockTemplate)
            // The identity guard reads the receiver's SLOT natively, so the
            // lane must be a slot index by construction (Dyn/SelfRef). A
            // scalar-lane receiver can never be the baked closure anyway.
            .filter(|_| matches!(recv, AV::Dyn(_) | AV::SelfRef));
        if let Some(bk) = baked {
            crate::codegen::TOTAL_DIRECT_SITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // The hoisted window: [self, param, block] + the callee's
            // scratch, allocated as OUR scratch (contiguous — alloc_scratch
            // is sequential within a translation).
            let w_self = self.alloc_scratch()?;
            let w_param = self.alloc_scratch()?;
            let w_block = self.alloc_scratch()?;
            let mut w_scratch = Vec::new();
            for _ in 0..bk.entry.n_scratch {
                w_scratch.push(self.alloc_scratch()?);
            }
            debug_assert_eq!(w_param, w_self + 1);
            debug_assert_eq!(w_block, w_self + 2);

            let generic_bl = b.create_block();
            let guard_bl = b.create_block();
            let direct_bl = b.create_block();
            let merge_bl = b.create_block();

            // 1. live epoch == baked epoch (native).
            let live = b
                .ins()
                .load(types::I64, MemFlagsData::trusted(), fx.epoch, 0);
            let want = b.ins().iconst(types::I64, bk.epoch as i64);
            let fresh = b.ins().icmp(IntCC::Equal, live, want);
            b.ins().brif(fresh, guard_bl, &[], generic_bl, &[]);

            // 2. block identity, FULLY NATIVE (the guard_block helper was
            //    the edge's last redundant crossing): load the receiver
            //    slot's 16 bytes and compare against the baked closure —
            //    identity implies template. The pointee is pinned in
            //    aot_baked_roots, so a recycled address cannot alias.
            //    NO fiber work: compiled frames never survive a park, so
            //    any resume re-enters through interpreted code and the
            //    next compiled ENTRY (invoke_tail -> entry_gates) marks the
            //    fiber before this edge can possibly run — the mark is
            //    already set, inductively, whenever we are here.
            b.switch_to_block(guard_bl);
            let (sptr, slen) = self.emit_head(b, fx);
            let id_bl = b.create_block();
            let in_b = b.ins().icmp(IntCC::UnsignedLessThan, rbits, slen);
            b.ins().brif(in_b, id_bl, &[], generic_bl, &[]);
            b.switch_to_block(id_bl);
            let roff = b.ins().imul_imm(rbits, 16);
            let raddr = b.ins().iadd(sptr, roff);
            let rtag = b.ins().load(types::I64, MemFlagsData::trusted(), raddr, 0);
            let rpay = b.ins().load(types::I64, MemFlagsData::trusted(), raddr, 8);
            let tag_ok = b
                .ins()
                .icmp_imm(IntCC::Equal, rtag, i64::from(bk.recv_kind));
            let want = b.ins().iconst(types::I64, bk.recv_ptr as i64);
            let pay_ok = b.ins().icmp(IntCC::Equal, rpay, want);
            let gok = b.ins().band(tag_ok, pay_ok);
            b.ins().brif(gok, direct_bl, &[], generic_bl, &[]);

            // 3. the direct edge.
            b.switch_to_block(direct_bl);
            // A3: the window writes are NATIVE stores (the whole point —
            // the slot_set version of this edge measured net-zero; see
            // notes). Param store shape is known statically from the AV.
            let w_param_abs = self.abs_slot(b, fx, w_param);
            match arg {
                AV::C(_, AotKind::Int) => {
                    self.emit_slot_store_scalar(b, fx, w_param_abs, KIND_INT, abits)
                }
                AV::C(_, AotKind::Double) => {
                    self.emit_slot_store_scalar(b, fx, w_param_abs, KIND_DOUBLE, abits)
                }
                AV::C(_, AotKind::Bool) => {
                    self.emit_slot_store_scalar(b, fx, w_param_abs, KIND_BOOL, abits)
                }
                AV::Nil => {
                    let zero = b.ins().iconst(types::I64, 0);
                    self.emit_slot_store_scalar(b, fx, w_param_abs, KIND_NIL, zero)
                }
                // Dyn/SelfRef lanes carry a source slot index in `abits`.
                AV::Dyn(_) | AV::SelfRef => self.emit_slot_copy(b, fx, w_param_abs, abits),
            }
            // The block object into slot 2: capture reads (`env_get`) go
            // through it. The guard verified this receiver; its lane is a
            // slot index (blocks are heap values).
            let w_block_abs = self.abs_slot(b, fx, w_block);
            self.emit_slot_copy(b, fx, w_block_abs, rbits);
            // F2: scratch slots are NIL at invocation entry.
            if !w_scratch.is_empty() {
                let zero = b.ins().iconst(types::I64, 0);
                for &wsl in &w_scratch {
                    let abs = self.abs_slot(b, fx, wsl);
                    self.emit_slot_store_scalar(b, fx, abs, KIND_NIL, zero);
                }
            }
            // the raw call: lanes = [param slot idx] in bits_buf lane 0.
            let ba2 = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
            b.ins().store(MemFlagsData::trusted(), w_param_abs, ba2, 0);
            let w_base_abs = self.abs_slot(b, fx, w_self);
            let raw_sig = {
                let ptr = self.module.target_config().pointer_type();
                let mut sig = self.module.make_signature();
                for _ in 0..6 {
                    sig.params.push(AbiParam::new(ptr)); // vm, mc, fuel, depth, epoch, slots
                }
                sig.params.push(AbiParam::new(types::I64)); // slot_base
                sig.params.push(AbiParam::new(ptr)); // args
                sig.params.push(AbiParam::new(ptr)); // ret
                sig.returns.push(AbiParam::new(types::I8));
                b.import_signature(sig)
            };
            let ptr_ty = self.module.target_config().pointer_type();
            let fnaddr = b.ins().iconst(ptr_ty, bk.entry.raw as usize as i64);
            let ret_addr = b.ins().stack_addr(ptr_ty, fx.direct_ret, 0);
            let dcall = b.ins().call_indirect(
                raw_sig,
                fnaddr,
                &[
                    fx.vm, fx.mc, fx.fuel, fx.depth, fx.epoch, fx.slots, w_base_abs, ba2, ret_addr,
                ],
            );
            let dtag = b.inst_results(dcall)[0];
            self.tag_check(b, fx, dtag);
            // result: blocks return via slot (Obj eff-ret) — the raw ret
            // lane is the result's absolute slot index; a NATIVE 16-byte
            // slot copy closes the site's Dyn contract. The head is fresh:
            // the callee's growth helpers synced at their exits.
            let retv = b.ins().stack_load(types::I64, fx.direct_ret, 0);
            self.emit_slot_copy(b, fx, out_idx, retv);
            b.ins().jump(merge_bl, &[]);

            // 4. guard miss: exactly today's generic seam.
            b.switch_to_block(generic_bl);
            let f = self.func_ref(b, self.helpers.block_call);
            let call = b.ins().call(
                f,
                &[
                    fx.vm, fx.mc, tid_v, ip_site, len_v, rk, rbits, ak, abits, out_idx,
                ],
            );
            let tag = b.inst_results(call)[0];
            self.tag_check(b, fx, tag);
            b.ins().jump(merge_bl, &[]);

            b.switch_to_block(merge_bl);
            return Ok(AV::Dyn(out_idx));
        }

        let f = self.func_ref(b, self.helpers.block_call);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_site, len_v, rk, rbits, ak, abits, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// `@name` read (S3): the receiver is this frame's slot-0 value and the
    /// slot cache is the shared `(template_id, ip)` cell — compiled and
    /// interpreted execution warm ONE field cache.
    /// `SendField`'s field read: the interpreter passes `cache_ip: None`
    /// there (the ip's cache slot belongs to the SEND), mirrored here by the
    /// out-of-range ip sentinel — probe and fill both miss past `bc_len`.
    fn emit_field_get_uncached(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
    ) -> Result<AV, Refusal> {
        let sentinel = self.cand.block.bytecode.0.len();
        self.emit_field_get(b, fx, name, sentinel)
    }

    fn emit_field_get(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
        ip: usize,
    ) -> Result<AV, Refusal> {
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        let name_ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
        let name_len = b.ins().iconst(types::I64, leaked.len() as i64);
        let self_idx = self.abs_slot(b, fx, 0);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.field_get);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, self_idx, name_ptr, name_len, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// `@name = v` (S3) — same shared cache; undeclared fields raise the
    /// interpreter's exact errors through the tag channel.
    fn emit_field_set(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        name: &str,
        v: AV,
        ip: usize,
    ) -> Result<(), Refusal> {
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        let name_ptr = b.ins().iconst(types::I64, leaked.as_ptr() as i64);
        let name_len = b.ins().iconst(types::I64, leaked.len() as i64);
        let self_idx = self.abs_slot(b, fx, 0);
        let (k, bits) = self.encode(b, fx, v);
        let (tid_v, ip_v, len_v) = self.site_consts(b, ip);
        let f = self.func_ref(b, self.helpers.field_set);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_v, len_v, self_idx, name_ptr, name_len, k, bits,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(())
    }

    /// B3b: materialize a closure at a compiled cold-path `Push(Block)` site.
    /// The snapshot env carries EVERY frame binding (scalars, slots, obj
    /// params, `self`) — exactly the visibility the interpreter's live env
    /// chain would give — and the gates guarantee the whole NEST only READS
    /// its frame captures (a captured-var write would mutate the snapshot,
    /// invisible to the compiled frame) or writes ones flushed back after the
    /// consuming send. Known accepted edge (documented): a closure that
    /// ESCAPES its consuming send (a custom `if:` storing it) sees the
    /// snapshot, not later frame writes.
    pub(super) fn materialize_closure(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        rc: &Arc<StaticBlock>,
        ip: usize,
    ) -> Result<AV, Refusal> {
        // Force still-deferred `var x = nil` locals into REAL slots before
        // snapshotting: the closure may read or write them through its
        // snapshot env, which "type decided at first store" cannot see. No
        // site init (F2): scratch slots are NIL at invocation entry, deferral
        // implies no store was translated, and in-loop declarations never
        // defer — so the slot already holds the declaration's value.
        let deferred: Vec<Symbol> = self.nil_deferred.drain().collect();
        for sym in deferred {
            let w = self.alloc_scratch()?;
            vars.insert(sym, VarSlot::Obj(w, None));
        }

        // Gates: scan the template's bytecode — TRANSITIVELY through nested
        // literals (S5b). The materialized closure runs INTERPRETED, so
        // nested blocks execute naturally (their env chain threads through
        // the closure's frame into the snapshot, and their `^^` home is
        // inherited from the closure's frame — the S5a machinery); the gate
        // only needs whole-nest knowledge of free WRITES (for writebacks),
        // `^^` presence, guarded blocks, and the trampoline signature.
        let mut scan = NestScan::default();
        scan_materialized_nest(rc, &HashSet::new(), self.cand.selector.as_str(), &mut scan)
            .map_err(|e| Refusal {
                kind: e.kind,
                why: format!("{} at ip {ip}", e.why),
            })?;
        let written_frees = scan.written_frees;
        let has_nlr = scan.has_nlr;
        // A `^^` with a catch-family send anywhere in the same nest: the
        // interpreted method would let a catch-all CATCH the `^^` crossing
        // it, which a compiled home cannot reproduce — refuse (mirrors the
        // send-head gate for method-level catch consumers).
        if has_nlr && scan.has_catch_send {
            return Err(refuse(
                RefusalKind::NlrCatch,
                format!("non-local return (^^) with a catch in a materialized nest at ip {ip}"),
            ));
        }
        // PROFITABILITY (S5a, empirical): a `^^` cold arm pays a snapshot
        // materialization (fresh EnvFrame + a bind per frame binding) each
        // time its site executes, so it is only worth compiling where it
        // runs AT MOST ONCE per invocation. Two shapes make it per-ITERATION
        // and pessimize the whole method — found by A/B: qnlib's `whileDo:`
        // trampoline made sieve 5.8x slower, `any?:` cost combinators 60%:
        // - the site sits inside a fused-loop span (a backward jump crosses
        //   it): one arm snapshot per element;
        // - the arm re-sends the candidate's OWN selector (the
        //   `^^s.whileDo:block` tail-recursive trampoline): one recursive
        //   call — and one snapshot — per iteration.
        // Straight-line early exits (richards' task bodies) pass both.
        if has_nlr {
            let insts = &self.cand.block.bytecode.0;
            let in_loop = Self::in_loop_span(insts, ip);
            // M3: a site inside a GUARD-FAIL COLD SPAN is exempt — it executes
            // only when the guard fails, and then the real send it feeds
            // dominates the snapshot cost (the interpreter-fallback economics
            // it already chose). This is what lets qnlib's `any?:` compile:
            // its `.if:{ ^^true }` arm splices on the hot path (an inline
            // MethodReturn, no closure), and the only materialization left is
            // the never-taken guarded-inline cold copy — which used to refuse
            // the whole method (measured: compiling any?: is −17% on
            // combinators). The own-selector rule below is deliberately NOT
            // exempted: the `whileDo:` trampoline's cold copy re-sends its own
            // selector, and compiling it deepens native recursion per
            // ITERATION (the original 5.8×-sieve shape). (The M3-era
            // deferred-nil condition is gone: F2 made the force init-free —
            // entry-nil slots — so a cold span can no longer re-nil a live
            // accumulator.)
            if in_loop && !Self::in_guard_cold_span(insts, ip) {
                return Err(refuse(
                    RefusalKind::MaterializationGate,
                    format!("per-iteration ^^ materialization (fused loop) at ip {ip}"),
                ));
            }
        }
        // The trampoline/recursion rule applies with or without a `^^`:
        // a nest that re-sends the candidate's OWN selector makes the
        // materialization cost recur per invocation level — one full-frame
        // snapshot per tree node in btrees' makeTree (compiling it measured
        // +6.8% on btrees; the interpreter's closures are a pointer share).
        // Compiling these shapes WELL (hoisted/lazy arm closures) is the
        // recorded follow-up, shared with qnlib's whileDo:. Deliberately NOT
        // cold-span-exempt — see the M3 note above.
        if scan.sends_own_selector {
            return Err(refuse(
                RefusalKind::RecursionGate,
                format!("per-invocation materialization in a recursive method at ip {ip}"),
            ));
        }
        // A write to a FRAME local mutates the snapshot — read back after the
        // consuming send. A write that resolves DEEPER than this frame walks
        // past the snapshot into the real env cells (exact as-is). A write to
        // a param/self has no writable home — refuse (M3 note: this and the
        // rules below stay unexempted; only the per-iteration profitability
        // rule above is about EXECUTION COUNT, which a cold span bounds).
        let mut writebacks: Vec<(Symbol, VarSlot)> = Vec::new();
        for s in written_frees {
            if writebacks.iter().any(|(w, _)| *w == s) {
                continue;
            }
            if let Some(&slot) = vars.get(&s) {
                writebacks.push((s, slot));
            } else if obj_params.contains_key(&s) || s == self_symbol() {
                return Err(refuse(
                    RefusalKind::WriteCapture,
                    format!(
                        "materialized block writes parameter/self '{}' at ip {ip}",
                        s.as_str()
                    ),
                ));
            }
        }
        // Build the closure in a scratch slot (rooted throughout), then bind
        // the whole frame environment into its snapshot env.
        let tmpl: &'static Arc<StaticBlock> = Box::leak(Box::new(rc.clone()));
        let tmpl_ptr = b
            .ins()
            .iconst(types::I64, tmpl as *const Arc<StaticBlock> as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let want_home = b.ins().iconst(types::I64, i64::from(has_nlr));
        let f = self.func_ref(b, self.helpers.make_closure);
        let call = b
            .ins()
            .call(f, &[fx.vm, fx.mc, tmpl_ptr, out_idx, want_home]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        let bind = |tr: &mut Self, b: &mut FunctionBuilder, sym: Symbol, v: AV| {
            let leaked: &'static Symbol = Box::leak(Box::new(sym));
            let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
            let (k, bits) = tr.encode(b, fx, v);
            let f = tr.func_ref(b, tr.helpers.closure_bind);
            let call = b.ins().call(f, &[fx.vm, fx.mc, out_idx, sym_ptr, k, bits]);
            let tag = b.inst_results(call)[0];
            tr.tag_check(b, fx, tag);
        };
        bind(self, b, self_symbol(), AV::SelfRef);
        for (&sym, &cv) in obj_params.iter() {
            bind(self, b, sym, AV::Dyn(cv));
        }
        let entries: Vec<(Symbol, VarSlot)> = vars.iter().map(|(&s, &v)| (s, v)).collect();
        for (sym, slot) in entries {
            let av = match slot {
                VarSlot::Scalar(var, k) => AV::C(b.use_var(var), k),
                VarSlot::Obj(w, _) => {
                    let idx = self.abs_slot(b, fx, w);
                    AV::Dyn(idx)
                }
            };
            bind(self, b, sym, av);
        }
        if !writebacks.is_empty() {
            self.pending_writebacks.insert(out_idx, writebacks);
        }
        self.materialized.insert(out_idx);
        if has_nlr {
            self.materialized_nlr.insert(out_idx);
        }
        Ok(AV::Dyn(out_idx))
    }

    /// Is `ip` inside a fused-loop span — crossed by any backward jump?
    /// (Every offset-carrying form counts; guard branches only go forward
    /// today, but a future backward fused form must not silently slip a
    /// per-iteration site past the checks that consult this.)
    fn in_loop_span(insts: &[Instruction], ip: usize) -> bool {
        insts.iter().enumerate().any(|(j, inst)| match inst {
            Instruction::Jump(o)
            | Instruction::IfJump(o)
            | Instruction::ElseJump(o)
            | Instruction::BranchIfNotBool(o)
            | Instruction::BranchIfNotList(o, _)
            | Instruction::BranchIfNotPlainNew(o) => {
                *o < 0 && {
                    let target = (j as isize + *o) as usize;
                    target <= ip && ip <= j
                }
            }
            _ => false,
        })
    }

    /// Is `ip` inside a GUARD-FAIL COLD SPAN — the `<cold>` half of the
    /// option-C dual emission `Branch*(→cold); <hot>; Jump(→join); <cold>`?
    /// Recognized structurally: a forward guard branch whose target is
    /// preceded by an unconditional forward `Jump` (the hot path's jump over
    /// the cold code) that lands past `ip`. Code there executes only when the
    /// guard FAILS, so a fused loop around it does not make it per-iteration
    /// in the common (guard-holds) case — the basis of the M3 exemption in
    /// `materialize_closure`.
    fn in_guard_cold_span(insts: &[Instruction], ip: usize) -> bool {
        insts.iter().enumerate().any(|(j, inst)| match inst {
            Instruction::BranchIfNotBool(o)
            | Instruction::BranchIfNotList(o, _)
            | Instruction::BranchIfNotPlainNew(o) => {
                *o > 0 && {
                    let t = (j as isize + *o) as usize;
                    t >= 1
                        && t <= ip
                        && matches!(insts.get(t - 1), Some(Instruction::Jump(o2))
                            if *o2 > 0 && ip < ((t - 1) as isize + *o2) as usize)
                }
            }
            _ => false,
        })
    }

    /// Flush a consumed closure's captured-write read-backs (B3b): each
    /// written frame local is read from the snapshot env and stored back into
    /// its SSA/slot home. Called right after the consuming send returns.
    fn flush_writebacks(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        consumed: &[AV],
    ) -> Result<(), Refusal> {
        for v in consumed {
            let AV::Dyn(idx) = v else { continue };
            let Some(wbs) = self.pending_writebacks.remove(idx) else {
                continue;
            };
            for (sym, slot) in wbs {
                let leaked: &'static Symbol = Box::leak(Box::new(sym));
                let sym_ptr = b.ins().iconst(types::I64, leaked as *const Symbol as i64);
                let tmp = self.alloc_scratch()?;
                let tmp_idx = self.abs_slot(b, fx, tmp);
                let f = self.func_ref(b, self.helpers.env_get);
                let call = b.ins().call(f, &[fx.vm, fx.mc, *idx, sym_ptr, tmp_idx]);
                let tag = b.inst_results(call)[0];
                self.tag_check(b, fx, tag);
                match slot {
                    VarSlot::Scalar(var, k) => {
                        let val = self.narrow_to_scalar(b, fx, tmp_idx, k);
                        b.def_var(var, val);
                    }
                    VarSlot::Obj(w, _) => {
                        let dst = self.abs_slot(b, fx, w);
                        let kind = b.ins().iconst(types::I64, KIND_SLOT);
                        let f = self.func_ref(b, self.helpers.slot_set);
                        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, kind, tmp_idx]);
                        let tag = b.inst_results(call)[0];
                        self.tag_check(b, fx, tag);
                    }
                }
            }
        }
        Ok(())
    }

    /// A materialized closure with OBLIGATIONS — pending write-backs, or a
    /// `^^` whose catch-parity gate must see its consumer — is tracked by
    /// the SSA value of its slot, and that bookkeeping does NOT survive a
    /// store/load round-trip through a local, field, or collection: the
    /// reloaded value is a fresh SSA id, so the obligations silently orphan.
    /// (Found live: `var blk = { total = total + 1 }; .run:blk` compiled to
    /// a method whose write-backs never flushed — `total` stayed 0.) Such
    /// closures must flow DIRECTLY from materialization to their consuming
    /// send; any escape refuses. Obligation-free closures may escape (their
    /// only divergence is the documented snapshot-vs-live-env edge).
    fn refuse_tracked_escape(&self, v: AV, ip: usize, what: &str) -> Result<(), Refusal> {
        if let AV::Dyn(idx) = v {
            if self.pending_writebacks.contains_key(&idx) {
                return Err(refuse(
                    RefusalKind::WriteCapture,
                    format!("write-capturing closure escapes to {what} at ip {ip}"),
                ));
            }
            if self.materialized_nlr.contains(&idx) {
                return Err(refuse(
                    RefusalKind::NlrEscape,
                    format!("non-local-return closure escapes to {what} at ip {ip}"),
                ));
            }
        }
        Ok(())
    }
}
