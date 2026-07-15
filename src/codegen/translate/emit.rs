//! IR-emission primitives on `Translator`: slot-window access, constants and
//! locals, sends/outcalls, returns and stack shaping, bails, fuel, and int/double
//! arithmetic.

use super::*;

impl<'a> Translator<'a> {
    pub(super) fn alloc_scratch(&mut self) -> Result<u32, Refusal> {
        if self.is_pure {
            // Translation-verified purity: the syntactic pure-set scan missed a
            // slot use (e.g. a sibling-selector send on a non-self receiver).
            // The caller demotes this member from the pure set and retries.
            self.pending_abort = Some(TranslateAbort::PurityDemote);
            return Err("scalar-pure member touched the slot window".into());
        }
        let fixed = match self.cand.role {
            // 0 = receiver, then ONE slot per param — scalar params occupy
            // (and waste) theirs, so the window layout coincides exactly with
            // the [receiver, args…] window every outcall/send caller already
            // pushed for rooting, letting `invoke` reuse it instead of
            // building a second copy (D1, docs/internal/OUTCALL_ARCH.md).
            AotRole::Method => 1 + self.cand.params.len() as u32,
            // 0 = self (the vWSOA arg), 1 = the param's own cell, 2 = the
            // block object (env access) — see `invoke_block`.
            AotRole::BlockTemplate => 3,
        };
        let k = fixed + self.next_scratch;
        self.next_scratch += 1;
        Ok(k)
    }

