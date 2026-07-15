//! The bytecode buffer (`CodeBlock`) and the superinstruction peephole pass
//! (`fuse_bytecode`) with its instruction-inspection helpers. Pure emit-side
//! plumbing: no `Compiler` state.

use super::*;

pub struct CodeBlock {
    pub bytecode: Vec<Instruction>,
    pub source_map: Vec<Option<SourceInfo>>,
    pub current_source: Option<SourceInfo>,
}

impl Default for CodeBlock {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeBlock {
    pub fn new() -> Self {
        Self {
            bytecode: Vec::new(),
            source_map: Vec::new(),
            current_source: None,
        }
    }

    pub fn push(&mut self, inst: Instruction) {
        self.bytecode.push(inst);
        self.source_map.push(self.current_source.clone());
    }

    pub fn pop(&mut self) -> Option<Instruction> {
        self.source_map.pop();
        self.bytecode.pop()
    }

    pub fn extend(&mut self, other: CodeBlock) {
        self.bytecode.extend(other.bytecode);
        self.source_map.extend(other.source_map);
    }

    pub fn len(&self) -> usize {
        self.bytecode.len()
    }
}

fn jump_offset(inst: &Instruction) -> Option<isize> {
    match inst {
        Instruction::Jump(o)
        | Instruction::IfJump(o)
        | Instruction::ElseJump(o)
        | Instruction::BranchIfNotBool(o)
        | Instruction::BranchIfNotList(o, _)
        | Instruction::BranchIfNotPlainNew(o) => Some(*o),
        _ => None,
    }
}

pub(super) fn set_jump_offset(inst: &mut Instruction, off: isize) {
    match inst {
        Instruction::Jump(o)
        | Instruction::IfJump(o)
        | Instruction::ElseJump(o)
        | Instruction::BranchIfNotBool(o)
        | Instruction::BranchIfNotList(o, _)
        | Instruction::BranchIfNotPlainNew(o) => *o = off,
        _ => {}
    }
}

fn is_store(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::StoreLocal(_) | Instruction::DefineLocal(_) | Instruction::StoreField(_)
    )
}

/// The store-and-keep superinstruction for a store (stores the top of stack without
/// popping it), i.e. the fusion of `Dup; <store>`.
fn store_keep_variant(inst: &Instruction) -> Option<Instruction> {
    match inst {
        Instruction::StoreLocal(s) => Some(Instruction::StoreLocalKeep(*s)),
        Instruction::DefineLocal(s) => Some(Instruction::DefineLocalKeep(*s)),
        Instruction::StoreField(f) => Some(Instruction::StoreFieldKeep(f.clone())),
        _ => None,
    }
}

/// Maps a standalone devirtualized `Int` op to its `IntBinKind`, for the fusion pass.
fn int_bin_kind(inst: &Instruction) -> Option<IntBinKind> {
    Some(match inst {
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
        _ => return None,
    })
}

/// The `IntBinKind` for a devirtualized `Double` op, for the fused `DoubleBinLL`/`LC` peephole
/// (the operator kind is type-agnostic — shared with the Integer path).
fn double_bin_kind(inst: &Instruction) -> Option<IntBinKind> {
    Some(match inst {
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
        _ => return None,
    })
}

