//! `[Lang]Parser` + `[Lang]Node` — the parser and a walkable AST as Quoin
//! objects, the metaprogramming substrate (lint rules, codemods via the
//! span-based `[Lang]Rewrite` in qnlib/lang/ast.qn, macro experiments through
//! `Runtime.eval:`). One node class, not thirty: every node answers `kind`
//! (a Symbol), `file`/`span`/`text` (full source fidelity), `children`, and
//! `at:#field` for kind-specific parts. The `kind_name`/`node_children`/
//! `node_field` matches below are EXHAUSTIVE over `NodeValue`, so adding a
//! parser variant without its AST mapping is a compile error — sync is the
//! compiler's job, not discipline.
//!
//! Nodes wrap the parser's own `Arc<Node>` tree (shared, never copied deeply)
//! plus the source text, so `text` is an exact slice and spans never lie.

use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use quoin_syntax::ast::{
    BinaryOperatorType, BlockNode, DeclKind, IdentifierNode, MethodSelectorNode, Node, NodeValue,
    TypeRefNode, UnaryOperatorType,
};
use std::any::Any;
use std::sync::Arc;

pub struct NativeAstNode {
    node: Arc<Node>,
    /// The whole unit's source — shared by every node of the tree, so `text`
    /// slices exactly and cheaply.
    source: Arc<str>,
}

impl std::fmt::Debug for NativeAstNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Lang]Node({})", kind_name(&self.node.value))
    }
}

impl AnyCollect for NativeAstNode {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {} // Arc-backed, no Gc fields
}

fn make_ast_node<'gc>(
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
    node: Arc<Node>,
    source: Arc<str>,
) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "[Lang]Node");
    vm.new_native_state(mc, class, NativeAstNode { node, source })
}

/// The node's kind, as the Symbol name Quoin sees. Exhaustive on purpose.
fn kind_name(v: &NodeValue) -> &'static str {
    match v {
        NodeValue::Unknown => "unknown",
        NodeValue::Assignment(_) => "assignment",
        NodeValue::Declaration(_) => "declaration",
        NodeValue::Bang3 => "placeholderUnreachable",
        NodeValue::BinaryOperator(_) => "binaryOp",
        NodeValue::Block(_) => "block",
        NodeValue::BlockArg(_) => "blockParameter",
        NodeValue::BlockDecl(_) => "blockLocal",
        NodeValue::BlockIgnoredArgument => "ignoredParameter",
        NodeValue::BlockReturn(_) => "blockReturn",
        NodeValue::ClassDefinition(_) => "classDefinition",
        NodeValue::ClassExtension(_) => "classExtension",
        NodeValue::ConstDefinition(_) => "constDefinition",
        NodeValue::Map(_) => "mapLiteral",
        NodeValue::Dot3 => "placeholderNotImplemented",
        NodeValue::Double(_) => "doubleLiteral",
        NodeValue::Huh3 => "placeholderWarn",
        NodeValue::IdentLValue(_) => "lvalue",
        NodeValue::Identifier(_) => "identifier",
        NodeValue::IgnoredLValue => "ignoredLvalue",
        NodeValue::IgnoredSplatLValue => "ignoredSplatLvalue",
        NodeValue::Integer(_) => "integerLiteral",
        NodeValue::List(_) => "listLiteral",
        NodeValue::MethodCallArguments(_) => "sendArguments",
        NodeValue::MethodCall(_) => "send",
        NodeValue::MethodDefinition(_) => "methodDefinition",
        NodeValue::MethodExtension(_) => "methodExtension",
        NodeValue::MethodReturn(_) => "methodReturn",
        NodeValue::MethodSelector(_) => "selector",
        NodeValue::Namespace(_) => "namespace",
        NodeValue::Program(_) => "program",
        NodeValue::Regex(_) => "regexLiteral",
        NodeValue::Set(_) => "setLiteral",
        NodeValue::SplatLValue(_) => "splatLvalue",
        NodeValue::Str(_) => "stringLiteral",
        NodeValue::SubLValue(_) => "subLvalue",
        NodeValue::Symbol(_) => "symbolLiteral",
        NodeValue::UnaryOperator(_) => "unaryOp",
        NodeValue::UserList(_) => "userList",
        NodeValue::UserString(_) => "userString",
        NodeValue::Use(_) => "use",
        NodeValue::YieldReturn(_) => "yieldReturn",
    }
}

