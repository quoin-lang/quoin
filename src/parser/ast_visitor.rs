#![allow(non_snake_case)]

use std::string::String;
use std::sync::Arc;

use antlr_rust::tree::{ParseTree, ParseTreeVisitorCompat};
use once_cell::sync::Lazy;
use regex::Captures;
use substring::Substring;

use crate::cast_node;
use crate::parser::ast_visitor::NodeValue::{
    BinaryOperator, Block, BlockArg, BlockDecl, BlockIgnoredArgument, BlockReturn, ClassDefinition, ClassExtension,
    ConstDefinition, Dictionary, Double, IdentLValue, Identifier, IgnoredLValue, IgnoredSplatLValue, Integer, List,
    MethodCall, MethodCallArguments, MethodDefinition, MethodExtension, MethodReturn, MethodSelector, Namespace,
    Program, Regex, Set, SplatLValue, Str, SubLValue, Symbol, UnaryOperator, UserString, YieldReturn,
};
use crate::parser::generated::buildingblocksparser::{
    AddExprContext, AndExprContext, ArgIdentContext, ArgIdentContextAttrs, ArgIdentInstContext,
    ArgIdentInstContextAttrs, AssignmentContext, AssignmentContextAttrs, AssignmentStmtContext,
    AssignmentStmtContextAttrs, Bang3StmtContext, BlockArgIgnoredContext, BlockArgTypedContext, BlockArgUntypedContext,
    BlockDeclTypedContext, BlockDeclUntypedContext, BlockDeclsContextAttrs, BlockNoDeclsContext,
    BlockNoDeclsContextAttrs, BlockReturnContext, BlockReturnContextAttrs, BlockWDeclsContext, BlockWDeclsContextAttrs,
    BuildingBlocksParserContextType, CallSigNoArgBangContext, CallSigNoArgBangContextAttrs, CallSigNoArgContext,
    CallSigNoArgContextAttrs, CallSigWArgContext, CallSigWArgContextAttrs, ClassDef2ExprContext,
    ClassDef2ExprContextAttrs, ClassDefExprContext, ClassDefExprContextAttrs, ClassExtExprContext,
    ClassExtExprContextAttrs, ConstDefExprContext, ConstDefExprContextAttrs, DefCallExprContext,
    DefCallExprContextAttrs, DictExprContext, DivExprContext, Dot3StmtContext, EqExprContext, ExprCallExprContext,
    ExprStmtContext, ExprStmtContextAttrs, FullNSContext, FullNSContextAttrs, GtEqExprContext, GtExprContext,
    Huh3StmtContext, IdentKeywordContext, IdentKeywordContextAttrs, IdentLValueContext, IdentLValueContextAttrs,
    IdentOtherContext, IdentOtherContextAttrs, IgnoredLValueContext, IgnoredSplatLValueContext, InstanceIdentContext,
    InstanceIdentContextAttrs, ListExprContext, ListExprContextAttrs, LiteralNumberContext, LiteralStringContext,
    LiteralStringContextAttrs, LiteralSymbolContext, LiteralSymbolContextAttrs, LocalIdentContext,
    LocalIdentContextAttrs, LtEqExprContext, LtExprContext, MatchExprContext, MethodDefExprContext,
    MethodDefExprContextAttrs, MethodExtExprContext, MethodExtExprContextAttrs, MethodReturnContext,
    MethodReturnContextAttrs, ModExprContext, MulExprContext, NamedBlockWDeclsContext, NamedBlockWDeclsContextAttrs,
    NamespacedIdentContext, NamespacedIdentContextAttrs, NestedExprContext, NestedExprContextAttrs, NotEqExprContext,
    OrExprContext, ProgramContext, ProgramContextAttrs, RangeExprContext, RegexExprContext, RegexExprContextAttrs,
    RootNSContext, SelectorNoArgsBangContext, SelectorNoArgsBangContextAttrs, SelectorNoArgsContext,
    SelectorNoArgsContextAttrs, SelectorSymbolContext, SelectorSymbolContextAttrs, SelectorWArgsContext,
    SelectorWArgsContextAttrs, SetExprContext, SetExprContextAttrs, SplatLValueContext, SplatLValueContextAttrs,
    SubExprContext, SubLValueContext, SubLValueContextAttrs, SymbolContext, SymbolContextAttrs, UnBangExprContext,
    UnBangExprContextAttrs, UnMinusExprContext, UnMinusExprContextAttrs, UnModExprContext, UnModExprContextAttrs,
    UnPlusExprContext, UnPlusExprContextAttrs, UserListExprContext, UserStringExprContext, UserStringExprContextAttrs,
    YieldReturnContext, YieldReturnContextAttrs,
};
use crate::parser::generated::buildingblocksvisitor::BuildingBlocksVisitorCompat;