    /// A3 native slot access (docs/internal/WINDOW_ARENA_ARCH.md §3): load the slot
    /// head's (ptr, len). Loaded fresh per access sequence — every growth
    /// helper re-syncs the head at exit, so any head read after a helper
    /// call sees truth (the vm_mc canary enforces the discipline in debug).
    pub(super) fn emit_head(&mut self, b: &mut FunctionBuilder, fx: &FnCtx) -> (CVal, CVal) {
        let ptr_ty = self.module.target_config().pointer_type();
        let ptr = b.ins().load(ptr_ty, MemFlagsData::trusted(), fx.slots, 0);
        let len = b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), fx.slots, 8);
        (ptr, len)
    }

    /// Bounds-checked native scalar store: `slots[idx] = Value{tag, bits}`.
    /// `tag` is a Value discriminant (== the KIND constant for scalars/nil).
    /// Out-of-bounds falls back to the slot_set helper, whose invariant
    /// error path reports exactly as before. Store order within the pair is
    /// free: gc_arena collects only at safepoints, and none can intervene
    /// in straight-line native code.
    pub(super) fn emit_slot_store_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        idx: CVal,
        tag: i64,
        bits: CVal,
    ) {
        let (ptr, len) = self.emit_head(b, fx);
        let ok_bl = b.create_block();
        let slow_bl = b.create_block();
        let done_bl = b.create_block();
        let in_bounds = b.ins().icmp(IntCC::UnsignedLessThan, idx, len);
        b.ins().brif(in_bounds, ok_bl, &[], slow_bl, &[]);

        b.switch_to_block(ok_bl);
        let off = b.ins().imul_imm(idx, 16);
        let addr = b.ins().iadd(ptr, off);
        b.ins().store(MemFlagsData::trusted(), bits, addr, 8);
        let tag_v = b.ins().iconst(types::I64, tag);
        b.ins().store(MemFlagsData::trusted(), tag_v, addr, 0);
        b.ins().jump(done_bl, &[]);

        b.switch_to_block(slow_bl);
        let kind_v = b.ins().iconst(types::I64, tag);
        let sf = self.func_ref(b, self.helpers.slot_set);
        let call = b.ins().call(sf, &[fx.vm, fx.mc, idx, kind_v, bits]);
        let t = b.inst_results(call)[0];
        self.tag_check(b, fx, t);
        b.ins().jump(done_bl, &[]);

        b.switch_to_block(done_bl);
    }

    /// Bounds-checked native 16-byte slot copy: `slots[dst] = slots[src]`.
    pub(super) fn emit_slot_copy(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        dst: CVal,
        src: CVal,
    ) {
        let (ptr, len) = self.emit_head(b, fx);
        let ok_bl = b.create_block();
        let slow_bl = b.create_block();
        let done_bl = b.create_block();
        let d_ok = b.ins().icmp(IntCC::UnsignedLessThan, dst, len);
        let s_ok = b.ins().icmp(IntCC::UnsignedLessThan, src, len);
        let both = b.ins().band(d_ok, s_ok);
        b.ins().brif(both, ok_bl, &[], slow_bl, &[]);

        b.switch_to_block(ok_bl);
        let soff = b.ins().imul_imm(src, 16);
        let saddr = b.ins().iadd(ptr, soff);
        let stag = b.ins().load(types::I64, MemFlagsData::trusted(), saddr, 0);
        let sbits = b.ins().load(types::I64, MemFlagsData::trusted(), saddr, 8);
        let doff = b.ins().imul_imm(dst, 16);
        let daddr = b.ins().iadd(ptr, doff);
        b.ins().store(MemFlagsData::trusted(), sbits, daddr, 8);
        b.ins().store(MemFlagsData::trusted(), stag, daddr, 0);
        b.ins().jump(done_bl, &[]);

        b.switch_to_block(slow_bl);
        let kind_v = b.ins().iconst(types::I64, KIND_SLOT);
        let sf = self.func_ref(b, self.helpers.slot_set);
        let call = b.ins().call(sf, &[fx.vm, fx.mc, dst, kind_v, src]);
        let t = b.inst_results(call)[0];
        self.tag_check(b, fx, t);
        b.ins().jump(done_bl, &[]);

        b.switch_to_block(done_bl);
    }

    pub(super) fn abs_slot(&self, b: &mut FunctionBuilder, fx: &FnCtx, window_idx: u32) -> CVal {
        // D3b: any absolute slot computation makes the body slot-dependent —
        // it can never run under a W0 edge's poison base.
        self.uses_slot_base.set(true);
        b.ins().iadd_imm(fx.slot_base, i64::from(window_idx))
    }

    /// Encode an AV as `(kind, bits)` lanes for a helper call. May allocate a
    /// scratch slot (boxing `Nil` never needs one; scalars pass by value).
    pub(super) fn encode(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> (CVal, CVal) {
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
                self.uses_self_slot.set(true);
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
    pub(super) fn tag_check(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, tag: CVal) {
        let bad = b.ins().icmp_imm(IntCC::NotEqual, tag, 0);
        let bad_bl = b.create_block();
        let ok_bl = b.create_block();
        b.ins().brif(bad, bad_bl, &[], ok_bl, &[]);
        b.switch_to_block(bad_bl);
        let zero = self.zero_of(b, fx.ret);
        b.ins().jump(fx.exit, &[tag.into(), zero.into()]);
        b.switch_to_block(ok_bl);
    }

    pub(super) fn const_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        c: &Constant,
        ip: usize,
    ) -> Result<AV, Refusal> {
        Ok(match c {
            Constant::Int(i) => AV::C(b.ins().iconst(types::I64, *i), AotKind::Int),
            Constant::Double(d) => AV::C(b.ins().f64const(*d), AotKind::Double),
            Constant::Bool(x) => AV::C(b.ins().iconst(types::I8, *x as i64), AotKind::Bool),
            Constant::Nil => AV::Nil,
            Constant::String(s) => {
                // BUGS.md Finding 5: DYNAMIC `%{…}` interpolation (`%` sent
                // to a computed string) reads the CALLER's locals by walking
                // the frame env, which a compiled frame does not materialize
                // — every local silently read as nil. A method whose string
                // constants can be interpolation sources therefore stays
                // interpreted (refusal = semantics preserved). A `%'…'`
                // LITERAL never trips this: the compiler lowers it to a `+`
                // chain, so no `%{` survives into its string constants.
                if s.contains("%{") {
                    return Err(refuse(
                        RefusalKind::Structural,
                        "string constant contains a %{…} interpolation source \
                         (reads caller locals via the frame env, which compiled \
                         frames do not materialize)"
                            .to_string(),
                    ));
                }
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
            Constant::Block(rc) => {
                // B3b: materialize a real closure over a SNAPSHOT of the whole
                // frame environment (docs/internal/BLOCK_AOT_ARCH.md §3). Gated to
                // read-only captures, no `^^`, no nested literals, no guard
                // block — anything else still refuses.
                let rc = rc.clone();
                return self.materialize_closure(b, fx, vars, obj_params, &rc, ip);
            }
            _ => {
                return Err(refuse(
                    RefusalKind::UnsupportedConstant,
                    format!("unsupported constant at ip {ip}"),
                ));
            }
        })
    }

    pub(super) fn local_av(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        ip: usize,
    ) -> Result<AV, Refusal> {
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
            None if self.nil_deferred.contains(&sym) => {
                // F2: a READ of a still-deferred `var x = nil` — give it a
                // slot and just read it. Scratch slots are NIL-initialized at
                // invocation entry (`invoke`/`invoke_block`), and a non-loop
                // declaration executes at most once before any read (a
                // bytecode-order read before the decl compiles as LoadGlobal,
                // never here), so no site init is needed — and one WOULD
                // re-nil a live accumulator when the read sits inside a fused
                // loop (the `reduce:` shape this un-refuses).
                self.nil_deferred.remove(&sym);
                let w = self.alloc_scratch()?;
                let idx = self.abs_slot(b, fx, w);
                vars.insert(sym, VarSlot::Obj(w, None));
                Ok(AV::Dyn(idx))
            }
            None if self.cand.role == AotRole::BlockTemplate => {
                // B3a: a free variable — read the closure's real EnvFrame cell.
                self.emit_env_get(b, fx, sym)
            }
            None => Err(refuse(
                RefusalKind::LocalTyping,
                format!(
                    "read of unknown/uninitialized local '{}' at ip {ip}",
                    sym.as_str()
                ),
            )),
        }
    }

    #[allow(clippy::too_many_arguments)] // scalar-lowering helper threads codegen context
    pub(super) fn local_scalar(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        want: AotKind,
        ip: usize,
    ) -> Result<CVal, Refusal> {
        match self.local_av(b, fx, vars, obj_params, sym, ip)? {
            AV::C(v, k) if k == want => Ok(v),
            _ => Err(refuse(
                RefusalKind::LocalTyping,
                format!("local '{}' is not a {want:?} at ip {ip}", sym.as_str()),
            )),
        }
    }

    pub(super) fn store_local(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vars: &mut HashMap<Symbol, VarSlot>,
        obj_params: &HashMap<Symbol, CVal>,
        sym: Symbol,
        v: AV,
    ) -> Result<(), Refusal> {
        if obj_params.contains_key(&sym) || sym == self_symbol() {
            return Err(refuse(
                RefusalKind::LocalTyping,
                format!("store to parameter/self '{}'", sym.as_str()),
            ));
        }
        self.nil_deferred.remove(&sym);
        match v {
            AV::C(cv, k) => match vars.get(&sym) {
                Some(&VarSlot::Scalar(var, vk)) => {
                    if vk != k {
                        return Err(refuse(
                            RefusalKind::LocalTyping,
                            format!("local '{}' changes kind", sym.as_str()),
                        ));
                    }
                    b.def_var(var, cv);
                    Ok(())
                }
                Some(VarSlot::Obj(..)) => Err(refuse(
                    RefusalKind::LocalTyping,
                    format!("local '{}' changes kind", sym.as_str()),
                )),
                None => {
                    let var = b.declare_var(kind_type(k));
                    b.def_var(var, cv);
                    vars.insert(sym, VarSlot::Scalar(var, k));
                    Ok(())
                }
            },
            AV::Dyn(idx) if matches!(vars.get(&sym), Some(VarSlot::Scalar(..))) => {
                // Accumulator pattern: `total = total + (dynamic)` — narrow
                // the dynamic value back into the scalar local, checked. A
                // STATICALLY wider store (e.g. `x = x + 0.5`, now a devirted
                // Double) never reaches here: mixed-kind arithmetic yields a
                // typed AV::C, so the `AV::C` arm's kind-change refusal
                // demotes it (BUGS.md Finding 3). What remains here is a
                // genuinely dynamic value (an outcall result) whose kind is
                // usually the slot's; the runtime check catches the rare
                // mismatch.
                let Some(&VarSlot::Scalar(var, k)) = vars.get(&sym) else {
                    unreachable!()
                };
                if k != AotKind::Double && self.double_tainted.contains(&idx) {
                    // Finding 3 (f3b): a possibly-Double value into a
                    // non-Double scalar local — demote rather than
                    // runtime-narrow-error a legal untyped program.
                    return Err(refuse(
                        RefusalKind::LocalTyping,
                        format!(
                            "untyped scalar local '{}' may be reassigned a wider (Double) kind",
                            sym.as_str()
                        ),
                    ));
                }
                let val = self.narrow_to_scalar(b, fx, idx, k);
                b.def_var(var, val);
                Ok(())
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
                        return Err(refuse(
                            RefusalKind::LocalTyping,
                            format!("local '{}' changes kind", sym.as_str()),
                        ));
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
    pub(super) fn cold_send(
        insts: &[Instruction],
        target: usize,
    ) -> Result<(Symbol, i64), Refusal> {
        for inst in insts.iter().skip(target).take(8) {
            if let Some((sel, n, _)) = inst.send_parts() {
                return Ok((*sel, n as i64));
            }
        }
        // The proven-nil MNU stub must name the interpreter's EXACT selector
        // and arity; an unclassifiable cold path used to silently default to
        // ("if:", 1) — a wrong error message waiting for the first cold
        // shape this scan doesn't recognize. Refuse instead (the member runs
        // interpreted, which raises the real MNU).
        Err(refuse(
            RefusalKind::SlotResidency,
            format!("cold-path send at ip {target} not identifiable for the nil-MNU stub"),
        ))
    }

    /// Fill the lane buffers with encoded AVs.
    pub(super) fn fill_lanes(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        vals: &[AV],
    ) -> Result<(), Refusal> {
        if vals.len() > MAX_OUTCALL_ARGS {
            return Err(refuse(
                RefusalKind::ArityCap,
                "too many arguments for the compiled ABI".to_string(),
            ));
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
        ip: usize,
    ) -> Result<AV, Refusal> {
        self.emit_outcall_inner(b, fx, recv, selector, args, ip, true)
    }

    /// `emit_outcall` for sites whose target can never be a compiled entry
    /// (the devirt-op native fallbacks — `at:`/`at:put:` on a Map): no D2
    /// site is minted, so the helper skips the always-miss peek.
    pub(super) fn emit_outcall_nosite(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        selector: &str,
        args: &[AV],
        ip: usize,
    ) -> Result<AV, Refusal> {
        self.emit_outcall_inner(b, fx, recv, selector, args, ip, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_outcall_inner(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        selector: &str,
        args: &[AV],
        ip: usize,
        with_site: bool,
    ) -> Result<AV, Refusal> {
        let (rk, rb) = self.encode(b, fx, recv);
        self.fill_lanes(b, fx, args)?;
        let sym: &'static Symbol = Box::leak(Box::new(Symbol::intern(selector)));
        let sel = b.ins().iconst(types::I64, sym as *const Symbol as i64);
        let out = self.alloc_scratch()?;
        let out_idx = self.abs_slot(b, fx, out);
        let ka = b.ins().stack_addr(types::I64, fx.kinds_buf, 0);
        let ba = b.ins().stack_addr(types::I64, fx.bits_buf, 0);
        let argc = b.ins().iconst(types::I64, args.len() as i64);
        let (tid_v, _ip_v, len_v) = self.site_consts(b, ip);
        // D2: every dispatch-reachable outcall site gets a cell in
        // `VmState::aot_sites` (the "AOT IC") — a warm hit dispatches
        // straight to the compiled entry. The site id rides the ip lane's
        // high bits (see helpers::outcall) so the helper keeps its pre-D2
        // 12-argument ABI; `u32::MAX` = no site (never peek).
        let site = if with_site {
            let s = self
                .prior_sites
                .as_ref()
                .and_then(|m| m.get(&ip).copied())
                .unwrap_or_else(crate::codegen::next_outcall_site);
            self.site_log.push((ip, s));
            s
        } else {
            u32::MAX
        };
        let ip_site = b
            .ins()
            .iconst(types::I64, (ip as i64) | ((i64::from(site)) << 32));

        // D3b (docs/internal/DIRECT_CALLS_ARCH.md §3.4): a baked W0 site emits a
        // guarded DIRECT edge — live-epoch check (native), receiver+fiber
        // guard (mini-helper), then one uniform call_indirect straight into
        // the callee's raw entry, with the generic helper call as the guard-
        // miss path. Static preconditions: every arg is an SSA scalar whose
        // kind matches the baked callee's lane plan, so `bits_buf` already
        // IS the raw lane layout and no runtime arg guard exists at all.
        let baked = if with_site {
            self.baked.get(&ip).copied().filter(|bk| {
                args.len() == bk.entry.lane_plan.len()
                    && args.iter().zip(bk.entry.lane_plan.iter()).all(|(a, &pl)| {
                        matches!(
                            (a, pl as i64),
                            (AV::C(_, AotKind::Int), KIND_INT)
                                | (AV::C(_, AotKind::Double), KIND_DOUBLE)
                                | (AV::C(_, AotKind::Bool), KIND_BOOL)
                        )
                    })
            })
        } else {
            None
        };
        if let Some(bk) = baked {
            crate::codegen::TOTAL_DIRECT_SITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let generic_bl = b.create_block();
            let recv_bl = b.create_block();
            let direct_bl = b.create_block();
            let merge_bl = b.create_block();

            // 1. epoch guard: the live dispatch epoch (through the ABI's
            //    pointer) must equal the bake-time constant.
            let live = b
                .ins()
                .load(types::I64, MemFlagsData::trusted(), fx.epoch, 0);
            let want = b.ins().iconst(types::I64, bk.epoch as i64);
            let fresh = b.ins().icmp(IntCC::Equal, live, want);
            b.ins().brif(fresh, recv_bl, &[], generic_bl, &[]);

            // 2. receiver + fiber guard (the mini-helper reproduces
            //    entry_gates' fiber marking exactly).
            b.switch_to_block(recv_bl);
            let gk = b.ins().iconst(types::I64, i64::from(bk.recv_kind));
            let gp = b.ins().iconst(types::I64, bk.recv_ptr as i64);
            let gf = self.func_ref(b, self.helpers.guard_recv);
            let gcall = b.ins().call(gf, &[fx.vm, fx.mc, rk, rb, gk, gp]);
            let gok = b.inst_results(gcall)[0];
            b.ins().brif(gok, direct_bl, &[], generic_bl, &[]);

            // 3. the direct edge: one uniform call_indirect into the baked
            //    raw entry. slot_base = 0 poison (W0 never derefs it);
            //    outcall_nesting untouched (a flat native call adds no
            //    Rust-stack alternation).
            b.switch_to_block(direct_bl);
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
            let poison_base = b.ins().iconst(types::I64, 0);
            let ret_addr = b.ins().stack_addr(ptr_ty, fx.direct_ret, 0);
            let dcall = b.ins().call_indirect(
                raw_sig,
                fnaddr,
                &[
                    fx.vm,
                    fx.mc,
                    fx.fuel,
                    fx.depth,
                    fx.epoch,
                    fx.slots,
                    poison_base,
                    ba,
                    ret_addr,
                ],
            );
            let dtag = b.inst_results(dcall)[0];
            self.tag_check(b, fx, dtag);
            // Deliver the scalar into the site's out slot so both paths
            // yield the same AV::Dyn contract downstream.
            let retv = b.ins().stack_load(types::I64, fx.direct_ret, 0);
            let ret_kind = match bk.entry.ret {
                AotRet::Scalar(AotKind::Int) => KIND_INT,
                AotRet::Scalar(AotKind::Double) => KIND_DOUBLE,
                AotRet::Scalar(AotKind::Bool) => KIND_BOOL,
                AotRet::Obj => unreachable!("w0_eligible excludes Obj rets"),
            };
            let rkc = b.ins().iconst(types::I64, ret_kind);
            let sf = self.func_ref(b, self.helpers.slot_set);
            let scall = b.ins().call(sf, &[fx.vm, fx.mc, out_idx, rkc, retv]);
            let stag = b.inst_results(scall)[0];
            self.tag_check(b, fx, stag);
            b.ins().jump(merge_bl, &[]);

            // 4. guard miss: exactly today's generic helper call.
            b.switch_to_block(generic_bl);
            let f = self.func_ref(b, self.helpers.outcall);
            let call = b.ins().call(
                f,
                &[
                    fx.vm, fx.mc, tid_v, ip_site, len_v, rk, rb, sel, argc, ka, ba, out_idx,
                ],
            );
            let tag = b.inst_results(call)[0];
            self.tag_check(b, fx, tag);
            b.ins().jump(merge_bl, &[]);

            b.switch_to_block(merge_bl);
            return Ok(AV::Dyn(out_idx));
        }

        let f = self.func_ref(b, self.helpers.outcall);
        let call = b.ins().call(
            f,
            &[
                fx.vm, fx.mc, tid_v, ip_site, len_v, rk, rb, sel, argc, ka, ba, out_idx,
            ],
        );
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(out_idx))
    }

    /// The `(template_id, ip, bytecode-len)` constants identifying this send
    /// site — the interpreted send's own inline-cache identity, shared with it.
    pub(super) fn site_consts(&mut self, b: &mut FunctionBuilder, ip: usize) -> (CVal, CVal, CVal) {
        let tid = self.cand.block.template_id.unwrap_or(u32::MAX);
        (
            b.ins().iconst(types::I64, i64::from(tid)),
            b.ins().iconst(types::I64, ip as i64),
            b.ins()
                .iconst(types::I64, self.cand.block.bytecode.0.len() as i64),
        )
    }

    /// A send site: direct native call when the callee is a scalar-pure
    /// sibling and the receiver is `self`; otherwise an outcall.
    pub(super) fn emit_send(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        recv: AV,
        sel: Symbol,
        args: &[AV],
        ip: usize,
    ) -> Result<AV, Refusal> {
        // Sealed scalar operators devirtualize when both operands PROVE
        // scalar (S2): Integer/Double are startup-sealed, so `Int +: Int` is
        // frozen semantics — the same guarantee the compiler's typed devirt
        // uses. Anything unproven falls through to the outcall.
        if args.len() == 1
            && let Some(kind) = IntBinKind::from_selector(sel.as_str())
        {
            match (recv, args[0]) {
                (AV::C(a, AotKind::Int), AV::C(c, AotKind::Int)) => {
                    return self.emit_int_bin(b, fx, kind, a, c);
                }
                (AV::C(a, AotKind::Double), AV::C(c, AotKind::Double)) => {
                    return Ok(self.emit_double_bin(b, kind, a, c));
                }
                // Mixed Int/Double: sealed numeric promotion (`100 + 0.5` is
                // Double). Devirt to the double op so the RESULT is a typed
                // AV::C(Double) — correct semantics, a perf win, and it makes
                // `x = x + 0.5` a statically-visible kind change that the
                // store's refusal demotes instead of a runtime narrow error
                // (BUGS.md Finding 3).
                (AV::C(a, AotKind::Int), AV::C(c, AotKind::Double)) => {
                    let ad = b.ins().fcvt_from_sint(types::F64, a);
                    return Ok(self.emit_double_bin(b, kind, ad, c));
                }
                (AV::C(a, AotKind::Double), AV::C(c, AotKind::Int)) => {
                    let cd = b.ins().fcvt_from_sint(types::F64, c);
                    return Ok(self.emit_double_bin(b, kind, a, cd));
                }
                _ => {}
            }
        }
        let key = (self.cand.group_id, sel.as_str().to_string());
        // An OPEN owner (B2) never emits direct calls — EXCEPT to itself
        // (S2): the entry records the redefinition epoch and `invoke` Bails
        // the whole method to the interpreter once ANY method table mutates,
        // so a stale direct recursion is never entered. Non-self sends keep
        // the outcall (dispatch-equivalent) seam.
        let own_tid = self.cand.block.template_id;
        if matches!(recv, AV::SelfRef)
            && let Some((psig, pret, callee_tid)) = self.siblings.get(&key)
            && (!self.cand.open_owner || Some(*callee_tid) == own_tid)
            && self.pure.contains(callee_tid)
            && psig.len() == args.len()
        {
            // Direct call. Scalar-pure callee: args must be exact scalars.
            let mut ok = true;
            let mut call_args = vec![
                fx.vm,
                fx.mc,
                fx.fuel,
                fx.depth,
                fx.epoch,
                fx.slots,
                fx.slot_base,
            ];
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
                if self.cand.open_owner {
                    self.used_direct_self = true;
                }
                // A self-call's ret may have been DEMOTED this retry; the
                // sibling map still holds the pre-demotion signature.
                let effective = if Some(*callee_tid) == own_tid {
                    self.eff_ret
                } else {
                    *pret
                };
                let callee_fid = self.inner_ids[callee_tid];
                let callee = self.func_ref(b, callee_fid);
                let call = b.ins().call(callee, &call_args);
                let res = b.inst_results(call).to_vec();
                let (tag, val) = (res[0], res[1]);
                self.tag_check(b, fx, tag);
                let AotRet::Scalar(rk) = effective else {
                    return Err(refuse(
                        RefusalKind::LocalTyping,
                        format!("pure sibling with non-scalar ret at ip {ip}"),
                    ));
                };
                return Ok(AV::C(val, rk));
            }
        }
        let out = self.emit_outcall(b, fx, recv, sel.as_str(), args, ip)?;
        // Double-taint propagation (Finding 3, f3b): a numeric-operator send
        // whose result is a Dyn taints it when an operand is Double or
        // already tainted — the runtime value may be Double.
        if let AV::Dyn(out_idx) = out
            && IntBinKind::from_selector(sel.as_str()).is_some()
        {
            let operand_double = |t: &Self, v: &AV| match v {
                AV::C(_, AotKind::Double) => true,
                AV::Dyn(i) => t.double_tainted.contains(i),
                _ => false,
            };
            if operand_double(self, &recv) || args.iter().any(|a| operand_double(self, a)) {
                self.double_tainted.insert(out_idx);
            }
        }
        Ok(out)
    }

    /// Checked narrow of a slot-resident value to a scalar kind: peek the
    /// kind, extract on match, raise a clear TypeError otherwise (the one
    /// deliberate divergence — a wrong dynamic type surfaces at the annotation
    /// or accumulator that expected the scalar, instead of corrupting later).
    pub(super) fn narrow_to_scalar(
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
    pub(super) fn emit_return(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        v: AV,
    ) -> Result<(), Refusal> {
        let tag0 = b.ins().iconst(types::I8, 0);
        match (fx.ret, v) {
            (AotRet::Scalar(want), AV::C(cv, k)) if k == want => {
                b.ins().jump(fx.exit, &[tag0.into(), cv.into()]);
            }
            // A SPECULATED scalar ret must be statically provable on every
            // return path — no runtime narrowing whose failure would raise an
            // error the interpreter wouldn't. Demote to Obj and retry.
            (AotRet::Scalar(_), _) if self.cand.spec_ret => {
                self.pending_abort = Some(TranslateAbort::RetDemote);
                return Err("speculated scalar return not provable on this path".into());
            }
            (AotRet::Scalar(want), AV::Dyn(idx)) => {
                let val = self.narrow_to_scalar(b, fx, idx, want);
                b.ins().jump(fx.exit, &[tag0.into(), val.into()]);
            }
            (AotRet::Scalar(_), _) => {
                return Err(refuse(
                    RefusalKind::LocalTyping,
                    "return value does not match the declared scalar type".to_string(),
                ));
            }
            (AotRet::Obj, v) => {
                let idx = match v {
                    AV::Dyn(idx) => idx,
                    AV::SelfRef => {
                        self.uses_self_slot.set(true);
                        self.abs_slot(b, fx, 0)
                    }
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

    pub(super) fn pop_kind(stack: &mut Vec<AV>, want: AotKind) -> Result<CVal, Refusal> {
        match stack.pop() {
            Some(AV::C(v, k)) if k == want => Ok(v),
            Some(AV::C(_, k)) => Err(format!("operand kind {k:?}, wanted {want:?}").into()),
            Some(_) => Err("non-scalar operand where a scalar was proven".into()),
            None => Err("stack underflow".into()),
        }
    }

    /// Box `Nil`/`SelfRef` stack entries into slots so they can cross a block
    /// boundary as jump arguments (a statement-position inlined `if:` joins an
    /// arm value with the nil of the not-taken path). Scalars and slot values
    /// pass through untouched.
    fn assert_no_pending_writebacks(&self, stack: &[AV], where_: &str) -> Result<(), Refusal> {
        for v in stack {
            if let AV::Dyn(idx) = v
                && self.pending_writebacks.contains_key(idx)
            {
                return Err(refuse(
                    RefusalKind::WriteCapture,
                    format!("write-captured closure crosses a block boundary ({where_})"),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn norm_stack(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, Refusal> {
        self.assert_no_pending_writebacks(stack, "norm_stack")?;
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

    pub(super) fn stack_args(stack: &[AV]) -> Result<Vec<BlockArg>, Refusal> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(cv, _) => Ok((*cv).into()),
                AV::Dyn(idx) => Ok((*idx).into()),
                _ => Err("self/nil live at block boundary".into()),
            })
            .collect()
    }

    fn stack_bkinds(stack: &[AV]) -> Result<Vec<BKind>, Refusal> {
        stack
            .iter()
            .map(|v| match v {
                AV::C(_, k) => Ok(BKind::S(*k)),
                AV::Dyn(_) => Ok(BKind::Dyn),
                _ => Err("self/nil live at block boundary".into()),
            })
            .collect()
    }

    pub(super) fn block_for(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        blocks: &mut HashMap<usize, (CBlock, Vec<BKind>)>,
        work: &mut Vec<usize>,
        ip: usize,
        stack: &mut [AV],
    ) -> Result<(CBlock, Vec<BKind>), Refusal> {
        // A merge FORCED to all-Dyn by an earlier retry (mixed scalar/Dyn
        // predecessors, S3): box scalars before shape computation, so every
        // predecessor unifies. The interpreted value world is uniform —
        // boxing is exact, only the abstraction loses precision.
        if self.dyn_merges.contains(&ip) {
            for slot in stack.iter_mut() {
                if let AV::C(..) = *slot {
                    *slot = self.box_av(b, fx, *slot)?;
                }
            }
        }
        let kinds = Self::stack_bkinds(stack)?;
        if let Some((bl, expect)) = blocks.get(&ip) {
            let (bl, expect) = (*bl, expect.clone());
            if expect != kinds {
                if expect.len() != kinds.len() {
                    return Err(format!(
                        "stack shape mismatch at merge ip {ip}: {expect:?} vs {kinds:?}"
                    )
                    .into());
                }
                for i in 0..kinds.len() {
                    match (&expect[i], &kinds[i]) {
                        (a, bk) if a == bk => {}
                        // Box toward an existing Dyn expectation, in place.
                        (BKind::Dyn, BKind::S(_)) => {
                            stack[i] = self.box_av(b, fx, stack[i])?;
                        }
                        // The merge was first planned scalar; a Dyn (or a
                        // different scalar) predecessor needs it re-planned
                        // all-Dyn — signal the retry.
                        _ => {
                            self.pending_abort = Some(TranslateAbort::MergeDemote(ip));
                            return Err(format!("merge at ip {ip} re-planned as Dyn").into());
                        }
                    }
                }
            }
            return Ok((bl, expect));
        }
        let bl = b.create_block();
        for &k in &kinds {
            b.append_block_param(bl, bkind_type(k));
        }
        blocks.insert(ip, (bl, kinds.clone()));
        work.push(ip);
        Ok((bl, kinds))
    }

    /// Box any AV into a fresh scratch slot (merge-shape unification).
    fn box_av(&mut self, b: &mut FunctionBuilder, fx: &FnCtx, v: AV) -> Result<AV, Refusal> {
        let slot = self.alloc_scratch()?;
        let dst = self.abs_slot(b, fx, slot);
        let (k, bits) = self.encode(b, fx, v);
        let f = self.func_ref(b, self.helpers.slot_set);
        let call = b.ins().call(f, &[fx.vm, fx.mc, dst, k, bits]);
        let tag = b.inst_results(call)[0];
        self.tag_check(b, fx, tag);
        Ok(AV::Dyn(dst))
    }

    pub(super) fn zero_of(&self, b: &mut FunctionBuilder, r: AotRet) -> CVal {
        match r {
            AotRet::Scalar(AotKind::Int) | AotRet::Obj => b.ins().iconst(types::I64, 0),
            AotRet::Scalar(AotKind::Double) => b.ins().f64const(0.0),
            AotRet::Scalar(AotKind::Bool) => b.ins().iconst(types::I8, 0),
        }
    }

    pub(super) fn bail(&self, b: &mut FunctionBuilder, fx: &FnCtx, tag: u8) {
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
    pub(super) fn emit_fuel_tick(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        stack: &[AV],
    ) -> Result<Vec<AV>, Refusal> {
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

    pub(super) fn emit_fuel_tick_empty(&mut self, b: &mut FunctionBuilder, fx: &FnCtx) {
        self.emit_fuel_tick(b, fx, &[])
            .expect("empty-stack tick cannot fail");
    }

    /// Integer ops with `devirt_ops::int_bin` semantics: overflow raises (a cold bail to
    /// `TAG_INT_OVERFLOW`), it does not wrap — the `*_overflow` instructions hand back the
    /// flag the hardware already computes, so the hot path pays one never-taken branch.
    pub(super) fn emit_int_bin(
        &mut self,
        b: &mut FunctionBuilder,
        fx: &FnCtx,
        kind: IntBinKind,
        a: CVal,
        rb: CVal,
    ) -> Result<AV, Refusal> {
        use IntBinKind::*;
        // `res = a <op> b`, bailing to the overflow tag when the flag is set.
        let checked = |b: &mut FunctionBuilder, res: CVal, of: CVal| -> AV {
            let of_bl = b.create_block();
            let cont = b.create_block();
            b.ins().brif(of, of_bl, &[], cont, &[]);
            b.switch_to_block(of_bl);
            self.bail(b, fx, TAG_INT_OVERFLOW);
            b.switch_to_block(cont);
            AV::C(res, AotKind::Int)
        };
        let out = match kind {
            Add => {
                let (res, of) = b.ins().sadd_overflow(a, rb);
                checked(b, res, of)
            }
            Sub => {
                let (res, of) = b.ins().ssub_overflow(a, rb);
                checked(b, res, of)
            }
            Mul => {
                let (res, of) = b.ins().smul_overflow(a, rb);
                checked(b, res, of)
            }
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
                    // The one overflowing quotient: `i64::MIN / -1`. Negation overflows
                    // exactly when `a == i64::MIN`, so check that rather than the result.
                    let is_min = b.ins().icmp_imm(IntCC::Equal, a, i64::MIN);
                    let min_bl = b.create_block();
                    let neg_bl = b.create_block();
                    b.ins().brif(is_min, min_bl, &[], neg_bl, &[]);
                    b.switch_to_block(min_bl);
                    self.bail(b, fx, TAG_INT_OVERFLOW);
                    b.switch_to_block(neg_bl);
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
    pub(super) fn emit_double_bin(
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