/// Wrap a `BlockNode` (a typed sub-struct, not an `Arc<Node>`) as a node —
/// shallow: its statement Arcs are shared, not copied.
fn block_as_node(b: &Arc<BlockNode>) -> Arc<Node> {
    Arc::new(Node {
        value: NodeValue::Block((**b).clone()),
        source_info: b.source_info.clone(),
    })
}

/// The node's structural children, in source order. Exhaustive; leaves answer
/// empty. Block parameters/locals are FIELDS (`at:#parameters`), not children —
/// children are code.
fn node_children(node: &Node) -> Vec<Arc<Node>> {
    match &node.value {
        NodeValue::Unknown
        | NodeValue::Bang3
        | NodeValue::Dot3
        | NodeValue::Huh3
        | NodeValue::BlockIgnoredArgument
        | NodeValue::IgnoredLValue
        | NodeValue::IgnoredSplatLValue
        | NodeValue::Double(_)
        | NodeValue::Integer(_)
        | NodeValue::Str(_)
        | NodeValue::Symbol(_)
        | NodeValue::Regex(_)
        | NodeValue::UserString(_)
        | NodeValue::Use(_)
        | NodeValue::Identifier(_)
        | NodeValue::Namespace(_)
        | NodeValue::MethodSelector(_)
        | NodeValue::BlockArg(_)
        | NodeValue::BlockDecl(_)
        | NodeValue::IdentLValue(_)
        | NodeValue::SplatLValue(_) => Vec::new(),
        NodeValue::Assignment(a) => {
            let mut v: Vec<_> = a.lvalues.to_vec();
            v.push(a.rvalue.clone());
            v
        }
        NodeValue::Declaration(d) => {
            let mut v: Vec<_> = d.lvalues.to_vec();
            v.push(d.rvalue.clone());
            v
        }
        NodeValue::BinaryOperator(b) => vec![b.left.clone(), b.right.clone()],
        NodeValue::Block(b) => b.statements.to_vec(),
        NodeValue::BlockReturn(r) => vec![r.value.clone()],
        NodeValue::MethodReturn(r) => vec![r.value.clone()],
        NodeValue::YieldReturn(r) => vec![r.value.clone()],
        NodeValue::ClassDefinition(c) => vec![block_as_node(&c.block)],
        NodeValue::ClassExtension(c) => vec![c.expression.clone(), block_as_node(&c.block)],
        NodeValue::ConstDefinition(c) => vec![c.rvalue.clone()],
        NodeValue::Map(m) => {
            // Interleaved k v k v, so a plain walk visits everything in order.
            let mut v = Vec::with_capacity(m.keys.len() * 2);
            for (k, val) in m.keys.iter().zip(m.values.iter()) {
                v.push(k.clone());
                v.push(val.clone());
            }
            v
        }
        NodeValue::List(l) => l.values.to_vec(),
        NodeValue::Set(s) => s.values.to_vec(),
        NodeValue::UserList(u) => u.values.to_vec(),
        NodeValue::MethodCallArguments(a) => a.expressions.to_vec(),
        NodeValue::MethodCall(mc) => {
            let mut v = Vec::new();
            if let Some(s) = &mc.subject {
                v.push(s.clone());
            }
            v.extend(mc.arguments.expressions.iter().cloned());
            v
        }
        NodeValue::MethodDefinition(m) => vec![block_as_node(&m.block)],
        NodeValue::MethodExtension(m) => vec![block_as_node(&m.block)],
        NodeValue::Program(p) => p.expressions.to_vec(),
        NodeValue::SubLValue(s) => s.lvalues.to_vec(),
        NodeValue::UnaryOperator(u) => vec![u.right.clone()],
    }
}

