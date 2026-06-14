use std::collections::HashSet;
use std::sync::Arc;
use crate::instruction::{Instruction, Constant, StaticBlock};
use crate::parser::ast_visitor::{
    Node, NodeValue, ProgramNode, BlockNode, AssignmentNode, MethodCallNode,
    BinaryOperatorNode, UnaryOperatorNode, BinaryOperatorType, UnaryOperatorType,
    IdentifierType
};

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
            scopes: vec![Scope { locals: HashSet::new() }],
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
        let mut bytecode = Vec::new();
        
        // Define default top-level self = nil
        bytecode.push(Instruction::Push(Constant::Nil));
        bytecode.push(Instruction::DefineLocal("self".to_string()));
        self.scopes[0].locals.insert("self".to_string());

        let len = program.expressions.len();
        for (idx, expr) in program.expressions.iter().enumerate() {
            self.compile_node(expr, &mut bytecode)?;
            if idx < len - 1 {
                bytecode.push(Instruction::Pop);
            }
        }

        if len == 0 {
            bytecode.push(Instruction::Push(Constant::Nil));
        }

        Ok(StaticBlock {
            name: Some("main".to_string()),
            is_nested_block: false,
            param_names: Vec::new(),
            bytecode,
        })
    }

    fn compile_node(&mut self, node: &Node, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
        match &node.value {
            NodeValue::Integer(n) => {
                bytecode.push(Instruction::Push(Constant::Int(n.value)));
            }
            NodeValue::Double(d) => {
                bytecode.push(Instruction::Push(Constant::Float(d.value)));
            }
            NodeValue::Str(s) => {
                bytecode.push(Instruction::Push(Constant::String(s.value.clone())));
            }
            NodeValue::Symbol(s) => {
                bytecode.push(Instruction::Push(Constant::String(s.value.clone())));
            }
            NodeValue::Identifier(id) => {
                if id.name == "nil" || id.name == "true" || id.name == "false" {
                    match id.name.as_str() {
                        "nil" => bytecode.push(Instruction::Push(Constant::Nil)),
                        "true" => bytecode.push(Instruction::Push(Constant::Bool(true))),
                        "false" => bytecode.push(Instruction::Push(Constant::Bool(false))),
                        _ => unreachable!(),
                    }
                } else if self.is_local(&id.name) {
                    bytecode.push(Instruction::LoadLocal(id.name.clone()));
                } else {
                    bytecode.push(Instruction::LoadGlobal(id.name.clone()));
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
            NodeValue::List(list) => {
                for item in &list.values {
                    self.compile_node(item, bytecode)?;
                }
                bytecode.push(Instruction::NewList(list.values.len()));
            }
            NodeValue::Dictionary(dict) => {
                if dict.keys.len() != dict.values.len() {
                    return Err("Dictionary keys and values count mismatch".to_string());
                }
                for i in 0..dict.keys.len() {
                    self.compile_node(&dict.keys[i], bytecode)?;
                    self.compile_node(&dict.values[i], bytecode)?;
                }
                bytecode.push(Instruction::NewDict(dict.keys.len()));
            }
            NodeValue::Regex(re) => {
                let mut pattern = re.value.clone();
                if pattern.starts_with("#/") && pattern.ends_with('/') {
                    pattern = pattern[2..pattern.len() - 1].to_string();
                }
                bytecode.push(Instruction::Push(Constant::String(pattern)));
                bytecode.push(Instruction::NewRegex);
            }
            NodeValue::Unknown => {
                return Err("Encountered Unknown NodeValue".to_string());
            }
            _ => {
                // Fallback for currently unsupported or skipped nodes
                bytecode.push(Instruction::Push(Constant::Nil));
            }
        }
        Ok(())
    }

    fn compile_assignment(&mut self, assign: &AssignmentNode, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
        if assign.lvalues.is_empty() {
            return Err("Assignment requires at least one target lvalue".to_string());
        }

        self.compile_node(&assign.rvalue, bytecode)?;

        if assign.lvalues.len() == 1 {
            let lval = &assign.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_store(lval, bytecode)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes.last_mut().unwrap().locals.insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(temp_var.clone()));

            self.compile_destruct(&assign.lvalues, &temp_var, bytecode)?;
        }

        Ok(())
    }

    fn compile_store(&mut self, lval: &Node, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
        match &lval.value {
            NodeValue::IdentLValue(ident_lval) => {
                let name = &ident_lval.identifier.name;
                if self.is_local(name) {
                    bytecode.push(Instruction::StoreLocal(name.clone()));
                } else {
                    bytecode.push(Instruction::StoreGlobal(name.clone()));
                }
            }
            _ => return Err(format!("Unsupported store target: {:?}", lval.value)),
        }
        Ok(())
    }

    fn compile_destruct(&mut self, lvalues: &[Arc<Node>], temp_var: &str, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
        for (i, lval) in lvalues.iter().enumerate() {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let name = &ident_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("at:".to_string(), 1));
                    
                    if self.is_local(name) {
                        bytecode.push(Instruction::StoreLocal(name.clone()));
                    } else {
                        bytecode.push(Instruction::StoreGlobal(name.clone()));
                    }
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let name = &splat_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("sliceFrom:".to_string(), 1));
                    
                    if self.is_local(name) {
                        bytecode.push(Instruction::StoreLocal(name.clone()));
                    } else {
                        bytecode.push(Instruction::StoreGlobal(name.clone()));
                    }
                }
                NodeValue::IgnoredLValue => {}
                NodeValue::IgnoredSplatLValue => {}
                NodeValue::SubLValue(sub_lval) => {
                    let nested_temp = self.new_temp_var();
                    self.scopes.last_mut().unwrap().locals.insert(nested_temp.clone());
                    
                    bytecode.push(Instruction::LoadLocal(temp_var.to_string()));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send("at:".to_string(), 1));
                    bytecode.push(Instruction::DefineLocal(nested_temp.clone()));
                    
                    self.compile_destruct(&sub_lval.lvalues, &nested_temp, bytecode)?;
                }
                _ => return Err(format!("Unsupported destructuring element: {:?}", lval.value)),
            }
        }
        Ok(())
    }

    fn compile_method_call(&mut self, call: &MethodCallNode, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
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

    fn compile_binary_operator(&mut self, op: &BinaryOperatorNode, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
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
            BinaryOperatorType::And => "&&",
            BinaryOperatorType::Or => "||",
            _ => return Err(format!("Unsupported binary operator type: {:?}", op.operator)),
        };

        bytecode.push(Instruction::Send(selector.to_string(), 1));
        Ok(())
    }

    fn compile_unary_operator(&mut self, op: &UnaryOperatorNode, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
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
            _ => return Err(format!("Unsupported unary operator type: {:?}", op.operator)),
        }
        Ok(())
    }

    fn compile_block(&mut self, block: &BlockNode, bytecode: &mut Vec<Instruction>) -> Result<(), String> {
        let mut param_names = Vec::new();
        let mut locals = HashSet::new();

        for arg in &block.arguments {
            let name = arg.identifier.name.clone();
            param_names.push(name.clone());
            locals.insert(name);
        }

        let mut decls_names = Vec::new();
        for decl in &block.decls {
            let name = decl.identifier.name.clone();
            decls_names.push(name.clone());
            locals.insert(name);
        }

        self.push_scope(locals);

        let mut block_bytecode = Vec::new();

        for name in &decls_names {
            block_bytecode.push(Instruction::Push(Constant::Nil));
            block_bytecode.push(Instruction::DefineLocal(name.clone()));
        }

        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            self.compile_node(stmt, &mut block_bytecode)?;
            if idx < len - 1 {
                block_bytecode.push(Instruction::Pop);
            }
        }

        if len == 0 {
            block_bytecode.push(Instruction::Push(Constant::Nil));
        }

        block_bytecode.push(Instruction::Return);
        self.pop_scope();

        let block_name = block.name.as_ref().map(|s| s.value.clone());

        let static_block = StaticBlock {
            name: block_name,
            is_nested_block: true,
            param_names,
            bytecode: block_bytecode,
        };

        bytecode.push(Instruction::Push(Constant::Block(static_block)));
        Ok(())
    }
}