/// Peephole pass: fuse hot adjacent instructions into single superinstructions, saving a
/// dispatch-loop step each. Two families:
/// - `<operand-load>; Send` → `SendLocal`/`SendConst`/`SendField` (the send's last operand
///   is overwhelmingly a local / constant / field). A leading `LoadLocal` receiver is also
///   absorbed (`LoadLocal; LoadLocal; Send` / `LoadLocal; Push; Send` →
///   `SendLocalLocal`/`SendLocalConst`), pushing two operands then dispatching.
/// - assignment: `Dup; <store>; Pop` (statement position) → plain `<store>` (drops the Dup
///   *and* the Pop); `Dup; <store>` (expression position) → a store-and-keep variant.
///
/// See `profiling/superinstructions`.
///
/// Jumps are relative and block-local, so removing an instruction requires: (a) never fusing
/// across a jump target — a pair/triple may only be fused if its non-leading members aren't
/// jump targets (a jump landing there must run that member, not a fused op that skipped it);
/// and (b) recomputing every jump offset against the old→new index map. `source_map` stays
/// index-aligned — the surviving slot keeps the entry where an error would surface (the Send
/// / the store). Targeting the *first* of a fused group stays correct: the fused op
/// reproduces the group's net effect.
pub(crate) fn fuse_bytecode(
    bytecode: Vec<Instruction>,
    source_map: Vec<Option<SourceInfo>>,
) -> (Vec<Instruction>, Vec<Option<SourceInfo>>) {
    let n = bytecode.len();

    // (a) Absolute jump-target set.
    let mut is_target = vec![false; n];
    for (i, inst) in bytecode.iter().enumerate() {
        if let Some(off) = jump_offset(inst) {
            let tgt = i as isize + off;
            if (0..n as isize).contains(&tgt) {
                is_target[tgt as usize] = true;
            }
        }
    }

    // Fuse eligible pairs; track old→new and new→old index maps for the jump fixup.
    let mut new_code: Vec<Instruction> = Vec::with_capacity(n);
    let mut new_smap: Vec<Option<SourceInfo>> = Vec::with_capacity(n);
    let mut old_to_new = vec![0usize; n + 1]; // +1 so a jump-to-end target maps cleanly
    let mut new_to_old: Vec<usize> = Vec::with_capacity(n);

    let mut i = 0;
    while i < n {
        old_to_new[i] = new_code.len();

        // Assignment fusions (Dup is only ever an assignment's value-keep).
        if matches!(bytecode[i], Instruction::Dup) {
            // Statement position `Dup; <store>; Pop` -> plain `<store>` (drops Dup + Pop;
            // the store pops, so the net stack effect is identical).
            if i + 2 < n
                && is_store(&bytecode[i + 1])
                && matches!(bytecode[i + 2], Instruction::Pop)
                && !is_target[i + 1]
                && !is_target[i + 2]
            {
                old_to_new[i + 1] = new_code.len();
                old_to_new[i + 2] = new_code.len();
                new_to_old.push(i);
                new_code.push(bytecode[i + 1].clone());
                new_smap.push(source_map[i + 1].clone());
                i += 3;
                continue;
            }
            // Expression position `Dup; <store>` -> store-and-keep variant.
            if i + 1 < n
                && !is_target[i + 1]
                && let Some(keep) = store_keep_variant(&bytecode[i + 1])
            {
                old_to_new[i + 1] = new_code.len();
                new_to_old.push(i);
                new_code.push(keep);
                new_smap.push(source_map[i + 1].clone());
                i += 2;
                continue;
            }
        }

        // 3-instruction send: a `LoadLocal` receiver + a second operand-load + Send fused
        // into one op that pushes both operands then dispatches (the two hottest shapes:
        // `LoadLocal; LoadLocal; Send` and `LoadLocal; Push; Send`). Checked before the
        // 2-window so the receiver load is absorbed too rather than left standalone.
        if i + 2 < n
            && !is_target[i + 1]
            && !is_target[i + 2]
            && let Instruction::LoadLocal(a) = &bytecode[i]
            && let Instruction::Send(sel, nargs) = &bytecode[i + 2]
        {
            let three = match &bytecode[i + 1] {
                Instruction::LoadLocal(b) => {
                    Some(Instruction::SendLocalLocal(*a, *b, *sel, *nargs))
                }
                Instruction::Push(c) => {
                    Some(Instruction::SendLocalConst(*a, c.clone(), *sel, *nargs))
                }
                _ => None,
            };
            if let Some(three) = three {
                old_to_new[i + 1] = new_code.len();
                old_to_new[i + 2] = new_code.len();
                new_to_old.push(i);
                new_code.push(three);
                new_smap.push(source_map[i + 2].clone()); // keep the Send's source entry
                i += 3;
                continue;
            }
        }

        // 3-instruction Int/Double op (Slice a1): fuse `LoadLocal; <LoadLocal|Push>; {Int,Double}Xxx`
        // into a single `{Int,Double}BinLL`/`BinLC` — same shape as the send triple above, but the
        // terminal is a devirtualized numeric op. Collapses both operand-loads into the op.
        if i + 2 < n
            && !is_target[i + 1]
            && !is_target[i + 2]
            && let Instruction::LoadLocal(a) = &bytecode[i]
            && let Some((kind, is_double)) = int_bin_kind(&bytecode[i + 2])
                .map(|k| (k, false))
                .or_else(|| double_bin_kind(&bytecode[i + 2]).map(|k| (k, true)))
        {
            let three = match (&bytecode[i + 1], is_double) {
                (Instruction::LoadLocal(b), false) => Some(Instruction::IntBinLL(*a, *b, kind)),
                (Instruction::LoadLocal(b), true) => Some(Instruction::DoubleBinLL(*a, *b, kind)),
                (Instruction::Push(c), false) => Some(Instruction::IntBinLC(*a, c.clone(), kind)),
                (Instruction::Push(c), true) => Some(Instruction::DoubleBinLC(*a, c.clone(), kind)),
                _ => None,
            };
            if let Some(three) = three {
                old_to_new[i + 1] = new_code.len();
                old_to_new[i + 2] = new_code.len();
                new_to_old.push(i);
                new_code.push(three);
                new_smap.push(source_map[i + 2].clone()); // keep the Int op's source entry
                i += 3;
                continue;
            }
        }

        if i + 1 < n
            && !is_target[i + 1]
            && let Instruction::Send(sel, nargs) = &bytecode[i + 1]
        {
            let fused = match &bytecode[i] {
                Instruction::LoadLocal(v) => Some(Instruction::SendLocal(*v, *sel, *nargs)),
                Instruction::Push(c) => Some(Instruction::SendConst(c.clone(), *sel, *nargs)),
                Instruction::LoadField(f) => Some(Instruction::SendField(f.clone(), *sel, *nargs)),
                _ => None,
            };
            if let Some(fused) = fused {
                old_to_new[i + 1] = new_code.len(); // never a jump target (guarded above)
                new_to_old.push(i);
                new_code.push(fused);
                new_smap.push(source_map[i + 1].clone()); // keep the Send's source entry
                i += 2;
                continue;
            }
        }
        new_to_old.push(i);
        new_code.push(bytecode[i].clone());
        new_smap.push(source_map[i].clone());
        i += 1;
    }
    old_to_new[n] = new_code.len();

    // (b) Recompute each jump's relative offset against the new layout.
    for new_idx in 0..new_code.len() {
        if let Some(old_off) = jump_offset(&new_code[new_idx]) {
            let old_idx = new_to_old[new_idx];
            let old_target = (old_idx as isize + old_off) as usize;
            let new_target = old_to_new[old_target] as isize;
            set_jump_offset(&mut new_code[new_idx], new_target - new_idx as isize);
        }
    }

    (new_code, new_smap)
}

// The static-type lattice lives in `crate::types::Type` (the shared substrate for the
// resolver/checker; docs/internal/TYPE_SYSTEM_ARCH.md). The optimizer below only *consumes* it: the
// devirt gates act on `Int`/`List`/`Bool` and treat every other type — `Any` included — as
// "no static knowledge", so untyped code compiles exactly as before. `Int` devirt is sound
// only for values proven `Int`; list devirt has a runtime fallback (sound even for a `var`).
