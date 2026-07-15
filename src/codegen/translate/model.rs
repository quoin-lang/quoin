//! The abstract-value model (`AV`/`BKind`/`VarSlot`/`DynProof`), the `Translator`
//! state itself, the closure-nest gate scan, and the per-function native context.

use super::*;

/// An abstract stack slot.
#[derive(Clone, Copy)]
pub(super) enum AV {
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
pub(super) enum BKind {
    S(AotKind),
    Dyn,
}

pub(super) fn bkind_type(k: BKind) -> Type {
    match k {
        BKind::S(k) => kind_type(k),
        BKind::Dyn => types::I64,
    }
}

/// A named local's storage.
#[derive(Clone, Copy)]
pub(super) enum VarSlot {
    Scalar(Variable, AotKind),
    /// Scratch-slot number in the frame window, plus what the translator can
    /// PROVE about the value it holds (tag-backed; docs/internal/GENERICS_ARCH.md §8).
    Obj(u32, Option<DynProof>),
}

/// A guarantee the translator carries for a slot-resident dynamic value —
/// only from sources the runtime enforces (a `TagCollection` it emitted, or
/// an element read from such a collection). Never from checker beliefs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DynProof {
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

pub(super) struct Translator<'a> {
    pub(super) module: &'a mut JITModule,
    pub(super) cand: &'a AotCandidate,
    /// The ret this member ACTUALLY compiles with: the candidate's, or Obj
    /// after a speculated-scalar demotion retry (S2).
    pub(super) eff_ret: AotRet,
    /// A direct self-recursion call was emitted (S2): the entry records the
    /// redefinition epoch and `invoke` Bails when it goes stale.
    pub(super) used_direct_self: bool,
    pub(super) siblings: &'a SiblingMap,
    pub(super) inner_ids: &'a HashMap<u32, FuncId>,
    pub(super) pure: &'a HashSet<u32>,
    pub(super) helpers: &'a Helpers,
    /// This member is in the scalar-pure set (direct-callable): it must not
    /// touch the slot window, because direct callers pass their own base.
    pub(super) is_pure: bool,
    pub(super) next_scratch: u32,
    /// Proofs for in-flight `AV::Dyn` values, keyed by their SSA index value.
    /// Values that cross a control-flow join (block params) drop their proofs
    /// — a sound degradation; the load-bearing flows (element read → inlined
    /// conditional) stay within one block, and locals carry proofs in
    /// `VarSlot::Obj` across blocks.
    pub(super) proofs: HashMap<CVal, DynProof>,
    /// Set when a fused-`each:` guard on `self` compiled hot-path-only (B2):
    /// becomes the entry's `needs_list_self` precondition.
    pub(super) needs_list_self: bool,
    /// Merge ips FORCED to all-Dyn shapes (S3 retry): scalars box on entry,
    /// so predecessors with mixed shapes unify.
    pub(super) dyn_merges: &'a HashSet<usize>,
    /// `var x = nil` declarations whose slot type is still DEFERRED to the
    /// first store. A closure materialization forces these into Obj slots
    /// first — see the DefineLocal arm.
    pub(super) nil_deferred: HashSet<Symbol>,
    /// Frame locals a materialized closure WRITES (through its snapshot env),
    /// keyed by the closure's slot-index SSA value: after the consuming send
    /// returns, each is read back from the snapshot into the frame local, so
    /// `count:`-style `{ n = n + 1 }` cold arms stay exact (B3b).
    pub(super) pending_writebacks: HashMap<CVal, Vec<(Symbol, VarSlot)>>,
    /// Every materialized closure's slot value: a send consuming TWO OR MORE
    /// of these where any writes a capture must refuse — sibling snapshots
    /// are INDEPENDENT envs, but interpreted siblings share one cell (the
    /// unfused-`whileDo:` bug: the body's `i` advanced while the condition's
    /// stayed frozen).
    pub(super) materialized: HashSet<CVal>,
    /// Out-of-band demote signal (see [`TranslateAbort`]): set at the same
    /// moment the aborting `Err` is returned, consumed by `compile_group`.
    pub(super) pending_abort: Option<TranslateAbort>,
    /// D3b: the body computed an absolute slot index (see
    /// `AotEntry::uses_slot_base`). Cell: `abs_slot` takes `&self`.
    pub(super) uses_slot_base: std::cell::Cell<bool>,
    /// Window-hoist: the body read SLOT 0 (`self`) specifically — a baked
    /// block edge provides a real hoisted window but never writes its
    /// self slot, so slot-0 readers are ineligible.
    pub(super) uses_self_slot: std::cell::Cell<bool>,
    /// D3b: baked direct-edge facts per ip, present only on a
    /// retranslation whose drain staged them.
    pub(super) baked: rustc_hash::FxHashMap<usize, crate::codegen::BakedW0>,
    /// BUGS.md Finding 3 (f3b): slot-resident Dyn results of arithmetic
    /// that involved a Double operand — i.e. values that could be Double at
    /// runtime. Storing one into an Int-slotted untyped local would
    /// runtime-narrow-error a legal program, so such a store REFUSES
    /// (demotes) instead. A clean Dyn (e.g. an `add:to:` result with no
    /// Double anywhere) stays the checked narrow.
    pub(super) double_tainted: std::collections::HashSet<CVal>,
    /// D3a: site ids from this tid's FIRST translation — a retranslation
    /// must reuse them (the D2 cells key on them; the generic fallback and
    /// interpreted IC stay warm through the swap).
    pub(super) prior_sites: Option<rustc_hash::FxHashMap<usize, u32>>,
    /// Every (ip, site) this translation minted or reused, for retention.
    pub(super) site_log: Vec<(usize, u32)>,
    /// The materialized closures whose bodies contain a `^^` (S5). A
    /// `catch`-family send consuming one must refuse: interpreted, a
    /// catch-all can catch the `^^` crossing it — a compiled home cannot
    /// reproduce that (the runtime treats an in-flight compiled-target `^^`
    /// as uncatchable), so the method stays interpreted.
    pub(super) materialized_nlr: HashSet<CVal>,
}