fn binary_op_text(op: &BinaryOperatorType) -> &'static str {
    match op {
        BinaryOperatorType::Unknown => "?",
        BinaryOperatorType::Add => "+",
        BinaryOperatorType::Sub => "-",
        BinaryOperatorType::Mul => "*",
        BinaryOperatorType::Div => "/",
        BinaryOperatorType::And => "&&",
        BinaryOperatorType::Or => "||",
        BinaryOperatorType::Eq => "==",
        BinaryOperatorType::NotEq => "!=",
        BinaryOperatorType::Gt => ">",
        BinaryOperatorType::GtEq => ">=",
        BinaryOperatorType::Lt => "<",
        BinaryOperatorType::LtEq => "<=",
        BinaryOperatorType::Range => "..",
        BinaryOperatorType::Mod => "%",
        BinaryOperatorType::Match => "~",
    }
}

fn unary_op_text(op: &UnaryOperatorType) -> &'static str {
    match op {
        UnaryOperatorType::Unknown => "?",
        UnaryOperatorType::Bang => "!",
        UnaryOperatorType::Add => "+",
        UnaryOperatorType::Sub => "-",
        UnaryOperatorType::Mod => "%",
    }
}

/// Render a type annotation back to its written form: `List(Integer)`,
/// `Block(Integer ^Boolean)`, `Integer?`.
fn render_typeref(t: &TypeRefNode) -> String {
    let mut s = qualified_name(&t.ident);
    if t.parenthesized || !t.args.is_empty() || t.ret.is_some() {
        s.push('(');
        let mut parts: Vec<String> = t.args.iter().map(|a| render_typeref(a)).collect();
        if let Some(r) = &t.ret {
            parts.push(format!("^{}", render_typeref(r)));
        }
        s.push_str(&parts.join(" "));
        s.push(')');
    }
    s
}

/// `[Ns]Name` for a namespaced identifier, plain `Name` otherwise.
fn qualified_name(id: &IdentifierNode) -> String {
    match namespace_of(id) {
        Some(ns) => format!("[{ns}]{}", id.name),
        None => id.name.clone(),
    }
}

fn namespace_of(id: &IdentifierNode) -> Option<String> {
    id.namespace.as_ref().map(|ns| {
        ns.identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join("][")
    })
}

/// Reconstruct a selector from its identifier parts: each part keeps its `:`
/// iff the source has one right after it — the honest way to tell a unary
/// `name` from a one-keyword `name:` without guessing from arity.
fn selector_text(sel: &MethodSelectorNode, source: &str) -> String {
    let mut s = String::new();
    for id in &sel.identifiers {
        s.push_str(&id.name);
        if let Some(si) = &id.source_info
            && source.as_bytes().get(si.start + id.name.len()) == Some(&b':')
        {
            s.push(':');
        }
    }
    s
}

fn new_string_list<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    items: Vec<String>,
) -> Value<'gc> {
    let vals = items.into_iter().map(|s| vm.new_string(mc, s)).collect();
    vm.new_list(mc, vals)
}

fn new_node_list<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    nodes: Vec<Arc<Node>>,
    source: &Arc<str>,
) -> Value<'gc> {
    let vals = nodes
        .into_iter()
        .map(|n| make_ast_node(vm, mc, n, source.clone()))
        .collect();
    vm.new_list(mc, vals)
}

