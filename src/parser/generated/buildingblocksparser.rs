// Generated from .\BuildingBlocks.g4 by ANTLR 4.8
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(nonstandard_style)]
#![allow(unused_imports)]
#![allow(unused_mut)]
#![allow(unused_braces)]
use super::buildingblockslistener::*;
use super::buildingblocksvisitor::*;
use antlr_rust::PredictionContextCache;
use antlr_rust::TokenSource;
use antlr_rust::atn::{ATN, INVALID_ALT};
use antlr_rust::atn_deserializer::ATNDeserializer;
use antlr_rust::dfa::DFA;
use antlr_rust::error_strategy::{DefaultErrorStrategy, ErrorStrategy};
use antlr_rust::errors::*;
use antlr_rust::int_stream::EOF;
use antlr_rust::parser::{BaseParser, Parser, ParserNodeType, ParserRecog};
use antlr_rust::parser_atn_simulator::ParserATNSimulator;
use antlr_rust::parser_rule_context::{BaseParserRuleContext, ParserRuleContext, cast, cast_mut};
use antlr_rust::recognizer::{Actions, Recognizer};
use antlr_rust::rule_context::{BaseRuleContext, CustomRuleContext, RuleContext};
use antlr_rust::token::{OwningToken, TOKEN_EOF, Token};
use antlr_rust::token_factory::{CommonTokenFactory, TokenAware, TokenFactory};
use antlr_rust::token_stream::TokenStream;
use antlr_rust::tree::*;
use antlr_rust::vocabulary::{Vocabulary, VocabularyImpl};

use antlr_rust::lazy_static;
use antlr_rust::{TidAble, TidExt};

use std::any::{Any, TypeId};
use std::borrow::{Borrow, BorrowMut};
use std::cell::RefCell;
use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::Arc;

pub const T__0: isize = 1;
pub const T__1: isize = 2;
pub const T__2: isize = 3;
pub const T__3: isize = 4;
pub const T__4: isize = 5;
pub const T__5: isize = 6;
pub const T__6: isize = 7;
pub const T__7: isize = 8;
pub const T__8: isize = 9;
pub const T__9: isize = 10;
pub const T__10: isize = 11;
pub const T__11: isize = 12;
pub const T__12: isize = 13;
pub const T__13: isize = 14;
pub const T__14: isize = 15;
pub const T__15: isize = 16;
pub const T__16: isize = 17;
pub const T__17: isize = 18;
pub const T__18: isize = 19;
pub const T__19: isize = 20;
pub const T__20: isize = 21;
pub const T__21: isize = 22;
pub const T__22: isize = 23;
pub const T__23: isize = 24;
pub const T__24: isize = 25;
pub const T__25: isize = 26;
pub const T__26: isize = 27;
pub const T__27: isize = 28;
pub const T__28: isize = 29;
pub const T__29: isize = 30;
pub const T__30: isize = 31;
pub const T__31: isize = 32;
pub const T__32: isize = 33;
pub const T__33: isize = 34;
pub const T__34: isize = 35;
pub const T__35: isize = 36;
pub const T__36: isize = 37;
pub const T__37: isize = 38;
pub const WS: isize = 39;
pub const IDENT: isize = 40;
pub const USER_LIST_START: isize = 41;
pub const SYMBOL: isize = 42;
pub const STRING: isize = 43;
pub const REGEXP: isize = 44;
pub const USER_STRING: isize = 45;
pub const EOL_COMMENT: isize = 46;
pub const METHOD_RETURN: isize = 47;
pub const YIELD_RETURN: isize = 48;
pub const BLOCK_RETURN: isize = 49;
pub const EMPTY_COMMENT: isize = 50;
pub const COMMENT: isize = 51;
pub const NUMBER: isize = 52;
pub const RULE_program: usize = 0;
pub const RULE_stmt: usize = 1;
pub const RULE_bang3: usize = 2;
pub const RULE_dot3: usize = 3;
pub const RULE_huh3: usize = 4;
pub const RULE_selector: usize = 5;
pub const RULE_assignment: usize = 6;
pub const RULE_lvalue: usize = 7;
pub const RULE_expr: usize = 8;
pub const RULE_userString: usize = 9;
pub const RULE_callSig: usize = 10;
pub const RULE_nsvarident: usize = 11;
pub const RULE_namespace: usize = 12;
pub const RULE_keyword: usize = 13;
pub const RULE_block: usize = 14;
pub const RULE_blockDecls: usize = 15;
pub const RULE_blockArg: usize = 16;
pub const RULE_blockDecl: usize = 17;
pub const RULE_string: usize = 18;
pub const RULE_argident: usize = 19;
pub const RULE_ident: usize = 20;
pub const RULE_symbol: usize = 21;
pub const RULE_number: usize = 22;
pub const ruleNames: [&'static str; 23] = [
    "program",
    "stmt",
    "bang3",
    "dot3",
    "huh3",
    "selector",
    "assignment",
    "lvalue",
    "expr",
    "userString",
    "callSig",
    "nsvarident",
    "namespace",
    "keyword",
    "block",
    "blockDecls",
    "blockArg",
    "blockDecl",
    "string",
    "argident",
    "ident",
    "symbol",
    "number",
];

pub const _LITERAL_NAMES: [Option<&'static str>; 50] = [
    None,
    Some("';'"),
    Some("'!!!'"),
    Some("'...'"),
    Some("'???'"),
    Some("'+'"),
    Some("':'"),
    Some("'!'"),
    Some("'='"),
    Some("'*'"),
    Some("'_'"),
    Some("'('"),
    Some("')'"),
    Some("'-'"),
    Some("'%'"),
    Some("'<-'"),
    Some("'<--'"),
    Some("'->'"),
    Some("'-->'"),
    Some("'..'"),
    Some("'.'"),
    Some("'/'"),
    Some("'~'"),
    Some("'>'"),
    Some("'<'"),
    Some("'&&'"),
    Some("'||'"),
    Some("'=='"),
    Some("'!='"),
    Some("'#'"),
    Some("'{'"),
    Some("'}'"),
    Some("'@'"),
    Some("'['"),
    Some("']'"),
    Some("'nil'"),
    Some("'true'"),
    Some("'false'"),
    Some("'|'"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("'^^'"),
    Some("'^>'"),
    Some("'^'"),
];
pub const _SYMBOLIC_NAMES: [Option<&'static str>; 53] = [
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("WS"),
    Some("IDENT"),
    Some("USER_LIST_START"),
    Some("SYMBOL"),
    Some("STRING"),
    Some("REGEXP"),
    Some("USER_STRING"),
    Some("EOL_COMMENT"),
    Some("METHOD_RETURN"),
    Some("YIELD_RETURN"),
    Some("BLOCK_RETURN"),
    Some("EMPTY_COMMENT"),
    Some("COMMENT"),
    Some("NUMBER"),
];
lazy_static! {
    static ref _shared_context_cache: Arc<PredictionContextCache> =
        Arc::new(PredictionContextCache::new());
    static ref VOCABULARY: Box<dyn Vocabulary> = Box::new(VocabularyImpl::new(
        _LITERAL_NAMES.iter(),
        _SYMBOLIC_NAMES.iter(),
        None
    ));
}

type BaseParserType<'input, I> = BaseParser<
    'input,
    BuildingBlocksParserExt<'input>,
    I,
    BuildingBlocksParserContextType,
    dyn BuildingBlocksListener<'input> + 'input,
>;

type TokenType<'input> = <LocalTokenFactory<'input> as TokenFactory<'input>>::Tok;
pub type LocalTokenFactory<'input> = CommonTokenFactory;

pub type BuildingBlocksTreeWalker<'input, 'a> = ParseTreeWalker<
    'input,
    'a,
    BuildingBlocksParserContextType,
    dyn BuildingBlocksListener<'input> + 'a,
>;

/// Parser for BuildingBlocks grammar
pub struct BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    base: BaseParserType<'input, I>,
    interpreter: Arc<ParserATNSimulator>,
    _shared_context_cache: Box<PredictionContextCache>,
    pub err_handler: H,
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn get_serialized_atn() -> &'static str {
        _serializedATN
    }

    pub fn set_error_strategy(&mut self, strategy: H) {
        self.err_handler = strategy
    }

    pub fn with_strategy(input: I, strategy: H) -> Self {
        antlr_rust::recognizer::check_version("0", "3");
        let interpreter = Arc::new(ParserATNSimulator::new(
            _ATN.clone(),
            _decision_to_DFA.clone(),
            _shared_context_cache.clone(),
        ));
        Self {
            base: BaseParser::new_base_parser(
                input,
                Arc::clone(&interpreter),
                BuildingBlocksParserExt {
                    _pd: Default::default(),
                },
            ),
            interpreter,
            _shared_context_cache: Box::new(PredictionContextCache::new()),
            err_handler: strategy,
        }
    }
}

type DynStrategy<'input, I> = Box<dyn ErrorStrategy<'input, BaseParserType<'input, I>> + 'input>;

impl<'input, I> BuildingBlocksParser<'input, I, DynStrategy<'input, I>>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn with_dyn_strategy(input: I) -> Self {
        Self::with_strategy(input, Box::new(DefaultErrorStrategy::new()))
    }
}

impl<'input, I>
    BuildingBlocksParser<'input, I, DefaultErrorStrategy<'input, BuildingBlocksParserContextType>>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn new(input: I) -> Self {
        Self::with_strategy(input, DefaultErrorStrategy::new())
    }
}

/// Trait for monomorphized trait object that corresponds to the nodes of parse tree generated for BuildingBlocksParser
pub trait BuildingBlocksParserContext<'input>: for<'x> Listenable<dyn BuildingBlocksListener<'input> + 'x>
    + for<'x> Visitable<dyn BuildingBlocksVisitor<'input> + 'x>
    + ParserRuleContext<'input, TF = LocalTokenFactory<'input>, Ctx = BuildingBlocksParserContextType>
{
}

antlr_rust::coerce_from! { 'input : BuildingBlocksParserContext<'input> }

impl<'input, 'x, T> VisitableDyn<T> for dyn BuildingBlocksParserContext<'input> + 'input
where
    T: BuildingBlocksVisitor<'input> + 'x,
{
    fn accept_dyn(&self, visitor: &mut T) {
        self.accept(visitor as &mut (dyn BuildingBlocksVisitor<'input> + 'x))
    }
}

impl<'input> BuildingBlocksParserContext<'input>
    for TerminalNode<'input, BuildingBlocksParserContextType>
{
}
impl<'input> BuildingBlocksParserContext<'input>
    for ErrorNode<'input, BuildingBlocksParserContextType>
{
}

antlr_rust::tid! { impl<'input> TidAble<'input> for dyn BuildingBlocksParserContext<'input> + 'input }

antlr_rust::tid! { impl<'input> TidAble<'input> for dyn BuildingBlocksListener<'input> + 'input }

pub struct BuildingBlocksParserContextType;
antlr_rust::tid! {BuildingBlocksParserContextType}

impl<'input> ParserNodeType<'input> for BuildingBlocksParserContextType {
    type TF = LocalTokenFactory<'input>;
    type Type = dyn BuildingBlocksParserContext<'input> + 'input;
}

impl<'input, I, H> Deref for BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    type Target = BaseParserType<'input, I>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<'input, I, H> DerefMut for BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct BuildingBlocksParserExt<'input> {
    _pd: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserExt<'input> {}
antlr_rust::tid! { BuildingBlocksParserExt<'a> }

impl<'input> TokenAware<'input> for BuildingBlocksParserExt<'input> {
    type TF = LocalTokenFactory<'input>;
}

impl<'input, I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>>
    ParserRecog<'input, BaseParserType<'input, I>> for BuildingBlocksParserExt<'input>
{
}

impl<'input, I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>>
    Actions<'input, BaseParserType<'input, I>> for BuildingBlocksParserExt<'input>
{
    fn get_grammar_file_name(&self) -> &str {
        "BuildingBlocks.g4"
    }

    fn get_rule_names(&self) -> &[&str] {
        &ruleNames
    }

    fn get_vocabulary(&self) -> &dyn Vocabulary {
        &**VOCABULARY
    }
    fn sempred(
        _localctx: Option<&(dyn BuildingBlocksParserContext<'input> + 'input)>,
        rule_index: isize,
        pred_index: isize,
        recog: &mut BaseParserType<'input, I>,
    ) -> bool {
        match rule_index {
            8 => BuildingBlocksParser::<'input, I, _>::expr_sempred(
                _localctx.and_then(|x| x.downcast_ref()),
                pred_index,
                recog,
            ),
            _ => true,
        }
    }
}

impl<'input, I>
    BuildingBlocksParser<'input, I, DefaultErrorStrategy<'input, BuildingBlocksParserContextType>>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    fn expr_sempred(
        _localctx: Option<&ExprContext<'input>>,
        pred_index: isize,
        recog: &mut <Self as Deref>::Target,
    ) -> bool {
        match pred_index {
            0 => recog.precpred(None, 28),
            1 => recog.precpred(None, 25),
            2 => recog.precpred(None, 24),
            3 => recog.precpred(None, 23),
            4 => recog.precpred(None, 22),
            5 => recog.precpred(None, 21),
            6 => recog.precpred(None, 20),
            7 => recog.precpred(None, 19),
            8 => recog.precpred(None, 18),
            9 => recog.precpred(None, 17),
            10 => recog.precpred(None, 16),
            11 => recog.precpred(None, 15),
            12 => recog.precpred(None, 14),
            13 => recog.precpred(None, 13),
            14 => recog.precpred(None, 12),
            15 => recog.precpred(None, 31),
            16 => recog.precpred(None, 26),
            _ => true,
        }
    }
}
//------------------- program ----------------
pub type ProgramContextAll<'input> = ProgramContext<'input>;

pub type ProgramContext<'input> = BaseParserRuleContext<'input, ProgramContextExt<'input>>;

#[derive(Clone)]
pub struct ProgramContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for ProgramContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ProgramContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_program(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_program(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ProgramContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_program(self);
    }
}

impl<'input> CustomRuleContext<'input> for ProgramContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_program
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_program }
}
antlr_rust::tid! {ProgramContextExt<'a>}

impl<'input> ProgramContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<ProgramContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ProgramContextExt { ph: PhantomData },
        ))
    }
}

