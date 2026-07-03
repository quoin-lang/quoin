use crate::class_table::{ClassSig, ClassTable};
use crate::instruction::{
    Constant, Instruction, IntBinKind, SharedBytecode, SharedSourceMap, StaticBlock,
};
use crate::parser::ast::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, ClassDefinitionNode,
    DeclKind, DeclarationNode, IdentifierNode, IdentifierType, MethodCallNode, MethodSelectorNode,
    Node, NodeValue, ProgramNode, UnaryOperatorNode, UnaryOperatorType,
};
use crate::symbol::Symbol;
use crate::types::{SeenTypes, Type};
use crate::value::{NamespacedName, SourceInfo};

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

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
        | Instruction::BranchIfNotBool(o) => Some(*o),
        _ => None,
    }
}

fn set_jump_offset(inst: &mut Instruction, off: isize) {
    match inst {
        Instruction::Jump(o)
        | Instruction::IfJump(o)
        | Instruction::ElseJump(o)
        | Instruction::BranchIfNotBool(o) => *o = off,
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

        // 3-instruction Int op (Slice a1): fuse `LoadLocal; <LoadLocal|Push>; IntXxx` into a
        // single `IntBinLL`/`IntBinLC` — same shape as the send triple above, but the terminal
        // is a devirtualized `Int` op. Collapses both operand-loads into the arithmetic op.
        if i + 2 < n
            && !is_target[i + 1]
            && !is_target[i + 2]
            && let Instruction::LoadLocal(a) = &bytecode[i]
            && let Some(kind) = int_bin_kind(&bytecode[i + 2])
        {
            let three = match &bytecode[i + 1] {
                Instruction::LoadLocal(b) => Some(Instruction::IntBinLL(*a, *b, kind)),
                Instruction::Push(c) => Some(Instruction::IntBinLC(*a, c.clone(), kind)),
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
    /// Method selector → declared return `Type` (methods that annotate a return).
    returns: HashMap<String, Type>,
    /// Every method selector defined directly in this class body.
    methods: HashSet<String>,
    /// The class is compile-sealed: `sealed!` appears as a direct (unconditional) body
    /// statement, so its method table is frozen and same-class self-sends can be
    /// devirtualized (Slice 2b-B).
    sealed: bool,
}

/// A non-fatal type diagnostic: the message plus the source span it points at, for `path:line:col`
/// rendering (Phase 4). `span` is `None` when a check can't attribute a precise location.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<SourceInfo>,
}

/// Unary methods safe to send to `nil` — they don't dereference the receiver, so a possibly-nil
/// receiver isn't flagged for these (Phase 3c nil-misuse check).
const NIL_SAFE_SELECTORS: &[&str] = &["defined?", "s", "pp", "class", "hash"];

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
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            class_ctx: Vec::new(),
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            return_type_stack: Vec::new(),
        }
    }

    pub fn new_with_locals(locals: HashSet<String>) -> Self {
        Self {
            scopes: vec![Scope {
                locals,
                immutable: HashSet::new(),
                types: HashMap::new(),
                declared_types: HashMap::new(),
                narrowed: HashMap::new(),
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            class_ctx: Vec::new(),
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            return_type_stack: Vec::new(),
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
    fn record_local_type(&mut self, name: &str, ty: Type) {
        if ty != Type::Any {
            self.scopes
                .last_mut()
                .unwrap()
                .types
                .insert(name.to_string(), ty);
        }
    }

    /// Record a local's *declared* (annotated) type — into both `types` (devirt) and
    /// `declared_types` (the reassignment check, which enforces only explicit contracts).
    fn record_declared_type(&mut self, name: &str, ty: Type) {
        if ty != Type::Any {
            let scope = self.scopes.last_mut().unwrap();
            scope.types.insert(name.to_string(), ty.clone());
            scope.declared_types.insert(name.to_string(), ty);
        }
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
                self.seen_types.insert(&cd.identifier.name);
                self.class_table
                    .insert(&cd.identifier.name, self.class_sig_from_def(cd));
            }
        }
    }

    /// Resolve a type-annotation name to a `Type`, flagging an unknown user class with a
    /// non-fatal `unknown type Foo` diagnostic (Phase 2). Resolution never fails: an unknown
    /// name still yields `Instance(name)` so lowering proceeds (gradual best-effort).
    /// Push a non-fatal type diagnostic, pointing at `span` when one is available (Phase 4).
    fn warn(&mut self, message: String, span: Option<&SourceInfo>) {
        self.diagnostics.push(Diagnostic {
            message,
            span: span.cloned(),
        });
    }

    fn resolve_annotation(&mut self, ident: &IdentifierNode) -> Type {
        let ty = Type::from_annotation_name(&ident.name);
        // `T?` is unknown iff its base `T` is unknown.
        let base = match &ty {
            Type::Nullable(inner) => inner.as_ref(),
            other => other,
        };
        if let Type::Instance(class) = base {
            if !self.seen_types.contains(class) {
                self.warn(
                    format!("unknown type `{}`", class),
                    ident.source_info.as_ref(),
                );
            }
        }
        ty
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
        self.warn(
            format!(
                "type mismatch: expected `{}`, found `{}`",
                expected.name(),
                actual.name()
            ),
            node.source_info.as_ref(),
        );
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

    /// After a guard send, if exactly one arm diverges the *other* arm's refinement holds for the
    /// rest of the current scope (the `x.defined?.else:{ ^^… }` early-return idiom).
    fn apply_post_guard_narrowing(&mut self, call: &MethodCallNode, g: &GuardInfo) {
        let idents = &call.arguments.signature.identifiers;
        let diverges = |kw: &str| -> bool {
            idents
                .iter()
                .position(|i| i.name == kw)
                .map(|k| Self::expr_diverges(&call.arguments.expressions[k]))
                .unwrap_or(false)
        };
        let survivor = match (diverges("if"), diverges("else")) {
            (true, false) => Some(g.else_arm.clone()),
            (false, true) => Some(g.if_arm.clone()),
            _ => None,
        };
        if let Some(ty) = survivor {
            self.scopes
                .last_mut()
                .unwrap()
                .narrowed
                .insert(g.key.clone(), ty);
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
            NodeValue::Block(_) => Type::Block,
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
            NodeValue::MethodCall(call) => match self.self_send_return_type(call) {
                Type::Any => self.object_rooted_return_type(call),
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
        let idents = &call.arguments.signature.identifiers;
        if idents.is_empty() {
            return Type::Any;
        }
        // Canonical selector: unary uses the bare name; a keyword send joins `name:` parts.
        // A variadic run folds to `name+:` in dispatch, which we don't reconstruct here — so
        // such a send simply stays Unknown rather than risking a mismatched selector.
        let selector = if call.arguments.expressions.is_empty() {
            idents[0].name.clone()
        } else {
            idents
                .iter()
                .map(|i| format!("{}:", i.name))
                .collect::<String>()
        };
        ctx.returns.get(&selector).cloned().unwrap_or(Type::Any)
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

    /// Selector → declared-return-`Type` map for a class body, from its method
    /// definitions/extensions that carry a return type.
    fn collect_class_ctx(&mut self, block: &BlockNode) -> ClassCtx {
        let mut returns = HashMap::new();
        let mut methods = HashSet::new();
        let mut sealed = false;
        for stmt in &block.statements {
            match &stmt.value {
                NodeValue::MethodDefinition(m) => {
                    if let Ok(selector) = self.reconstruct_selector(&m.signature) {
                        methods.insert(selector.clone());
                        if let Some(rt) = &m.block.return_type {
                            returns.insert(selector, self.resolve_annotation(rt));
                        }
                    }
                }
                NodeValue::MethodExtension(m) => {
                    if let Ok(selector) = self.reconstruct_selector(&m.signature) {
                        methods.insert(selector.clone());
                        if let Some(rt) = &m.block.return_type {
                            returns.insert(selector, self.resolve_annotation(rt));
                        }
                    }
                }
                // A direct (unconditional) `sealed!` statement seals the class at compile
                // time, freezing its method table so same-class self-sends devirtualize.
                NodeValue::MethodCall(call) if Self::is_sealed_marker(call) => sealed = true,
                _ => {}
            }
        }
        ClassCtx {
            returns,
            methods,
            sealed,
        }
    }

    /// A bare `sealed!` self-send (`sealed!` or `self.sealed!`, no args).
    fn is_sealed_marker(call: &MethodCallNode) -> bool {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        is_self
            && call.arguments.expressions.is_empty()
            && call.arguments.signature.identifiers.len() == 1
            && call.arguments.signature.identifiers[0].name == "sealed!"
    }

    /// The class name in a `.mix:X` self-send (a mixin application), if this call is one.
    fn mixin_target(call: &MethodCallNode) -> Option<&str> {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        if !is_self
            || call.arguments.signature.identifiers.len() != 1
            || call.arguments.signature.identifiers[0].name != "mix"
            || call.arguments.expressions.len() != 1
        {
            return None;
        }
        match &call.arguments.expressions[0].value {
            NodeValue::Identifier(id) => Some(id.name.as_str()),
            _ => None,
        }
    }

    /// Build a `ClassSig` from a class definition's AST — the current-unit source for the class
    /// table (Phase 3b). Selectors come from the same `reconstruct_selector` as `collect_class_ctx`,
    /// so the method set can't drift from it. `has_catch_all` is left `false` here (only MNU uses
    /// it, and MNU consults VM-sourced sigs); the parent comes from the def, mixins from `.mix:`.
    fn class_sig_from_def(&self, class_def: &ClassDefinitionNode) -> ClassSig {
        let mut own_selectors = HashSet::new();
        let mut mixins = Vec::new();
        let mut sealed = false;
        for stmt in &class_def.block.statements {
            match &stmt.value {
                NodeValue::MethodDefinition(m) => {
                    if let Ok(sel) = self.reconstruct_selector(&m.signature) {
                        own_selectors.insert(Arc::from(sel.as_str()));
                    }
                }
                NodeValue::MethodExtension(m) => {
                    if let Ok(sel) = self.reconstruct_selector(&m.signature) {
                        own_selectors.insert(Arc::from(sel.as_str()));
                    }
                }
                NodeValue::MethodCall(call) if Self::is_sealed_marker(call) => sealed = true,
                NodeValue::MethodCall(call) => {
                    if let Some(mixin) = Self::mixin_target(call) {
                        mixins.push(Arc::from(mixin));
                    }
                }
                _ => {}
            }
        }
        ClassSig {
            parent: class_def
                .parent_identifier
                .as_ref()
                .map(|p| Arc::from(p.name.as_str())),
            mixins,
            own_selectors,
            sealed,
            has_catch_all: false,
            from_vm: false,
            method_params: HashMap::new(),
            method_returns: self.declared_method_returns(&class_def.block),
        }
    }

    /// Declared return types (`selector → Type`) for the methods written directly in a class body —
    /// only those with a `^Ret` header. Pure (`&self`, no diagnostics): the return-type check
    /// already warns on unknown annotations, so recording resolves names without re-warning. Feeds
    /// `ClassSig::method_returns` for both `Foo <- {}` defs and `Foo <-- {}` reopens (Phase 3c·4).
    fn declared_method_returns(&self, block: &BlockNode) -> HashMap<Arc<str>, Type> {
        let mut out = HashMap::new();
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            if let (Ok(sel), Some(rt)) = (self.reconstruct_selector(sig), &blk.return_type) {
                out.insert(
                    Arc::from(sel.as_str()),
                    Type::from_annotation_name(&rt.name),
                );
            }
        }
        out
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
            let over = Type::from_annotation_name(&rt.name);
            if self.override_return_violates(&over, &base) {
                self.warn(
                    format!(
                        "override of `{}` returns `{}`, incompatible with `{}` from `{}`",
                        selector,
                        over.name(),
                        base.name(),
                        from,
                    ),
                    rt.source_info.as_ref(),
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
            name: None,
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode(Rc::new(bytecode)),
            source_info: program.source_info.clone(),
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(source_map)),
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
                    bytecode.push(Instruction::LoadField(id.name.clone()));
                } else if id.name == "nil" || id.name == "true" || id.name == "false" {
                    match id.name.as_str() {
                        "nil" => bytecode.push(Instruction::Push(Constant::Nil)),
                        "true" => bytecode.push(Instruction::Push(Constant::Bool(true))),
                        "false" => bytecode.push(Instruction::Push(Constant::Bool(false))),
                        _ => unreachable!(),
                    }
                } else if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced
                {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                } else if self.is_local(&id.name) {
                    bytecode.push(Instruction::LoadLocal(Symbol::intern(&(id.name.clone()))));
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
                // Record the class as known as soon as it's defined — covers classes in nested
                // blocks the top-level pre-scan can't reach (a def-before-use in any scope).
                self.seen_types.insert(&name.name);
                self.class_table
                    .insert(&name.name, self.class_sig_from_def(class_def));
                self.check_return_covariance(&name.name, &class_def.block);
                let parent_name = class_def
                    .parent_identifier
                    .as_ref()
                    .map(|id| NamespacedName::from_ast(id));
                let mut instance_vars = Vec::new();
                for arg in &class_def.block.arguments {
                    instance_vars.push(arg.identifier.name.clone());
                }
                let is_value_type =
                    matches!(name.name.as_str(), "Integer" | "Double" | "Boolean" | "Nil");
                if is_value_type && !instance_vars.is_empty() {
                    return Err(format!(
                        "value type '{}' cannot declare instance variables (@{})",
                        name.name, instance_vars[0]
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
                let ctx = self.collect_class_ctx(&class_def.block);
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
                // A `Foo <-- {}` reopen contributes its methods' declared returns to `Foo`'s
                // signature — how the core classes (`Object <-- {}`, `nil <-- {}`, …) carry their
                // return contracts, since they're reopened rather than defined with `<-` (Phase 3c·4).
                if let NodeValue::Identifier(target) = &class_ext.expression.value {
                    self.class_table
                        .add_returns(&target.name, self.declared_method_returns(&class_ext.block));
                    self.check_return_covariance(&target.name, &class_ext.block);
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
                let ctx = self.collect_class_ctx(&class_ext.block);
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

    fn collect_lvalue_names(&self, lvalues: &[Arc<Node>], names: &mut Vec<String>) {
        for lval in lvalues {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let id = &ident_lval.identifier;
                    if id.namespace.is_none()
                        && id.identifier_type != IdentifierType::Namespaced
                        && id.identifier_type != IdentifierType::Instance
                    {
                        names.push(id.name.clone());
                    }
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let id = &splat_lval.identifier;
                    if id.namespace.is_none()
                        && id.identifier_type != IdentifierType::Namespaced
                        && id.identifier_type != IdentifierType::Instance
                    {
                        names.push(id.name.clone());
                    }
                }
                NodeValue::SubLValue(sub_lval) => {
                    self.collect_lvalue_names(&sub_lval.lvalues, names);
                }
                _ => {}
            }
        }
    }

    fn compile_assignment(
        &mut self,
        assign: &AssignmentNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        if assign.lvalues.is_empty() {
            return Err("Assignment requires at least one target lvalue".to_string());
        }

        // Strict mode: assignment never declares. Plain-local targets must already be in
        // scope (compile_ident_store errors otherwise); a new local is introduced with
        // `var`/`let` (compile_declaration). Globals (`Foo`) and fields (`@x`) are handled
        // per-target in compile_ident_store and are unaffected by this rule.

        // Phase 3a: a reassignment to a *typed* local is checked (and numeric literals promoted)
        // against its declared type — the var's contract. An untyped/unrecorded target resolves to
        // `Any`, so `compile_expecting` compiles it unchecked. Destructuring targets are untyped.
        if let [lval] = assign.lvalues.as_slice()
            && let NodeValue::IdentLValue(l) = &lval.value
            && let Some(expected) = self.declared_type(&l.identifier.name)
        {
            self.compile_expecting(&assign.rvalue, &expected, bytecode)?;
            // Phase 3c: the local now holds the rvalue's type — flow-update its narrowing (a
            // concrete type re-narrows; `Any` widens to gradual). Declared targets only, so the
            // optimizer's inferred type for an untyped `var` is never shadowed.
            let rt = self.static_type(&assign.rvalue);
            self.update_narrowing(NarrowKey::Local(l.identifier.name.clone()), rt);
        } else {
            self.compile_node(&assign.rvalue, bytecode)?;
        }

        if assign.lvalues.len() == 1 {
            let lval = &assign.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_lvalue_store(lval, bytecode, false)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(Symbol::intern(
                &(temp_var.clone()),
            )));
            self.compile_destruct(&assign.lvalues, &temp_var, bytecode, false)?;
        }

        Ok(())
    }

    fn compile_declaration(
        &mut self,
        decl: &DeclarationNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        if decl.lvalues.is_empty() {
            return Err("declaration requires at least one target".to_string());
        }
        let mutable = matches!(decl.kind, DeclKind::Var);

        // `var`/`let` declares plain locals only.
        self.validate_decl_targets(&decl.lvalues)?;

        // Introduce the fresh bindings BEFORE compiling the initializer, so a recursive
        // reference resolves — `var f = { … f … }` (a self-recursive block) must see its
        // own name. The name binds in the enclosing env the closure captures; the actual
        // store runs after the value is built, so the captured frame is populated by the
        // time the closure is invoked. (Same-scope redeclaration is an error.)
        let mut names = Vec::new();
        self.collect_lvalue_names(&decl.lvalues, &mut names);
        for name in &names {
            self.declare_local(name, mutable)?;
        }

        // Phase 3a: an annotated `var x: T = expr` resolves `T` (flagging an unknown type) and
        // checks/promotes the initializer against it; un-annotated decls compile plainly.
        let annotated = decl.type_hint.as_ref().map(|h| self.resolve_annotation(h));
        match &annotated {
            Some(expected) => self.compile_expecting(&decl.rvalue, expected, bytecode)?,
            None => self.compile_node(&decl.rvalue, bytecode)?,
        }

        // Record the local's type for the checker + devirt. The annotation is authoritative (and
        // matches a promoted initializer); otherwise infer `Int`/`List` from the initializer —
        // both devirt paths have a runtime fallback, so a stale inferred type is harmless. `Bool`
        // is excluded: the `if:else:` inline for a statically-`Bool` `var` has no fallback, so a
        // reassigned `var` could go stale.
        if decl.lvalues.len() == 1
            && let NodeValue::IdentLValue(l) = &decl.lvalues[0].value
        {
            match &annotated {
                // An explicit annotation is the local's declared type (the reassignment contract).
                // `Bool` is excluded — its `if:else:` inline has no fallback for a stale `var`.
                Some(t) if *t != Type::Bool && *t != Type::Any => {
                    self.record_declared_type(&l.identifier.name, t.clone());
                }
                Some(_) => {}
                None => {
                    // No annotation: infer Int/List from the initializer for devirt only (a hint,
                    // not a contract — an untyped `var` may be reassigned to any type).
                    let ty = self.static_type(&decl.rvalue);
                    if ty == Type::Int || ty == Type::List {
                        self.record_local_type(&l.identifier.name, ty);
                    }
                }
            }
        }

        if decl.lvalues.len() == 1 {
            let lval = &decl.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_lvalue_store(lval, bytecode, true)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(Symbol::intern(
                &(temp_var.clone()),
            )));
            self.compile_destruct(&decl.lvalues, &temp_var, bytecode, true)?;
        }

        Ok(())
    }

    /// A `var`/`let` target must be a plain local (or `_` / splat / nested thereof) — not a
    /// global (`Foo`), an instance variable (`@x`), or a namespaced name.
    fn validate_decl_targets(&self, lvalues: &[Arc<Node>]) -> Result<(), String> {
        for lval in lvalues {
            match &lval.value {
                NodeValue::IdentLValue(l) => self.validate_decl_ident(&l.identifier)?,
                NodeValue::SplatLValue(l) => self.validate_decl_ident(&l.identifier)?,
                NodeValue::IgnoredLValue | NodeValue::IgnoredSplatLValue => {}
                NodeValue::SubLValue(s) => self.validate_decl_targets(&s.lvalues)?,
                other => return Err(format!("unsupported `var`/`let` target: {:?}", other)),
            }
        }
        Ok(())
    }

    fn validate_decl_ident(&self, id: &IdentifierNode) -> Result<(), String> {
        if id.identifier_type == IdentifierType::Instance {
            return Err(format!(
                "`var`/`let` cannot declare an instance variable (`@{}`); \
                 declare instance variables in the class header",
                id.name
            ));
        }
        if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
            return Err(format!(
                "`var`/`let` cannot declare a namespaced name (`{}`)",
                id.name
            ));
        }
        if id
            .name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        {
            return Err(format!(
                "`var`/`let` declares locals; `{}` is uppercase — globals/classes use `{} = …`",
                id.name, id.name
            ));
        }
        Ok(())
    }

    fn compile_lvalue_store(
        &mut self,
        lval: &Node,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        match &lval.value {
            NodeValue::IdentLValue(ident_lval) => {
                let id = &ident_lval.identifier;
                if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::StoreGlobal(ns_name, false));
                } else {
                    let name = &id.name;
                    self.compile_ident_store(&id.identifier_type, name, bytecode, declaring)?;
                }
            }
            NodeValue::IgnoredLValue => {
                bytecode.push(Instruction::Pop);
            }
            NodeValue::IgnoredSplatLValue => {
                bytecode.push(Instruction::Pop);
            }
            _ => return Err(format!("Unsupported store target: {:?}", lval.value)),
        }
        Ok(())
    }

    fn compile_ident_store(
        &mut self,
        ident_type: &IdentifierType,
        name: &String,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        // A `var`/`let` declaration introduces a fresh binding. The target was
        // validated as a plain local and inserted into the current scope by
        // `compile_declaration`, so here we just emit the binding instruction.
        if declaring {
            bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
            return Ok(());
        }
        // Reserved identifiers parse as assignable lvalues (`true = false`); emit a store
        // so the runtime raises "Can't modify reserved identifier" (unchanged behavior),
        // rather than the compile-time "undeclared local" error below.
        if matches!(name.as_str(), "true" | "false" | "nil") {
            bytecode.push(Instruction::StoreLocal(Symbol::intern(&(name.clone()))));
            return Ok(());
        }
        let first_char = name.chars().next().unwrap_or('\0');
        if first_char.is_ascii_uppercase() {
            let ns_name = NamespacedName::new(Vec::new(), name.clone());
            bytecode.push(Instruction::StoreGlobal(ns_name, false));
        } else if ident_type == &IdentifierType::Instance {
            if self.value_type_def_depth > 0 {
                return Err(format!(
                    "value types cannot have instance variables (found '@{}')",
                    name
                ));
            }
            bytecode.push(Instruction::StoreField(name.clone()));
        } else if self.is_local(name) {
            if self.is_immutable(name) {
                return Err(format!("cannot reassign `let` binding `{}`", name));
            }
            bytecode.push(Instruction::StoreLocal(Symbol::intern(&(name.clone()))));
        } else if self.scopes.last().map(|s| s.is_init).unwrap_or(false) {
            // Inside an object-initializer block (`X.new:{ … }`), a bare `field = value`
            // binds an instance field — no `var` needed. The instantiating frame binds it
            // into the new object at runtime.
            bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
        } else {
            return Err(format!(
                "undeclared local `{}` — declare it with `var {} = …` \
                 (assignment no longer implicitly declares locals)",
                name, name
            ));
        }
        Ok(())
    }

    fn compile_destruct(
        &mut self,
        lvalues: &[Arc<Node>],
        temp_var: &str,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        for (i, lval) in lvalues.iter().enumerate() {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let name = &ident_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("at:"), 1));

                    self.compile_ident_store(
                        &ident_lval.identifier.identifier_type,
                        name,
                        bytecode,
                        declaring,
                    )?;
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let name = &splat_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("sliceFrom:"), 1));

                    self.compile_ident_store(
                        &splat_lval.identifier.identifier_type,
                        name,
                        bytecode,
                        declaring,
                    )?;
                }
                NodeValue::IgnoredLValue => {}
                NodeValue::IgnoredSplatLValue => {}
                NodeValue::SubLValue(sub_lval) => {
                    let nested_temp = self.new_temp_var();
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .locals
                        .insert(nested_temp.clone());

                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("at:"), 1));
                    bytecode.push(Instruction::DefineLocal(Symbol::intern(
                        &(nested_temp.clone()),
                    )));

                    self.compile_destruct(&sub_lval.lvalues, &nested_temp, bytecode, declaring)?;
                }
                _ => {
                    return Err(format!(
                        "Unsupported destructuring element: {:?}",
                        lval.value
                    ));
                }
            }
        }
        Ok(())
    }

    /// Emit a call instruction: `CallSelfDirect` for a self-send to a same-class method of
    /// a compile-sealed class (devirtualizable — Slice 2b-B), else a normal `Send`.
    fn emit_call(&self, bytecode: &mut CodeBlock, selector: &str, num_args: usize, is_self: bool) {
        if is_self {
            if let Some(ctx) = self.class_ctx.last() {
                if ctx.sealed && ctx.methods.contains(selector) {
                    bytecode.push(Instruction::CallSelfDirect(
                        Symbol::intern(selector),
                        num_args,
                    ));
                    return;
                }
            }
        }
        bytecode.push(Instruction::Send(Symbol::intern(selector), num_args));
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

        // Evaluate receiver
        if let Some(ref subject) = call.subject {
            self.compile_node(subject, bytecode)?;
        } else {
            bytecode.push(Instruction::LoadLocal(Symbol::intern("self")));
        }

        // No-argument selector (unary / bang / symbol): a single component, no args.
        if args.expressions.is_empty() {
            if args.signature.identifiers.is_empty() {
                return Err("No identifiers found in method call selector".to_string());
            }
            let selector = args.signature.identifiers[0].name.clone();
            self.emit_call(bytecode, &selector, 0, is_self);
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
        // Phase 3c: if this is a nil-guard conditional (`RECV.defined?.if:`/`.else:`), the per-arm
        // narrowing to install while compiling each arm, and post-guard on divergence.
        let guard = self.guard_narrowing(call);
        let idents = &args.signature.identifiers;
        debug_assert_eq!(idents.len(), args.expressions.len());
        let mut selector = String::new();
        let mut num_components = 0usize;
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
                // Phase 3c: narrow the guarded path inside this arm's block (`if` → non-nil arm,
                // `else` → nil arm). One-shot, consumed by the arm's `compile_block`.
                if let Some(g) = &guard
                    && matches!(arg.value, NodeValue::Block(_))
                    && let Some(arm_ty) = g.arm_type(&idents[i].name)
                {
                    self.next_block_narrowing = Some((g.key.clone(), arm_ty));
                }
                match &param_types {
                    Some(params) => self.compile_expecting(arg, &params[i + j], bytecode)?,
                    None => self.compile_node(arg, bytecode)?,
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

        // Phase 3c: after a guard send, a diverging arm narrows the receiver for the rest of the
        // enclosing scope (the `x.defined?.else:{ ^^… }` early-return idiom).
        if let Some(g) = &guard {
            self.apply_post_guard_narrowing(call, g);
        }

        // Slice 2e: devirtualize `at:`/`at:put:`/`add:` when the receiver is statically a
        // `List`. The operands a send would consume are already on the stack in send order,
        // so the op is a drop-in replacement.
        if let Some(op) = self.list_devirt_op(call, &selector, num_components) {
            bytecode.push(op);
            return Ok(());
        }

        self.emit_call(bytecode, &selector, num_components, is_self);
        Ok(())
    }

    /// The devirtualized `List` op for a keyword send whose receiver is statically a `List`
    /// (Slice 2e), or `None` to fall through to a normal send.
    fn list_devirt_op(
        &self,
        call: &MethodCallNode,
        selector: &str,
        num_args: usize,
    ) -> Option<Instruction> {
        let subject = call.subject.as_ref()?;
        if self.static_type(subject) != Type::List {
            return None;
        }
        match (selector, num_args) {
            ("at:", 1) => Some(Instruction::ListGet),
            ("at:put:", 2) => Some(Instruction::ListSet),
            ("add:", 1) => Some(Instruction::ListPush),
            _ => None,
        }
    }

    /// Slice 2d — control-flow inlining. If `call` is `recv.if:{…}` or
    /// `recv.if:{…}else:{…}` where `recv` is statically `Boolean` and every block arg is
    /// a literal, parameter-less, declaration-free block, splice the block bodies inline
    /// as `ElseJump`/`Jump` bytecode (no block alloc, no dispatch, no block frame) and
    /// return `true`. Otherwise emit nothing and return `false` so the caller compiles the
    /// normal send.
    ///
    /// Soundness: `Boolean` is sealed (prelude), so `if:`/`if:else:` on a statically-Bool
    /// receiver always resolve to the built-in `True`/`False` methods — treating them as
    /// inlinable built-ins is a language guarantee, matching Smalltalk `ifTrue:ifFalse:`.
    fn try_compile_inlined_conditional(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let subject = match &call.subject {
            Some(s) => s,
            None => return Ok(false),
        };
        let idents = &call.arguments.signature.identifiers;
        let exprs = &call.arguments.expressions;

        // Selector shape: `if:` (then only) or `if:else:` (then + else).
        let kws: Vec<&str> = idents.iter().map(|i| i.name.as_str()).collect();
        let has_else = match kws.as_slice() {
            ["if"] => false,
            ["if", "else"] => true,
            _ => return Ok(false),
        };

        // Phase 3c: a nil-guard on a *declared-nullable* path is not inlined — it takes the general
        // send path so its arms narrow (via each arm's `compile_block` scope). Rare and opt-in;
        // untyped guards (the common case) aren't `Nullable`, so they still inline — no perf change.
        if self.guard_narrowing(call).is_some() {
            return Ok(false);
        }

        // Bool receiver → inline directly. A known-non-Bool receiver (Int/List) → normal send
        // (the guard would always miss). Everything else — `Any`, and any other static type we
        // don't specifically reason about — → guarded inline (option C): a runtime Bool-check
        // falls back to the real send for a non-Bool receiver.
        let guarded = match self.static_type(subject) {
            Type::Bool => false,
            Type::Int | Type::List => return Ok(false),
            _ => true,
        };

        // Every arg must be a literal, 0-arg, declaration-free block (v1).
        let then_blk = match Self::inlinable_block(&exprs[0]) {
            Some(b) => b,
            None => return Ok(false),
        };
        let else_blk = if has_else {
            match Self::inlinable_block(&exprs[1]) {
                Some(b) => Some(b),
                None => return Ok(false),
            }
        } else {
            None
        };

        // Condition → stack.
        self.compile_node(subject, bytecode)?;

        if !guarded {
            self.emit_inline_conditional_body(then_blk, else_blk, bytecode)?;
            return Ok(true);
        }

        // Guarded (option C): if the receiver isn't a Bool at runtime, jump past the inlined
        // body to a cold path that reissues the real send. The inlined body is
        // self-contained (leaves its value on the stack), so it is wrapped verbatim.
        let mut hot_bc = CodeBlock::new();
        hot_bc.current_source = bytecode.current_source.clone();
        self.emit_inline_conditional_body(then_blk, else_blk, &mut hot_bc)?;

        let mut cold_bc = CodeBlock::new();
        cold_bc.current_source = bytecode.current_source.clone();
        self.compile_block(then_blk, &mut cold_bc)?;
        if let Some(else_blk) = else_blk {
            self.compile_block(else_blk, &mut cold_bc)?;
            self.emit_call(&mut cold_bc, "if:else:", 2, false);
        } else {
            self.emit_call(&mut cold_bc, "if:", 1, false);
        }

        let h = hot_bc.len() as isize;
        let k = cold_bc.len() as isize;
        bytecode.push(Instruction::BranchIfNotBool(h + 2));
        bytecode.extend(hot_bc);
        bytecode.push(Instruction::Jump(k + 1));
        bytecode.extend(cold_bc);
        Ok(true)
    }

    /// Emit the unguarded inlined form of `if:`/`if:else:` (receiver already on the stack)
    /// into `out`: `ElseJump; <then>; Jump; <else | Push(Nil)>`, leaving the construct's
    /// value on the stack. Shared by the Bool-receiver path and the guarded (option C) hot
    /// path.
    fn emit_inline_conditional_body(
        &mut self,
        then_blk: &BlockNode,
        else_blk: Option<&BlockNode>,
        out: &mut CodeBlock,
    ) -> Result<(), String> {
        let mut then_bc = CodeBlock::new();
        then_bc.current_source = out.current_source.clone();
        self.inline_block_body(then_blk, &mut then_bc)?;
        let t = then_bc.len() as isize;

        if let Some(else_blk) = else_blk {
            let mut else_bc = CodeBlock::new();
            else_bc.current_source = out.current_source.clone();
            self.inline_block_body(else_blk, &mut else_bc)?;
            let e = else_bc.len() as isize;
            // cond false → skip the then-body and its trailing Jump, land on the else-body.
            out.push(Instruction::ElseJump(t + 2));
            out.extend(then_bc);
            out.push(Instruction::Jump(e + 1));
            out.extend(else_bc);
        } else {
            // No else: a false condition makes the construct's value `nil`.
            out.push(Instruction::ElseJump(t + 2));
            out.extend(then_bc);
            out.push(Instruction::Jump(2));
            out.push(Instruction::Push(Constant::Nil));
        }
        Ok(())
    }

    /// A literal block usable for control-flow inlining: no parameters and no local
    /// declarations. (v1 — declaration-carrying blocks need alpha-renaming, a follow-up.)
    ///
    /// A body `var`/`let` is a `Declaration` *statement*, not a `decls` header entry, so
    /// both must be checked: inlining a block that binds a top-level local would splice
    /// that binding into the method scope, colliding with a sibling branch's same-named
    /// local (they are isolated only by their now-absent block frames).
    fn inlinable_block(node: &Node) -> Option<&BlockNode> {
        if let NodeValue::Block(b) = &node.value {
            let declares_local = b
                .statements
                .iter()
                .any(|s| matches!(&s.value, NodeValue::Declaration(_)));
            if b.arguments.is_empty() && b.decls.is_empty() && !declares_local {
                return Some(b);
            }
        }
        None
    }

    /// Compile an inlined control-flow block body into `out`: its statements spliced
    /// inline (value-on-stack like a block, but no frame and no trailing `Return`), with
    /// each top-level `^expr` redirected to a `Jump` past the body (patched here). `^^`
    /// (MethodReturn) is left untouched — it still returns from the enclosing method.
    fn inline_block_body(&mut self, block: &BlockNode, out: &mut CodeBlock) -> Result<(), String> {
        let saved = self.inline_carets.replace(Vec::new());
        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            out.current_source = stmt.source_info.clone();
            self.compile_node(stmt, out)?;
            // Discard the value of every statement but the last (the block's value).
            if idx + 1 < len {
                out.push(Instruction::Pop);
            }
        }
        if len == 0 {
            out.push(Instruction::Push(Constant::Nil));
        }
        // Patch each top-level `^` to jump to just past the body (falls through to the
        // construct's merge point).
        let carets = self.inline_carets.take().unwrap_or_default();
        let end = out.len() as isize;
        for pos in carets {
            set_jump_offset(&mut out.bytecode[pos], end - pos as isize);
        }
        self.inline_carets = saved;
        Ok(())
    }

    /// Slice 2d (v2) — inline `{cond}.whileDo:{body}` when both the receiver (`cond`) and
    /// the body are literal, 0-arg, declaration-free blocks, into a native jump loop.
    /// Eliminates the per-iteration block allocation, dispatch, and frame — and the
    /// recursion, since the bootstrap `whileDo:` recurses once per iteration
    /// (`^^s.whileDo:block`). Returns `true` if inlined. Evaluates to `nil`, matching the
    /// bootstrap (the terminating `if:` has no else). `^` in `cond`/`body` ends that block
    /// (redirected by `inline_block_body`); `^^` still returns from the enclosing method.
    fn try_compile_inlined_while(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let subject = match &call.subject {
            Some(s) => s,
            None => return Ok(false),
        };
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if kws.as_slice() != ["whileDo"] {
            return Ok(false);
        }
        let cond_blk = match Self::inlinable_block(subject) {
            Some(b) => b,
            None => return Ok(false),
        };
        let body_blk = match Self::inlinable_block(&call.arguments.expressions[0]) {
            Some(b) => b,
            None => return Ok(false),
        };

        // Compile cond/body into their own sub-blocks so their lengths size the jumps.
        let mut cond_bc = CodeBlock::new();
        cond_bc.current_source = bytecode.current_source.clone();
        self.inline_block_body(cond_blk, &mut cond_bc)?;
        let c = cond_bc.len() as isize;

        let mut body_bc = CodeBlock::new();
        body_bc.current_source = bytecode.current_source.clone();
        self.inline_block_body(body_blk, &mut body_bc)?;
        let b = body_bc.len() as isize;

        // Layout (each jump offset is relative to its own position):
        //   [start] <cond>          (c instrs; leaves the condition on the stack)
        //           ElseJump(b+3)    cond false → exit to the trailing nil
        //           <body>          (b instrs; leaves the body value)
        //           Pop              discard the body value
        //           Jump(-(c+b+2))   back to [start]
        //   [end]   Push(Nil)        whileDo: evaluates to nil
        bytecode.extend(cond_bc);
        bytecode.push(Instruction::ElseJump(b + 3));
        bytecode.extend(body_bc);
        bytecode.push(Instruction::Pop);
        bytecode.push(Instruction::Jump(-(c + b + 2)));
        bytecode.push(Instruction::Push(Constant::Nil));
        Ok(true)
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
        let devirt =
            self.static_type(&op.left) == Type::Int && self.static_type(&op.right) == Type::Int;

        self.compile_node(&op.left, bytecode)?;
        self.compile_node(&op.right, bytecode)?;

        if devirt {
            if let Some(op_instr) = Self::int_devirt_op(&op.operator) {
                bytecode.push(op_instr);
                return Ok(());
            }
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
        // A real block gets its own frame, so any enclosing inlined-region caret
        // redirection (Slice 2d) must not leak into it: a `^` here is a genuine
        // `BlockReturn` for this block. Cleared on entry, restored on exit.
        let saved_inline = self.inline_carets.take();
        let mut param_names = Vec::new();
        let mut param_types = Vec::new();
        let mut locals = HashSet::new();

        for arg in &block.arguments {
            let name = arg.identifier.name.clone();
            param_names.push(name.clone());
            // An unannotated parameter defaults to `Object` (the universal supertype),
            // so `|x|` and `|x:Object|` are the same signature everywhere downstream.
            let type_name = arg
                .type_hint
                .as_ref()
                .map(|id| id.name.clone())
                .unwrap_or_else(|| "Object".to_string());
            param_types.push(type_name);
            locals.insert(name);
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

        // Seed declared param types so arithmetic on a typed param devirtualizes. Dispatch only
        // selects a typed method when the arg matches, so the param is provably that type inside
        // the body — no runtime guard needed. Resolve the annotation (flagging unknown types).
        // An *un-annotated* param is `Any` (gradual, unchecked), NOT `Object` — the `Object`
        // default above is only the runtime dispatch signature, not a static type.
        for arg in &block.arguments {
            if let Some(hint) = &arg.type_hint {
                let ty = self.resolve_annotation(hint);
                self.record_local_type(&arg.identifier.name, ty);
            }
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
            .map(|rt| Type::from_annotation_name(&rt.name));
        self.return_type_stack.push(expected_ret.clone());

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
            if idx < len - 1 {
                block_bytecode.push(Instruction::Pop);
            }
        }
        self.return_type_stack.pop();

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
                Some(Box::new(sb))
            } else {
                None
            }
        } else {
            None
        };

        self.pop_scope();

        let block_name = block.name.as_ref().map(|s| s.value.clone());

        let (fused_bytecode, fused_source_map) =
            fuse_bytecode(block_bytecode.bytecode, block_bytecode.source_map);
        let static_block = StaticBlock {
            name: block_name,
            is_nested_block: true,
            param_syms: crate::value::intern_param_syms(&param_names),
            param_types,
            bytecode: SharedBytecode(Rc::new(fused_bytecode)),
            source_info: block.source_info.clone(),
            decl_block,
            source_map: SharedSourceMap(Rc::new(fused_source_map)),
        };

        bytecode.push(Instruction::Push(Constant::Block(static_block)));
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
mod tests {
    use super::*;
    use crate::parser::ast::*;
    use crate::parser::parse_quoin_string;
    use crate::value::NamespacedName;

    use std::sync::Arc;

    fn ns(name: &str) -> NamespacedName {
        NamespacedName::parse(name)
    }

    // Helpers to easily construct Nodes
    fn int(value: i64) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Integer(IntegerNode { value }),
        }
    }

    fn double(value: f64) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Double(DoubleNode { value }),
        }
    }

    fn string(value: &str) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Str(StringNode {
                value: value.to_string(),
            }),
        }
    }

    fn sym(value: &str) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Symbol(SymbolNode {
                value: value.to_string(),
            }),
        }
    }

    fn local_id(name: &str) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Identifier(IdentifierNode {
                source_info: None,
                namespace: None,
                name: name.to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    // Builds a `var` declaration. First-binding compilation is now `var` (a bare
    // assignment to an undeclared local is a strict-mode error — tested separately in
    // `strict_declaration_semantics`). A fresh `var` binding emits the same
    // Dup/DefineLocal bytecode the old implicit first-assignment did.
    fn assign_node(lvals: Vec<Node>, rval: Node) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Declaration(DeclarationNode {
                kind: DeclKind::Var,
                lvalues: lvals.into_iter().map(Arc::new).collect(),
                type_hint: None,
                rvalue: Arc::new(rval),
            }),
        }
    }

    #[test]
    fn resolver_flags_unknown_types() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics().iter().map(|d| d.message.clone()).collect()
        }

        // Builtins resolve silently — in a return type and in a param type.
        assert!(diags("Foo <- { greet -> { |^String| ^^ 'hi' } }").is_empty());
        assert!(diags("Foo <- { take -> { |n: Integer| ^^ n } }").is_empty());

        // An unknown class is flagged (non-fatal: compilation still succeeds).
        let d = diags("Foo <- { make -> { |^Widget| ^^ nil } }");
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("unknown type `Widget`"), "{d:?}");
        // …and in a param type.
        assert!(diags("Foo <- { take -> { |g: Gadget| ^^ g } }")[0].contains("Gadget"));

        // `T?` is flagged iff its base is unknown.
        assert!(diags("Foo <- { make -> { |^Widget?| ^^ nil } }")[0].contains("Widget"));
        assert!(diags("Foo <- { make -> { |^String?| ^^ nil } }").is_empty());

        // A class defined anywhere in the unit is known — forward reference via the pre-scan.
        // (`^Widget?` so the `nil` body is a valid return, not a nil-misuse.)
        assert!(diags("Foo <- { make -> { |^Widget?| ^^ nil } }; Widget <- { }").is_empty());
    }

    #[test]
    fn records_declared_method_returns_from_ast() {
        // Compile `src` and return the recorded returns for class `name` (Phase 3c·4a).
        fn returns_of(src: &str, name: &str) -> HashMap<Arc<str>, Type> {
            let node = parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.class_table.get(name).unwrap().method_returns
        }

        // A `^Ret` header on a `Foo <- {}` method is recorded; a header-less method is not.
        let r = returns_of("Foo <- { make -> { |^Integer| 5 }; plain -> { 1 } }", "Foo");
        assert_eq!(r.get("make"), Some(&Type::Int));
        assert_eq!(r.get("plain"), None);

        // A `Foo <-- {}` reopen contributes its declared returns too (how the core classes do it).
        let r = returns_of("Foo <- { }; Foo <-- { grow -> { |^String| 'x' } }", "Foo");
        assert_eq!(r.get("grow"), Some(&Type::String));
    }

    #[test]
    fn checker_flags_return_covariance() {
        fn diags(src: &str) -> Vec<String> {
            let node = parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics()
                .iter()
                .map(|d| d.message.clone())
                .filter(|m| m.contains("override of"))
                .collect()
        }

        // Subclassing is `Parent <- Child <- { }`. Dog <: Animal, and B <: A below.
        // Widening an inherited return is a violation: base `get:` returns `Dog`, the override
        // returns `Animal` — a supertype, not usable where `Dog` is expected.
        let d = diags(
            "Animal <- { }; Animal <- Dog <- { }; \
             A <- { get: -> { |x ^Dog| x } }; A <- B <- { get: -> { |x ^Animal| x } }",
        );
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("override of `get:`") && d[0].contains("incompatible"));

        // Narrowing to a subtype is fine (covariant returns): base `Animal`, override `Dog`.
        let d = diags(
            "Animal <- { }; Animal <- Dog <- { }; \
             A <- { get: -> { |x ^Animal| x } }; A <- B <- { get: -> { |x ^Dog| x } }",
        );
        assert!(d.is_empty(), "{d:?}");

        // A confident scalar mismatch is flagged (base `String`, override `Integer`).
        let d = diags("A <- { n -> { |^String| 'x' } }; A <- B <- { n -> { |^Integer| 5 } }");
        assert_eq!(d.len(), 1, "{d:?}");

        // Same scalar return is silent.
        let d = diags("A <- { n -> { |^String| 'x' } }; A <- B <- { n -> { |^String| 'y' } }");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn defined_guard_inlines_directly_when_object_contract_is_known() {
        fn bytecode(seed_object: bool) -> Vec<Instruction> {
            let node = parse_quoin_string("var x = 5; x.defined?.if:{ 1 } else:{ 2 }");
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            if seed_object {
                // Simulate the loaded bootstrap contract `Object#defined? : Boolean`.
                let mut r = HashMap::new();
                r.insert(Arc::from("defined?"), Type::Bool);
                c.class_table.add_returns("Object", r);
            }
            c.compile_program(p)
                .unwrap()
                .bytecode
                .0
                .iter()
                .cloned()
                .collect()
        }
        let has_guard = |bc: &[Instruction]| {
            bc.iter()
                .any(|i| matches!(i, Instruction::BranchIfNotBool(_)))
        };

        // Without the contract `x.defined?` is `Any` → a *guarded* inline (a runtime Bool-check
        // that falls back to the real send for a non-Bool receiver).
        assert!(
            has_guard(&bytecode(false)),
            "expected a guarded inline without the Object contract"
        );
        // With `Object#defined? : Boolean` known, covariance makes `x.defined?` statically
        // `Boolean`, so the guard devirt-inlines as a *direct* Bool conditional — no runtime check.
        assert!(
            !has_guard(&bytecode(true)),
            "expected a direct inline with the Object contract"
        );
    }

    #[test]
    fn checker_flags_return_mismatches() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics().iter().map(|d| d.message.clone()).collect()
        }

        // A confident return mismatch is flagged (non-fatal).
        assert!(
            diags("F <- { m -> { |^Integer| 'x' } }")[0]
                .contains("expected `Integer`, found `String`")
        );
        // Correct returns are silent.
        assert!(diags("F <- { m -> { |^Integer| 40 + 2 } }").is_empty());
        assert!(diags("F <- { m -> { |^String| 'hi' } }").is_empty());
        // Nullable: `nil` satisfies `T?`.
        assert!(diags("F <- { m -> { |^Integer?| nil } }").is_empty());
        // Numeric literal promotion: an Integer literal where a Double is declared is fine…
        assert!(diags("F <- { m -> { |^Double| 1 } }").is_empty());
        // …but a non-constant Integer where a Double is expected is flagged (strict signatures).
        assert!(
            diags("F <- { m: -> { |n: Integer ^Double| n } }")[0]
                .contains("expected `Double`, found `Integer`")
        );
        // An explicit `^` return is checked too.
        assert!(diags("F <- { m -> { |^Integer| ^'x' } }")[0].contains("found `String`"));
    }

    #[test]
    fn checker_flags_decl_mismatches() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics().iter().map(|d| d.message.clone()).collect()
        }

        assert!(diags("var x: Integer = 'hi'")[0].contains("expected `Integer`, found `String`"));
        assert!(diags("var x: Integer = 5").is_empty());
        // Numeric literal promotion applies to initializers too.
        assert!(diags("var x: Double = 1").is_empty());
        assert!(diags("var x: String = 'hi'").is_empty());
        // Nullable: `nil` satisfies `T?`.
        assert!(diags("var x: Integer? = nil").is_empty());
        // A typed decl now resolves its annotation, so an unknown type is flagged.
        assert!(diags("var x: Nope = 5")[0].contains("unknown type `Nope`"));
    }

    #[test]
    fn checker_subtyping_via_class_table() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics().iter().map(|d| d.message.clone()).collect()
        }

        // `Animal <- Dog` makes Dog a subtype of Animal — a Dog where an Animal is wanted is fine.
        assert!(
            diags("Animal <- { }; Animal <- Dog <- { }; var d: Dog = Dog.new; var a: Animal = d")
                .is_empty()
        );
        // Unrelated classes in the same hierarchy still mismatch.
        assert!(
            diags("Animal <- { }; Animal <- Dog <- { }; Animal <- Cat <- { }; var d: Dog = Dog.new; var c: Cat = d")[0]
                .contains("expected `Cat`, found `Dog`")
        );
    }

    #[test]
    fn checker_flags_typed_reassignment() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics().iter().map(|d| d.message.clone()).collect()
        }

        // Reassigning an *annotated* var to a wrong type is flagged.
        assert!(
            diags("var x: Integer = 5; x = nil")[0].contains("expected `Integer`, found `Nil`")
        );
        assert!(
            diags("var x: Integer = 5; x = 'hi'")[0].contains("expected `Integer`, found `String`")
        );
        // Correct type — and a promotable Integer literal into a Double var — are silent.
        assert!(diags("var x: Integer = 5; x = 6").is_empty());
        assert!(diags("var x: Double = 1.0; x = 2").is_empty());
        // An UN-annotated var is untyped: its inferred type is a devirt hint, not a contract, so
        // reassigning it to any type is fine (the `optimisticIntFallback` corpus pattern).
        assert!(diags("var x = 5; x = 'hi'").is_empty());
    }

    #[test]
    fn narrowing_overlay_reads_innermost_scope() {
        // 3c·0 plumbing: the narrowing overlay stores per-scope refinements; innermost wins, and
        // an absent key falls through (gradual). No rules install narrowing yet, so this exercises
        // the store/lookup directly.
        let mut c = Compiler::new();
        let x = NarrowKey::Local("x".to_string());
        assert_eq!(c.narrowed_type(&x), None);

        c.scopes
            .last_mut()
            .unwrap()
            .narrowed
            .insert(x.clone(), Type::Int);
        assert_eq!(c.narrowed_type(&x), Some(Type::Int));

        // A pushed inner scope still sees the outer narrowing…
        c.push_scope(HashSet::new());
        assert_eq!(c.narrowed_type(&x), Some(Type::Int));
        // …but its own narrowing shadows it.
        c.scopes
            .last_mut()
            .unwrap()
            .narrowed
            .insert(x.clone(), Type::String);
        assert_eq!(c.narrowed_type(&x), Some(Type::String));

        // An absent key stays `None`.
        assert_eq!(c.narrowed_type(&NarrowKey::Field("y".to_string())), None);
    }

    #[test]
    fn checker_narrows_nullable_after_defined_guard() {
        // Narrowing is observable through the decl check: `var y: Integer = x` type-checks only
        // where `x: Integer?` has been narrowed non-nil.
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics()
                .iter()
                .filter(|d| d.message.contains("type mismatch"))
                .map(|d| d.message.clone())
                .collect()
        }

        // Unguarded: assigning a nullable to an `Integer` local → warns.
        assert!(!diags("Foo <- { m -> { |x: Integer?| var y: Integer = x } }").is_empty());
        // Guarded true-arm narrows `x` non-nil, so the arm's decl type-checks.
        assert!(
            diags("Foo <- { m -> { |x: Integer?| x.defined?.if:{ var y: Integer = x } } }")
                .is_empty()
        );
        // Divergent nil-arm: after `.else:{ ^^0 }`, `x` is non-nil for the rest of the body.
        assert!(
            diags("Foo <- { m -> { |x: Integer?| x.defined?.else:{ ^^0 }; var y: Integer = x } }")
                .is_empty()
        );
        // Reassignment re-narrows a declared nullable local: `x = 5` makes it non-nil.
        assert!(
            diags("Foo <- { m -> { var x: Integer? = nil; x = 5; var y: Integer = x } }")
                .is_empty()
        );
    }

    #[test]
    fn checker_flags_nil_misuse() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics()
                .iter()
                .filter(|d| d.message.contains("may be nil"))
                .map(|d| d.message.clone())
                .collect()
        }

        // A non-nil-safe send to an un-narrowed nullable → warns.
        assert!(!diags("Foo <- { m -> { |x: Integer?| x.abs } }").is_empty());
        // Guarded: `x` is narrowed non-nil in the arm → silent.
        assert!(diags("Foo <- { m -> { |x: Integer?| x.defined?.if:{ x.abs } } }").is_empty());
        // Nil-safe methods don't dereference → silent even on a nullable.
        assert!(diags("Foo <- { m -> { |x: Integer?| x.s } }").is_empty());
        // Non-nullable, and gradual `Any`, receivers → silent.
        assert!(diags("Foo <- { m -> { |x: Integer| x.abs } }").is_empty());
        assert!(diags("Foo <- { m -> { |x| x.abs } }").is_empty());
    }

    #[test]
    fn checker_nil_misuse_binops_and_conditions() {
        fn diags(src: &str) -> Vec<String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            let mut c = Compiler::new();
            c.compile_program(p).unwrap();
            c.diagnostics()
                .iter()
                .filter(|d| d.message.contains("may be nil"))
                .map(|d| d.message.clone())
                .collect()
        }

        // Binop nil-misuse: `x + 1` dereferences a nullable left → warns; `==` is nil-safe.
        assert!(!diags("Foo <- { m -> { |x: Integer?| x + 1 } }").is_empty());
        assert!(diags("Foo <- { m -> { |x: Integer?| x == 1 } }").is_empty());

        // `!= nil` / `== nil` guard conditions narrow their arms.
        assert!(diags("Foo <- { m -> { |x: Integer?| (x != nil).if:{ x + 1 } } }").is_empty());
        assert!(
            diags("Foo <- { m -> { |x: Integer?| (x == nil).if:{ 0 } else:{ x + 1 } } }")
                .is_empty()
        );

        // `&&` short-circuit narrows the RHS.
        assert!(diags("Foo <- { m -> { |x: Integer?| x.defined? && (x + 1) } }").is_empty());
    }

    #[test]
    fn strict_declaration_semantics() {
        fn compile_src(src: &str) -> Result<StaticBlock, String> {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            Compiler::new().compile_program(p)
        }

        // `var` declares; a later plain assignment reassigns the same binding.
        assert!(compile_src("var x = 5; x = 6").is_ok());
        assert!(compile_src("var a b = #(1 2); a b = #(3 4)").is_ok());
        assert!(compile_src("var f = { |n| n * f.value: 1 }").is_ok()); // recursive self-ref

        // A bare assignment to an undeclared local is a strict-mode error.
        let e = compile_src("z = 10").unwrap_err();
        assert!(e.contains("undeclared local"), "{e}");

        // A `let` binding cannot be reassigned.
        let e = compile_src("let w = 1; w = 2").unwrap_err();
        assert!(e.contains("let"), "{e}");

        // Re-declaring a name in the same scope is an error.
        let e = compile_src("var d = 1; var d = 2").unwrap_err();
        assert!(e.contains("already declared"), "{e}");

        // `var`/`let` cannot declare an instance variable.
        let e = compile_src("var @x = 1").unwrap_err();
        assert!(e.contains("instance variable"), "{e}");
    }

    /// Recursively check whether a static block (or any nested block) contains a
    /// `CallSelfDirect` instruction.
    fn has_call_self_direct(sb: &StaticBlock) -> bool {
        sb.bytecode.0.iter().any(|inst| match inst {
            Instruction::CallSelfDirect(..) => true,
            Instruction::Push(Constant::Block(nested)) => has_call_self_direct(nested),
            _ => false,
        })
    }

    #[test]
    fn sealed_self_send_emits_call_self_direct() {
        fn compile_src(src: &str) -> StaticBlock {
            let node = crate::parser::parse_quoin_string(src);
            let NodeValue::Program(p) = &node.value else {
                panic!("expected a program");
            };
            Compiler::new().compile_program(p).unwrap()
        }

        // A self-send to a same-class method of a SEALED class devirtualizes.
        let sealed = compile_src("Foo <- { bar: -> { |n| .bar:(n) }; .sealed! }");
        assert!(
            has_call_self_direct(&sealed),
            "sealed same-class self-send should emit CallSelfDirect"
        );

        // Without the seal it stays a normal dynamic Send.
        let unsealed = compile_src("Foo <- { bar: -> { |n| .bar:(n) } }");
        assert!(
            !has_call_self_direct(&unsealed),
            "un-sealed self-send must stay a Send"
        );
    }

    fn binary(op: BinaryOperatorType, left: Node, right: Node) -> Node {
        Node {
            source_info: None,
            value: NodeValue::BinaryOperator(BinaryOperatorNode {
                operator: op,
                left: Arc::new(left),
                right: Arc::new(right),
            }),
        }
    }

    fn unary(op: UnaryOperatorType, right: Node) -> Node {
        Node {
            source_info: None,
            value: NodeValue::UnaryOperator(UnaryOperatorNode {
                operator: op,
                right: Arc::new(right),
            }),
        }
    }

    fn call(subject: Option<Node>, selector_name: &str, args: Vec<Node>) -> Node {
        Node {
            source_info: None,
            value: NodeValue::MethodCall(MethodCallNode {
                subject: subject.map(Arc::new),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: selector_name.to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    expressions: args.into_iter().map(Arc::new).collect(),
                }),
            }),
        }
    }

    // Helper to compile ProgramNode
    fn compile(exprs: Vec<Node>) -> Result<StaticBlock, String> {
        let mut compiler = Compiler::new();
        let program = ProgramNode {
            expressions: exprs.into_iter().map(Arc::new).collect(),
            source_info: None,
        };
        let mut block = compiler.compile_program(&program)?;
        if block.bytecode.last() == Some(&Instruction::Return) {
            Rc::make_mut(&mut block.bytecode.0).pop();
        }
        Ok(block)
    }

    // Default prefix for every program
    fn prefix_ops() -> Vec<Instruction> {
        vec![
            Instruction::Push(Constant::Nil),
            Instruction::DefineLocal(Symbol::intern("self")),
        ]
    }

    // Apply the same superinstruction fusion the compiler runs, so these tests can express
    // their expected bytecode as the readable *unfused* lowering and assert the compiler
    // emits its fused form. (Fusion itself is pinned by the `fuse_*` tests above; for a
    // snippet with no fuseable pair this is the identity.)
    fn fused(v: Vec<Instruction>) -> Vec<Instruction> {
        let n = v.len();
        fuse_bytecode(v, vec![None; n]).0
    }

    #[test]
    fn test_compile_literals() {
        let res = compile(vec![int(123)]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(123)));
        assert_eq!(res.bytecode, fused(expected));

        let res = compile(vec![double(1.5)]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Double(1.5)));
        assert_eq!(res.bytecode, fused(expected));

        let res = compile(vec![string("hello")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("hello".to_string())));
        assert_eq!(res.bytecode, fused(expected));

        let res = compile(vec![sym("mysym")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Symbol("mysym".to_string())));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_identifiers() {
        let res = compile(vec![local_id("nil")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Nil));
        assert_eq!(res.bytecode, fused(expected));

        let res = compile(vec![local_id("true")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Bool(true)));
        assert_eq!(res.bytecode, fused(expected));

        let res = compile(vec![local_id("false")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Bool(false)));
        assert_eq!(res.bytecode, fused(expected));

        // self is always local
        let res = compile(vec![local_id("self")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadLocal(Symbol::intern("self")));
        assert_eq!(res.bytecode, fused(expected));

        // unknown name defaults to LoadGlobal
        let res = compile(vec![local_id("my_var")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("my_var")));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_assignments() {
        // Single global assignment
        let lval = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "x".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let res = compile(vec![assign_node(vec![lval.clone()], int(42))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(42)));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal(Symbol::intern("x")));
        assert_eq!(res.bytecode, fused(expected));

        // Destructuring assignment (e.g. a b = x)
        let lval_a = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "a".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_b = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "b".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let res = compile(vec![assign_node(vec![lval_a, lval_b], local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("a")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("b")));
        assert_eq!(res.bytecode, fused(expected));

        // Splat: *rest = x; (under destruct)
        let lval_rest = Node {
            source_info: None,
            value: NodeValue::SplatLValue(SplatLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "rest".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_ignore = Node {
            source_info: None,
            value: NodeValue::IgnoredLValue,
        };
        let res = compile(vec![assign_node(
            vec![lval_ignore, lval_rest],
            local_id("x"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send(Symbol::intern("sliceFrom:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("rest")));
        assert_eq!(res.bytecode, fused(expected));

        // IgnoredSplatLValue: _ *_ = x;
        let lval_ignore = Node {
            source_info: None,
            value: NodeValue::IgnoredLValue,
        };
        let lval_ignore_splat = Node {
            source_info: None,
            value: NodeValue::IgnoredSplatLValue,
        };
        let res = compile(vec![assign_node(
            vec![lval_ignore, lval_ignore_splat],
            local_id("x"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
        assert_eq!(res.bytecode, fused(expected));

        // SubLValue: a (b c) = x;
        let lval_a = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "a".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_b = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "b".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_c = Node {
            source_info: None,
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "c".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_nested = Node {
            source_info: None,
            value: NodeValue::SubLValue(SubLValueNode {
                lvalues: vec![Arc::new(lval_b), Arc::new(lval_c)],
            }),
        };
        let res = compile(vec![assign_node(vec![lval_a, lval_nested], local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("a")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_2")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_2")));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("b")));
        expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_2")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send(Symbol::intern("at:"), 1));
        expected.push(Instruction::DefineLocal(Symbol::intern("c")));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_method_calls() {
        // x.foo: 1
        let res = compile(vec![call(Some(local_id("x")), "foo", vec![int(1)])]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send(Symbol::intern("foo:"), 1));
        assert_eq!(res.bytecode, fused(expected));

        // Implicit subject (self): .foo
        let res = compile(vec![call(None, "foo", vec![])]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadLocal(Symbol::intern("self")));
        expected.push(Instruction::Send(Symbol::intern("foo"), 0));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_binary_unary_operators() {
        // 1 + 2  — two Integer literals devirtualize to a direct IntAdd (no method send).
        let res = compile(vec![binary(BinaryOperatorType::Add, int(1), int(2))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Push(Constant::Int(2)));
        expected.push(Instruction::IntAdd);
        assert_eq!(res.bytecode, fused(expected));

        // -x
        let res = compile(vec![unary(UnaryOperatorType::Sub, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Send(Symbol::intern("-"), 0));
        assert_eq!(res.bytecode, fused(expected));

        // !x
        let res = compile(vec![unary(UnaryOperatorType::Bang, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Send(Symbol::intern("!"), 0));
        assert_eq!(res.bytecode, fused(expected));

        // +x
        let res = compile(vec![unary(UnaryOperatorType::Add, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Send(Symbol::intern("+"), 0));
        assert_eq!(res.bytecode, fused(expected));

        // x && y
        let res = compile(vec![binary(
            BinaryOperatorType::And,
            local_id("x"),
            local_id("y"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::ElseJump(3));
        expected.push(Instruction::Pop);
        expected.push(Instruction::LoadGlobal(ns("y")));
        assert_eq!(res.bytecode, fused(expected));

        // x || y
        let res = compile(vec![binary(
            BinaryOperatorType::Or,
            local_id("x"),
            local_id("y"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Dup);
        expected.push(Instruction::IfJump(3));
        expected.push(Instruction::Pop);
        expected.push(Instruction::LoadGlobal(ns("y")));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_blocks() {
        // { |x| x + 1 }
        let block_node = BlockNode {
            return_type: None,
            source_info: None,
            name: None,
            arguments: vec![Arc::new(BlockArgNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "x".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                type_hint: None,
            })],
            decls: vec![],
            decl_block: None,
            statements: vec![Arc::new(binary(
                BinaryOperatorType::Add,
                local_id("x"),
                int(1),
            ))],
        };
        let res = compile(vec![Node {
            source_info: None,
            value: NodeValue::Block(block_node),
        }])
        .unwrap();

        // The inner block body fuses too: LoadLocal(x); Push(1); Send(+:) -> LoadLocal(x);
        // SendConst(1, +:). Fuse the readable lowering (bytecode + source map together).
        let (inner_bc, inner_sm) = fuse_bytecode(
            vec![
                Instruction::LoadLocal(Symbol::intern("x")),
                Instruction::Push(Constant::Int(1)),
                Instruction::Send(Symbol::intern("+:"), 1),
                Instruction::Return,
            ],
            vec![None; 4],
        );
        let inner_static = StaticBlock {
            name: None,
            is_nested_block: true,
            param_syms: crate::value::intern_param_syms(&vec!["x".to_string()]),
            param_types: vec!["Object".to_string()],
            bytecode: SharedBytecode(Rc::new(inner_bc)),
            source_info: None,
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(inner_sm)),
        };
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Block(inner_static)));
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_lists_maps_regex() {
        // #(1 2)
        let list = Node {
            source_info: None,
            value: NodeValue::List(ListNode {
                values: vec![Arc::new(int(1)), Arc::new(int(2))],
            }),
        };
        let res = compile(vec![list]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Push(Constant::Int(2)));
        expected.push(Instruction::NewList(2));
        assert_eq!(res.bytecode, fused(expected));

        // #{'a': 1}
        let map = Node {
            source_info: None,
            value: NodeValue::Map(MapNode {
                keys: vec![Arc::new(string("a"))],
                values: vec![Arc::new(int(1))],
            }),
        };
        let res = compile(vec![map]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("a".to_string())));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::NewMap(1));
        assert_eq!(res.bytecode, fused(expected));

        // #/^[a-z]+$/
        let regex = Node {
            source_info: None,
            value: NodeValue::Regex(RegexNode {
                value: "#/^[a-z]+$/".to_string(),
            }),
        };
        let res = compile(vec![regex]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("^[a-z]+$".to_string())));
        expected.push(Instruction::NewRegex);
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_compile_errors_and_fallbacks() {
        // Unknown NodeValue returns error
        let res = compile(vec![Node {
            source_info: None,
            value: NodeValue::Unknown,
        }]);
        assert!(res.is_err());
        assert_eq!(
            res.err().unwrap(),
            "Encountered Unknown NodeValue (ast_visitor bug)"
        );

        // Map mismatch keys/values returns error
        let map_mismatch = Node {
            source_info: None,
            value: NodeValue::Map(MapNode {
                keys: vec![Arc::new(string("a"))],
                values: vec![],
            }),
        };
        let res = compile(vec![map_mismatch]);
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "Map keys and values count mismatch");
    }

    #[test]
    fn test_compile_class_and_method_definitions() {
        let block_node = BlockNode {
            return_type: None,
            source_info: None,
            arguments: vec![
                Arc::new(BlockArgNode {
                    identifier: Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: "a".to_string(),
                        identifier_type: IdentifierType::Instance,
                    }),
                    type_hint: None,
                }),
                Arc::new(BlockArgNode {
                    identifier: Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: "b".to_string(),
                        identifier_type: IdentifierType::Instance,
                    }),
                    type_hint: None,
                }),
            ],
            decls: vec![],
            decl_block: None,
            statements: vec![],
            name: None,
        };
        let class_def = Node {
            source_info: None,
            value: NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "MyClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: Some(Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "Object".to_string(),
                    identifier_type: IdentifierType::Local,
                })),
                block: Arc::new(block_node.clone()),
            }),
        };

        let res = compile(vec![class_def]).unwrap();
        let expected_block = StaticBlock {
            name: None,
            is_nested_block: true,
            param_syms: crate::value::intern_param_syms(&vec!["a".to_string(), "b".to_string()]),
            param_types: vec!["Object".to_string(), "Object".to_string()],
            bytecode: SharedBytecode(Rc::new(vec![
                Instruction::Push(Constant::Nil),
                Instruction::Return,
            ])),
            source_info: None,
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(vec![None; 2])),
        };
        let mut expected = prefix_ops();
        expected.push(Instruction::DefineClass {
            name: ns("MyClass"),
            parent_name: Some(ns("Object")),
            instance_vars: vec!["a".to_string(), "b".to_string()],
        });
        expected.push(Instruction::Push(Constant::Block(expected_block)));
        expected.push(Instruction::ExecuteBlockWithSelf);
        assert_eq!(res.bytecode, fused(expected));
    }

    #[test]
    fn test_source_info_propagation() {
        let code = "{ 1 + 2 };";
        let ast = parse_quoin_string(code);
        let mut compiler = Compiler::new();

        // The root program node itself should have the source info
        if let NodeValue::Program(ref prog) = ast.value {
            let info = prog.source_info.as_ref().unwrap();
            assert_eq!(info.filename, "<string>");
            assert_eq!(info.line, 1);
            assert_eq!(info.column, 0);
            assert_eq!(
                info.source_text.as_ref().map(|s| s.as_str()),
                Some("{ 1 + 2 };")
            );
        } else {
            panic!("Expected Program node");
        }

        let compiled = compiler
            .compile_program(match &ast.value {
                NodeValue::Program(p) => p,
                _ => unreachable!(),
            })
            .unwrap();

        // The program compiled StaticBlock should have source info
        assert!(compiled.source_info.is_some());
        let prog_info = compiled.source_info.as_ref().unwrap();
        assert_eq!(prog_info.filename, "<string>");

        // Let's find the inner block pushed in the bytecode
        let mut found_inner_block = false;
        for instr in compiled.bytecode.iter().cloned() {
            if let Instruction::Push(Constant::Block(sb)) = instr {
                found_inner_block = true;
                assert!(sb.source_info.is_some());
                let info = sb.source_info.as_ref().unwrap();
                assert_eq!(info.filename, "<string>");
                assert_eq!(info.line, 1);
                assert_eq!(info.column, 0);
                assert_eq!(
                    info.source_text.as_ref().map(|s| s.as_str()),
                    Some("{ 1 + 2 }")
                );
            }
        }
        assert!(found_inner_block);
    }

    // --- superinstruction fusion (`fuse_bytecode`) ---

    fn si(line: usize) -> Option<SourceInfo> {
        Some(SourceInfo {
            filename: String::new(),
            line,
            column: 0,
            start: 0,
            end: 0,
            source_text: None,
        })
    }

    #[test]
    fn fuse_basic_operand_send_pairs() {
        let sel = Symbol::intern("foo:");
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("a")),
            Instruction::Send(sel, 1),
            Instruction::Push(Constant::Int(3)),
            Instruction::Send(sel, 1),
            Instruction::LoadField("x".into()),
            Instruction::Send(sel, 1),
            Instruction::Return,
        ];
        let (out, out_smap) = fuse_bytecode(code.clone(), vec![None; code.len()]);
        assert_eq!(
            out,
            vec![
                Instruction::SendLocal(Symbol::intern("a"), sel, 1),
                Instruction::SendConst(Constant::Int(3), sel, 1),
                Instruction::SendField("x".into(), sel, 1),
                Instruction::Return,
            ]
        );
        assert_eq!(out.len(), out_smap.len());
    }

    #[test]
    fn fuse_leaves_non_fuseable_sends_alone() {
        // A Send with no preceding fuseable operand-load stays a plain Send.
        let sel = Symbol::intern("g");
        let code = vec![Instruction::Send(sel, 0), Instruction::Return];
        let (out, _) = fuse_bytecode(code.clone(), vec![None; code.len()]);
        assert_eq!(out, code);
    }

    #[test]
    fn fuse_does_not_cross_jump_target() {
        let sel = Symbol::intern("f");
        // The IfJump targets the Send of a (LoadLocal, Send) pair — fusing would let the
        // jump skip the LoadLocal, so it must stay unfused.
        let code = vec![
            Instruction::Push(Constant::Bool(true)),     // 0
            Instruction::IfJump(3),                      // 1 -> target 4 (the Send)
            Instruction::Push(Constant::Nil),            // 2
            Instruction::LoadLocal(Symbol::intern("a")), // 3
            Instruction::Send(sel, 1),                   // 4  (jump target)
            Instruction::Return,                         // 5
        ];
        let (out, _) = fuse_bytecode(code.clone(), vec![None; code.len()]);
        assert_eq!(out, code); // nothing fuseable here, all left intact
        let jpos = out
            .iter()
            .position(|i| matches!(i, Instruction::IfJump(_)))
            .unwrap();
        if let Instruction::IfJump(off) = out[jpos] {
            assert!(matches!(
                out[(jpos as isize + off) as usize],
                Instruction::Send(_, _)
            ));
        }
    }

    #[test]
    fn fuse_fixes_forward_jump_offset() {
        let sel = Symbol::intern("f");
        // Jump forward *over* a fused pair: the collapsed slot shrinks the offset.
        let code = vec![
            Instruction::Push(Constant::Bool(true)),     // 0
            Instruction::IfJump(4),                      // 1 -> target 5 (Return)
            Instruction::LoadLocal(Symbol::intern("a")), // 2 \ fuse
            Instruction::Send(sel, 0),                   // 3 /
            Instruction::Pop,                            // 4
            Instruction::Return,                         // 5  (target)
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 6]);
        assert_eq!(
            out,
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::IfJump(3),
                Instruction::SendLocal(Symbol::intern("a"), sel, 0),
                Instruction::Pop,
                Instruction::Return,
            ]
        );
        if let Instruction::IfJump(off) = out[1] {
            assert!(matches!(out[(1 + off) as usize], Instruction::Return));
        }
    }

    #[test]
    fn fuse_fixes_backward_jump_offset() {
        let sel = Symbol::intern("f");
        // Back-edge over a fused pair at the loop top: offset grows toward 0 by one.
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("a")), // 0 \ fuse (loop top)
            Instruction::Send(sel, 0),                   // 1 /
            Instruction::Push(Constant::Bool(true)),     // 2
            Instruction::IfJump(-3),                     // 3 -> target 0
            Instruction::Return,                         // 4
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 5]);
        assert_eq!(
            out,
            vec![
                Instruction::SendLocal(Symbol::intern("a"), sel, 0),
                Instruction::Push(Constant::Bool(true)),
                Instruction::IfJump(-2),
                Instruction::Return,
            ]
        );
        if let Instruction::IfJump(off) = out[2] {
            assert!(matches!(
                out[(2 + off) as usize],
                Instruction::SendLocal(..)
            ));
        }
    }

    #[test]
    fn fuse_keeps_source_map_aligned_to_send() {
        let sel = Symbol::intern("f");
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("a")),
            Instruction::Send(sel, 0),
            Instruction::Return,
        ];
        let (out, out_smap) = fuse_bytecode(code, vec![si(1), si(2), si(3)]);
        assert_eq!(out.len(), out_smap.len());
        // The fused slot keeps the Send's entry (line 2), not the LoadLocal's (line 1).
        assert_eq!(out_smap[0], si(2));
        assert_eq!(out_smap[1], si(3));
    }

    #[test]
    fn fuse_dup_store_pop_collapses_to_plain_store() {
        // Statement assignment: Dup; Store; Pop -> Store (drops Dup + Pop).
        let code = vec![
            Instruction::Push(Constant::Int(1)),
            Instruction::Dup,
            Instruction::StoreLocal(Symbol::intern("x")),
            Instruction::Pop,
            Instruction::Return,
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 5]);
        assert_eq!(
            out,
            vec![
                Instruction::Push(Constant::Int(1)),
                Instruction::StoreLocal(Symbol::intern("x")),
                Instruction::Return,
            ]
        );
    }

    #[test]
    fn fuse_dup_store_keeps_in_expression_position() {
        // Expression assignment (no trailing Pop): Dup; StoreField -> StoreFieldKeep.
        let code = vec![
            Instruction::Push(Constant::Int(1)),
            Instruction::Dup,
            Instruction::StoreField("y".into()),
            Instruction::Return,
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 4]);
        assert_eq!(
            out,
            vec![
                Instruction::Push(Constant::Int(1)),
                Instruction::StoreFieldKeep("y".into()),
                Instruction::Return,
            ]
        );
    }

    #[test]
    fn fuse_dup_store_pop_respects_jump_into_the_pop() {
        // A jump targets the Pop -> can't drop it; fall back to the keep variant and fix
        // the offset so the jump still lands on the standalone Pop.
        let code = vec![
            Instruction::Push(Constant::Bool(true)),      // 0
            Instruction::IfJump(4),                       // 1 -> target 5 (the Pop)
            Instruction::Push(Constant::Int(1)),          // 2
            Instruction::Dup,                             // 3
            Instruction::StoreLocal(Symbol::intern("x")), // 4
            Instruction::Pop,                             // 5  (jump target)
            Instruction::Return,                          // 6
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 7]);
        assert_eq!(
            out,
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::IfJump(3),
                Instruction::Push(Constant::Int(1)),
                Instruction::StoreLocalKeep(Symbol::intern("x")),
                Instruction::Pop,
                Instruction::Return,
            ]
        );
        if let Instruction::IfJump(off) = out[1] {
            assert!(matches!(out[(1 + off) as usize], Instruction::Pop));
        }
    }

    #[test]
    fn fuse_dup_store_not_fused_when_store_is_jump_target() {
        // A jump targets the store itself (skipping the Dup) -> no fusion at all.
        let code = vec![
            Instruction::Push(Constant::Bool(true)),      // 0
            Instruction::IfJump(3),                       // 1 -> target 4 (the store)
            Instruction::Push(Constant::Int(1)),          // 2
            Instruction::Dup,                             // 3
            Instruction::StoreLocal(Symbol::intern("x")), // 4  (jump target)
            Instruction::Return,                          // 5
        ];
        let (out, _) = fuse_bytecode(code.clone(), vec![None; 6]);
        assert_eq!(out, code);
    }

    #[test]
    fn fuse_3instr_send_local_local() {
        let sel = Symbol::intern("foo:");
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("a")),
            Instruction::LoadLocal(Symbol::intern("b")),
            Instruction::Send(sel, 1),
            Instruction::Return,
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 4]);
        assert_eq!(
            out,
            vec![
                Instruction::SendLocalLocal(Symbol::intern("a"), Symbol::intern("b"), sel, 1),
                Instruction::Return,
            ]
        );
    }

    #[test]
    fn fuse_3instr_send_local_const() {
        let sel = Symbol::intern("-:");
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("n")),
            Instruction::Push(Constant::Int(1)),
            Instruction::Send(sel, 1),
            Instruction::Return,
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 4]);
        assert_eq!(
            out,
            vec![
                Instruction::SendLocalConst(Symbol::intern("n"), Constant::Int(1), sel, 1),
                Instruction::Return,
            ]
        );
    }

    #[test]
    fn fuse_3instr_absorbs_only_the_last_two_operands() {
        // A 2-arg send: the receiver load stays, the last two operand loads fuse.
        let sel = Symbol::intern("at:put:");
        let code = vec![
            Instruction::LoadLocal(Symbol::intern("list")),
            Instruction::LoadLocal(Symbol::intern("i")),
            Instruction::LoadLocal(Symbol::intern("v")),
            Instruction::Send(sel, 2),
            Instruction::Return,
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 5]);
        assert_eq!(
            out,
            vec![
                Instruction::LoadLocal(Symbol::intern("list")),
                Instruction::SendLocalLocal(Symbol::intern("i"), Symbol::intern("v"), sel, 2),
                Instruction::Return,
            ]
        );
    }

    #[test]
    fn fuse_3instr_fixes_jump_offset() {
        let sel = Symbol::intern("f");
        // Jump forward over a 3->1 collapse: offset shrinks by two.
        let code = vec![
            Instruction::Push(Constant::Bool(true)),     // 0
            Instruction::IfJump(5),                      // 1 -> target 6 (Return)
            Instruction::LoadLocal(Symbol::intern("a")), // 2 \
            Instruction::LoadLocal(Symbol::intern("b")), // 3  > fuse
            Instruction::Send(sel, 1),                   // 4 /
            Instruction::Pop,                            // 5
            Instruction::Return,                         // 6  (target)
        ];
        let (out, _) = fuse_bytecode(code, vec![None; 7]);
        assert_eq!(
            out,
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::IfJump(3),
                Instruction::SendLocalLocal(Symbol::intern("a"), Symbol::intern("b"), sel, 1),
                Instruction::Pop,
                Instruction::Return,
            ]
        );
        if let Instruction::IfJump(off) = out[1] {
            assert!(matches!(out[(1 + off) as usize], Instruction::Return));
        }
    }
}
