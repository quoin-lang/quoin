use crate::class_table::{ClassSig, ClassTable};
use crate::instruction::{
    Constant, Instruction, IntBinKind, SharedBytecode, SharedSourceMap, StaticBlock,
};
use crate::parser::ast::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, ClassDefinitionNode,
    DeclKind, DeclarationNode, IdentifierNode, IdentifierType, MethodCallNode, MethodSelectorNode,
    Node, NodeValue, ProgramNode, TypeRefNode, UnaryOperatorNode, UnaryOperatorType,
};
use crate::runtime::elem_tag::ElemTag;
use crate::symbol::Symbol;
use crate::types::{SeenTypes, Type};
use crate::value::{NamespacedName, SourceInfo};

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

mod assignment;
mod class_info;
mod devirt;
mod inlining;

/// Canonical string form of a type-annotation (or class-name) identifier — bare for a
/// root name (`Integer`, `Foo?`), bracket-qualified when namespaced (`[Web]Halt`).
/// Must agree with `NamespacedName`'s `Display`, which keys `globals`, runtime dispatch
/// hints, and `populate_from_vm`'s class-table entries.
pub(crate) fn annotation_name(tr: &TypeRefNode) -> String {
    let base = NamespacedName::from_ast(&tr.ident).to_string();
    if !tr.parenthesized {
        return base;
    }
    let mut parts: Vec<String> = tr.args.iter().map(|a| annotation_name(a)).collect();
    if let Some(r) = &tr.ret {
        parts.push(format!("^{}", annotation_name(r)));
    }
    format!("{}({})", base, parts.join(" "))
}

/// A plain identifier's rendered name (parent classes, mixin targets — the
/// non-type-annotation positions).
pub(crate) fn ident_name(ident: &IdentifierNode) -> String {
    NamespacedName::from_ast(ident).to_string()
}

/// Pure structural `Type` of an annotation — no diagnostics, no type-variable
/// scope (the resolver's `resolve_annotation` layers those on top). Unknown
/// bases become `Instance`; malformed generic arity degrades to the bare base.
pub(crate) fn type_from_ref(tr: &TypeRefNode) -> Type {
    type_from_ref_with_vars(tr, &[])
}

/// `type_from_ref` with the enclosing class's declared type parameters in
/// scope: a bare matching name resolves to `Var` (signature recording for the
/// class table, where the compiler's ctx stack isn't available).
pub(crate) fn type_from_ref_with_vars(tr: &TypeRefNode, vars: &[String]) -> Type {
    let base = NamespacedName::from_ast(&tr.ident).to_string();
    if !tr.parenthesized {
        if tr.ident.namespace.is_none() {
            let (core, nullable) = match base.strip_suffix('?') {
                Some(b) => (b, true),
                None => (base.as_str(), false),
            };
            if vars.iter().any(|v| v == core) {
                let v = Type::Var(Arc::from(core));
                return if nullable {
                    Type::Nullable(Box::new(v))
                } else {
                    v
                };
            }
        }
        return Type::from_annotation_name(&base);
    }
    // `Block(args… ^Ret)`: any arity (zero included — `Block()`); no `^`-tail
    // means an `Any` return (docs/GENERICS_ARCH.md §11).
    if base == "Block" {
        return Type::BlockOf {
            params: tr
                .args
                .iter()
                .map(|a| type_from_ref_with_vars(a, vars))
                .collect(),
            ret: Box::new(
                tr.ret
                    .as_ref()
                    .map(|r| type_from_ref_with_vars(r, vars))
                    .unwrap_or(Type::Any),
            ),
        };
    }
    match (base.as_str(), tr.args.len()) {
        ("List", 1) => Type::ListOf(Box::new(type_from_ref_with_vars(&tr.args[0], vars))),
        ("Set", 1) => Type::SetOf(Box::new(type_from_ref_with_vars(&tr.args[0], vars))),
        ("Map", 2) => Type::MapOf(Box::new(type_from_ref_with_vars(&tr.args[1], vars))),
        _ => Type::from_annotation_name(&base),
    }
}

pub struct CodeBlock {
    pub bytecode: Vec<Instruction>,
    pub source_map: Vec<Option<SourceInfo>>,
    pub current_source: Option<SourceInfo>,
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

    pub fn is_empty(&self) -> bool {
        self.bytecode.is_empty()
    }
}

fn jump_offset(inst: &Instruction) -> Option<isize> {
    match inst {
        Instruction::Jump(o)
        | Instruction::IfJump(o)
        | Instruction::ElseJump(o)
        | Instruction::BranchIfNotBool(o)
        | Instruction::BranchIfNotList(o) => Some(*o),
        _ => None,
    }
}

fn set_jump_offset(inst: &mut Instruction, off: isize) {
    match inst {
        Instruction::Jump(o)
        | Instruction::IfJump(o)
        | Instruction::ElseJump(o)
        | Instruction::BranchIfNotBool(o)
        | Instruction::BranchIfNotList(o) => *o = off,
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
// resolver/checker; docs/TYPE_SYSTEM_ARCH.md). The optimizer below only *consumes* it: the
// devirt gates act on `Int`/`List`/`Bool` and treat every other type — `Any` included — as
// "no static knowledge", so untyped code compiles exactly as before. `Int` devirt is sound
// only for values proven `Int`; list devirt has a runtime fallback (sound even for a `var`).

/// Compile-time context for the class body currently being compiled (Slice 2b). Pushed
/// on a stack while compiling a class def / extension; used to type self-sends by their
/// callee's declared return type and to devirtualize self-sends in a sealed class.
struct ClassCtx {
    /// Unique id for this class-body/extension context within the process — the
    /// AOT sibling-group key (self-calls compile to direct calls only among
    /// methods that share it, i.e. the same method table + receiver shape).
    id: u32,
    /// The class name for a named definition/extension; empty for anonymous
    /// extension targets (e.g. `.meta <-- { … }`), whose sealedness is looked up
    /// through to the nearest named enclosing context.
    name: String,
    /// Selectors defined more than once in this body (typed multimethod
    /// variants) — excluded from AOT candidacy, since a direct call would bypass
    /// the runtime variant scoring.
    multi: HashSet<String>,
    /// Class/mixin-header type parameters (`Iterate(T U)`) — the type variables
    /// this body's method signatures may use (checker-only).
    type_params: Vec<String>,
    /// Method selector → declared return `Type` (methods that annotate a return).
    returns: HashMap<String, Type>,
    /// The class is compile-sealed: `sealed!` appears as a direct (unconditional) body
    /// statement, so its method table is frozen and same-class self-sends can be
    /// devirtualized (Slice 2b-B).
    sealed: bool,
    /// Selector → method body, for inlining a leaf self-send (Phase 5·1). A sealed class can't be
    /// subclassed, so `self.foo` provably resolves to this class's own `foo`; a trivial body is
    /// spliced at the call site instead of dispatched.
    bodies: HashMap<String, Arc<BlockNode>>,
}

/// A non-fatal type diagnostic: the message plus the source span it points at, for `path:line:col`
/// rendering (Phase 4). `span` is `None` when a check can't attribute a precise location.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<SourceInfo>,
    /// Secondary "why-chain" notes (Phase 4 provenance): e.g. where a variable got the type that
    /// caused this diagnostic. Rendered indented under the main message, each at its own span.
    pub notes: Vec<Note>,
}

/// A secondary note attached to a [`Diagnostic`] — a message plus the span it points at.
#[derive(Clone, Debug)]
pub struct Note {
    pub message: String,
    pub span: Option<SourceInfo>,
}

/// Where a local's type came from (Phase 4 provenance), for the why-chain note: the declaration
/// span plus a short origin phrase (`declared`, `` inferred from `name` ``, `parameter`).
#[derive(Clone, Debug)]
struct TypeProvenance {
    span: SourceInfo,
    origin: String,
}

/// Unary methods safe to send to `nil` — they don't dereference the receiver, so a possibly-nil
/// receiver isn't flagged for these (Phase 3c nil-misuse check).
const NIL_SAFE_SELECTORS: &[&str] = &["defined?", "s", "pp", "class", "hash", "print"];

/// A flow-narrowable path — what a guard (Phase 3c) can refine the type of. Only locals and
/// instance fields (`@name`) narrow; global, namespaced, and reserved reads do not.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum NarrowKey {
    Local(String),
    Field(String),
}

impl NarrowKey {
    /// The narrowable path an identifier read refers to, or `None` if it isn't one (a global,
    /// namespaced, or reserved `nil`/`true`/`false` read).
    fn from_ident(id: &IdentifierNode) -> Option<NarrowKey> {
        if id.identifier_type == IdentifierType::Instance {
            Some(NarrowKey::Field(id.name.clone()))
        } else if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
            None
        } else if matches!(id.name.as_str(), "nil" | "true" | "false") {
            None
        } else {
            Some(NarrowKey::Local(id.name.clone()))
        }
    }
}

// ---- AST shape matchers (Phase 3c) -----------------------------------------------------------
// Small, shallow structural matchers shared by the checker's recognizers. They match *one* level
// of shape and **bottom out on the semantic helpers** — path classification via
// `NarrowKey::from_ident`, selector reconstruction via `call_selector_*` — so a match can never
// silently disagree with the VM's dispatch (e.g. the variadic-fold selector). Compose these rather
// than re-deriving shapes inline; new checks add matchers here.

/// `RECV.sel` with no arguments → (receiver, selector). `None` for a keyword send or a
/// receiver-less (`self`) send.
fn as_unary_send(node: &Node) -> Option<(&Node, &str)> {
    let NodeValue::MethodCall(mc) = &node.value else {
        return None;
    };
    if !mc.arguments.expressions.is_empty() {
        return None;
    }
    let idents = &mc.arguments.signature.identifiers;
    if idents.len() != 1 {
        return None;
    }
    Some((mc.subject.as_deref()?, idents[0].name.as_str()))
}

/// The narrowable path an expression reads, if it is a bare local or `@field` identifier.
fn as_path(node: &Node) -> Option<NarrowKey> {
    match &node.value {
        NodeValue::Identifier(id) => NarrowKey::from_ident(id),
        _ => None,
    }
}

/// The reserved `nil` literal.
fn is_nil_literal(node: &Node) -> bool {
    matches!(&node.value, NodeValue::Identifier(id) if id.name == "nil")
}

/// A recognized nil-guard's narrowing (Phase 3c): the path it tests and the type it refines to in
/// each arm. For `x.defined?.if:{…} else:{…}` with `x: T?`, `if_arm = T`, `else_arm = Nil`.
struct GuardInfo {
    key: NarrowKey,
    if_arm: Type,
    else_arm: Type,
}

impl GuardInfo {
    /// The refinement for the arm reached by keyword `kw` (`if` → true branch, `else` → false).
    fn arm_type(&self, kw: &str) -> Option<Type> {
        match kw {
            "if" => Some(self.if_arm.clone()),
            "else" => Some(self.else_arm.clone()),
            _ => None,
        }
    }
}

struct Scope {
    locals: HashSet<String>,
    /// Subset of `locals` declared with `let` — reassigning one is a compile error.
    immutable: HashSet<String>,
    /// Declared type of a local/param, when known (Integer/Boolean); absent = Unknown.
    types: HashMap<String, Type>,
    /// Subset of `types` that came from an *explicit* annotation, not devirt inference. A
    /// reassignment is checked against the declared type only for these — an inferred type is a
    /// hint, not a contract, so `var x = 0` reassigned to a String is fine, but `var x: Integer`
    /// reassigned to a String is not (Phase 3a).
    declared_types: HashMap<String, Type>,
    /// Flow-narrowed types active in this scope (Phase 3c) — a guard refines a local/field here;
    /// `narrowed_type` reads the innermost. Empty until 3c·1 installs the narrowing rules.
    narrowed: HashMap<NarrowKey, Type>,
    /// Provenance of each local's recorded type (Phase 4 why-chain): where it was declared/inferred
    /// and a short origin phrase. Keyed like `types`; read when a diagnostic blames the local.
    provenance: HashMap<String, TypeProvenance>,
    /// True for the top-level scope of an object-initializer block (`X.new:{ … }`),
    /// where a bare `field = value` binds an instance field (no `var` required).
    is_init: bool,
}

