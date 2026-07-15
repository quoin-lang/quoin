//! The compile driver: `compile_all` over candidate sibling groups with the
//! typed demote-retry protocol (`TranslateAbort`), reachability, and the
//! scalar-pure (direct-callable) subset analysis.

use super::*;

/// Tag a refusal with its `VM.stats` bucket; the message stays free-form.
pub(super) fn refuse(kind: RefusalKind, why: String) -> Refusal {
    Refusal { kind, why }
}

pub(super) type SiblingMap = HashMap<(u32, String), (Vec<AotParam>, AotRet, u32)>;

/// Why a member's translation attempt stopped — a TYPED protocol between
/// the translator and `compile_all`'s retry loop. The demote variants are
/// RETRY instructions, never user-facing refusals; they used to ride
/// in-band as magic strings in the same `Err(String)` as refusal reasons,
/// matched by `==`/prefix-parse, where one context-adding `.map_err` on the
/// propagation path silently downgraded a retry into a permanent refusal.
/// They now travel out-of-band (`Translator::pending_abort`, set at the
/// same moment the aborting `Err` is returned), so the refusal strings stay
/// free-form messages.
pub(super) enum TranslateAbort {
    /// A real refusal: the member runs interpreted; the [`Refusal`] carries
    /// the `VM.stats` bucket plus the human-readable reason (surfaced under
    /// `QN_AOT_VERBOSE`).
    Refuse(Refusal),
    /// The member used the slot window while marked scalar-pure — demote it
    /// from the direct-call set and retry.
    PurityDemote,
    /// A SPECULATED scalar return hit a return path the translator can't
    /// prove scalar — retry with the ret demoted to Obj (S2). Never used
    /// for annotated returns, whose checked-narrow divergence is deliberate.
    RetDemote,
    /// A merge point first planned SCALAR sees a Dyn predecessor (S3) —
    /// retry with the merge at this ip FORCED to Dyn from the start
    /// (scalars box on entry).
    MergeDemote(usize),
}

/// Compile every group; members are refused individually. Returns the
/// registered entries and refusals `(selector, reason)`.
#[allow(clippy::type_complexity)]
pub(crate) fn compile_all(
    cands: &[AotCandidate],
    siblings: &SiblingMap,
) -> (
    Vec<(u32, AotEntry, Vec<(usize, u32)>)>,
    Vec<(String, Refusal)>,
) {
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
        // small; worst-case quadratic compile cost is trivial — as are the
        // per-attempt `Box::leak`ed selector/name/template constants of a
        // FAILED attempt (bounded by the monotone retry sets; a successful
        // attempt's leaks are load-bearing, referenced by the compiled code
        // for the process lifetime).
        let mut demoted: HashSet<u32> = HashSet::new();
        let mut ret_demoted: HashSet<u32> = HashSet::new();
        let mut dyn_merges: HashMap<u32, HashSet<usize>> = HashMap::new();
        loop {
            if active.is_empty() {
                break;
            }
            match compile_group(&active, siblings, &demoted, &ret_demoted, &dyn_merges) {
                Ok(mut entries) => {
                    compiled.append(&mut entries);
                    break;
                }
                Err((failed_tid, TranslateAbort::PurityDemote)) => {
                    demoted.insert(failed_tid);
                }
                Err((failed_tid, TranslateAbort::RetDemote)) => {
                    ret_demoted.insert(failed_tid);
                }
                Err((failed_tid, TranslateAbort::MergeDemote(ip))) => {
                    dyn_merges.entry(failed_tid).or_default().insert(ip);
                }
                Err((failed_tid, TranslateAbort::Refuse(reason))) => {
                    if let Some(i) = active
                        .iter()
                        .position(|c| c.block.template_id == Some(failed_tid))
                    {
                        refused.push((active[i].selector.clone(), reason));
                        active.remove(i);
                    } else {
                        // A failure not attributable to one member (a
                        // candidate without a template id, or a module-level
                        // error blamed on a placeholder tid): refuse the
                        // whole group instead of panicking on a member
                        // lookup that cannot succeed.
                        for c in active.drain(..) {
                            refused.push((c.selector.clone(), reason.clone()));
                        }
                    }
                }
            }
        }
    }
    (compiled, refused)
}