/// A kind-specific field, or `None` for a field this kind doesn't have (Quoin
/// sees nil — map semantics, honest). Exhaustive over `NodeValue`.
fn node_field<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    node: &Node,
    source: &Arc<str>,
    field: &str,
) -> Option<Value<'gc>> {
    match &node.value {
        NodeValue::Unknown
        | NodeValue::Bang3
        | NodeValue::Dot3
        | NodeValue::Huh3
        | NodeValue::BlockIgnoredArgument
        | NodeValue::IgnoredLValue
        | NodeValue::IgnoredSplatLValue
        | NodeValue::Namespace(_)
        | NodeValue::MethodCallArguments(_)
        | NodeValue::Program(_)
        | NodeValue::SubLValue(_) => None,
        NodeValue::Assignment(_) => None,
        NodeValue::Declaration(d) => match field {
            "kind" => Some(vm.new_symbol(
                mc,
                match d.kind {
                    DeclKind::Var => "var".to_string(),
                    DeclKind::Let => "let".to_string(),
                },
            )),
            "type" => d
                .type_hint
                .as_ref()
                .map(|t| render_typeref(t))
                .map(|s| vm.new_string(mc, s)),
            _ => None,
        },
        NodeValue::BinaryOperator(b) => match field {
            "operator" => Some(vm.new_string(mc, binary_op_text(&b.operator).to_string())),
            "left" => Some(make_ast_node(vm, mc, b.left.clone(), source.clone())),
            "right" => Some(make_ast_node(vm, mc, b.right.clone(), source.clone())),
            _ => None,
        },
        NodeValue::UnaryOperator(u) => match field {
            "operator" => Some(vm.new_string(mc, unary_op_text(&u.operator).to_string())),
            "operand" => Some(make_ast_node(vm, mc, u.right.clone(), source.clone())),
            _ => None,
        },
        NodeValue::Block(b) => match field {
            "parameters" => Some(new_string_list(
                vm,
                mc,
                b.arguments
                    .iter()
                    .map(|a| a.identifier.name.clone())
                    .collect(),
            )),
            "parameterTypes" => {
                let vals = b
                    .arguments
                    .iter()
                    .map(|a| match &a.type_hint {
                        Some(t) => {
                            let s = render_typeref(t);
                            vm.new_string(mc, s)
                        }
                        None => vm.new_nil(mc),
                    })
                    .collect();
                Some(vm.new_list(mc, vals))
            }
            "locals" => Some(new_string_list(
                vm,
                mc,
                b.decls.iter().map(|d| d.identifier.name.clone()).collect(),
            )),
            "name" => b.name.as_ref().map(|n| vm.new_symbol(mc, n.value.clone())),
            "returnType" => b
                .return_type
                .as_ref()
                .map(|t| render_typeref(t))
                .map(|s| vm.new_string(mc, s)),
            _ => None,
        },
        NodeValue::BlockArg(a) => match field {
            "name" => Some(vm.new_string(mc, a.identifier.name.clone())),
            "type" => a
                .type_hint
                .as_ref()
                .map(|t| render_typeref(t))
                .map(|s| vm.new_string(mc, s)),
            _ => None,
        },
        NodeValue::BlockDecl(d) => match field {
            "name" => Some(vm.new_string(mc, d.identifier.name.clone())),
            "type" => d
                .type_hint
                .as_ref()
                .map(|t| render_typeref(t))
                .map(|s| vm.new_string(mc, s)),
            _ => None,
        },
        NodeValue::BlockReturn(_) | NodeValue::MethodReturn(_) | NodeValue::YieldReturn(_) => None,
        NodeValue::ClassDefinition(c) => match field {
            "name" => Some(vm.new_string(mc, c.identifier.name.clone())),
            "namespace" => namespace_of(&c.identifier).map(|s| vm.new_string(mc, s)),
            "qualifiedName" => Some(vm.new_string(mc, qualified_name(&c.identifier))),
            "parent" => c
                .parent_identifier
                .as_ref()
                .map(|p| qualified_name(p))
                .map(|s| vm.new_string(mc, s)),
            "typeParameters" => Some(new_string_list(vm, mc, c.type_params.clone())),
            "body" => Some(make_ast_node(
                vm,
                mc,
                block_as_node(&c.block),
                source.clone(),
            )),
            _ => None,
        },
        NodeValue::ClassExtension(c) => match field {
            "target" => Some(make_ast_node(vm, mc, c.expression.clone(), source.clone())),
            "body" => Some(make_ast_node(
                vm,
                mc,
                block_as_node(&c.block),
                source.clone(),
            )),
            _ => None,
        },
        NodeValue::ConstDefinition(c) => match field {
            "name" => Some(vm.new_string(mc, qualified_name(&c.identifier))),
            "value" => Some(make_ast_node(vm, mc, c.rvalue.clone(), source.clone())),
            _ => None,
        },
        NodeValue::Map(m) => match field {
            "keys" => Some(new_node_list(vm, mc, m.keys.to_vec(), source)),
            "values" => Some(new_node_list(vm, mc, m.values.to_vec(), source)),
            _ => None,
        },
        NodeValue::Double(d) => (field == "value").then(|| vm.new_double(mc, d.value)),
        NodeValue::Integer(i) => (field == "value").then(|| vm.new_int(mc, i.value)),
        NodeValue::Str(s) => (field == "value").then(|| vm.new_string(mc, s.value.clone())),
        NodeValue::Symbol(s) => (field == "value").then(|| vm.new_symbol(mc, s.value.clone())),
        NodeValue::Regex(r) => (field == "value").then(|| {
            // The parser keeps the literal's delimiters; the field answers the
            // bare pattern (`text` still has the exact source form).
            let pat = r
                .value
                .strip_prefix("#/")
                .and_then(|v| v.strip_suffix('/'))
                .unwrap_or(&r.value);
            vm.new_string(mc, pat.to_string())
        }),
        NodeValue::IdentLValue(l) => {
            (field == "name").then(|| vm.new_string(mc, l.identifier.name.clone()))
        }
        NodeValue::SplatLValue(l) => {
            (field == "name").then(|| vm.new_string(mc, l.identifier.name.clone()))
        }
        NodeValue::Identifier(id) => match field {
            "name" => Some(vm.new_string(mc, id.name.clone())),
            "namespace" => namespace_of(id).map(|s| vm.new_string(mc, s)),
            "qualifiedName" => Some(vm.new_string(mc, qualified_name(id))),
            _ => None,
        },
        NodeValue::List(_) | NodeValue::Set(_) => None,
        NodeValue::MethodCall(call) => match field {
            "selector" => Some({
                let s = selector_text(&call.arguments.signature, source);
                vm.new_string(mc, s)
            }),
            "receiver" => call
                .subject
                .as_ref()
                .map(|s| make_ast_node(vm, mc, s.clone(), source.clone())),
            "arguments" => Some(new_node_list(
                vm,
                mc,
                call.arguments.expressions.to_vec(),
                source,
            )),
            _ => None,
        },
        NodeValue::MethodDefinition(m) => match field {
            "selector" => Some({
                let s = selector_text(&m.signature, source);
                vm.new_string(mc, s)
            }),
            "body" => Some(make_ast_node(
                vm,
                mc,
                block_as_node(&m.block),
                source.clone(),
            )),
            _ => None,
        },
        NodeValue::MethodExtension(m) => match field {
            "selector" => Some({
                let s = selector_text(&m.signature, source);
                vm.new_string(mc, s)
            }),
            "body" => Some(make_ast_node(
                vm,
                mc,
                block_as_node(&m.block),
                source.clone(),
            )),
            _ => None,
        },
        NodeValue::MethodSelector(sel) => (field == "name").then(|| {
            let s = selector_text(sel, source);
            vm.new_string(mc, s)
        }),
        NodeValue::UserList(u) => {
            (field == "tag").then(|| vm.new_string(mc, u.identifier.name.clone()))
        }
        NodeValue::UserString(u) => match field {
            "tag" => Some(vm.new_string(mc, u.identifier.name.clone())),
            "value" => Some(vm.new_string(mc, u.value.clone())),
            _ => None,
        },
        NodeValue::Use(u) => match field {
            "path" => Some(vm.new_string(mc, u.path.clone())),
            "package" => u.package.as_ref().map(|p| vm.new_string(mc, p.clone())),
            "glob" => Some(vm.new_bool(mc, u.glob)),
            _ => None,
        },
    }
}

