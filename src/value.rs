use crate::instruction::Instruction;
use crate::vm::VmState;

use gc_arena::{lock::RefLock, Collect, Gc};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct BBRegex(pub Regex);

impl fmt::Debug for BBRegex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Regex({})", self.0.as_str())
    }
}

#[derive(Clone, Copy)]
pub struct NativeFunc(
    pub  for<'a> fn(
        &mut VmState<'a>,
        &gc_arena::Mutation<'a>,
        Vec<Value<'a>>,
    ) -> Result<Value<'a>, Box<dyn Error>>,
);

impl NativeFunc {
    pub fn new(
        f: for<'a> fn(
            &mut VmState<'a>,
            &gc_arena::Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, Box<dyn Error>>,
    ) -> Self {
        Self(f)
    }
}

unsafe impl<'gc> Collect<'gc> for NativeFunc {
    const NEEDS_TRACE: bool = false;
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Value<'gc> {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Gc<'gc, String>),
    List(Gc<'gc, RefLock<Vec<Value<'gc>>>>),
    Dict(Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>),
    Regex(Gc<'gc, BBRegex>),
    Block(Gc<'gc, Block<'gc>>),
    Method(Gc<'gc, Method<'gc>>),
    Native(NativeFunc),
    Class(Gc<'gc, RefLock<Class<'gc>>>),
    Object(Gc<'gc, RefLock<Object<'gc>>>),
}

unsafe impl<'gc> Collect<'gc> for Value<'gc> {
    const NEEDS_TRACE: bool = true;

    #[inline]
    fn trace<C: gc_arena::collect::Trace<'gc>>(&self, cc: &mut C) {
        match self {
            Value::Nil | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Native(_) => {}
            Value::String(s) => cc.trace(s),
            Value::List(l) => cc.trace(l),
            Value::Dict(d) => cc.trace(d),
            Value::Regex(r) => cc.trace(r),
            Value::Block(b) => cc.trace(b),
            Value::Method(m) => {
                cc.trace(&m.receiver);
                cc.trace(&m.block);
            }
            Value::Class(c) => cc.trace(c),
            Value::Object(o) => cc.trace(o),
        }
    }
}

impl<'gc> Value<'gc> {
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "Nil",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::String(_) => "String",
            Value::List(_) => "List",
            Value::Dict(_) => "Dict",
            Value::Regex(_) => "Regex",
            Value::Block(_) => "Block",
            Value::Method(_) => "Method",
            Value::Native(_) => "Native",
            Value::Class(_) => "Class",
            Value::Object(_) => "Object",
        }
    }
}

impl<'gc> PartialEq for Value<'gc> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => **a == **b,
            (Value::List(a), Value::List(b)) => Gc::ptr_eq(*a, *b),
            (Value::Dict(a), Value::Dict(b)) => Gc::ptr_eq(*a, *b),
            (Value::Regex(a), Value::Regex(b)) => Gc::ptr_eq(*a, *b),
            (Value::Block(a), Value::Block(b)) => Gc::ptr_eq(*a, *b),
            (Value::Method(a), Value::Method(b)) => {
                Gc::ptr_eq(a.block, b.block) && a.receiver == b.receiver
            }
            (Value::Class(a), Value::Class(b)) => Gc::ptr_eq(*a, *b),
            (Value::Object(a), Value::Object(b)) => Gc::ptr_eq(*a, *b),
            (Value::Native(a), Value::Native(b)) => {
                let a_ptr = a.0 as *const ();
                let b_ptr = b.0 as *const ();
                a_ptr == b_ptr
            }
            _ => false,
        }
    }
}

impl<'gc> fmt::Debug for Value<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "Nil"),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Int(i) => write!(f, "Int({})", i),
            Value::Float(fl) => write!(f, "Float({})", fl),
            Value::String(s) => write!(f, "String({:?})", **s),
            Value::List(_) => write!(f, "List(...)"),
            Value::Dict(_) => write!(f, "Dict(...)"),
            Value::Regex(r) => write!(f, "{:?}", r),
            Value::Block(b) => write!(f, "Block({:?})", b.name),
            Value::Method(m) => write!(f, "Method({}#{})", m.receiver.type_name(), m.name),
            Value::Native(_) => write!(f, "Native(<fn>)"),
            Value::Class(c) => write!(f, "Class({})", c.borrow().name),
            Value::Object(o) => {
                let name = o.borrow().class.borrow().name.clone();
                write!(f, "Object({}, {{{:?}}})", name, o.borrow().fields)
            }
        }
    }
}