/// Instruction reachability from the entry, following jumps — the pure-set
/// scan's model of which instructions a PURE translation would visit. Edge
/// policy mirrors the translator:
///
/// - `BranchIfNotBool` follows only the fall-through edge. In a member with
///   no slot sources the operand is always a translation-time constant, so
///   the guard either folds away (a provably-Bool condition — untyped fib's
///   speculated `n <= 1` — takes the hot edge and the cold span is dead) or
///   pins the cold edge, whose slot ops then trip the translation purity
///   check and demote the member — the same verdict either way, decided by
///   the authority.
/// - Unknown or future instructions default to plain fall-through. Every
///   inaccuracy here only ADMITS too much: the translation-time purity check
///   is the soundness backstop for every admission the scan makes, and the
///   cost of a wrong admission is one demote-retry compile, never a wrong
///   program.
pub(super) fn reachable_ips(insts: &[Instruction]) -> Vec<bool> {
    let mut seen = vec![false; insts.len()];
    let mut work = vec![0usize];
    while let Some(ip) = work.pop() {
        if ip >= insts.len() || std::mem::replace(&mut seen[ip], true) {
            continue;
        }
        let mut succ = |o: isize| {
            if let Some(t) = ip.checked_add_signed(o) {
                work.push(t);
            }
        };
        match &insts[ip] {
            Instruction::Jump(o) => succ(*o),
            Instruction::IfJump(o) | Instruction::ElseJump(o) => {
                succ(*o);
                succ(1);
            }
            Instruction::BranchIfNotBool(_) => succ(1),
            Instruction::BranchIfNotList(o, _) | Instruction::BranchIfNotPlainNew(o) => {
                succ(*o);
                succ(1);
            }
            Instruction::Return | Instruction::MethodReturn | Instruction::BlockReturn => {}
            _ => succ(1),
        }
    }
    seen
}

