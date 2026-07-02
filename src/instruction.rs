use crate::symbol::Symbol;
use crate::value::{NamespacedName, SourceInfo};

use gc_arena::Collect;
use std::ops::Deref;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedBytecode(pub Rc<Vec<Instruction>>);

unsafe impl<'gc> Collect<'gc> for SharedBytecode {
    const NEEDS_TRACE: bool = false;
}

impl Deref for SharedBytecode {
    type Target = [Instruction];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<Instruction>> for SharedBytecode {
    fn from(v: Vec<Instruction>) -> Self {
        SharedBytecode(Rc::new(v))
    }
}

impl PartialEq<Vec<Instruction>> for SharedBytecode {
    fn eq(&self, other: &Vec<Instruction>) -> bool {
        self.0.as_ref() == other
    }
}

impl PartialEq<SharedBytecode> for Vec<Instruction> {
    fn eq(&self, other: &SharedBytecode) -> bool {
        self == other.0.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedSourceMap(pub Rc<Vec<Option<SourceInfo>>>);

unsafe impl<'gc> Collect<'gc> for SharedSourceMap {
    const NEEDS_TRACE: bool = false;
}

impl Deref for SharedSourceMap {
    type Target = [Option<SourceInfo>];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<Option<SourceInfo>>> for SharedSourceMap {
    fn from(v: Vec<Option<SourceInfo>>) -> Self {
        SharedSourceMap(Rc::new(v))
    }
}

impl PartialEq<Vec<Option<SourceInfo>>> for SharedSourceMap {
    fn eq(&self, other: &Vec<Option<SourceInfo>>) -> bool {
        self.0.as_ref() == other
    }
}

impl PartialEq<SharedSourceMap> for Vec<Option<SourceInfo>> {
    fn eq(&self, other: &SharedSourceMap) -> bool {
        self == other.0.as_ref()
    }
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub struct StaticBlock {
    pub name: Option<String>,
    pub is_nested_block: bool,
    /// Parameter names interned at compile time; copied into the runtime `Block`.
    pub param_syms: Vec<Symbol>,
    pub param_types: Vec<String>,
    pub bytecode: SharedBytecode,
    pub source_info: Option<SourceInfo>,
    pub decl_block: Option<Box<StaticBlock>>,
    pub source_map: SharedSourceMap,
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Constant {
    Nil,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Symbol(String),
    Block(StaticBlock),
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Instruction {
    LoadLocal(Symbol),
    DefineLocal(Symbol),
    StoreLocal(Symbol),
    LoadGlobal(NamespacedName),
    StoreGlobal(NamespacedName, bool),
    Push(Constant),
    Pop,
    Dup,
    Send(Symbol, usize), // selector, num_args
    // A self-send to a same-class method of a *sealed* class (Slice 2b-B). Same operand
    // shape as `Send` (receiver + args on the stack); emitted by the compiler when it can
    // prove the target method is fixed. Phase 1: behaves exactly like `Send`. Phase 2: a
    // guard-free per-call-site cache resolves the callable once and reuses it (sealed ⇒
    // never invalidated), skipping `lookup_method`.
    CallSelfDirect(Symbol, usize), // selector, num_args
    // Superinstructions: a single fused op for the hot `<operand-load>; Send` pairs (the
    // last operand of a send is overwhelmingly a local / constant / field — see
    // profiling/superinstructions). Each pushes its operand then runs the normal send,
    // saving one dispatch-loop step per send. Produced by the `fuse_bytecode` peephole
    // pass; never emitted directly by the AST compiler.
    SendLocal(Symbol, Symbol, usize), // var, selector, num_args  (was LoadLocal; Send)
    SendConst(Constant, Symbol, usize), // constant, selector, num_args  (was Push; Send)
    SendField(String, Symbol, usize), // field, selector, num_args  (was LoadField; Send)
    // Store-and-keep superinstructions: a `Dup; Store*` pair (an assignment whose value is
    // used as an expression) fused into one op that stores the *top* of stack without
    // popping it. The statement-position form `Dup; Store*; Pop` is instead collapsed to a
    // plain `Store*` (both by the `fuse_bytecode` pass). Mirror DefineLocal/StoreLocal/
    // StoreField but peek instead of pop.
    DefineLocalKeep(Symbol),
    StoreLocalKeep(Symbol),
    StoreFieldKeep(String),
    // 3-instruction sends: absorb a *second* operand-load into a fused send, so one op
    // pushes two operands (left-to-right) then dispatches. Covers the two hottest
    // receiver+last-operand shapes — `LoadLocal; LoadLocal; Send` (e.g. `i < n`) and
    // `LoadLocal; Push; Send` (e.g. `n - 1`). The operands are just the last two pushed
    // before the send (receiver + arg for a 1-arg send); produced by `fuse_bytecode`.
    SendLocalLocal(Symbol, Symbol, Symbol, usize), // local, local, selector, num_args
    SendLocalConst(Symbol, Constant, Symbol, usize), // local, constant, selector, num_args
    // Devirtualized Integer operators (Slice 2a): the compiler emits these instead of a
    // `Send("+:", 1)` etc. when both operands are statically `Integer` (a sealed value
    // type). Each pops two `Value::Int`s and pushes the result directly — no method
    // lookup, no dispatch. Semantics match Integer's native ops exactly: `+`/`-`/`*` wrap
    // like i64; `/`/`%` raise "Division by zero" on a zero divisor; compares yield a Bool.
    IntAdd,
    IntSub,
    IntMul,
    IntDiv,
    IntMod,
    IntLt,
    IntLe,
    IntGt,
    IntGe,
    IntEq,
    IntNe,
    Return,
    Yeet,
    BlockReturn,
    MethodReturn,
    Jump(isize),
    IfJump(isize),
    ElseJump(isize),
    // Guard for control-flow inlining on a non-statically-Bool receiver (Slice 2d, option
    // C). Peeks the stack top (the conditional's receiver): if it is *not* a `Bool`, jump
    // by the offset to a cold path that performs the real `if:`/`if:else:` send (preserving
    // MessageNotUnderstood / a user-defined `if:else:`), leaving the receiver on the stack;
    // if it *is* a `Bool`, fall through to the inlined branch (which consumes it). Never pops.
    BranchIfNotBool(isize),
    NewList(usize), // num_elements
    NewMap(usize),  // num_pairs (key/value count)
    NewSet(usize),  // num_elements
    NewRegex,
    DefineClass {
        name: NamespacedName,
        parent_name: Option<NamespacedName>,
        instance_vars: Vec<String>,
    },
    ExecuteBlockWithSelf,
    DefineMethod(String),
    OverrideMethod(String),
    LoadField(String),
    StoreField(String),
    /// `use (pkg:)? path;` — load a file once. `package` is `None` for stdlib; `path`
    /// has `.qn` implied; `glob` loads every `.qn` in the directory (Stage 2).
    Use {
        package: Option<String>,
        path: String,
        glob: bool,
    },
}