#[derive(Debug, Default, Clone, PartialEq)]
pub enum IdentifierType {
    #[default]
    Unknown,
    Local,
    Instance,
    Namespaced,
    Keyword,
}
#[derive(Debug, Default, Clone)]
pub enum UnaryOperatorType {
    #[default]
    Unknown,
    Bang,
    Add,
    Sub,
    Mod,
}
#[derive(Debug, Default, Clone)]
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

#[derive(Debug, Clone)]
pub struct AssignmentNode {
    pub lvalues: Vec<Arc<Node>>,
    pub rvalue: Arc<Node>,
}
#[derive(Debug, Clone)]
pub struct BinaryOperatorNode {
    pub operator: BinaryOperatorType,
    pub left: Arc<Node>,
    pub right: Arc<Node>,
}

#[derive(Debug, Clone)]
pub struct BlockNode {
    pub arguments: Vec<Arc<BlockArgNode>>,
    pub decls: Vec<Arc<BlockArgNode>>,
    pub decl_block: Option<Arc<BlockNode>>,
    pub statements: Vec<Arc<Node>>,
    pub name: Option<Arc<SymbolNode>>,
}

#[derive(Debug, Clone)]
pub struct BlockArgNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<IdentifierNode>>,
}
#[derive(Debug, Clone)]
pub struct BlockDeclNode {
    pub identifier: Arc<IdentifierNode>,
    pub type_hint: Option<Arc<IdentifierNode>>,
}
#[derive(Debug, Clone)]
pub struct BlockReturnNode {
    pub value: Arc<Node>,
}
#[derive(Debug, Clone)]
pub struct ClassDefinitionNode {
    pub identifier: Arc<IdentifierNode>,
    pub parent_identifier: Option<Arc<IdentifierNode>>,
    pub block: Arc<BlockNode>,
}
#[derive(Debug, Clone)]
pub struct ClassExtensionNode {
    pub expression: Arc<Node>,
    pub block: Arc<BlockNode>,
}
#[derive(Debug, Clone)]
pub struct ConstDefinitionNode {
    pub identifier: Arc<IdentifierNode>,
    pub rvalue: Arc<Node>,
}
#[derive(Debug, Clone)]
pub struct DictionaryNode {
    pub keys: Vec<Arc<Node>>,
    pub values: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct DoubleNode {
    pub value: f64,
}
#[derive(Debug, Clone)]
pub struct IdentLValueNode {
    pub identifier: Arc<IdentifierNode>,
}
#[derive(Debug, Clone)]
pub struct IdentifierNode {
    pub namespace: Option<Arc<NamespaceNode>>,
    pub name: String,
    pub identifier_type: IdentifierType,
}
#[derive(Debug, Clone)]
pub struct IntegerNode {
    pub value: i64,
}
#[derive(Debug, Clone)]
pub struct ListNode {
    pub values: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct MethodCallArgumentsNode {
    pub signature: Arc<MethodSelectorNode>,
    pub expressions: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct MethodCallNode {
    pub subject: Option<Arc<Node>>,
    pub arguments: Arc<MethodCallArgumentsNode>,
}
#[derive(Debug, Clone)]
pub struct MethodDefinitionNode {
    pub signature: Arc<MethodSelectorNode>,
    pub block: Arc<BlockNode>,
}
#[derive(Debug, Clone)]
pub struct MethodExtensionNode {
    pub signature: Arc<MethodSelectorNode>,
    pub block: Arc<BlockNode>,
}
#[derive(Debug, Clone)]
pub struct MethodReturnNode {
    pub value: Arc<Node>,
}
#[derive(Debug, Clone)]
pub struct MethodSelectorNode {
    pub identifiers: Vec<Arc<IdentifierNode>>,
}
#[derive(Debug, Clone)]
pub struct NamespaceNode {
    pub identifiers: Vec<Arc<IdentifierNode>>,
}
#[derive(Debug, Clone)]
pub struct ProgramNode {
    pub expressions: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct RegexNode {
    pub value: String,
}
#[derive(Debug, Clone)]
pub struct SetNode {
    pub values: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct SplatLValueNode {
    pub identifier: Arc<IdentifierNode>,
}
#[derive(Debug, Clone)]
pub struct StringNode {
    pub value: String,
}
#[derive(Debug, Clone)]
pub struct SubLValueNode {
    pub lvalues: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct SymbolNode {
    pub value: String,
}
#[derive(Debug, Clone)]
pub struct UnaryOperatorNode {
    pub operator: UnaryOperatorType,
    pub right: Arc<Node>,
}
#[derive(Debug, Clone)]
pub struct UserListNode {
    pub identifier: Arc<IdentifierNode>,
    pub values: Vec<Arc<Node>>,
}
#[derive(Debug, Clone)]
pub struct UserStringNode {
    pub identifier: Arc<IdentifierNode>,
    pub value: String,
}
#[derive(Debug, Clone)]
pub struct YieldReturnNode {
    pub value: Arc<Node>,
}

#[derive(Default, Debug, Clone)]
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
    Dictionary(DictionaryNode),
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
    YieldReturn(YieldReturnNode),
}

#[derive(Default, Debug, Clone)]
pub struct Node {
    pub value: NodeValue,
}

pub struct AstVisitor {
    pub x: Node,
}

impl<'a> ParseTreeVisitorCompat<'a> for AstVisitor {
    type Node = BuildingBlocksParserContextType;
    type Return = Node;

    fn temp_result(&mut self) -> &mut Self::Return {
        &mut self.x
    }

    fn aggregate_results(&self, _aggregate: Self::Return, _next: Self::Return) -> Self::Return {
        _next
    }
}

impl<'a> BuildingBlocksVisitorCompat<'a> for AstVisitor {
    fn visit_program(&mut self, ctx: &ProgramContext<'a>) -> Self::Return {
        let mut stmts: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            stmts.push(Arc::new(self.visit(&*node)));
        }
        Node { value: Program(ProgramNode { expressions: stmts }) }
    }

    fn visit_MethodReturn(&mut self, ctx: &MethodReturnContext<'a>) -> Self::Return {
        Node {
            value: MethodReturn(MethodReturnNode { value: Arc::new(self.visit(&*ctx.expr().unwrap())) }),
        }
    }

    fn visit_YieldReturn(&mut self, ctx: &YieldReturnContext<'a>) -> Self::Return {
        Node {
            value: YieldReturn(YieldReturnNode { value: Arc::new(self.visit(&*ctx.expr().unwrap())) }),
        }
    }

    fn visit_BlockReturn(&mut self, ctx: &BlockReturnContext<'a>) -> Self::Return {
        Node {
            value: BlockReturn(BlockReturnNode { value: Arc::new(self.visit(&*ctx.expr().unwrap())) }),
        }
    }

    fn visit_AssignmentStmt(&mut self, ctx: &AssignmentStmtContext<'a>) -> Self::Return {
        self.visit(&*ctx.assignment().unwrap())
    }

    fn visit_Bang3Stmt(&mut self, _ctx: &Bang3StmtContext<'a>) -> Self::Return {
        Node { value: NodeValue::Bang3 }
    }

    fn visit_Dot3Stmt(&mut self, _ctx: &Dot3StmtContext<'a>) -> Self::Return {
        Node { value: NodeValue::Dot3 }
    }

    fn visit_Huh3Stmt(&mut self, _ctx: &Huh3StmtContext<'a>) -> Self::Return {
        Node { value: NodeValue::Huh3 }
    }

    fn visit_ExprStmt(&mut self, ctx: &ExprStmtContext<'a>) -> Self::Return {
        self.visit(&*ctx.expr().unwrap())
    }

    fn visit_SelectorWArgs(&mut self, ctx: &SelectorWArgsContext<'a>) -> Self::Return {
        let mut idents: Vec<Arc<IdentifierNode>> = Vec::new();
        for node in ctx.ident_all() {
            idents.push(Arc::new(cast_node!(Identifier(id), id, self.visit(&*node))));
        }
        Node {
            value: MethodSelector(MethodSelectorNode { identifiers: idents }),
        }
    }

    fn visit_SelectorNoArgs(&mut self, ctx: &SelectorNoArgsContext<'a>) -> Self::Return {
        Node {
            value: MethodSelector(MethodSelectorNode {
                identifiers: vec![Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.ident().unwrap())))],
            }),
        }
    }

