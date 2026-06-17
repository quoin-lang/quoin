#![allow(nonstandard_style)]
// Generated from ./BuildingBlocks.g4 by ANTLR 4.8
use super::buildingblocksparser::*;
use antlr_rust::tree::{ParseTreeVisitor, ParseTreeVisitorCompat};

/**
 * This interface defines a complete generic visitor for a parse tree produced
 * by {@link BuildingBlocksParser}.
 */
pub trait BuildingBlocksVisitor<'input>:
    ParseTreeVisitor<'input, BuildingBlocksParserContextType>
{
    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#program}.
     * @param ctx the parse tree
     */
    fn visit_program(&mut self, ctx: &ProgramContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_MethodReturn(&mut self, ctx: &MethodReturnContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code YieldReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_YieldReturn(&mut self, ctx: &YieldReturnContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_BlockReturn(&mut self, ctx: &BlockReturnContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AssignmentStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_AssignmentStmt(&mut self, ctx: &AssignmentStmtContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Bang3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Bang3Stmt(&mut self, ctx: &Bang3StmtContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Dot3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Dot3Stmt(&mut self, ctx: &Dot3StmtContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Huh3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Huh3Stmt(&mut self, ctx: &Huh3StmtContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_ExprStmt(&mut self, ctx: &ExprStmtContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#bang3}.
     * @param ctx the parse tree
     */
    fn visit_bang3(&mut self, ctx: &Bang3Context<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#dot3}.
     * @param ctx the parse tree
     */
    fn visit_dot3(&mut self, ctx: &Dot3Context<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#huh3}.
     * @param ctx the parse tree
     */
    fn visit_huh3(&mut self, ctx: &Huh3Context<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorWArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorWArgs(&mut self, ctx: &SelectorWArgsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorNoArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorNoArgs(&mut self, ctx: &SelectorNoArgsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorNoArgsBang}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorNoArgsBang(&mut self, ctx: &SelectorNoArgsBangContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorSymbol}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorSymbol(&mut self, ctx: &SelectorSymbolContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#assignment}.
     * @param ctx the parse tree
     */
    fn visit_assignment(&mut self, ctx: &AssignmentContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IdentLValue(&mut self, ctx: &IdentLValueContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_SplatLValue(&mut self, ctx: &SplatLValueContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IgnoredLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IgnoredLValue(&mut self, ctx: &IgnoredLValueContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IgnoredSplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IgnoredSplatLValue(&mut self, ctx: &IgnoredSplatLValueContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SubLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_SubLValue(&mut self, ctx: &SubLValueContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DefCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_DefCallWArgExpr(&mut self, ctx: &DefCallWArgExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_ExprCallWArgExpr(&mut self, ctx: &ExprCallWArgExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RichExprBase}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_RichExprBase(&mut self, ctx: &RichExprBaseContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MulExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MulExpr(&mut self, ctx: &MulExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AndExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_AndExpr(&mut self, ctx: &AndExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralString}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralString(&mut self, ctx: &LiteralStringContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UserStringExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UserStringExpr(&mut self, ctx: &UserStringExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RegexExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_RegexExpr(&mut self, ctx: &RegexExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code GtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_GtExpr(&mut self, ctx: &GtExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LtExpr(&mut self, ctx: &LtExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UserListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UserListExpr(&mut self, ctx: &UserListExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LtEqExpr(&mut self, ctx: &LtEqExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MethodDefExpr(&mut self, ctx: &MethodDefExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralSymbol}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralSymbol(&mut self, ctx: &LiteralSymbolContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassDefExpr(&mut self, ctx: &ClassDefExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ExprCallExpr(&mut self, ctx: &ExprCallExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SetExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_SetExpr(&mut self, ctx: &SetExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnModExpr(&mut self, ctx: &UnModExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MethodExtExpr(&mut self, ctx: &MethodExtExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DictExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DictExpr(&mut self, ctx: &DictExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ListExpr(&mut self, ctx: &ListExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_IdentExpr(&mut self, ctx: &IdentExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SubExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_SubExpr(&mut self, ctx: &SubExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AddExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_AddExpr(&mut self, ctx: &AddExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ConstDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ConstDefExpr(&mut self, ctx: &ConstDefExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RangeExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_RangeExpr(&mut self, ctx: &RangeExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnPlusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnPlusExpr(&mut self, ctx: &UnPlusExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_BlockExpr(&mut self, ctx: &BlockExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code OrExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_OrExpr(&mut self, ctx: &OrExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassDef2Expr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassDef2Expr(&mut self, ctx: &ClassDef2ExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code GtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_GtEqExpr(&mut self, ctx: &GtEqExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DivExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DivExpr(&mut self, ctx: &DivExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnBangExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnBangExpr(&mut self, ctx: &UnBangExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NotEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_NotEqExpr(&mut self, ctx: &NotEqExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnMinusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnMinusExpr(&mut self, ctx: &UnMinusExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code EqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_EqExpr(&mut self, ctx: &EqExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassExtExpr(&mut self, ctx: &ClassExtExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NestedExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_NestedExpr(&mut self, ctx: &NestedExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ModExpr(&mut self, ctx: &ModExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MatchExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MatchExpr(&mut self, ctx: &MatchExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DefCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DefCallExpr(&mut self, ctx: &DefCallExprContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralNumber}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralNumber(&mut self, ctx: &LiteralNumberContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#userString}.
     * @param ctx the parse tree
     */
    fn visit_userString(&mut self, ctx: &UserStringContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigWArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigWArg(&mut self, ctx: &CallSigWArgContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArg(&mut self, ctx: &CallSigNoArgContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgBang}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgBang(&mut self, ctx: &CallSigNoArgBangContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#callSigWithArg}.
     * @param ctx the parse tree
     */
    fn visit_callSigWithArg(&mut self, ctx: &CallSigWithArgContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgDefCallWArg}
     * labeled alternative in {@link BuildingBlocksParser#argExpr}.
     * @param ctx the parse tree
     */
    fn visit_ArgDefCallWArg(&mut self, ctx: &ArgDefCallWArgContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgExprBase}
     * labeled alternative in {@link BuildingBlocksParser#argExpr}.
     * @param ctx the parse tree
     */
    fn visit_ArgExprBase(&mut self, ctx: &ArgExprBaseContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgNormal(&mut self, ctx: &CallSigNoArgNormalContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgBangNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgBangNormal(&mut self, ctx: &CallSigNoArgBangNormalContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NamespacedIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_NamespacedIdent(&mut self, ctx: &NamespacedIdentContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code InstanceIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_InstanceIdent(&mut self, ctx: &InstanceIdentContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LocalIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_LocalIdent(&mut self, ctx: &LocalIdentContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code FullNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn visit_FullNS(&mut self, ctx: &FullNSContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RootNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn visit_RootNS(&mut self, ctx: &RootNSContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#keyword}.
     * @param ctx the parse tree
     */
    fn visit_keyword(&mut self, ctx: &KeywordContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NamedBlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_NamedBlockWDecls(&mut self, ctx: &NamedBlockWDeclsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_BlockWDecls(&mut self, ctx: &BlockWDeclsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockNoDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_BlockNoDecls(&mut self, ctx: &BlockNoDeclsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#blockDecls}.
     * @param ctx the parse tree
     */
    fn visit_blockDecls(&mut self, ctx: &BlockDeclsContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgIgnored}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgIgnored(&mut self, ctx: &BlockArgIgnoredContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgTyped(&mut self, ctx: &BlockArgTypedContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgUntyped(&mut self, ctx: &BlockArgUntypedContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockDeclTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn visit_BlockDeclTyped(&mut self, ctx: &BlockDeclTypedContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockDeclUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn visit_BlockDeclUntyped(&mut self, ctx: &BlockDeclUntypedContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#string}.
     * @param ctx the parse tree
     */
    fn visit_string(&mut self, ctx: &StringContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgIdentInst}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn visit_ArgIdentInst(&mut self, ctx: &ArgIdentInstContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgIdentNormal}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn visit_ArgIdentNormal(&mut self, ctx: &ArgIdentNormalContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentKeyword}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn visit_IdentKeyword(&mut self, ctx: &IdentKeywordContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentOther}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn visit_IdentOther(&mut self, ctx: &IdentOtherContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#symbol}.
     * @param ctx the parse tree
     */
    fn visit_symbol(&mut self, ctx: &SymbolContext<'input>) {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#number}.
     * @param ctx the parse tree
     */
    fn visit_number(&mut self, ctx: &NumberContext<'input>) {
        self.visit_children(ctx)
    }
}

pub trait BuildingBlocksVisitorCompat<'input>:
    ParseTreeVisitorCompat<'input, Node = BuildingBlocksParserContextType>
{
    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#program}.
     * @param ctx the parse tree
     */
    fn visit_program(&mut self, ctx: &ProgramContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_MethodReturn(&mut self, ctx: &MethodReturnContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code YieldReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_YieldReturn(&mut self, ctx: &YieldReturnContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_BlockReturn(&mut self, ctx: &BlockReturnContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AssignmentStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_AssignmentStmt(&mut self, ctx: &AssignmentStmtContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Bang3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Bang3Stmt(&mut self, ctx: &Bang3StmtContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Dot3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Dot3Stmt(&mut self, ctx: &Dot3StmtContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code Huh3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_Huh3Stmt(&mut self, ctx: &Huh3StmtContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn visit_ExprStmt(&mut self, ctx: &ExprStmtContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#bang3}.
     * @param ctx the parse tree
     */
    fn visit_bang3(&mut self, ctx: &Bang3Context<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#dot3}.
     * @param ctx the parse tree
     */
    fn visit_dot3(&mut self, ctx: &Dot3Context<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#huh3}.
     * @param ctx the parse tree
     */
    fn visit_huh3(&mut self, ctx: &Huh3Context<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorWArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorWArgs(&mut self, ctx: &SelectorWArgsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorNoArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorNoArgs(&mut self, ctx: &SelectorNoArgsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorNoArgsBang}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorNoArgsBang(
        &mut self,
        ctx: &SelectorNoArgsBangContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SelectorSymbol}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn visit_SelectorSymbol(&mut self, ctx: &SelectorSymbolContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#assignment}.
     * @param ctx the parse tree
     */
    fn visit_assignment(&mut self, ctx: &AssignmentContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IdentLValue(&mut self, ctx: &IdentLValueContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_SplatLValue(&mut self, ctx: &SplatLValueContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IgnoredLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IgnoredLValue(&mut self, ctx: &IgnoredLValueContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IgnoredSplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_IgnoredSplatLValue(
        &mut self,
        ctx: &IgnoredSplatLValueContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SubLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn visit_SubLValue(&mut self, ctx: &SubLValueContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DefCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_DefCallWArgExpr(&mut self, ctx: &DefCallWArgExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_ExprCallWArgExpr(&mut self, ctx: &ExprCallWArgExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RichExprBase}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn visit_RichExprBase(&mut self, ctx: &RichExprBaseContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MulExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MulExpr(&mut self, ctx: &MulExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AndExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_AndExpr(&mut self, ctx: &AndExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralString}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralString(&mut self, ctx: &LiteralStringContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UserStringExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UserStringExpr(&mut self, ctx: &UserStringExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RegexExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_RegexExpr(&mut self, ctx: &RegexExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code GtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_GtExpr(&mut self, ctx: &GtExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LtExpr(&mut self, ctx: &LtExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UserListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UserListExpr(&mut self, ctx: &UserListExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LtEqExpr(&mut self, ctx: &LtEqExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MethodDefExpr(&mut self, ctx: &MethodDefExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralSymbol}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralSymbol(&mut self, ctx: &LiteralSymbolContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassDefExpr(&mut self, ctx: &ClassDefExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ExprCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ExprCallExpr(&mut self, ctx: &ExprCallExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SetExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_SetExpr(&mut self, ctx: &SetExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnModExpr(&mut self, ctx: &UnModExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MethodExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MethodExtExpr(&mut self, ctx: &MethodExtExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DictExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DictExpr(&mut self, ctx: &DictExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ListExpr(&mut self, ctx: &ListExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_IdentExpr(&mut self, ctx: &IdentExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code SubExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_SubExpr(&mut self, ctx: &SubExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code AddExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_AddExpr(&mut self, ctx: &AddExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ConstDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ConstDefExpr(&mut self, ctx: &ConstDefExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RangeExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_RangeExpr(&mut self, ctx: &RangeExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnPlusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnPlusExpr(&mut self, ctx: &UnPlusExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_BlockExpr(&mut self, ctx: &BlockExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code OrExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_OrExpr(&mut self, ctx: &OrExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassDef2Expr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassDef2Expr(&mut self, ctx: &ClassDef2ExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code GtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_GtEqExpr(&mut self, ctx: &GtEqExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DivExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DivExpr(&mut self, ctx: &DivExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnBangExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnBangExpr(&mut self, ctx: &UnBangExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NotEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_NotEqExpr(&mut self, ctx: &NotEqExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code UnMinusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_UnMinusExpr(&mut self, ctx: &UnMinusExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code EqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_EqExpr(&mut self, ctx: &EqExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ClassExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ClassExtExpr(&mut self, ctx: &ClassExtExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NestedExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_NestedExpr(&mut self, ctx: &NestedExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_ModExpr(&mut self, ctx: &ModExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code MatchExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_MatchExpr(&mut self, ctx: &MatchExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code DefCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_DefCallExpr(&mut self, ctx: &DefCallExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LiteralNumber}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn visit_LiteralNumber(&mut self, ctx: &LiteralNumberContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#userString}.
     * @param ctx the parse tree
     */
    fn visit_userString(&mut self, ctx: &UserStringContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigWArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigWArg(&mut self, ctx: &CallSigWArgContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArg(&mut self, ctx: &CallSigNoArgContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgBang}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgBang(&mut self, ctx: &CallSigNoArgBangContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#callSigWithArg}.
     * @param ctx the parse tree
     */
    fn visit_callSigWithArg(&mut self, ctx: &CallSigWithArgContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgDefCallWArg}
     * labeled alternative in {@link BuildingBlocksParser#argExpr}.
     * @param ctx the parse tree
     */
    fn visit_ArgDefCallWArg(&mut self, ctx: &ArgDefCallWArgContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgExprBase}
     * labeled alternative in {@link BuildingBlocksParser#argExpr}.
     * @param ctx the parse tree
     */
    fn visit_ArgExprBase(&mut self, ctx: &ArgExprBaseContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgNormal(
        &mut self,
        ctx: &CallSigNoArgNormalContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code CallSigNoArgBangNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn visit_CallSigNoArgBangNormal(
        &mut self,
        ctx: &CallSigNoArgBangNormalContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NamespacedIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_NamespacedIdent(&mut self, ctx: &NamespacedIdentContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code InstanceIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_InstanceIdent(&mut self, ctx: &InstanceIdentContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code LocalIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn visit_LocalIdent(&mut self, ctx: &LocalIdentContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code FullNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn visit_FullNS(&mut self, ctx: &FullNSContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code RootNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn visit_RootNS(&mut self, ctx: &RootNSContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#keyword}.
     * @param ctx the parse tree
     */
    fn visit_keyword(&mut self, ctx: &KeywordContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code NamedBlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_NamedBlockWDecls(&mut self, ctx: &NamedBlockWDeclsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_BlockWDecls(&mut self, ctx: &BlockWDeclsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockNoDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn visit_BlockNoDecls(&mut self, ctx: &BlockNoDeclsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#blockDecls}.
     * @param ctx the parse tree
     */
    fn visit_blockDecls(&mut self, ctx: &BlockDeclsContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgIgnored}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgIgnored(&mut self, ctx: &BlockArgIgnoredContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgTyped(&mut self, ctx: &BlockArgTypedContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockArgUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn visit_BlockArgUntyped(&mut self, ctx: &BlockArgUntypedContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockDeclTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn visit_BlockDeclTyped(&mut self, ctx: &BlockDeclTypedContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code BlockDeclUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn visit_BlockDeclUntyped(&mut self, ctx: &BlockDeclUntypedContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#string}.
     * @param ctx the parse tree
     */
    fn visit_string(&mut self, ctx: &StringContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgIdentInst}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn visit_ArgIdentInst(&mut self, ctx: &ArgIdentInstContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code ArgIdentNormal}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn visit_ArgIdentNormal(&mut self, ctx: &ArgIdentNormalContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentKeyword}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn visit_IdentKeyword(&mut self, ctx: &IdentKeywordContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by the {@code IdentOther}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn visit_IdentOther(&mut self, ctx: &IdentOtherContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#symbol}.
     * @param ctx the parse tree
     */
    fn visit_symbol(&mut self, ctx: &SymbolContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /**
     * Visit a parse tree produced by {@link BuildingBlocksParser#number}.
     * @param ctx the parse tree
     */
    fn visit_number(&mut self, ctx: &NumberContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }
}

impl<'input, T> BuildingBlocksVisitor<'input> for T
where
    T: BuildingBlocksVisitorCompat<'input>,
{
    fn visit_program(&mut self, ctx: &ProgramContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_program(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MethodReturn(&mut self, ctx: &MethodReturnContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_MethodReturn(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_YieldReturn(&mut self, ctx: &YieldReturnContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_YieldReturn(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockReturn(&mut self, ctx: &BlockReturnContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockReturn(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_AssignmentStmt(&mut self, ctx: &AssignmentStmtContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_AssignmentStmt(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Bang3Stmt(&mut self, ctx: &Bang3StmtContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_Bang3Stmt(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Dot3Stmt(&mut self, ctx: &Dot3StmtContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_Dot3Stmt(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Huh3Stmt(&mut self, ctx: &Huh3StmtContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_Huh3Stmt(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ExprStmt(&mut self, ctx: &ExprStmtContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ExprStmt(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_bang3(&mut self, ctx: &Bang3Context<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_bang3(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_dot3(&mut self, ctx: &Dot3Context<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_dot3(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_huh3(&mut self, ctx: &Huh3Context<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_huh3(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SelectorWArgs(&mut self, ctx: &SelectorWArgsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SelectorWArgs(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SelectorNoArgs(&mut self, ctx: &SelectorNoArgsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SelectorNoArgs(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SelectorNoArgsBang(&mut self, ctx: &SelectorNoArgsBangContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SelectorNoArgsBang(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SelectorSymbol(&mut self, ctx: &SelectorSymbolContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SelectorSymbol(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_assignment(&mut self, ctx: &AssignmentContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_assignment(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IdentLValue(&mut self, ctx: &IdentLValueContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IdentLValue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SplatLValue(&mut self, ctx: &SplatLValueContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SplatLValue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IgnoredLValue(&mut self, ctx: &IgnoredLValueContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IgnoredLValue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IgnoredSplatLValue(&mut self, ctx: &IgnoredSplatLValueContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IgnoredSplatLValue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SubLValue(&mut self, ctx: &SubLValueContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SubLValue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_DefCallWArgExpr(&mut self, ctx: &DefCallWArgExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_DefCallWArgExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ExprCallWArgExpr(&mut self, ctx: &ExprCallWArgExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ExprCallWArgExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_RichExprBase(&mut self, ctx: &RichExprBaseContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_RichExprBase(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MulExpr(&mut self, ctx: &MulExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_MulExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_AndExpr(&mut self, ctx: &AndExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_AndExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LiteralString(&mut self, ctx: &LiteralStringContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LiteralString(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UserStringExpr(&mut self, ctx: &UserStringExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UserStringExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_RegexExpr(&mut self, ctx: &RegexExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_RegexExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_GtExpr(&mut self, ctx: &GtExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_GtExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LtExpr(&mut self, ctx: &LtExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LtExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UserListExpr(&mut self, ctx: &UserListExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UserListExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LtEqExpr(&mut self, ctx: &LtEqExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LtEqExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MethodDefExpr(&mut self, ctx: &MethodDefExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_MethodDefExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LiteralSymbol(&mut self, ctx: &LiteralSymbolContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LiteralSymbol(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ClassDefExpr(&mut self, ctx: &ClassDefExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ClassDefExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ExprCallExpr(&mut self, ctx: &ExprCallExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ExprCallExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SetExpr(&mut self, ctx: &SetExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SetExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UnModExpr(&mut self, ctx: &UnModExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UnModExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MethodExtExpr(&mut self, ctx: &MethodExtExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_MethodExtExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_DictExpr(&mut self, ctx: &DictExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_DictExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ListExpr(&mut self, ctx: &ListExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ListExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IdentExpr(&mut self, ctx: &IdentExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IdentExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SubExpr(&mut self, ctx: &SubExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_SubExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_AddExpr(&mut self, ctx: &AddExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_AddExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ConstDefExpr(&mut self, ctx: &ConstDefExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ConstDefExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_RangeExpr(&mut self, ctx: &RangeExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_RangeExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UnPlusExpr(&mut self, ctx: &UnPlusExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UnPlusExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockExpr(&mut self, ctx: &BlockExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_OrExpr(&mut self, ctx: &OrExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_OrExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ClassDef2Expr(&mut self, ctx: &ClassDef2ExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ClassDef2Expr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_GtEqExpr(&mut self, ctx: &GtEqExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_GtEqExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_DivExpr(&mut self, ctx: &DivExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_DivExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UnBangExpr(&mut self, ctx: &UnBangExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UnBangExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_NotEqExpr(&mut self, ctx: &NotEqExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_NotEqExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_UnMinusExpr(&mut self, ctx: &UnMinusExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_UnMinusExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_EqExpr(&mut self, ctx: &EqExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_EqExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ClassExtExpr(&mut self, ctx: &ClassExtExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ClassExtExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_NestedExpr(&mut self, ctx: &NestedExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_NestedExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ModExpr(&mut self, ctx: &ModExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ModExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MatchExpr(&mut self, ctx: &MatchExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_MatchExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_DefCallExpr(&mut self, ctx: &DefCallExprContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_DefCallExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LiteralNumber(&mut self, ctx: &LiteralNumberContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LiteralNumber(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_userString(&mut self, ctx: &UserStringContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_userString(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CallSigWArg(&mut self, ctx: &CallSigWArgContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_CallSigWArg(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CallSigNoArg(&mut self, ctx: &CallSigNoArgContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_CallSigNoArg(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CallSigNoArgBang(&mut self, ctx: &CallSigNoArgBangContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_CallSigNoArgBang(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_callSigWithArg(&mut self, ctx: &CallSigWithArgContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_callSigWithArg(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ArgDefCallWArg(&mut self, ctx: &ArgDefCallWArgContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ArgDefCallWArg(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ArgExprBase(&mut self, ctx: &ArgExprBaseContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ArgExprBase(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CallSigNoArgNormal(&mut self, ctx: &CallSigNoArgNormalContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_CallSigNoArgNormal(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CallSigNoArgBangNormal(&mut self, ctx: &CallSigNoArgBangNormalContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_CallSigNoArgBangNormal(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_NamespacedIdent(&mut self, ctx: &NamespacedIdentContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_NamespacedIdent(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_InstanceIdent(&mut self, ctx: &InstanceIdentContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_InstanceIdent(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LocalIdent(&mut self, ctx: &LocalIdentContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_LocalIdent(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_FullNS(&mut self, ctx: &FullNSContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_FullNS(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_RootNS(&mut self, ctx: &RootNSContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_RootNS(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_keyword(&mut self, ctx: &KeywordContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_keyword(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_NamedBlockWDecls(&mut self, ctx: &NamedBlockWDeclsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_NamedBlockWDecls(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockWDecls(&mut self, ctx: &BlockWDeclsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockWDecls(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockNoDecls(&mut self, ctx: &BlockNoDeclsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockNoDecls(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_blockDecls(&mut self, ctx: &BlockDeclsContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_blockDecls(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockArgIgnored(&mut self, ctx: &BlockArgIgnoredContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockArgIgnored(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockArgTyped(&mut self, ctx: &BlockArgTypedContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockArgTyped(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockArgUntyped(&mut self, ctx: &BlockArgUntypedContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockArgUntyped(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockDeclTyped(&mut self, ctx: &BlockDeclTypedContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockDeclTyped(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BlockDeclUntyped(&mut self, ctx: &BlockDeclUntypedContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_BlockDeclUntyped(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_string(&mut self, ctx: &StringContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_string(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ArgIdentInst(&mut self, ctx: &ArgIdentInstContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ArgIdentInst(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ArgIdentNormal(&mut self, ctx: &ArgIdentNormalContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_ArgIdentNormal(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IdentKeyword(&mut self, ctx: &IdentKeywordContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IdentKeyword(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_IdentOther(&mut self, ctx: &IdentOtherContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_IdentOther(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_symbol(&mut self, ctx: &SymbolContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_symbol(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_number(&mut self, ctx: &NumberContext<'input>) {
        let result = <Self as BuildingBlocksVisitorCompat>::visit_number(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }
}
