use crate::instruction::{Constant, Instruction, SharedBytecode, SharedSourceMap, StaticBlock};
use crate::parser::ast::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, DeclKind, DeclarationNode,
    IdentifierNode, IdentifierType, MethodCallNode, MethodSelectorNode, Node, NodeValue,
    ProgramNode, UnaryOperatorNode, UnaryOperatorType,
};
use crate::symbol::Symbol;
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
        Instruction::Jump(o) | Instruction::IfJump(o) | Instruction::ElseJump(o) => Some(*o),
        _ => None,
    }
}

fn set_jump_offset(inst: &mut Instruction, off: isize) {
    match inst {
        Instruction::Jump(o) | Instruction::IfJump(o) | Instruction::ElseJump(o) => *o = off,
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

/// Statically-known type of an expression, used to devirtualize numeric operators
/// (Slice 2a). `Int`/`Bool` are proven; everything dynamic is `Unknown` (the default,
/// so untyped code compiles exactly as before). Double is intentionally not tracked
/// yet (Integer-only slice — see docs/TYPED_DEVIRT_ARCH.md §10 decision B).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StaticType {
    Int,
    Bool,
    Unknown,
}

/// Map a declared type name (param/local annotation) to a tracked `StaticType`.
fn static_type_from_name(name: &str) -> StaticType {
    match name {
        "Integer" => StaticType::Int,
        "Boolean" => StaticType::Bool,
        _ => StaticType::Unknown,
    }
}

struct Scope {
    locals: HashSet<String>,
    /// Subset of `locals` declared with `let` — reassigning one is a compile error.
    immutable: HashSet<String>,
    /// Declared type of a local/param, when known (Integer/Boolean); absent = Unknown.
    types: HashMap<String, StaticType>,
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
    /// Stack of "current class" method return types (selector → declared `StaticType`),
    /// pushed while compiling a class body. A self-send to a method with an `Integer`
    /// return is statically `Int`, so arithmetic on its result devirtualizes (Slice 2b-A).
    method_returns: Vec<HashMap<String, StaticType>>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope {
                locals: HashSet::new(),
                immutable: HashSet::new(),
                types: HashMap::new(),
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            method_returns: Vec::new(),
        }
    }

    pub fn new_with_locals(locals: HashSet<String>) -> Self {
        Self {
            scopes: vec![Scope {
                locals,
                immutable: HashSet::new(),
                types: HashMap::new(),
                is_init: false,
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            method_returns: Vec::new(),
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

    /// Declared `StaticType` of a local/param — the nearest binding's recorded type,
    /// or `Unknown` (untyped, or not a plain local).
    fn local_type(&self, name: &str) -> StaticType {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope
                    .types
                    .get(name)
                    .copied()
                    .unwrap_or(StaticType::Unknown);
            }
        }
        StaticType::Unknown
    }

    /// Record a known type for a local just declared in the innermost scope.
    fn record_local_type(&mut self, name: &str, ty: StaticType) {
        if ty != StaticType::Unknown {
            self.scopes
                .last_mut()
                .unwrap()
                .types
                .insert(name.to_string(), ty);
        }
    }

    /// Statically infer an expression's type for devirtualization. Conservative: only
    /// literals, typed locals/params, and numeric operators on them are known; anything
    /// else is `Unknown` and compiles to a normal dynamic `Send`.
    fn static_type(&self, node: &Node) -> StaticType {
        match &node.value {
            NodeValue::Integer(_) => StaticType::Int,
            NodeValue::Identifier(id) => {
                if id.namespace.is_none()
                    && id.identifier_type != IdentifierType::Namespaced
                    && id.identifier_type != IdentifierType::Instance
                {
                    self.local_type(&id.name)
                } else {
                    StaticType::Unknown
                }
            }
            NodeValue::BinaryOperator(op) => self.binop_result_type(op),
            NodeValue::MethodCall(call) => self.self_send_return_type(call),
            _ => StaticType::Unknown,
        }
    }

    /// A self-send (`.sel:(…)` — no explicit receiver, or an explicit `self`) to a
    /// current-class method with a declared return type is statically that type. Non-self
    /// sends, unknown selectors, and variadic sends stay `Unknown` (a safe miss).
    fn self_send_return_type(&self, call: &MethodCallNode) -> StaticType {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        if !is_self {
            return StaticType::Unknown;
        }
        let Some(frame) = self.method_returns.last() else {
            return StaticType::Unknown;
        };
        let idents = &call.arguments.signature.identifiers;
        if idents.is_empty() {
            return StaticType::Unknown;
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
        frame.get(&selector).copied().unwrap_or(StaticType::Unknown)
    }

    /// Selector → declared-return-`StaticType` map for a class body, from its method
    /// definitions/extensions that carry a return type.
    fn collect_method_returns(&self, block: &BlockNode) -> HashMap<String, StaticType> {
        let mut map = HashMap::new();
        for stmt in &block.statements {
            let (sig, ret) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.return_type),
                NodeValue::MethodExtension(m) => (&m.signature, &m.return_type),
                _ => continue,
            };
            if let Some(rt) = ret {
                if let Ok(selector) = self.reconstruct_selector(sig) {
                    map.insert(selector, static_type_from_name(&rt.name));
                }
            }
        }
        map
    }

    /// Result type of a binary operator when both operands are statically `Int`:
    /// arithmetic yields `Int`, comparison yields `Bool`; otherwise `Unknown`.
    fn binop_result_type(&self, op: &BinaryOperatorNode) -> StaticType {
        use BinaryOperatorType::*;
        if self.static_type(&op.left) == StaticType::Int
            && self.static_type(&op.right) == StaticType::Int
        {
            match op.operator {
                Add | Sub | Mul | Div | Mod => StaticType::Int,
                Lt | LtEq | Gt | GtEq | Eq | NotEq => StaticType::Bool,
                _ => StaticType::Unknown,
            }
        } else {
            StaticType::Unknown
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
                self.compile_node(&ret.value, bytecode)?;
                bytecode.push(Instruction::BlockReturn);
            }
            NodeValue::MethodReturn(ret) => {
                self.compile_node(&ret.value, bytecode)?;
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
                let mrets = self.collect_method_returns(&class_def.block);
                self.method_returns.push(mrets);
                let r = self.compile_block(&class_def.block, bytecode);
                self.method_returns.pop();
                if is_value_type {
                    self.value_type_def_depth -= 1;
                }
                r?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::ClassExtension(class_ext) => {
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
                let mrets = self.collect_method_returns(&class_ext.block);
                self.method_returns.push(mrets);
                let r = self.compile_block(&class_ext.block, bytecode);
                self.method_returns.pop();
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
        self.compile_node(&assign.rvalue, bytecode)?;

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

        self.compile_node(&decl.rvalue, bytecode)?;

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

    fn compile_method_call(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        let args = &call.arguments;

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
            bytecode.push(Instruction::Send(Symbol::intern(&selector), 0));
            return Ok(());
        }

        // Keyword send. Keywords and argument expressions are 1:1 here (the parser builds them in
        // lockstep). A run of the *same* consecutive keyword is a variadic group: its arguments
        // fold into one `List` and the keyword interns as `name+:`, matching a `name+:` method
        // definition. A lone keyword stays `name:`. This is resolved entirely at compile time, so
        // dispatch only ever sees a canonical interned selector — no runtime collapse.
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
                self.compile_node(arg, bytecode)?;
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

        bytecode.push(Instruction::Send(Symbol::intern(&selector), num_components));
        Ok(())
    }

    fn compile_binary_operator(
        &mut self,
        op: &BinaryOperatorNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        if op.operator == BinaryOperatorType::And {
            self.compile_node(&op.left, bytecode)?;
            bytecode.push(Instruction::Dup);

            let mut right_bytecode = CodeBlock::new();
            right_bytecode.current_source = bytecode.current_source.clone();
            self.compile_node(&op.right, &mut right_bytecode)?;

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
        let devirt = self.static_type(&op.left) == StaticType::Int
            && self.static_type(&op.right) == StaticType::Int;

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

        // Seed declared param types (Integer/Boolean) so arithmetic on a typed param
        // devirtualizes. Dispatch only selects a typed method when the arg matches, so
        // the param is provably that type inside the body — no runtime guard needed.
        for (name, tyname) in param_names.iter().zip(param_types.iter()) {
            self.record_local_type(name, static_type_from_name(tyname));
        }

        let mut block_bytecode = CodeBlock::new();
        block_bytecode.current_source = block.source_info.clone();

        for name in &decls_names {
            block_bytecode.push(Instruction::Push(Constant::Nil));
            block_bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
        }

        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            block_bytecode.current_source = stmt.source_info.clone();
            self.compile_node(stmt, &mut block_bytecode)?;
            if idx < len - 1 {
                block_bytecode.push(Instruction::Pop);
            }
        }

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