/// What a whole materialized NEST (a cold-path block plus every literal
/// nested inside it, transitively — S5b) does to the enclosing compiled
/// frame. The nest runs INTERPRETED, so nested execution needs no compiled
/// support; the translator only needs these facts for its gates.
#[derive(Default)]
pub(super) struct NestScan {
    /// Symbols written that are free through the WHOLE nest — they resolve
    /// to the snapshot env, so the consuming send must flush them back.
    pub(super) written_frees: Vec<Symbol>,
    /// A `^^` anywhere in the nest (profitability + catch-parity gates).
    pub(super) has_nlr: bool,
    /// A `catch`-family send anywhere in the nest.
    pub(super) has_catch_send: bool,
    /// A send of the enclosing candidate's own selector anywhere in the
    /// nest (the `^^s.whileDo:block` trampoline signature).
    pub(super) sends_own_selector: bool,
}

/// Recursive gate scan for [`Translator::materialize_closure`]: each level's
/// params + `DefineLocal`s shadow the levels above, so only writes free
/// through EVERY level reach the snapshot.
pub(super) fn scan_materialized_nest(
    rc: &StaticBlock,
    inherited: &HashSet<Symbol>,
    own_selector: &str,
    out: &mut NestScan,
) -> Result<(), Refusal> {
    if rc.decl_block.is_some() {
        return Err(refuse(
            RefusalKind::MaterializationGate,
            "guarded block in a materialized nest".to_string(),
        ));
    }
    let mut defined = inherited.clone();
    defined.extend(rc.param_syms.iter().copied());
    for inst in rc.bytecode.0.iter() {
        if let Instruction::DefineLocal(s) | Instruction::DefineLocalKeep(s) = inst {
            defined.insert(*s);
        }
    }
    for inst in rc.bytecode.0.iter() {
        match inst {
            Instruction::MethodReturn => out.has_nlr = true,
            Instruction::Push(Constant::Block(nb)) => {
                scan_materialized_nest(nb, &defined, own_selector, out)?;
            }
            Instruction::StoreLocal(s) | Instruction::StoreLocalKeep(s)
                // A `new:{...}` config literal's stores BIND LOCALLY by
                // construction (StaticBlock::is_init_literal — static
                // semantics, (E)); they are the field-binding DSL, never
                // capture writes, so they need no write-back. This is what
                // un-refuses `Class.new:{ field=local }` inside cold arms
                // (btrees' makeTree).
                if !rc.is_init_literal && !defined.contains(s) => {
                    out.written_frees.push(*s);
                }
            _ => {}
        }
    }
    for inst in rc.bytecode.0.iter() {
        let Some((sel, _, fused_const)) = inst.send_parts() else {
            continue;
        };
        if sel.as_str() == own_selector {
            out.sends_own_selector = true;
        }
        if crate::runtime::block::is_catch_family(sel.as_str()) {
            out.has_catch_send = true;
        }
        if let Some(Constant::Block(nb)) = fused_const {
            scan_materialized_nest(nb, &defined, own_selector, out)?;
        }
    }
    Ok(())
}

pub(super) struct FnCtx {
    pub(super) vm: CVal,
    pub(super) mc: CVal,
    pub(super) fuel: CVal,
    pub(super) depth: CVal,
    /// D3a plumbing: pointer to the VM's `dispatch_epoch` (read by D3b's
    /// baked-guard sites; forwarded on sibling direct calls meanwhile).
    pub(super) epoch: CVal,
    /// A3: the SlotStack head pointer — native slot access re-loads
    /// (ptr, len) through it per access.
    pub(super) slots: CVal,
    pub(super) slot_base: CVal,
    pub(super) exit: CBlock,
    pub(super) ret: AotRet,
    /// Native-stack lane buffers for helper calls (kinds, bits) and the
    /// peek out-parameter.
    pub(super) kinds_buf: cranelift_codegen::ir::StackSlot,
    pub(super) bits_buf: cranelift_codegen::ir::StackSlot,
    pub(super) peek_out: cranelift_codegen::ir::StackSlot,
    /// D3b: the baked direct edge's raw-call ret out-parameter (8 bytes).
    pub(super) direct_ret: cranelift_codegen::ir::StackSlot,
}

pub(super) fn int_inst_kind(i: &Instruction) -> IntBinKind {
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

pub(super) fn double_inst_kind(i: &Instruction) -> IntBinKind {
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
