#![allow(non_snake_case)]

use crate::cast_node;
use crate::parser::ast_visitor::NodeValue::{
    Assignment, Bang3, BinaryOperator, Block, BlockArg, BlockDecl, BlockIgnoredArgument,
    BlockReturn, ClassDefinition, ClassExtension, ConstDefinition, Dot3, Double, Huh3, IdentLValue,
    Identifier, IgnoredLValue, IgnoredSplatLValue, Integer, List, Map, MethodCall,
    MethodCallArguments, MethodDefinition, MethodExtension, MethodReturn, MethodSelector,
    Namespace, Program, Regex, Set, SplatLValue, Str, SubLValue, Symbol, UnaryOperator, UserList,
    UserString, YieldReturn,
};
use crate::parser::generated::buildingblocksparser::{
    AddExprContext, AndExprContext, ArgIdentInstContext, ArgIdentInstContextAttrs,
    ArgIdentNormalContext, ArgIdentNormalContextAttrs, AssignmentContext, AssignmentContextAttrs,
    AssignmentStmtContext, AssignmentStmtContextAttrs, Bang3StmtContext, BlockArgIgnoredContext,
    BlockArgTypedContext, BlockArgUntypedContext, BlockDeclTypedContext, BlockDeclUntypedContext,
    BlockDeclsContext, BlockDeclsContextAttrs, BlockNoDeclsContext, BlockNoDeclsContextAttrs,
    BlockReturnContext, BlockReturnContextAttrs, BlockWDeclsContext, BlockWDeclsContextAttrs,
    BuildingBlocksParserContextType, CallSigNoArgBangContext, CallSigNoArgBangContextAttrs,
    CallSigNoArgContext, CallSigNoArgContextAttrs, CallSigWArgContext, CallSigWArgContextAttrs,
    ClassDef2ExprContext, ClassDef2ExprContextAttrs, ClassDefExprContext, ClassDefExprContextAttrs,
    ClassExtExprContext, ClassExtExprContextAttrs, ConstDefExprContext, ConstDefExprContextAttrs,
    DefCallExprContext, DictExprContext, DivExprContext, Dot3StmtContext,
    EqExprContext, ExprCallExprContext, ExprStmtContext, ExprStmtContextAttrs, FullNSContext,
    FullNSContextAttrs, GtEqExprContext, GtExprContext, Huh3StmtContext, IdentKeywordContext,
    IdentKeywordContextAttrs, IdentLValueContext, IdentLValueContextAttrs, IdentOtherContext,
    IdentOtherContextAttrs, IgnoredLValueContext, IgnoredSplatLValueContext, InstanceIdentContext,
    InstanceIdentContextAttrs, ListExprContext, ListExprContextAttrs, LiteralNumberContext,
    LiteralStringContext, LiteralStringContextAttrs, LiteralSymbolContext,
    LiteralSymbolContextAttrs, LocalIdentContext, LocalIdentContextAttrs, LtEqExprContext,
    LtExprContext, MatchExprContext, MethodDefExprContext, MethodDefExprContextAttrs,
    MethodExtExprContext, MethodExtExprContextAttrs, MethodReturnContext, MethodReturnContextAttrs,
    ModExprContext, MulExprContext, NamedBlockWDeclsContext, NamedBlockWDeclsContextAttrs,
    NamespacedIdentContext, NamespacedIdentContextAttrs, NestedExprContext, NestedExprContextAttrs,
    NotEqExprContext, OrExprContext, ProgramContext, ProgramContextAttrs, RangeExprContext,
    RegexExprContext, RegexExprContextAttrs, RootNSContext, SelectorNoArgsBangContext,
    SelectorNoArgsBangContextAttrs, SelectorNoArgsContext, SelectorNoArgsContextAttrs,
    SelectorSymbolContext, SelectorSymbolContextAttrs, SelectorWArgsContext,
    SelectorWArgsContextAttrs, SetExprContext, SetExprContextAttrs, SplatLValueContext,
    SplatLValueContextAttrs, SubExprContext, SubLValueContext, SubLValueContextAttrs,
    SymbolContext, SymbolContextAttrs, UnBangExprContext, UnBangExprContextAttrs,
    UnMinusExprContext, UnMinusExprContextAttrs, UnModExprContext, UnModExprContextAttrs,
    UnPlusExprContext, UnPlusExprContextAttrs, UserListExprContext, UserListExprContextAttrs,
    UserStringExprContext, UserStringExprContextAttrs, YieldReturnContext, YieldReturnContextAttrs,
};
use crate::parser::generated::buildingblocksvisitor::BuildingBlocksVisitorCompat;
use crate::value::SourceInfo;

