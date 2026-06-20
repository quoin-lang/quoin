use crate::instruction::{Constant, Instruction, SharedBytecode, SharedSourceMap, StaticBlock};
use crate::parser::ast::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, IdentifierType,
    MethodCallNode, MethodSelectorNode, Node, NodeValue, ProgramNode, UnaryOperatorNode,
    UnaryOperatorType,
};
use crate::value::{NamespacedName, SourceInfo};

use std::collections::HashSet;
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

struct Scope {
    locals: HashSet<String>,
}

pub struct Compiler {
    scopes: Vec<Scope>,
    temp_counter: usize,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope {
                locals: HashSet::new(),
            }],
            temp_counter: 0,
        }
    }

    pub fn new_with_locals(locals: HashSet<String>) -> Self {
        Self {
            scopes: vec![Scope { locals }],
            temp_counter: 0,
        }
    }

    fn new_temp_var(&mut self) -> String {
        self.temp_counter += 1;
        format!("__bb_temp_{}", self.temp_counter)
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
        self.scopes.push(Scope { locals });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn compile_program(&mut self, program: &ProgramNode) -> Result<StaticBlock, String> {
        let mut cb = CodeBlock::new();

        // Define default top-level self = nil
        cb.current_source = program.source_info.clone();
        cb.push(Instruction::Push(Constant::Nil));
        cb.push(Instruction::DefineLocal("self".to_string()));
        self.scopes[0].locals.insert("self".to_string());

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

        Ok(StaticBlock {
            name: None,
            is_nested_block: false,
            param_names: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode(Rc::new(cb.bytecode)),
            source_info: program.source_info.clone(),
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(cb.source_map)),
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
                bytecode.push(Instruction::Push(Constant::String(s.value.clone())));
            }
            NodeValue::Identifier(id) => {
                if id.identifier_type == IdentifierType::Instance {
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
                    bytecode.push(Instruction::LoadLocal(id.name.clone()));
                } else {
                    let ns_name = NamespacedName::new(Vec::new(), id.name.clone());
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                }
            }
            NodeValue::Assignment(assign) => {
                self.compile_assignment(assign, bytecode)?;
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
                bytecode.push(Instruction::Send("yield:".to_string(), 1));
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
                bytecode.push(Instruction::DefineClass {
                    name,
                    parent_name,
                    instance_vars,
                });
                self.compile_block(&class_def.block, bytecode)?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::ClassExtension(class_ext) => {
                self.compile_node(&class_ext.expression, bytecode)?;
                self.compile_block(&class_ext.block, bytecode)?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::MethodDefinition(method_def) => {
                let mut selector = self.reconstruct_selector(&method_def.signature)?;
                if selector == "-" && method_def.block.arguments.is_empty() {
                    selector = "negated".to_string();
                } else if selector == "+" && method_def.block.arguments.is_empty() {
                    selector = "posated".to_string();
                }
                self.compile_block(&method_def.block, bytecode)?;
                bytecode.push(Instruction::DefineMethod(selector));
            }
            NodeValue::MethodExtension(method_ext) => {
                let mut selector = self.reconstruct_selector(&method_ext.signature)?;
                if selector == "-" && method_ext.block.arguments.is_empty() {
                    selector = "negated".to_string();
                } else if selector == "+" && method_ext.block.arguments.is_empty() {
                    selector = "posated".to_string();
                }
                self.compile_block(&method_ext.block, bytecode)?;
                bytecode.push(Instruction::OverrideMethod(selector));
            }
            NodeValue::ConstDefinition(const_def) => {
                let ns_name = NamespacedName::from_ast(&const_def.identifier);
                self.compile_node(&const_def.rvalue, bytecode)?;
                bytecode.push(Instruction::Dup);
                bytecode.push(Instruction::StoreGlobal(ns_name, true));
            }
            NodeValue::UserString(user_str) => {
                let ns_name = NamespacedName::from_ast(&user_str.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                bytecode.push(Instruction::Push(Constant::String(user_str.value.clone())));
                bytecode.push(Instruction::Send("newUserString:".to_string(), 1));
            }
            NodeValue::UserList(user_list) => {
                let ns_name = NamespacedName::from_ast(&user_list.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                for val in &user_list.values {
                    self.compile_node(val, bytecode)?;
                }
                bytecode.push(Instruction::NewList(user_list.values.len()));
                bytecode.push(Instruction::Send("newUserList:".to_string(), 1));
            }
            NodeValue::Dot3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("...".to_string())));
                bytecode.push(Instruction::Send("throw".to_string(), 0));
            }
            NodeValue::Huh3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("???".to_string())));
                bytecode.push(Instruction::Send("throw".to_string(), 0));
            }
            NodeValue::Bang3 => {
                // TODO: For now, just throw the string.
                bytecode.push(Instruction::Push(Constant::String("!!!".to_string())));
                bytecode.push(Instruction::Send("throw".to_string(), 0));
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

        let mut target_names = Vec::new();
        self.collect_lvalue_names(&assign.lvalues, &mut target_names);

        let mut pre_declared = Vec::new();
        for name in &target_names {
            if !self.is_local(name) {
                self.scopes.last_mut().unwrap().locals.insert(name.clone());
                pre_declared.push(name.clone());
            }
        }

        self.compile_node(&assign.rvalue, bytecode)?;

        for name in &pre_declared {
            self.scopes.last_mut().unwrap().locals.remove(name);
        }

        if assign.lvalues.len() == 1 {
            let lval = &assign.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_lvalue_store(lval, bytecode)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(temp_var.clone()));

            self.compile_destruct(&assign.lvalues, &temp_var, bytecode)?;
        }

        Ok(())
    }

    fn compile_lvalue_store(
        &mut self,
        lval: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        match &lval.value {
            NodeValue::IdentLValue(ident_lval) => {
                let id = &ident_lval.identifier;
                if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::StoreGlobal(ns_name, false));
                } else {
                    let name = &id.name;
                    self.compile_ident_store(&id.identifier_type, name, bytecode);
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
    ) {
        let first_char = name.chars().next().unwrap_or('\0');
        if first_char.is_ascii_uppercase() {
            let ns_name = NamespacedName::new(Vec::new(), name.clone());
            bytecode.push(Instruction::StoreGlobal(ns_name, false));
        } else if ident_type == &IdentifierType::Instance {
            bytecode.push(Instruction::StoreField(name.clone()));
        } else if self.is_local(name) {
            bytecode.push(Instruction::StoreLocal(name.clone()));
        } else {
            self.scopes.last_mut().unwrap().locals.insert(name.clone());
            bytecode.push(Instruction::DefineLocal(name.clone()));
        }
    }

    fn compile_destruct(
        &mut self,
        lvalues: &[Arc<Node>],
        temp_var: &str,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        for (i, lval) in lvalues.iter().enumerate() {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let name = &ident_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("at:".to_string(), 1));

                    self.compile_ident_store(
                        &ident_lval.identifier.identifier_type,
                        name,
                        bytecode,
                    );
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let name = &splat_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("sliceFrom:".to_string(), 1));

                    self.compile_ident_store(
                        &splat_lval.identifier.identifier_type,
                        name,
                        bytecode,
                    );
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

                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("at:".to_string(), 1));
                    bytecode.push(Instruction::DefineLocal(nested_temp.clone()));

                    self.compile_destruct(&sub_lval.lvalues, &nested_temp, bytecode)?;
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
            bytecode.push(Instruction::LoadLocal("self".to_string()));
        }

        // Evaluate args
        for expr in &args.expressions {
            self.compile_node(expr, bytecode)?;
        }

        // Reconstruct selector string
        let is_w_args = !args.expressions.is_empty();
        let selector = if is_w_args {
            let mut s = String::new();
            for ident in &args.signature.identifiers {
                s.push_str(&ident.name);
                s.push(':');
            }
            s
        } else {
            if args.signature.identifiers.is_empty() {
                return Err("No identifiers found in method call selector".to_string());
            }
            args.signature.identifiers[0].name.clone()
        };

        let num_args = args.expressions.len();
        bytecode.push(Instruction::Send(selector, num_args));
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

        self.compile_node(&op.left, bytecode)?;
        self.compile_node(&op.right, bytecode)?;

        let selector = match op.operator {
            BinaryOperatorType::Add => "+",
            BinaryOperatorType::Sub => "-",
            BinaryOperatorType::Mul => "*",
            BinaryOperatorType::Div => "/",
            BinaryOperatorType::Eq => "==",
            BinaryOperatorType::NotEq => "!=",
            BinaryOperatorType::Lt => "<",
            BinaryOperatorType::Gt => ">",
            BinaryOperatorType::LtEq => "<=",
            BinaryOperatorType::GtEq => ">=",
            BinaryOperatorType::Mod => "%",
            BinaryOperatorType::Match => "~",
            BinaryOperatorType::Range => "..:",
            _ => {
                return Err(format!(
                    "Unsupported binary operator type: {:?}",
                    op.operator
                ));
            }
        };

        bytecode.push(Instruction::Send(selector.to_string(), 1));
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
                bytecode.push(Instruction::Send("!".to_string(), 0));
            }
            UnaryOperatorType::Sub => {
                bytecode.push(Instruction::Send("negated".to_string(), 0));
            }
            UnaryOperatorType::Add => {} // Unary + is a no-op
            UnaryOperatorType::Mod => {
                bytecode.push(Instruction::Send("mod".to_string(), 0));
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
        let mut param_names = Vec::new();
        let mut param_types = Vec::new();
        let mut locals = HashSet::new();

        for arg in &block.arguments {
            let name = arg.identifier.name.clone();
            param_names.push(name.clone());
            let type_name = arg.type_hint.as_ref().map(|id| id.name.clone());
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

        let mut block_bytecode = CodeBlock::new();
        block_bytecode.current_source = block.source_info.clone();

        for name in &decls_names {
            block_bytecode.push(Instruction::Push(Constant::Nil));
            block_bytecode.push(Instruction::DefineLocal(name.clone()));
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

        let static_block = StaticBlock {
            name: block_name,
            is_nested_block: true,
            param_names,
            param_types,
            bytecode: SharedBytecode(Rc::new(block_bytecode.bytecode)),
            source_info: block.source_info.clone(),
            decl_block,
            source_map: SharedSourceMap(Rc::new(block_bytecode.source_map)),
        };

        bytecode.push(Instruction::Push(Constant::Block(static_block)));
        Ok(())
    }

    fn reconstruct_selector(&self, sig: &MethodSelectorNode) -> Result<String, String> {
        if sig.identifiers.is_empty() {
            return Err("No identifiers found in method selector".to_string());
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
    use crate::parser::parse_building_blocks_string;
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

    fn assign_node(lvals: Vec<Node>, rval: Node) -> Node {
        Node {
            source_info: None,
            value: NodeValue::Assignment(AssignmentNode {
                lvalues: lvals.into_iter().map(Arc::new).collect(),
                rvalue: Arc::new(rval),
            }),
        }
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
            Instruction::DefineLocal("self".to_string()),
        ]
    }

    #[test]
    fn test_compile_literals() {
        let res = compile(vec![int(123)]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(123)));
        assert_eq!(res.bytecode, expected);

        let res = compile(vec![double(1.5)]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Double(1.5)));
        assert_eq!(res.bytecode, expected);

        let res = compile(vec![string("hello")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("hello".to_string())));
        assert_eq!(res.bytecode, expected);

        let res = compile(vec![sym("mysym")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("mysym".to_string())));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_identifiers() {
        let res = compile(vec![local_id("nil")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Nil));
        assert_eq!(res.bytecode, expected);

        let res = compile(vec![local_id("true")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Bool(true)));
        assert_eq!(res.bytecode, expected);

        let res = compile(vec![local_id("false")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Bool(false)));
        assert_eq!(res.bytecode, expected);

        // self is always local
        let res = compile(vec![local_id("self")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadLocal("self".to_string()));
        assert_eq!(res.bytecode, expected);

        // unknown name defaults to LoadGlobal
        let res = compile(vec![local_id("my_var")]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("my_var")));
        assert_eq!(res.bytecode, expected);
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
        expected.push(Instruction::DefineLocal("x".to_string()));
        assert_eq!(res.bytecode, expected);

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
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("a".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("b".to_string()));
        assert_eq!(res.bytecode, expected);

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
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("sliceFrom:".to_string(), 1));
        expected.push(Instruction::DefineLocal("rest".to_string()));
        assert_eq!(res.bytecode, expected);

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
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        assert_eq!(res.bytecode, expected);

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
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("a".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("__bb_temp_2".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_2".to_string()));
        expected.push(Instruction::Push(Constant::Int(0)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("b".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_2".to_string()));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("at:".to_string(), 1));
        expected.push(Instruction::DefineLocal("c".to_string()));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_method_calls() {
        // x.foo: 1
        let res = compile(vec![call(Some(local_id("x")), "foo", vec![int(1)])]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("foo:".to_string(), 1));
        assert_eq!(res.bytecode, expected);

        // Implicit subject (self): .foo
        let res = compile(vec![call(None, "foo", vec![])]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadLocal("self".to_string()));
        expected.push(Instruction::Send("foo".to_string(), 0));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_binary_unary_operators() {
        // 1 + 2
        let res = compile(vec![binary(BinaryOperatorType::Add, int(1), int(2))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Push(Constant::Int(2)));
        expected.push(Instruction::Send("+".to_string(), 1));
        assert_eq!(res.bytecode, expected);

        // -x
        let res = compile(vec![unary(UnaryOperatorType::Sub, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Send("negated".to_string(), 0));
        assert_eq!(res.bytecode, expected);

        // !x
        let res = compile(vec![unary(UnaryOperatorType::Bang, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        expected.push(Instruction::Send("!".to_string(), 0));
        assert_eq!(res.bytecode, expected);

        // +x (no-op)
        let res = compile(vec![unary(UnaryOperatorType::Add, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal(ns("x")));
        assert_eq!(res.bytecode, expected);

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
        assert_eq!(res.bytecode, expected);

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
        assert_eq!(res.bytecode, expected);
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

        let inner_static = StaticBlock {
            name: None,
            is_nested_block: true,
            param_names: vec!["x".to_string()],
            param_types: vec![None],
            bytecode: SharedBytecode(Rc::new(vec![
                Instruction::LoadLocal("x".to_string()),
                Instruction::Push(Constant::Int(1)),
                Instruction::Send("+".to_string(), 1),
                Instruction::Return,
            ])),
            source_info: None,
            decl_block: None,
            source_map: SharedSourceMap(Rc::new(vec![None; 4])),
        };
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Block(inner_static)));
        assert_eq!(res.bytecode, expected);
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
        assert_eq!(res.bytecode, expected);

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
        assert_eq!(res.bytecode, expected);

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
        assert_eq!(res.bytecode, expected);
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
            param_names: vec!["a".to_string(), "b".to_string()],
            param_types: vec![None, None],
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
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_source_info_propagation() {
        let code = "{ 1 + 2 };";
        let ast = parse_building_blocks_string(code);
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
}