/// The scalar-pure subset of a group: all-scalar signatures whose bodies stay
/// in the scalar instruction set and send only to other scalar-pure siblings.
/// These keep the direct native-call path; everything else outcalls.
pub(super) fn scalar_pure_set(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
    ret_demoted: &HashSet<u32>,
) -> HashSet<u32> {
    let mut pure: HashSet<u32> = members
        .iter()
        .filter(|c| {
            c.params.iter().all(|p| matches!(p, AotParam::Scalar(_)))
                && matches!(eff_ret(c, ret_demoted), AotRet::Scalar(_))
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
            // The scan checks only instructions a pure translation would
            // VISIT (`reachable_ips`), so a guard's dead cold span — F1's
            // strict-Boolean conditionals re-materialize their arm blocks and
            // re-dispatch the real send there — cannot evict the member. A
            // reachability-blind version of this scan shipped and cost
            // untyped fib 8x: eviction killed direct self-recursion, which
            // made its speculated scalar return unprovable, which demoted it
            // to an Obj ret.
            let insts = &c.block.bytecode.0;
            let live = reachable_ips(insts);
            let ok = insts.iter().enumerate().all(|(i, inst)| match inst {
                _ if !live[i] => true,
                // The strict-Boolean guards themselves cost nothing in a pure
                // member: with no slot sources the operand is a translation
                // constant, so they fold (see `reachable_ips`).
                Instruction::BranchIfNotBool(..) | Instruction::RequireBool => true,
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
                | Instruction::BlockReturn
                | Instruction::MethodReturn => true,
                inst => match inst.send_parts() {
                    // A sealed scalar operator devirtualizes at translation
                    // (S2) when its operands prove scalar — optimistically
                    // pure here; a member that still needs an outcall trips
                    // the translation purity check and demotes. Via the
                    // exhaustive `send_parts` — but `SendField` stays OUT of
                    // the pure set deliberately: its field read needs a slot,
                    // so admission would only demote-retry back out, paying
                    // a wasted compile attempt per such member.
                    Some((sel, _, _)) if !matches!(inst, Instruction::SendField(..)) => {
                        IntBinKind::from_selector(sel.as_str()).is_some()
                            || siblings
                                .get(&(c.group_id, sel.as_str().to_string()))
                                .is_some_and(|(_, _, callee)| pure.contains(callee))
                    }
                    _ => false,
                },
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

/// Compile one attempt at a group. `Err((template_id, reason))` names the
/// member to refuse before retrying.
pub(super) fn eff_ret(c: &AotCandidate, ret_demoted: &HashSet<u32>) -> AotRet {
    match c.block.template_id {
        Some(tid) if ret_demoted.contains(&tid) => AotRet::Obj,
        _ => c.ret,
    }
}

#[allow(clippy::type_complexity)] // per-tid (entry, direct-call fixups) result rows
pub(super) fn compile_group(
    members: &[&AotCandidate],
    siblings: &SiblingMap,
    demoted: &HashSet<u32>,
    ret_demoted: &HashSet<u32>,
    dyn_merges: &HashMap<u32, HashSet<usize>>,
) -> Result<Vec<(u32, AotEntry, Vec<(usize, u32)>)>, (u32, TranslateAbort)> {
    let fail = |tid: u32, e: Refusal| (tid, TranslateAbort::Refuse(e));
    let any_tid = members[0].block.template_id.unwrap_or(0);

    let mut flags = settings::builder();
    flags
        .set("opt_level", "speed")
        .map_err(|e| fail(any_tid, e.to_string().into()))?;
    let isa = cranelift_native::builder()
        .map_err(|e| fail(any_tid, e.to_string().into()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| fail(any_tid, e.to_string().into()))?;
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, addr) in helper_symbols() {
        jb.symbol(name, addr);
    }
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let helpers = declare_helpers(&mut module, ptr).map_err(|e| fail(any_tid, e))?;
    let mut pure = scalar_pure_set(members, siblings, ret_demoted);
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
        let sig = inner_sig(&mut module, ptr, m, eff_ret(m, ret_demoted));
        let fid = module
            .declare_function(&format!("t{tid}"), Linkage::Local, &sig)
            .map_err(|e| fail(tid, e.to_string().into()))?;
        inner_ids.insert(tid, fid);
    }

    let mut fb_ctx = FunctionBuilderContext::new();
    #[allow(clippy::type_complexity)]
    let mut tramp_ids: Vec<(
        u32,
        FuncId,
        &AotCandidate,
        u32,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        Vec<(usize, u32)>,
    )> = Vec::new();

    for m in members {
        let tid = m.block.template_id.unwrap();
        if std::env::var("QN_AOT_DUMP").is_ok_and(|v| v == m.selector) {
            eprintln!(
                "=== bytecode {} (tid {tid}; pure={}, ret={:?}, spec_ret={}, open={}) ===",
                m.selector,
                pure.contains(&tid),
                eff_ret(m, ret_demoted),
                m.spec_ret,
                m.open_owner
            );
            for (i, inst) in m.block.bytecode.0.iter().enumerate() {
                eprintln!("  {i:3}: {inst:?}");
            }
        }
        let mut ctx = module.make_context();
        ctx.func.signature = inner_sig(&mut module, ptr, m, eff_ret(m, ret_demoted));
        let n_scratch;
        let needs_list_self;
        let direct_self;
        let materializes_nlr;
        let materializes;
        let uses_slot_base;
        let uses_self_slot;
        let site_log;
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            static EMPTY_MERGES: std::sync::OnceLock<HashSet<usize>> = std::sync::OnceLock::new();
            let mut tr = Translator {
                module: &mut module,
                cand: m,
                eff_ret: eff_ret(m, ret_demoted),
                used_direct_self: false,
                dyn_merges: dyn_merges
                    .get(&tid)
                    .unwrap_or_else(|| EMPTY_MERGES.get_or_init(HashSet::new)),
                siblings,
                inner_ids: &inner_ids,
                pure: &pure,
                helpers: &helpers,
                is_pure: pure.contains(&tid),
                next_scratch: 0,
                proofs: HashMap::new(),
                needs_list_self: false,
                nil_deferred: HashSet::new(),

                pending_writebacks: HashMap::new(),
                materialized: HashSet::new(),
                materialized_nlr: HashSet::new(),
                pending_abort: None,
                uses_slot_base: std::cell::Cell::new(false),
                uses_self_slot: std::cell::Cell::new(false),
                baked: crate::codegen::take_baked_for(tid),
                double_tainted: std::collections::HashSet::new(),
                prior_sites: crate::codegen::prior_sites_for(tid),
                site_log: Vec::new(),
            };
            if let Err(e) = tr.build_inner(&mut b) {
                // A demote signal set alongside the aborting Err travels
                // out-of-band — the message string is free-form and may be
                // wrapped without breaking the retry protocol.
                let abort = tr.pending_abort.take().unwrap_or(TranslateAbort::Refuse(e));
                return Err((tid, abort));
            }
            n_scratch = tr.next_scratch;
            needs_list_self = tr.needs_list_self;
            direct_self = tr.used_direct_self;
            materializes_nlr = !tr.materialized_nlr.is_empty();
            materializes = !tr.materialized.is_empty();
            uses_slot_base = tr.uses_slot_base.get();
            uses_self_slot = tr.uses_self_slot.get();
            site_log = std::mem::take(&mut tr.site_log);
            b.seal_all_blocks();
            b.finalize();
        }
        let fid = inner_ids[&tid];
        module
            .define_function(fid, &mut ctx)
            .map_err(|e| fail(tid, format!("{e:?}\nIR:\n{}", ctx.func.display()).into()))?;
        if std::env::var("QN_AOT_DUMP").is_ok_and(|v| v == m.selector || v == "1") {
            eprintln!("=== {} (tid {tid}) ===\n{}", m.selector, ctx.func.display());
        }

        let mut tctx = module.make_context();
        tctx.func.signature = tramp_sig(&mut module, ptr);
        let tramp_id = module
            .declare_function(
                &format!("t{tid}_tramp"),
                Linkage::Local,
                &tctx.func.signature,
            )
            .map_err(|e| fail(tid, e.to_string().into()))?;
        {
            let mut b = FunctionBuilder::new(&mut tctx.func, &mut fb_ctx);
            build_trampoline(&mut module, &mut b, m, fid, eff_ret(m, ret_demoted));
            b.seal_all_blocks();
            b.finalize();
        }
        module
            .define_function(tramp_id, &mut tctx)
            .map_err(|e| fail(tid, e.to_string().into()))?;
        tramp_ids.push((
            tid,
            tramp_id,
            m,
            n_scratch,
            needs_list_self,
            direct_self,
            materializes_nlr,
            materializes,
            uses_slot_base,
            uses_self_slot,
            site_log,
        ));
    }

    module
        .finalize_definitions()
        .map_err(|e| fail(any_tid, e.to_string().into()))?;
    let mut out = Vec::new();
    for (
        tid,
        tramp_id,
        m,
        n_scratch,
        needs_list_self,
        direct_self,
        materializes_nlr,
        materializes,
        uses_slot_base,
        uses_self_slot,
        site_log,
    ) in tramp_ids
    {
        let addr = module.get_finalized_function(tramp_id);
        let raw: AotRawFn = unsafe { std::mem::transmute(addr) };
        out.push((
            tid,
            AotEntry {
                raw,
                params: m.params.clone().into_boxed_slice(),
                ret: eff_ret(m, ret_demoted),
                n_scratch,
                needs_list_self,
                role: m.role,
                template_id: tid,
                selector: m.selector.clone(),
                param_preconditions: m.spec_preconditions.clone().into_boxed_slice(),
                spec_bails: std::sync::atomic::AtomicU32::new(0),
                direct_self,
                compile_epoch: crate::codegen::redef_epoch(),
                materializes_nlr,
                materializes,
                uses_slot_base,
                uses_self_slot,
                is_closed: crate::instruction::template_is_closed(&m.block),
                lane_plan: crate::codegen::build_lane_plan(&m.params, &m.spec_preconditions),
            },
            site_log,
        ));
    }
    // The code must live for the process (fn pointers are registered
    // globally): leak the module, same append-only lifetime as the interner.
    std::mem::forget(module);
    Ok(out)
}
