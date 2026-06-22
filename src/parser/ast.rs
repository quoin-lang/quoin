use crate::value::SourceInfo;
use std::sync::Arc;
use std::string::String;

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

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryOperatorNode {
    pub operator: BinaryOperatorType,
    pub left: Arc<Node>,
    pub right: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockNode {
    pub arguments: Vec<Arc<BlockArgNode>>,
    pub decls: Vec<Arc<BlockDeclNode>>,
    pub decl_block: Option<Arc<BlockNode>>,
    pub statements: Vec<Arc<Node>>,
    pub name: Option<Arc<SymbolNode>>,
    pub source_info: Option<SourceInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockArgNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<IdentifierNode>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDeclNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<IdentifierNode>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockReturnNode {
    pub value: Arc<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDefinitionNode {
    pub identifier: Arc<IdentifierNode>,
    pub parent_identifier: Option<Arc<IdentifierNode>>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ProgramNode {
    pub expressions: Vec<Arc<Node>>,
    pub source_info: Option<SourceInfo>,
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
                Arc::make_mut(hint).source_info = None;
            }
        }
        for d in &mut self.decls {
            let decl = Arc::make_mut(d);
            Arc::make_mut(&mut decl.identifier).source_info = None;
            if let Some(hint) = &mut decl.type_hint {
                Arc::make_mut(hint).source_info = None;
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