pub fn build_lang_parser_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[Lang]Parser", Some("Object"))
        .construct_with("[Lang]Parser is all class methods (use [Lang]Parser.parse:)")
        .class_doc(
            "The Quoin parser, exposed: `parse:` answers the program's `[Lang]Node` tree — \
             the same AST the compiler consumes, wrapped, never copied. Unparseable source \
             throws the same catchable ParseError `Runtime.eval:` throws. `parse:named:` \
             labels the unit, and the name rides every node's `file` (diagnostics built \
             from the tree point somewhere real). See [Lang]Node, and qnlib/lang/ast.qn \
             for the traversal vocabulary and the span-based [Lang]Rewrite.",
        )
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let source = arg!(args, String, 0).to_string();
            parse_to_node(vm, mc, source, "<string>".to_string())
        })
        .doc(
            "Parse Quoin source into its `[Lang]Node` tree (the root is kind #program); \
             unparseable source throws a catchable ParseError. Nodes' `file` reads \
             '<string>' — use `parse:named:` to label the unit.",
        )
        .typed_class_method("parse:named:", &["String", "String"], |vm, mc, _r, args| {
            let source = arg!(args, String, 0).to_string();
            let name = arg!(args, String, 1).to_string();
            parse_to_node(vm, mc, source, name)
        })
        .doc(
            "`parse:`, with the unit named: every node's `file` answers `name`, so \
             tooling built on the tree reports real locations.",
        )
}