impl<'gc> fmt::Display for Value<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "{}", **s),
            Value::List(l) => {
                let borrowed = l.borrow();
                write!(f, "#[")?;
                for (i, val) in borrowed.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            Value::Dict(d) => {
                let borrowed = d.borrow();
                write!(f, "#{{")?;
                for (i, (k, v)) in borrowed.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Regex(r) => write!(f, "#/{}/", r.0.as_str()),
            Value::Block(b) => {
                if let Some(ref name) = b.name {
                    write!(f, "<block {}>", name)
                } else {
                    write!(f, "<block>")
                }
            }
            Value::Method(m) => write!(f, "<method {}#{}>", m.receiver.type_name(), m.name),
            Value::Native(_) => write!(f, "<native fn>"),
            Value::Class(c) => write!(f, "class {}", c.borrow().name),
            Value::Object(o) => {
                let name = o.borrow().class.borrow().name.clone();
                write!(f, "{}{{", name)?;
                for (i, (k, v)) in o.borrow().fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Block<'gc> {
    pub name: Option<String>,
    pub is_nested_block: bool,
    pub param_names: Vec<String>,
    pub bytecode: Vec<Instruction>,
    pub parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct Method<'gc> {
    pub name: String,
    pub receiver: Value<'gc>,
    pub block: Gc<'gc, Block<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct EnvFrame<'gc> {
    pub parent: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    pub vars: HashMap<String, Value<'gc>>,
}

impl<'gc> EnvFrame<'gc> {
    pub fn new(parent: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>) -> Self {
        Self {
            parent,
            vars: HashMap::new(),
        }
    }

    pub fn get(frame: Gc<'gc, RefLock<Self>>, name: &str) -> Option<Value<'gc>> {
        let borrowed = frame.borrow();
        if let Some(val) = borrowed.vars.get(name) {
            Some(*val)
        } else if let Some(parent) = borrowed.parent {
            Self::get(parent, name)
        } else {
            None
        }
    }

    pub fn set(
        frame: Gc<'gc, RefLock<Self>>,
        mc: &gc_arena::Mutation<'gc>,
        name: &str,
        val: Value<'gc>,
    ) -> bool {
        let mut current = Some(frame);
        while let Some(curr) = current {
            if curr.borrow().vars.contains_key(name) {
                curr.borrow_mut(mc).vars.insert(name.to_string(), val);
                return true;
            }
            current = curr.borrow().parent;
        }
        false
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Class<'gc> {
    pub name: String,
    pub parent: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub instance_methods: HashMap<String, Value<'gc>>,
    pub class_methods: HashMap<String, Value<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Object<'gc> {
    pub class: Gc<'gc, RefLock<Class<'gc>>>,
    pub fields: HashMap<String, Value<'gc>>,
}

impl<'gc> Object<'gc> {
    pub fn class_name(&self) -> String {
        self.class.borrow().name.clone()
    }

    pub fn get_field_or_default(&self, name: &str) -> Value<'gc> {
        self.fields.get(name).copied().unwrap_or(Value::Nil)
    }
}

pub trait NativeClass {
    fn parent_name(&self) -> Option<&'static str>;
    fn name(&self) -> &'static str;
    fn class_methods(&self) -> HashMap<String, NativeFunc>;
    fn instance_methods(&self) -> HashMap<String, NativeFunc>;
}

pub struct NativeClassBuilder {
    parent_name: Option<&'static str>,
    name: &'static str,
    class_methods: HashMap<String, NativeFunc>,
    instance_methods: HashMap<String, NativeFunc>,
}

impl NativeClassBuilder {
    pub fn new(name: &'static str, parent_name: Option<&'static str>) -> Self {
        Self {
            parent_name,
            name,
            class_methods: HashMap::new(),
            instance_methods: HashMap::new(),
        }
    }

    pub fn class_method(
        mut self,
        selector: &str,
        f: for<'a> fn(
            &mut VmState<'a>,
            &gc_arena::Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, Box<dyn Error>>,
    ) -> Self {
        self.class_methods
            .insert(selector.to_string(), NativeFunc(f));
        self
    }

    pub fn instance_method(
        mut self,
        selector: &str,
        f: for<'a> fn(
            &mut VmState<'a>,
            &gc_arena::Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, Box<dyn Error>>,
    ) -> Self {
        self.instance_methods
            .insert(selector.to_string(), NativeFunc(f));
        self
    }
}

impl NativeClass for NativeClassBuilder {
    fn parent_name(&self) -> Option<&'static str> {
        self.parent_name
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn class_methods(&self) -> HashMap<String, NativeFunc> {
        self.class_methods.clone()
    }

    fn instance_methods(&self) -> HashMap<String, NativeFunc> {
        self.instance_methods.clone()
    }
}