use once_cell::sync::Lazy;
use regex::Captures;
use std::string::String;
use std::sync::Arc;
use substring::Substring;

use antlr_rust::parser_rule_context::ParserRuleContext;
use antlr_rust::token::Token;
use antlr_rust::tree::{ParseTree, ParseTreeVisitorCompat};

#[derive(Debug, Default, Clone, PartialEq)]
pub enum IdentifierType {
    #[default]
    Unknown,
    Local,
    Instance,
    Namespaced,
    Keyword,
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
    YieldReturn(YieldReturnNode),
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Node {
    pub value: NodeValue,
    pub source_info: Option<SourceInfo>,
}

pub struct AstVisitor {
    pub x: Node,
    pub filename: String,
    pub source_text: String,
}

impl AstVisitor {
    fn extract_source_info<'a, T>(&self, ctx: &T) -> Option<SourceInfo>
    where
        T: ParserRuleContext<'a> + ?Sized,
    {
        let start_tok = ctx.start();
        let stop_tok = ctx.stop();
        let line = start_tok.get_line() as usize;
        let column = start_tok.get_column() as usize;
        let start_char = start_tok.get_start() as usize;
        let stop_char = stop_tok.get_stop() as usize;
        let source_text = self
            .source_text
            .get(start_char..=stop_char)
            .map(|s| s.to_string());
        Some(SourceInfo {
            filename: self.filename.clone(),
            line,
            column,
            start: start_char,
            end: stop_char + 1,
            source_text,
        })
    }
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
        let source_info = self.extract_source_info(ctx);
        Node {
            source_info: source_info.clone(),
            value: Program(ProgramNode {
                expressions: stmts,
                source_info,
            }),
        }
    }