pub trait ProgramContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<ProgramContextExt<'input>>
{
    fn stmt_all(&self) -> Vec<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn stmt(&self, i: usize) -> Option<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> ProgramContextAttrs<'input> for ProgramContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn program(&mut self) -> Result<Rc<ProgramContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = ProgramContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 0, RULE_program);
        let mut _localctx: Rc<ProgramContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(50);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                loop {
                    {
                        {
                            /*InvokeRule stmt*/
                            recog.base.set_state(46);
                            recog.stmt()?;

                            recog.base.set_state(48);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            if _la == T__0 {
                                {
                                    recog.base.set_state(47);
                                    recog.base.match_token(T__0, &mut recog.err_handler)?;
                                }
                            }
                        }
                    }
                    recog.base.set_state(52);
                    recog.err_handler.sync(&mut recog.base)?;
                    _la = recog.base.input.la(1);
                    if !((((_la) & !0x3f) == 0
                        && ((1usize << _la)
                            & ((1usize << T__1)
                                | (1usize << T__2)
                                | (1usize << T__3)
                                | (1usize << T__4)
                                | (1usize << T__6)
                                | (1usize << T__8)
                                | (1usize << T__9)
                                | (1usize << T__10)
                                | (1usize << T__12)
                                | (1usize << T__13)
                                | (1usize << T__19)
                                | (1usize << T__28)
                                | (1usize << T__29)))
                            != 0)
                        || (((_la - 32) & !0x3f) == 0
                            && ((1usize << (_la - 32))
                                & ((1usize << (T__31 - 32))
                                    | (1usize << (T__32 - 32))
                                    | (1usize << (T__34 - 32))
                                    | (1usize << (T__35 - 32))
                                    | (1usize << (T__36 - 32))
                                    | (1usize << (IDENT - 32))
                                    | (1usize << (USER_LIST_START - 32))
                                    | (1usize << (SYMBOL - 32))
                                    | (1usize << (STRING - 32))
                                    | (1usize << (REGEXP - 32))
                                    | (1usize << (USER_STRING - 32))
                                    | (1usize << (METHOD_RETURN - 32))
                                    | (1usize << (YIELD_RETURN - 32))
                                    | (1usize << (BLOCK_RETURN - 32))
                                    | (1usize << (NUMBER - 32))))
                                != 0))
                    {
                        break;
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- stmt ----------------
#[derive(Debug)]
pub enum StmtContextAll<'input> {
    MethodReturnContext(MethodReturnContext<'input>),
    ExprStmtContext(ExprStmtContext<'input>),
    Dot3StmtContext(Dot3StmtContext<'input>),
    AssignmentStmtContext(AssignmentStmtContext<'input>),
    Huh3StmtContext(Huh3StmtContext<'input>),
    BlockReturnContext(BlockReturnContext<'input>),
    Bang3StmtContext(Bang3StmtContext<'input>),
    YieldReturnContext(YieldReturnContext<'input>),
    Error(StmtContext<'input>),
}
antlr_rust::tid! {StmtContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for StmtContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for StmtContextAll<'input> {}

impl<'input> Deref for StmtContextAll<'input> {
    type Target = dyn StmtContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use StmtContextAll::*;
        match self {
            MethodReturnContext(inner) => inner,
            ExprStmtContext(inner) => inner,
            Dot3StmtContext(inner) => inner,
            AssignmentStmtContext(inner) => inner,
            Huh3StmtContext(inner) => inner,
            BlockReturnContext(inner) => inner,
            Bang3StmtContext(inner) => inner,
            YieldReturnContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for StmtContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for StmtContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type StmtContext<'input> = BaseParserRuleContext<'input, StmtContextExt<'input>>;

#[derive(Clone)]
pub struct StmtContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for StmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for StmtContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for StmtContext<'input> {}

impl<'input> CustomRuleContext<'input> for StmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}
antlr_rust::tid! {StmtContextExt<'a>}

impl<'input> StmtContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                StmtContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait StmtContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<StmtContextExt<'input>>
{
}

impl<'input> StmtContextAttrs<'input> for StmtContext<'input> {}

pub type MethodReturnContext<'input> =
    BaseParserRuleContext<'input, MethodReturnContextExt<'input>>;

pub trait MethodReturnContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token METHOD_RETURN
    /// Returns `None` if there is no child corresponding to token METHOD_RETURN
    fn METHOD_RETURN(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(METHOD_RETURN, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> MethodReturnContextAttrs<'input> for MethodReturnContext<'input> {}

pub struct MethodReturnContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {MethodReturnContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for MethodReturnContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for MethodReturnContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_MethodReturn(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_MethodReturn(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for MethodReturnContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_MethodReturn(self);
    }
}

impl<'input> CustomRuleContext<'input> for MethodReturnContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for MethodReturnContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for MethodReturnContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for MethodReturnContext<'input> {}

impl<'input> MethodReturnContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::MethodReturnContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MethodReturnContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ExprStmtContext<'input> = BaseParserRuleContext<'input, ExprStmtContextExt<'input>>;

pub trait ExprStmtContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ExprStmtContextAttrs<'input> for ExprStmtContext<'input> {}

pub struct ExprStmtContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ExprStmtContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ExprStmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ExprStmtContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ExprStmt(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ExprStmt(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ExprStmtContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ExprStmt(self);
    }
}

impl<'input> CustomRuleContext<'input> for ExprStmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for ExprStmtContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for ExprStmtContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for ExprStmtContext<'input> {}

impl<'input> ExprStmtContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::ExprStmtContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ExprStmtContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type Dot3StmtContext<'input> = BaseParserRuleContext<'input, Dot3StmtContextExt<'input>>;

pub trait Dot3StmtContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn dot3(&self) -> Option<Rc<Dot3ContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> Dot3StmtContextAttrs<'input> for Dot3StmtContext<'input> {}

pub struct Dot3StmtContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {Dot3StmtContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for Dot3StmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Dot3StmtContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_Dot3Stmt(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_Dot3Stmt(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Dot3StmtContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_Dot3Stmt(self);
    }
}

impl<'input> CustomRuleContext<'input> for Dot3StmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for Dot3StmtContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for Dot3StmtContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for Dot3StmtContext<'input> {}

impl<'input> Dot3StmtContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::Dot3StmtContext(
            BaseParserRuleContext::copy_from(
                ctx,
                Dot3StmtContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type AssignmentStmtContext<'input> =
    BaseParserRuleContext<'input, AssignmentStmtContextExt<'input>>;

pub trait AssignmentStmtContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn assignment(&self) -> Option<Rc<AssignmentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> AssignmentStmtContextAttrs<'input> for AssignmentStmtContext<'input> {}

pub struct AssignmentStmtContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {AssignmentStmtContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for AssignmentStmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for AssignmentStmtContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_AssignmentStmt(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_AssignmentStmt(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for AssignmentStmtContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_AssignmentStmt(self);
    }
}

impl<'input> CustomRuleContext<'input> for AssignmentStmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for AssignmentStmtContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for AssignmentStmtContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for AssignmentStmtContext<'input> {}

impl<'input> AssignmentStmtContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::AssignmentStmtContext(
            BaseParserRuleContext::copy_from(
                ctx,
                AssignmentStmtContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type Huh3StmtContext<'input> = BaseParserRuleContext<'input, Huh3StmtContextExt<'input>>;

pub trait Huh3StmtContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn huh3(&self) -> Option<Rc<Huh3ContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> Huh3StmtContextAttrs<'input> for Huh3StmtContext<'input> {}

pub struct Huh3StmtContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {Huh3StmtContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for Huh3StmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Huh3StmtContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_Huh3Stmt(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_Huh3Stmt(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Huh3StmtContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_Huh3Stmt(self);
    }
}

impl<'input> CustomRuleContext<'input> for Huh3StmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for Huh3StmtContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for Huh3StmtContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for Huh3StmtContext<'input> {}

impl<'input> Huh3StmtContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::Huh3StmtContext(
            BaseParserRuleContext::copy_from(
                ctx,
                Huh3StmtContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockReturnContext<'input> = BaseParserRuleContext<'input, BlockReturnContextExt<'input>>;

pub trait BlockReturnContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token BLOCK_RETURN
    /// Returns `None` if there is no child corresponding to token BLOCK_RETURN
    fn BLOCK_RETURN(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(BLOCK_RETURN, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockReturnContextAttrs<'input> for BlockReturnContext<'input> {}

pub struct BlockReturnContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockReturnContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockReturnContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockReturnContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockReturn(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockReturn(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockReturnContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockReturn(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockReturnContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for BlockReturnContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for BlockReturnContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for BlockReturnContext<'input> {}

impl<'input> BlockReturnContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::BlockReturnContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockReturnContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type Bang3StmtContext<'input> = BaseParserRuleContext<'input, Bang3StmtContextExt<'input>>;

pub trait Bang3StmtContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn bang3(&self) -> Option<Rc<Bang3ContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> Bang3StmtContextAttrs<'input> for Bang3StmtContext<'input> {}

pub struct Bang3StmtContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {Bang3StmtContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for Bang3StmtContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Bang3StmtContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_Bang3Stmt(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_Bang3Stmt(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Bang3StmtContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_Bang3Stmt(self);
    }
}

impl<'input> CustomRuleContext<'input> for Bang3StmtContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for Bang3StmtContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for Bang3StmtContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for Bang3StmtContext<'input> {}

impl<'input> Bang3StmtContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::Bang3StmtContext(
            BaseParserRuleContext::copy_from(
                ctx,
                Bang3StmtContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type YieldReturnContext<'input> = BaseParserRuleContext<'input, YieldReturnContextExt<'input>>;

pub trait YieldReturnContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token YIELD_RETURN
    /// Returns `None` if there is no child corresponding to token YIELD_RETURN
    fn YIELD_RETURN(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(YIELD_RETURN, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> YieldReturnContextAttrs<'input> for YieldReturnContext<'input> {}

pub struct YieldReturnContextExt<'input> {
    base: StmtContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {YieldReturnContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for YieldReturnContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for YieldReturnContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_YieldReturn(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_YieldReturn(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for YieldReturnContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_YieldReturn(self);
    }
}

impl<'input> CustomRuleContext<'input> for YieldReturnContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_stmt
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_stmt }
}

impl<'input> Borrow<StmtContextExt<'input>> for YieldReturnContext<'input> {
    fn borrow(&self) -> &StmtContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<StmtContextExt<'input>> for YieldReturnContext<'input> {
    fn borrow_mut(&mut self) -> &mut StmtContextExt<'input> {
        &mut self.base
    }
}

impl<'input> StmtContextAttrs<'input> for YieldReturnContext<'input> {}

impl<'input> YieldReturnContextExt<'input> {
    fn new(ctx: &dyn StmtContextAttrs<'input>) -> Rc<StmtContextAll<'input>> {
        Rc::new(StmtContextAll::YieldReturnContext(
            BaseParserRuleContext::copy_from(
                ctx,
                YieldReturnContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn stmt(&mut self) -> Result<Rc<StmtContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = StmtContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 2, RULE_stmt);
        let mut _localctx: Rc<StmtContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(65);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(2, &mut recog.base)? {
                1 => {
                    let tmp = MethodReturnContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(54);
                        recog
                            .base
                            .match_token(METHOD_RETURN, &mut recog.err_handler)?;

                        /*InvokeRule expr*/
                        recog.base.set_state(55);
                        recog.expr_rec(0)?;
                    }
                }
                2 => {
                    let tmp = YieldReturnContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(56);
                        recog
                            .base
                            .match_token(YIELD_RETURN, &mut recog.err_handler)?;

                        /*InvokeRule expr*/
                        recog.base.set_state(57);
                        recog.expr_rec(0)?;
                    }
                }
                3 => {
                    let tmp = BlockReturnContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        recog.base.set_state(58);
                        recog
                            .base
                            .match_token(BLOCK_RETURN, &mut recog.err_handler)?;

                        /*InvokeRule expr*/
                        recog.base.set_state(59);
                        recog.expr_rec(0)?;
                    }
                }
                4 => {
                    let tmp = AssignmentStmtContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 4);
                    _localctx = tmp;
                    {
                        /*InvokeRule assignment*/
                        recog.base.set_state(60);
                        recog.assignment()?;
                    }
                }
                5 => {
                    let tmp = Bang3StmtContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 5);
                    _localctx = tmp;
                    {
                        /*InvokeRule bang3*/
                        recog.base.set_state(61);
                        recog.bang3()?;
                    }
                }
                6 => {
                    let tmp = Dot3StmtContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 6);
                    _localctx = tmp;
                    {
                        /*InvokeRule dot3*/
                        recog.base.set_state(62);
                        recog.dot3()?;
                    }
                }
                7 => {
                    let tmp = Huh3StmtContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 7);
                    _localctx = tmp;
                    {
                        /*InvokeRule huh3*/
                        recog.base.set_state(63);
                        recog.huh3()?;
                    }
                }
                8 => {
                    let tmp = ExprStmtContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 8);
                    _localctx = tmp;
                    {
                        /*InvokeRule expr*/
                        recog.base.set_state(64);
                        recog.expr_rec(0)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- bang3 ----------------
pub type Bang3ContextAll<'input> = Bang3Context<'input>;

pub type Bang3Context<'input> = BaseParserRuleContext<'input, Bang3ContextExt<'input>>;

#[derive(Clone)]
pub struct Bang3ContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for Bang3Context<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Bang3Context<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_bang3(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_bang3(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Bang3Context<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_bang3(self);
    }
}

impl<'input> CustomRuleContext<'input> for Bang3ContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_bang3
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_bang3 }
}
antlr_rust::tid! {Bang3ContextExt<'a>}

impl<'input> Bang3ContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<Bang3ContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            Bang3ContextExt { ph: PhantomData },
        ))
    }
}

pub trait Bang3ContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<Bang3ContextExt<'input>>
{
}

impl<'input> Bang3ContextAttrs<'input> for Bang3Context<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn bang3(&mut self) -> Result<Rc<Bang3ContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = Bang3ContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 4, RULE_bang3);
        let mut _localctx: Rc<Bang3ContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(67);
                recog.base.match_token(T__1, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- dot3 ----------------
pub type Dot3ContextAll<'input> = Dot3Context<'input>;

pub type Dot3Context<'input> = BaseParserRuleContext<'input, Dot3ContextExt<'input>>;

#[derive(Clone)]
pub struct Dot3ContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for Dot3Context<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Dot3Context<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_dot3(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_dot3(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Dot3Context<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_dot3(self);
    }
}

impl<'input> CustomRuleContext<'input> for Dot3ContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_dot3
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_dot3 }
}
antlr_rust::tid! {Dot3ContextExt<'a>}

impl<'input> Dot3ContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<Dot3ContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            Dot3ContextExt { ph: PhantomData },
        ))
    }
}

pub trait Dot3ContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<Dot3ContextExt<'input>>
{
}

impl<'input> Dot3ContextAttrs<'input> for Dot3Context<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn dot3(&mut self) -> Result<Rc<Dot3ContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = Dot3ContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 6, RULE_dot3);
        let mut _localctx: Rc<Dot3ContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(69);
                recog.base.match_token(T__2, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- huh3 ----------------
pub type Huh3ContextAll<'input> = Huh3Context<'input>;

pub type Huh3Context<'input> = BaseParserRuleContext<'input, Huh3ContextExt<'input>>;

#[derive(Clone)]
pub struct Huh3ContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for Huh3Context<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for Huh3Context<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_huh3(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_huh3(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for Huh3Context<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_huh3(self);
    }
}

impl<'input> CustomRuleContext<'input> for Huh3ContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_huh3
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_huh3 }
}
antlr_rust::tid! {Huh3ContextExt<'a>}

impl<'input> Huh3ContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<Huh3ContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            Huh3ContextExt { ph: PhantomData },
        ))
    }
}

pub trait Huh3ContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<Huh3ContextExt<'input>>
{
}

impl<'input> Huh3ContextAttrs<'input> for Huh3Context<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn huh3(&mut self) -> Result<Rc<Huh3ContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = Huh3ContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 8, RULE_huh3);
        let mut _localctx: Rc<Huh3ContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(71);
                recog.base.match_token(T__3, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- selector ----------------
#[derive(Debug)]
pub enum SelectorContextAll<'input> {
    SelectorNoArgsContext(SelectorNoArgsContext<'input>),
    SelectorNoArgsBangContext(SelectorNoArgsBangContext<'input>),
    SelectorSymbolContext(SelectorSymbolContext<'input>),
    SelectorWArgsContext(SelectorWArgsContext<'input>),
    Error(SelectorContext<'input>),
}
antlr_rust::tid! {SelectorContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for SelectorContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for SelectorContextAll<'input> {}

impl<'input> Deref for SelectorContextAll<'input> {
    type Target = dyn SelectorContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use SelectorContextAll::*;
        match self {
            SelectorNoArgsContext(inner) => inner,
            SelectorNoArgsBangContext(inner) => inner,
            SelectorSymbolContext(inner) => inner,
            SelectorWArgsContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SelectorContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SelectorContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type SelectorContext<'input> = BaseParserRuleContext<'input, SelectorContextExt<'input>>;

#[derive(Clone)]
pub struct SelectorContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for SelectorContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for SelectorContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SelectorContext<'input> {}

impl<'input> CustomRuleContext<'input> for SelectorContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_selector
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_selector }
}
antlr_rust::tid! {SelectorContextExt<'a>}

impl<'input> SelectorContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<SelectorContextAll<'input>> {
        Rc::new(SelectorContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                SelectorContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait SelectorContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<SelectorContextExt<'input>>
{
}

impl<'input> SelectorContextAttrs<'input> for SelectorContext<'input> {}

pub type SelectorNoArgsContext<'input> =
    BaseParserRuleContext<'input, SelectorNoArgsContextExt<'input>>;

pub trait SelectorNoArgsContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> SelectorNoArgsContextAttrs<'input> for SelectorNoArgsContext<'input> {}

pub struct SelectorNoArgsContextExt<'input> {
    base: SelectorContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SelectorNoArgsContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SelectorNoArgsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SelectorNoArgsContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SelectorNoArgs(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SelectorNoArgs(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for SelectorNoArgsContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SelectorNoArgs(self);
    }
}

impl<'input> CustomRuleContext<'input> for SelectorNoArgsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_selector
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_selector }
}

impl<'input> Borrow<SelectorContextExt<'input>> for SelectorNoArgsContext<'input> {
    fn borrow(&self) -> &SelectorContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<SelectorContextExt<'input>> for SelectorNoArgsContext<'input> {
    fn borrow_mut(&mut self) -> &mut SelectorContextExt<'input> {
        &mut self.base
    }
}

impl<'input> SelectorContextAttrs<'input> for SelectorNoArgsContext<'input> {}

impl<'input> SelectorNoArgsContextExt<'input> {
    fn new(ctx: &dyn SelectorContextAttrs<'input>) -> Rc<SelectorContextAll<'input>> {
        Rc::new(SelectorContextAll::SelectorNoArgsContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SelectorNoArgsContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SelectorNoArgsBangContext<'input> =
    BaseParserRuleContext<'input, SelectorNoArgsBangContextExt<'input>>;

pub trait SelectorNoArgsBangContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> SelectorNoArgsBangContextAttrs<'input> for SelectorNoArgsBangContext<'input> {}

pub struct SelectorNoArgsBangContextExt<'input> {
    base: SelectorContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SelectorNoArgsBangContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SelectorNoArgsBangContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SelectorNoArgsBangContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SelectorNoArgsBang(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SelectorNoArgsBang(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for SelectorNoArgsBangContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SelectorNoArgsBang(self);
    }
}

impl<'input> CustomRuleContext<'input> for SelectorNoArgsBangContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_selector
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_selector }
}

impl<'input> Borrow<SelectorContextExt<'input>> for SelectorNoArgsBangContext<'input> {
    fn borrow(&self) -> &SelectorContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<SelectorContextExt<'input>> for SelectorNoArgsBangContext<'input> {
    fn borrow_mut(&mut self) -> &mut SelectorContextExt<'input> {
        &mut self.base
    }
}

impl<'input> SelectorContextAttrs<'input> for SelectorNoArgsBangContext<'input> {}

impl<'input> SelectorNoArgsBangContextExt<'input> {
    fn new(ctx: &dyn SelectorContextAttrs<'input>) -> Rc<SelectorContextAll<'input>> {
        Rc::new(SelectorContextAll::SelectorNoArgsBangContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SelectorNoArgsBangContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SelectorSymbolContext<'input> =
    BaseParserRuleContext<'input, SelectorSymbolContextExt<'input>>;

pub trait SelectorSymbolContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn symbol(&self) -> Option<Rc<SymbolContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> SelectorSymbolContextAttrs<'input> for SelectorSymbolContext<'input> {}

pub struct SelectorSymbolContextExt<'input> {
    base: SelectorContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SelectorSymbolContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SelectorSymbolContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SelectorSymbolContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SelectorSymbol(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SelectorSymbol(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for SelectorSymbolContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SelectorSymbol(self);
    }
}

impl<'input> CustomRuleContext<'input> for SelectorSymbolContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_selector
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_selector }
}

impl<'input> Borrow<SelectorContextExt<'input>> for SelectorSymbolContext<'input> {
    fn borrow(&self) -> &SelectorContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<SelectorContextExt<'input>> for SelectorSymbolContext<'input> {
    fn borrow_mut(&mut self) -> &mut SelectorContextExt<'input> {
        &mut self.base
    }
}

impl<'input> SelectorContextAttrs<'input> for SelectorSymbolContext<'input> {}

impl<'input> SelectorSymbolContextExt<'input> {
    fn new(ctx: &dyn SelectorContextAttrs<'input>) -> Rc<SelectorContextAll<'input>> {
        Rc::new(SelectorContextAll::SelectorSymbolContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SelectorSymbolContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SelectorWArgsContext<'input> =
    BaseParserRuleContext<'input, SelectorWArgsContextExt<'input>>;

pub trait SelectorWArgsContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident_all(&self) -> Vec<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn ident(&self, i: usize) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> SelectorWArgsContextAttrs<'input> for SelectorWArgsContext<'input> {}

pub struct SelectorWArgsContextExt<'input> {
    base: SelectorContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SelectorWArgsContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SelectorWArgsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SelectorWArgsContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SelectorWArgs(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SelectorWArgs(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for SelectorWArgsContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SelectorWArgs(self);
    }
}

impl<'input> CustomRuleContext<'input> for SelectorWArgsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_selector
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_selector }
}

impl<'input> Borrow<SelectorContextExt<'input>> for SelectorWArgsContext<'input> {
    fn borrow(&self) -> &SelectorContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<SelectorContextExt<'input>> for SelectorWArgsContext<'input> {
    fn borrow_mut(&mut self) -> &mut SelectorContextExt<'input> {
        &mut self.base
    }
}

impl<'input> SelectorContextAttrs<'input> for SelectorWArgsContext<'input> {}

impl<'input> SelectorWArgsContextExt<'input> {
    fn new(ctx: &dyn SelectorContextAttrs<'input>) -> Rc<SelectorContextAll<'input>> {
        Rc::new(SelectorContextAll::SelectorWArgsContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SelectorWArgsContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn selector(&mut self) -> Result<Rc<SelectorContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = SelectorContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 10, RULE_selector);
        let mut _localctx: Rc<SelectorContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(88);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(5, &mut recog.base)? {
                1 => {
                    let tmp = SelectorWArgsContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(79);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        loop {
                            {
                                {
                                    /*InvokeRule ident*/
                                    recog.base.set_state(73);
                                    recog.ident()?;

                                    recog.base.set_state(75);
                                    recog.err_handler.sync(&mut recog.base)?;
                                    _la = recog.base.input.la(1);
                                    if _la == T__4 {
                                        {
                                            recog.base.set_state(74);
                                            recog.base.match_token(T__4, &mut recog.err_handler)?;
                                        }
                                    }

                                    recog.base.set_state(77);
                                    recog.base.match_token(T__5, &mut recog.err_handler)?;
                                }
                            }
                            recog.base.set_state(81);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            if !(((_la - 35) & !0x3f) == 0
                                && ((1usize << (_la - 35))
                                    & ((1usize << (T__34 - 35))
                                        | (1usize << (T__35 - 35))
                                        | (1usize << (T__36 - 35))
                                        | (1usize << (IDENT - 35))))
                                    != 0)
                            {
                                break;
                            }
                        }
                    }
                }
                2 => {
                    let tmp = SelectorNoArgsContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(83);
                        recog.ident()?;
                    }
                }
                3 => {
                    let tmp = SelectorNoArgsBangContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(84);
                        recog.ident()?;

                        recog.base.set_state(85);
                        recog.base.match_token(T__6, &mut recog.err_handler)?;
                    }
                }
                4 => {
                    let tmp = SelectorSymbolContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 4);
                    _localctx = tmp;
                    {
                        /*InvokeRule symbol*/
                        recog.base.set_state(87);
                        recog.symbol()?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- assignment ----------------
pub type AssignmentContextAll<'input> = AssignmentContext<'input>;

pub type AssignmentContext<'input> = BaseParserRuleContext<'input, AssignmentContextExt<'input>>;

#[derive(Clone)]
pub struct AssignmentContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for AssignmentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for AssignmentContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_assignment(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_assignment(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for AssignmentContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_assignment(self);
    }
}

impl<'input> CustomRuleContext<'input> for AssignmentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_assignment
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_assignment }
}
antlr_rust::tid! {AssignmentContextExt<'a>}

impl<'input> AssignmentContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<AssignmentContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            AssignmentContextExt { ph: PhantomData },
        ))
    }
}

pub trait AssignmentContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<AssignmentContextExt<'input>>
{
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn lvalue_all(&self) -> Vec<Rc<LvalueContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn lvalue(&self, i: usize) -> Option<Rc<LvalueContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> AssignmentContextAttrs<'input> for AssignmentContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn assignment(&mut self) -> Result<Rc<AssignmentContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = AssignmentContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 12, RULE_assignment);
        let mut _localctx: Rc<AssignmentContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(91);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                loop {
                    {
                        {
                            /*InvokeRule lvalue*/
                            recog.base.set_state(90);
                            recog.lvalue()?;
                        }
                    }
                    recog.base.set_state(93);
                    recog.err_handler.sync(&mut recog.base)?;
                    _la = recog.base.input.la(1);
                    if !(((_la - 9) & !0x3f) == 0
                        && ((1usize << (_la - 9))
                            & ((1usize << (T__8 - 9))
                                | (1usize << (T__9 - 9))
                                | (1usize << (T__10 - 9))
                                | (1usize << (T__31 - 9))
                                | (1usize << (T__32 - 9))
                                | (1usize << (T__34 - 9))
                                | (1usize << (T__35 - 9))
                                | (1usize << (T__36 - 9))
                                | (1usize << (IDENT - 9))))
                            != 0)
                    {
                        break;
                    }
                }
                recog.base.set_state(95);
                recog.base.match_token(T__7, &mut recog.err_handler)?;

                /*InvokeRule expr*/
                recog.base.set_state(96);
                recog.expr_rec(0)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- lvalue ----------------
#[derive(Debug)]
pub enum LvalueContextAll<'input> {
    IdentLValueContext(IdentLValueContext<'input>),
    SplatLValueContext(SplatLValueContext<'input>),
    SubLValueContext(SubLValueContext<'input>),
    IgnoredSplatLValueContext(IgnoredSplatLValueContext<'input>),
    IgnoredLValueContext(IgnoredLValueContext<'input>),
    Error(LvalueContext<'input>),
}
antlr_rust::tid! {LvalueContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for LvalueContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for LvalueContextAll<'input> {}

impl<'input> Deref for LvalueContextAll<'input> {
    type Target = dyn LvalueContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use LvalueContextAll::*;
        match self {
            IdentLValueContext(inner) => inner,
            SplatLValueContext(inner) => inner,
            SubLValueContext(inner) => inner,
            IgnoredSplatLValueContext(inner) => inner,
            IgnoredLValueContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for LvalueContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for LvalueContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type LvalueContext<'input> = BaseParserRuleContext<'input, LvalueContextExt<'input>>;

#[derive(Clone)]
pub struct LvalueContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for LvalueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for LvalueContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for LvalueContext<'input> {}

impl<'input> CustomRuleContext<'input> for LvalueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}
antlr_rust::tid! {LvalueContextExt<'a>}

impl<'input> LvalueContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                LvalueContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait LvalueContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<LvalueContextExt<'input>>
{
}

impl<'input> LvalueContextAttrs<'input> for LvalueContext<'input> {}

pub type IdentLValueContext<'input> = BaseParserRuleContext<'input, IdentLValueContextExt<'input>>;

pub trait IdentLValueContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn nsvarident(&self) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> IdentLValueContextAttrs<'input> for IdentLValueContext<'input> {}

pub struct IdentLValueContextExt<'input> {
    base: LvalueContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IdentLValueContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IdentLValueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for IdentLValueContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IdentLValue(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IdentLValue(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentLValueContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IdentLValue(self);
    }
}

impl<'input> CustomRuleContext<'input> for IdentLValueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}

impl<'input> Borrow<LvalueContextExt<'input>> for IdentLValueContext<'input> {
    fn borrow(&self) -> &LvalueContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LvalueContextExt<'input>> for IdentLValueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LvalueContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LvalueContextAttrs<'input> for IdentLValueContext<'input> {}

impl<'input> IdentLValueContextExt<'input> {
    fn new(ctx: &dyn LvalueContextAttrs<'input>) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::IdentLValueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IdentLValueContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SplatLValueContext<'input> = BaseParserRuleContext<'input, SplatLValueContextExt<'input>>;

pub trait SplatLValueContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn nsvarident(&self) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> SplatLValueContextAttrs<'input> for SplatLValueContext<'input> {}

pub struct SplatLValueContextExt<'input> {
    base: LvalueContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SplatLValueContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SplatLValueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for SplatLValueContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SplatLValue(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SplatLValue(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SplatLValueContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SplatLValue(self);
    }
}

impl<'input> CustomRuleContext<'input> for SplatLValueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}

impl<'input> Borrow<LvalueContextExt<'input>> for SplatLValueContext<'input> {
    fn borrow(&self) -> &LvalueContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LvalueContextExt<'input>> for SplatLValueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LvalueContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LvalueContextAttrs<'input> for SplatLValueContext<'input> {}

impl<'input> SplatLValueContextExt<'input> {
    fn new(ctx: &dyn LvalueContextAttrs<'input>) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::SplatLValueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SplatLValueContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SubLValueContext<'input> = BaseParserRuleContext<'input, SubLValueContextExt<'input>>;

pub trait SubLValueContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn lvalue_all(&self) -> Vec<Rc<LvalueContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn lvalue(&self, i: usize) -> Option<Rc<LvalueContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> SubLValueContextAttrs<'input> for SubLValueContext<'input> {}

pub struct SubLValueContextExt<'input> {
    base: LvalueContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SubLValueContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SubLValueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for SubLValueContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SubLValue(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SubLValue(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SubLValueContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SubLValue(self);
    }
}

impl<'input> CustomRuleContext<'input> for SubLValueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}

impl<'input> Borrow<LvalueContextExt<'input>> for SubLValueContext<'input> {
    fn borrow(&self) -> &LvalueContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LvalueContextExt<'input>> for SubLValueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LvalueContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LvalueContextAttrs<'input> for SubLValueContext<'input> {}

impl<'input> SubLValueContextExt<'input> {
    fn new(ctx: &dyn LvalueContextAttrs<'input>) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::SubLValueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SubLValueContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IgnoredSplatLValueContext<'input> =
    BaseParserRuleContext<'input, IgnoredSplatLValueContextExt<'input>>;

pub trait IgnoredSplatLValueContextAttrs<'input>: BuildingBlocksParserContext<'input> {}

impl<'input> IgnoredSplatLValueContextAttrs<'input> for IgnoredSplatLValueContext<'input> {}

pub struct IgnoredSplatLValueContextExt<'input> {
    base: LvalueContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IgnoredSplatLValueContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IgnoredSplatLValueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for IgnoredSplatLValueContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IgnoredSplatLValue(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IgnoredSplatLValue(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for IgnoredSplatLValueContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IgnoredSplatLValue(self);
    }
}

impl<'input> CustomRuleContext<'input> for IgnoredSplatLValueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}

impl<'input> Borrow<LvalueContextExt<'input>> for IgnoredSplatLValueContext<'input> {
    fn borrow(&self) -> &LvalueContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LvalueContextExt<'input>> for IgnoredSplatLValueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LvalueContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LvalueContextAttrs<'input> for IgnoredSplatLValueContext<'input> {}

impl<'input> IgnoredSplatLValueContextExt<'input> {
    fn new(ctx: &dyn LvalueContextAttrs<'input>) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::IgnoredSplatLValueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IgnoredSplatLValueContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IgnoredLValueContext<'input> =
    BaseParserRuleContext<'input, IgnoredLValueContextExt<'input>>;

pub trait IgnoredLValueContextAttrs<'input>: BuildingBlocksParserContext<'input> {}

impl<'input> IgnoredLValueContextAttrs<'input> for IgnoredLValueContext<'input> {}

pub struct IgnoredLValueContextExt<'input> {
    base: LvalueContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IgnoredLValueContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IgnoredLValueContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for IgnoredLValueContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IgnoredLValue(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IgnoredLValue(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for IgnoredLValueContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IgnoredLValue(self);
    }
}

impl<'input> CustomRuleContext<'input> for IgnoredLValueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_lvalue
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_lvalue }
}

impl<'input> Borrow<LvalueContextExt<'input>> for IgnoredLValueContext<'input> {
    fn borrow(&self) -> &LvalueContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LvalueContextExt<'input>> for IgnoredLValueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LvalueContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LvalueContextAttrs<'input> for IgnoredLValueContext<'input> {}

impl<'input> IgnoredLValueContextExt<'input> {
    fn new(ctx: &dyn LvalueContextAttrs<'input>) -> Rc<LvalueContextAll<'input>> {
        Rc::new(LvalueContextAll::IgnoredLValueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IgnoredLValueContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn lvalue(&mut self) -> Result<Rc<LvalueContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = LvalueContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 14, RULE_lvalue);
        let mut _localctx: Rc<LvalueContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(112);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(8, &mut recog.base)? {
                1 => {
                    let tmp = IdentLValueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        /*InvokeRule nsvarident*/
                        recog.base.set_state(98);
                        recog.nsvarident()?;
                    }
                }
                2 => {
                    let tmp = SplatLValueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(99);
                        recog.base.match_token(T__8, &mut recog.err_handler)?;

                        /*InvokeRule nsvarident*/
                        recog.base.set_state(100);
                        recog.nsvarident()?;
                    }
                }
                3 => {
                    let tmp = IgnoredLValueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        recog.base.set_state(101);
                        recog.base.match_token(T__9, &mut recog.err_handler)?;
                    }
                }
                4 => {
                    let tmp = IgnoredSplatLValueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 4);
                    _localctx = tmp;
                    {
                        recog.base.set_state(102);
                        recog.base.match_token(T__8, &mut recog.err_handler)?;

                        recog.base.set_state(103);
                        recog.base.match_token(T__9, &mut recog.err_handler)?;
                    }
                }
                5 => {
                    let tmp = SubLValueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 5);
                    _localctx = tmp;
                    {
                        recog.base.set_state(104);
                        recog.base.match_token(T__10, &mut recog.err_handler)?;

                        recog.base.set_state(106);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        loop {
                            {
                                {
                                    /*InvokeRule lvalue*/
                                    recog.base.set_state(105);
                                    recog.lvalue()?;
                                }
                            }
                            recog.base.set_state(108);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            if !(((_la - 9) & !0x3f) == 0
                                && ((1usize << (_la - 9))
                                    & ((1usize << (T__8 - 9))
                                        | (1usize << (T__9 - 9))
                                        | (1usize << (T__10 - 9))
                                        | (1usize << (T__31 - 9))
                                        | (1usize << (T__32 - 9))
                                        | (1usize << (T__34 - 9))
                                        | (1usize << (T__35 - 9))
                                        | (1usize << (T__36 - 9))
                                        | (1usize << (IDENT - 9))))
                                    != 0)
                            {
                                break;
                            }
                        }
                        recog.base.set_state(110);
                        recog.base.match_token(T__11, &mut recog.err_handler)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- expr ----------------
#[derive(Debug)]
pub enum ExprContextAll<'input> {
    MulExprContext(MulExprContext<'input>),
    AndExprContext(AndExprContext<'input>),
    LiteralStringContext(LiteralStringContext<'input>),
    UserStringExprContext(UserStringExprContext<'input>),
    RegexExprContext(RegexExprContext<'input>),
    GtExprContext(GtExprContext<'input>),
    LtExprContext(LtExprContext<'input>),
    UserListExprContext(UserListExprContext<'input>),
    LtEqExprContext(LtEqExprContext<'input>),
    MethodDefExprContext(MethodDefExprContext<'input>),
    LiteralSymbolContext(LiteralSymbolContext<'input>),
    ClassDefExprContext(ClassDefExprContext<'input>),
    ExprCallExprContext(ExprCallExprContext<'input>),
    SetExprContext(SetExprContext<'input>),
    UnModExprContext(UnModExprContext<'input>),
    MethodExtExprContext(MethodExtExprContext<'input>),
    DictExprContext(DictExprContext<'input>),
    ListExprContext(ListExprContext<'input>),
    IdentExprContext(IdentExprContext<'input>),
    SubExprContext(SubExprContext<'input>),
    AddExprContext(AddExprContext<'input>),
    ConstDefExprContext(ConstDefExprContext<'input>),
    RangeExprContext(RangeExprContext<'input>),
    UnPlusExprContext(UnPlusExprContext<'input>),
    BlockExprContext(BlockExprContext<'input>),
    OrExprContext(OrExprContext<'input>),
    ClassDef2ExprContext(ClassDef2ExprContext<'input>),
    GtEqExprContext(GtEqExprContext<'input>),
    DivExprContext(DivExprContext<'input>),
    UnBangExprContext(UnBangExprContext<'input>),
    NotEqExprContext(NotEqExprContext<'input>),
    UnMinusExprContext(UnMinusExprContext<'input>),
    EqExprContext(EqExprContext<'input>),
    ClassExtExprContext(ClassExtExprContext<'input>),
    NestedExprContext(NestedExprContext<'input>),
    ModExprContext(ModExprContext<'input>),
    MatchExprContext(MatchExprContext<'input>),
    DefCallExprContext(DefCallExprContext<'input>),
    LiteralNumberContext(LiteralNumberContext<'input>),
    Error(ExprContext<'input>),
}
antlr_rust::tid! {ExprContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for ExprContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for ExprContextAll<'input> {}

impl<'input> Deref for ExprContextAll<'input> {
    type Target = dyn ExprContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use ExprContextAll::*;
        match self {
            MulExprContext(inner) => inner,
            AndExprContext(inner) => inner,
            LiteralStringContext(inner) => inner,
            UserStringExprContext(inner) => inner,
            RegexExprContext(inner) => inner,
            GtExprContext(inner) => inner,
            LtExprContext(inner) => inner,
            UserListExprContext(inner) => inner,
            LtEqExprContext(inner) => inner,
            MethodDefExprContext(inner) => inner,
            LiteralSymbolContext(inner) => inner,
            ClassDefExprContext(inner) => inner,
            ExprCallExprContext(inner) => inner,
            SetExprContext(inner) => inner,
            UnModExprContext(inner) => inner,
            MethodExtExprContext(inner) => inner,
            DictExprContext(inner) => inner,
            ListExprContext(inner) => inner,
            IdentExprContext(inner) => inner,
            SubExprContext(inner) => inner,
            AddExprContext(inner) => inner,
            ConstDefExprContext(inner) => inner,
            RangeExprContext(inner) => inner,
            UnPlusExprContext(inner) => inner,
            BlockExprContext(inner) => inner,
            OrExprContext(inner) => inner,
            ClassDef2ExprContext(inner) => inner,
            GtEqExprContext(inner) => inner,
            DivExprContext(inner) => inner,
            UnBangExprContext(inner) => inner,
            NotEqExprContext(inner) => inner,
            UnMinusExprContext(inner) => inner,
            EqExprContext(inner) => inner,
            ClassExtExprContext(inner) => inner,
            NestedExprContext(inner) => inner,
            ModExprContext(inner) => inner,
            MatchExprContext(inner) => inner,
            DefCallExprContext(inner) => inner,
            LiteralNumberContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ExprContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ExprContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type ExprContext<'input> = BaseParserRuleContext<'input, ExprContextExt<'input>>;

#[derive(Clone)]
pub struct ExprContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for ExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ExprContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ExprContext<'input> {}

impl<'input> CustomRuleContext<'input> for ExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}
antlr_rust::tid! {ExprContextExt<'a>}

impl<'input> ExprContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                ExprContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait ExprContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<ExprContextExt<'input>>
{
}

impl<'input> ExprContextAttrs<'input> for ExprContext<'input> {}

pub type MulExprContext<'input> = BaseParserRuleContext<'input, MulExprContextExt<'input>>;

pub trait MulExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> MulExprContextAttrs<'input> for MulExprContext<'input> {}

pub struct MulExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {MulExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for MulExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for MulExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_MulExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_MulExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for MulExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_MulExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for MulExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for MulExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for MulExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for MulExprContext<'input> {}

impl<'input> MulExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::MulExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MulExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type AndExprContext<'input> = BaseParserRuleContext<'input, AndExprContextExt<'input>>;

pub trait AndExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> AndExprContextAttrs<'input> for AndExprContext<'input> {}

pub struct AndExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {AndExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for AndExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for AndExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_AndExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_AndExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for AndExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_AndExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for AndExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for AndExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for AndExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for AndExprContext<'input> {}

impl<'input> AndExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::AndExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                AndExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LiteralStringContext<'input> =
    BaseParserRuleContext<'input, LiteralStringContextExt<'input>>;

pub trait LiteralStringContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn string(&self) -> Option<Rc<StringContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> LiteralStringContextAttrs<'input> for LiteralStringContext<'input> {}

pub struct LiteralStringContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LiteralStringContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LiteralStringContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for LiteralStringContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LiteralString(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LiteralString(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for LiteralStringContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LiteralString(self);
    }
}

impl<'input> CustomRuleContext<'input> for LiteralStringContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for LiteralStringContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for LiteralStringContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for LiteralStringContext<'input> {}

impl<'input> LiteralStringContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::LiteralStringContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LiteralStringContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UserStringExprContext<'input> =
    BaseParserRuleContext<'input, UserStringExprContextExt<'input>>;

pub trait UserStringExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn userString(&self) -> Option<Rc<UserStringContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> UserStringExprContextAttrs<'input> for UserStringExprContext<'input> {}

pub struct UserStringExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UserStringExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UserStringExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for UserStringExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UserStringExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UserStringExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for UserStringExprContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UserStringExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UserStringExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UserStringExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UserStringExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UserStringExprContext<'input> {}

impl<'input> UserStringExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UserStringExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UserStringExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type RegexExprContext<'input> = BaseParserRuleContext<'input, RegexExprContextExt<'input>>;

pub trait RegexExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token REGEXP
    /// Returns `None` if there is no child corresponding to token REGEXP
    fn REGEXP(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(REGEXP, 0)
    }
}

impl<'input> RegexExprContextAttrs<'input> for RegexExprContext<'input> {}

pub struct RegexExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {RegexExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for RegexExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for RegexExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_RegexExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_RegexExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for RegexExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_RegexExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for RegexExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for RegexExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for RegexExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for RegexExprContext<'input> {}

impl<'input> RegexExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::RegexExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                RegexExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type GtExprContext<'input> = BaseParserRuleContext<'input, GtExprContextExt<'input>>;

pub trait GtExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> GtExprContextAttrs<'input> for GtExprContext<'input> {}

pub struct GtExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {GtExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for GtExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for GtExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_GtExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_GtExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for GtExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_GtExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for GtExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for GtExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for GtExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for GtExprContext<'input> {}

impl<'input> GtExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::GtExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                GtExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LtExprContext<'input> = BaseParserRuleContext<'input, LtExprContextExt<'input>>;

pub trait LtExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> LtExprContextAttrs<'input> for LtExprContext<'input> {}

pub struct LtExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LtExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LtExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for LtExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LtExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LtExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for LtExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LtExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for LtExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for LtExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for LtExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for LtExprContext<'input> {}

impl<'input> LtExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::LtExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LtExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UserListExprContext<'input> =
    BaseParserRuleContext<'input, UserListExprContextExt<'input>>;

pub trait UserListExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token USER_LIST_START
    /// Returns `None` if there is no child corresponding to token USER_LIST_START
    fn USER_LIST_START(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(USER_LIST_START, 0)
    }
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> UserListExprContextAttrs<'input> for UserListExprContext<'input> {}

pub struct UserListExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UserListExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UserListExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for UserListExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UserListExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UserListExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UserListExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UserListExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UserListExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UserListExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UserListExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UserListExprContext<'input> {}

impl<'input> UserListExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UserListExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UserListExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LtEqExprContext<'input> = BaseParserRuleContext<'input, LtEqExprContextExt<'input>>;

pub trait LtEqExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> LtEqExprContextAttrs<'input> for LtEqExprContext<'input> {}

pub struct LtEqExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LtEqExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LtEqExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for LtEqExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LtEqExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LtEqExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for LtEqExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LtEqExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for LtEqExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for LtEqExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for LtEqExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for LtEqExprContext<'input> {}

impl<'input> LtEqExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::LtEqExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LtEqExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type MethodDefExprContext<'input> =
    BaseParserRuleContext<'input, MethodDefExprContextExt<'input>>;

pub trait MethodDefExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn selector(&self) -> Option<Rc<SelectorContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> MethodDefExprContextAttrs<'input> for MethodDefExprContext<'input> {}

pub struct MethodDefExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {MethodDefExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for MethodDefExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for MethodDefExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_MethodDefExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_MethodDefExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for MethodDefExprContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_MethodDefExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for MethodDefExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for MethodDefExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for MethodDefExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for MethodDefExprContext<'input> {}

impl<'input> MethodDefExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::MethodDefExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MethodDefExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LiteralSymbolContext<'input> =
    BaseParserRuleContext<'input, LiteralSymbolContextExt<'input>>;

pub trait LiteralSymbolContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn symbol(&self) -> Option<Rc<SymbolContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> LiteralSymbolContextAttrs<'input> for LiteralSymbolContext<'input> {}

pub struct LiteralSymbolContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LiteralSymbolContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LiteralSymbolContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for LiteralSymbolContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LiteralSymbol(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LiteralSymbol(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for LiteralSymbolContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LiteralSymbol(self);
    }
}

impl<'input> CustomRuleContext<'input> for LiteralSymbolContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for LiteralSymbolContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for LiteralSymbolContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for LiteralSymbolContext<'input> {}

impl<'input> LiteralSymbolContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::LiteralSymbolContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LiteralSymbolContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ClassDefExprContext<'input> =
    BaseParserRuleContext<'input, ClassDefExprContextExt<'input>>;

pub trait ClassDefExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn nsvarident(&self) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ClassDefExprContextAttrs<'input> for ClassDefExprContext<'input> {}

pub struct ClassDefExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub name: Option<Rc<NsvaridentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ClassDefExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ClassDefExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ClassDefExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ClassDefExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ClassDefExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ClassDefExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ClassDefExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ClassDefExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ClassDefExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ClassDefExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ClassDefExprContext<'input> {}

impl<'input> ClassDefExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ClassDefExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ClassDefExprContextExt {
                    name: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ExprCallExprContext<'input> =
    BaseParserRuleContext<'input, ExprCallExprContextExt<'input>>;

pub trait ExprCallExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn callSig(&self) -> Option<Rc<CallSigContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ExprCallExprContextAttrs<'input> for ExprCallExprContext<'input> {}

pub struct ExprCallExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub subject: Option<Rc<ExprContextAll<'input>>>,
    pub sig: Option<Rc<CallSigContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ExprCallExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ExprCallExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ExprCallExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ExprCallExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ExprCallExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ExprCallExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ExprCallExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ExprCallExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ExprCallExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ExprCallExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ExprCallExprContext<'input> {}

impl<'input> ExprCallExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ExprCallExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ExprCallExprContextExt {
                    subject: None,
                    sig: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SetExprContext<'input> = BaseParserRuleContext<'input, SetExprContextExt<'input>>;

pub trait SetExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> SetExprContextAttrs<'input> for SetExprContext<'input> {}

pub struct SetExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SetExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SetExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for SetExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SetExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SetExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SetExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SetExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for SetExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for SetExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for SetExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for SetExprContext<'input> {}

impl<'input> SetExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::SetExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SetExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UnModExprContext<'input> = BaseParserRuleContext<'input, UnModExprContextExt<'input>>;

pub trait UnModExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> UnModExprContextAttrs<'input> for UnModExprContext<'input> {}

pub struct UnModExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UnModExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UnModExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for UnModExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UnModExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UnModExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UnModExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UnModExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UnModExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UnModExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UnModExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UnModExprContext<'input> {}

impl<'input> UnModExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UnModExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UnModExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type MethodExtExprContext<'input> =
    BaseParserRuleContext<'input, MethodExtExprContextExt<'input>>;

pub trait MethodExtExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn selector(&self) -> Option<Rc<SelectorContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> MethodExtExprContextAttrs<'input> for MethodExtExprContext<'input> {}

pub struct MethodExtExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {MethodExtExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for MethodExtExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for MethodExtExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_MethodExtExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_MethodExtExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for MethodExtExprContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_MethodExtExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for MethodExtExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for MethodExtExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for MethodExtExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for MethodExtExprContext<'input> {}

impl<'input> MethodExtExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::MethodExtExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MethodExtExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type DictExprContext<'input> = BaseParserRuleContext<'input, DictExprContextExt<'input>>;

pub trait DictExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> DictExprContextAttrs<'input> for DictExprContext<'input> {}

pub struct DictExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub expr: Option<Rc<ExprContextAll<'input>>>,
    pub k: Vec<Rc<ExprContextAll<'input>>>,
    pub v: Vec<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {DictExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for DictExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for DictExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_DictExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_DictExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for DictExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_DictExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for DictExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for DictExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for DictExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for DictExprContext<'input> {}

impl<'input> DictExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::DictExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                DictExprContextExt {
                    expr: None,
                    k: Vec::new(),
                    v: Vec::new(),
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ListExprContext<'input> = BaseParserRuleContext<'input, ListExprContextExt<'input>>;

pub trait ListExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> ListExprContextAttrs<'input> for ListExprContext<'input> {}

pub struct ListExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ListExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ListExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ListExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ListExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ListExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ListExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ListExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ListExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ListExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ListExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ListExprContext<'input> {}

impl<'input> ListExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ListExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ListExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IdentExprContext<'input> = BaseParserRuleContext<'input, IdentExprContextExt<'input>>;

pub trait IdentExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn nsvarident(&self) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> IdentExprContextAttrs<'input> for IdentExprContext<'input> {}

pub struct IdentExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IdentExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IdentExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for IdentExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IdentExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IdentExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IdentExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for IdentExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for IdentExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for IdentExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for IdentExprContext<'input> {}

impl<'input> IdentExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::IdentExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IdentExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SubExprContext<'input> = BaseParserRuleContext<'input, SubExprContextExt<'input>>;

pub trait SubExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> SubExprContextAttrs<'input> for SubExprContext<'input> {}

pub struct SubExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {SubExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for SubExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for SubExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_SubExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_SubExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SubExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_SubExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for SubExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for SubExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for SubExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for SubExprContext<'input> {}

impl<'input> SubExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::SubExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SubExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type AddExprContext<'input> = BaseParserRuleContext<'input, AddExprContextExt<'input>>;

pub trait AddExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> AddExprContextAttrs<'input> for AddExprContext<'input> {}

pub struct AddExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {AddExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for AddExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for AddExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_AddExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_AddExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for AddExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_AddExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for AddExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for AddExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for AddExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for AddExprContext<'input> {}

impl<'input> AddExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::AddExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                AddExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ConstDefExprContext<'input> =
    BaseParserRuleContext<'input, ConstDefExprContextExt<'input>>;

pub trait ConstDefExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn nsvarident(&self) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ConstDefExprContextAttrs<'input> for ConstDefExprContext<'input> {}

pub struct ConstDefExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ConstDefExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ConstDefExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ConstDefExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ConstDefExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ConstDefExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ConstDefExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ConstDefExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ConstDefExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ConstDefExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ConstDefExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ConstDefExprContext<'input> {}

impl<'input> ConstDefExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ConstDefExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ConstDefExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type RangeExprContext<'input> = BaseParserRuleContext<'input, RangeExprContextExt<'input>>;

pub trait RangeExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> RangeExprContextAttrs<'input> for RangeExprContext<'input> {}

pub struct RangeExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {RangeExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for RangeExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for RangeExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_RangeExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_RangeExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for RangeExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_RangeExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for RangeExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for RangeExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for RangeExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for RangeExprContext<'input> {}

impl<'input> RangeExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::RangeExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                RangeExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UnPlusExprContext<'input> = BaseParserRuleContext<'input, UnPlusExprContextExt<'input>>;

pub trait UnPlusExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> UnPlusExprContextAttrs<'input> for UnPlusExprContext<'input> {}

pub struct UnPlusExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UnPlusExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UnPlusExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for UnPlusExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UnPlusExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UnPlusExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UnPlusExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UnPlusExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UnPlusExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UnPlusExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UnPlusExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UnPlusExprContext<'input> {}

impl<'input> UnPlusExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UnPlusExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UnPlusExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockExprContext<'input> = BaseParserRuleContext<'input, BlockExprContextExt<'input>>;

pub trait BlockExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockExprContextAttrs<'input> for BlockExprContext<'input> {}

pub struct BlockExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for BlockExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for BlockExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for BlockExprContext<'input> {}

impl<'input> BlockExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::BlockExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type OrExprContext<'input> = BaseParserRuleContext<'input, OrExprContextExt<'input>>;

pub trait OrExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> OrExprContextAttrs<'input> for OrExprContext<'input> {}

pub struct OrExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {OrExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for OrExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for OrExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_OrExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_OrExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for OrExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_OrExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for OrExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for OrExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for OrExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for OrExprContext<'input> {}

impl<'input> OrExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::OrExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                OrExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ClassDef2ExprContext<'input> =
    BaseParserRuleContext<'input, ClassDef2ExprContextExt<'input>>;

pub trait ClassDef2ExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn nsvarident_all(&self) -> Vec<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn nsvarident(&self, i: usize) -> Option<Rc<NsvaridentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> ClassDef2ExprContextAttrs<'input> for ClassDef2ExprContext<'input> {}

pub struct ClassDef2ExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub parent: Option<Rc<NsvaridentContextAll<'input>>>,
    pub name: Option<Rc<NsvaridentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ClassDef2ExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ClassDef2ExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ClassDef2ExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ClassDef2Expr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ClassDef2Expr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for ClassDef2ExprContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ClassDef2Expr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ClassDef2ExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ClassDef2ExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ClassDef2ExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ClassDef2ExprContext<'input> {}

impl<'input> ClassDef2ExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ClassDef2ExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ClassDef2ExprContextExt {
                    parent: None,
                    name: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type GtEqExprContext<'input> = BaseParserRuleContext<'input, GtEqExprContextExt<'input>>;

pub trait GtEqExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> GtEqExprContextAttrs<'input> for GtEqExprContext<'input> {}

pub struct GtEqExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {GtEqExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for GtEqExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for GtEqExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_GtEqExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_GtEqExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for GtEqExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_GtEqExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for GtEqExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for GtEqExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for GtEqExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for GtEqExprContext<'input> {}

impl<'input> GtEqExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::GtEqExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                GtEqExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type DivExprContext<'input> = BaseParserRuleContext<'input, DivExprContextExt<'input>>;

pub trait DivExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> DivExprContextAttrs<'input> for DivExprContext<'input> {}

pub struct DivExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {DivExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for DivExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for DivExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_DivExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_DivExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for DivExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_DivExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for DivExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for DivExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for DivExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for DivExprContext<'input> {}

impl<'input> DivExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::DivExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                DivExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UnBangExprContext<'input> = BaseParserRuleContext<'input, UnBangExprContextExt<'input>>;

pub trait UnBangExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> UnBangExprContextAttrs<'input> for UnBangExprContext<'input> {}

pub struct UnBangExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UnBangExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UnBangExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for UnBangExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UnBangExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UnBangExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UnBangExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UnBangExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UnBangExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UnBangExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UnBangExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UnBangExprContext<'input> {}

impl<'input> UnBangExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UnBangExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UnBangExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NotEqExprContext<'input> = BaseParserRuleContext<'input, NotEqExprContextExt<'input>>;

pub trait NotEqExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> NotEqExprContextAttrs<'input> for NotEqExprContext<'input> {}

pub struct NotEqExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {NotEqExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for NotEqExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for NotEqExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_NotEqExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_NotEqExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NotEqExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_NotEqExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for NotEqExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for NotEqExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for NotEqExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for NotEqExprContext<'input> {}

impl<'input> NotEqExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::NotEqExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NotEqExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UnMinusExprContext<'input> = BaseParserRuleContext<'input, UnMinusExprContextExt<'input>>;

pub trait UnMinusExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> UnMinusExprContextAttrs<'input> for UnMinusExprContext<'input> {}

pub struct UnMinusExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {UnMinusExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for UnMinusExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for UnMinusExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_UnMinusExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_UnMinusExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UnMinusExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_UnMinusExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for UnMinusExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for UnMinusExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for UnMinusExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for UnMinusExprContext<'input> {}

impl<'input> UnMinusExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::UnMinusExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UnMinusExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type EqExprContext<'input> = BaseParserRuleContext<'input, EqExprContextExt<'input>>;

pub trait EqExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> EqExprContextAttrs<'input> for EqExprContext<'input> {}

pub struct EqExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {EqExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for EqExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for EqExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_EqExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_EqExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for EqExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_EqExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for EqExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for EqExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for EqExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for EqExprContext<'input> {}

impl<'input> EqExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::EqExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                EqExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ClassExtExprContext<'input> =
    BaseParserRuleContext<'input, ClassExtExprContextExt<'input>>;

pub trait ClassExtExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ClassExtExprContextAttrs<'input> for ClassExtExprContext<'input> {}

pub struct ClassExtExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ClassExtExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ClassExtExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ClassExtExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ClassExtExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ClassExtExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ClassExtExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ClassExtExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ClassExtExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ClassExtExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ClassExtExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ClassExtExprContext<'input> {}

impl<'input> ClassExtExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ClassExtExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ClassExtExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NestedExprContext<'input> = BaseParserRuleContext<'input, NestedExprContextExt<'input>>;

pub trait NestedExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> NestedExprContextAttrs<'input> for NestedExprContext<'input> {}

pub struct NestedExprContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {NestedExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for NestedExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for NestedExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_NestedExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_NestedExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NestedExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_NestedExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for NestedExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for NestedExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for NestedExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for NestedExprContext<'input> {}

impl<'input> NestedExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::NestedExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NestedExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ModExprContext<'input> = BaseParserRuleContext<'input, ModExprContextExt<'input>>;

pub trait ModExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> ModExprContextAttrs<'input> for ModExprContext<'input> {}

pub struct ModExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ModExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ModExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ModExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ModExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ModExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ModExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ModExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ModExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for ModExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for ModExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for ModExprContext<'input> {}

impl<'input> ModExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::ModExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ModExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type MatchExprContext<'input> = BaseParserRuleContext<'input, MatchExprContextExt<'input>>;

pub trait MatchExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> MatchExprContextAttrs<'input> for MatchExprContext<'input> {}

pub struct MatchExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub left: Option<Rc<ExprContextAll<'input>>>,
    pub right: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {MatchExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for MatchExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for MatchExprContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_MatchExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_MatchExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for MatchExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_MatchExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for MatchExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for MatchExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for MatchExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for MatchExprContext<'input> {}

impl<'input> MatchExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::MatchExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MatchExprContextExt {
                    left: None,
                    right: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type DefCallExprContext<'input> = BaseParserRuleContext<'input, DefCallExprContextExt<'input>>;

pub trait DefCallExprContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn callSig(&self) -> Option<Rc<CallSigContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> DefCallExprContextAttrs<'input> for DefCallExprContext<'input> {}

pub struct DefCallExprContextExt<'input> {
    base: ExprContextExt<'input>,
    pub sig: Option<Rc<CallSigContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {DefCallExprContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for DefCallExprContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for DefCallExprContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_DefCallExpr(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_DefCallExpr(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for DefCallExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_DefCallExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for DefCallExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for DefCallExprContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for DefCallExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for DefCallExprContext<'input> {}

impl<'input> DefCallExprContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::DefCallExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                DefCallExprContextExt {
                    sig: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LiteralNumberContext<'input> =
    BaseParserRuleContext<'input, LiteralNumberContextExt<'input>>;

pub trait LiteralNumberContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn number(&self) -> Option<Rc<NumberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> LiteralNumberContextAttrs<'input> for LiteralNumberContext<'input> {}

pub struct LiteralNumberContextExt<'input> {
    base: ExprContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LiteralNumberContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LiteralNumberContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for LiteralNumberContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LiteralNumber(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LiteralNumber(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for LiteralNumberContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LiteralNumber(self);
    }
}

impl<'input> CustomRuleContext<'input> for LiteralNumberContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}

impl<'input> Borrow<ExprContextExt<'input>> for LiteralNumberContext<'input> {
    fn borrow(&self) -> &ExprContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ExprContextExt<'input>> for LiteralNumberContext<'input> {
    fn borrow_mut(&mut self) -> &mut ExprContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ExprContextAttrs<'input> for LiteralNumberContext<'input> {}

impl<'input> LiteralNumberContextExt<'input> {
    fn new(ctx: &dyn ExprContextAttrs<'input>) -> Rc<ExprContextAll<'input>> {
        Rc::new(ExprContextAll::LiteralNumberContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LiteralNumberContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn expr(&mut self) -> Result<Rc<ExprContextAll<'input>>, ANTLRError> {
        self.expr_rec(0)
    }

    fn expr_rec(&mut self, _p: isize) -> Result<Rc<ExprContextAll<'input>>, ANTLRError> {
        let recog = self;
        let _parentctx = recog.ctx.take();
        let _parentState = recog.base.get_state();
        let mut _localctx = ExprContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_recursion_rule(_localctx.clone(), 16, RULE_expr, _p);
        let mut _localctx: Rc<ExprContextAll> = _localctx;
        let mut _prevctx = _localctx.clone();
        let _startState = 16;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: isize;
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(196);
                recog.err_handler.sync(&mut recog.base)?;
                match recog.interpreter.adaptive_predict(13, &mut recog.base)? {
                    1 => {
                        {
                            let mut tmp = NestedExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();

                            recog.base.set_state(115);
                            recog.base.match_token(T__10, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(116);
                            recog.expr_rec(0)?;

                            recog.base.set_state(117);
                            recog.base.match_token(T__11, &mut recog.err_handler)?;
                        }
                    }
                    2 => {
                        {
                            let mut tmp = UnMinusExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(119);
                            recog.base.match_token(T__12, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(120);
                            recog.expr_rec(38)?;
                        }
                    }
                    3 => {
                        {
                            let mut tmp = UnPlusExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(121);
                            recog.base.match_token(T__4, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(122);
                            recog.expr_rec(37)?;
                        }
                    }
                    4 => {
                        {
                            let mut tmp = UnBangExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(123);
                            recog.base.match_token(T__6, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(124);
                            recog.expr_rec(36)?;
                        }
                    }
                    5 => {
                        {
                            let mut tmp = UnModExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(125);
                            recog.base.match_token(T__13, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(126);
                            recog.expr_rec(35)?;
                        }
                    }
                    6 => {
                        {
                            let mut tmp = ClassDef2ExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule nsvarident*/
                            recog.base.set_state(127);
                            let tmp = recog.nsvarident()?;
                            if let ExprContextAll::ClassDef2ExprContext(ctx) =
                                cast_mut::<_, ExprContextAll>(&mut _localctx)
                            {
                                ctx.parent = Some(tmp.clone());
                            } else {
                                unreachable!("cant cast");
                            }

                            recog.base.set_state(128);
                            recog.base.match_token(T__14, &mut recog.err_handler)?;

                            /*InvokeRule nsvarident*/
                            recog.base.set_state(129);
                            let tmp = recog.nsvarident()?;
                            if let ExprContextAll::ClassDef2ExprContext(ctx) =
                                cast_mut::<_, ExprContextAll>(&mut _localctx)
                            {
                                ctx.name = Some(tmp.clone());
                            } else {
                                unreachable!("cant cast");
                            }

                            recog.base.set_state(130);
                            recog.base.match_token(T__14, &mut recog.err_handler)?;

                            /*InvokeRule block*/
                            recog.base.set_state(131);
                            recog.block()?;
                        }
                    }
                    7 => {
                        {
                            let mut tmp = ClassDefExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule nsvarident*/
                            recog.base.set_state(133);
                            let tmp = recog.nsvarident()?;
                            if let ExprContextAll::ClassDefExprContext(ctx) =
                                cast_mut::<_, ExprContextAll>(&mut _localctx)
                            {
                                ctx.name = Some(tmp.clone());
                            } else {
                                unreachable!("cant cast");
                            }

                            recog.base.set_state(134);
                            recog.base.match_token(T__14, &mut recog.err_handler)?;

                            /*InvokeRule block*/
                            recog.base.set_state(135);
                            recog.block()?;
                        }
                    }
                    8 => {
                        {
                            let mut tmp = ConstDefExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule nsvarident*/
                            recog.base.set_state(137);
                            recog.nsvarident()?;

                            recog.base.set_state(138);
                            recog.base.match_token(T__14, &mut recog.err_handler)?;

                            /*InvokeRule expr*/
                            recog.base.set_state(139);
                            recog.expr_rec(32)?;
                        }
                    }
                    9 => {
                        {
                            let mut tmp = MethodDefExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule selector*/
                            recog.base.set_state(141);
                            recog.selector()?;

                            recog.base.set_state(142);
                            recog.base.match_token(T__16, &mut recog.err_handler)?;

                            /*InvokeRule block*/
                            recog.base.set_state(143);
                            recog.block()?;
                        }
                    }
                    10 => {
                        {
                            let mut tmp = MethodExtExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule selector*/
                            recog.base.set_state(145);
                            recog.selector()?;

                            recog.base.set_state(146);
                            recog.base.match_token(T__17, &mut recog.err_handler)?;

                            /*InvokeRule block*/
                            recog.base.set_state(147);
                            recog.block()?;
                        }
                    }
                    11 => {
                        {
                            let mut tmp = DefCallExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            {
                                recog.base.set_state(149);
                                recog.base.match_token(T__19, &mut recog.err_handler)?;

                                /*InvokeRule callSig*/
                                recog.base.set_state(150);
                                let tmp = recog.callSig()?;
                                if let ExprContextAll::DefCallExprContext(ctx) =
                                    cast_mut::<_, ExprContextAll>(&mut _localctx)
                                {
                                    ctx.sig = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }
                    }
                    12 => {
                        {
                            let mut tmp = UserListExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(151);
                            recog
                                .base
                                .match_token(USER_LIST_START, &mut recog.err_handler)?;

                            recog.base.set_state(155);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            while (((_la) & !0x3f) == 0
                                && ((1usize << _la)
                                    & ((1usize << T__4)
                                        | (1usize << T__6)
                                        | (1usize << T__10)
                                        | (1usize << T__12)
                                        | (1usize << T__13)
                                        | (1usize << T__19)
                                        | (1usize << T__28)
                                        | (1usize << T__29)))
                                    != 0)
                                || (((_la - 32) & !0x3f) == 0
                                    && ((1usize << (_la - 32))
                                        & ((1usize << (T__31 - 32))
                                            | (1usize << (T__32 - 32))
                                            | (1usize << (T__34 - 32))
                                            | (1usize << (T__35 - 32))
                                            | (1usize << (T__36 - 32))
                                            | (1usize << (IDENT - 32))
                                            | (1usize << (USER_LIST_START - 32))
                                            | (1usize << (SYMBOL - 32))
                                            | (1usize << (STRING - 32))
                                            | (1usize << (REGEXP - 32))
                                            | (1usize << (USER_STRING - 32))
                                            | (1usize << (NUMBER - 32))))
                                        != 0)
                            {
                                {
                                    {
                                        /*InvokeRule expr*/
                                        recog.base.set_state(152);
                                        recog.expr_rec(0)?;
                                    }
                                }
                                recog.base.set_state(157);
                                recog.err_handler.sync(&mut recog.base)?;
                                _la = recog.base.input.la(1);
                            }
                            recog.base.set_state(158);
                            recog.base.match_token(T__11, &mut recog.err_handler)?;
                        }
                    }
                    13 => {
                        {
                            let mut tmp = ListExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(159);
                            recog.base.match_token(T__28, &mut recog.err_handler)?;

                            recog.base.set_state(160);
                            recog.base.match_token(T__10, &mut recog.err_handler)?;

                            recog.base.set_state(164);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            while (((_la) & !0x3f) == 0
                                && ((1usize << _la)
                                    & ((1usize << T__4)
                                        | (1usize << T__6)
                                        | (1usize << T__10)
                                        | (1usize << T__12)
                                        | (1usize << T__13)
                                        | (1usize << T__19)
                                        | (1usize << T__28)
                                        | (1usize << T__29)))
                                    != 0)
                                || (((_la - 32) & !0x3f) == 0
                                    && ((1usize << (_la - 32))
                                        & ((1usize << (T__31 - 32))
                                            | (1usize << (T__32 - 32))
                                            | (1usize << (T__34 - 32))
                                            | (1usize << (T__35 - 32))
                                            | (1usize << (T__36 - 32))
                                            | (1usize << (IDENT - 32))
                                            | (1usize << (USER_LIST_START - 32))
                                            | (1usize << (SYMBOL - 32))
                                            | (1usize << (STRING - 32))
                                            | (1usize << (REGEXP - 32))
                                            | (1usize << (USER_STRING - 32))
                                            | (1usize << (NUMBER - 32))))
                                        != 0)
                            {
                                {
                                    {
                                        /*InvokeRule expr*/
                                        recog.base.set_state(161);
                                        recog.expr_rec(0)?;
                                    }
                                }
                                recog.base.set_state(166);
                                recog.err_handler.sync(&mut recog.base)?;
                                _la = recog.base.input.la(1);
                            }
                            recog.base.set_state(167);
                            recog.base.match_token(T__11, &mut recog.err_handler)?;
                        }
                    }
                    14 => {
                        {
                            let mut tmp = SetExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(168);
                            recog.base.match_token(T__28, &mut recog.err_handler)?;

                            recog.base.set_state(169);
                            recog.base.match_token(T__23, &mut recog.err_handler)?;

                            recog.base.set_state(173);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            while (((_la) & !0x3f) == 0
                                && ((1usize << _la)
                                    & ((1usize << T__4)
                                        | (1usize << T__6)
                                        | (1usize << T__10)
                                        | (1usize << T__12)
                                        | (1usize << T__13)
                                        | (1usize << T__19)
                                        | (1usize << T__28)
                                        | (1usize << T__29)))
                                    != 0)
                                || (((_la - 32) & !0x3f) == 0
                                    && ((1usize << (_la - 32))
                                        & ((1usize << (T__31 - 32))
                                            | (1usize << (T__32 - 32))
                                            | (1usize << (T__34 - 32))
                                            | (1usize << (T__35 - 32))
                                            | (1usize << (T__36 - 32))
                                            | (1usize << (IDENT - 32))
                                            | (1usize << (USER_LIST_START - 32))
                                            | (1usize << (SYMBOL - 32))
                                            | (1usize << (STRING - 32))
                                            | (1usize << (REGEXP - 32))
                                            | (1usize << (USER_STRING - 32))
                                            | (1usize << (NUMBER - 32))))
                                        != 0)
                            {
                                {
                                    {
                                        /*InvokeRule expr*/
                                        recog.base.set_state(170);
                                        recog.expr_rec(0)?;
                                    }
                                }
                                recog.base.set_state(175);
                                recog.err_handler.sync(&mut recog.base)?;
                                _la = recog.base.input.la(1);
                            }
                            recog.base.set_state(176);
                            recog.base.match_token(T__22, &mut recog.err_handler)?;
                        }
                    }
                    15 => {
                        {
                            let mut tmp = DictExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            recog.base.set_state(177);
                            recog.base.match_token(T__28, &mut recog.err_handler)?;

                            recog.base.set_state(178);
                            recog.base.match_token(T__29, &mut recog.err_handler)?;

                            recog.base.set_state(185);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            while (((_la) & !0x3f) == 0
                                && ((1usize << _la)
                                    & ((1usize << T__4)
                                        | (1usize << T__6)
                                        | (1usize << T__10)
                                        | (1usize << T__12)
                                        | (1usize << T__13)
                                        | (1usize << T__19)
                                        | (1usize << T__28)
                                        | (1usize << T__29)))
                                    != 0)
                                || (((_la - 32) & !0x3f) == 0
                                    && ((1usize << (_la - 32))
                                        & ((1usize << (T__31 - 32))
                                            | (1usize << (T__32 - 32))
                                            | (1usize << (T__34 - 32))
                                            | (1usize << (T__35 - 32))
                                            | (1usize << (T__36 - 32))
                                            | (1usize << (IDENT - 32))
                                            | (1usize << (USER_LIST_START - 32))
                                            | (1usize << (SYMBOL - 32))
                                            | (1usize << (STRING - 32))
                                            | (1usize << (REGEXP - 32))
                                            | (1usize << (USER_STRING - 32))
                                            | (1usize << (NUMBER - 32))))
                                        != 0)
                            {
                                {
                                    {
                                        /*InvokeRule expr*/
                                        recog.base.set_state(179);
                                        let tmp = recog.expr_rec(0)?;
                                        if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.expr = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        let temp = if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.expr.clone().unwrap()
                                        } else {
                                            unreachable!("cant cast");
                                        };
                                        if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.k.push(temp);
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.base.set_state(180);
                                        recog.base.match_token(T__5, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(181);
                                        let tmp = recog.expr_rec(0)?;
                                        if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.expr = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        let temp = if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.expr.clone().unwrap()
                                        } else {
                                            unreachable!("cant cast");
                                        };
                                        if let ExprContextAll::DictExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.v.push(temp);
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                recog.base.set_state(187);
                                recog.err_handler.sync(&mut recog.base)?;
                                _la = recog.base.input.la(1);
                            }
                            recog.base.set_state(188);
                            recog.base.match_token(T__30, &mut recog.err_handler)?;
                        }
                    }
                    16 => {
                        {
                            let mut tmp = LiteralNumberContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule number*/
                            recog.base.set_state(189);
                            recog.number()?;
                        }
                    }
                    17 => {
                        {
                            let mut tmp = LiteralStringContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule string*/
                            recog.base.set_state(190);
                            recog.string()?;
                        }
                    }
                    18 => {
                        {
                            let mut tmp = LiteralSymbolContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule symbol*/
                            recog.base.set_state(191);
                            recog.symbol()?;
                        }
                    }
                    19 => {
                        {
                            let mut tmp = BlockExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule block*/
                            recog.base.set_state(192);
                            recog.block()?;
                        }
                    }
                    20 => {
                        {
                            let mut tmp = IdentExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule nsvarident*/
                            recog.base.set_state(193);
                            recog.nsvarident()?;
                        }
                    }
                    21 => {
                        let mut tmp = RegexExprContextExt::new(&**_localctx);
                        recog.ctx = Some(tmp.clone());
                        _localctx = tmp;
                        _prevctx = _localctx.clone();
                        recog.base.set_state(194);
                        recog.base.match_token(REGEXP, &mut recog.err_handler)?;
                    }
                    22 => {
                        {
                            let mut tmp = UserStringExprContextExt::new(&**_localctx);
                            recog.ctx = Some(tmp.clone());
                            _localctx = tmp;
                            _prevctx = _localctx.clone();
                            /*InvokeRule userString*/
                            recog.base.set_state(195);
                            recog.userString()?;
                        }
                    }

                    _ => {}
                }

                let tmp = recog.input.lt(-1).cloned();
                recog.ctx.as_ref().unwrap().set_stop(tmp);
                recog.base.set_state(253);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(15, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        recog.trigger_exit_rule_event();
                        _prevctx = _localctx.clone();
                        {
                            recog.base.set_state(251);
                            recog.err_handler.sync(&mut recog.base)?;
                            match recog.interpreter.adaptive_predict(14, &mut recog.base)? {
                                1 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            RangeExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::RangeExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(198);
                                        if !({ recog.precpred(None, 28) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 28)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(199);
                                        recog.base.match_token(T__18, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(200);
                                        let tmp = recog.expr_rec(29)?;
                                        if let ExprContextAll::RangeExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                2 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            AddExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::AddExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(201);
                                        if !({ recog.precpred(None, 25) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 25)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(202);
                                        recog.base.match_token(T__4, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(203);
                                        let tmp = recog.expr_rec(26)?;
                                        if let ExprContextAll::AddExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                3 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            SubExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::SubExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(204);
                                        if !({ recog.precpred(None, 24) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 24)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(205);
                                        recog.base.match_token(T__12, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(206);
                                        let tmp = recog.expr_rec(25)?;
                                        if let ExprContextAll::SubExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                4 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            DivExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::DivExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(207);
                                        if !({ recog.precpred(None, 23) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 23)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(208);
                                        recog.base.match_token(T__20, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(209);
                                        let tmp = recog.expr_rec(24)?;
                                        if let ExprContextAll::DivExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                5 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            MulExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::MulExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(210);
                                        if !({ recog.precpred(None, 22) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 22)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(211);
                                        recog.base.match_token(T__8, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(212);
                                        let tmp = recog.expr_rec(23)?;
                                        if let ExprContextAll::MulExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                6 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            ModExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::ModExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(213);
                                        if !({ recog.precpred(None, 21) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 21)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(214);
                                        recog.base.match_token(T__13, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(215);
                                        let tmp = recog.expr_rec(22)?;
                                        if let ExprContextAll::ModExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                7 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            MatchExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::MatchExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(216);
                                        if !({ recog.precpred(None, 20) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 20)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(217);
                                        recog.base.match_token(T__21, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(218);
                                        let tmp = recog.expr_rec(21)?;
                                        if let ExprContextAll::MatchExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                8 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            GtEqExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::GtEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(219);
                                        if !({ recog.precpred(None, 19) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 19)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(220);
                                        recog.base.match_token(T__22, &mut recog.err_handler)?;

                                        recog.base.set_state(221);
                                        recog.base.match_token(T__7, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(222);
                                        let tmp = recog.expr_rec(20)?;
                                        if let ExprContextAll::GtEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                9 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            GtExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::GtExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(223);
                                        if !({ recog.precpred(None, 18) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 18)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(224);
                                        recog.base.match_token(T__22, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(225);
                                        let tmp = recog.expr_rec(19)?;
                                        if let ExprContextAll::GtExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                10 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            LtEqExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::LtEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(226);
                                        if !({ recog.precpred(None, 17) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 17)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(227);
                                        recog.base.match_token(T__23, &mut recog.err_handler)?;

                                        recog.base.set_state(228);
                                        recog.base.match_token(T__7, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(229);
                                        let tmp = recog.expr_rec(18)?;
                                        if let ExprContextAll::LtEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                11 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            LtExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::LtExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(230);
                                        if !({ recog.precpred(None, 16) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 16)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(231);
                                        recog.base.match_token(T__23, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(232);
                                        let tmp = recog.expr_rec(17)?;
                                        if let ExprContextAll::LtExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                12 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            AndExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::AndExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(233);
                                        if !({ recog.precpred(None, 15) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 15)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(234);
                                        recog.base.match_token(T__24, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(235);
                                        let tmp = recog.expr_rec(16)?;
                                        if let ExprContextAll::AndExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                13 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            OrExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::OrExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(236);
                                        if !({ recog.precpred(None, 14) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 14)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(237);
                                        recog.base.match_token(T__25, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(238);
                                        let tmp = recog.expr_rec(15)?;
                                        if let ExprContextAll::OrExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                14 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            EqExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::EqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(239);
                                        if !({ recog.precpred(None, 13) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 13)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(240);
                                        recog.base.match_token(T__26, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(241);
                                        let tmp = recog.expr_rec(14)?;
                                        if let ExprContextAll::EqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                15 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            NotEqExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::NotEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.left = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(242);
                                        if !({ recog.precpred(None, 12) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 12)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(243);
                                        recog.base.match_token(T__27, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(244);
                                        let tmp = recog.expr_rec(13)?;
                                        if let ExprContextAll::NotEqExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut _localctx)
                                        {
                                            ctx.right = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                16 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            ClassExtExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(245);
                                        if !({ recog.precpred(None, 31) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 31)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(246);
                                        recog.base.match_token(T__15, &mut recog.err_handler)?;

                                        /*InvokeRule block*/
                                        recog.base.set_state(247);
                                        recog.block()?;
                                    }
                                }
                                17 => {
                                    {
                                        /*recRuleLabeledAltStartAction*/
                                        let mut tmp =
                                            ExprCallExprContextExt::new(&**ExprContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        if let ExprContextAll::ExprCallExprContext(ctx) =
                                            cast_mut::<_, ExprContextAll>(&mut tmp)
                                        {
                                            ctx.subject = Some(_prevctx.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_expr,
                                        );
                                        _localctx = tmp;
                                        recog.base.set_state(248);
                                        if !({ recog.precpred(None, 26) }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 26)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        {
                                            recog.base.set_state(249);
                                            recog
                                                .base
                                                .match_token(T__19, &mut recog.err_handler)?;

                                            /*InvokeRule callSig*/
                                            recog.base.set_state(250);
                                            let tmp = recog.callSig()?;
                                            if let ExprContextAll::ExprCallExprContext(ctx) =
                                                cast_mut::<_, ExprContextAll>(&mut _localctx)
                                            {
                                                ctx.sig = Some(tmp.clone());
                                            } else {
                                                unreachable!("cant cast");
                                            }
                                        }
                                    }
                                }

                                _ => {}
                            }
                        }
                    }
                    recog.base.set_state(255);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(15, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.unroll_recursion_context(_parentctx);

        Ok(_localctx)
    }
}
//------------------- userString ----------------
pub type UserStringContextAll<'input> = UserStringContext<'input>;

pub type UserStringContext<'input> = BaseParserRuleContext<'input, UserStringContextExt<'input>>;

#[derive(Clone)]
pub struct UserStringContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for UserStringContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for UserStringContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_userString(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_userString(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for UserStringContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_userString(self);
    }
}

impl<'input> CustomRuleContext<'input> for UserStringContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_userString
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_userString }
}
antlr_rust::tid! {UserStringContextExt<'a>}

impl<'input> UserStringContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<UserStringContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            UserStringContextExt { ph: PhantomData },
        ))
    }
}

pub trait UserStringContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<UserStringContextExt<'input>>
{
    /// Retrieves first TerminalNode corresponding to token USER_STRING
    /// Returns `None` if there is no child corresponding to token USER_STRING
    fn USER_STRING(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(USER_STRING, 0)
    }
}

impl<'input> UserStringContextAttrs<'input> for UserStringContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn userString(&mut self) -> Result<Rc<UserStringContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = UserStringContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 18, RULE_userString);
        let mut _localctx: Rc<UserStringContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(256);
                recog
                    .base
                    .match_token(USER_STRING, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- callSig ----------------
#[derive(Debug)]
pub enum CallSigContextAll<'input> {
    CallSigWArgContext(CallSigWArgContext<'input>),
    CallSigNoArgContext(CallSigNoArgContext<'input>),
    CallSigNoArgBangContext(CallSigNoArgBangContext<'input>),
    Error(CallSigContext<'input>),
}
antlr_rust::tid! {CallSigContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for CallSigContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for CallSigContextAll<'input> {}

impl<'input> Deref for CallSigContextAll<'input> {
    type Target = dyn CallSigContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use CallSigContextAll::*;
        match self {
            CallSigWArgContext(inner) => inner,
            CallSigNoArgContext(inner) => inner,
            CallSigNoArgBangContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for CallSigContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for CallSigContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type CallSigContext<'input> = BaseParserRuleContext<'input, CallSigContextExt<'input>>;

#[derive(Clone)]
pub struct CallSigContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for CallSigContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for CallSigContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for CallSigContext<'input> {}

impl<'input> CustomRuleContext<'input> for CallSigContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_callSig
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_callSig }
}
antlr_rust::tid! {CallSigContextExt<'a>}

impl<'input> CallSigContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<CallSigContextAll<'input>> {
        Rc::new(CallSigContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                CallSigContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait CallSigContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<CallSigContextExt<'input>>
{
}

impl<'input> CallSigContextAttrs<'input> for CallSigContext<'input> {}

pub type CallSigWArgContext<'input> = BaseParserRuleContext<'input, CallSigWArgContextExt<'input>>;

pub trait CallSigWArgContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident_all(&self) -> Vec<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn ident(&self, i: usize) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> CallSigWArgContextAttrs<'input> for CallSigWArgContext<'input> {}

pub struct CallSigWArgContextExt<'input> {
    base: CallSigContextExt<'input>,
    pub ident: Option<Rc<IdentContextAll<'input>>>,
    pub id: Vec<Rc<IdentContextAll<'input>>>,
    pub expr: Option<Rc<ExprContextAll<'input>>>,
    pub val: Vec<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {CallSigWArgContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for CallSigWArgContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for CallSigWArgContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_CallSigWArg(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_CallSigWArg(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for CallSigWArgContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_CallSigWArg(self);
    }
}

impl<'input> CustomRuleContext<'input> for CallSigWArgContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_callSig
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_callSig }
}

impl<'input> Borrow<CallSigContextExt<'input>> for CallSigWArgContext<'input> {
    fn borrow(&self) -> &CallSigContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<CallSigContextExt<'input>> for CallSigWArgContext<'input> {
    fn borrow_mut(&mut self) -> &mut CallSigContextExt<'input> {
        &mut self.base
    }
}

impl<'input> CallSigContextAttrs<'input> for CallSigWArgContext<'input> {}

impl<'input> CallSigWArgContextExt<'input> {
    fn new(ctx: &dyn CallSigContextAttrs<'input>) -> Rc<CallSigContextAll<'input>> {
        Rc::new(CallSigContextAll::CallSigWArgContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CallSigWArgContextExt {
                    ident: None,
                    expr: None,
                    id: Vec::new(),
                    val: Vec::new(),
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type CallSigNoArgContext<'input> =
    BaseParserRuleContext<'input, CallSigNoArgContextExt<'input>>;

pub trait CallSigNoArgContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> CallSigNoArgContextAttrs<'input> for CallSigNoArgContext<'input> {}

pub struct CallSigNoArgContextExt<'input> {
    base: CallSigContextExt<'input>,
    pub id: Option<Rc<IdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {CallSigNoArgContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for CallSigNoArgContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for CallSigNoArgContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_CallSigNoArg(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_CallSigNoArg(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for CallSigNoArgContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_CallSigNoArg(self);
    }
}

impl<'input> CustomRuleContext<'input> for CallSigNoArgContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_callSig
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_callSig }
}

impl<'input> Borrow<CallSigContextExt<'input>> for CallSigNoArgContext<'input> {
    fn borrow(&self) -> &CallSigContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<CallSigContextExt<'input>> for CallSigNoArgContext<'input> {
    fn borrow_mut(&mut self) -> &mut CallSigContextExt<'input> {
        &mut self.base
    }
}

impl<'input> CallSigContextAttrs<'input> for CallSigNoArgContext<'input> {}

impl<'input> CallSigNoArgContextExt<'input> {
    fn new(ctx: &dyn CallSigContextAttrs<'input>) -> Rc<CallSigContextAll<'input>> {
        Rc::new(CallSigContextAll::CallSigNoArgContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CallSigNoArgContextExt {
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type CallSigNoArgBangContext<'input> =
    BaseParserRuleContext<'input, CallSigNoArgBangContextExt<'input>>;

pub trait CallSigNoArgBangContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> CallSigNoArgBangContextAttrs<'input> for CallSigNoArgBangContext<'input> {}

pub struct CallSigNoArgBangContextExt<'input> {
    base: CallSigContextExt<'input>,
    pub id: Option<Rc<IdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {CallSigNoArgBangContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for CallSigNoArgBangContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for CallSigNoArgBangContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_CallSigNoArgBang(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_CallSigNoArgBang(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for CallSigNoArgBangContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_CallSigNoArgBang(self);
    }
}

impl<'input> CustomRuleContext<'input> for CallSigNoArgBangContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_callSig
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_callSig }
}

impl<'input> Borrow<CallSigContextExt<'input>> for CallSigNoArgBangContext<'input> {
    fn borrow(&self) -> &CallSigContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<CallSigContextExt<'input>> for CallSigNoArgBangContext<'input> {
    fn borrow_mut(&mut self) -> &mut CallSigContextExt<'input> {
        &mut self.base
    }
}

impl<'input> CallSigContextAttrs<'input> for CallSigNoArgBangContext<'input> {}

impl<'input> CallSigNoArgBangContextExt<'input> {
    fn new(ctx: &dyn CallSigContextAttrs<'input>) -> Rc<CallSigContextAll<'input>> {
        Rc::new(CallSigContextAll::CallSigNoArgBangContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CallSigNoArgBangContextExt {
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn callSig(&mut self) -> Result<Rc<CallSigContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = CallSigContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 20, RULE_callSig);
        let mut _localctx: Rc<CallSigContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: isize;
            recog.base.set_state(270);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(17, &mut recog.base)? {
                1 => {
                    let tmp = CallSigWArgContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(262);
                        recog.err_handler.sync(&mut recog.base)?;
                        _alt = 1;
                        loop {
                            match _alt {
                                x if x == 1 => {
                                    {
                                        /*InvokeRule ident*/
                                        recog.base.set_state(258);
                                        let tmp = recog.ident()?;
                                        if let CallSigContextAll::CallSigWArgContext(ctx) =
                                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                        {
                                            ctx.ident = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        let temp =
                                            if let CallSigContextAll::CallSigWArgContext(ctx) =
                                                cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                            {
                                                ctx.ident.clone().unwrap()
                                            } else {
                                                unreachable!("cant cast");
                                            };
                                        if let CallSigContextAll::CallSigWArgContext(ctx) =
                                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                        {
                                            ctx.id.push(temp);
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                        recog.base.set_state(259);
                                        recog.base.match_token(T__5, &mut recog.err_handler)?;

                                        /*InvokeRule expr*/
                                        recog.base.set_state(260);
                                        let tmp = recog.expr_rec(0)?;
                                        if let CallSigContextAll::CallSigWArgContext(ctx) =
                                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                        {
                                            ctx.expr = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        let temp =
                                            if let CallSigContextAll::CallSigWArgContext(ctx) =
                                                cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                            {
                                                ctx.expr.clone().unwrap()
                                            } else {
                                                unreachable!("cant cast");
                                            };
                                        if let CallSigContextAll::CallSigWArgContext(ctx) =
                                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                                        {
                                            ctx.val.push(temp);
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }

                                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                                    &mut recog.base,
                                )))?,
                            }
                            recog.base.set_state(264);
                            recog.err_handler.sync(&mut recog.base)?;
                            _alt = recog.interpreter.adaptive_predict(16, &mut recog.base)?;
                            if _alt == 2 || _alt == INVALID_ALT {
                                break;
                            }
                        }
                    }
                }
                2 => {
                    let tmp = CallSigNoArgContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(266);
                        let tmp = recog.ident()?;
                        if let CallSigContextAll::CallSigNoArgContext(ctx) =
                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                3 => {
                    let tmp = CallSigNoArgBangContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(267);
                        let tmp = recog.ident()?;
                        if let CallSigContextAll::CallSigNoArgBangContext(ctx) =
                            cast_mut::<_, CallSigContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(268);
                        recog.base.match_token(T__6, &mut recog.err_handler)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- nsvarident ----------------
#[derive(Debug)]
pub enum NsvaridentContextAll<'input> {
    InstanceIdentContext(InstanceIdentContext<'input>),
    NamespacedIdentContext(NamespacedIdentContext<'input>),
    LocalIdentContext(LocalIdentContext<'input>),
    Error(NsvaridentContext<'input>),
}
antlr_rust::tid! {NsvaridentContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for NsvaridentContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for NsvaridentContextAll<'input> {}

impl<'input> Deref for NsvaridentContextAll<'input> {
    type Target = dyn NsvaridentContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use NsvaridentContextAll::*;
        match self {
            InstanceIdentContext(inner) => inner,
            NamespacedIdentContext(inner) => inner,
            LocalIdentContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for NsvaridentContextAll<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for NsvaridentContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type NsvaridentContext<'input> = BaseParserRuleContext<'input, NsvaridentContextExt<'input>>;

#[derive(Clone)]
pub struct NsvaridentContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for NsvaridentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for NsvaridentContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NsvaridentContext<'input> {}

impl<'input> CustomRuleContext<'input> for NsvaridentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_nsvarident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_nsvarident }
}
antlr_rust::tid! {NsvaridentContextExt<'a>}

impl<'input> NsvaridentContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<NsvaridentContextAll<'input>> {
        Rc::new(NsvaridentContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                NsvaridentContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait NsvaridentContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<NsvaridentContextExt<'input>>
{
}

impl<'input> NsvaridentContextAttrs<'input> for NsvaridentContext<'input> {}

pub type InstanceIdentContext<'input> =
    BaseParserRuleContext<'input, InstanceIdentContextExt<'input>>;

pub trait InstanceIdentContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> InstanceIdentContextAttrs<'input> for InstanceIdentContext<'input> {}

pub struct InstanceIdentContextExt<'input> {
    base: NsvaridentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {InstanceIdentContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for InstanceIdentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for InstanceIdentContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_InstanceIdent(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_InstanceIdent(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for InstanceIdentContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_InstanceIdent(self);
    }
}

impl<'input> CustomRuleContext<'input> for InstanceIdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_nsvarident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_nsvarident }
}

impl<'input> Borrow<NsvaridentContextExt<'input>> for InstanceIdentContext<'input> {
    fn borrow(&self) -> &NsvaridentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<NsvaridentContextExt<'input>> for InstanceIdentContext<'input> {
    fn borrow_mut(&mut self) -> &mut NsvaridentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> NsvaridentContextAttrs<'input> for InstanceIdentContext<'input> {}

impl<'input> InstanceIdentContextExt<'input> {
    fn new(ctx: &dyn NsvaridentContextAttrs<'input>) -> Rc<NsvaridentContextAll<'input>> {
        Rc::new(NsvaridentContextAll::InstanceIdentContext(
            BaseParserRuleContext::copy_from(
                ctx,
                InstanceIdentContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NamespacedIdentContext<'input> =
    BaseParserRuleContext<'input, NamespacedIdentContextExt<'input>>;

pub trait NamespacedIdentContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn namespace(&self) -> Option<Rc<NamespaceContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> NamespacedIdentContextAttrs<'input> for NamespacedIdentContext<'input> {}

pub struct NamespacedIdentContextExt<'input> {
    base: NsvaridentContextExt<'input>,
    pub ns: Option<Rc<NamespaceContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {NamespacedIdentContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for NamespacedIdentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for NamespacedIdentContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_NamespacedIdent(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_NamespacedIdent(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for NamespacedIdentContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_NamespacedIdent(self);
    }
}

impl<'input> CustomRuleContext<'input> for NamespacedIdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_nsvarident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_nsvarident }
}

impl<'input> Borrow<NsvaridentContextExt<'input>> for NamespacedIdentContext<'input> {
    fn borrow(&self) -> &NsvaridentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<NsvaridentContextExt<'input>> for NamespacedIdentContext<'input> {
    fn borrow_mut(&mut self) -> &mut NsvaridentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> NsvaridentContextAttrs<'input> for NamespacedIdentContext<'input> {}

impl<'input> NamespacedIdentContextExt<'input> {
    fn new(ctx: &dyn NsvaridentContextAttrs<'input>) -> Rc<NsvaridentContextAll<'input>> {
        Rc::new(NsvaridentContextAll::NamespacedIdentContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NamespacedIdentContextExt {
                    ns: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type LocalIdentContext<'input> = BaseParserRuleContext<'input, LocalIdentContextExt<'input>>;

pub trait LocalIdentContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> LocalIdentContextAttrs<'input> for LocalIdentContext<'input> {}

pub struct LocalIdentContextExt<'input> {
    base: NsvaridentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {LocalIdentContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for LocalIdentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for LocalIdentContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_LocalIdent(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_LocalIdent(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for LocalIdentContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_LocalIdent(self);
    }
}

impl<'input> CustomRuleContext<'input> for LocalIdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_nsvarident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_nsvarident }
}

impl<'input> Borrow<NsvaridentContextExt<'input>> for LocalIdentContext<'input> {
    fn borrow(&self) -> &NsvaridentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<NsvaridentContextExt<'input>> for LocalIdentContext<'input> {
    fn borrow_mut(&mut self) -> &mut NsvaridentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> NsvaridentContextAttrs<'input> for LocalIdentContext<'input> {}

impl<'input> LocalIdentContextExt<'input> {
    fn new(ctx: &dyn NsvaridentContextAttrs<'input>) -> Rc<NsvaridentContextAll<'input>> {
        Rc::new(NsvaridentContextAll::LocalIdentContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LocalIdentContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn nsvarident(&mut self) -> Result<Rc<NsvaridentContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = NsvaridentContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 22, RULE_nsvarident);
        let mut _localctx: Rc<NsvaridentContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(278);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.base.input.la(1) {
                T__32 => {
                    let tmp = NamespacedIdentContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        /*InvokeRule namespace*/
                        recog.base.set_state(272);
                        let tmp = recog.namespace()?;
                        if let NsvaridentContextAll::NamespacedIdentContext(ctx) =
                            cast_mut::<_, NsvaridentContextAll>(&mut _localctx)
                        {
                            ctx.ns = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        /*InvokeRule ident*/
                        recog.base.set_state(273);
                        recog.ident()?;
                    }
                }

                T__31 => {
                    let tmp = InstanceIdentContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(275);
                        recog.base.match_token(T__31, &mut recog.err_handler)?;

                        /*InvokeRule ident*/
                        recog.base.set_state(276);
                        recog.ident()?;
                    }
                }

                T__34 | T__35 | T__36 | IDENT => {
                    let tmp = LocalIdentContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(277);
                        recog.ident()?;
                    }
                }

                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                    &mut recog.base,
                )))?,
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- namespace ----------------
#[derive(Debug)]
pub enum NamespaceContextAll<'input> {
    FullNSContext(FullNSContext<'input>),
    RootNSContext(RootNSContext<'input>),
    Error(NamespaceContext<'input>),
}
antlr_rust::tid! {NamespaceContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for NamespaceContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for NamespaceContextAll<'input> {}

impl<'input> Deref for NamespaceContextAll<'input> {
    type Target = dyn NamespaceContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use NamespaceContextAll::*;
        match self {
            FullNSContext(inner) => inner,
            RootNSContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NamespaceContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for NamespaceContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type NamespaceContext<'input> = BaseParserRuleContext<'input, NamespaceContextExt<'input>>;

#[derive(Clone)]
pub struct NamespaceContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for NamespaceContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for NamespaceContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NamespaceContext<'input> {}

impl<'input> CustomRuleContext<'input> for NamespaceContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_namespace
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_namespace }
}
antlr_rust::tid! {NamespaceContextExt<'a>}

impl<'input> NamespaceContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<NamespaceContextAll<'input>> {
        Rc::new(NamespaceContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                NamespaceContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait NamespaceContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<NamespaceContextExt<'input>>
{
}

impl<'input> NamespaceContextAttrs<'input> for NamespaceContext<'input> {}

pub type FullNSContext<'input> = BaseParserRuleContext<'input, FullNSContextExt<'input>>;

pub trait FullNSContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident_all(&self) -> Vec<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn ident(&self, i: usize) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> FullNSContextAttrs<'input> for FullNSContext<'input> {}

pub struct FullNSContextExt<'input> {
    base: NamespaceContextExt<'input>,
    pub first: Option<Rc<IdentContextAll<'input>>>,
    pub rest: Option<Rc<IdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {FullNSContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for FullNSContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for FullNSContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_FullNS(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_FullNS(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for FullNSContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_FullNS(self);
    }
}

impl<'input> CustomRuleContext<'input> for FullNSContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_namespace
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_namespace }
}

impl<'input> Borrow<NamespaceContextExt<'input>> for FullNSContext<'input> {
    fn borrow(&self) -> &NamespaceContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<NamespaceContextExt<'input>> for FullNSContext<'input> {
    fn borrow_mut(&mut self) -> &mut NamespaceContextExt<'input> {
        &mut self.base
    }
}

impl<'input> NamespaceContextAttrs<'input> for FullNSContext<'input> {}

impl<'input> FullNSContextExt<'input> {
    fn new(ctx: &dyn NamespaceContextAttrs<'input>) -> Rc<NamespaceContextAll<'input>> {
        Rc::new(NamespaceContextAll::FullNSContext(
            BaseParserRuleContext::copy_from(
                ctx,
                FullNSContextExt {
                    first: None,
                    rest: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type RootNSContext<'input> = BaseParserRuleContext<'input, RootNSContextExt<'input>>;

pub trait RootNSContextAttrs<'input>: BuildingBlocksParserContext<'input> {}

impl<'input> RootNSContextAttrs<'input> for RootNSContext<'input> {}

pub struct RootNSContextExt<'input> {
    base: NamespaceContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {RootNSContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for RootNSContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for RootNSContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_RootNS(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_RootNS(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for RootNSContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_RootNS(self);
    }
}

impl<'input> CustomRuleContext<'input> for RootNSContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_namespace
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_namespace }
}

impl<'input> Borrow<NamespaceContextExt<'input>> for RootNSContext<'input> {
    fn borrow(&self) -> &NamespaceContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<NamespaceContextExt<'input>> for RootNSContext<'input> {
    fn borrow_mut(&mut self) -> &mut NamespaceContextExt<'input> {
        &mut self.base
    }
}

impl<'input> NamespaceContextAttrs<'input> for RootNSContext<'input> {}

impl<'input> RootNSContextExt<'input> {
    fn new(ctx: &dyn NamespaceContextAttrs<'input>) -> Rc<NamespaceContextAll<'input>> {
        Rc::new(NamespaceContextAll::RootNSContext(
            BaseParserRuleContext::copy_from(
                ctx,
                RootNSContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn namespace(&mut self) -> Result<Rc<NamespaceContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = NamespaceContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 24, RULE_namespace);
        let mut _localctx: Rc<NamespaceContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: isize;
            recog.base.set_state(297);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(21, &mut recog.base)? {
                1 => {
                    let tmp = FullNSContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(280);
                        recog.base.match_token(T__32, &mut recog.err_handler)?;

                        /*InvokeRule ident*/
                        recog.base.set_state(281);
                        let tmp = recog.ident()?;
                        if let NamespaceContextAll::FullNSContext(ctx) =
                            cast_mut::<_, NamespaceContextAll>(&mut _localctx)
                        {
                            ctx.first = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(286);
                        recog.err_handler.sync(&mut recog.base)?;
                        _alt = recog.interpreter.adaptive_predict(19, &mut recog.base)?;
                        while { _alt != 2 && _alt != INVALID_ALT } {
                            if _alt == 1 {
                                {
                                    {
                                        recog.base.set_state(282);
                                        recog.base.match_token(T__20, &mut recog.err_handler)?;

                                        /*InvokeRule ident*/
                                        recog.base.set_state(283);
                                        let tmp = recog.ident()?;
                                        if let NamespaceContextAll::FullNSContext(ctx) =
                                            cast_mut::<_, NamespaceContextAll>(&mut _localctx)
                                        {
                                            ctx.rest = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                            }
                            recog.base.set_state(288);
                            recog.err_handler.sync(&mut recog.base)?;
                            _alt = recog.interpreter.adaptive_predict(19, &mut recog.base)?;
                        }
                        recog.base.set_state(290);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == T__20 {
                            {
                                recog.base.set_state(289);
                                recog.base.match_token(T__20, &mut recog.err_handler)?;
                            }
                        }

                        recog.base.set_state(292);
                        recog.base.match_token(T__33, &mut recog.err_handler)?;
                    }
                }
                2 => {
                    let tmp = RootNSContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(294);
                        recog.base.match_token(T__32, &mut recog.err_handler)?;

                        recog.base.set_state(295);
                        recog.base.match_token(T__20, &mut recog.err_handler)?;

                        recog.base.set_state(296);
                        recog.base.match_token(T__33, &mut recog.err_handler)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- keyword ----------------
pub type KeywordContextAll<'input> = KeywordContext<'input>;

pub type KeywordContext<'input> = BaseParserRuleContext<'input, KeywordContextExt<'input>>;

#[derive(Clone)]
pub struct KeywordContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for KeywordContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for KeywordContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_keyword(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_keyword(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for KeywordContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_keyword(self);
    }
}

impl<'input> CustomRuleContext<'input> for KeywordContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_keyword
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_keyword }
}
antlr_rust::tid! {KeywordContextExt<'a>}

impl<'input> KeywordContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<KeywordContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            KeywordContextExt { ph: PhantomData },
        ))
    }
}

pub trait KeywordContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<KeywordContextExt<'input>>
{
}

impl<'input> KeywordContextAttrs<'input> for KeywordContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn keyword(&mut self) -> Result<Rc<KeywordContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = KeywordContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 26, RULE_keyword);
        let mut _localctx: Rc<KeywordContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(299);
                _la = recog.base.input.la(1);
                if {
                    !(((_la - 35) & !0x3f) == 0
                        && ((1usize << (_la - 35))
                            & ((1usize << (T__34 - 35))
                                | (1usize << (T__35 - 35))
                                | (1usize << (T__36 - 35))))
                            != 0)
                } {
                    recog.err_handler.recover_inline(&mut recog.base)?;
                } else {
                    if recog.base.input.la(1) == TOKEN_EOF {
                        recog.base.matched_eof = true
                    };
                    recog.err_handler.report_match(&mut recog.base);
                    recog.base.consume(&mut recog.err_handler);
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- block ----------------
#[derive(Debug)]
pub enum BlockContextAll<'input> {
    BlockNoDeclsContext(BlockNoDeclsContext<'input>),
    NamedBlockWDeclsContext(NamedBlockWDeclsContext<'input>),
    BlockWDeclsContext(BlockWDeclsContext<'input>),
    Error(BlockContext<'input>),
}
antlr_rust::tid! {BlockContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for BlockContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for BlockContextAll<'input> {}

impl<'input> Deref for BlockContextAll<'input> {
    type Target = dyn BlockContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use BlockContextAll::*;
        match self {
            BlockNoDeclsContext(inner) => inner,
            NamedBlockWDeclsContext(inner) => inner,
            BlockWDeclsContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type BlockContext<'input> = BaseParserRuleContext<'input, BlockContextExt<'input>>;

#[derive(Clone)]
pub struct BlockContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for BlockContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockContext<'input> {}

impl<'input> CustomRuleContext<'input> for BlockContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_block
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_block }
}
antlr_rust::tid! {BlockContextExt<'a>}

impl<'input> BlockContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<BlockContextAll<'input>> {
        Rc::new(BlockContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                BlockContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait BlockContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<BlockContextExt<'input>>
{
}

impl<'input> BlockContextAttrs<'input> for BlockContext<'input> {}

pub type BlockNoDeclsContext<'input> =
    BaseParserRuleContext<'input, BlockNoDeclsContextExt<'input>>;

pub trait BlockNoDeclsContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn stmt_all(&self) -> Vec<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn stmt(&self, i: usize) -> Option<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> BlockNoDeclsContextAttrs<'input> for BlockNoDeclsContext<'input> {}

pub struct BlockNoDeclsContextExt<'input> {
    base: BlockContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockNoDeclsContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockNoDeclsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockNoDeclsContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockNoDecls(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockNoDecls(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockNoDeclsContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockNoDecls(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockNoDeclsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_block
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_block }
}

impl<'input> Borrow<BlockContextExt<'input>> for BlockNoDeclsContext<'input> {
    fn borrow(&self) -> &BlockContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockContextExt<'input>> for BlockNoDeclsContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockContextAttrs<'input> for BlockNoDeclsContext<'input> {}

impl<'input> BlockNoDeclsContextExt<'input> {
    fn new(ctx: &dyn BlockContextAttrs<'input>) -> Rc<BlockContextAll<'input>> {
        Rc::new(BlockContextAll::BlockNoDeclsContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockNoDeclsContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NamedBlockWDeclsContext<'input> =
    BaseParserRuleContext<'input, NamedBlockWDeclsContextExt<'input>>;

pub trait NamedBlockWDeclsContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn symbol(&self) -> Option<Rc<SymbolContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn blockDecls(&self) -> Option<Rc<BlockDeclsContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn stmt_all(&self) -> Vec<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn stmt(&self, i: usize) -> Option<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> NamedBlockWDeclsContextAttrs<'input> for NamedBlockWDeclsContext<'input> {}

pub struct NamedBlockWDeclsContextExt<'input> {
    base: BlockContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {NamedBlockWDeclsContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for NamedBlockWDeclsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for NamedBlockWDeclsContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_NamedBlockWDecls(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_NamedBlockWDecls(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for NamedBlockWDeclsContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_NamedBlockWDecls(self);
    }
}

impl<'input> CustomRuleContext<'input> for NamedBlockWDeclsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_block
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_block }
}

impl<'input> Borrow<BlockContextExt<'input>> for NamedBlockWDeclsContext<'input> {
    fn borrow(&self) -> &BlockContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockContextExt<'input>> for NamedBlockWDeclsContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockContextAttrs<'input> for NamedBlockWDeclsContext<'input> {}

impl<'input> NamedBlockWDeclsContextExt<'input> {
    fn new(ctx: &dyn BlockContextAttrs<'input>) -> Rc<BlockContextAll<'input>> {
        Rc::new(BlockContextAll::NamedBlockWDeclsContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NamedBlockWDeclsContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockWDeclsContext<'input> = BaseParserRuleContext<'input, BlockWDeclsContextExt<'input>>;

pub trait BlockWDeclsContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn blockDecls(&self) -> Option<Rc<BlockDeclsContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn stmt_all(&self) -> Vec<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn stmt(&self, i: usize) -> Option<Rc<StmtContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> BlockWDeclsContextAttrs<'input> for BlockWDeclsContext<'input> {}

pub struct BlockWDeclsContextExt<'input> {
    base: BlockContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockWDeclsContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockWDeclsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockWDeclsContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockWDecls(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockWDecls(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockWDeclsContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockWDecls(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockWDeclsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_block
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_block }
}

impl<'input> Borrow<BlockContextExt<'input>> for BlockWDeclsContext<'input> {
    fn borrow(&self) -> &BlockContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockContextExt<'input>> for BlockWDeclsContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockContextAttrs<'input> for BlockWDeclsContext<'input> {}

impl<'input> BlockWDeclsContextExt<'input> {
    fn new(ctx: &dyn BlockContextAttrs<'input>) -> Rc<BlockContextAll<'input>> {
        Rc::new(BlockContextAll::BlockWDeclsContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockWDeclsContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn block(&mut self) -> Result<Rc<BlockContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = BlockContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 28, RULE_block);
        let mut _localctx: Rc<BlockContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(339);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(28, &mut recog.base)? {
                1 => {
                    let tmp = NamedBlockWDeclsContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(301);
                        recog.base.match_token(T__29, &mut recog.err_handler)?;

                        /*InvokeRule symbol*/
                        recog.base.set_state(302);
                        recog.symbol()?;

                        /*InvokeRule blockDecls*/
                        recog.base.set_state(303);
                        recog.blockDecls()?;

                        recog.base.set_state(310);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while (((_la) & !0x3f) == 0
                            && ((1usize << _la)
                                & ((1usize << T__1)
                                    | (1usize << T__2)
                                    | (1usize << T__3)
                                    | (1usize << T__4)
                                    | (1usize << T__6)
                                    | (1usize << T__8)
                                    | (1usize << T__9)
                                    | (1usize << T__10)
                                    | (1usize << T__12)
                                    | (1usize << T__13)
                                    | (1usize << T__19)
                                    | (1usize << T__28)
                                    | (1usize << T__29)))
                                != 0)
                            || (((_la - 32) & !0x3f) == 0
                                && ((1usize << (_la - 32))
                                    & ((1usize << (T__31 - 32))
                                        | (1usize << (T__32 - 32))
                                        | (1usize << (T__34 - 32))
                                        | (1usize << (T__35 - 32))
                                        | (1usize << (T__36 - 32))
                                        | (1usize << (IDENT - 32))
                                        | (1usize << (USER_LIST_START - 32))
                                        | (1usize << (SYMBOL - 32))
                                        | (1usize << (STRING - 32))
                                        | (1usize << (REGEXP - 32))
                                        | (1usize << (USER_STRING - 32))
                                        | (1usize << (METHOD_RETURN - 32))
                                        | (1usize << (YIELD_RETURN - 32))
                                        | (1usize << (BLOCK_RETURN - 32))
                                        | (1usize << (NUMBER - 32))))
                                    != 0)
                        {
                            {
                                {
                                    /*InvokeRule stmt*/
                                    recog.base.set_state(304);
                                    recog.stmt()?;

                                    recog.base.set_state(306);
                                    recog.err_handler.sync(&mut recog.base)?;
                                    _la = recog.base.input.la(1);
                                    if _la == T__0 {
                                        {
                                            recog.base.set_state(305);
                                            recog.base.match_token(T__0, &mut recog.err_handler)?;
                                        }
                                    }
                                }
                            }
                            recog.base.set_state(312);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(313);
                        recog.base.match_token(T__30, &mut recog.err_handler)?;
                    }
                }
                2 => {
                    let tmp = BlockWDeclsContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(315);
                        recog.base.match_token(T__29, &mut recog.err_handler)?;

                        /*InvokeRule blockDecls*/
                        recog.base.set_state(316);
                        recog.blockDecls()?;

                        recog.base.set_state(323);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while (((_la) & !0x3f) == 0
                            && ((1usize << _la)
                                & ((1usize << T__1)
                                    | (1usize << T__2)
                                    | (1usize << T__3)
                                    | (1usize << T__4)
                                    | (1usize << T__6)
                                    | (1usize << T__8)
                                    | (1usize << T__9)
                                    | (1usize << T__10)
                                    | (1usize << T__12)
                                    | (1usize << T__13)
                                    | (1usize << T__19)
                                    | (1usize << T__28)
                                    | (1usize << T__29)))
                                != 0)
                            || (((_la - 32) & !0x3f) == 0
                                && ((1usize << (_la - 32))
                                    & ((1usize << (T__31 - 32))
                                        | (1usize << (T__32 - 32))
                                        | (1usize << (T__34 - 32))
                                        | (1usize << (T__35 - 32))
                                        | (1usize << (T__36 - 32))
                                        | (1usize << (IDENT - 32))
                                        | (1usize << (USER_LIST_START - 32))
                                        | (1usize << (SYMBOL - 32))
                                        | (1usize << (STRING - 32))
                                        | (1usize << (REGEXP - 32))
                                        | (1usize << (USER_STRING - 32))
                                        | (1usize << (METHOD_RETURN - 32))
                                        | (1usize << (YIELD_RETURN - 32))
                                        | (1usize << (BLOCK_RETURN - 32))
                                        | (1usize << (NUMBER - 32))))
                                    != 0)
                        {
                            {
                                {
                                    /*InvokeRule stmt*/
                                    recog.base.set_state(317);
                                    recog.stmt()?;

                                    recog.base.set_state(319);
                                    recog.err_handler.sync(&mut recog.base)?;
                                    _la = recog.base.input.la(1);
                                    if _la == T__0 {
                                        {
                                            recog.base.set_state(318);
                                            recog.base.match_token(T__0, &mut recog.err_handler)?;
                                        }
                                    }
                                }
                            }
                            recog.base.set_state(325);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(326);
                        recog.base.match_token(T__30, &mut recog.err_handler)?;
                    }
                }
                3 => {
                    let tmp = BlockNoDeclsContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        recog.base.set_state(328);
                        recog.base.match_token(T__29, &mut recog.err_handler)?;

                        recog.base.set_state(335);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while (((_la) & !0x3f) == 0
                            && ((1usize << _la)
                                & ((1usize << T__1)
                                    | (1usize << T__2)
                                    | (1usize << T__3)
                                    | (1usize << T__4)
                                    | (1usize << T__6)
                                    | (1usize << T__8)
                                    | (1usize << T__9)
                                    | (1usize << T__10)
                                    | (1usize << T__12)
                                    | (1usize << T__13)
                                    | (1usize << T__19)
                                    | (1usize << T__28)
                                    | (1usize << T__29)))
                                != 0)
                            || (((_la - 32) & !0x3f) == 0
                                && ((1usize << (_la - 32))
                                    & ((1usize << (T__31 - 32))
                                        | (1usize << (T__32 - 32))
                                        | (1usize << (T__34 - 32))
                                        | (1usize << (T__35 - 32))
                                        | (1usize << (T__36 - 32))
                                        | (1usize << (IDENT - 32))
                                        | (1usize << (USER_LIST_START - 32))
                                        | (1usize << (SYMBOL - 32))
                                        | (1usize << (STRING - 32))
                                        | (1usize << (REGEXP - 32))
                                        | (1usize << (USER_STRING - 32))
                                        | (1usize << (METHOD_RETURN - 32))
                                        | (1usize << (YIELD_RETURN - 32))
                                        | (1usize << (BLOCK_RETURN - 32))
                                        | (1usize << (NUMBER - 32))))
                                    != 0)
                        {
                            {
                                {
                                    /*InvokeRule stmt*/
                                    recog.base.set_state(329);
                                    recog.stmt()?;

                                    recog.base.set_state(331);
                                    recog.err_handler.sync(&mut recog.base)?;
                                    _la = recog.base.input.la(1);
                                    if _la == T__0 {
                                        {
                                            recog.base.set_state(330);
                                            recog.base.match_token(T__0, &mut recog.err_handler)?;
                                        }
                                    }
                                }
                            }
                            recog.base.set_state(337);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(338);
                        recog.base.match_token(T__30, &mut recog.err_handler)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- blockDecls ----------------
pub type BlockDeclsContextAll<'input> = BlockDeclsContext<'input>;

pub type BlockDeclsContext<'input> = BaseParserRuleContext<'input, BlockDeclsContextExt<'input>>;

#[derive(Clone)]
pub struct BlockDeclsContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for BlockDeclsContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockDeclsContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_blockDecls(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_blockDecls(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockDeclsContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_blockDecls(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockDeclsContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockDecls
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockDecls }
}
antlr_rust::tid! {BlockDeclsContextExt<'a>}

impl<'input> BlockDeclsContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<BlockDeclsContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            BlockDeclsContextExt { ph: PhantomData },
        ))
    }
}

pub trait BlockDeclsContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<BlockDeclsContextExt<'input>>
{
    fn blockArg_all(&self) -> Vec<Rc<BlockArgContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn blockArg(&self, i: usize) -> Option<Rc<BlockArgContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    fn block(&self) -> Option<Rc<BlockContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn blockDecl_all(&self) -> Vec<Rc<BlockDeclContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn blockDecl(&self, i: usize) -> Option<Rc<BlockDeclContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
}

impl<'input> BlockDeclsContextAttrs<'input> for BlockDeclsContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn blockDecls(&mut self) -> Result<Rc<BlockDeclsContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = BlockDeclsContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 30, RULE_blockDecls);
        let mut _localctx: Rc<BlockDeclsContextAll> = _localctx;
        let mut _la: isize = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(370);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(34, &mut recog.base)? {
                1 => {
                    //recog.base.enter_outer_alt(_localctx.clone(), 1);
                    recog.base.enter_outer_alt(None, 1);
                    {
                        recog.base.set_state(341);
                        recog.base.match_token(T__37, &mut recog.err_handler)?;

                        recog.base.set_state(345);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while ((_la - 10) & !0x3f) == 0
                            && ((1usize << (_la - 10))
                                & ((1usize << (T__9 - 10))
                                    | (1usize << (T__31 - 10))
                                    | (1usize << (T__34 - 10))
                                    | (1usize << (T__35 - 10))
                                    | (1usize << (T__36 - 10))
                                    | (1usize << (IDENT - 10))))
                                != 0
                        {
                            {
                                {
                                    /*InvokeRule blockArg*/
                                    recog.base.set_state(342);
                                    recog.blockArg()?;
                                }
                            }
                            recog.base.set_state(347);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(349);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == T__29 {
                            {
                                /*InvokeRule block*/
                                recog.base.set_state(348);
                                recog.block()?;
                            }
                        }

                        recog.base.set_state(351);
                        recog.base.match_token(T__12, &mut recog.err_handler)?;

                        recog.base.set_state(355);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while ((_la - 32) & !0x3f) == 0
                            && ((1usize << (_la - 32))
                                & ((1usize << (T__31 - 32))
                                    | (1usize << (T__34 - 32))
                                    | (1usize << (T__35 - 32))
                                    | (1usize << (T__36 - 32))
                                    | (1usize << (IDENT - 32))))
                                != 0
                        {
                            {
                                {
                                    /*InvokeRule blockDecl*/
                                    recog.base.set_state(352);
                                    recog.blockDecl()?;
                                }
                            }
                            recog.base.set_state(357);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(358);
                        recog.base.match_token(T__37, &mut recog.err_handler)?;
                    }
                }
                2 => {
                    //recog.base.enter_outer_alt(_localctx.clone(), 2);
                    recog.base.enter_outer_alt(None, 2);
                    {
                        recog.base.set_state(359);
                        recog.base.match_token(T__37, &mut recog.err_handler)?;

                        recog.base.set_state(363);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while ((_la - 10) & !0x3f) == 0
                            && ((1usize << (_la - 10))
                                & ((1usize << (T__9 - 10))
                                    | (1usize << (T__31 - 10))
                                    | (1usize << (T__34 - 10))
                                    | (1usize << (T__35 - 10))
                                    | (1usize << (T__36 - 10))
                                    | (1usize << (IDENT - 10))))
                                != 0
                        {
                            {
                                {
                                    /*InvokeRule blockArg*/
                                    recog.base.set_state(360);
                                    recog.blockArg()?;
                                }
                            }
                            recog.base.set_state(365);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(367);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == T__29 {
                            {
                                /*InvokeRule block*/
                                recog.base.set_state(366);
                                recog.block()?;
                            }
                        }

                        recog.base.set_state(369);
                        recog.base.match_token(T__37, &mut recog.err_handler)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- blockArg ----------------
#[derive(Debug)]
pub enum BlockArgContextAll<'input> {
    BlockArgTypedContext(BlockArgTypedContext<'input>),
    BlockArgIgnoredContext(BlockArgIgnoredContext<'input>),
    BlockArgUntypedContext(BlockArgUntypedContext<'input>),
    Error(BlockArgContext<'input>),
}
antlr_rust::tid! {BlockArgContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for BlockArgContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for BlockArgContextAll<'input> {}

impl<'input> Deref for BlockArgContextAll<'input> {
    type Target = dyn BlockArgContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use BlockArgContextAll::*;
        match self {
            BlockArgTypedContext(inner) => inner,
            BlockArgIgnoredContext(inner) => inner,
            BlockArgUntypedContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockArgContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockArgContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type BlockArgContext<'input> = BaseParserRuleContext<'input, BlockArgContextExt<'input>>;

#[derive(Clone)]
pub struct BlockArgContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for BlockArgContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockArgContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockArgContext<'input> {}

impl<'input> CustomRuleContext<'input> for BlockArgContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockArg
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockArg }
}
antlr_rust::tid! {BlockArgContextExt<'a>}

impl<'input> BlockArgContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<BlockArgContextAll<'input>> {
        Rc::new(BlockArgContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                BlockArgContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait BlockArgContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<BlockArgContextExt<'input>>
{
}

impl<'input> BlockArgContextAttrs<'input> for BlockArgContext<'input> {}

pub type BlockArgTypedContext<'input> =
    BaseParserRuleContext<'input, BlockArgTypedContextExt<'input>>;

pub trait BlockArgTypedContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn argident(&self) -> Option<Rc<ArgidentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockArgTypedContextAttrs<'input> for BlockArgTypedContext<'input> {}

pub struct BlockArgTypedContextExt<'input> {
    base: BlockArgContextExt<'input>,
    pub name: Option<Rc<ArgidentContextAll<'input>>>,
    pub argtype: Option<Rc<IdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockArgTypedContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockArgTypedContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockArgTypedContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockArgTyped(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockArgTyped(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for BlockArgTypedContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockArgTyped(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockArgTypedContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockArg
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockArg }
}

impl<'input> Borrow<BlockArgContextExt<'input>> for BlockArgTypedContext<'input> {
    fn borrow(&self) -> &BlockArgContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockArgContextExt<'input>> for BlockArgTypedContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockArgContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockArgContextAttrs<'input> for BlockArgTypedContext<'input> {}

impl<'input> BlockArgTypedContextExt<'input> {
    fn new(ctx: &dyn BlockArgContextAttrs<'input>) -> Rc<BlockArgContextAll<'input>> {
        Rc::new(BlockArgContextAll::BlockArgTypedContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockArgTypedContextExt {
                    name: None,
                    argtype: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockArgIgnoredContext<'input> =
    BaseParserRuleContext<'input, BlockArgIgnoredContextExt<'input>>;

pub trait BlockArgIgnoredContextAttrs<'input>: BuildingBlocksParserContext<'input> {}

impl<'input> BlockArgIgnoredContextAttrs<'input> for BlockArgIgnoredContext<'input> {}

pub struct BlockArgIgnoredContextExt<'input> {
    base: BlockArgContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockArgIgnoredContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockArgIgnoredContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockArgIgnoredContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockArgIgnored(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockArgIgnored(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for BlockArgIgnoredContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockArgIgnored(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockArgIgnoredContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockArg
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockArg }
}

impl<'input> Borrow<BlockArgContextExt<'input>> for BlockArgIgnoredContext<'input> {
    fn borrow(&self) -> &BlockArgContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockArgContextExt<'input>> for BlockArgIgnoredContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockArgContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockArgContextAttrs<'input> for BlockArgIgnoredContext<'input> {}

impl<'input> BlockArgIgnoredContextExt<'input> {
    fn new(ctx: &dyn BlockArgContextAttrs<'input>) -> Rc<BlockArgContextAll<'input>> {
        Rc::new(BlockArgContextAll::BlockArgIgnoredContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockArgIgnoredContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockArgUntypedContext<'input> =
    BaseParserRuleContext<'input, BlockArgUntypedContextExt<'input>>;

pub trait BlockArgUntypedContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn argident(&self) -> Option<Rc<ArgidentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockArgUntypedContextAttrs<'input> for BlockArgUntypedContext<'input> {}

pub struct BlockArgUntypedContextExt<'input> {
    base: BlockArgContextExt<'input>,
    pub name: Option<Rc<ArgidentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockArgUntypedContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockArgUntypedContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockArgUntypedContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockArgUntyped(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockArgUntyped(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for BlockArgUntypedContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockArgUntyped(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockArgUntypedContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockArg
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockArg }
}

impl<'input> Borrow<BlockArgContextExt<'input>> for BlockArgUntypedContext<'input> {
    fn borrow(&self) -> &BlockArgContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockArgContextExt<'input>> for BlockArgUntypedContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockArgContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockArgContextAttrs<'input> for BlockArgUntypedContext<'input> {}

impl<'input> BlockArgUntypedContextExt<'input> {
    fn new(ctx: &dyn BlockArgContextAttrs<'input>) -> Rc<BlockArgContextAll<'input>> {
        Rc::new(BlockArgContextAll::BlockArgUntypedContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockArgUntypedContextExt {
                    name: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn blockArg(&mut self) -> Result<Rc<BlockArgContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = BlockArgContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 32, RULE_blockArg);
        let mut _localctx: Rc<BlockArgContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(378);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(35, &mut recog.base)? {
                1 => {
                    let tmp = BlockArgIgnoredContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(372);
                        recog.base.match_token(T__9, &mut recog.err_handler)?;
                    }
                }
                2 => {
                    let tmp = BlockArgTypedContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        /*InvokeRule argident*/
                        recog.base.set_state(373);
                        let tmp = recog.argident()?;
                        if let BlockArgContextAll::BlockArgTypedContext(ctx) =
                            cast_mut::<_, BlockArgContextAll>(&mut _localctx)
                        {
                            ctx.name = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(374);
                        recog.base.match_token(T__5, &mut recog.err_handler)?;

                        /*InvokeRule ident*/
                        recog.base.set_state(375);
                        let tmp = recog.ident()?;
                        if let BlockArgContextAll::BlockArgTypedContext(ctx) =
                            cast_mut::<_, BlockArgContextAll>(&mut _localctx)
                        {
                            ctx.argtype = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                3 => {
                    let tmp = BlockArgUntypedContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3);
                    _localctx = tmp;
                    {
                        /*InvokeRule argident*/
                        recog.base.set_state(377);
                        let tmp = recog.argident()?;
                        if let BlockArgContextAll::BlockArgUntypedContext(ctx) =
                            cast_mut::<_, BlockArgContextAll>(&mut _localctx)
                        {
                            ctx.name = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- blockDecl ----------------
#[derive(Debug)]
pub enum BlockDeclContextAll<'input> {
    BlockDeclUntypedContext(BlockDeclUntypedContext<'input>),
    BlockDeclTypedContext(BlockDeclTypedContext<'input>),
    Error(BlockDeclContext<'input>),
}
antlr_rust::tid! {BlockDeclContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for BlockDeclContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for BlockDeclContextAll<'input> {}

impl<'input> Deref for BlockDeclContextAll<'input> {
    type Target = dyn BlockDeclContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use BlockDeclContextAll::*;
        match self {
            BlockDeclUntypedContext(inner) => inner,
            BlockDeclTypedContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockDeclContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockDeclContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type BlockDeclContext<'input> = BaseParserRuleContext<'input, BlockDeclContextExt<'input>>;

#[derive(Clone)]
pub struct BlockDeclContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for BlockDeclContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for BlockDeclContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for BlockDeclContext<'input> {}

impl<'input> CustomRuleContext<'input> for BlockDeclContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockDecl
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockDecl }
}
antlr_rust::tid! {BlockDeclContextExt<'a>}

impl<'input> BlockDeclContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<BlockDeclContextAll<'input>> {
        Rc::new(BlockDeclContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                BlockDeclContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait BlockDeclContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<BlockDeclContextExt<'input>>
{
}

impl<'input> BlockDeclContextAttrs<'input> for BlockDeclContext<'input> {}

pub type BlockDeclUntypedContext<'input> =
    BaseParserRuleContext<'input, BlockDeclUntypedContextExt<'input>>;

pub trait BlockDeclUntypedContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn argident(&self) -> Option<Rc<ArgidentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockDeclUntypedContextAttrs<'input> for BlockDeclUntypedContext<'input> {}

pub struct BlockDeclUntypedContextExt<'input> {
    base: BlockDeclContextExt<'input>,
    pub name: Option<Rc<ArgidentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockDeclUntypedContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockDeclUntypedContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockDeclUntypedContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockDeclUntyped(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockDeclUntyped(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for BlockDeclUntypedContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockDeclUntyped(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockDeclUntypedContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockDecl
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockDecl }
}

impl<'input> Borrow<BlockDeclContextExt<'input>> for BlockDeclUntypedContext<'input> {
    fn borrow(&self) -> &BlockDeclContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockDeclContextExt<'input>> for BlockDeclUntypedContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockDeclContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockDeclContextAttrs<'input> for BlockDeclUntypedContext<'input> {}

impl<'input> BlockDeclUntypedContextExt<'input> {
    fn new(ctx: &dyn BlockDeclContextAttrs<'input>) -> Rc<BlockDeclContextAll<'input>> {
        Rc::new(BlockDeclContextAll::BlockDeclUntypedContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockDeclUntypedContextExt {
                    name: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BlockDeclTypedContext<'input> =
    BaseParserRuleContext<'input, BlockDeclTypedContextExt<'input>>;

pub trait BlockDeclTypedContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn argident(&self) -> Option<Rc<ArgidentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> BlockDeclTypedContextAttrs<'input> for BlockDeclTypedContext<'input> {}

pub struct BlockDeclTypedContextExt<'input> {
    base: BlockDeclContextExt<'input>,
    pub name: Option<Rc<ArgidentContextAll<'input>>>,
    pub argtype: Option<Rc<IdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {BlockDeclTypedContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for BlockDeclTypedContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for BlockDeclTypedContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_BlockDeclTyped(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_BlockDeclTyped(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a>
    for BlockDeclTypedContext<'input>
{
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_BlockDeclTyped(self);
    }
}

impl<'input> CustomRuleContext<'input> for BlockDeclTypedContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_blockDecl
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_blockDecl }
}

impl<'input> Borrow<BlockDeclContextExt<'input>> for BlockDeclTypedContext<'input> {
    fn borrow(&self) -> &BlockDeclContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<BlockDeclContextExt<'input>> for BlockDeclTypedContext<'input> {
    fn borrow_mut(&mut self) -> &mut BlockDeclContextExt<'input> {
        &mut self.base
    }
}

impl<'input> BlockDeclContextAttrs<'input> for BlockDeclTypedContext<'input> {}

impl<'input> BlockDeclTypedContextExt<'input> {
    fn new(ctx: &dyn BlockDeclContextAttrs<'input>) -> Rc<BlockDeclContextAll<'input>> {
        Rc::new(BlockDeclContextAll::BlockDeclTypedContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BlockDeclTypedContextExt {
                    name: None,
                    argtype: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn blockDecl(&mut self) -> Result<Rc<BlockDeclContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = BlockDeclContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 34, RULE_blockDecl);
        let mut _localctx: Rc<BlockDeclContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(385);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(36, &mut recog.base)? {
                1 => {
                    let tmp = BlockDeclTypedContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        /*InvokeRule argident*/
                        recog.base.set_state(380);
                        let tmp = recog.argident()?;
                        if let BlockDeclContextAll::BlockDeclTypedContext(ctx) =
                            cast_mut::<_, BlockDeclContextAll>(&mut _localctx)
                        {
                            ctx.name = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(381);
                        recog.base.match_token(T__5, &mut recog.err_handler)?;

                        /*InvokeRule ident*/
                        recog.base.set_state(382);
                        let tmp = recog.ident()?;
                        if let BlockDeclContextAll::BlockDeclTypedContext(ctx) =
                            cast_mut::<_, BlockDeclContextAll>(&mut _localctx)
                        {
                            ctx.argtype = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                2 => {
                    let tmp = BlockDeclUntypedContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        /*InvokeRule argident*/
                        recog.base.set_state(384);
                        let tmp = recog.argident()?;
                        if let BlockDeclContextAll::BlockDeclUntypedContext(ctx) =
                            cast_mut::<_, BlockDeclContextAll>(&mut _localctx)
                        {
                            ctx.name = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- string ----------------
pub type StringContextAll<'input> = StringContext<'input>;

pub type StringContext<'input> = BaseParserRuleContext<'input, StringContextExt<'input>>;

#[derive(Clone)]
pub struct StringContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for StringContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for StringContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_string(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_string(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for StringContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_string(self);
    }
}

impl<'input> CustomRuleContext<'input> for StringContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_string
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_string }
}
antlr_rust::tid! {StringContextExt<'a>}

impl<'input> StringContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<StringContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            StringContextExt { ph: PhantomData },
        ))
    }
}

pub trait StringContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<StringContextExt<'input>>
{
    /// Retrieves first TerminalNode corresponding to token STRING
    /// Returns `None` if there is no child corresponding to token STRING
    fn STRING(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(STRING, 0)
    }
}

impl<'input> StringContextAttrs<'input> for StringContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn string(&mut self) -> Result<Rc<StringContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = StringContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 36, RULE_string);
        let mut _localctx: Rc<StringContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(387);
                recog.base.match_token(STRING, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- argident ----------------
#[derive(Debug)]
pub enum ArgidentContextAll<'input> {
    ArgIdentContext(ArgIdentContext<'input>),
    ArgIdentInstContext(ArgIdentInstContext<'input>),
    Error(ArgidentContext<'input>),
}
antlr_rust::tid! {ArgidentContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for ArgidentContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for ArgidentContextAll<'input> {}

impl<'input> Deref for ArgidentContextAll<'input> {
    type Target = dyn ArgidentContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use ArgidentContextAll::*;
        match self {
            ArgIdentContext(inner) => inner,
            ArgIdentInstContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ArgidentContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ArgidentContextAll<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type ArgidentContext<'input> = BaseParserRuleContext<'input, ArgidentContextExt<'input>>;

#[derive(Clone)]
pub struct ArgidentContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for ArgidentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ArgidentContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ArgidentContext<'input> {}

impl<'input> CustomRuleContext<'input> for ArgidentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_argident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_argident }
}
antlr_rust::tid! {ArgidentContextExt<'a>}

impl<'input> ArgidentContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<ArgidentContextAll<'input>> {
        Rc::new(ArgidentContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                ArgidentContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait ArgidentContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<ArgidentContextExt<'input>>
{
}

impl<'input> ArgidentContextAttrs<'input> for ArgidentContext<'input> {}

pub type ArgIdentContext<'input> = BaseParserRuleContext<'input, ArgIdentContextExt<'input>>;

pub trait ArgIdentContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ArgIdentContextAttrs<'input> for ArgIdentContext<'input> {}

pub struct ArgIdentContextExt<'input> {
    base: ArgidentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ArgIdentContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ArgIdentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for ArgIdentContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ArgIdent(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ArgIdent(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ArgIdentContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ArgIdent(self);
    }
}

impl<'input> CustomRuleContext<'input> for ArgIdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_argident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_argident }
}

impl<'input> Borrow<ArgidentContextExt<'input>> for ArgIdentContext<'input> {
    fn borrow(&self) -> &ArgidentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ArgidentContextExt<'input>> for ArgIdentContext<'input> {
    fn borrow_mut(&mut self) -> &mut ArgidentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ArgidentContextAttrs<'input> for ArgIdentContext<'input> {}

impl<'input> ArgIdentContextExt<'input> {
    fn new(ctx: &dyn ArgidentContextAttrs<'input>) -> Rc<ArgidentContextAll<'input>> {
        Rc::new(ArgidentContextAll::ArgIdentContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ArgIdentContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ArgIdentInstContext<'input> =
    BaseParserRuleContext<'input, ArgIdentInstContextExt<'input>>;

pub trait ArgIdentInstContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn ident(&self) -> Option<Rc<IdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ArgIdentInstContextAttrs<'input> for ArgIdentInstContext<'input> {}

pub struct ArgIdentInstContextExt<'input> {
    base: ArgidentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {ArgIdentInstContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for ArgIdentInstContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for ArgIdentInstContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_ArgIdentInst(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_ArgIdentInst(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for ArgIdentInstContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_ArgIdentInst(self);
    }
}

impl<'input> CustomRuleContext<'input> for ArgIdentInstContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_argident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_argident }
}

impl<'input> Borrow<ArgidentContextExt<'input>> for ArgIdentInstContext<'input> {
    fn borrow(&self) -> &ArgidentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<ArgidentContextExt<'input>> for ArgIdentInstContext<'input> {
    fn borrow_mut(&mut self) -> &mut ArgidentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> ArgidentContextAttrs<'input> for ArgIdentInstContext<'input> {}

impl<'input> ArgIdentInstContextExt<'input> {
    fn new(ctx: &dyn ArgidentContextAttrs<'input>) -> Rc<ArgidentContextAll<'input>> {
        Rc::new(ArgidentContextAll::ArgIdentInstContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ArgIdentInstContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn argident(&mut self) -> Result<Rc<ArgidentContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = ArgidentContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 38, RULE_argident);
        let mut _localctx: Rc<ArgidentContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(392);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.base.input.la(1) {
                T__31 => {
                    let tmp = ArgIdentInstContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        recog.base.set_state(389);
                        recog.base.match_token(T__31, &mut recog.err_handler)?;

                        /*InvokeRule ident*/
                        recog.base.set_state(390);
                        recog.ident()?;
                    }
                }

                T__34 | T__35 | T__36 | IDENT => {
                    let tmp = ArgIdentContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        /*InvokeRule ident*/
                        recog.base.set_state(391);
                        recog.ident()?;
                    }
                }

                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                    &mut recog.base,
                )))?,
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- ident ----------------
#[derive(Debug)]
pub enum IdentContextAll<'input> {
    IdentOtherContext(IdentOtherContext<'input>),
    IdentKeywordContext(IdentKeywordContext<'input>),
    Error(IdentContext<'input>),
}
antlr_rust::tid! {IdentContextAll<'a>}

impl<'input> antlr_rust::parser_rule_context::DerefSeal for IdentContextAll<'input> {}

impl<'input> BuildingBlocksParserContext<'input> for IdentContextAll<'input> {}

impl<'input> Deref for IdentContextAll<'input> {
    type Target = dyn IdentContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use IdentContextAll::*;
        match self {
            IdentOtherContext(inner) => inner,
            IdentKeywordContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for IdentContextAll<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        self.deref().exit(listener)
    }
}

pub type IdentContext<'input> = BaseParserRuleContext<'input, IdentContextExt<'input>>;

#[derive(Clone)]
pub struct IdentContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for IdentContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for IdentContext<'input> {}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentContext<'input> {}

impl<'input> CustomRuleContext<'input> for IdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_ident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_ident }
}
antlr_rust::tid! {IdentContextExt<'a>}

impl<'input> IdentContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<IdentContextAll<'input>> {
        Rc::new(IdentContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                IdentContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait IdentContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<IdentContextExt<'input>>
{
}

impl<'input> IdentContextAttrs<'input> for IdentContext<'input> {}

pub type IdentOtherContext<'input> = BaseParserRuleContext<'input, IdentOtherContextExt<'input>>;

pub trait IdentOtherContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token IDENT
    /// Returns `None` if there is no child corresponding to token IDENT
    fn IDENT(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(IDENT, 0)
    }
}

impl<'input> IdentOtherContextAttrs<'input> for IdentOtherContext<'input> {}

pub struct IdentOtherContextExt<'input> {
    base: IdentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IdentOtherContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IdentOtherContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for IdentOtherContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IdentOther(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IdentOther(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentOtherContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IdentOther(self);
    }
}

impl<'input> CustomRuleContext<'input> for IdentOtherContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_ident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_ident }
}

impl<'input> Borrow<IdentContextExt<'input>> for IdentOtherContext<'input> {
    fn borrow(&self) -> &IdentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<IdentContextExt<'input>> for IdentOtherContext<'input> {
    fn borrow_mut(&mut self) -> &mut IdentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> IdentContextAttrs<'input> for IdentOtherContext<'input> {}

impl<'input> IdentOtherContextExt<'input> {
    fn new(ctx: &dyn IdentContextAttrs<'input>) -> Rc<IdentContextAll<'input>> {
        Rc::new(IdentContextAll::IdentOtherContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IdentOtherContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IdentKeywordContext<'input> =
    BaseParserRuleContext<'input, IdentKeywordContextExt<'input>>;

pub trait IdentKeywordContextAttrs<'input>: BuildingBlocksParserContext<'input> {
    fn keyword(&self) -> Option<Rc<KeywordContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> IdentKeywordContextAttrs<'input> for IdentKeywordContext<'input> {}

pub struct IdentKeywordContextExt<'input> {
    base: IdentContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr_rust::tid! {IdentKeywordContextExt<'a>}

impl<'input> BuildingBlocksParserContext<'input> for IdentKeywordContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a>
    for IdentKeywordContext<'input>
{
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_IdentKeyword(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_IdentKeyword(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for IdentKeywordContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_IdentKeyword(self);
    }
}

impl<'input> CustomRuleContext<'input> for IdentKeywordContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_ident
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_ident }
}

impl<'input> Borrow<IdentContextExt<'input>> for IdentKeywordContext<'input> {
    fn borrow(&self) -> &IdentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<IdentContextExt<'input>> for IdentKeywordContext<'input> {
    fn borrow_mut(&mut self) -> &mut IdentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> IdentContextAttrs<'input> for IdentKeywordContext<'input> {}

impl<'input> IdentKeywordContextExt<'input> {
    fn new(ctx: &dyn IdentContextAttrs<'input>) -> Rc<IdentContextAll<'input>> {
        Rc::new(IdentContextAll::IdentKeywordContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IdentKeywordContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn ident(&mut self) -> Result<Rc<IdentContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = IdentContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 40, RULE_ident);
        let mut _localctx: Rc<IdentContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(396);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.base.input.la(1) {
                T__34 | T__35 | T__36 => {
                    let tmp = IdentKeywordContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1);
                    _localctx = tmp;
                    {
                        /*InvokeRule keyword*/
                        recog.base.set_state(394);
                        recog.keyword()?;
                    }
                }

                IDENT => {
                    let tmp = IdentOtherContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2);
                    _localctx = tmp;
                    {
                        recog.base.set_state(395);
                        recog.base.match_token(IDENT, &mut recog.err_handler)?;
                    }
                }

                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                    &mut recog.base,
                )))?,
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- symbol ----------------
pub type SymbolContextAll<'input> = SymbolContext<'input>;

pub type SymbolContext<'input> = BaseParserRuleContext<'input, SymbolContextExt<'input>>;

#[derive(Clone)]
pub struct SymbolContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for SymbolContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for SymbolContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_symbol(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_symbol(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for SymbolContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_symbol(self);
    }
}

impl<'input> CustomRuleContext<'input> for SymbolContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_symbol
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_symbol }
}
antlr_rust::tid! {SymbolContextExt<'a>}

impl<'input> SymbolContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<SymbolContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            SymbolContextExt { ph: PhantomData },
        ))
    }
}

pub trait SymbolContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<SymbolContextExt<'input>>
{
    /// Retrieves first TerminalNode corresponding to token SYMBOL
    /// Returns `None` if there is no child corresponding to token SYMBOL
    fn SYMBOL(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(SYMBOL, 0)
    }
}

impl<'input> SymbolContextAttrs<'input> for SymbolContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn symbol(&mut self) -> Result<Rc<SymbolContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = SymbolContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 42, RULE_symbol);
        let mut _localctx: Rc<SymbolContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(398);
                recog.base.match_token(SYMBOL, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}
//------------------- number ----------------
pub type NumberContextAll<'input> = NumberContext<'input>;

pub type NumberContext<'input> = BaseParserRuleContext<'input, NumberContextExt<'input>>;

#[derive(Clone)]
pub struct NumberContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> BuildingBlocksParserContext<'input> for NumberContext<'input> {}

impl<'input, 'a> Listenable<dyn BuildingBlocksListener<'input> + 'a> for NumberContext<'input> {
    fn enter(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.enter_every_rule(self);
        listener.enter_number(self);
    }
    fn exit(&self, listener: &mut (dyn BuildingBlocksListener<'input> + 'a)) {
        listener.exit_number(self);
        listener.exit_every_rule(self);
    }
}

impl<'input, 'a> Visitable<dyn BuildingBlocksVisitor<'input> + 'a> for NumberContext<'input> {
    fn accept(&self, visitor: &mut (dyn BuildingBlocksVisitor<'input> + 'a)) {
        visitor.visit_number(self);
    }
}

impl<'input> CustomRuleContext<'input> for NumberContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = BuildingBlocksParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_number
    }
    //fn type_rule_index() -> usize where Self: Sized { RULE_number }
}
antlr_rust::tid! {NumberContextExt<'a>}

impl<'input> NumberContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn BuildingBlocksParserContext<'input> + 'input>>,
        invoking_state: isize,
    ) -> Rc<NumberContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            NumberContextExt { ph: PhantomData },
        ))
    }
}

pub trait NumberContextAttrs<'input>:
    BuildingBlocksParserContext<'input> + BorrowMut<NumberContextExt<'input>>
{
    /// Retrieves first TerminalNode corresponding to token NUMBER
    /// Returns `None` if there is no child corresponding to token NUMBER
    fn NUMBER(&self) -> Option<Rc<TerminalNode<'input, BuildingBlocksParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(NUMBER, 0)
    }
}

impl<'input> NumberContextAttrs<'input> for NumberContext<'input> {}

impl<'input, I, H> BuildingBlocksParser<'input, I, H>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
    H: ErrorStrategy<'input, BaseParserType<'input, I>>,
{
    pub fn number(&mut self) -> Result<Rc<NumberContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = NumberContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 44, RULE_number);
        let mut _localctx: Rc<NumberContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            //recog.base.enter_outer_alt(_localctx.clone(), 1);
            recog.base.enter_outer_alt(None, 1);
            {
                recog.base.set_state(400);
                recog.base.match_token(NUMBER, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule();

        Ok(_localctx)
    }
}

lazy_static! {
    static ref _ATN: Arc<ATN> =
        Arc::new(ATNDeserializer::new(None).deserialize(_serializedATN.chars()));
    static ref _decision_to_DFA: Arc<Vec<antlr_rust::RwLock<DFA>>> = {
        let mut dfa = Vec::new();
        let size = _ATN.decision_to_state.len();
        for i in 0..size {
            dfa.push(DFA::new(_ATN.clone(), _ATN.get_decision_state(i), i as isize).into())
        }
        Arc::new(dfa)
    };
}

const _serializedATN: &'static str = "\x03\u{608b}\u{a72a}\u{8133}\u{b9ed}\u{417c}\u{3be7}\u{7786}\u{5964}\x03\
	\x36\u{195}\x04\x02\x09\x02\x04\x03\x09\x03\x04\x04\x09\x04\x04\x05\x09\
	\x05\x04\x06\x09\x06\x04\x07\x09\x07\x04\x08\x09\x08\x04\x09\x09\x09\x04\
	\x0a\x09\x0a\x04\x0b\x09\x0b\x04\x0c\x09\x0c\x04\x0d\x09\x0d\x04\x0e\x09\
	\x0e\x04\x0f\x09\x0f\x04\x10\x09\x10\x04\x11\x09\x11\x04\x12\x09\x12\x04\
	\x13\x09\x13\x04\x14\x09\x14\x04\x15\x09\x15\x04\x16\x09\x16\x04\x17\x09\
	\x17\x04\x18\x09\x18\x03\x02\x03\x02\x05\x02\x33\x0a\x02\x06\x02\x35\x0a\
	\x02\x0d\x02\x0e\x02\x36\x03\x03\x03\x03\x03\x03\x03\x03\x03\x03\x03\x03\
	\x03\x03\x03\x03\x03\x03\x03\x03\x03\x03\x05\x03\x44\x0a\x03\x03\x04\x03\
	\x04\x03\x05\x03\x05\x03\x06\x03\x06\x03\x07\x03\x07\x05\x07\x4e\x0a\x07\
	\x03\x07\x03\x07\x06\x07\x52\x0a\x07\x0d\x07\x0e\x07\x53\x03\x07\x03\x07\
	\x03\x07\x03\x07\x03\x07\x05\x07\x5b\x0a\x07\x03\x08\x06\x08\x5e\x0a\x08\
	\x0d\x08\x0e\x08\x5f\x03\x08\x03\x08\x03\x08\x03\x09\x03\x09\x03\x09\x03\
	\x09\x03\x09\x03\x09\x03\x09\x03\x09\x06\x09\x6d\x0a\x09\x0d\x09\x0e\x09\
	\x6e\x03\x09\x03\x09\x05\x09\x73\x0a\x09\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x07\x0a\
	\u{9c}\x0a\x0a\x0c\x0a\x0e\x0a\u{9f}\x0b\x0a\x03\x0a\x03\x0a\x03\x0a\x03\
	\x0a\x07\x0a\u{a5}\x0a\x0a\x0c\x0a\x0e\x0a\u{a8}\x0b\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x07\x0a\u{ae}\x0a\x0a\x0c\x0a\x0e\x0a\u{b1}\x0b\x0a\x03\
	\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x07\x0a\u{ba}\x0a\x0a\
	\x0c\x0a\x0e\x0a\u{bd}\x0b\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\
	\x0a\x03\x0a\x03\x0a\x05\x0a\u{c7}\x0a\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x03\x0a\
	\x03\x0a\x03\x0a\x03\x0a\x03\x0a\x07\x0a\u{fe}\x0a\x0a\x0c\x0a\x0e\x0a\u{101}\
	\x0b\x0a\x03\x0b\x03\x0b\x03\x0c\x03\x0c\x03\x0c\x03\x0c\x06\x0c\u{109}\
	\x0a\x0c\x0d\x0c\x0e\x0c\u{10a}\x03\x0c\x03\x0c\x03\x0c\x03\x0c\x05\x0c\
	\u{111}\x0a\x0c\x03\x0d\x03\x0d\x03\x0d\x03\x0d\x03\x0d\x03\x0d\x05\x0d\
	\u{119}\x0a\x0d\x03\x0e\x03\x0e\x03\x0e\x03\x0e\x07\x0e\u{11f}\x0a\x0e\x0c\
	\x0e\x0e\x0e\u{122}\x0b\x0e\x03\x0e\x05\x0e\u{125}\x0a\x0e\x03\x0e\x03\x0e\
	\x03\x0e\x03\x0e\x03\x0e\x05\x0e\u{12c}\x0a\x0e\x03\x0f\x03\x0f\x03\x10\
	\x03\x10\x03\x10\x03\x10\x03\x10\x05\x10\u{135}\x0a\x10\x07\x10\u{137}\x0a\
	\x10\x0c\x10\x0e\x10\u{13a}\x0b\x10\x03\x10\x03\x10\x03\x10\x03\x10\x03\
	\x10\x03\x10\x05\x10\u{142}\x0a\x10\x07\x10\u{144}\x0a\x10\x0c\x10\x0e\x10\
	\u{147}\x0b\x10\x03\x10\x03\x10\x03\x10\x03\x10\x03\x10\x05\x10\u{14e}\x0a\
	\x10\x07\x10\u{150}\x0a\x10\x0c\x10\x0e\x10\u{153}\x0b\x10\x03\x10\x05\x10\
	\u{156}\x0a\x10\x03\x11\x03\x11\x07\x11\u{15a}\x0a\x11\x0c\x11\x0e\x11\u{15d}\
	\x0b\x11\x03\x11\x05\x11\u{160}\x0a\x11\x03\x11\x03\x11\x07\x11\u{164}\x0a\
	\x11\x0c\x11\x0e\x11\u{167}\x0b\x11\x03\x11\x03\x11\x03\x11\x07\x11\u{16c}\
	\x0a\x11\x0c\x11\x0e\x11\u{16f}\x0b\x11\x03\x11\x05\x11\u{172}\x0a\x11\x03\
	\x11\x05\x11\u{175}\x0a\x11\x03\x12\x03\x12\x03\x12\x03\x12\x03\x12\x03\
	\x12\x05\x12\u{17d}\x0a\x12\x03\x13\x03\x13\x03\x13\x03\x13\x03\x13\x05\
	\x13\u{184}\x0a\x13\x03\x14\x03\x14\x03\x15\x03\x15\x03\x15\x05\x15\u{18b}\
	\x0a\x15\x03\x16\x03\x16\x05\x16\u{18f}\x0a\x16\x03\x17\x03\x17\x03\x18\
	\x03\x18\x03\x18\x02\x03\x12\x19\x02\x04\x06\x08\x0a\x0c\x0e\x10\x12\x14\
	\x16\x18\x1a\x1c\x1e\x20\x22\x24\x26\x28\x2a\x2c\x2e\x02\x03\x03\x02\x25\
	\x27\x02\u{1d6}\x02\x34\x03\x02\x02\x02\x04\x43\x03\x02\x02\x02\x06\x45\
	\x03\x02\x02\x02\x08\x47\x03\x02\x02\x02\x0a\x49\x03\x02\x02\x02\x0c\x5a\
	\x03\x02\x02\x02\x0e\x5d\x03\x02\x02\x02\x10\x72\x03\x02\x02\x02\x12\u{c6}\
	\x03\x02\x02\x02\x14\u{102}\x03\x02\x02\x02\x16\u{110}\x03\x02\x02\x02\x18\
	\u{118}\x03\x02\x02\x02\x1a\u{12b}\x03\x02\x02\x02\x1c\u{12d}\x03\x02\x02\
	\x02\x1e\u{155}\x03\x02\x02\x02\x20\u{174}\x03\x02\x02\x02\x22\u{17c}\x03\
	\x02\x02\x02\x24\u{183}\x03\x02\x02\x02\x26\u{185}\x03\x02\x02\x02\x28\u{18a}\
	\x03\x02\x02\x02\x2a\u{18e}\x03\x02\x02\x02\x2c\u{190}\x03\x02\x02\x02\x2e\
	\u{192}\x03\x02\x02\x02\x30\x32\x05\x04\x03\x02\x31\x33\x07\x03\x02\x02\
	\x32\x31\x03\x02\x02\x02\x32\x33\x03\x02\x02\x02\x33\x35\x03\x02\x02\x02\
	\x34\x30\x03\x02\x02\x02\x35\x36\x03\x02\x02\x02\x36\x34\x03\x02\x02\x02\
	\x36\x37\x03\x02\x02\x02\x37\x03\x03\x02\x02\x02\x38\x39\x07\x31\x02\x02\
	\x39\x44\x05\x12\x0a\x02\x3a\x3b\x07\x32\x02\x02\x3b\x44\x05\x12\x0a\x02\
	\x3c\x3d\x07\x33\x02\x02\x3d\x44\x05\x12\x0a\x02\x3e\x44\x05\x0e\x08\x02\
	\x3f\x44\x05\x06\x04\x02\x40\x44\x05\x08\x05\x02\x41\x44\x05\x0a\x06\x02\
	\x42\x44\x05\x12\x0a\x02\x43\x38\x03\x02\x02\x02\x43\x3a\x03\x02\x02\x02\
	\x43\x3c\x03\x02\x02\x02\x43\x3e\x03\x02\x02\x02\x43\x3f\x03\x02\x02\x02\
	\x43\x40\x03\x02\x02\x02\x43\x41\x03\x02\x02\x02\x43\x42\x03\x02\x02\x02\
	\x44\x05\x03\x02\x02\x02\x45\x46\x07\x04\x02\x02\x46\x07\x03\x02\x02\x02\
	\x47\x48\x07\x05\x02\x02\x48\x09\x03\x02\x02\x02\x49\x4a\x07\x06\x02\x02\
	\x4a\x0b\x03\x02\x02\x02\x4b\x4d\x05\x2a\x16\x02\x4c\x4e\x07\x07\x02\x02\
	\x4d\x4c\x03\x02\x02\x02\x4d\x4e\x03\x02\x02\x02\x4e\x4f\x03\x02\x02\x02\
	\x4f\x50\x07\x08\x02\x02\x50\x52\x03\x02\x02\x02\x51\x4b\x03\x02\x02\x02\
	\x52\x53\x03\x02\x02\x02\x53\x51\x03\x02\x02\x02\x53\x54\x03\x02\x02\x02\
	\x54\x5b\x03\x02\x02\x02\x55\x5b\x05\x2a\x16\x02\x56\x57\x05\x2a\x16\x02\
	\x57\x58\x07\x09\x02\x02\x58\x5b\x03\x02\x02\x02\x59\x5b\x05\x2c\x17\x02\
	\x5a\x51\x03\x02\x02\x02\x5a\x55\x03\x02\x02\x02\x5a\x56\x03\x02\x02\x02\
	\x5a\x59\x03\x02\x02\x02\x5b\x0d\x03\x02\x02\x02\x5c\x5e\x05\x10\x09\x02\
	\x5d\x5c\x03\x02\x02\x02\x5e\x5f\x03\x02\x02\x02\x5f\x5d\x03\x02\x02\x02\
	\x5f\x60\x03\x02\x02\x02\x60\x61\x03\x02\x02\x02\x61\x62\x07\x0a\x02\x02\
	\x62\x63\x05\x12\x0a\x02\x63\x0f\x03\x02\x02\x02\x64\x73\x05\x18\x0d\x02\
	\x65\x66\x07\x0b\x02\x02\x66\x73\x05\x18\x0d\x02\x67\x73\x07\x0c\x02\x02\
	\x68\x69\x07\x0b\x02\x02\x69\x73\x07\x0c\x02\x02\x6a\x6c\x07\x0d\x02\x02\
	\x6b\x6d\x05\x10\x09\x02\x6c\x6b\x03\x02\x02\x02\x6d\x6e\x03\x02\x02\x02\
	\x6e\x6c\x03\x02\x02\x02\x6e\x6f\x03\x02\x02\x02\x6f\x70\x03\x02\x02\x02\
	\x70\x71\x07\x0e\x02\x02\x71\x73\x03\x02\x02\x02\x72\x64\x03\x02\x02\x02\
	\x72\x65\x03\x02\x02\x02\x72\x67\x03\x02\x02\x02\x72\x68\x03\x02\x02\x02\
	\x72\x6a\x03\x02\x02\x02\x73\x11\x03\x02\x02\x02\x74\x75\x08\x0a\x01\x02\
	\x75\x76\x07\x0d\x02\x02\x76\x77\x05\x12\x0a\x02\x77\x78\x07\x0e\x02\x02\
	\x78\u{c7}\x03\x02\x02\x02\x79\x7a\x07\x0f\x02\x02\x7a\u{c7}\x05\x12\x0a\
	\x28\x7b\x7c\x07\x07\x02\x02\x7c\u{c7}\x05\x12\x0a\x27\x7d\x7e\x07\x09\x02\
	\x02\x7e\u{c7}\x05\x12\x0a\x26\x7f\u{80}\x07\x10\x02\x02\u{80}\u{c7}\x05\
	\x12\x0a\x25\u{81}\u{82}\x05\x18\x0d\x02\u{82}\u{83}\x07\x11\x02\x02\u{83}\
	\u{84}\x05\x18\x0d\x02\u{84}\u{85}\x07\x11\x02\x02\u{85}\u{86}\x05\x1e\x10\
	\x02\u{86}\u{c7}\x03\x02\x02\x02\u{87}\u{88}\x05\x18\x0d\x02\u{88}\u{89}\
	\x07\x11\x02\x02\u{89}\u{8a}\x05\x1e\x10\x02\u{8a}\u{c7}\x03\x02\x02\x02\
	\u{8b}\u{8c}\x05\x18\x0d\x02\u{8c}\u{8d}\x07\x11\x02\x02\u{8d}\u{8e}\x05\
	\x12\x0a\x22\u{8e}\u{c7}\x03\x02\x02\x02\u{8f}\u{90}\x05\x0c\x07\x02\u{90}\
	\u{91}\x07\x13\x02\x02\u{91}\u{92}\x05\x1e\x10\x02\u{92}\u{c7}\x03\x02\x02\
	\x02\u{93}\u{94}\x05\x0c\x07\x02\u{94}\u{95}\x07\x14\x02\x02\u{95}\u{96}\
	\x05\x1e\x10\x02\u{96}\u{c7}\x03\x02\x02\x02\u{97}\u{98}\x07\x16\x02\x02\
	\u{98}\u{c7}\x05\x16\x0c\x02\u{99}\u{9d}\x07\x2b\x02\x02\u{9a}\u{9c}\x05\
	\x12\x0a\x02\u{9b}\u{9a}\x03\x02\x02\x02\u{9c}\u{9f}\x03\x02\x02\x02\u{9d}\
	\u{9b}\x03\x02\x02\x02\u{9d}\u{9e}\x03\x02\x02\x02\u{9e}\u{a0}\x03\x02\x02\
	\x02\u{9f}\u{9d}\x03\x02\x02\x02\u{a0}\u{c7}\x07\x0e\x02\x02\u{a1}\u{a2}\
	\x07\x1f\x02\x02\u{a2}\u{a6}\x07\x0d\x02\x02\u{a3}\u{a5}\x05\x12\x0a\x02\
	\u{a4}\u{a3}\x03\x02\x02\x02\u{a5}\u{a8}\x03\x02\x02\x02\u{a6}\u{a4}\x03\
	\x02\x02\x02\u{a6}\u{a7}\x03\x02\x02\x02\u{a7}\u{a9}\x03\x02\x02\x02\u{a8}\
	\u{a6}\x03\x02\x02\x02\u{a9}\u{c7}\x07\x0e\x02\x02\u{aa}\u{ab}\x07\x1f\x02\
	\x02\u{ab}\u{af}\x07\x1a\x02\x02\u{ac}\u{ae}\x05\x12\x0a\x02\u{ad}\u{ac}\
	\x03\x02\x02\x02\u{ae}\u{b1}\x03\x02\x02\x02\u{af}\u{ad}\x03\x02\x02\x02\
	\u{af}\u{b0}\x03\x02\x02\x02\u{b0}\u{b2}\x03\x02\x02\x02\u{b1}\u{af}\x03\
	\x02\x02\x02\u{b2}\u{c7}\x07\x19\x02\x02\u{b3}\u{b4}\x07\x1f\x02\x02\u{b4}\
	\u{bb}\x07\x20\x02\x02\u{b5}\u{b6}\x05\x12\x0a\x02\u{b6}\u{b7}\x07\x08\x02\
	\x02\u{b7}\u{b8}\x05\x12\x0a\x02\u{b8}\u{ba}\x03\x02\x02\x02\u{b9}\u{b5}\
	\x03\x02\x02\x02\u{ba}\u{bd}\x03\x02\x02\x02\u{bb}\u{b9}\x03\x02\x02\x02\
	\u{bb}\u{bc}\x03\x02\x02\x02\u{bc}\u{be}\x03\x02\x02\x02\u{bd}\u{bb}\x03\
	\x02\x02\x02\u{be}\u{c7}\x07\x21\x02\x02\u{bf}\u{c7}\x05\x2e\x18\x02\u{c0}\
	\u{c7}\x05\x26\x14\x02\u{c1}\u{c7}\x05\x2c\x17\x02\u{c2}\u{c7}\x05\x1e\x10\
	\x02\u{c3}\u{c7}\x05\x18\x0d\x02\u{c4}\u{c7}\x07\x2e\x02\x02\u{c5}\u{c7}\
	\x05\x14\x0b\x02\u{c6}\x74\x03\x02\x02\x02\u{c6}\x79\x03\x02\x02\x02\u{c6}\
	\x7b\x03\x02\x02\x02\u{c6}\x7d\x03\x02\x02\x02\u{c6}\x7f\x03\x02\x02\x02\
	\u{c6}\u{81}\x03\x02\x02\x02\u{c6}\u{87}\x03\x02\x02\x02\u{c6}\u{8b}\x03\
	\x02\x02\x02\u{c6}\u{8f}\x03\x02\x02\x02\u{c6}\u{93}\x03\x02\x02\x02\u{c6}\
	\u{97}\x03\x02\x02\x02\u{c6}\u{99}\x03\x02\x02\x02\u{c6}\u{a1}\x03\x02\x02\
	\x02\u{c6}\u{aa}\x03\x02\x02\x02\u{c6}\u{b3}\x03\x02\x02\x02\u{c6}\u{bf}\
	\x03\x02\x02\x02\u{c6}\u{c0}\x03\x02\x02\x02\u{c6}\u{c1}\x03\x02\x02\x02\
	\u{c6}\u{c2}\x03\x02\x02\x02\u{c6}\u{c3}\x03\x02\x02\x02\u{c6}\u{c4}\x03\
	\x02\x02\x02\u{c6}\u{c5}\x03\x02\x02\x02\u{c7}\u{ff}\x03\x02\x02\x02\u{c8}\
	\u{c9}\x0c\x1e\x02\x02\u{c9}\u{ca}\x07\x15\x02\x02\u{ca}\u{fe}\x05\x12\x0a\
	\x1f\u{cb}\u{cc}\x0c\x1b\x02\x02\u{cc}\u{cd}\x07\x07\x02\x02\u{cd}\u{fe}\
	\x05\x12\x0a\x1c\u{ce}\u{cf}\x0c\x1a\x02\x02\u{cf}\u{d0}\x07\x0f\x02\x02\
	\u{d0}\u{fe}\x05\x12\x0a\x1b\u{d1}\u{d2}\x0c\x19\x02\x02\u{d2}\u{d3}\x07\
	\x17\x02\x02\u{d3}\u{fe}\x05\x12\x0a\x1a\u{d4}\u{d5}\x0c\x18\x02\x02\u{d5}\
	\u{d6}\x07\x0b\x02\x02\u{d6}\u{fe}\x05\x12\x0a\x19\u{d7}\u{d8}\x0c\x17\x02\
	\x02\u{d8}\u{d9}\x07\x10\x02\x02\u{d9}\u{fe}\x05\x12\x0a\x18\u{da}\u{db}\
	\x0c\x16\x02\x02\u{db}\u{dc}\x07\x18\x02\x02\u{dc}\u{fe}\x05\x12\x0a\x17\
	\u{dd}\u{de}\x0c\x15\x02\x02\u{de}\u{df}\x07\x19\x02\x02\u{df}\u{e0}\x07\
	\x0a\x02\x02\u{e0}\u{fe}\x05\x12\x0a\x16\u{e1}\u{e2}\x0c\x14\x02\x02\u{e2}\
	\u{e3}\x07\x19\x02\x02\u{e3}\u{fe}\x05\x12\x0a\x15\u{e4}\u{e5}\x0c\x13\x02\
	\x02\u{e5}\u{e6}\x07\x1a\x02\x02\u{e6}\u{e7}\x07\x0a\x02\x02\u{e7}\u{fe}\
	\x05\x12\x0a\x14\u{e8}\u{e9}\x0c\x12\x02\x02\u{e9}\u{ea}\x07\x1a\x02\x02\
	\u{ea}\u{fe}\x05\x12\x0a\x13\u{eb}\u{ec}\x0c\x11\x02\x02\u{ec}\u{ed}\x07\
	\x1b\x02\x02\u{ed}\u{fe}\x05\x12\x0a\x12\u{ee}\u{ef}\x0c\x10\x02\x02\u{ef}\
	\u{f0}\x07\x1c\x02\x02\u{f0}\u{fe}\x05\x12\x0a\x11\u{f1}\u{f2}\x0c\x0f\x02\
	\x02\u{f2}\u{f3}\x07\x1d\x02\x02\u{f3}\u{fe}\x05\x12\x0a\x10\u{f4}\u{f5}\
	\x0c\x0e\x02\x02\u{f5}\u{f6}\x07\x1e\x02\x02\u{f6}\u{fe}\x05\x12\x0a\x0f\
	\u{f7}\u{f8}\x0c\x21\x02\x02\u{f8}\u{f9}\x07\x12\x02\x02\u{f9}\u{fe}\x05\
	\x1e\x10\x02\u{fa}\u{fb}\x0c\x1c\x02\x02\u{fb}\u{fc}\x07\x16\x02\x02\u{fc}\
	\u{fe}\x05\x16\x0c\x02\u{fd}\u{c8}\x03\x02\x02\x02\u{fd}\u{cb}\x03\x02\x02\
	\x02\u{fd}\u{ce}\x03\x02\x02\x02\u{fd}\u{d1}\x03\x02\x02\x02\u{fd}\u{d4}\
	\x03\x02\x02\x02\u{fd}\u{d7}\x03\x02\x02\x02\u{fd}\u{da}\x03\x02\x02\x02\
	\u{fd}\u{dd}\x03\x02\x02\x02\u{fd}\u{e1}\x03\x02\x02\x02\u{fd}\u{e4}\x03\
	\x02\x02\x02\u{fd}\u{e8}\x03\x02\x02\x02\u{fd}\u{eb}\x03\x02\x02\x02\u{fd}\
	\u{ee}\x03\x02\x02\x02\u{fd}\u{f1}\x03\x02\x02\x02\u{fd}\u{f4}\x03\x02\x02\
	\x02\u{fd}\u{f7}\x03\x02\x02\x02\u{fd}\u{fa}\x03\x02\x02\x02\u{fe}\u{101}\
	\x03\x02\x02\x02\u{ff}\u{fd}\x03\x02\x02\x02\u{ff}\u{100}\x03\x02\x02\x02\
	\u{100}\x13\x03\x02\x02\x02\u{101}\u{ff}\x03\x02\x02\x02\u{102}\u{103}\x07\
	\x2f\x02\x02\u{103}\x15\x03\x02\x02\x02\u{104}\u{105}\x05\x2a\x16\x02\u{105}\
	\u{106}\x07\x08\x02\x02\u{106}\u{107}\x05\x12\x0a\x02\u{107}\u{109}\x03\
	\x02\x02\x02\u{108}\u{104}\x03\x02\x02\x02\u{109}\u{10a}\x03\x02\x02\x02\
	\u{10a}\u{108}\x03\x02\x02\x02\u{10a}\u{10b}\x03\x02\x02\x02\u{10b}\u{111}\
	\x03\x02\x02\x02\u{10c}\u{111}\x05\x2a\x16\x02\u{10d}\u{10e}\x05\x2a\x16\
	\x02\u{10e}\u{10f}\x07\x09\x02\x02\u{10f}\u{111}\x03\x02\x02\x02\u{110}\
	\u{108}\x03\x02\x02\x02\u{110}\u{10c}\x03\x02\x02\x02\u{110}\u{10d}\x03\
	\x02\x02\x02\u{111}\x17\x03\x02\x02\x02\u{112}\u{113}\x05\x1a\x0e\x02\u{113}\
	\u{114}\x05\x2a\x16\x02\u{114}\u{119}\x03\x02\x02\x02\u{115}\u{116}\x07\
	\x22\x02\x02\u{116}\u{119}\x05\x2a\x16\x02\u{117}\u{119}\x05\x2a\x16\x02\
	\u{118}\u{112}\x03\x02\x02\x02\u{118}\u{115}\x03\x02\x02\x02\u{118}\u{117}\
	\x03\x02\x02\x02\u{119}\x19\x03\x02\x02\x02\u{11a}\u{11b}\x07\x23\x02\x02\
	\u{11b}\u{120}\x05\x2a\x16\x02\u{11c}\u{11d}\x07\x17\x02\x02\u{11d}\u{11f}\
	\x05\x2a\x16\x02\u{11e}\u{11c}\x03\x02\x02\x02\u{11f}\u{122}\x03\x02\x02\
	\x02\u{120}\u{11e}\x03\x02\x02\x02\u{120}\u{121}\x03\x02\x02\x02\u{121}\
	\u{124}\x03\x02\x02\x02\u{122}\u{120}\x03\x02\x02\x02\u{123}\u{125}\x07\
	\x17\x02\x02\u{124}\u{123}\x03\x02\x02\x02\u{124}\u{125}\x03\x02\x02\x02\
	\u{125}\u{126}\x03\x02\x02\x02\u{126}\u{127}\x07\x24\x02\x02\u{127}\u{12c}\
	\x03\x02\x02\x02\u{128}\u{129}\x07\x23\x02\x02\u{129}\u{12a}\x07\x17\x02\
	\x02\u{12a}\u{12c}\x07\x24\x02\x02\u{12b}\u{11a}\x03\x02\x02\x02\u{12b}\
	\u{128}\x03\x02\x02\x02\u{12c}\x1b\x03\x02\x02\x02\u{12d}\u{12e}\x09\x02\
	\x02\x02\u{12e}\x1d\x03\x02\x02\x02\u{12f}\u{130}\x07\x20\x02\x02\u{130}\
	\u{131}\x05\x2c\x17\x02\u{131}\u{138}\x05\x20\x11\x02\u{132}\u{134}\x05\
	\x04\x03\x02\u{133}\u{135}\x07\x03\x02\x02\u{134}\u{133}\x03\x02\x02\x02\
	\u{134}\u{135}\x03\x02\x02\x02\u{135}\u{137}\x03\x02\x02\x02\u{136}\u{132}\
	\x03\x02\x02\x02\u{137}\u{13a}\x03\x02\x02\x02\u{138}\u{136}\x03\x02\x02\
	\x02\u{138}\u{139}\x03\x02\x02\x02\u{139}\u{13b}\x03\x02\x02\x02\u{13a}\
	\u{138}\x03\x02\x02\x02\u{13b}\u{13c}\x07\x21\x02\x02\u{13c}\u{156}\x03\
	\x02\x02\x02\u{13d}\u{13e}\x07\x20\x02\x02\u{13e}\u{145}\x05\x20\x11\x02\
	\u{13f}\u{141}\x05\x04\x03\x02\u{140}\u{142}\x07\x03\x02\x02\u{141}\u{140}\
	\x03\x02\x02\x02\u{141}\u{142}\x03\x02\x02\x02\u{142}\u{144}\x03\x02\x02\
	\x02\u{143}\u{13f}\x03\x02\x02\x02\u{144}\u{147}\x03\x02\x02\x02\u{145}\
	\u{143}\x03\x02\x02\x02\u{145}\u{146}\x03\x02\x02\x02\u{146}\u{148}\x03\
	\x02\x02\x02\u{147}\u{145}\x03\x02\x02\x02\u{148}\u{149}\x07\x21\x02\x02\
	\u{149}\u{156}\x03\x02\x02\x02\u{14a}\u{151}\x07\x20\x02\x02\u{14b}\u{14d}\
	\x05\x04\x03\x02\u{14c}\u{14e}\x07\x03\x02\x02\u{14d}\u{14c}\x03\x02\x02\
	\x02\u{14d}\u{14e}\x03\x02\x02\x02\u{14e}\u{150}\x03\x02\x02\x02\u{14f}\
	\u{14b}\x03\x02\x02\x02\u{150}\u{153}\x03\x02\x02\x02\u{151}\u{14f}\x03\
	\x02\x02\x02\u{151}\u{152}\x03\x02\x02\x02\u{152}\u{154}\x03\x02\x02\x02\
	\u{153}\u{151}\x03\x02\x02\x02\u{154}\u{156}\x07\x21\x02\x02\u{155}\u{12f}\
	\x03\x02\x02\x02\u{155}\u{13d}\x03\x02\x02\x02\u{155}\u{14a}\x03\x02\x02\
	\x02\u{156}\x1f\x03\x02\x02\x02\u{157}\u{15b}\x07\x28\x02\x02\u{158}\u{15a}\
	\x05\x22\x12\x02\u{159}\u{158}\x03\x02\x02\x02\u{15a}\u{15d}\x03\x02\x02\
	\x02\u{15b}\u{159}\x03\x02\x02\x02\u{15b}\u{15c}\x03\x02\x02\x02\u{15c}\
	\u{15f}\x03\x02\x02\x02\u{15d}\u{15b}\x03\x02\x02\x02\u{15e}\u{160}\x05\
	\x1e\x10\x02\u{15f}\u{15e}\x03\x02\x02\x02\u{15f}\u{160}\x03\x02\x02\x02\
	\u{160}\u{161}\x03\x02\x02\x02\u{161}\u{165}\x07\x0f\x02\x02\u{162}\u{164}\
	\x05\x24\x13\x02\u{163}\u{162}\x03\x02\x02\x02\u{164}\u{167}\x03\x02\x02\
	\x02\u{165}\u{163}\x03\x02\x02\x02\u{165}\u{166}\x03\x02\x02\x02\u{166}\
	\u{168}\x03\x02\x02\x02\u{167}\u{165}\x03\x02\x02\x02\u{168}\u{175}\x07\
	\x28\x02\x02\u{169}\u{16d}\x07\x28\x02\x02\u{16a}\u{16c}\x05\x22\x12\x02\
	\u{16b}\u{16a}\x03\x02\x02\x02\u{16c}\u{16f}\x03\x02\x02\x02\u{16d}\u{16b}\
	\x03\x02\x02\x02\u{16d}\u{16e}\x03\x02\x02\x02\u{16e}\u{171}\x03\x02\x02\
	\x02\u{16f}\u{16d}\x03\x02\x02\x02\u{170}\u{172}\x05\x1e\x10\x02\u{171}\
	\u{170}\x03\x02\x02\x02\u{171}\u{172}\x03\x02\x02\x02\u{172}\u{173}\x03\
	\x02\x02\x02\u{173}\u{175}\x07\x28\x02\x02\u{174}\u{157}\x03\x02\x02\x02\
	\u{174}\u{169}\x03\x02\x02\x02\u{175}\x21\x03\x02\x02\x02\u{176}\u{17d}\
	\x07\x0c\x02\x02\u{177}\u{178}\x05\x28\x15\x02\u{178}\u{179}\x07\x08\x02\
	\x02\u{179}\u{17a}\x05\x2a\x16\x02\u{17a}\u{17d}\x03\x02\x02\x02\u{17b}\
	\u{17d}\x05\x28\x15\x02\u{17c}\u{176}\x03\x02\x02\x02\u{17c}\u{177}\x03\
	\x02\x02\x02\u{17c}\u{17b}\x03\x02\x02\x02\u{17d}\x23\x03\x02\x02\x02\u{17e}\
	\u{17f}\x05\x28\x15\x02\u{17f}\u{180}\x07\x08\x02\x02\u{180}\u{181}\x05\
	\x2a\x16\x02\u{181}\u{184}\x03\x02\x02\x02\u{182}\u{184}\x05\x28\x15\x02\
	\u{183}\u{17e}\x03\x02\x02\x02\u{183}\u{182}\x03\x02\x02\x02\u{184}\x25\
	\x03\x02\x02\x02\u{185}\u{186}\x07\x2d\x02\x02\u{186}\x27\x03\x02\x02\x02\
	\u{187}\u{188}\x07\x22\x02\x02\u{188}\u{18b}\x05\x2a\x16\x02\u{189}\u{18b}\
	\x05\x2a\x16\x02\u{18a}\u{187}\x03\x02\x02\x02\u{18a}\u{189}\x03\x02\x02\
	\x02\u{18b}\x29\x03\x02\x02\x02\u{18c}\u{18f}\x05\x1c\x0f\x02\u{18d}\u{18f}\
	\x07\x2a\x02\x02\u{18e}\u{18c}\x03\x02\x02\x02\u{18e}\u{18d}\x03\x02\x02\
	\x02\u{18f}\x2b\x03\x02\x02\x02\u{190}\u{191}\x07\x2c\x02\x02\u{191}\x2d\
	\x03\x02\x02\x02\u{192}\u{193}\x07\x36\x02\x02\u{193}\x2f\x03\x02\x02\x02\
	\x29\x32\x36\x43\x4d\x53\x5a\x5f\x6e\x72\u{9d}\u{a6}\u{af}\u{bb}\u{c6}\u{fd}\
	\u{ff}\u{10a}\u{110}\u{118}\u{120}\u{124}\u{12b}\u{134}\u{138}\u{141}\u{145}\
	\u{14d}\u{151}\u{155}\u{15b}\u{15f}\u{165}\u{16d}\u{171}\u{174}\u{17c}\u{183}\
	\u{18a}\u{18e}";
