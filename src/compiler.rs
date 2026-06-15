use crate::instruction::{Constant, Instruction, StaticBlock};
use crate::parser::ast_visitor::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, IdentifierType,
    MethodCallNode, MethodSelectorNode, Node, NodeValue, ProgramNode, UnaryOperatorNode,
    UnaryOperatorType,
};

use std::collections::HashSet;
use std::sync::Arc;

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
            name: None,
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
            NodeValue::ClassDefinition(class_def) => {
                let name = class_def.identifier.name.clone();
                let parent_name = class_def
                    .parent_identifier
                    .as_ref()
                    .map(|id| id.name.clone());
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
                self.compile_node(&const_def.rvalue, bytecode)?;
                bytecode.push(Instruction::Dup);
                bytecode.push(Instruction::StoreGlobal(const_def.identifier.name.clone()));
            }
            NodeValue::UserString(user_str) => {
                bytecode.push(Instruction::LoadGlobal(user_str.identifier.name.clone()));
                bytecode.push(Instruction::Push(Constant::String(user_str.value.clone())));
                bytecode.push(Instruction::Send("newUserString:".to_string(), 1));
            }
            NodeValue::UserList(user_list) => {
                bytecode.push(Instruction::LoadGlobal(user_list.identifier.name.clone()));
                for val in &user_list.values {
                    self.compile_node(val, bytecode)?;
                }
                bytecode.push(Instruction::NewList(user_list.values.len()));
                bytecode.push(Instruction::Send("newUserList:".to_string(), 1));
            }
            NodeValue::Unknown => {
                return Err("Encountered Unknown NodeValue".to_string());
            }
            _ => {
                return Err(format!("Unsupported NodeValue: {:?}", node.value));
            }
        }
        Ok(())
    }

    fn compile_assignment(
        &mut self,
        assign: &AssignmentNode,
        bytecode: &mut Vec<Instruction>,
    ) -> Result<(), String> {
        if assign.lvalues.is_empty() {
            return Err("Assignment requires at least one target lvalue".to_string());
        }

        self.compile_node(&assign.rvalue, bytecode)?;

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
        bytecode: &mut Vec<Instruction>,
    ) -> Result<(), String> {
        match &lval.value {
            NodeValue::IdentLValue(ident_lval) => {
                let name = &ident_lval.identifier.name;
                self.compile_ident_store(&ident_lval.identifier.identifier_type, name, bytecode);
            }
            _ => return Err(format!("Unsupported store target: {:?}", lval.value)),
        }
        Ok(())
    }

    fn compile_ident_store(
        &mut self,
        ident_type: &IdentifierType,
        name: &String,
        bytecode: &mut Vec<Instruction>,
    ) {
        if ident_type == &IdentifierType::Instance {
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
        bytecode: &mut Vec<Instruction>,
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
        bytecode: &mut Vec<Instruction>,
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
        bytecode: &mut Vec<Instruction>,
    ) -> Result<(), String> {
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
            BinaryOperatorType::Mod => "%",
            BinaryOperatorType::Match => "~",
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
        bytecode: &mut Vec<Instruction>,
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

    fn compile_block(
        &mut self,
        block: &BlockNode,
        bytecode: &mut Vec<Instruction>,
    ) -> Result<(), String> {
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
    use crate::parser::ast_visitor::*;
    use std::sync::Arc;

    // Helpers to easily construct Nodes
    fn int(value: i64) -> Node {
        Node {
            value: NodeValue::Integer(IntegerNode { value }),
        }
    }

    fn double(value: f64) -> Node {
        Node {
            value: NodeValue::Double(DoubleNode { value }),
        }
    }

    fn string(value: &str) -> Node {
        Node {
            value: NodeValue::Str(StringNode {
                value: value.to_string(),
            }),
        }
    }

    fn sym(value: &str) -> Node {
        Node {
            value: NodeValue::Symbol(SymbolNode {
                value: value.to_string(),
            }),
        }
    }

    fn local_id(name: &str) -> Node {
        Node {
            value: NodeValue::Identifier(IdentifierNode {
                namespace: None,
                name: name.to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn assign_node(lvals: Vec<Node>, rval: Node) -> Node {
        Node {
            value: NodeValue::Assignment(AssignmentNode {
                lvalues: lvals.into_iter().map(Arc::new).collect(),
                rvalue: Arc::new(rval),
            }),
        }
    }

    fn binary(op: BinaryOperatorType, left: Node, right: Node) -> Node {
        Node {
            value: NodeValue::BinaryOperator(BinaryOperatorNode {
                operator: op,
                left: Arc::new(left),
                right: Arc::new(right),
            }),
        }
    }

    fn unary(op: UnaryOperatorType, right: Node) -> Node {
        Node {
            value: NodeValue::UnaryOperator(UnaryOperatorNode {
                operator: op,
                right: Arc::new(right),
            }),
        }
    }

    fn call(subject: Option<Node>, selector_name: &str, args: Vec<Node>) -> Node {
        Node {
            value: NodeValue::MethodCall(MethodCallNode {
                subject: subject.map(Arc::new),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
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
        };
        compiler.compile_program(&program)
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
        expected.push(Instruction::LoadGlobal("my_var".to_string()));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_assignments() {
        // Single global assignment
        let lval = Node {
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
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
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "a".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_b = Node {
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "b".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let res = compile(vec![assign_node(vec![lval_a, lval_b], local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
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
            value: NodeValue::SplatLValue(SplatLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "rest".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_ignore = Node {
            value: NodeValue::IgnoredLValue,
        };
        let res = compile(vec![assign_node(
            vec![lval_ignore, lval_rest],
            local_id("x"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::LoadLocal("__bb_temp_1".to_string()));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::Send("sliceFrom:".to_string(), 1));
        expected.push(Instruction::DefineLocal("rest".to_string()));
        assert_eq!(res.bytecode, expected);

        // IgnoredSplatLValue: _ *_ = x;
        let lval_ignore = Node {
            value: NodeValue::IgnoredLValue,
        };
        let lval_ignore_splat = Node {
            value: NodeValue::IgnoredSplatLValue,
        };
        let res = compile(vec![assign_node(
            vec![lval_ignore, lval_ignore_splat],
            local_id("x"),
        )])
        .unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
        expected.push(Instruction::Dup);
        expected.push(Instruction::DefineLocal("__bb_temp_1".to_string()));
        assert_eq!(res.bytecode, expected);

        // SubLValue: a (b c) = x;
        let lval_a = Node {
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "a".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_b = Node {
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "b".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_c = Node {
            value: NodeValue::IdentLValue(IdentLValueNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "c".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            }),
        };
        let lval_nested = Node {
            value: NodeValue::SubLValue(SubLValueNode {
                lvalues: vec![Arc::new(lval_b), Arc::new(lval_c)],
            }),
        };
        let res = compile(vec![assign_node(vec![lval_a, lval_nested], local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
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
        expected.push(Instruction::LoadGlobal("x".to_string()));
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
        expected.push(Instruction::LoadGlobal("x".to_string()));
        expected.push(Instruction::Send("negated".to_string(), 0));
        assert_eq!(res.bytecode, expected);

        // !x
        let res = compile(vec![unary(UnaryOperatorType::Bang, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
        expected.push(Instruction::Send("!".to_string(), 0));
        assert_eq!(res.bytecode, expected);

        // +x (no-op)
        let res = compile(vec![unary(UnaryOperatorType::Add, local_id("x"))]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::LoadGlobal("x".to_string()));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_blocks() {
        // { |x| x + 1 }
        let block_node = BlockNode {
            name: None,
            arguments: vec![Arc::new(BlockArgNode {
                identifier: Arc::new(IdentifierNode {
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
            value: NodeValue::Block(block_node),
        }])
        .unwrap();

        let inner_static = StaticBlock {
            name: None,
            is_nested_block: true,
            param_names: vec!["x".to_string()],
            bytecode: vec![
                Instruction::LoadLocal("x".to_string()),
                Instruction::Push(Constant::Int(1)),
                Instruction::Send("+".to_string(), 1),
                Instruction::Return,
            ],
        };
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::Block(inner_static)));
        assert_eq!(res.bytecode, expected);
    }

    #[test]
    fn test_compile_lists_dicts_regex() {
        // #(1 2)
        let list = Node {
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
        let dict = Node {
            value: NodeValue::Dictionary(DictionaryNode {
                keys: vec![Arc::new(string("a"))],
                values: vec![Arc::new(int(1))],
            }),
        };
        let res = compile(vec![dict]).unwrap();
        let mut expected = prefix_ops();
        expected.push(Instruction::Push(Constant::String("a".to_string())));
        expected.push(Instruction::Push(Constant::Int(1)));
        expected.push(Instruction::NewDict(1));
        assert_eq!(res.bytecode, expected);

        // #/^[a-z]+$/
        let regex = Node {
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
            value: NodeValue::Unknown,
        }]);
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "Encountered Unknown NodeValue");

        // Dictionary mismatch keys/values returns error
        let dict_mismatch = Node {
            value: NodeValue::Dictionary(DictionaryNode {
                keys: vec![Arc::new(string("a"))],
                values: vec![],
            }),
        };
        let res = compile(vec![dict_mismatch]);
        assert!(res.is_err());
        assert_eq!(
            res.err().unwrap(),
            "Dictionary keys and values count mismatch"
        );
    }

    #[test]
    fn test_compile_class_and_method_definitions() {
        let block_node = BlockNode {
            arguments: vec![
                Arc::new(BlockArgNode {
                    identifier: Arc::new(IdentifierNode {
                        namespace: None,
                        name: "a".to_string(),
                        identifier_type: IdentifierType::Instance,
                    }),
                    type_hint: None,
                }),
                Arc::new(BlockArgNode {
                    identifier: Arc::new(IdentifierNode {
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
            value: NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "MyClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: Some(Arc::new(IdentifierNode {
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
            bytecode: vec![Instruction::Push(Constant::Nil), Instruction::Return],
        };
        let mut expected = prefix_ops();
        expected.push(Instruction::DefineClass {
            name: "MyClass".to_string(),
            parent_name: Some("Object".to_string()),
            instance_vars: vec!["a".to_string(), "b".to_string()],
        });
        expected.push(Instruction::Push(Constant::Block(expected_block)));
        expected.push(Instruction::ExecuteBlockWithSelf);
        assert_eq!(res.bytecode, expected);
    }
}