    fn visit_MethodReturn(&mut self, ctx: &MethodReturnContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodReturn(MethodReturnNode {
                value: Arc::new(self.visit(&*ctx.expr().unwrap())),
            }),
        }
    }

    fn visit_YieldReturn(&mut self, ctx: &YieldReturnContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: YieldReturn(YieldReturnNode {
                value: Arc::new(self.visit(&*ctx.expr().unwrap())),
            }),
        }
    }

    fn visit_BlockReturn(&mut self, ctx: &BlockReturnContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: BlockReturn(BlockReturnNode {
                value: Arc::new(self.visit(&*ctx.expr().unwrap())),
            }),
        }
    }

    fn visit_AssignmentStmt(&mut self, ctx: &AssignmentStmtContext<'a>) -> Self::Return {
        self.visit(&*ctx.assignment().unwrap())
    }

    fn visit_Bang3Stmt(&mut self, ctx: &Bang3StmtContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Bang3,
        }
    }

    fn visit_Dot3Stmt(&mut self, ctx: &Dot3StmtContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Dot3,
        }
    }

    fn visit_Huh3Stmt(&mut self, ctx: &Huh3StmtContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Huh3,
        }
    }

    fn visit_ExprStmt(&mut self, ctx: &ExprStmtContext<'a>) -> Self::Return {
        self.visit(&*ctx.expr().unwrap())
    }

    fn visit_SelectorWArgs(&mut self, ctx: &SelectorWArgsContext<'a>) -> Self::Return {
        let mut idents: Vec<Arc<IdentifierNode>> = Vec::new();
        for node in ctx.ident_all() {
            let id = cast_node!(Identifier(id), id, self.visit(&*node));
            idents.push(Arc::new(IdentifierNode {
                source_info: id.source_info.clone(),
                namespace: id.namespace.clone(),
                name: format!("{}:", id.name),
                identifier_type: id.identifier_type,
            }));
        }
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodSelector(MethodSelectorNode {
                identifiers: idents,
            }),
        }
    }

    fn visit_SelectorNoArgs(&mut self, ctx: &SelectorNoArgsContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodSelector(MethodSelectorNode {
                identifiers: vec![Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.ident().unwrap())
                ))],
            }),
        }
    }

    fn visit_SelectorNoArgsBang(&mut self, ctx: &SelectorNoArgsBangContext<'a>) -> Self::Return {
        let node = self.visit(&*ctx.ident().unwrap());
        let ident = Self::add_bang_to_ident(cast_node!(Identifier(id), id, node));
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodSelector(MethodSelectorNode {
                identifiers: vec![Arc::new(ident)],
            }),
        }
    }

    fn visit_SelectorSymbol(&mut self, ctx: &SelectorSymbolContext<'a>) -> Self::Return {
        let binding = ctx.symbol().unwrap().get_text();
        let selectorText = binding.trim_start_matches('#').trim_matches('\'');
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodSelector(MethodSelectorNode {
                identifiers: vec![Arc::new(IdentifierNode {
                    source_info: self.extract_source_info(ctx),
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
            source_info: self.extract_source_info(ctx),
            value: Assignment(AssignmentNode {
                lvalues: nodes,
                rvalue: Arc::new(self.visit(&*ctx.expr().unwrap())),
            }),
        }
    }

    fn visit_IdentLValue(&mut self, ctx: &IdentLValueContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: IdentLValue(IdentLValueNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.nsvarident().unwrap())
                )),
            }),
        }
    }

    fn visit_SplatLValue(&mut self, ctx: &SplatLValueContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: SplatLValue(SplatLValueNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.nsvarident().unwrap())
                )),
            }),
        }
    }

    fn visit_IgnoredLValue(&mut self, ctx: &IgnoredLValueContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: IgnoredLValue,
        }
    }

    fn visit_IgnoredSplatLValue(&mut self, ctx: &IgnoredSplatLValueContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: IgnoredSplatLValue,
        }
    }

    fn visit_SubLValue(&mut self, ctx: &SubLValueContext<'a>) -> Self::Return {
        let mut lvalues: Vec<Arc<Node>> = Vec::new();
        for node in ctx.lvalue_all() {
            lvalues.push(Arc::new(self.visit(&*node)));
        }
        Node {
            source_info: self.extract_source_info(ctx),
            value: SubLValue(SubLValueNode { lvalues }),
        }
    }

    fn visit_MulExpr(&mut self, ctx: &MulExprContext<'a>) -> Self::Return {
        let left = self.visit(&*ctx.left.clone().unwrap());
        let right = self.visit(&*ctx.right.clone().unwrap());
        self.make_binary_operator(
            BinaryOperatorType::Mul,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_AndExpr(&mut self, ctx: &AndExprContext<'a>) -> Self::Return {
        let left = self.visit(&*ctx.left.clone().unwrap());
        let right = self.visit(&*ctx.right.clone().unwrap());
        self.make_binary_operator(
            BinaryOperatorType::And,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_LiteralString(&mut self, ctx: &LiteralStringContext<'a>) -> Self::Return {
        let raw_string = ctx.string().unwrap().get_text().to_string();
        let inner_string = raw_string.substring(1, raw_string.len() - 1).to_string();
        let unescaped_string = Self::unescape(inner_string);
        Node {
            source_info: self.extract_source_info(ctx),
            value: Str(StringNode {
                value: unescaped_string,
            }),
        }
    }

    fn visit_UserStringExpr(&mut self, ctx: &UserStringExprContext<'a>) -> Self::Return {
        // #Ident'......'
        let raw_string = ctx.userString().unwrap().get_text();

        let string_start = raw_string
            .find('\'')
            .unwrap_or_else(|| panic!("Invalid user string: {}", raw_string));
        let ident_string = raw_string.substring(1, string_start);
        let string_string = raw_string
            .substring(string_start + 1, raw_string.len() - 1)
            .to_string();
        let unescaped_string = Self::unescape(string_string.clone());

        Node {
            source_info: self.extract_source_info(ctx),
            value: UserString(UserStringNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
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
            source_info: self.extract_source_info(ctx),
            value: Regex(RegexNode {
                value: ctx.REGEXP().unwrap().get_text(),
            }),
        }
    }

    fn visit_GtExpr(&mut self, ctx: &GtExprContext<'a>) -> Self::Return {
        let left = self.visit(&*ctx.left.clone().unwrap());
        let right = self.visit(&*ctx.right.clone().unwrap());
        self.make_binary_operator(
            BinaryOperatorType::Gt,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_LtExpr(&mut self, ctx: &LtExprContext<'a>) -> Self::Return {
        let left = self.visit(&*ctx.left.clone().unwrap());
        let right = self.visit(&*ctx.right.clone().unwrap());
        self.make_binary_operator(
            BinaryOperatorType::Lt,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_UserListExpr(&mut self, ctx: &UserListExprContext<'a>) -> Self::Return {
        let raw_start = ctx.USER_LIST_START().unwrap().get_text();
        let ident_name = raw_start
            .trim_start_matches('#')
            .trim_end_matches('(')
            .to_string();

        let mut values: Vec<Arc<Node>> = Vec::new();
        for node in ctx.expr_all() {
            values.push(Arc::new(self.visit(&*node)));
        }

        Node {
            source_info: self.extract_source_info(ctx),
            value: UserList(UserListNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: ident_name,
                    identifier_type: IdentifierType::Local,
                }),
                values,
            }),
        }
    }

    fn visit_LtEqExpr(&mut self, ctx: &LtEqExprContext<'a>) -> Self::Return {
        let left = self.visit(&*ctx.left.clone().unwrap());
        let right = self.visit(&*ctx.right.clone().unwrap());
        self.make_binary_operator(
            BinaryOperatorType::LtEq,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_MethodDefExpr(&mut self, ctx: &MethodDefExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(cast_node!(
                    MethodSelector(ms),
                    ms,
                    self.visit(&*ctx.selector().unwrap())
                )),
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_LiteralSymbol(&mut self, ctx: &LiteralSymbolContext<'a>) -> Self::Return {
        let binding = ctx.symbol().unwrap().get_text();
        let symbolText = binding.trim_start_matches('#').trim_matches('\'');
        Node {
            source_info: self.extract_source_info(ctx),
            value: Symbol(SymbolNode {
                value: String::from(symbolText),
            }),
        }
    }

    fn visit_ClassDefExpr(&mut self, ctx: &ClassDefExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
                parent_identifier: None,
                block: Arc::new(cast_node!(Block(b), b, self.visit(&*ctx.block().unwrap()))),
            }),
        }
    }

    fn visit_ExprCallExpr(&mut self, ctx: &ExprCallExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
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

        Node {
            source_info: self.extract_source_info(ctx),
            value: Set(SetNode { values: exprs }),
        }
    }

    fn visit_UnModExpr(&mut self, ctx: &UnModExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Mod,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_MethodExtExpr(&mut self, ctx: &MethodExtExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodExtension(MethodExtensionNode {
                signature: Arc::new(cast_node!(
                    MethodSelector(ms),
                    ms,
                    self.visit(&*ctx.selector().unwrap())
                )),
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

        Node {
            source_info: self.extract_source_info(ctx),
            value: Map(MapNode { keys, values }),
        }
    }

    fn visit_ListExpr(&mut self, ctx: &ListExprContext<'a>) -> Self::Return {
        let mut exprs: Vec<Arc<Node>> = Vec::new();
        for node in ctx.expr_all() {
            exprs.push(Arc::new(self.visit(&*node)));
        }

        Node {
            source_info: self.extract_source_info(ctx),
            value: List(ListNode { values: exprs }),
        }
    }

    fn visit_SubExpr(&mut self, ctx: &SubExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Sub,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_AddExpr(&mut self, ctx: &AddExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Add,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_ConstDefExpr(&mut self, ctx: &ConstDefExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: ConstDefinition(ConstDefinitionNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.nsvarident().unwrap())
                )),
                rvalue: Arc::new(self.visit(ctx.expr().clone().unwrap().as_ref())),
            }),
        }
    }

    fn visit_RangeExpr(&mut self, ctx: &RangeExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Range,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_UnPlusExpr(&mut self, ctx: &UnPlusExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Add,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_OrExpr(&mut self, ctx: &OrExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Or,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_ClassDef2Expr(&mut self, ctx: &ClassDef2ExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
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
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::GtEq,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_DivExpr(&mut self, ctx: &DivExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Div,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_UnBangExpr(&mut self, ctx: &UnBangExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Bang,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_NotEqExpr(&mut self, ctx: &NotEqExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::NotEq,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_UnMinusExpr(&mut self, ctx: &UnMinusExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: UnaryOperator(UnaryOperatorNode {
                operator: UnaryOperatorType::Sub,
                right: Arc::new(self.visit(&*ctx.expr().clone().unwrap())),
            }),
        }
    }

    fn visit_EqExpr(&mut self, ctx: &EqExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Eq,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_ClassExtExpr(&mut self, ctx: &ClassExtExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
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
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Mod,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_MatchExpr(&mut self, ctx: &MatchExprContext<'a>) -> Self::Return {
        let left = self.visit(ctx.left.clone().unwrap().as_ref());
        let right = self.visit(ctx.right.clone().unwrap().as_ref());
        self.make_binary_operator(
            BinaryOperatorType::Match,
            left,
            right,
            self.extract_source_info(ctx),
        )
    }

    fn visit_DefCallExpr(&mut self, ctx: &DefCallExprContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(cast_node!(
                    MethodCallArguments(args),
                    args,
                    self.visit(&*ctx.sig.clone().unwrap())
                )),
            }),
        }
    }

    fn visit_LiteralNumber(&mut self, ctx: &LiteralNumberContext<'a>) -> Self::Return {
        let numtext = ctx.get_text();

        let nodeValue = if numtext.contains('.') {
            Double(DoubleNode {
                value: numtext.parse::<f64>().unwrap(),
            })
        } else {
            Integer(IntegerNode {
                value: numtext.parse::<i64>().unwrap(),
            })
        };

        Node {
            source_info: self.extract_source_info(ctx),
            value: nodeValue,
        }
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
            source_info: self.extract_source_info(ctx),
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: idents,
                }),
                expressions: exprs,
            }),
        }
    }

    fn visit_CallSigNoArg(&mut self, ctx: &CallSigNoArgContext<'a>) -> Self::Return {
        let ident = cast_node!(Identifier(id), id, self.visit(&*ctx.ident().unwrap()));
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(ident)],
                }),
                expressions: vec![],
            }),
        }
    }

    fn visit_CallSigNoArgBang(&mut self, ctx: &CallSigNoArgBangContext<'a>) -> Self::Return {
        let ident = cast_node!(Identifier(id), id, self.visit(&*ctx.ident().unwrap()));
        let ident = Self::add_bang_to_ident(ident);
        Node {
            source_info: self.extract_source_info(ctx),
            value: MethodCallArguments(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(ident)],
                }),
                expressions: vec![],
            }),
        }
    }

    fn visit_NamespacedIdent(&mut self, ctx: &NamespacedIdentContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: Some(Arc::new(cast_node!(
                    Namespace(ns),
                    ns,
                    self.visit(&*ctx.namespace().unwrap())
                ))),
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Namespaced,
            }),
        }
    }

    fn visit_InstanceIdent(&mut self, ctx: &InstanceIdentContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Instance,
            }),
        }
    }

    fn visit_LocalIdent(&mut self, ctx: &LocalIdentContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
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

        Node {
            source_info: self.extract_source_info(ctx),
            value: Namespace(NamespaceNode {
                source_info: self.extract_source_info(ctx),
                identifiers: idents,
            }),
        }
    }

    fn visit_RootNS(&mut self, ctx: &RootNSContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Namespace(NamespaceNode {
                source_info: self.extract_source_info(ctx),
                identifiers: vec![],
            }),
        }
    }

    fn visit_NamedBlockWDecls(&mut self, ctx: &NamedBlockWDeclsContext<'a>) -> Self::Return {
        let decl_ctx = ctx.blockDecls().unwrap();
        let (arguments, decls, decl_block) = self.parse_block_decls(&decl_ctx);

        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        let source_info = self.extract_source_info(ctx);
        Node {
            source_info: source_info.clone(),
            value: Block(BlockNode {
                name: Some(Arc::new(cast_node!(
                    Symbol(s),
                    s,
                    self.visit(&*ctx.symbol().unwrap())
                ))),
                arguments,
                decls,
                decl_block,
                statements,
                source_info,
            }),
        }
    }

    fn visit_BlockWDecls(&mut self, ctx: &BlockWDeclsContext<'a>) -> Self::Return {
        let decl_ctx = ctx.blockDecls().unwrap();
        let (arguments, decls, decl_block) = self.parse_block_decls(&decl_ctx);

        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        let source_info = self.extract_source_info(ctx);
        Node {
            source_info: source_info.clone(),
            value: Block(BlockNode {
                name: None,
                arguments,
                decls,
                decl_block,
                statements,
                source_info,
            }),
        }
    }

    fn visit_BlockNoDecls(&mut self, ctx: &BlockNoDeclsContext<'a>) -> Self::Return {
        let mut statements: Vec<Arc<Node>> = Vec::new();
        for node in ctx.stmt_all() {
            statements.push(Arc::new(self.visit(&*node)));
        }

        let source_info = self.extract_source_info(ctx);
        Node {
            source_info: source_info.clone(),
            value: Block(BlockNode {
                name: None,
                arguments: vec![],
                decls: vec![],
                decl_block: None,
                statements,
                source_info,
            }),
        }
    }

    fn visit_BlockArgIgnored(&mut self, ctx: &BlockArgIgnoredContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: BlockIgnoredArgument,
        }
    }

    fn visit_BlockArgTyped(&mut self, ctx: &BlockArgTypedContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: BlockArg(BlockArgNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
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
            source_info: self.extract_source_info(ctx),
            value: BlockArg(BlockArgNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
                type_hint: None,
            }),
        }
    }

    fn visit_BlockDeclTyped(&mut self, ctx: &BlockDeclTypedContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: BlockDecl(BlockDeclNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
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
            source_info: self.extract_source_info(ctx),
            value: BlockDecl(BlockDeclNode {
                identifier: Arc::new(cast_node!(
                    Identifier(id),
                    id,
                    self.visit(&*ctx.name.clone().unwrap())
                )),
                type_hint: None,
            }),
        }
    }

    fn visit_ArgIdentInst(&mut self, ctx: &ArgIdentInstContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Instance,
            }),
        }
    }

    fn visit_ArgIdentNormal(&mut self, ctx: &ArgIdentNormalContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: None,
                name: ctx.ident().unwrap().get_text(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn visit_IdentKeyword(&mut self, ctx: &IdentKeywordContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: None,
                name: ctx.keyword().unwrap().get_text(),
                identifier_type: IdentifierType::Keyword,
            }),
        }
    }

    fn visit_IdentOther(&mut self, ctx: &IdentOtherContext<'a>) -> Self::Return {
        Node {
            source_info: self.extract_source_info(ctx),
            value: Identifier(IdentifierNode {
                source_info: self.extract_source_info(ctx),
                namespace: None,
                name: ctx.IDENT().clone().unwrap().get_text(),
                identifier_type: IdentifierType::Local,
            }),
        }
    }

    fn visit_symbol(&mut self, ctx: &SymbolContext<'a>) -> Self::Return {
        let symbolText = ctx.SYMBOL().unwrap().symbol.text.to_string();
        Node {
            source_info: self.extract_source_info(ctx),
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
    fn parse_block_decls<'a>(
        &mut self,
        decl_ctx: &BlockDeclsContext<'a>,
    ) -> (
        Vec<Arc<BlockArgNode>>,
        Vec<Arc<BlockDeclNode>>,
        Option<Arc<BlockNode>>,
    ) {
        let mut arguments: Vec<Arc<BlockArgNode>> = Vec::new();
        for node in decl_ctx.blockArg_all() {
            match self.visit(&*node).value {
                BlockArg(arg) => {
                    arguments.push(Arc::new(arg));
                }
                BlockIgnoredArgument => {
                    arguments.push(Arc::new(BlockArgNode {
                        identifier: Arc::new(IdentifierNode {
                            source_info: None,
                            name: "_".to_string(),
                            namespace: None,
                            identifier_type: IdentifierType::Local,
                        }),
                        type_hint: None,
                    }));
                }
                x => panic!("Very unexpected node type {:?} in block decls", x),
            }
        }

        let mut decls: Vec<Arc<BlockDeclNode>> = Vec::new();
        for node in decl_ctx.blockDecl_all() {
            decls.push(Arc::new(cast_node!(
                BlockDecl(arg),
                arg,
                self.visit(&*node)
            )));
        }

        let decl_block = match decl_ctx.block() {
            Some(db) => Some(Arc::new(cast_node!(Block(b), b, self.visit(&*db)))),
            None => None,
        };

        (arguments, decls, decl_block)
    }

    fn make_binary_operator(
        &mut self,
        op: BinaryOperatorType,
        left: Node,
        right: Node,
        source_info: Option<SourceInfo>,
    ) -> Node {
        Node {
            source_info,
            value: BinaryOperator(BinaryOperatorNode {
                operator: op,
                left: Arc::new(left),
                right: Arc::new(right),
            }),
        }
    }

    fn add_bang_to_ident(id: IdentifierNode) -> IdentifierNode {
        IdentifierNode {
            source_info: id.source_info,
            namespace: id.namespace,
            name: id.name + "!",
            identifier_type: id.identifier_type,
        }
    }

    fn unescape(s: String) -> String {
        static ESCAPED_CHAR: Lazy<regex::Regex> = Lazy::new(|| {
            regex::Regex::new("\\\\(u[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]|[\\\\tnr\"'])")
                .unwrap()
        });

        ESCAPED_CHAR
            .replace_all(s.as_str(), |caps: &Captures| {
                let s = caps[1].to_string();
                match s.as_str().substring(0, 1) {
                    "n" => "\n".to_string(),
                    "r" => "\r".to_string(),
                    "t" => "\t".to_string(),
                    "u" => {
                        let maybe_char =
                            Self::unicode_from_hex(s.substring(1, s.len()).to_string());
                        match maybe_char {
                            Some(x) => x.to_string(),
                            None => panic!("Invalid unicode escape sequence \\u{s}"),
                        }
                    }
                    "x" => {
                        let maybe_char =
                            Self::unicode_from_hex(s.substring(1, s.len()).to_string());
                        match maybe_char {
                            Some(x) => x.to_string(),
                            None => panic!("Invalid unicode escape sequence \\x{s}"),
                        }
                    }
                    _ => s,
                }
            })
            .to_string()
    }

    fn unicode_from_hex(s: String) -> Option<char> {
        let char_num: u32 = match u32::from_str_radix(s.as_str(), 16) {
            Ok(n) => n,
            Err(e) => panic!("Invalid unicode hex value \\x{s}: {}", e),
        };

        char::from_u32(char_num)
    }
}

impl BlockNode {
    pub fn clear_source_info(&mut self) {
        self.source_info = None;
        for s in &mut self.statements {
            Arc::make_mut(s).clear_source_info();
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
