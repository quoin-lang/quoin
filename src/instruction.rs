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
    // Superinstructions: a single fused op for the hot `<operand-load>; Send` pairs (the
    // last operand of a send is overwhelmingly a local / constant / field — see
    // profiling/superinstructions). Each pushes its operand then runs the normal send,
    // saving one dispatch-loop step per send. Produced by the `fuse_bytecode` peephole
    // pass; never emitted directly by the AST compiler.
    SendLocal(Symbol, Symbol, usize), // var, selector, num_args  (was LoadLocal; Send)
    SendConst(Constant, Symbol, usize), // constant, selector, num_args  (was Push; Send)
    SendField(String, Symbol, usize), // field, selector, num_args  (was LoadField; Send)
    Return,
    Yeet,
    BlockReturn,
    MethodReturn,
    Jump(isize),
    IfJump(isize),
    ElseJump(isize),
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