pub struct Compiler {
    scopes: Vec<Scope>,
    temp_counter: usize,
    /// >0 while compiling the body of a `<-`/`<--` block whose target is an
    /// immediate value type (Integer/Double/Boolean/Nil). Instance variables are
    /// rejected there so the "value types have no fields" rule surfaces at compile
    /// time rather than only when a method runs.
    value_type_def_depth: usize,
    /// One-shot flag set right before compiling the block argument of `X.new:{ … }`;
    /// consumed by the next `compile_block` to mark that block's scope `is_init`.
    next_block_is_init: bool,
    /// One-shot narrowing set right before compiling a guard arm block (Phase 3c); the next
    /// `compile_block` installs it into that arm's scope. Mirrors `next_block_is_init`.
    next_block_narrowing: Option<(NarrowKey, Type)>,
    /// One-shot: the declared param types of the `Block(…)`-typed parameter the next block
    /// literal is being passed to, receiver-bound (G4b, GENERICS_ARCH.md §11.3). The literal's
    /// `compile_block` seeds its UNANNOTATED params from these as narrowing-grade beliefs —
    /// never contracts, never devirt.
    next_block_expected: Option<Vec<Type>>,
    /// Sharpened static types of compiled block literals (`Block(args ^Ret)` shapes), keyed by
    /// `BlockNode` address — how `static_type` sees a literal after its body compiled; before
    /// that it is bare `Block` (gradual, a safe miss). Lives only for this compile.
    block_literal_types: HashMap<usize, Type>,
    /// Per-`compile_block` accumulator for the body's ACTUAL return type: the join of the tail
    /// expression and every real (non-inlined) `^` return — `^^` diverges the block and
    /// contributes nothing. Innermost last; drives block-return inference (§11.3).
    block_ret_harvest: Vec<Type>,
    /// One-shot request set right before compiling a guard arm block: the key whose narrowed type
    /// the arm's `compile_block` should snapshot at exit (into `captured_arm_exit`) so the join at
    /// the conditional's end can merge the arms' exit states (Phase 3c join/merge).
    next_block_capture: Option<NarrowKey>,
    /// The exit narrowing captured for `next_block_capture`'s key by the most recent arm
    /// `compile_block` — read by `compile_method_call` right after the arm compiles.
    captured_arm_exit: Option<Type>,
    /// Current self-send inlining nesting depth (Phase 5·2), bounded by `MAX_INLINE_DEPTH`.
    inline_depth: usize,
    /// `class name → (selector → method body)` for every class compiled so far in this unit, so an
    /// explicit-receiver `v.x` can inline against *any* in-unit sealed class, not just the one being
    /// compiled (Phase 5·3b). Backward references only; cross-unit bodies aren't available as AST.
    class_bodies: HashMap<String, HashMap<String, Arc<BlockNode>>>,
    /// While splicing a computed method body at an *explicit-receiver* call site (Phase 5·3c), the
    /// local holding the receiver: `self`/`@field`/implicit self-sends in the body are rebound to it
    /// (`self` was the callee `v`, not the caller). `None` outside such a splice.
    self_override: Option<Symbol>,
    /// While splicing a body that has parameters (Phase 5·4): each param name → the temp holding its
    /// argument, so a bare param reference in the body loads that temp. Saved/restored across nesting.
    param_override: HashMap<String, Symbol>,
    /// Stack of per-class compile context, pushed while compiling a class body: method
    /// return types (Slice 2b-A) + the method set + whether the class is sealed (2b-B).
    class_ctx: Vec<ClassCtx>,
    /// While compiling an *inlined* control-flow block body (Slice 2d), collects the
    /// bytecode positions of top-level `^` (BlockReturn) placeholder jumps so
    /// `inline_block_body` can patch them to land just past the inlined region. `None`
    /// outside inlining (a `^` then compiles to a normal `BlockReturn`). Cleared on entry
    /// to a real nested block so its `^` isn't captured by an enclosing inlined region.
    inline_carets: Option<Vec<usize>>,
    /// Class names known so far — builtins + classes defined by earlier-compiled units +
    /// this program's own defs (seeded by `prescan_class_defs`). Shared across units so a
    /// later unit sees earlier ones' classes; consulted by `resolve_annotation` (Phase 2).
    seen_types: SeenTypes,
    /// Shared class-signature table for the cross-class checks (Phase 3b) — parallel to
    /// `seen_types`, populated from the current unit's AST and threaded across units.
    class_table: ClassTable,
    /// Non-fatal type diagnostics (e.g. `unknown type Foo`) collected during compilation.
    /// Surfaced by the caller; never blocks lowering (gradual best-effort).
    diagnostics: Vec<Diagnostic>,
    /// Declared return types of the block(s) currently being compiled (`|args ^T|`), innermost
    /// last. A `^`/`^^` return or a block's tail expression is checked (and numeric literals
    /// promoted) against the top entry; `None` = no declared return → not checked. Phase 3a.
    return_type_stack: Vec<Option<Type>>,
    /// Collect AOT candidates (docs/AOT_ARCH.md) while compiling: methods of
    /// sealed classes with all-scalar params and return. Only the runner's
    /// once-per-unit compiles opt in, and only when `QN_AOT=1`.
    collect_aot: bool,
    aot_candidates: Vec<crate::codegen::AotCandidate>,
    /// Source of `ClassCtx::id`.
    class_ctx_counter: u32,
    /// Mint a `template_id` for every compiled block literal, so all its closures
    /// share one inline-cache array (`VmState::ic_registry`). Only the runner's
    /// once-per-unit compiles opt in (`with_template_ids`); eval/REPL/interpolation
    /// compile per evaluation, and per-compile ids would grow the registry without
    /// bound. Default: off.
    mint_template_ids: bool,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope {
                locals: HashSet::new(),
                immutable: HashSet::new(),
                types: HashMap::new(),
                declared_types: HashMap::new(),
                narrowed: HashMap::new(),
                provenance: HashMap::new(),
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            next_block_expected: None,
            block_literal_types: HashMap::new(),
            block_ret_harvest: Vec::new(),
            next_block_capture: None,
            captured_arm_exit: None,
            inline_depth: 0,
            class_bodies: HashMap::new(),
            self_override: None,
            param_override: HashMap::new(),
            class_ctx: Vec::new(),
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            return_type_stack: Vec::new(),
            mint_template_ids: false,
            collect_aot: false,
            aot_candidates: Vec::new(),
            class_ctx_counter: 0,
        }
    }

    /// Opt this compile into template-id minting (shared inline-cache arrays).
    pub fn with_template_ids(mut self) -> Self {
        self.mint_template_ids = true;
        self
    }

    /// AOT candidacy (docs/AOT_ARCH.md §3): a method of a sealed class whose
    /// params and return are all scalar, unguarded, and single-variant. The
    /// authoritative bytecode walk happens in `codegen::translate` — this is the
    /// cheap proof-of-eligibility filter; refusal there is silent and safe.
    /// Sealedness looks through anonymous extension contexts (`.meta <-- {…}`)
    /// to the nearest named enclosing class body: the class body (including its
    /// meta extension) runs to completion — sealing both tables — before any
    /// external caller can dispatch, the same argument Phase 5 inlining trusts.
    fn maybe_collect_aot_candidate(
        &mut self,
        selector: &str,
        block_node: &BlockNode,
        bytecode: &CodeBlock,
    ) {
        if !self.collect_aot || !self.mint_template_ids {
            return;
        }
        let Some(imm) = self.class_ctx.last() else {
            return;
        };
        let sealed = imm.sealed
            || self
                .class_ctx
                .iter()
                .rev()
                .find(|c| !c.name.is_empty())
                .is_some_and(|c| c.sealed);
        if imm.multi.contains(selector) || block_node.decl_block.is_some() {
            return;
        }
        // B2 (docs/BLOCK_AOT_ARCH.md §3): an OPEN owner's method may compile —
        // marked so the translator emits no direct sibling calls (every send
        // crosses a dispatch-equivalent seam; a reopen then simply dispatches
        // to its new template, per-dispatch minting making the stale entry
        // unreachable). Sealed owners keep the direct-call fast path.
        let open_owner = !sealed;
        let mut params = Vec::new();
        let mut spec_params = Vec::new();
        for arg in &block_node.arguments {
            if arg.identifier.identifier_type == IdentifierType::Instance {
                return; // not a plain method parameter list
            }
            let Some(hint) = &arg.type_hint else {
                // Speculative-AOT (S0): an UNANNOTATED param no longer ends
                // candidacy — it rides as an Obj placeholder whose real kind
                // the runtime profile supplies at compile time (with an entry
                // precondition, S1). An annotation stays a dispatch GUARANTEE
                // and is preferred whenever present.
                params.push(crate::codegen::AotParam::Obj);
                spec_params.push(true);
                continue;
            };
            // The ERASED dispatch name (`List(Integer)` → `List`), exactly what the
            // runtime guarantees about the arg; the element tag rides separately in
            // `StaticBlock.param_elem_tags`, where the translator picks it up as a
            // `CollectionOf` proof (B1). A type-var param erases to `Object` and
            // stays a non-candidate, as before.
            let Some(k) = crate::codegen::AotParam::from_annotation(&self.dispatch_type_name(hint))
            else {
                return;
            };
            params.push(k);
            spec_params.push(false);
        }
        let mut spec_ret = false;
        let ret = match &block_node.return_type {
            // Same erasure as params: `^List(U)` returns a List at runtime
            // (the variables are checker-only); a type-var return erases to
            // `Object` and stays a non-candidate.
            Some(rt) => {
                let Some(ret) =
                    crate::codegen::AotRet::from_annotation(&self.dispatch_type_name(rt))
                else {
                    return;
                };
                ret
            }
            // An absent return annotation was ALSO a candidacy cliff before
            // S0; the profile observes the real return kind for S2 (Obj until
            // then).
            None => {
                spec_ret = true;
                crate::codegen::AotRet::Obj
            }
        };
        // `compile_block` just pushed the compiled body as a block constant.
        let Some(Instruction::Push(Constant::Block(rc))) = bytecode.bytecode.last() else {
            return;
        };
        let group_id = imm.id;
        self.aot_candidates.push(crate::codegen::AotCandidate {
            group_id,
            selector: selector.to_string(),
            block: rc.clone(),
            params,
            ret,
            open_owner,
            role: crate::codegen::AotRole::Method,
            spec_params,
            spec_ret,
            spec_preconditions: Vec::new(),
        });
    }

    /// Collect a nested block LITERAL as a block-template candidate (B3a,
    /// docs/BLOCK_AOT_ARCH.md §3): invoked via `valueWithSelfOrArg:` from the
    /// combinator seams when the registry has a compiled entry. Cheap
    /// prefilter only — translation refusals do the real gating; the prescan
    /// skips the two shapes that always refuse (a nested literal push, a
    /// non-local return) to keep unit-load compile time down.
    fn maybe_collect_block_candidate(&mut self, rc: &Rc<StaticBlock>) {
        if !self.collect_aot || !self.mint_template_ids {
            return;
        }
        if rc.template_id.is_none()
            || rc.param_syms.len() > 1
            || rc.decl_block.is_some()
            || rc.name.is_some()
            // A config literal's stores bind into its own frame (E); the
            // template translator's free-variable write path (env_set) would
            // chain-write instead. Configs are never invoked through the
            // vWSOA seam anyway — this is the defensive mirror.
            || rc.is_init_literal
        {
            return;
        }
        let hopeless = rc.bytecode.0.iter().any(|i| {
            matches!(
                i,
                Instruction::Push(Constant::Block(_)) | Instruction::MethodReturn
            )
        });
        if hopeless {
            return;
        }
        self.aot_candidates.push(crate::codegen::AotCandidate {
            // Blocks have no sibling group (no direct calls either way).
            group_id: u32::MAX,
            selector: format!("block@{}", rc.template_id.unwrap()),
            block: rc.clone(),
            params: vec![crate::codegen::AotParam::Obj],
            ret: crate::codegen::AotRet::Obj,
            open_owner: true,
            role: crate::codegen::AotRole::BlockTemplate,
            spec_params: vec![false],
            spec_ret: false,
            spec_preconditions: Vec::new(),
        });
    }

    /// Opt this compile into AOT-candidate collection (implies nothing at
    /// runtime by itself — the runner hands the candidates to codegen).
    pub fn with_aot(mut self) -> Self {
        self.collect_aot = true;
        self
    }

    /// The AOT candidates collected during compilation (drained).
    pub fn take_aot_candidates(&mut self) -> Vec<crate::codegen::AotCandidate> {
        std::mem::take(&mut self.aot_candidates)
    }

    pub fn new_with_locals(locals: HashSet<String>) -> Self {
        Self {
            scopes: vec![Scope {
                locals,
                immutable: HashSet::new(),
                types: HashMap::new(),
                declared_types: HashMap::new(),
                narrowed: HashMap::new(),
                provenance: HashMap::new(),
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            next_block_expected: None,
            block_literal_types: HashMap::new(),
            block_ret_harvest: Vec::new(),
            next_block_capture: None,
            captured_arm_exit: None,
            inline_depth: 0,
            class_bodies: HashMap::new(),
            self_override: None,
            param_override: HashMap::new(),
            class_ctx: Vec::new(),
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            return_type_stack: Vec::new(),
            mint_template_ids: false,
            collect_aot: false,
            aot_candidates: Vec::new(),
            class_ctx_counter: 0,
        }
    }

    /// Is this `<-`/`<--` target an immediate value type? `true`/`false`/`nil` are
    /// `Identifier` nodes by name, alongside the `Integer`/`Double`/`Boolean`/`Nil`
    /// class names.
    ///
    /// NOTE: this is a *static* check, so it only catches syntactically-literal
    /// targets. A *computed* target that resolves to a value type — e.g.
    /// `(1 + 2) <-- { |@x| test -> { @x } }` — is not recognized here, so the
    /// compiler accepts it. It's harmless rather than wrong (the `@x` reads `nil`
    /// and any `@x =` throws at runtime), but it's also useless. Catching it
    /// requires a *runtime* check at `get_target_class_for_def` time: reject
    /// instance-variable declaration/use when the receiver resolves to a value
    /// type. See QUOIN_TODO.md.
    fn is_value_type_target(node: &Node) -> bool {
        match &node.value {
            // Literal value-type instances: `5 <-- …`, `3.14 <-- …`.
            NodeValue::Integer(_) | NodeValue::Double(_) => true,
            // Class names, plus `true` / `false` / `nil` (which are identifiers by
            // name): `Integer <-- …`, `true <-- …`, etc.
            NodeValue::Identifier(id) => matches!(
                id.name.as_str(),
                "Integer" | "Double" | "Boolean" | "Nil" | "true" | "false" | "nil"
            ),
            _ => false,
        }
    }

    fn new_temp_var(&mut self) -> String {
        self.temp_counter += 1;
        format!("__qn_temp_{}", self.temp_counter)
    }

    fn is_local(&self, name: &str) -> bool {
        if name == "self" {
            return true;
        }
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return true;
            }
        }
        false
    }

    fn push_scope(&mut self, locals: HashSet<String>) {
        self.scopes.push(Scope {
            locals,
            immutable: HashSet::new(),
            types: HashMap::new(),
            declared_types: HashMap::new(),
            narrowed: HashMap::new(),
            provenance: HashMap::new(),
            is_init: false,
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Declare a fresh local in the current (innermost) scope. Errors if the name is
    /// already declared *in this scope* (redeclaration); shadowing an outer scope is
    /// allowed. `let` bindings are recorded as immutable.
    fn declare_local(&mut self, name: &str, mutable: bool) -> Result<(), String> {
        let scope = self.scopes.last_mut().unwrap();
        if scope.locals.contains(name) {
            return Err(format!("`{}` is already declared in this scope", name));
        }
        scope.locals.insert(name.to_string());
        if !mutable {
            scope.immutable.insert(name.to_string());
        }
        Ok(())
    }

    /// Was `name` declared with `let`? Resolves to the nearest scope that binds it
    /// (matching `is_local`'s innermost-first walk).
    fn is_immutable(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.immutable.contains(name);
            }
        }
        false
    }

    /// Declared `Type` of a local/param — the nearest binding's recorded type,
    /// or `Unknown` (untyped, or not a plain local).
    fn local_type(&self, name: &str) -> Type {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.types.get(name).cloned().unwrap_or(Type::Any);
            }
        }
        Type::Any
    }

    /// Record a known type for a local just declared in the innermost scope.
    fn record_local_type(&mut self, name: &str, ty: Type, provenance: Option<TypeProvenance>) {
        if ty != Type::Any {
            let scope = self.scopes.last_mut().unwrap();
            scope.types.insert(name.to_string(), ty);
            if let Some(p) = provenance {
                scope.provenance.insert(name.to_string(), p);
            }
        }
    }

    /// Record a local's *declared* (annotated) type — into both `types` (devirt) and
    /// `declared_types` (the reassignment check, which enforces only explicit contracts).
    fn record_declared_type(&mut self, name: &str, ty: Type, provenance: Option<TypeProvenance>) {
        if ty != Type::Any {
            let scope = self.scopes.last_mut().unwrap();
            scope.types.insert(name.to_string(), ty.clone());
            scope.declared_types.insert(name.to_string(), ty);
            if let Some(p) = provenance {
                scope.provenance.insert(name.to_string(), p);
            }
        }
    }

    /// Build a [`TypeProvenance`] pointing at `node`'s span with origin phrase `origin`, or `None`
    /// if `node` carries no source location (nothing useful to point at).
    fn provenance_at(node: &Node, origin: String) -> Option<TypeProvenance> {
        Self::provenance_from(node.source_info.clone(), origin)
    }

    /// Build a [`TypeProvenance`] from a raw span (e.g. a param's `IdentifierNode`), or `None`.
    fn provenance_from(span: Option<SourceInfo>, origin: String) -> Option<TypeProvenance> {
        span.map(|span| TypeProvenance { span, origin })
    }

    /// The provenance of a local's recorded type — where it was declared/inferred (Phase 4).
    fn local_provenance(&self, name: &str) -> Option<&TypeProvenance> {
        self.scopes
            .iter()
            .rev()
            .find(|s| s.locals.contains(name))
            .and_then(|s| s.provenance.get(name))
    }

    /// The explicitly-declared type of a local, if any — `None` for an untyped local even when a
    /// type was *inferred* for it (an inferred type is a devirt hint, not a reassignment contract).
    fn declared_type(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.declared_types.get(name).cloned();
            }
        }
        None
    }

    /// The flow-narrowed type of a path at the current point, if any — innermost scope wins
    /// (Phase 3c). Empty until 3c·1 installs narrowing, so today this always returns `None`.
    fn narrowed_type(&self, key: &NarrowKey) -> Option<Type> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.narrowed.get(key).cloned())
    }

    /// Add every top-level `Name <- …` class definition to `seen_types`, so an annotation can
    /// forward-reference a class defined later in the same unit (and so later units see it).
    /// Only simple top-level defs are collected — the common case.
    fn prescan_class_defs(&self, program: &ProgramNode) {
        for expr in &program.expressions {
            if let NodeValue::ClassDefinition(cd) = &expr.value {
                let name = ident_name(&cd.identifier);
                self.seen_types.insert(&name);
                self.class_table.insert(&name, self.class_sig_from_def(cd));
            }
        }
    }

    /// Resolve a type-annotation name to a `Type`, flagging an unknown user class with a
    /// non-fatal `unknown type Foo` diagnostic (Phase 2). Resolution never fails: an unknown
    /// name still yields `Instance(name)` so lowering proceeds (gradual best-effort).
    /// Push a non-fatal type diagnostic, pointing at `span` when one is available (Phase 4).
    fn warn(&mut self, message: String, span: Option<&SourceInfo>) {
        self.warn_with_notes(message, span, Vec::new());
    }

    /// Like [`warn`](Self::warn) but with secondary why-chain notes (Phase 4 provenance).
    fn warn_with_notes(&mut self, message: String, span: Option<&SourceInfo>, notes: Vec<Note>) {
        self.diagnostics.push(Diagnostic {
            message,
            span: span.cloned(),
            notes,
        });
    }

    fn resolve_annotation(&mut self, tr: &TypeRefNode) -> Type {
        // A bare name that matches a declared class/mixin-header type parameter
        // is a type variable (`T?` rides the nullable suffix inside the ident,
        // like every annotation).
        if !tr.parenthesized && tr.ident.namespace.is_none() {
            let (base, nullable) = match tr.ident.name.strip_suffix('?') {
                Some(b) => (b, true),
                None => (tr.ident.name.as_str(), false),
            };
            if self.declared_type_var(base) {
                let v = Type::Var(Arc::from(base));
                return if nullable {
                    Type::Nullable(Box::new(v))
                } else {
                    v
                };
            }
        }
        if tr.parenthesized {
            let base = ident_name(&tr.ident);
            // The `^`-marked return tail is block-type syntax only
            // (`Block(Integer ^Boolean)`, GENERICS_ARCH.md §11).
            if tr.ret.is_some() && base != "Block" {
                self.warn(
                    format!(
                        "`^` return types belong to `Block(…)` annotations; `{base}` \
                         takes plain type arguments"
                    ),
                    tr.ident.source_info.as_ref(),
                );
            }
            match (base.as_str(), tr.args.len()) {
                // Any arity, zero included (`Block()` = no args, `Any` return).
                ("Block", _) => {}
                ("List", 1) | ("Set", 1) => {}
                ("Map", 2) => {
                    let key = annotation_name(&tr.args[0]);
                    if key != "String" {
                        self.warn(
                            format!(
                                "Map keys are String (got `Map({} …)`); only the value \
                                 type is generic for now",
                                key
                            ),
                            tr.ident.source_info.as_ref(),
                        );
                    }
                }
                ("List", n) | ("Set", n) => {
                    self.warn(
                        format!("`{base}` takes 1 type argument, got {n}"),
                        tr.ident.source_info.as_ref(),
                    );
                }
                ("Map", n) => {
                    self.warn(
                        format!("`Map` takes 2 type arguments (`Map(String V)`), got {n}"),
                        tr.ident.source_info.as_ref(),
                    );
                }
                _ => {
                    self.warn(
                        format!("type `{base}` does not take generic arguments"),
                        tr.ident.source_info.as_ref(),
                    );
                }
            }
            for a in &tr.args {
                // Resolve arguments for their own diagnostics (unknown names etc.).
                let _ = self.resolve_annotation(a);
            }
            if let Some(r) = &tr.ret {
                let _ = self.resolve_annotation(r);
            }
            return type_from_ref_with_vars(tr, &self.ctx_type_params());
        }
        let ty = Type::from_annotation_name(&ident_name(&tr.ident));
        // `T?` is unknown iff its base `T` is unknown.
        let base = match &ty {
            Type::Nullable(inner) => inner.as_ref(),
            other => other,
        };
        if let Type::Instance(class) = base {
            if !self.seen_types.contains(class) {
                self.warn(
                    format!("unknown type `{}`", class),
                    tr.ident.source_info.as_ref(),
                );
            }
        }
        ty
    }

    /// Is `name` a type parameter declared by any enclosing class/mixin header?
    fn declared_type_var(&self, name: &str) -> bool {
        self.class_ctx
            .iter()
            .any(|c| c.type_params.iter().any(|p| p == name))
    }

    /// Every type parameter in scope (all enclosing class/mixin headers).
    fn ctx_type_params(&self) -> Vec<String> {
        self.class_ctx
            .iter()
            .flat_map(|c| c.type_params.iter().cloned())
            .collect()
    }

    /// The element-tag *requirement* a generic param annotation places on
    /// dispatch: `|l: List(Integer)|` matches only Integer-tagged lists
    /// (GENERICS_ARCH.md §5). `None` = no requirement (bare or unenforceable).
    fn param_elem_tag(&mut self, tr: &TypeRefNode) -> Option<ElemTag> {
        if tr.args.is_empty() {
            return None;
        }
        let inner = match (ident_name(&tr.ident).as_str(), tr.args.len()) {
            ("List", 1) | ("Set", 1) => &tr.args[0],
            ("Map", 2) => &tr.args[1],
            _ => return None,
        };
        self.enforceable_elem_tag_of_ref(inner)
    }

    /// Is this decl a collection literal whose declared type is generic —
    /// the tagged-literal construction pattern?
    pub(super) fn generic_literal_decl(expected: &Type, rvalue: &Node) -> bool {
        matches!(
            (expected, &rvalue.value),
            (Type::ListOf(_), NodeValue::List(_))
                | (Type::MapOf(_), NodeValue::Map(_))
                | (Type::SetOf(_), NodeValue::Set(_))
        )
    }

    /// `enforceable_elem_tag_of_ref`, but from a resolved `Type` (the decl
    /// path, where the annotation is already resolved). Same honesty rules.
    pub(super) fn enforceable_elem_tag_of_type(
        &mut self,
        inner: &Type,
        decl: &DeclarationNode,
    ) -> Option<ElemTag> {
        match ElemTag::from_type(inner) {
            Some(tag) => Some(tag),
            None => {
                let base = match inner {
                    Type::ListOf(_) => Some(ElemTag::List),
                    Type::MapOf(_) => Some(ElemTag::Map),
                    Type::SetOf(_) => Some(ElemTag::Set),
                    _ => None, // Var/Any/…: checker-only, no tag
                };
                if let Some(b) = base {
                    self.warn(
                        format!(
                            "nested element types are checker-only; `{}` is enforced as \
                             `{}` at runtime",
                            inner.name(),
                            b.name(),
                        ),
                        decl.rvalue.source_info.as_ref(),
                    );
                }
                base
            }
        }
    }

    /// The runtime-enforceable tag for an element annotation, with the
    /// guarantee-honesty degradations: a nested generic degrades to its base
    /// (with a warning — `List(List(Integer))` is enforced as `List(List)`),
    /// and a type variable or `Any` yields no tag at all (checker-only).
    fn enforceable_elem_tag_of_ref(&mut self, tr: &TypeRefNode) -> Option<ElemTag> {
        if tr.args.is_empty() && tr.ident.namespace.is_none() {
            let base = tr.ident.name.strip_suffix('?').unwrap_or(&tr.ident.name);
            if self.declared_type_var(base) {
                return None;
            }
        }
        let t = type_from_ref(tr);
        match ElemTag::from_type(&t) {
            Some(tag) => Some(tag),
            None => {
                let base = match t {
                    Type::ListOf(_) => Some(ElemTag::List),
                    Type::MapOf(_) => Some(ElemTag::Map),
                    Type::SetOf(_) => Some(ElemTag::Set),
                    _ => None,
                };
                if let Some(b) = base {
                    self.warn(
                        format!(
                            "nested element types are checker-only; `{}` is enforced as \
                             `{}` at runtime",
                            annotation_name(tr),
                            b.name(),
                        ),
                        tr.ident.source_info.as_ref(),
                    );
                }
                base
            }
        }
    }

    /// The runtime dispatch signature for a param annotation: generic arguments
    /// erase to the base class (the tag requirement rides separately in
    /// `param_elem_tags`), and a declared
    /// type variable erases to `Object` (variables never dispatch;
    /// GENERICS_ARCH.md §4.4/§5).
    fn dispatch_type_name(&self, tr: &TypeRefNode) -> String {
        if tr.args.is_empty() && tr.ident.namespace.is_none() {
            let base = tr.ident.name.strip_suffix('?').unwrap_or(&tr.ident.name);
            if self.declared_type_var(base) {
                return "Object".to_string();
            }
        }
        ident_name(&tr.ident)
    }

    /// Compile `node` in a position that expects `expected`. A numeric *literal* promotes to
    /// match (`1` where a `Double` is wanted → the Double `1.0`); otherwise it compiles normally
    /// and its synthesized type is checked against `expected`. Phase 3a.
    fn compile_expecting(
        &mut self,
        node: &Node,
        expected: &Type,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Value-level promotion: an Integer *literal* where a Double is wanted becomes a Double.
        if *expected == Type::Double {
            if let NodeValue::Integer(i) = &node.value {
                bytecode.push(Instruction::Push(Constant::Double(i.value as f64)));
                return Ok(());
            }
        }
        self.compile_node(node, bytecode)?;
        self.check_type(node, expected);
        Ok(())
    }

    /// Warn if `node`'s statically-known type is confidently incompatible with `expected`. Silent
    /// whenever either side is `Any`, `expected` is an unknown class (already flagged as `unknown
    /// type`), or the actual type can't be pinned down — the gradual "never speak on Any" rule.
    fn check_type(&mut self, node: &Node, expected: &Type) {
        match expected {
            Type::Any => return,
            Type::Instance(n) if !self.seen_types.contains(n) => return,
            _ => {}
        }
        let actual = self.static_type(node);
        if actual.compatible_with(expected) {
            return;
        }
        // Instance-vs-Instance: the class table may prove a subtype relation that structural
        // `compatible_with` (exact match only) can't. `None` (unknown hierarchy) stays silent.
        if let (Type::Instance(sub), Type::Instance(sup)) = (&actual, expected) {
            match self.class_table.is_subtype(sub, sup) {
                Some(true) | None => return,
                Some(false) => {}
            }
        }
        let notes = self.mismatch_notes(node, &actual);
        self.warn_with_notes(
            format!(
                "type mismatch: expected `{}`, found `{}`",
                expected.name(),
                actual.name()
            ),
            node.source_info.as_ref(),
            notes,
        );
    }

    /// Why-chain notes for a type mismatch (Phase 4 provenance): if the offending expression is a
    /// local read, point back at where that local got its type (`` `x` is `String` (inferred from
    /// `name`) ``). Empty for literals/other expressions — their type is self-evident at the site.
    fn mismatch_notes(&self, node: &Node, actual: &Type) -> Vec<Note> {
        if let NodeValue::Identifier(id) = &node.value
            && let Some(NarrowKey::Local(name)) = NarrowKey::from_ident(id)
            && let Some(prov) = self.local_provenance(&name)
        {
            return vec![Note {
                message: format!("`{}` is `{}` ({})", name, actual.name(), prov.origin),
                span: Some(prov.span.clone()),
            }];
        }
        Vec::new()
    }

    /// Compile a returned value (`^expr` / `^^expr`), checked and promoted against the innermost
    /// declared return type on `return_type_stack`. `None` → compile normally, unchecked.
    fn compile_return_value(
        &mut self,
        value: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        match self.return_type_stack.last().cloned().flatten() {
            Some(expected) => self.compile_expecting(value, &expected, bytecode),
            None => self.compile_node(value, bytecode),
        }
    }

    /// Compile-time MessageNotUnderstood: warn when a send targets a selector the receiver's class
    /// provably doesn't respond to. Sound only for an authoritative (`from_vm`), `sealed`, catch-all-
    /// free class — otherwise a future extension or dynamic handler could resolve it, so we stay
    /// silent (a missed MNU is fine; a wrong one is not). Resolution reuses `responds_to`, which is
    /// the VM's own dispatch walk.
    fn check_mnu(&mut self, call: &MethodCallNode) {
        let Some(class) = self.receiver_class(call) else {
            return;
        };
        let Some(sig) = self.class_table.get(&class) else {
            return;
        };
        if !sig.from_vm || !sig.sealed || sig.has_catch_all {
            return;
        }
        let Some(selector) = Self::call_selector_simple(call) else {
            return;
        };
        if self.class_table.responds_to(&class, &selector) == Some(false) {
            self.warn(
                format!("`{class}` does not respond to `{selector}`"),
                call.subject.as_deref().and_then(|n| n.source_info.as_ref()),
            );
        }
    }

    /// The receiver's concrete class name, if statically known. Only a user-class `Instance` —
    /// builtins aren't `sealed` (so MNU never fires on them), and `Any`/nullable receivers skip.
    fn receiver_class(&self, call: &MethodCallNode) -> Option<String> {
        match self.static_type(call.subject.as_ref()?) {
            Type::Instance(c) => Some(c.to_string()),
            _ => None,
        }
    }

    /// The canonical dispatched selector for a call — but only for the unambiguous shapes (unary, or
    /// a single keyword with one argument). Multi-keyword and variadic runs (which fold to `name+:`)
    /// return `None`, so MNU never reconstructs a selector that could differ from dispatch's.
    fn call_selector_simple(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if call.arguments.expressions.is_empty() {
            return idents.first().map(|i| i.name.clone());
        }
        if idents.len() == 1 && call.arguments.expressions.len() == 1 {
            return Some(format!("{}:", idents[0].name));
        }
        None
    }

    /// The canonical non-variadic selector of a call *with* args (`foo:` / `foo:bar:`). `None` for a
    /// no-arg call, or any variadic run (a repeated consecutive keyword, which folds to `name+:`).
    fn call_selector_nonvariadic(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if call.arguments.expressions.is_empty() || idents.len() != call.arguments.expressions.len()
        {
            return None;
        }
        if idents.windows(2).any(|w| w[0].name == w[1].name) {
            return None; // a variadic run — its dispatched selector is `name+:`
        }
        Some(idents.iter().map(|i| format!("{}:", i.name)).collect())
    }

    /// The declared parameter types for a call, when they're checkable: the receiver is an
    /// authoritative (`from_vm`), `sealed` class, and the (non-variadic) selector resolves to a
    /// single fully-typed method whose arity matches. `None` → args compile unchecked (gradual).
    fn call_param_types(&self, call: &MethodCallNode) -> Option<Vec<Type>> {
        let class = self.receiver_class(call)?;
        let sig = self.class_table.get(&class)?;
        if !sig.from_vm || !sig.sealed {
            return None;
        }
        let selector = Self::call_selector_nonvariadic(call)?;
        let params = self.class_table.own_method_params(&class, &selector)?;
        (params.len() == call.arguments.expressions.len()).then_some(params)
    }

    /// The call's declared parameter types with the receiver's element type already bound
    /// (PARTIAL substitution — unbound variables stay variables). The front half of
    /// `typed_receiver_return_type`, run BEFORE the args compile so a block-literal argument
    /// can carry its declared `Block(…)` shape into `compile_block` as the expectation
    /// (G4b, GENERICS_ARCH.md §11.3). `None` = no declaration found (gradual).
    fn receiver_bound_param_types(&self, call: &MethodCallNode) -> Option<Vec<Type>> {
        let subject = call.subject.as_ref()?;
        let recv_t = self.static_type(subject);
        let (class_name, recv_elem) = match &recv_t {
            Type::Any | Type::Never | Type::Nullable(_) => return None,
            Type::ListOf(e) => ("List".to_string(), Some((**e).clone())),
            Type::MapOf(e) => ("Map".to_string(), Some((**e).clone())),
            Type::SetOf(e) => ("Set".to_string(), Some((**e).clone())),
            concrete => (concrete.name(), None),
        };
        let selector = Self::reconstruct_send_selector(call)?;
        let (params, defining) = self
            .class_table
            .declared_params_with_source(&class_name, &selector)?;
        if params.len() != call.arguments.expressions.len() {
            return None;
        }
        let def_params = self.class_table.type_params_of(&defining);
        let mut bindings: std::collections::HashMap<Arc<str>, Type> =
            std::collections::HashMap::new();
        // The same Map nuance as `typed_receiver_return_type`: a Map's iteration
        // element is a key/value pair, so a MapOf receiver binds only Map's own methods.
        let elem_binds = !(matches!(recv_t, Type::MapOf(_)) && defining.as_ref() != "Map");
        if let (true, Some(elem), Some(p0)) = (elem_binds, recv_elem, def_params.first()) {
            bindings.insert(p0.clone(), elem);
        }
        Some(
            params
                .iter()
                .map(|p| p.substitute_bound(&bindings))
                .collect(),
        )
    }

    /// Recognize a nil-condition on a narrowable path (Phase 3c): `RECV.defined?`, or `RECV == nil`
    /// / `RECV != nil` (either operand order). Returns the path and whether a *true* result means
    /// RECV is non-nil (`.defined?` and `!= nil` → `true`; `== nil` → `false`).
    fn match_nil_condition(node: &Node) -> Option<(NarrowKey, bool)> {
        // `RECV.defined?` → a true result means RECV is non-nil.
        if let Some((recv, "defined?")) = as_unary_send(node) {
            return Some((as_path(recv)?, true));
        }
        // `RECV == nil` (⇒ nil) / `RECV != nil` (⇒ non-nil), either operand order.
        if let NodeValue::BinaryOperator(op) = &node.value
            && matches!(
                op.operator,
                BinaryOperatorType::Eq | BinaryOperatorType::NotEq
            )
        {
            return Some((
                Self::nil_comparison_key(&op.left, &op.right)?,
                op.operator == BinaryOperatorType::NotEq,
            ));
        }
        None
    }

    /// One operand is the reserved `nil`, the other a narrowable path → that path.
    fn nil_comparison_key(a: &Node, b: &Node) -> Option<NarrowKey> {
        if is_nil_literal(a) {
            as_path(b)
        } else if is_nil_literal(b) {
            as_path(a)
        } else {
            None
        }
    }

    /// A path's type at the current point: its flow-narrowed type if any, else the recorded local
    /// type (a field carries none → `Any`).
    fn path_type(&self, key: &NarrowKey) -> Type {
        self.narrowed_type(key).unwrap_or_else(|| match key {
            NarrowKey::Local(name) => self.local_type(name),
            NarrowKey::Field(_) => Type::Any,
        })
    }

    /// If `call` is a nil-guard conditional (`RECV.defined?` composed with `.if:`/`.if:else:`/
    /// `.else:`) whose path is currently `Nullable(T)`, the per-arm refinement. `None` otherwise —
    /// so narrowing only fires on a declared-nullable path, never on the optimizer's inferred types.
    fn guard_narrowing(&self, call: &MethodCallNode) -> Option<GuardInfo> {
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if !matches!(kws.as_slice(), ["if"] | ["if", "else"] | ["else"]) {
            return None;
        }
        let (key, true_is_nonnil) = Self::match_nil_condition(call.subject.as_deref()?)?;
        let Type::Nullable(inner) = self.path_type(&key) else {
            return None;
        };
        let non_nil = *inner;
        let (if_arm, else_arm) = if true_is_nonnil {
            (non_nil, Type::Nil)
        } else {
            (Type::Nil, non_nil)
        };
        Some(GuardInfo {
            key,
            if_arm,
            else_arm,
        })
    }

    /// Does this arm expression always exit non-locally (its tail is `^^`/`^`)? Used for post-guard
    /// narrowing: when the nil-arm diverges, the surviving arm's refinement holds afterward.
    fn expr_diverges(node: &Node) -> bool {
        let NodeValue::Block(b) = &node.value else {
            return false;
        };
        matches!(
            b.statements.last().map(|s| &s.value),
            Some(NodeValue::MethodReturn(_)) | Some(NodeValue::BlockReturn(_))
        )
    }

    /// After a guard send, merge the arms' exit states into the enclosing scope (Phase 3c
    /// join/merge). The conditional has two paths — condition true (the `if:` block, or a straight
    /// fall-through with the guard's true refinement when there's no `if:`) and condition false
    /// (the `else:` block, or a fall-through with the false refinement). A path whose arm diverges
    /// (`^^`/`^`) drops out; the guarded key's type afterward is the **join** of the surviving
    /// paths' exit types (`if_exit`/`else_exit` are those arms' captured exits, defaulting to the
    /// bare refinement). Both diverging ⇒ the code after is unreachable, so nothing is installed.
    fn apply_guard_join(
        &mut self,
        call: &MethodCallNode,
        g: &GuardInfo,
        if_exit: Option<Type>,
        else_exit: Option<Type>,
    ) {
        let idents = &call.arguments.signature.identifiers;
        let arm = |kw: &str| idents.iter().position(|i| i.name == kw);
        let diverges = |k: usize| Self::expr_diverges(&call.arguments.expressions[k]);

        let true_exit = match arm("if") {
            Some(k) if diverges(k) => None,
            Some(_) => Some(if_exit.unwrap_or_else(|| g.if_arm.clone())),
            None => Some(g.if_arm.clone()), // no `if:` block ⇒ true path falls through
        };
        let false_exit = match arm("else") {
            Some(k) if diverges(k) => None,
            Some(_) => Some(else_exit.unwrap_or_else(|| g.else_arm.clone())),
            None => Some(g.else_arm.clone()), // no `else:` block ⇒ false path falls through
        };

        let joined = match (true_exit, false_exit) {
            (Some(a), Some(b)) => Some(a.join(&b)),
            (Some(t), None) | (None, Some(t)) => Some(t),
            (None, None) => None,
        };
        if let Some(ty) = joined {
            self.update_narrowing(g.key.clone(), ty);
        }
    }

    /// Flow-update a *declared* path's narrowing after a (re)assignment (Phase 3c): a concrete
    /// rvalue type re-narrows it; an `Any` (unknown) rvalue drops to gradual. Only called for
    /// declared targets, so an untyped `var`'s inferred type is never shadowed.
    fn update_narrowing(&mut self, key: NarrowKey, ty: Type) {
        let scope = self.scopes.last_mut().unwrap();
        if ty == Type::Any {
            scope.narrowed.remove(&key);
        } else {
            scope.narrowed.insert(key, ty);
        }
    }

    /// G2: warn when an insertion into a statically-checked collection would
    /// raise the runtime tag TypeError — `xs.add:'s'` where `xs: List(Integer)`.
    /// Mirrors the runtime check exactly: nil always passes (the element
    /// position is honestly `T?`), and a variable-typed element claims nothing.
    fn check_generic_insertion(&mut self, call: &MethodCallNode) {
        let Some(subject) = call.subject.as_deref() else {
            return;
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return;
        };
        let recv_t = self.static_type(subject);
        let (elem, arg_idx) = match (&recv_t, selector.as_str()) {
            (Type::ListOf(e), "add:" | "push:") => ((**e).clone(), 0),
            (Type::ListOf(e), "at:put:") => ((**e).clone(), 1),
            (Type::SetOf(e), "add:") => ((**e).clone(), 0),
            (Type::MapOf(e), "at:put:") => ((**e).clone(), 1),
            _ => return,
        };
        if elem.contains_var() {
            return;
        }
        let Some(arg) = call.arguments.expressions.get(arg_idx) else {
            return;
        };
        let actual = self.static_type(arg);
        let allowed = Type::Nullable(Box::new(elem.clone()));
        if actual.compatible_with(&allowed) {
            return;
        }
        // Instance subtyping may rescue (a Circle into List(Shape)).
        if let (Type::Instance(sub), Type::Instance(sup)) = (&actual, &elem) {
            if self.class_table.is_subtype(sub, sup) != Some(false) {
                return;
            }
        }
        self.warn(
            format!(
                "`{}` rejects a `{}` element — this raises a TypeError at runtime",
                recv_t.name(),
                actual.name(),
            ),
            arg.source_info.as_ref(),
        );
    }

    /// Phase 3c: warn on a non-nil-safe send to a receiver whose current (narrowed) type is
    /// confidently `Nullable(T)` — `nil.<sel>` would fail at runtime. Gated to explicit `T?` /
    /// narrowed paths (silent on `Any`), so it speaks only on opt-in nullable annotations, and a
    /// guarded (narrowed non-nil) receiver reads as `T` here and is silent.
    fn check_nil_misuse(&mut self, call: &MethodCallNode) {
        let Some(subject) = call.subject.as_deref() else {
            return; // a self-send has no nullable receiver
        };
        if !matches!(self.static_type(subject), Type::Nullable(_)) {
            return;
        }
        let idents = &call.arguments.signature.identifiers;
        // A nil-safe unary method (`s`, `class`, `defined?`, …) doesn't dereference the receiver.
        if call.arguments.expressions.is_empty()
            && let Some(first) = idents.first()
            && NIL_SAFE_SELECTORS.contains(&first.name.as_str())
        {
            return;
        }
        let selector = if call.arguments.expressions.is_empty() {
            idents.first().map(|i| i.name.clone()).unwrap_or_default()
        } else {
            Self::call_selector_nonvariadic(call).unwrap_or_else(|| {
                format!(
                    "{}:",
                    idents.first().map(|i| i.name.as_str()).unwrap_or("?")
                )
            })
        };
        self.warn(
            format!("receiver of `{selector}` may be nil"),
            subject.source_info.as_ref(),
        );
    }

    /// Phase 3c: warn on a nil-dereferencing binary op whose left operand is confidently nullable
    /// (`x + 1` where `x: Integer?`). Equality and logical ops tolerate a `nil` left and are exempt.
    fn check_binop_nil_misuse(&mut self, op: &BinaryOperatorNode) {
        use BinaryOperatorType::*;
        if matches!(op.operator, Eq | NotEq | And | Or | Unknown) {
            return;
        }
        if matches!(self.static_type(&op.left), Type::Nullable(_)) {
            self.warn(
                format!(
                    "left operand of `{}` may be nil",
                    Self::binop_symbol(&op.operator)
                ),
                op.left.source_info.as_ref(),
            );
        }
    }

    fn binop_symbol(op: &BinaryOperatorType) -> &'static str {
        use BinaryOperatorType::*;
        match op {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Mod => "%",
            Gt => ">",
            GtEq => ">=",
            Lt => "<",
            LtEq => "<=",
            Range => "..",
            Match => "=~",
            Eq => "==",
            NotEq => "!=",
            And => "&&",
            Or => "||",
            Unknown => "?",
        }
    }

    /// Install the *true*-branch refinement of a nil-condition into the current scope, returning
    /// what to restore. Used for `&&` short-circuit narrowing (`RECV.defined? && EXPR`).
    fn push_true_narrowing(&mut self, cond: &Node) -> Option<(NarrowKey, Option<Type>)> {
        let (key, true_is_nonnil) = Self::match_nil_condition(cond)?;
        let Type::Nullable(inner) = self.path_type(&key) else {
            return None;
        };
        let refined = if true_is_nonnil { *inner } else { Type::Nil };
        let scope = self.scopes.last_mut().unwrap();
        let saved = scope.narrowed.get(&key).cloned();
        scope.narrowed.insert(key.clone(), refined);
        Some((key, saved))
    }

    fn pop_narrowing(&mut self, restore: Option<(NarrowKey, Option<Type>)>) {
        if let Some((key, saved)) = restore {
            let scope = self.scopes.last_mut().unwrap();
            match saved {
                Some(t) => scope.narrowed.insert(key, t),
                None => scope.narrowed.remove(&key),
            };
        }
    }

    /// The non-fatal type diagnostics collected during compilation (Phase 2 warnings).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Use a shared class-name accumulator instead of this compiler's own. The runner threads
    /// one set through every unit (via the VM), so a later unit sees the classes that
    /// earlier-compiled units — the prelude, `use`d modules — defined.
    pub fn set_seen_types(&mut self, seen: SeenTypes) {
        self.seen_types = seen;
    }

    /// Use a shared class-signature table (threaded alongside `seen_types`).
    pub fn set_class_table(&mut self, table: ClassTable) {
        self.class_table = table;
    }

    /// Statically infer an expression's type for devirtualization. Conservative: only
    /// literals, typed locals/params, and numeric operators on them are known; anything
    /// else is `Unknown` and compiles to a normal dynamic `Send`.
    fn static_type(&self, node: &Node) -> Type {
        match &node.value {
            // Literals synthesize their builtin type. (Only `Int`/`List`/`Bool` drive devirt;
            // the rest are inert there but let the checker see real mismatches — Phase 3a.)
            NodeValue::Integer(_) => Type::Int,
            NodeValue::Double(_) => Type::Double,
            NodeValue::Str(_) => Type::String,
            NodeValue::List(_) => Type::List,
            NodeValue::Map(_) => Type::Map,
            NodeValue::Set(_) => Type::Set,
            // A block literal is bare `Block` until its body compiles; after that, its sharpened
            // `Block(args ^Ret)` shape if it has one (G4b block-literal inference, §11.3).
            NodeValue::Block(b) => self
                .block_literal_types
                .get(&(b as *const BlockNode as usize))
                .cloned()
                .unwrap_or(Type::Block),
            NodeValue::Identifier(id) => match NarrowKey::from_ident(id) {
                // A narrowable read (local or `@field`): its flow-narrowed type if any (Phase 3c),
                // else the recorded local type (a field carries none → `Any`).
                Some(key) => self.path_type(&key),
                // Not narrowable: `nil`/`true`/`false` are reserved names (they parse as plain
                // idents, so match by name); everything else (globals/namespaced) is unknown here.
                None => match id.name.as_str() {
                    "nil" => Type::Nil,
                    "true" | "false" => Type::Bool,
                    _ => Type::Any,
                },
            },
            NodeValue::BinaryOperator(op) => self.binop_result_type(op),
            // A send's static type: a self-send to a current-class method, else the receiver's
            // own/inherited declared return (known-typed receiver), else an Object-rooted return
            // (universal, any receiver). Each is a safe miss → `Any`, so they layer by confidence.
            NodeValue::MethodCall(call) => match self.self_send_return_type(call) {
                Type::Any => match self.construction_return_type(call) {
                    Type::Any => match self.typed_receiver_return_type(call) {
                        Type::Any => self.object_rooted_return_type(call),
                        t => t,
                    },
                    t => t,
                },
                t => t,
            },
            _ => Type::Any,
        }
    }

    /// A self-send (`.sel:(…)` — no explicit receiver, or an explicit `self`) to a
    /// current-class method with a declared return type is statically that type. Non-self
    /// sends, unknown selectors, and variadic sends stay `Unknown` (a safe miss).
    fn self_send_return_type(&self, call: &MethodCallNode) -> Type {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        if !is_self {
            return Type::Any;
        }
        let Some(ctx) = self.class_ctx.last() else {
            return Type::Any;
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        ctx.returns.get(&selector).cloned().unwrap_or(Type::Any)
    }

    /// Reconstruct a send's selector from its arguments — the bare name for a unary send, the
    /// joined `name:` parts for a keyword send. `None` for an empty signature. A variadic run
    /// (a keyword repeated, dispatched as `name+:`) isn't reconstructed, so such a send simply
    /// misses — a safe `Any` rather than a mismatched selector.
    fn reconstruct_send_selector(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if idents.is_empty() {
            return None;
        }
        Some(if call.arguments.expressions.is_empty() {
            idents[0].name.clone()
        } else {
            idents.iter().map(|i| format!("{}:", i.name)).collect()
        })
    }

    /// The static return type of a send whose *receiver* has a known concrete type: the receiver
    /// class's own or inherited declared return for the selector (`list.count` → `Integer`,
    /// `d.floor` → `Integer`, `set.contains?:x` → `Boolean`). `Any` when the receiver's type is
    /// unknown/nullable or no return is declared. Sound like the Object-rooted path — return
    /// covariance guarantees any override returns a compatible type, so the declared return
    /// bounds the actual one.
    fn typed_receiver_return_type(&self, call: &MethodCallNode) -> Type {
        let Some(subject) = &call.subject else {
            return Type::Any;
        };
        // Only a receiver with a known concrete class qualifies; a nullable receiver's send is the
        // nil-misuse check's concern, not typed here. A checked collection looks up under its BASE
        // class and carries its element type into type-variable binding (GENERICS_ARCH.md §4.4).
        let recv_t = self.static_type(subject);
        let (class_name, recv_elem) = match &recv_t {
            Type::Any | Type::Never | Type::Nullable(_) => return Type::Any,
            Type::ListOf(e) => ("List".to_string(), Some((**e).clone())),
            Type::MapOf(e) => ("Map".to_string(), Some((**e).clone())),
            Type::SetOf(e) => ("Set".to_string(), Some((**e).clone())),
            concrete => (concrete.name(), None),
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        let Some((ret, defining)) = self
            .class_table
            .declared_return_with_source(&class_name, &selector)
        else {
            return Type::Any;
        };
        if !ret.contains_var() {
            return ret;
        }
        // Bind the defining class's variables at this call site: the receiver's
        // element type binds the FIRST header parameter; declared param types
        // (if recorded) unify structurally against the arguments' static types.
        let def_params = self.class_table.type_params_of(&defining);
        let mut bindings: std::collections::HashMap<Arc<str>, Type> =
            std::collections::HashMap::new();
        // A Map's tag is its VALUE type, but its ITERATION element is a
        // key/value pair — so a MapOf receiver binds only methods Map itself
        // defines (`at:` → V?); an inherited/mixin method (Iterate's
        // combinators) must not claim the value type for pair elements.
        let elem_binds = !(matches!(recv_t, Type::MapOf(_)) && defining.as_ref() != "Map");
        if let (true, Some(elem), Some(p0)) = (elem_binds, recv_elem, def_params.first()) {
            bindings.insert(p0.clone(), elem);
        }
        if let Some(decl_params) = self.class_table.own_method_params_of(&defining, &selector) {
            let args = &call.arguments.expressions;
            for (decl, arg) in decl_params.iter().zip(args.iter()) {
                Type::unify_into(decl, &self.static_type(arg), &mut bindings);
            }
        }
        Self::normalize_any_elems(ret.substitute(&bindings))
    }

    /// `List(Any)` (an unbound variable after substitution) is just `List` —
    /// don't let inference mint element claims out of nothing.
    fn normalize_any_elems(t: Type) -> Type {
        match t {
            Type::ListOf(e) if *e == Type::Any => Type::List,
            Type::MapOf(e) if *e == Type::Any => Type::Map,
            Type::SetOf(e) if *e == Type::Any => Type::Set,
            Type::Nullable(inner) => match Self::normalize_any_elems(*inner) {
                Type::Any => Type::Any,
                t => Type::Nullable(Box::new(t)),
            },
            other => other,
        }
    }

    /// Construction inference for the checked-conversion surface: `List.of:X`,
    /// `Map.of:X`, `Set.of:X`, and `recv.ensure:X` — the element class is a
    /// statically-visible Identifier argument, so the result types as the
    /// checked collection (GENERICS_ARCH.md §7.1's static sources).
    fn construction_return_type(&self, call: &MethodCallNode) -> Type {
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        if selector != "of:" && selector != "ensure:" {
            return Type::Any;
        }
        let Some(subject) = &call.subject else {
            return Type::Any;
        };
        let [arg] = call.arguments.expressions.as_slice() else {
            return Type::Any;
        };
        let NodeValue::Identifier(elem_id) = &arg.value else {
            return Type::Any;
        };
        let elem = Type::from_annotation_name(&ident_name(elem_id));
        if matches!(elem, Type::Any | Type::Nil | Type::Never) {
            return Type::Any;
        }
        let base = if selector == "of:" {
            // `List.of:X` — the receiver is the collection class itself.
            match &subject.value {
                NodeValue::Identifier(id) => ident_name(id),
                _ => return Type::Any,
            }
        } else {
            // `xs.ensure:X` — the receiver is a collection value.
            match self.static_type(subject) {
                Type::List | Type::ListOf(_) => "List".to_string(),
                Type::Map | Type::MapOf(_) => "Map".to_string(),
                Type::Set | Type::SetOf(_) => "Set".to_string(),
                _ => return Type::Any,
            }
        };
        match base.as_str() {
            "List" => Type::ListOf(Box::new(elem)),
            "Map" => Type::MapOf(Box::new(elem)),
            "Set" => Type::SetOf(Box::new(elem)),
            _ => Type::Any,
        }
    }

    /// The static return type of a no-arg send whose selector is declared on `Object`, the
    /// universal root — e.g. `x.defined?` → `Boolean`. Sound for *any* receiver because the
    /// return-covariance check (Phase 3c·4b) guarantees every override returns a compatible type.
    /// This is what lets narrowing/nil-misuse see through a `.defined?` guard and lets the guard
    /// devirt-inline as a real Bool conditional (Phase 3c·4c). Only `Object`-rooted selectors
    /// qualify, so it can't misjudge an unrelated same-named method on some other class.
    fn object_rooted_return_type(&self, call: &MethodCallNode) -> Type {
        if !call.arguments.expressions.is_empty() {
            return Type::Any;
        }
        let [sel] = call.arguments.signature.identifiers.as_slice() else {
            return Type::Any;
        };
        self.class_table
            .get("Object")
            .and_then(|s| s.method_returns.get(sel.name.as_str()).cloned())
            .unwrap_or(Type::Any)
    }

    /// Return-type covariance (the Liskov rule): a method that overrides an ancestor's method must
    /// return a type usable where the ancestor's *declared* return is expected — a subtype is fine,
    /// a widened or unrelated type is not. Warns on a confident violation, pointing at the override's
    /// `^Ret` annotation. Gradual: silent unless both returns are known and the mismatch can't be
    /// explained by class subtyping. This is what makes `Object#defined? : Boolean` a contract every
    /// override must honor, so `x.defined?` is soundly `Boolean` (Phase 3c·4b). `class_name` and its
    /// ancestors must already be in the class table (true at the class's compile site).
    fn check_return_covariance(&mut self, class_name: &str, block: &BlockNode) {
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            let Some(rt) = &blk.return_type else { continue };
            let Ok(selector) = self.reconstruct_selector(sig) else {
                continue;
            };
            let Some((base, from)) = self.class_table.inherited_return(class_name, &selector)
            else {
                continue;
            };
            let over = type_from_ref_with_vars(rt, &self.ctx_type_params());
            if self.override_return_violates(&over, &base) {
                self.warn(
                    format!(
                        "override of `{}` returns `{}`, incompatible with `{}` from `{}`",
                        selector,
                        over.name(),
                        base.name(),
                        from,
                    ),
                    rt.ident.source_info.as_ref(),
                );
            }
        }
    }

    /// Is an override returning `over` a *confident* covariance violation against a base return
    /// `base`? Only speaks when sure — a scalar mismatch (no class subtyping can rescue it) or a
    /// *proven* non-subtype between two bare classes. Anything the type/class lattice can't
    /// adjudicate (mixed class/scalar, nullable-of-class, unknown classes) stays silent (no FP).
    fn override_return_violates(&self, over: &Type, base: &Type) -> bool {
        if over.compatible_with(base) {
            return false; // Any/Never/exact/nullable-rules all fit
        }
        if Self::type_is_class_free(over) && Self::type_is_class_free(base) {
            return true; // e.g. `String` where `Boolean` is declared
        }
        if let (Type::Instance(o), Type::Instance(b)) = (over, base) {
            // Covariant returns permit a subtype; only a proven non-subtype is a violation.
            return self.class_table.is_subtype(o, b) == Some(false);
        }
        false
    }

    /// Does `ty` mention no class name (recursing through `Nullable`)? Such types have no subtype
    /// relation beyond `compatible_with`, so an incompatibility between two of them is definite.
    fn type_is_class_free(ty: &Type) -> bool {
        match ty {
            Type::Instance(_) => false,
            Type::Nullable(inner) => Self::type_is_class_free(inner),
            _ => true,
        }
    }

    /// Static result type of a binary operator. Comparison/equality operators yield `Bool`
    /// for *any* operands (Slice 2d, option B) — a language guarantee that they return
    /// `Boolean`, which lets `(a < b).if:…` / `(x == y).if:…` inline even when the operands
    /// aren't statically typed. Arithmetic yields `Int` only when *both* operands are
    /// statically `Int` — the soundness condition for devirtualizing to the direct i64 ops.
    /// Everything else (incl. `~`/`..`, and `&&`/`||`, which return an operand value not a
    /// `Bool`) stays `Unknown`.
    fn binop_result_type(&self, op: &BinaryOperatorNode) -> Type {
        use BinaryOperatorType::*;
        match op.operator {
            Lt | LtEq | Gt | GtEq | Eq | NotEq => Type::Bool,
            Add | Sub | Mul | Div | Mod
                if self.static_type(&op.left) == Type::Int
                    && self.static_type(&op.right) == Type::Int =>
            {
                Type::Int
            }
            Add | Sub | Mul | Div | Mod
                if self.static_type(&op.left) == Type::Double
                    && self.static_type(&op.right) == Type::Double =>
            {
                Type::Double
            }
            _ => Type::Any,
        }
    }

    /// The devirtualized Integer instruction for a binary operator, if it has one.
    fn int_devirt_op(operator: &BinaryOperatorType) -> Option<Instruction> {
        use BinaryOperatorType::*;
        Some(match operator {
            Add => Instruction::IntAdd,
            Sub => Instruction::IntSub,
            Mul => Instruction::IntMul,
            Div => Instruction::IntDiv,
            Mod => Instruction::IntMod,
            Lt => Instruction::IntLt,
            LtEq => Instruction::IntLe,
            Gt => Instruction::IntGt,
            GtEq => Instruction::IntGe,
            Eq => Instruction::IntEq,
            NotEq => Instruction::IntNe,
            _ => return None,
        })
    }

    /// The devirtualized Double instruction for a binary operator, if it has one (the f64 mirror
    /// of `int_devirt_op`).
    fn double_devirt_op(operator: &BinaryOperatorType) -> Option<Instruction> {
        use BinaryOperatorType::*;
        Some(match operator {
            Add => Instruction::DoubleAdd,
            Sub => Instruction::DoubleSub,
            Mul => Instruction::DoubleMul,
            Div => Instruction::DoubleDiv,
            Mod => Instruction::DoubleMod,
            Lt => Instruction::DoubleLt,
            LtEq => Instruction::DoubleLe,
            Gt => Instruction::DoubleGt,
            GtEq => Instruction::DoubleGe,
            Eq => Instruction::DoubleEq,
            NotEq => Instruction::DoubleNe,
            _ => return None,
        })
    }

    pub fn compile_program(&mut self, program: &ProgramNode) -> Result<StaticBlock, String> {
        self.compile_program_with(program, true)
    }

    /// Compile a top-level program. `define_self` emits the default top-level `self = nil`;
    /// pass `false` when the unit runs *as a method* with a receiver (`eval:self:`), where the
    /// frame setup (`start_block_as_method`) binds `self` to the receiver — otherwise this
    /// `self = nil` init would clobber it. `self` still compiles as a local either way
    /// (`is_local` special-cases it), resolving through the env (receiver, or nil when unbound).
    pub fn compile_program_with(
        &mut self,
        program: &ProgramNode,
        define_self: bool,
    ) -> Result<StaticBlock, String> {
        // Pre-scan this unit's class defs so annotations can forward-reference them (and so
        // later-compiled units see them via the shared `seen_types`).
        self.prescan_class_defs(program);
        let mut cb = CodeBlock::new();

        cb.current_source = program.source_info.clone();
        if define_self {
            cb.push(Instruction::Push(Constant::Nil));
            cb.push(Instruction::DefineLocal(Symbol::intern("self")));
            self.scopes[0].locals.insert("self".to_string());
        }

        let len = program.expressions.len();
        for (idx, expr) in program.expressions.iter().enumerate() {
            cb.current_source = expr.source_info.clone();
            self.compile_node(expr, &mut cb)?;
            if idx < len - 1 {
                cb.push(Instruction::Pop);
            }
        }

        cb.current_source = program.source_info.clone();
        if len == 0 {
            cb.push(Instruction::Push(Constant::Nil));
        }

        cb.push(Instruction::Return);

        let (bytecode, source_map) = fuse_bytecode(cb.bytecode, cb.source_map);
        Ok(StaticBlock {
            spec_state: Default::default(),
            name: None,
            is_nested_block: false,
            is_init_literal: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            param_elem_tags: Vec::new(),
            bytecode: SharedBytecode(Rc::new(bytecode)),
            source_info: program.source_info.clone(),
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(source_map)),
            template_id: self
                .mint_template_ids
                .then(crate::instruction::fresh_template_id),
        })
    }

    fn compile_node(&mut self, node: &Node, bytecode: &mut CodeBlock) -> Result<(), String> {
        let prev_source = bytecode.current_source.clone();
        bytecode.current_source = node.source_info.clone();
        let res = self.compile_node_internal(node, bytecode);
        bytecode.current_source = prev_source;
        res
    }

    fn compile_node_internal(
        &mut self,
        node: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        match &node.value {
            NodeValue::Integer(n) => {
                bytecode.push(Instruction::Push(Constant::Int(n.value)));
            }
            NodeValue::Double(d) => {
                bytecode.push(Instruction::Push(Constant::Double(d.value)));
            }
            NodeValue::Str(s) => {
                bytecode.push(Instruction::Push(Constant::String(s.value.clone())));
            }
            NodeValue::Symbol(s) => {
                bytecode.push(Instruction::Push(Constant::Symbol(s.value.clone())));
            }
            NodeValue::Identifier(id) => {
                if id.identifier_type == IdentifierType::Instance {
                    if self.value_type_def_depth > 0 {
                        return Err(format!(
                            "value types cannot have instance variables (found '@{}')",
                            id.name
                        ));
                    }
                    // Phase 5·3c: inside a spliced computed body, `@x` reads the override receiver's
                    // field, not the caller's `self`.
                    if let Some(over) = self.self_override {
                        bytecode.push(Instruction::LoadLocal(over));
                        bytecode.push(Instruction::LoadFieldOf(id.name.clone()));
                    } else {
                        bytecode.push(Instruction::LoadField(id.name.clone()));
                    }
                } else if id.name == "nil" || id.name == "true" || id.name == "false" {
                    match id.name.as_str() {
                        "nil" => bytecode.push(Instruction::Push(Constant::Nil)),
                        "true" => bytecode.push(Instruction::Push(Constant::Bool(true))),
                        "false" => bytecode.push(Instruction::Push(Constant::Bool(false))),
                        _ => unreachable!(),
                    }
                } else if let Some(&sym) = self.param_override.get(&id.name) {
                    // Phase 5·4: inside a spliced body, a param reference loads its bound-arg temp.
                    bytecode.push(Instruction::LoadLocal(sym));
                } else if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced
                {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                } else if self.is_local(&id.name) {
                    // Phase 5·3c: inside a spliced computed body, a bare `self` is the override.
                    let sym = match self.self_override {
                        Some(over) if id.name == "self" => over,
                        _ => Symbol::intern(&id.name),
                    };
                    bytecode.push(Instruction::LoadLocal(sym));
                } else {
                    let ns_name = NamespacedName::new(Vec::new(), id.name.clone());
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                }
            }
            NodeValue::Assignment(assign) => {
                self.compile_assignment(assign, bytecode)?;
            }
            NodeValue::Declaration(decl) => {
                self.compile_declaration(decl, bytecode)?;
            }
            NodeValue::MethodCall(call) => {
                self.compile_method_call(call, bytecode)?;
            }
            NodeValue::BinaryOperator(op) => {
                self.compile_binary_operator(op, bytecode)?;
            }
            NodeValue::UnaryOperator(op) => {
                self.compile_unary_operator(op, bytecode)?;
            }
            NodeValue::Block(block) => {
                self.compile_block(block, bytecode)?;
                // B3a: a block LITERAL is a block-template candidate (method
                // bodies are collected as Method candidates at their def site).
                if let Some(Instruction::Push(Constant::Block(rc))) = bytecode.bytecode.last() {
                    let rc = rc.clone();
                    self.maybe_collect_block_candidate(&rc);
                }
            }
            NodeValue::BlockReturn(ret) => {
                self.compile_return_value(&ret.value, bytecode)?;
                // Inside an inlined control-flow block (Slice 2d), `^expr` yields the
                // block's value and jumps past the inlined region rather than popping a
                // (now-absent) block frame; `inline_block_body` patches the placeholder.
                if let Some(positions) = self.inline_carets.as_mut() {
                    positions.push(bytecode.len());
                    bytecode.push(Instruction::Jump(0));
                } else {
                    // G4b: a real `^` is one of the enclosing block's return values —
                    // join it into the return harvest (§11.3). An inlined-region `^`
                    // (above) is the *conditional's* value, not a block return.
                    let t = self.static_type(&ret.value);
                    if let Some(h) = self.block_ret_harvest.last_mut() {
                        let joined = h.join(&t);
                        *h = joined;
                    }
                    bytecode.push(Instruction::BlockReturn);
                }
            }
            NodeValue::MethodReturn(ret) => {
                self.compile_return_value(&ret.value, bytecode)?;
                bytecode.push(Instruction::MethodReturn);
            }
            NodeValue::YieldReturn(ret) => {
                // `^> expr` is sugar for `Fiber.yield:expr`: suspend the current
                // fiber, hand `expr` out to the resumer, and evaluate to whatever
                // the next `resume:` passes back in.
                bytecode.push(Instruction::LoadGlobal(NamespacedName::new(
                    Vec::new(),
                    "Fiber".to_string(),
                )));
                self.compile_node(&ret.value, bytecode)?;
                bytecode.push(Instruction::Send(Symbol::intern("yield:"), 1));
            }
            NodeValue::List(list) => {
                for item in &list.values {
                    self.compile_node(item, bytecode)?;
                }
                bytecode.push(Instruction::NewList(list.values.len()));
            }
            NodeValue::Map(map) => {
                if map.keys.len() != map.values.len() {
                    return Err("Map keys and values count mismatch".to_string());
                }
                for i in 0..map.keys.len() {
                    self.compile_node(&map.keys[i], bytecode)?;
                    self.compile_node(&map.values[i], bytecode)?;
                }
                bytecode.push(Instruction::NewMap(map.keys.len()));
            }
            NodeValue::Set(set) => {
                for item in &set.values {
                    self.compile_node(item, bytecode)?;
                }
                bytecode.push(Instruction::NewSet(set.values.len()));
            }
            NodeValue::Regex(re) => {
                let mut pattern = re.value.clone();
                if pattern.starts_with("#/") && pattern.ends_with('/') {
                    pattern = pattern[2..pattern.len() - 1].to_string();
                }
                bytecode.push(Instruction::Push(Constant::String(pattern)));
                bytecode.push(Instruction::NewRegex);
            }
            NodeValue::ClassDefinition(class_def) => {
                let name = NamespacedName::from_ast(&class_def.identifier);
                // Checker/class-table key: the qualified form (`[Web]Halt`), matching the
                // `populate_from_vm` keying so AST- and VM-sourced sigs can't diverge.
                let class_name = name.to_string();
                // Record the class as known as soon as it's defined — covers classes in nested
                // blocks the top-level pre-scan can't reach (a def-before-use in any scope).
                self.seen_types.insert(&class_name);
                self.class_table
                    .insert(&class_name, self.class_sig_from_def(class_def));
                self.check_return_covariance(&class_name, &class_def.block);
                let parent_name = class_def
                    .parent_identifier
                    .as_ref()
                    .map(|id| NamespacedName::from_ast(id));
                let mut instance_vars = Vec::new();
                for arg in &class_def.block.arguments {
                    instance_vars.push(arg.identifier.name.clone());
                }
                let is_value_type = matches!(
                    class_name.as_str(),
                    "Integer" | "Double" | "Boolean" | "Nil"
                );
                if is_value_type && !instance_vars.is_empty() {
                    return Err(format!(
                        "value type '{}' cannot declare instance variables (@{})",
                        class_name, instance_vars[0]
                    ));
                }
                bytecode.push(Instruction::DefineClass {
                    name,
                    parent_name,
                    instance_vars,
                });
                if is_value_type {
                    self.value_type_def_depth += 1;
                }
                let ctx = self.collect_class_ctx(
                    &class_name,
                    &class_def.block,
                    class_def.type_params.clone(),
                );
                self.class_ctx.push(ctx);
                let r = self.compile_block(&class_def.block, bytecode);
                self.class_ctx.pop();
                if is_value_type {
                    self.value_type_def_depth -= 1;
                }
                r?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::ClassExtension(class_ext) => {
                // A `Foo <-- {}` reopen contributes its methods' declared returns AND params to
                // `Foo`'s signature — how the core classes (`Object <-- {}`, `List <-- {}`, …)
                // carry their contracts, since they're reopened rather than defined with `<-`
                // (Phase 3c·4; params for the G4b expectation channel, §11.3/§11.4). The
                // target's declared type parameters come from the table (a reopen header can't
                // declare them), so `^Set(T)` records as `Var("T")`, not a bogus `Instance`.
                if let NodeValue::Identifier(target) = &class_ext.expression.value {
                    let target_name = ident_name(target);
                    let vars: Vec<String> = self
                        .class_table
                        .type_params_of(&target_name)
                        .iter()
                        .map(|p| p.to_string())
                        .collect();
                    self.class_table.add_returns(
                        &target_name,
                        self.declared_method_returns_with_vars(&class_ext.block, &vars),
                    );
                    self.class_table.add_params(
                        &target_name,
                        self.declared_method_params(&class_ext.block, &vars),
                    );
                    // A reopen's `.mix:` runs at runtime, after the from_vm snapshot — record
                    // it now or the hierarchy walk can't reach the mixin's typed signatures.
                    let mixins: Vec<Arc<str>> = class_ext
                        .block
                        .statements
                        .iter()
                        .filter_map(|stmt| match &stmt.value {
                            NodeValue::MethodCall(call) => {
                                Self::mixin_target(call).map(|m| Arc::from(m.as_str()))
                            }
                            _ => None,
                        })
                        .collect();
                    self.class_table.add_mixins(&target_name, mixins);
                    self.check_return_covariance(&target_name, &class_ext.block);
                }
                self.compile_node(&class_ext.expression, bytecode)?;
                let is_value_type = Self::is_value_type_target(&class_ext.expression);
                if is_value_type {
                    if let Some(arg) = class_ext
                        .block
                        .arguments
                        .iter()
                        .find(|a| a.identifier.identifier_type == IdentifierType::Instance)
                    {
                        return Err(format!(
                            "value type cannot declare instance variables (@{})",
                            arg.identifier.name
                        ));
                    }
                    self.value_type_def_depth += 1;
                }
                let ext_name = match &class_ext.expression.value {
                    NodeValue::Identifier(id) => ident_name(id),
                    _ => String::new(),
                };
                let ext_params: Vec<String> = self
                    .class_table
                    .type_params_of(&ext_name)
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                let ctx = self.collect_class_ctx(&ext_name, &class_ext.block, ext_params);
                self.class_ctx.push(ctx);
                let r = self.compile_block(&class_ext.block, bytecode);
                self.class_ctx.pop();
                if is_value_type {
                    self.value_type_def_depth -= 1;
                }
                r?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::MethodDefinition(method_def) => {
                let selector = self.reconstruct_selector(&method_def.signature)?;
                self.compile_block(&method_def.block, bytecode)?;
                self.maybe_collect_aot_candidate(&selector, &method_def.block, bytecode);
                bytecode.push(Instruction::DefineMethod(selector));
            }
            NodeValue::MethodExtension(method_ext) => {
                let selector = self.reconstruct_selector(&method_ext.signature)?;
                self.compile_block(&method_ext.block, bytecode)?;
                bytecode.push(Instruction::OverrideMethod(selector));
            }
            NodeValue::ConstDefinition(const_def) => {
                let ns_name = NamespacedName::from_ast(&const_def.identifier);
                self.compile_node(&const_def.rvalue, bytecode)?;
                bytecode.push(Instruction::Dup);
                bytecode.push(Instruction::StoreGlobal(ns_name, true));
            }
            NodeValue::Use(use_node) => {
                bytecode.push(Instruction::Use {
                    package: use_node.package.clone(),
                    path: use_node.path.clone(),
                    glob: use_node.glob,
                });
            }
            NodeValue::UserString(user_str) => {
                let ns_name = NamespacedName::from_ast(&user_str.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                bytecode.push(Instruction::Push(Constant::String(user_str.value.clone())));
                bytecode.push(Instruction::Send(Symbol::intern("newUserString:"), 1));
            }
            NodeValue::UserList(user_list) => {
                let ns_name = NamespacedName::from_ast(&user_list.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                for val in &user_list.values {
                    self.compile_node(val, bytecode)?;
                }
                bytecode.push(Instruction::NewList(user_list.values.len()));
                bytecode.push(Instruction::Send(Symbol::intern("newUserList:"), 1));
            }
            NodeValue::Dot3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("...".to_string())));
                bytecode.push(Instruction::Send(Symbol::intern("throw"), 0));
            }
            NodeValue::Huh3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("???".to_string())));
                bytecode.push(Instruction::Send(Symbol::intern("throw"), 0));
            }
            NodeValue::Bang3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("!!!".to_string())));
                bytecode.push(Instruction::Send(Symbol::intern("throw"), 0));
            }
            NodeValue::Unknown => {
                return Err("Encountered Unknown NodeValue (ast_visitor bug)".to_string());
            }
            _ => {
                return Err(format!("Unsupported NodeValue: {:?}", node.value));
            }
        }
        Ok(())
    }

    fn compile_method_call(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Phase 3b: compile-time MNU (a pure analysis, before any inlining/lowering).
        self.check_mnu(call);
        // Phase 3c: a non-nil-safe send to a confidently-nullable, un-narrowed receiver.
        self.check_nil_misuse(call);
        self.check_generic_insertion(call);
        let args = &call.arguments;
        // A self-send (no explicit receiver, or an explicit `self`) — eligible for
        // devirtualization when the enclosing class is sealed (see `emit_call`).
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };

        // Slice 2d: inline `if:`/`if:else:` on a statically-Bool receiver with literal,
        // 0-arg, declaration-free block args into native jumps — no block allocation, no
        // dispatch, no block-invocation frame. Falls through to a normal send otherwise.
        if self.try_compile_inlined_conditional(call, bytecode)? {
            return Ok(());
        }
        if self.try_compile_inlined_while(call, bytecode)? {
            return Ok(());
        }
        // B1 (docs/BLOCK_AOT_ARCH.md §3): fuse `recv.each:{ |x| … }` into a guarded
        // native index loop — closure-free per element on any native-List receiver,
        // with the real send as the cold path (the guard IS the dispatch).
        if self.try_compile_inlined_each(call, bytecode)? {
            return Ok(());
        }
        // Phase 5·1/5·2: inline a self-send to a sealed class's own method with an inline-safe body
        // (`self.width` → the field load; `self.area` → `.width * .height`) — no receiver push, no
        // dispatch. Before the receiver is evaluated, since the inline replaces it entirely.
        if self.try_inline_self_send(call, is_self, bytecode)? {
            return Ok(());
        }
        // Phase 5·3/5·3b/5·3c: inline an explicit-receiver `v.foo` (field accessor, or a computed
        // body with `self` rebound to `v`) to a sealed in-unit class. Before the receiver push, since
        // the inline evaluates `v` itself.
        if self.try_inline_exact_receiver(call, bytecode)? {
            return Ok(());
        }

        // Evaluate receiver. Inside a spliced computed body (5·3c), a bare self-send targets the
        // override receiver, not the caller's `self`.
        if let Some(ref subject) = call.subject {
            self.compile_node(subject, bytecode)?;
        } else {
            bytecode.push(Instruction::LoadLocal(
                self.self_override.unwrap_or_else(|| Symbol::intern("self")),
            ));
        }

        // No-argument selector (unary / bang / symbol): a single component, no args.
        if args.expressions.is_empty() {
            if args.signature.identifiers.is_empty() {
                return Err("No identifiers found in method call selector".to_string());
            }
            let selector = args.signature.identifiers[0].name.clone();
            self.emit_call(bytecode, &selector, 0);
            return Ok(());
        }

        // Keyword send. Keywords and argument expressions are 1:1 here (the parser builds them in
        // lockstep). A run of the *same* consecutive keyword is a variadic group: its arguments
        // fold into one `List` and the keyword interns as `name+:`, matching a `name+:` method
        // definition. A lone keyword stays `name:`. This is resolved entirely at compile time, so
        // dispatch only ever sees a canonical interned selector — no runtime collapse.
        // Phase 3b arg-checks: when the receiver + method params are known, args are checked and
        // numeric literals promoted against them; otherwise compiled unchecked (gradual). `Some`
        // only for fully non-variadic calls, so `i + j` indexes `params` directly.
        let param_types = self.call_param_types(call);
        // G4b: the declared param types from the class-table walk, receiver-bound — feeds a
        // block-literal argument its declared `Block(…)` shape (§11.3). Computed once per call;
        // consulted only for literal block args below.
        let has_block_arg = call
            .arguments
            .expressions
            .iter()
            .any(|a| matches!(a.value, NodeValue::Block(_)));
        let block_expectations = has_block_arg
            .then(|| self.receiver_bound_param_types(call))
            .flatten();
        // Phase 3c: if this is a nil-guard conditional (`RECV.defined?.if:`/`.else:`), the per-arm
        // narrowing to install while compiling each arm, and post-guard on divergence.
        let guard = self.guard_narrowing(call);
        let idents = &args.signature.identifiers;
        debug_assert_eq!(idents.len(), args.expressions.len());
        let mut selector = String::new();
        let mut num_components = 0usize;
        // Phase 3c join/merge: each guard arm's captured exit narrowing for the guarded key.
        let mut if_exit: Option<Type> = None;
        let mut else_exit: Option<Type> = None;
        let mut i = 0;
        while i < idents.len() {
            // Extent of the run of the keyword at `i`.
            let mut run = 1;
            while i + run < idents.len() && idents[i + run].name == idents[i].name {
                run += 1;
            }
            // Evaluate this component's argument expression(s); a run folds into one list value.
            for j in 0..run {
                let arg = &args.expressions[i + j];
                // `X.new:{ … }` — the block argument is an object-initializer block, in
                // which a bare `field = value` binds an instance field (see compile_block
                // / Scope::is_init). Only a literal block gets the flag, and it's consumed
                // immediately by that block's compile_block, so it can't leak.
                if run == 1 && idents[i].name == "new" && matches!(arg.value, NodeValue::Block(_)) {
                    self.next_block_is_init = true;
                }
                // G4b: a literal block argument whose declared param is a `Block(…)` shape
                // compiles with that shape as its expectation — seeding its unannotated
                // params and closing the loop for `U`-binding (§11.3). One-shot, consumed
                // by the literal's own `compile_block`.
                if matches!(arg.value, NodeValue::Block(_))
                    && let Some(dp) = &block_expectations
                    && let Some(Type::BlockOf { params, .. }) = dp.get(i + j)
                {
                    self.next_block_expected = Some(params.clone());
                }
                // Phase 3c: narrow the guarded path inside this arm's block (`if` → non-nil arm,
                // `else` → nil arm). One-shot, consumed by the arm's `compile_block`. Also request a
                // snapshot of the arm's exit narrowing for the join/merge after the loop.
                let capture_this_arm = if let Some(g) = &guard
                    && matches!(arg.value, NodeValue::Block(_))
                    && let Some(arm_ty) = g.arm_type(&idents[i].name)
                {
                    self.next_block_narrowing = Some((g.key.clone(), arm_ty));
                    self.next_block_capture = Some(g.key.clone());
                    true
                } else {
                    false
                };
                match &param_types {
                    Some(params) => self.compile_expecting(arg, &params[i + j], bytecode)?,
                    None => self.compile_node(arg, bytecode)?,
                }
                if capture_this_arm {
                    let exit = self.captured_arm_exit.take();
                    match idents[i].name.as_str() {
                        "if" => if_exit = exit,
                        "else" => else_exit = exit,
                        _ => {}
                    }
                }
            }
            if run > 1 {
                bytecode.push(Instruction::NewList(run));
            }
            selector.push_str(&idents[i].name);
            if run > 1 {
                selector.push('+');
            }
            selector.push(':');
            num_components += 1;
            i += run;
        }

        // Phase 3c: after a guard send, merge the arms' exit states into the enclosing scope —
        // a diverging arm drops out (`x.defined?.else:{ ^^… }`), the surviving/fall-through paths
        // join. Both diverging ⇒ unreachable, no narrowing.
        if let Some(g) = &guard {
            self.apply_guard_join(call, g, if_exit, else_exit);
        }

        // Slice 2e: devirtualize `at:`/`at:put:`/`add:` when the receiver is statically a
        // `List`. The operands a send would consume are already on the stack in send order,
        // so the op is a drop-in replacement.
        if let Some(op) = self.collection_devirt_op(call, &selector, num_components) {
            bytecode.push(op);
            return Ok(());
        }

        self.emit_call(bytecode, &selector, num_components);
        Ok(())
    }

    fn compile_binary_operator(
        &mut self,
        op: &BinaryOperatorNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Phase 3c: a nil-dereferencing binop on a confidently-nullable left operand.
        self.check_binop_nil_misuse(op);

        if op.operator == BinaryOperatorType::And {
            self.compile_node(&op.left, bytecode)?;
            bytecode.push(Instruction::Dup);

            // Phase 3c: `RECV.defined? && EXPR` narrows RECV non-nil within EXPR (short-circuit).
            let restore = self.push_true_narrowing(&op.left);
            let mut right_bytecode = CodeBlock::new();
            right_bytecode.current_source = bytecode.current_source.clone();
            self.compile_node(&op.right, &mut right_bytecode)?;
            self.pop_narrowing(restore);

            let offset = 2 + right_bytecode.len() as isize;
            bytecode.push(Instruction::ElseJump(offset));
            bytecode.push(Instruction::Pop);
            bytecode.extend(right_bytecode);
            return Ok(());
        }

        if op.operator == BinaryOperatorType::Or {
            self.compile_node(&op.left, bytecode)?;
            bytecode.push(Instruction::Dup);

            let mut right_bytecode = CodeBlock::new();
            right_bytecode.current_source = bytecode.current_source.clone();
            self.compile_node(&op.right, &mut right_bytecode)?;

            let offset = 2 + right_bytecode.len() as isize;
            bytecode.push(Instruction::IfJump(offset));
            bytecode.push(Instruction::Pop);
            bytecode.extend(right_bytecode);
            return Ok(());
        }

        // Devirtualize when both operands are statically Integer: emit the direct i64 op
        // instead of a method send. Computed from the AST before compiling the operands
        // (no side effects). Integer is a sealed value type (see prelude.qn), so its
        // arithmetic operators can't be redefined — this is sound.
        // Integer and Double are sealed value types (prelude.qn), so their arithmetic operators
        // can't be redefined — devirt to a direct op when both operands are statically that same
        // type. Types computed from the AST before compiling the operands (no side effects); a
        // runtime type mismatch (stale inference) falls back to the real send.
        let (lt, rt) = (self.static_type(&op.left), self.static_type(&op.right));

        self.compile_node(&op.left, bytecode)?;
        self.compile_node(&op.right, bytecode)?;

        let devirt_op = if lt == Type::Int && rt == Type::Int {
            Self::int_devirt_op(&op.operator)
        } else if lt == Type::Double && rt == Type::Double {
            Self::double_devirt_op(&op.operator)
        } else {
            None
        };
        if let Some(op_instr) = devirt_op {
            bytecode.push(op_instr);
            return Ok(());
        }

        let selector = match op.operator {
            BinaryOperatorType::Add => "+:",
            BinaryOperatorType::Sub => "-:",
            BinaryOperatorType::Mul => "*:",
            BinaryOperatorType::Div => "/:",
            BinaryOperatorType::Eq => "==:",
            BinaryOperatorType::NotEq => "!=:",
            BinaryOperatorType::Lt => "<:",
            BinaryOperatorType::Gt => ">:",
            BinaryOperatorType::LtEq => "<=:",
            BinaryOperatorType::GtEq => ">=:",
            BinaryOperatorType::Mod => "%:",
            BinaryOperatorType::Match => "~:",
            BinaryOperatorType::Range => "..:",
            _ => {
                return Err(format!(
                    "Unsupported binary operator type: {:?}",
                    op.operator
                ));
            }
        };

        bytecode.push(Instruction::Send(Symbol::intern(selector), 1));
        Ok(())
    }

    fn compile_unary_operator(
        &mut self,
        op: &UnaryOperatorNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Compile operand (receiver)
        self.compile_node(&op.right, bytecode)?;

        match op.operator {
            UnaryOperatorType::Bang => {
                bytecode.push(Instruction::Send(Symbol::intern("!"), 0));
            }
            UnaryOperatorType::Sub => {
                bytecode.push(Instruction::Send(Symbol::intern("-"), 0));
            }
            UnaryOperatorType::Add => {
                bytecode.push(Instruction::Send(Symbol::intern("+"), 0));
            }
            UnaryOperatorType::Mod => {
                bytecode.push(Instruction::Send(Symbol::intern("mod"), 0));
            }
            _ => {
                return Err(format!(
                    "Unsupported unary operator type: {:?}",
                    op.operator
                ));
            }
        }
        Ok(())
    }

    fn compile_block(&mut self, block: &BlockNode, bytecode: &mut CodeBlock) -> Result<(), String> {
        // Consume the one-shot init-block flag (set by `compile_method_call` for a
        // `X.new:{ … }` argument) before anything can reset it; nested blocks compiled
        // within read it as `false`.
        let is_init = std::mem::take(&mut self.next_block_is_init);
        // Phase 3c: a guard arm's narrowing, installed into this block's scope below. Taken here
        // (one-shot) so nested blocks don't inherit it.
        let block_narrowing = std::mem::take(&mut self.next_block_narrowing);
        // G4b: the declared `Block(…)` shape this literal is being passed to, receiver-bound —
        // one-shot like the narrowing so nested blocks don't inherit it (§11.3).
        let expected_params = std::mem::take(&mut self.next_block_expected);
        // Phase 3c join/merge: the key whose exit narrowing this arm should snapshot. Taken at
        // entry (bound to THIS block) and read from its scope just before `pop_scope`, so the
        // snapshot reflects the arm's straight-line effect (guard refinement + top-level
        // reassignments); nested blocks pop first and don't consume it.
        let capture_key = std::mem::take(&mut self.next_block_capture);
        // A real block gets its own frame, so any enclosing inlined-region caret
        // redirection (Slice 2d) must not leak into it: a `^` here is a genuine
        // `BlockReturn` for this block. Cleared on entry, restored on exit.
        let saved_inline = self.inline_carets.take();
        let mut param_names = Vec::new();
        let mut param_types = Vec::new();
        let mut param_elem_tags: Vec<Option<ElemTag>> = Vec::new();
        let mut locals = HashSet::new();

        for arg in &block.arguments {
            let name = arg.identifier.name.clone();
            param_names.push(name.clone());
            // An unannotated parameter defaults to `Object` (the universal supertype),
            // so `|x|` and `|x:Object|` are the same signature everywhere downstream.
            let type_name = arg
                .type_hint
                .as_ref()
                .map(|tr| self.dispatch_type_name(tr))
                .unwrap_or_else(|| "Object".to_string());
            param_types.push(type_name);
            param_elem_tags.push(
                arg.type_hint
                    .as_ref()
                    .and_then(|tr| self.param_elem_tag(tr)),
            );
            locals.insert(name);
        }

        // All-None normalizes to empty: legacy blocks share one shape, dispatch
        // scoring skips tag work entirely on `is_empty`, and variant identity
        // compares equal across pre- and post-generics compiles.
        if param_elem_tags.iter().all(Option::is_none) {
            param_elem_tags.clear();
        }

        let mut decls_names = Vec::new();
        for decl in &block.decls {
            let name = decl.identifier.name.clone();
            decls_names.push(name.clone());
            locals.insert(name);
        }

        self.push_scope(locals);
        self.scopes.last_mut().unwrap().is_init = is_init;
        if let Some((key, ty)) = block_narrowing {
            self.scopes.last_mut().unwrap().narrowed.insert(key, ty);
        }

        // Seed declared param types so arithmetic on a typed param devirtualizes, and so the
        // annotation acts as a *contract*: a reassignment is checked against it and flow-updates the
        // param's narrowing (Phase 3c), exactly like a `var x: T` local. In the METHOD role,
        // dispatch only selects a typed method when the arg matches, so the param is provably that
        // type on entry — no runtime guard needed; a `value:`-invoked bare literal gets no such
        // check, and its seeding stays operationally safe only because the devirt ops it feeds are
        // value-guarded (GENERICS_ARCH.md §11.1). An *un-annotated* param is `Any` (gradual,
        // unchecked), NOT `Object` — the `Object` default above is only the runtime dispatch
        // signature, not a static type.
        //
        // `param_beliefs` doubles as this literal's outward param shape (§11.3): the explicit
        // annotation where present, else the expectation's seed, else `Any`.
        let mut param_beliefs: Vec<Type> = Vec::with_capacity(block.arguments.len());
        for (i, arg) in block.arguments.iter().enumerate() {
            if let Some(hint) = &arg.type_hint {
                let ty = self.resolve_annotation(hint);
                let prov = Self::provenance_from(
                    arg.identifier.source_info.clone(),
                    "parameter".to_string(),
                );
                self.record_declared_type(&arg.identifier.name, ty.clone(), prov);
                param_beliefs.push(ty);
                continue;
            }
            // G4b: an UNANNOTATED param seeds from the declared `Block(…)` shape this literal is
            // being passed to — a narrowing-grade belief: read by `static_type`/warnings/
            // nil-narrowing, dissolved by any reassignment, never a contract and never devirt
            // (§11.1). `T` not `T?`: elements present during iteration are never the OOB nil
            // (§10.3). A type still mentioning an unbound variable claims nothing.
            let seed = expected_params
                .as_ref()
                .and_then(|e| e.get(i))
                .filter(|t| !matches!(t, Type::Any) && !t.contains_var())
                .cloned();
            if let Some(ty) = &seed {
                self.scopes
                    .last_mut()
                    .unwrap()
                    .narrowed
                    .insert(NarrowKey::Local(arg.identifier.name.clone()), ty.clone());
            }
            param_beliefs.push(seed.unwrap_or(Type::Any));
        }

        let mut block_bytecode = CodeBlock::new();
        block_bytecode.current_source = block.source_info.clone();

        for name in &decls_names {
            block_bytecode.push(Instruction::Push(Constant::Nil));
            block_bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
        }

        // Phase 3a: check/promote returns against this block's declared return type (`|args ^T|`).
        let expected_ret = block
            .return_type
            .as_ref()
            .map(|rt| type_from_ref_with_vars(rt, &self.ctx_type_params()));
        self.return_type_stack.push(expected_ret.clone());
        // G4b: accumulate the body's ACTUAL return type — the tail expression joined with every
        // real `^` return (the `BlockReturn` arm joins in; `^^` diverges the block and adds
        // nothing). Starts at `Never`, the join identity (§11.3).
        self.block_ret_harvest.push(Type::Never);

        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            block_bytecode.current_source = stmt.source_info.clone();
            // The final statement is the block's implicit return value; check it against the
            // declared return type. Explicit `^`/`^^` returns are handled by their own arms.
            let is_tail_expr = idx == len - 1
                && !matches!(
                    &stmt.value,
                    NodeValue::BlockReturn(_) | NodeValue::MethodReturn(_)
                );
            if let (true, Some(expected)) = (is_tail_expr, &expected_ret) {
                self.compile_expecting(stmt, expected, &mut block_bytecode)?;
            } else {
                self.compile_node(stmt, &mut block_bytecode)?;
            }
            if is_tail_expr {
                let t = self.static_type(stmt);
                if let Some(h) = self.block_ret_harvest.last_mut() {
                    let joined = h.join(&t);
                    *h = joined;
                }
            }
            if idx < len - 1 {
                block_bytecode.push(Instruction::Pop);
            }
        }
        self.return_type_stack.pop();
        let harvested = self.block_ret_harvest.pop().unwrap_or(Type::Never);
        // An empty body yields nil.
        let harvested = if len == 0 { Type::Nil } else { harvested };

        block_bytecode.current_source = block.source_info.clone();
        if len == 0 {
            block_bytecode.push(Instruction::Push(Constant::Nil));
        }

        block_bytecode.push(Instruction::Return);

        let decl_block = if let Some(db) = &block.decl_block {
            let mut db_bytecode = CodeBlock::new();
            db_bytecode.current_source = db.source_info.clone();
            self.compile_block(db, &mut db_bytecode)?;
            if let Some(Instruction::Push(Constant::Block(sb))) = db_bytecode.pop() {
                Some(sb)
            } else {
                None
            }
        } else {
            None
        };

        // Phase 3c join/merge: snapshot the guarded key's narrowed type at the arm's exit before
        // its scope is discarded. Absent from the overlay ⇒ the arm widened it to `Any`.
        if let Some(key) = &capture_key {
            let exit = self
                .scopes
                .last()
                .unwrap()
                .narrowed
                .get(key)
                .cloned()
                .unwrap_or(Type::Any);
            self.captured_arm_exit = Some(exit);
        }

        self.pop_scope();

        // G4b: record the literal's sharpened outward type (§11.3) — its header with the names
        // stripped, inference filling what the header leaves blank: params from annotations or
        // expectation seeds, the return from the declared `^Ret` or the harvested join. Recorded
        // only when it says something (all-`Any` stays bare `Block`, minting no claims). This is
        // what `static_type` answers for the literal from here on — and what call-site
        // unification binds `U` from (`collect:`'s `Block(T ^U)`).
        let ret_belief = expected_ret.unwrap_or(harvested);
        if param_beliefs.iter().any(|t| *t != Type::Any) || ret_belief != Type::Any {
            self.block_literal_types.insert(
                block as *const BlockNode as usize,
                Type::BlockOf {
                    params: param_beliefs,
                    ret: Box::new(ret_belief),
                },
            );
        }

        let block_name = block.name.as_ref().map(|s| s.value.clone());

        let (fused_bytecode, fused_source_map) =
            fuse_bytecode(block_bytecode.bytecode, block_bytecode.source_map);
        let static_block = StaticBlock {
            spec_state: Default::default(),
            name: block_name,
            is_nested_block: true,
            is_init_literal: is_init,
            param_syms: crate::value::intern_param_syms(&param_names),
            param_types,
            param_elem_tags,
            bytecode: SharedBytecode(Rc::new(fused_bytecode)),
            source_info: block.source_info.clone(),
            decl_block,
            source_map: SharedSourceMap(Rc::new(fused_source_map)),
            // Every closure of this literal shares one inline-cache array via this id.
            template_id: self
                .mint_template_ids
                .then(crate::instruction::fresh_template_id),
        };

        bytecode.push(Instruction::Push(Constant::Block(Rc::new(static_block))));
        self.inline_carets = saved_inline;
        Ok(())
    }

    fn reconstruct_selector(&self, sig: &MethodSelectorNode) -> Result<String, String> {
        if sig.identifiers.is_empty() {
            return Err("No identifiers found in method selector".to_string());
        }
        // The wildcard-selector rule: a definition may not write the same keyword twice in a row.
        // Consecutive repetition is the call-site idiom for a variadic component, so a literal
        // repeat (`foo:foo:`) is almost certainly a missing `+` — reject it so call-site folding
        // stays unambiguous. `+` is the only way to declare a repeated keyword.
        fn base(n: &str) -> &str {
            n.trim_end_matches(':').trim_end_matches('+')
        }
        for pair in sig.identifiers.windows(2) {
            if base(&pair[0].name) == base(&pair[1].name) {
                let kw = base(&pair[0].name);
                return Err(format!(
                    "selector repeats keyword '{kw}:'; declare it variadic with '{kw}+:' instead"
                ));
            }
        }
        let mut s = String::new();
        for ident in &sig.identifiers {
            s.push_str(&ident.name);
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests;
