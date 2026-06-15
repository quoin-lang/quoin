use crate::value::NamespacedName;
use gc_arena::Collect;

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub struct StaticBlock {
    pub name: Option<String>,
    pub is_nested_block: bool,
    pub param_names: Vec<String>,
    pub bytecode: Vec<Instruction>,
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Constant {
    Nil,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Block(StaticBlock),
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Instruction {
    LoadLocal(String),
    DefineLocal(String),
    StoreLocal(String),
    LoadGlobal(NamespacedName),
    StoreGlobal(NamespacedName),
    Push(Constant),
    Pop,
    Dup,
    Send(String, usize), // selector, num_args
    Return,
    Yeet,
    BlockReturn,
    MethodReturn,
    Jump(isize),
    IfJump(isize),
    ElseJump(isize),
    NewList(usize), // num_elements
    NewDict(usize), // num_pairs (key/value count)
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
}