fn parse_to_node<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    source: String,
    name: String,
) -> Result<Value<'gc>, QuoinError> {
    match crate::parser::try_parse_quoin_string_named(&source, &name) {
        Ok(node) => Ok(make_ast_node(vm, mc, Arc::new(node), Arc::from(source))),
        Err(e) => Err(QuoinError::ParseError(format!("[Lang]Parser.parse:: {e}"))),
    }
}

pub fn build_lang_node_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[Lang]Node", Some("Object"))
        .construct_with("use [Lang]Parser.parse:")
        .class_doc(
            "One node of a parsed program — every node, one class: `kind` answers WHICH \
             (a Symbol: #send, #classDefinition, #stringLiteral, …), `children` the \
             structural children in source order, and `at:#field` the kind-specific \
             parts (#selector/#receiver/#arguments on a #send; #name/#parent/#body on a \
             #classDefinition; #value on literals; nil for a field the kind doesn't \
             have). Source fidelity is total: `file`, `span` (#( start end line column )), \
             and `text` (the exact source slice) — which is what makes the span-based \
             [Lang]Rewrite (qnlib/lang/ast.qn) safe. Trees are immutable views; transform \
             by rewriting source, then parse again.\n\n\
             ```\n\
             use std:lang/ast\n\
             var ast = [Lang]Parser.parse:'x = 1 + 2'\n\
             (ast.allNodes.select:{ |n| n.kind == #integerLiteral })\n\
                 .collect:{ |n| n.at:#value }     \"* -> #(1 2)\n\
             ```",
        )
        .instance_method("kind", |vm, mc, receiver, _args| {
            let name = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| kind_name(&s.node.value))
                .map_err(QuoinError::Other)?;
            Ok(vm.new_symbol(mc, name.to_string()))
        })
        .doc("The node's kind, as a Symbol — #program, #send, #classDefinition, ….")
        .instance_method("file", |vm, mc, receiver, _args| {
            let file = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| {
                    s.node.source_info.as_ref().map(|si| si.filename.clone())
                })
                .map_err(QuoinError::Other)?;
            Ok(match file {
                Some(f) => vm.new_string(mc, f),
                None => vm.new_nil(mc),
            })
        })
        .doc(
            "The file (or `parse:named:` label) this node came from; nil on the rare \
             node the parser synthesized without a location.",
        )
        .instance_method("span", |vm, mc, receiver, _args| {
            let span = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| {
                    s.node
                        .source_info
                        .as_ref()
                        .map(|si| (si.start, si.end, si.line, si.column))
                })
                .map_err(QuoinError::Other)?;
            Ok(match span {
                Some((start, end, line, column)) => {
                    let vals = vec![
                        vm.new_int(mc, start as i64),
                        vm.new_int(mc, end as i64),
                        vm.new_int(mc, line as i64),
                        vm.new_int(mc, column as i64),
                    ];
                    vm.new_list(mc, vals)
                }
                None => vm.new_nil(mc),
            })
        })
        .doc(
            "#( start end line column ): byte offsets (end exclusive — what \
             [Lang]Rewrite edits by), then the 1-indexed line and 0-indexed column of \
             the start. nil when the node has no recorded location.",
        )
        .instance_method("text", |vm, mc, receiver, _args| {
            let text = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| {
                    s.node
                        .source_info
                        .as_ref()
                        .and_then(|si| s.source.get(si.start..si.end).map(|t| t.to_string()))
                })
                .map_err(QuoinError::Other)?;
            Ok(match text {
                Some(t) => vm.new_string(mc, t),
                None => vm.new_nil(mc),
            })
        })
        .doc("The node's exact source slice — the bytes `span` names; nil without a span.")
        .instance_method("children", |vm, mc, receiver, _args| {
            let (children, source) = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| {
                    (node_children(&s.node), s.source.clone())
                })
                .map_err(QuoinError::Other)?;
            Ok(new_node_list(vm, mc, children, &source))
        })
        .doc(
            "The structural children, in source order (a #block's statements, a #send's \
             receiver then arguments, …). Block parameters are fields, not children — \
             children are code.",
        )
        .instance_method("at:", |vm, mc, receiver, args| {
            let field = match args.first() {
                Some(Value::Object(obj)) => match &obj.borrow().payload {
                    ObjectPayload::Symbol(s) | ObjectPayload::String(s) => Some((**s).clone()),
                    _ => None,
                },
                _ => None,
            };
            let Some(field) = field else {
                return Err(QuoinError::TypeError {
                    expected: "Symbol or String".to_string(),
                    got: args
                        .first()
                        .map(|v| v.type_name().to_string())
                        .unwrap_or_else(|| "None".to_string()),
                    msg: "[Lang]Node.at:: expects a field name (e.g. #selector)".to_string(),
                });
            };
            let (node, source) = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| (s.node.clone(), s.source.clone()))
                .map_err(QuoinError::Other)?;
            match node_field(vm, mc, &node, &source, &field) {
                Some(v) => Ok(v),
                None => Ok(vm.new_nil(mc)),
            }
        })
        .doc(
            "The kind-specific field named by a Symbol: #selector / #receiver / \
             #arguments on a #send; #name / #parent / #body on a #classDefinition; \
             #value on literals; #operator / #left / #right on a #binaryOp; #parameters \
             / #locals / #name on a #block; #path / #glob on a #use — nil for anything \
             the kind doesn't have (map semantics: absence is an answer).",
        )
        .instance_method("s", |vm, mc, receiver, _args| {
            let (kind, loc) = receiver
                .with_native_state::<NativeAstNode, _, _>(|s| {
                    let loc = s
                        .node
                        .source_info
                        .as_ref()
                        .map(|si| format!(" @ {}:{}", si.filename, si.line));
                    (kind_name(&s.node.value), loc)
                })
                .map_err(QuoinError::Other)?;
            Ok(vm.new_string(
                mc,
                format!("[Lang]Node(#{kind}{})", loc.unwrap_or_default()),
            ))
        })
        .doc("A short description: the kind and where it came from.")
}
