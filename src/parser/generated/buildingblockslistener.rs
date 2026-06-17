#![allow(nonstandard_style)]
// Generated from ./BuildingBlocks.g4 by ANTLR 4.8
use super::buildingblocksparser::*;
use antlr_rust::tree::ParseTreeListener;

pub trait BuildingBlocksListener<'input>:
    ParseTreeListener<'input, BuildingBlocksParserContextType>
{
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#program}.
     * @param ctx the parse tree
     */
    fn enter_program(&mut self, _ctx: &ProgramContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#program}.
     * @param ctx the parse tree
     */
    fn exit_program(&mut self, _ctx: &ProgramContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code MethodReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_MethodReturn(&mut self, _ctx: &MethodReturnContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code MethodReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_MethodReturn(&mut self, _ctx: &MethodReturnContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code YieldReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_YieldReturn(&mut self, _ctx: &YieldReturnContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code YieldReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_YieldReturn(&mut self, _ctx: &YieldReturnContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_BlockReturn(&mut self, _ctx: &BlockReturnContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockReturn}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_BlockReturn(&mut self, _ctx: &BlockReturnContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code AssignmentStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_AssignmentStmt(&mut self, _ctx: &AssignmentStmtContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code AssignmentStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_AssignmentStmt(&mut self, _ctx: &AssignmentStmtContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code Bang3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_Bang3Stmt(&mut self, _ctx: &Bang3StmtContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code Bang3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_Bang3Stmt(&mut self, _ctx: &Bang3StmtContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code Dot3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_Dot3Stmt(&mut self, _ctx: &Dot3StmtContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code Dot3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_Dot3Stmt(&mut self, _ctx: &Dot3StmtContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code Huh3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_Huh3Stmt(&mut self, _ctx: &Huh3StmtContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code Huh3Stmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_Huh3Stmt(&mut self, _ctx: &Huh3StmtContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ExprStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn enter_ExprStmt(&mut self, _ctx: &ExprStmtContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ExprStmt}
     * labeled alternative in {@link BuildingBlocksParser#stmt}.
     * @param ctx the parse tree
     */
    fn exit_ExprStmt(&mut self, _ctx: &ExprStmtContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#bang3}.
     * @param ctx the parse tree
     */
    fn enter_bang3(&mut self, _ctx: &Bang3Context<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#bang3}.
     * @param ctx the parse tree
     */
    fn exit_bang3(&mut self, _ctx: &Bang3Context<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#dot3}.
     * @param ctx the parse tree
     */
    fn enter_dot3(&mut self, _ctx: &Dot3Context<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#dot3}.
     * @param ctx the parse tree
     */
    fn exit_dot3(&mut self, _ctx: &Dot3Context<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#huh3}.
     * @param ctx the parse tree
     */
    fn enter_huh3(&mut self, _ctx: &Huh3Context<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#huh3}.
     * @param ctx the parse tree
     */
    fn exit_huh3(&mut self, _ctx: &Huh3Context<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SelectorWArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn enter_SelectorWArgs(&mut self, _ctx: &SelectorWArgsContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SelectorWArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn exit_SelectorWArgs(&mut self, _ctx: &SelectorWArgsContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SelectorNoArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn enter_SelectorNoArgs(&mut self, _ctx: &SelectorNoArgsContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SelectorNoArgs}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn exit_SelectorNoArgs(&mut self, _ctx: &SelectorNoArgsContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SelectorNoArgsBang}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn enter_SelectorNoArgsBang(&mut self, _ctx: &SelectorNoArgsBangContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SelectorNoArgsBang}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn exit_SelectorNoArgsBang(&mut self, _ctx: &SelectorNoArgsBangContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SelectorSymbol}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn enter_SelectorSymbol(&mut self, _ctx: &SelectorSymbolContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SelectorSymbol}
     * labeled alternative in {@link BuildingBlocksParser#selector}.
     * @param ctx the parse tree
     */
    fn exit_SelectorSymbol(&mut self, _ctx: &SelectorSymbolContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#assignment}.
     * @param ctx the parse tree
     */
    fn enter_assignment(&mut self, _ctx: &AssignmentContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#assignment}.
     * @param ctx the parse tree
     */
    fn exit_assignment(&mut self, _ctx: &AssignmentContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IdentLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn enter_IdentLValue(&mut self, _ctx: &IdentLValueContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IdentLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn exit_IdentLValue(&mut self, _ctx: &IdentLValueContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn enter_SplatLValue(&mut self, _ctx: &SplatLValueContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn exit_SplatLValue(&mut self, _ctx: &SplatLValueContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IgnoredLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn enter_IgnoredLValue(&mut self, _ctx: &IgnoredLValueContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IgnoredLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn exit_IgnoredLValue(&mut self, _ctx: &IgnoredLValueContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IgnoredSplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn enter_IgnoredSplatLValue(&mut self, _ctx: &IgnoredSplatLValueContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IgnoredSplatLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn exit_IgnoredSplatLValue(&mut self, _ctx: &IgnoredSplatLValueContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SubLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn enter_SubLValue(&mut self, _ctx: &SubLValueContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SubLValue}
     * labeled alternative in {@link BuildingBlocksParser#lvalue}.
     * @param ctx the parse tree
     */
    fn exit_SubLValue(&mut self, _ctx: &SubLValueContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code DefCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn enter_DefCallWArgExpr(&mut self, _ctx: &DefCallWArgExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code DefCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn exit_DefCallWArgExpr(&mut self, _ctx: &DefCallWArgExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ExprCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn enter_ExprCallWArgExpr(&mut self, _ctx: &ExprCallWArgExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ExprCallWArgExpr}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn exit_ExprCallWArgExpr(&mut self, _ctx: &ExprCallWArgExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code RichExprBase}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn enter_RichExprBase(&mut self, _ctx: &RichExprBaseContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code RichExprBase}
     * labeled alternative in {@link BuildingBlocksParser#richExpr}.
     * @param ctx the parse tree
     */
    fn exit_RichExprBase(&mut self, _ctx: &RichExprBaseContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code MulExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_MulExpr(&mut self, _ctx: &MulExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code MulExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_MulExpr(&mut self, _ctx: &MulExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code AndExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_AndExpr(&mut self, _ctx: &AndExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code AndExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_AndExpr(&mut self, _ctx: &AndExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LiteralString}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_LiteralString(&mut self, _ctx: &LiteralStringContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LiteralString}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_LiteralString(&mut self, _ctx: &LiteralStringContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UserStringExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UserStringExpr(&mut self, _ctx: &UserStringExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UserStringExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UserStringExpr(&mut self, _ctx: &UserStringExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code RegexExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_RegexExpr(&mut self, _ctx: &RegexExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code RegexExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_RegexExpr(&mut self, _ctx: &RegexExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code GtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_GtExpr(&mut self, _ctx: &GtExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code GtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_GtExpr(&mut self, _ctx: &GtExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_LtExpr(&mut self, _ctx: &LtExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_LtExpr(&mut self, _ctx: &LtExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UserListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UserListExpr(&mut self, _ctx: &UserListExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UserListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UserListExpr(&mut self, _ctx: &UserListExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_LtEqExpr(&mut self, _ctx: &LtEqExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_LtEqExpr(&mut self, _ctx: &LtEqExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code MethodDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_MethodDefExpr(&mut self, _ctx: &MethodDefExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code MethodDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_MethodDefExpr(&mut self, _ctx: &MethodDefExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LiteralSymbol}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_LiteralSymbol(&mut self, _ctx: &LiteralSymbolContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LiteralSymbol}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_LiteralSymbol(&mut self, _ctx: &LiteralSymbolContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ClassDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ClassDefExpr(&mut self, _ctx: &ClassDefExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ClassDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ClassDefExpr(&mut self, _ctx: &ClassDefExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ExprCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ExprCallExpr(&mut self, _ctx: &ExprCallExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ExprCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ExprCallExpr(&mut self, _ctx: &ExprCallExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SetExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_SetExpr(&mut self, _ctx: &SetExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SetExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_SetExpr(&mut self, _ctx: &SetExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UnModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UnModExpr(&mut self, _ctx: &UnModExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UnModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UnModExpr(&mut self, _ctx: &UnModExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code MethodExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_MethodExtExpr(&mut self, _ctx: &MethodExtExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code MethodExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_MethodExtExpr(&mut self, _ctx: &MethodExtExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code DictExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_DictExpr(&mut self, _ctx: &DictExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code DictExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_DictExpr(&mut self, _ctx: &DictExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ListExpr(&mut self, _ctx: &ListExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ListExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ListExpr(&mut self, _ctx: &ListExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IdentExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_IdentExpr(&mut self, _ctx: &IdentExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IdentExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_IdentExpr(&mut self, _ctx: &IdentExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code SubExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_SubExpr(&mut self, _ctx: &SubExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code SubExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_SubExpr(&mut self, _ctx: &SubExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code AddExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_AddExpr(&mut self, _ctx: &AddExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code AddExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_AddExpr(&mut self, _ctx: &AddExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ConstDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ConstDefExpr(&mut self, _ctx: &ConstDefExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ConstDefExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ConstDefExpr(&mut self, _ctx: &ConstDefExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code RangeExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_RangeExpr(&mut self, _ctx: &RangeExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code RangeExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_RangeExpr(&mut self, _ctx: &RangeExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UnPlusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UnPlusExpr(&mut self, _ctx: &UnPlusExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UnPlusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UnPlusExpr(&mut self, _ctx: &UnPlusExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_BlockExpr(&mut self, _ctx: &BlockExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_BlockExpr(&mut self, _ctx: &BlockExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code OrExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_OrExpr(&mut self, _ctx: &OrExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code OrExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_OrExpr(&mut self, _ctx: &OrExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ClassDef2Expr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ClassDef2Expr(&mut self, _ctx: &ClassDef2ExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ClassDef2Expr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ClassDef2Expr(&mut self, _ctx: &ClassDef2ExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code GtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_GtEqExpr(&mut self, _ctx: &GtEqExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code GtEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_GtEqExpr(&mut self, _ctx: &GtEqExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code DivExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_DivExpr(&mut self, _ctx: &DivExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code DivExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_DivExpr(&mut self, _ctx: &DivExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UnBangExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UnBangExpr(&mut self, _ctx: &UnBangExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UnBangExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UnBangExpr(&mut self, _ctx: &UnBangExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code NotEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_NotEqExpr(&mut self, _ctx: &NotEqExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code NotEqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_NotEqExpr(&mut self, _ctx: &NotEqExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code UnMinusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_UnMinusExpr(&mut self, _ctx: &UnMinusExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code UnMinusExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_UnMinusExpr(&mut self, _ctx: &UnMinusExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code EqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_EqExpr(&mut self, _ctx: &EqExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code EqExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_EqExpr(&mut self, _ctx: &EqExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ClassExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ClassExtExpr(&mut self, _ctx: &ClassExtExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ClassExtExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ClassExtExpr(&mut self, _ctx: &ClassExtExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code NestedExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_NestedExpr(&mut self, _ctx: &NestedExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code NestedExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_NestedExpr(&mut self, _ctx: &NestedExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_ModExpr(&mut self, _ctx: &ModExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ModExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_ModExpr(&mut self, _ctx: &ModExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code MatchExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_MatchExpr(&mut self, _ctx: &MatchExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code MatchExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_MatchExpr(&mut self, _ctx: &MatchExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code DefCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_DefCallExpr(&mut self, _ctx: &DefCallExprContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code DefCallExpr}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_DefCallExpr(&mut self, _ctx: &DefCallExprContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LiteralNumber}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn enter_LiteralNumber(&mut self, _ctx: &LiteralNumberContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LiteralNumber}
     * labeled alternative in {@link BuildingBlocksParser#expr}.
     * @param ctx the parse tree
     */
    fn exit_LiteralNumber(&mut self, _ctx: &LiteralNumberContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#userString}.
     * @param ctx the parse tree
     */
    fn enter_userString(&mut self, _ctx: &UserStringContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#userString}.
     * @param ctx the parse tree
     */
    fn exit_userString(&mut self, _ctx: &UserStringContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code CallSigWArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn enter_CallSigWArg(&mut self, _ctx: &CallSigWArgContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code CallSigWArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn exit_CallSigWArg(&mut self, _ctx: &CallSigWArgContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code CallSigNoArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn enter_CallSigNoArg(&mut self, _ctx: &CallSigNoArgContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code CallSigNoArg}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn exit_CallSigNoArg(&mut self, _ctx: &CallSigNoArgContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code CallSigNoArgBang}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn enter_CallSigNoArgBang(&mut self, _ctx: &CallSigNoArgBangContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code CallSigNoArgBang}
     * labeled alternative in {@link BuildingBlocksParser#callSig}.
     * @param ctx the parse tree
     */
    fn exit_CallSigNoArgBang(&mut self, _ctx: &CallSigNoArgBangContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#callSigWithArg}.
     * @param ctx the parse tree
     */
    fn enter_callSigWithArg(&mut self, _ctx: &CallSigWithArgContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#callSigWithArg}.
     * @param ctx the parse tree
     */
    fn exit_callSigWithArg(&mut self, _ctx: &CallSigWithArgContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code CallSigNoArgNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn enter_CallSigNoArgNormal(&mut self, _ctx: &CallSigNoArgNormalContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code CallSigNoArgNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn exit_CallSigNoArgNormal(&mut self, _ctx: &CallSigNoArgNormalContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code CallSigNoArgBangNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn enter_CallSigNoArgBangNormal(&mut self, _ctx: &CallSigNoArgBangNormalContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code CallSigNoArgBangNormal}
     * labeled alternative in {@link BuildingBlocksParser#callSigNoArgOrBang}.
     * @param ctx the parse tree
     */
    fn exit_CallSigNoArgBangNormal(&mut self, _ctx: &CallSigNoArgBangNormalContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code NamespacedIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn enter_NamespacedIdent(&mut self, _ctx: &NamespacedIdentContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code NamespacedIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn exit_NamespacedIdent(&mut self, _ctx: &NamespacedIdentContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code InstanceIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn enter_InstanceIdent(&mut self, _ctx: &InstanceIdentContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code InstanceIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn exit_InstanceIdent(&mut self, _ctx: &InstanceIdentContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code LocalIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn enter_LocalIdent(&mut self, _ctx: &LocalIdentContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code LocalIdent}
     * labeled alternative in {@link BuildingBlocksParser#nsvarident}.
     * @param ctx the parse tree
     */
    fn exit_LocalIdent(&mut self, _ctx: &LocalIdentContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code FullNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn enter_FullNS(&mut self, _ctx: &FullNSContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code FullNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn exit_FullNS(&mut self, _ctx: &FullNSContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code RootNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn enter_RootNS(&mut self, _ctx: &RootNSContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code RootNS}
     * labeled alternative in {@link BuildingBlocksParser#namespace}.
     * @param ctx the parse tree
     */
    fn exit_RootNS(&mut self, _ctx: &RootNSContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#keyword}.
     * @param ctx the parse tree
     */
    fn enter_keyword(&mut self, _ctx: &KeywordContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#keyword}.
     * @param ctx the parse tree
     */
    fn exit_keyword(&mut self, _ctx: &KeywordContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code NamedBlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn enter_NamedBlockWDecls(&mut self, _ctx: &NamedBlockWDeclsContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code NamedBlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn exit_NamedBlockWDecls(&mut self, _ctx: &NamedBlockWDeclsContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn enter_BlockWDecls(&mut self, _ctx: &BlockWDeclsContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockWDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn exit_BlockWDecls(&mut self, _ctx: &BlockWDeclsContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockNoDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn enter_BlockNoDecls(&mut self, _ctx: &BlockNoDeclsContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockNoDecls}
     * labeled alternative in {@link BuildingBlocksParser#block}.
     * @param ctx the parse tree
     */
    fn exit_BlockNoDecls(&mut self, _ctx: &BlockNoDeclsContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#blockDecls}.
     * @param ctx the parse tree
     */
    fn enter_blockDecls(&mut self, _ctx: &BlockDeclsContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#blockDecls}.
     * @param ctx the parse tree
     */
    fn exit_blockDecls(&mut self, _ctx: &BlockDeclsContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockArgIgnored}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn enter_BlockArgIgnored(&mut self, _ctx: &BlockArgIgnoredContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockArgIgnored}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn exit_BlockArgIgnored(&mut self, _ctx: &BlockArgIgnoredContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockArgTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn enter_BlockArgTyped(&mut self, _ctx: &BlockArgTypedContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockArgTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn exit_BlockArgTyped(&mut self, _ctx: &BlockArgTypedContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockArgUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn enter_BlockArgUntyped(&mut self, _ctx: &BlockArgUntypedContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockArgUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockArg}.
     * @param ctx the parse tree
     */
    fn exit_BlockArgUntyped(&mut self, _ctx: &BlockArgUntypedContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockDeclTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn enter_BlockDeclTyped(&mut self, _ctx: &BlockDeclTypedContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockDeclTyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn exit_BlockDeclTyped(&mut self, _ctx: &BlockDeclTypedContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code BlockDeclUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn enter_BlockDeclUntyped(&mut self, _ctx: &BlockDeclUntypedContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code BlockDeclUntyped}
     * labeled alternative in {@link BuildingBlocksParser#blockDecl}.
     * @param ctx the parse tree
     */
    fn exit_BlockDeclUntyped(&mut self, _ctx: &BlockDeclUntypedContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#string}.
     * @param ctx the parse tree
     */
    fn enter_string(&mut self, _ctx: &StringContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#string}.
     * @param ctx the parse tree
     */
    fn exit_string(&mut self, _ctx: &StringContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ArgIdentInst}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn enter_ArgIdentInst(&mut self, _ctx: &ArgIdentInstContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ArgIdentInst}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn exit_ArgIdentInst(&mut self, _ctx: &ArgIdentInstContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code ArgIdentNormal}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn enter_ArgIdentNormal(&mut self, _ctx: &ArgIdentNormalContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code ArgIdentNormal}
     * labeled alternative in {@link BuildingBlocksParser#argIdent}.
     * @param ctx the parse tree
     */
    fn exit_ArgIdentNormal(&mut self, _ctx: &ArgIdentNormalContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IdentKeyword}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn enter_IdentKeyword(&mut self, _ctx: &IdentKeywordContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IdentKeyword}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn exit_IdentKeyword(&mut self, _ctx: &IdentKeywordContext<'input>) {}
    /**
     * Enter a parse tree produced by the {@code IdentOther}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn enter_IdentOther(&mut self, _ctx: &IdentOtherContext<'input>) {}
    /**
     * Exit a parse tree produced by the {@code IdentOther}
     * labeled alternative in {@link BuildingBlocksParser#ident}.
     * @param ctx the parse tree
     */
    fn exit_IdentOther(&mut self, _ctx: &IdentOtherContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#symbol}.
     * @param ctx the parse tree
     */
    fn enter_symbol(&mut self, _ctx: &SymbolContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#symbol}.
     * @param ctx the parse tree
     */
    fn exit_symbol(&mut self, _ctx: &SymbolContext<'input>) {}
    /**
     * Enter a parse tree produced by {@link BuildingBlocksParser#number}.
     * @param ctx the parse tree
     */
    fn enter_number(&mut self, _ctx: &NumberContext<'input>) {}
    /**
     * Exit a parse tree produced by {@link BuildingBlocksParser#number}.
     * @param ctx the parse tree
     */
    fn exit_number(&mut self, _ctx: &NumberContext<'input>) {}
}

antlr_rust::coerce_from! { 'input : BuildingBlocksListener<'input> }