    fn visit_SelectorNoArgsBang(&mut self, ctx: &SelectorNoArgsBangContext<'a>) -> Self::Return {
        let node = self.visit(&*ctx.ident().unwrap());
        let ident = Self::add_bang_to_ident(cast_node!(Identifier(id), id, node));
        Node {
            value: MethodSelector(MethodSelectorNode { identifiers: vec![Arc::new(ident)] }),
        }
    }

    fn visit_SelectorSymbol(&mut self, ctx: &SelectorSymbolContext<'a>) -> Self::Return {
        let binding = ctx.symbol().unwrap().get_text();
        let selectorText = binding.trim_start_matches('#').trim_matches('\'').trim_end_matches(':');
        Node {
            value: MethodSelector(MethodSelectorNode {
                identifiers: vec![Arc::new(IdentifierNode {
                    namespace: None,
                    name: String::from(selectorText),
                    identifier_type: IdentifierType::Local,
                })],
            }),
        }
    }

    fn visit_assignment(&mut self, ctx: &AssignmentContext<'a>) -> Self::Return {
        let mut nodes: Vec<Arc<Node>> = Vec::new();
        for subctx in ctx.lvalue_all() {
            let n = self.visit(&*subctx);
            nodes.push(Arc::new(n));
        }

        Node {
            value: NodeValue::Assignment(AssignmentNode {
                lvalues: nodes,
                rvalue: Arc::new(self.visit(&*ctx.expr().unwrap())),
            }),
        }
    }

