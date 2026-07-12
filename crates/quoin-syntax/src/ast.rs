use crate::source_info::SourceInfo;
use std::string::String;
use std::sync::Arc;

#[derive(Debug, Default, Clone, PartialEq)]
pub enum IdentifierType {
    #[default]
    Unknown,
    Local,
    Instance,
    Namespaced,
    /// A reserved identifier — `true` / `false` / `nil`. (Distinct from a *keyword*
    /// like `use`.)
    ReservedIdentifier,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum UnaryOperatorType {
    #[default]
    Unknown,
    Bang,
    Add,
    Sub,
    Mod,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum BinaryOperatorType {
    #[default]
    Unknown,
    Add,
    Sub,
    Mul,
    Div,
    And,
    Or,
    Eq,
    NotEq,
    Gt,
    GtEq,
    Lt,
    LtEq,
    Range,
    Mod,
    Match,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentNode {
    pub lvalues: Vec<Arc<Node>>,
    pub rvalue: Arc<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeclKind {
    Var,
    Let,
}

/// A `var`/`let` local declaration: `var x = e`, `let x: Integer = e`,
/// `var a b *c = e`. Distinct from `AssignmentNode` — every target is a *fresh*
/// binding (the compiler errors on same-scope redeclaration and on assigning a
/// `let`), which is what retires the old first-assignment-declares rule.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclarationNode {
    pub kind: DeclKind,
    /// Declared targets, same node kinds as `AssignmentNode.lvalues`
    /// (IdentLValue / SplatLValue / IgnoredLValue / IgnoredSplatLValue / SubLValue).
    pub lvalues: Vec<Arc<Node>>,
    /// Type annotation — only for a single ident target (`var x: Integer = …`);
    /// destructuring targets are always untyped.
    pub type_hint: Option<Arc<TypeRefNode>>,
    pub rvalue: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryOperatorNode {
    pub operator: BinaryOperatorType,
    pub left: Arc<Node>,
    pub right: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockNode {
    pub arguments: Vec<Arc<BlockArgNode>>,
    /// Declared return type (`|args ^Integer|`), for return-type-aware devirtualization. On a
    /// method-body block this is the method's return type (`sel -> { |args ^Integer| … }`).
    pub return_type: Option<Arc<TypeRefNode>>,
    pub decls: Vec<Arc<BlockDeclNode>>,
    pub decl_block: Option<Arc<BlockNode>>,
    pub statements: Vec<Arc<Node>>,
    pub name: Option<Arc<SymbolNode>>,
    pub source_info: Option<SourceInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockArgNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<TypeRefNode>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDeclNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<TypeRefNode>>,
}

/// A type-annotation reference: base identifier plus optional generic
/// arguments (`List(Integer)`, `Map(String List(Integer))`). All four
/// annotation positions (`var x: T`, `|x:T|`, block-local `- x:T`, `^T`)
/// carry one of these; class-header type parameters are plain names on
/// `ClassDefinitionNode` instead. See docs/internal/GENERICS_ARCH.md.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRefNode {
    pub ident: Arc<IdentifierNode>,
    pub args: Vec<Arc<TypeRefNode>>,
    /// The `^`-marked return type in a block type's argument list
    /// (`Block(Integer ^Boolean)`). Meaningful only on a `Block` base; the
    /// resolver warns and degrades elsewhere (GENERICS_ARCH.md §11).
    pub ret: Option<Arc<TypeRefNode>>,
    /// Were parens present at all? Distinguishes `Block()` (zero args, `Any`
    /// return) from bare `Block` (fully unconstrained). Always true when
    /// `args`/`ret` are non-empty.
    pub parenthesized: bool,
}

impl TypeRefNode {
    pub fn clear_source_info(&mut self) {
        Arc::make_mut(&mut self.ident).source_info = None;
        for a in &mut self.args {
            Arc::make_mut(a).clear_source_info();
        }
        if let Some(r) = &mut self.ret {
            Arc::make_mut(r).clear_source_info();
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockReturnNode {
    pub value: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDefinitionNode {
    pub identifier: Arc<IdentifierNode>,
    pub parent_identifier: Option<Arc<IdentifierNode>>,
    /// Class/mixin-header type parameters (`Iterate(T U) <- { … }`) — the type
    /// variables the body's method signatures may use. Checker-only; never
    /// reaches the runtime (docs/internal/GENERICS_ARCH.md §4.4).
    pub type_params: Vec<String>,
    pub block: Arc<BlockNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassExtensionNode {
    pub expression: Arc<Node>,
    pub block: Arc<BlockNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDefinitionNode {
    pub identifier: Arc<IdentifierNode>,
    pub rvalue: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapNode {
    pub keys: Vec<Arc<Node>>,
    pub values: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoubleNode {
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdentLValueNode {
    pub identifier: Arc<IdentifierNode>,
}

#[derive(Debug, Clone)]
pub struct IdentifierNode {
    pub namespace: Option<Arc<NamespaceNode>>,
    pub name: String,
    pub identifier_type: IdentifierType,
    pub source_info: Option<SourceInfo>,
}

// source_info is excluded from equality so that AST comparisons (and the
// parser tests, which compare against source-info-free expected trees) match
// regardless of where in the source an identifier appeared.
impl PartialEq for IdentifierNode {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.name == other.name
            && self.identifier_type == other.identifier_type
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegerNode {
    pub value: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListNode {
    pub values: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodCallArgumentsNode {
    pub signature: Arc<MethodSelectorNode>,
    pub expressions: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodCallNode {
    pub subject: Option<Arc<Node>>,
    pub arguments: Arc<MethodCallArgumentsNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodDefinitionNode {
    pub signature: Arc<MethodSelectorNode>,
    /// `sel -> { … }`. The return type (if any) lives on `block.return_type` (`|args ^Ret|`).
    pub block: Arc<BlockNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodExtensionNode {
    pub signature: Arc<MethodSelectorNode>,
    pub block: Arc<BlockNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodReturnNode {
    pub value: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodSelectorNode {
    pub identifiers: Vec<Arc<IdentifierNode>>,
}

#[derive(Debug, Clone)]
pub struct NamespaceNode {
    pub identifiers: Vec<Arc<IdentifierNode>>,
    pub source_info: Option<SourceInfo>,
}

// source_info excluded from equality (see IdentifierNode).
impl PartialEq for NamespaceNode {
    fn eq(&self, other: &Self) -> bool {
        self.identifiers == other.identifiers
    }
}

/// A `"* allow: <kind …>` warning-suppression pragma recovered from raw source
/// (comments are pest trivia, so the parser re-scans for them — `pragmas.rs`).
/// The scanner is deliberately dumb: kind-name validation and the trailing-only
/// rule are the checker's job, so its diagnostics carry proper spans.
#[derive(Debug, Clone, PartialEq)]
pub struct AllowPragma {
    /// 1-indexed line the comment sits on — the line whose warnings it suppresses.
    pub line: usize,
    /// The names after `allow:`, comma/space separated, as written.
    pub kinds: Vec<String>,
    /// Whether code precedes the comment on its line. A pragma must *trail* the
    /// statement it suppresses: on its own line it would be captured as a doc
    /// comment by the `"*`-block adjacency rules (docs.rs §4).
    pub trailing: bool,
    pub span: SourceInfo,
}

#[derive(Debug, Clone)]
pub struct ProgramNode {
    pub expressions: Vec<Arc<Node>>,
    pub source_info: Option<SourceInfo>,
    /// Suppression pragmas for the checker, in source order. Trivia: excluded
    /// from equality like `source_info` on identifier nodes.
    pub allow_pragmas: Vec<AllowPragma>,
}

// allow_pragmas excluded from equality (trivia, see IdentifierNode's source_info).
impl PartialEq for ProgramNode {
    fn eq(&self, other: &Self) -> bool {
        self.expressions == other.expressions && self.source_info == other.source_info
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegexNode {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetNode {
    pub values: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplatLValueNode {
    pub identifier: Arc<IdentifierNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StringNode {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubLValueNode {
    pub lvalues: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolNode {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryOperatorNode {
    pub operator: UnaryOperatorType,
    pub right: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserListNode {
    pub identifier: Arc<IdentifierNode>,
    pub values: Vec<Arc<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserStringNode {
    pub identifier: Arc<IdentifierNode>,
    pub value: String,
}

/// `use (pkg:)? path;` — explicit file loading. `package` is `None` for the stdlib
/// default; `path` is the slash path with `.qn` implied (e.g. `"io/file"`); `glob` is
/// set when the target ended in `/*` (load every `.qn` in that directory).
#[derive(Debug, Clone, PartialEq)]
pub struct UseNode {
    pub package: Option<String>,
    pub path: String,
    pub glob: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct YieldReturnNode {
    pub value: Arc<Node>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub enum NodeValue {
    #[default]
    Unknown,
    Assignment(AssignmentNode),
    Declaration(DeclarationNode),
    Bang3,
    BinaryOperator(BinaryOperatorNode),
    Block(BlockNode),
    BlockArg(BlockArgNode),
    BlockDecl(BlockDeclNode),
    BlockIgnoredArgument,
    BlockReturn(BlockReturnNode),
    ClassDefinition(ClassDefinitionNode),
    ClassExtension(ClassExtensionNode),
    ConstDefinition(ConstDefinitionNode),
    Map(MapNode),
    Dot3,
    Double(DoubleNode),
    Huh3,
    IdentLValue(IdentLValueNode),
    Identifier(IdentifierNode),
    IgnoredLValue,
    IgnoredSplatLValue,
    Integer(IntegerNode),
    List(ListNode),
    MethodCallArguments(MethodCallArgumentsNode),
    MethodCall(MethodCallNode),
    MethodDefinition(MethodDefinitionNode),
    MethodExtension(MethodExtensionNode),
    MethodReturn(MethodReturnNode),
    MethodSelector(MethodSelectorNode),
    Namespace(NamespaceNode),
    Program(ProgramNode),
    Regex(RegexNode),
    Set(SetNode),
    SplatLValue(SplatLValueNode),
    Str(StringNode),
    SubLValue(SubLValueNode),
    Symbol(SymbolNode),
    UnaryOperator(UnaryOperatorNode),
    UserList(UserListNode),
    UserString(UserStringNode),
    Use(UseNode),
    YieldReturn(YieldReturnNode),
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Node {
    pub value: NodeValue,
    pub source_info: Option<SourceInfo>,
}

use NodeValue::*;

impl BlockNode {
    pub fn clear_source_info(&mut self) {
        self.source_info = None;
        for s in &mut self.statements {
            Arc::make_mut(s).clear_source_info();
        }
        for a in &mut self.arguments {
            let arg = Arc::make_mut(a);
            Arc::make_mut(&mut arg.identifier).source_info = None;
            if let Some(hint) = &mut arg.type_hint {
                Arc::make_mut(hint).clear_source_info();
            }
        }
        for d in &mut self.decls {
            let decl = Arc::make_mut(d);
            Arc::make_mut(&mut decl.identifier).source_info = None;
            if let Some(hint) = &mut decl.type_hint {
                Arc::make_mut(hint).clear_source_info();
            }
        }
        if let Some(decl) = &mut self.decl_block {
            Arc::make_mut(decl).clear_source_info();
        }
    }
}

impl Node {
    pub fn clear_source_info(&mut self) {
        self.source_info = None;
        match &mut self.value {
            Assignment(node) => {
                for l in &mut node.lvalues {
                    Arc::make_mut(l).clear_source_info();
                }
                Arc::make_mut(&mut node.rvalue).clear_source_info();
            }
            Declaration(node) => {
                for l in &mut node.lvalues {
                    Arc::make_mut(l).clear_source_info();
                }
                if let Some(hint) = &mut node.type_hint {
                    Arc::make_mut(hint).clear_source_info();
                }
                Arc::make_mut(&mut node.rvalue).clear_source_info();
            }
            BinaryOperator(node) => {
                Arc::make_mut(&mut node.left).clear_source_info();
                Arc::make_mut(&mut node.right).clear_source_info();
            }
            Block(node) => {
                node.clear_source_info();
            }
            BlockReturn(node) => {
                Arc::make_mut(&mut node.value).clear_source_info();
            }
            ClassDefinition(node) => {
                Arc::make_mut(&mut node.block).clear_source_info();
            }
            ClassExtension(node) => {
                Arc::make_mut(&mut node.expression).clear_source_info();
                Arc::make_mut(&mut node.block).clear_source_info();
            }
            ConstDefinition(node) => {
                Arc::make_mut(&mut node.rvalue).clear_source_info();
            }
            Map(node) => {
                for k in &mut node.keys {
                    Arc::make_mut(k).clear_source_info();
                }
                for v in &mut node.values {
                    Arc::make_mut(v).clear_source_info();
                }
            }
            List(node) => {
                for val in &mut node.values {
                    Arc::make_mut(val).clear_source_info();
                }
            }
            MethodCall(node) => {
                if let Some(sub) = &mut node.subject {
                    Arc::make_mut(sub).clear_source_info();
                }
                let args = Arc::make_mut(&mut node.arguments);
                for expr in &mut args.expressions {
                    Arc::make_mut(expr).clear_source_info();
                }
            }
            MethodDefinition(node) => {
                Arc::make_mut(&mut node.block).clear_source_info();
            }
            MethodExtension(node) => {
                Arc::make_mut(&mut node.block).clear_source_info();
            }
            MethodReturn(node) => {
                Arc::make_mut(&mut node.value).clear_source_info();
            }
            Program(node) => {
                node.source_info = None;
                for expr in &mut node.expressions {
                    Arc::make_mut(expr).clear_source_info();
                }
            }
            Set(node) => {
                for val in &mut node.values {
                    Arc::make_mut(val).clear_source_info();
                }
            }
            SubLValue(node) => {
                for l in &mut node.lvalues {
                    Arc::make_mut(l).clear_source_info();
                }
            }
            UnaryOperator(node) => {
                Arc::make_mut(&mut node.right).clear_source_info();
            }
            UserList(node) => {
                for val in &mut node.values {
                    Arc::make_mut(val).clear_source_info();
                }
            }
            YieldReturn(node) => {
                Arc::make_mut(&mut node.value).clear_source_info();
            }
            _ => {}
        }
    }
}