    fn visit_IdentLValue(&mut self, ctx: &IdentLValueContext<'a>) -> Self::Return {
        Node {
            value: IdentLValue(IdentLValueNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.nsvarident().unwrap()))),
            }),
        }
    }

    fn visit_SplatLValue(&mut self, ctx: &SplatLValueContext<'a>) -> Self::Return {
        Node {
            value: SplatLValue(SplatLValueNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.nsvarident().unwrap()))),
            }),
        }
    }

    fn visit_IgnoredLValue(&mut self, _ctx: &IgnoredLValueContext<'a>) -> Self::Return {
        Node { value: IgnoredLValue }
    }

    fn visit_IgnoredSplatLValue(&mut self, _ctx: &IgnoredSplatLValueContext<'a>) -> Self::Return {
        Node { value: IgnoredSplatLValue }
    }

    fn visit_SubLValue(&mut self, ctx: &SubLValueContext<'a>) -> Self::Return {
        let mut lvalues: Vec<Arc<Node>> = Vec::new();
        for node in ctx.lvalue_all() {
            lvalues.push(Arc::new(self.visit(&*node)));
        }
        Node { value: SubLValue(SubLValueNode { lvalues }) }
    }

    fn visit_MulExpr(&mut self, ctx: &MulExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Mul,
                left: Arc::new(self.visit(&*ctx.left.clone().unwrap())),
                right: Arc::new(self.visit(&*ctx.right.clone().unwrap())),
            }),
        }
    }

    fn visit_AndExpr(&mut self, ctx: &AndExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::And,
                left: Arc::new(self.visit(&*ctx.left.clone().unwrap())),
                right: Arc::new(self.visit(&*ctx.right.clone().unwrap())),
            }),
        }
    }

    fn visit_LiteralString(&mut self, ctx: &LiteralStringContext<'a>) -> Self::Return {
        let raw_string = ctx.string().unwrap().get_text().to_string();
        let inner_string = raw_string.substring(1, raw_string.len() - 1).to_string();
        let unescaped_string = Self::unescape(inner_string);
        Node { value: Str(StringNode { value: unescaped_string }) }
    }

    fn visit_UserStringExpr(&mut self, ctx: &UserStringExprContext<'a>) -> Self::Return {
        // #Ident'......'
        let raw_string = ctx.userString().unwrap().get_text();

        let string_start = raw_string
            .find('\'')
            .unwrap_or_else(|| panic!("Invalid user string: {}", raw_string));
        let ident_string = raw_string.substring(1, string_start);
        let string_string = raw_string.substring(string_start + 1, raw_string.len() - 1).to_string();
        let unescaped_string = Self::unescape(string_string.clone());

        Node {
            value: UserString(UserStringNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: ident_string.to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                value: unescaped_string.to_string(),
            }),
        }
    }

    fn visit_RegexExpr(&mut self, ctx: &RegexExprContext<'a>) -> Self::Return {
        Node {
            value: Regex(RegexNode { value: ctx.REGEXP().unwrap().get_text() }),
        }
    }

    fn visit_GtExpr(&mut self, ctx: &GtExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Gt,
                left: Arc::new(self.visit(&*ctx.left.clone().unwrap())),
                right: Arc::new(self.visit(&*ctx.right.clone().unwrap())),
            }),
        }
    }

    fn visit_LtExpr(&mut self, ctx: &LtExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Lt,
                left: Arc::new(self.visit(&*ctx.left.clone().unwrap())),
                right: Arc::new(self.visit(&*ctx.right.clone().unwrap())),
            }),
        }
    }

    fn visit_UserListExpr(&mut self, _ctx: &UserListExprContext<'a>) -> Self::Return {
        todo!()
    }

    fn visit_LtEqExpr(&mut self, ctx: &LtEqExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::LtEq,
                left: Arc::new(self.visit(&*ctx.left.clone().unwrap())),
                right: Arc::new(self.visit(&*ctx.right.clone().unwrap())),
            }),
        }
    }

    fn visit_MethodDefExpr(&mut self, ctx: &MethodDefExprContext<'a>) -> Self::Return {
        Node {
            value: MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(cast_node!(MethodSelector(ms), ms, self.visit(&*ctx.selector().unwrap()))),
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_LiteralSymbol(&mut self, ctx: &LiteralSymbolContext<'a>) -> Self::Return {
        let binding = ctx.symbol().unwrap().get_text();
        let symbolText = binding.trim_start_matches('#').trim_matches('\'');
        Node {
            value: Symbol(SymbolNode { value: String::from(symbolText) }),
        }
    }

    fn visit_ClassDefExpr(&mut self, ctx: &ClassDefExprContext<'a>) -> Self::Return {
        Node {
            value: ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                parent_identifier: None,
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_ExprCallExpr(&mut self, ctx: &ExprCallExprContext<'a>) -> Self::Return {
        Node {
            value: MethodCall(MethodCallNode {
                subject: Some(Arc::new(self.visit(&*ctx.subject.clone().unwrap()))),
                arguments: Arc::new(cast_node!(
                    MethodCallArguments(args),
                    args,
                    self.visit(&*ctx.sig.clone().unwrap())
                )),
            }),
        }
    }

    fn visit_SetExpr(&mut self, ctx: &SetExprContext<'a>) -> Self::Return {
        let mut exprs: Vec<Arc<Node>> = Vec::new();
        for node in ctx.expr_all() {
            exprs.push(Arc::new(self.visit(&*node)));
        }

        Node { value: Set(SetNode { values: exprs }) }
    }

    fn visit_UnModExpr(&mut self, ctx: &UnModExprContext<'a>) -> Self::Return {
        Node {
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Mod,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_MethodExtExpr(&mut self, ctx: &MethodExtExprContext<'a>) -> Self::Return {
        Node {
            value: MethodExtension(MethodExtensionNode {
                signature: Arc::new(cast_node!(MethodSelector(ms), ms, self.visit(&*ctx.selector().unwrap()))),
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_DictExpr(&mut self, ctx: &DictExprContext<'a>) -> Self::Return {
        let mut keys: Vec<Arc<Node>> = Vec::new();
        for node in ctx.k.clone() {
            keys.push(Arc::new(self.visit(&*node)));
        }

        let mut values: Vec<Arc<Node>> = Vec::new();
        for node in ctx.v.clone() {
            values.push(Arc::new(self.visit(&*node)));
        }

        Node { value: Dictionary(DictionaryNode { keys, values }) }
    }

    fn visit_ListExpr(&mut self, ctx: &ListExprContext<'a>) -> Self::Return {
        let mut exprs: Vec<Arc<Node>> = Vec::new();
        for node in ctx.expr_all() {
            exprs.push(Arc::new(self.visit(&*node)));
        }

        Node { value: List(ListNode { values: exprs }) }
    }

    fn visit_SubExpr(&mut self, ctx: &SubExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Sub,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_AddExpr(&mut self, ctx: &AddExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Add,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_ConstDefExpr(&mut self, ctx: &ConstDefExprContext<'a>) -> Self::Return {
        Node {
            value: ConstDefinition(ConstDefinitionNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.nsvarident().unwrap()))),
                rvalue: Arc::new(self.visit(ctx.expr().clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_RangeExpr(&mut self, ctx: &RangeExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Range,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_UnPlusExpr(&mut self, ctx: &UnPlusExprContext<'a>) -> Self::Return {
        Node {
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Add,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_OrExpr(&mut self, ctx: &OrExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Or,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_ClassDef2Expr(&mut self, ctx: &ClassDef2ExprContext<'a>) -> Self::Return {
        Node {
            value: ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                parent_identifier: Some(Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.parent.clone().unwrap())
                ))),
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_GtEqExpr(&mut self, ctx: &GtEqExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::GtEq,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_DivExpr(&mut self, ctx: &DivExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Div,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_UnBangExpr(&mut self, ctx: &UnBangExprContext<'a>) -> Self::Return {
        Node {
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Bang,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_NotEqExpr(&mut self, ctx: &NotEqExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::NotEq,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_UnMinusExpr(&mut self, ctx: &UnMinusExprContext<'a>) -> Self::Return {
        Node {
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Sub,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_EqExpr(&mut self, ctx: &EqExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Eq,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_ClassExtExpr(&mut self, ctx: &ClassExtExprContext<'a>) -> Self::Return {
        Node {
            value: ClassExtension(ClassExtensionNode {
                expression: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_NestedExpr(&mut self, ctx: &NestedExprContext<'a>) -> Self::Return {
        self.visit(&*ctx.expr().unwrap())
    }

    fn visit_ModExpr(&mut self, ctx: &ModExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Mod,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_MatchExpr(&mut self, ctx: &MatchExprContext<'a>) -> Self::Return {
        Node {
            value: BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Match,
                left: Arc::new(self.visit(ctx.left.clone().unwrap().as_ref())),
                right: Arc::new(self.visit(ctx.right.clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_DefCallExpr(&mut self, ctx: &DefCallExprContext<'a>) -> Self::Return {
        Node {
            value: MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(cast_node!(
                    MethodCallArguments(args),
                    args,
                    self.visit(&*ctx.callSig().unwrap())
                )),
            }),
        }
    }

    fn visit_LiteralNumber(&mut self, ctx: &LiteralNumberContext<'a>) -> Self::Return {
        let numtext = ctx.get_text();

        let nodeValue = if numtext.contains('.') {
            Double(DoubleNode { value: numtext.parse::<f64>().unwrap() })
        } else {
            Integer(IntegerNode { value: numtext.parse::<i64>().unwrap() })
        };

        Node { value: nodeValue }
    }

    fn visit_CallSigWArg(&mut self, ctx: &CallSigWArgContext<'a>) -> Self::Return {
        let mut idents: Vec<Arc<IdentifierNode>> = Vec::new();
        for node in ctx.ident_all() {
            idents.push(Arc::new(cast_node!(Identifier(id), id, self.visit(&*node))));
        }

        let mut exprs: Vec<Arc<Node>> = Vec::new();
        for node in ctx.expr_all() {
            exprs.push(Arc::new(self.visit(&*node)));
        }

        Node {
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode { identifiers: idents }),
                expressions: exprs,
            }),
        }
    }

    fn visit_CallSigNoArg(&mut self, ctx: &CallSigNoArgContext<'a>) -> Self::Return {
        let ident = cast_node!(Identifier(id), id, self.visit(&*ctx.ident().unwrap()));
        Node {
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode { identifiers: vec![Arc::new(ident)] }),
                expressions: vec![],
            }),
        }
    }

    fn visit_CallSigNoArgBang(&mut self, ctx: &CallSigNoArgBangContext<'a>) -> Self::Return {
        let ident = cast_node!(Identifier(id), id, self.visit(&*ctx.ident().unwrap()));
        let ident = Self::add_bang_to_ident(ident);
        Node {
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode { identifiers: vec![Arc::new(ident)] }),
                expressions: vec![],
            }),
        }
    }

    fn visit_NamespacedIdent(&mut self, ctx: &NamespacedIdentContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: Some(Arc::new(cast_node!(Namespace(ns), ns, self.visit(&*ctx.namespace().unwrap())))),
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Namespaced,
            }),
        }
    }

    fn visit_InstanceIdent(&mut self, ctx: &InstanceIdentContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Instance,
            }),
        }
    }

    fn visit_LocalIdent(&mut self, ctx: &LocalIdentContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn visit_FullNS(&mut self, ctx: &FullNSContext<'a>) -> Self::Return {
        let mut idents: Vec<Arc<IdentifierNode>> = Vec::new();
        for node in ctx.ident_all() {
            idents.push(Arc::new(cast_node!(Identifier(id), id, self.visit(&*node))));
        }

        Node { value: Namespace(NamespaceNode { identifiers: idents }) }
    }

    fn visit_RootNS(&mut self, _ctx: &RootNSContext<'a>) -> Self::Return {
        Node { value: Namespace(NamespaceNode { identifiers: vec![] }) }
    }

    fn visit_NamedBlockWDecls(&mut self, ctx: &NamedBlockWDeclsContext<'a>) -> Self::Return {
        let mut arguments: Vec<Arc<BlockArgNode>> = Vec::new();
        for node in ctx.blockDecls().unwrap().blockArg_all() {
            arguments.push(Arc::new(cast_node!(BlockArg(arg), arg, self.visit(&*node))));
        }

        let mut decls: Vec<Arc<BlockArgNode>> = Vec::new();
        for node in ctx.blockDecls().unwrap().blockDecl_all() {
            decls.push(Arc::new(cast_node!(BlockArg(arg), arg, self.visit(&*node))));
        }

        let decl_block = match ctx.blockDecls().unwrap().block() {
            | Some(db) => Some(Arc::new(cast_node!(Block(b), b, self.visit(&*db)))),
            | None => None,
        };

        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        Node {
            value: Block(BlockNode {
                name: Some(Arc::new(cast_node!(Symbol(s), s, self.visit(&*ctx.symbol().unwrap())))),
                arguments,
                decls,
                decl_block,
                statements,
            }),
        }
    }

    fn visit_BlockWDecls(&mut self, ctx: &BlockWDeclsContext<'a>) -> Self::Return {
        let declCtx = ctx.blockDecls().unwrap();

        let mut arguments: Vec<Arc<BlockArgNode>> = Vec::new();
        for node in declCtx.blockArg_all() {
            match self.visit(&*node).value {
                | BlockArg(arg) => {
                    arguments.push(Arc::new(arg));
                }
                | BlockIgnoredArgument => {
                    arguments.push(Arc::new(BlockArgNode {
                        identifier: Arc::new(IdentifierNode {
                            name: "?".to_string(),
                            namespace: None,
                            identifier_type: IdentifierType::Local,
                        }),
                        type_hint: None,
                    }));
                }
                | x => panic!("Very unexpected node type {:?} in block decls", x),
            }
        }

        let mut decls: Vec<Arc<BlockArgNode>> = Vec::new();
        for node in declCtx.blockDecl_all() {
            decls.push(Arc::new(cast_node!(BlockArg(arg), arg, self.visit(&*node))));
        }

        let decl_block = match declCtx.block() {
            | Some(db) => Some(Arc::new(cast_node!(Block(b), b, self.visit(&*db)))),
            | None => None,
        };

        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        Node {
            value: Block(BlockNode { name: None, arguments, decls, decl_block, statements }),
        }
    }

    fn visit_BlockNoDecls(&mut self, ctx: &BlockNoDeclsContext<'a>) -> Self::Return {
        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        Node {
            value: Block(BlockNode {
                name: None,
                arguments: vec![],
                decls: vec![],
                decl_block: None,
                statements,
            }),
        }
    }

    fn visit_BlockArgIgnored(&mut self, _ctx: &BlockArgIgnoredContext<'a>) -> Self::Return {
        Node { value: BlockIgnoredArgument }
    }

    fn visit_BlockArgTyped(&mut self, ctx: &BlockArgTypedContext<'a>) -> Self::Return {
        Node {
            value: BlockArg(BlockArgNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                type_hint: Some(Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.argtype.clone().unwrap())
                ))),
            }),
        }
    }

    fn visit_BlockArgUntyped(&mut self, ctx: &BlockArgUntypedContext<'a>) -> Self::Return {
        Node {
            value: BlockArg(BlockArgNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                type_hint: None,
            }),
        }
    }

    fn visit_BlockDeclTyped(&mut self, ctx: &BlockDeclTypedContext<'a>) -> Self::Return {
        Node {
            value: BlockDecl(BlockDeclNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                type_hint: Some(Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.argtype.clone().unwrap())
                ))),
            }),
        }
    }

    fn visit_BlockDeclUntyped(&mut self, ctx: &BlockDeclUntypedContext<'a>) -> Self::Return {
        Node {
            value: BlockDecl(BlockDeclNode {
                identifier: Arc::new(cast_node!(Identifier(id), id, self.visit(&*ctx.name.clone().unwrap()))),
                type_hint: None,
            }),
        }
    }

    fn visit_ArgIdentInst(&mut self, ctx: &ArgIdentInstContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Instance,
            }),
        }
    }

    fn visit_ArgIdent(&mut self, ctx: &ArgIdentContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn visit_IdentKeyword(&mut self, ctx: &IdentKeywordContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.keyword().unwrap().get_text(),
                identifier_type: IdentifierType::Keyword,
            }),
        }
    }

    fn visit_IdentOther(&mut self, ctx: &IdentOtherContext<'a>) -> Self::Return {
        Node {
            value: Identifier(IdentifierNode {
                namespace: None,
                name: ctx.IDENT().clone().unwrap().get_text(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn visit_symbol(&mut self, ctx: &SymbolContext<'a>) -> Self::Return {
        let symbolText = ctx.SYMBOL().unwrap().symbol.text.to_string();
        Node {
            value: Symbol(SymbolNode {
                value: symbolText
                    .trim_start_matches(&['#', '\''])
                    .trim_end_matches('\'')
                    .to_string(),
            }),
        }
    }
}

impl AstVisitor {
    fn add_bang_to_ident(id: IdentifierNode) -> IdentifierNode {
        IdentifierNode {
            namespace: id.namespace,
            name: id.name + "!",
            identifier_type: id.identifier_type,
        }
    }

    fn unescape(s: String) -> String {
        static ESCAPED_CHAR: Lazy<regex::Regex> = Lazy::new(|| {
            regex::Regex::new("\\\\(u[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]|[\\\\tnr\"'])").unwrap()
        });

        ESCAPED_CHAR
            .replace_all(s.as_str(), |caps: &Captures| {
                let s = caps[1].to_string();
                match s.as_str().substring(0, 1) {
                    | "n" => "\n".to_string(),
                    | "r" => "\r".to_string(),
                    | "t" => "\t".to_string(),
                    | "u" => {
                        let maybe_char = Self::unicode_from_hex(s.substring(1, s.len()).to_string());
                        match maybe_char {
                            | Some(x) => x.to_string(),
                            | None => panic!("Invalid unicode escape sequence \\u{s}"),
                        }
                    }
                    | "x" => {
                        let maybe_char = Self::unicode_from_hex(s.substring(1, s.len()).to_string());
                        match maybe_char {
                            | Some(x) => x.to_string(),
                            | None => panic!("Invalid unicode escape sequence \\x{s}"),
                        }
                    }
                    | _ => s,
                }
            })
            .to_string()
    }

    fn unicode_from_hex(s: String) -> Option<char> {
        let char_num: u32 = match u32::from_str_radix(s.as_str(), 16) {
            | Ok(n) => n,
            | Err(e) => panic!("Invalid unicode hex value \\x{s}: {}", e),
        };

        char::from_u32(char_num)
    }
}
