use crate::error::BBError;
use crate::instruction::Instruction;
use crate::vm::VmState;

use gc_arena::{lock::RefLock, Collect, Gc, Mutation};
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Formatter};
use ulid::Ulid;

#[derive(Clone, Copy, Debug, PartialEq, Hash)]
pub struct GcUlid(pub Ulid);

unsafe impl<'gc> Collect<'gc> for GcUlid {
    const NEEDS_TRACE: bool = false;
}

#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct GcRegex(pub Regex);

impl fmt::Debug for GcRegex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Regex({})", self.0.as_str())
    }
}

pub trait AnyCollect: Debug {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn trace_gc<'gc>(&self, cc: &mut dyn gc_arena::collect::Trace<'gc>);
}

unsafe impl<'gc> Collect<'gc> for Box<dyn AnyCollect> {
    const NEEDS_TRACE: bool = true;
    fn trace<T: gc_arena::collect::Trace<'gc>>(&self, cc: &mut T) {
        self.as_ref().trace_gc(cc);
    }
}

pub struct OpaqueState<T>(pub T);

impl<T: 'static> Debug for OpaqueState<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "OpaqueState({:?})", self)
    }
}

impl<T: 'static> AnyCollect for OpaqueState<T> {
    fn as_any(&self) -> &dyn std::any::Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        &mut self.0
    }

    fn trace_gc<'gc>(&self, _cc: &mut dyn gc_arena::collect::Trace<'gc>) {}
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Collect)]
#[collect(require_static)]
pub struct NamespacedName {
    pub path: Vec<String>,
    pub name: String,
}

impl NamespacedName {
    pub fn new(path: Vec<String>, name: String) -> Self {
        Self { path, name }
    }

    pub fn parse(s: &str) -> Self {
        if s.starts_with('[') {
            if let Some(close_idx) = s.find(']') {
                let ns_part = &s[1..close_idx];
                let name = s[close_idx + 1..].to_string();
                let path = if ns_part == "/" || ns_part.is_empty() {
                    Vec::new()
                } else {
                    ns_part.split('/').map(|x| x.to_string()).collect()
                };
                return Self { path, name };
            }
        }
        Self {
            path: Vec::new(),
            name: s.to_string(),
        }
    }

    pub fn from_ast(id: &crate::parser::ast_visitor::IdentifierNode) -> Self {
        let path = if let Some(ns) = &id.namespace {
            ns.identifiers
                .iter()
                .map(|ident| ident.name.clone())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            path,
            name: id.name.clone(),
        }
    }

    pub fn to_explicit_string(&self) -> String {
        if self.path.is_empty() {
            format!("[/]{}", self.name)
        } else {
            format!("[{}]{}", self.path.join("/"), self.name)
        }
    }
}

impl fmt::Display for NamespacedName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "[{}]{}", self.path.join("/"), self.name)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NativeFunc(
    pub for<'a> fn(&mut VmState<'a>, &Mutation<'a>, Vec<Value<'a>>) -> Result<Value<'a>, BBError>,
);

impl NativeFunc {
    pub fn new(
        f: for<'a> fn(
            &mut VmState<'a>,
            &Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, BBError>,
    ) -> Self {
        Self(f)
    }
}

unsafe impl<'gc> Collect<'gc> for NativeFunc {
    const NEEDS_TRACE: bool = false;
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub enum Value<'gc> {
    Object(Gc<'gc, RefLock<Object<'gc>>>),
    Class(Gc<'gc, RefLock<Class<'gc>>>),
    ClassMeta(Gc<'gc, RefLock<Class<'gc>>>),
}

#[derive(Clone, Copy, Collect, Debug)]
#[collect(no_drop)]
pub enum ObjectPayload<'gc> {
    Nil,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(Gc<'gc, String>),
    Dict(Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>),
    Regex(Gc<'gc, GcRegex>),
    Block(Gc<'gc, Block<'gc>>),
    Native(NativeFunc),
    Instance,
    NativeState(Gc<'gc, RefLock<Box<dyn AnyCollect>>>),
}

impl<'gc> Value<'gc> {
    pub fn is_nil(&self) -> bool {
        if let Value::Object(obj) = self
            && let ObjectPayload::Nil = &obj.borrow().payload
        {
            true
        } else {
            false
        }
    }

    pub fn is_true(&self) -> bool {
        if let Value::Object(obj) = self
            && let ObjectPayload::Bool(b) = &obj.borrow().payload
        {
            *b
        } else {
            false
        }
    }

    pub fn is_false(&self) -> bool {
        if let Value::Object(obj) = self
            && let ObjectPayload::Bool(b) = &obj.borrow().payload
        {
            !*b
        } else {
            false
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Nil => false,
                ObjectPayload::Bool(b) => *b,
                _ => true,
            },
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Class(_) => "Class",
            Value::ClassMeta(_) => "ClassMeta",
            Value::Object(obj) => {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::Nil => "Nil",
                    ObjectPayload::Bool(_) => "Boolean",
                    ObjectPayload::Int(_) => "Integer",
                    ObjectPayload::Double(_) => "Double",
                    ObjectPayload::String(_) => "String",
                    ObjectPayload::Dict(_) => "Dictionary",
                    ObjectPayload::Regex(_) => "Regex",
                    ObjectPayload::Block(_) => "Block",
                    ObjectPayload::Native(_) => "Native",
                    _ => {
                        if borrowed.class_name() == "List" {
                            "List"
                        } else {
                            "Object"
                        }
                    }
                }
            }
        }
    }

    pub fn with_native_state<T: 'static, R, F: FnOnce(&T) -> R>(&self, f: F) -> Result<R, String> {
        if let Value::Object(obj) = self {
            let borrowed = obj.borrow();
            if let ObjectPayload::NativeState(state_cell) = &borrowed.payload {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                if let Some(concrete) = any_ref.downcast_ref::<T>() {
                    return Ok(f(concrete));
                }
            }
        }
        Err("Not a native state of the requested type".to_string())
    }

    pub fn with_native_state_mut<T: 'static, R, F: FnOnce(&mut T) -> R>(
        &self,
        mc: &Mutation<'gc>,
        f: F,
    ) -> Result<R, String> {
        if let Value::Object(obj) = self {
            let borrowed = obj.borrow();
            if let ObjectPayload::NativeState(state_cell) = &borrowed.payload {
                let mut state_ref = state_cell.borrow_mut(mc);
                let any_mut = (**state_ref).as_any_mut();
                if let Some(concrete) = any_mut.downcast_mut::<T>() {
                    return Ok(f(concrete));
                }
            }
        }
        Err("Not a native state of the requested type".to_string())
    }
}

impl<'gc> PartialEq for Value<'gc> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Class(a), Value::Class(b)) => Gc::ptr_eq(*a, *b),
            (Value::ClassMeta(a), Value::ClassMeta(b)) => Gc::ptr_eq(*a, *b),
            (Value::Object(a), Value::Object(b)) => {
                let a_borrow = a.borrow();
                let b_borrow = b.borrow();
                match (&a_borrow.payload, &b_borrow.payload) {
                    (ObjectPayload::Nil, ObjectPayload::Nil) => true,
                    (ObjectPayload::Bool(x), ObjectPayload::Bool(y)) => x == y,
                    (ObjectPayload::Int(x), ObjectPayload::Int(y)) => x == y,
                    (ObjectPayload::Double(x), ObjectPayload::Double(y)) => x == y,
                    (ObjectPayload::Int(x), ObjectPayload::Double(y)) => (*x as f64) == *y,
                    (ObjectPayload::Double(x), ObjectPayload::Int(y)) => *x == (*y as f64),
                    (ObjectPayload::String(x), ObjectPayload::String(y)) => **x == **y,
                    (ObjectPayload::NativeState(x), ObjectPayload::NativeState(y)) => Gc::ptr_eq(*x, *y),
                    (ObjectPayload::Dict(x), ObjectPayload::Dict(y)) => Gc::ptr_eq(*x, *y),
                    (ObjectPayload::Regex(x), ObjectPayload::Regex(y)) => Gc::ptr_eq(*x, *y),
                    (ObjectPayload::Block(x), ObjectPayload::Block(y)) => Gc::ptr_eq(*x, *y),
                    (ObjectPayload::Native(x), ObjectPayload::Native(y)) => {
                        let a_ptr = x.0 as *const ();
                        let b_ptr = y.0 as *const ();
                        a_ptr == b_ptr
                    }
                    _ => a_borrow.id == b_borrow.id,
                }
            }
            _ => false,
        }
    }
}

impl<'gc> fmt::Debug for Value<'gc> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Value::Class(c) => write!(f, "Class({})", c.borrow().name),
            Value::ClassMeta(c) => write!(f, "ClassMeta({})", c.borrow().name),
            Value::Object(o) => {
                let o_borrow = o.borrow();
                match &o_borrow.payload {
                    ObjectPayload::Nil => write!(f, "Nil"),
                    ObjectPayload::Bool(b) => write!(f, "Bool({})", b),
                    ObjectPayload::Int(i) => write!(f, "Int({})", i),
                    ObjectPayload::Double(fl) => write!(f, "Float({})", fl),
                    ObjectPayload::String(s) => write!(f, "String({:?})", *s),
                    _ if o_borrow.class_name() == "List" => write!(f, "List(...)"),
                    ObjectPayload::Dict(_) => write!(f, "Dict(...)"),
                    ObjectPayload::Regex(r) => write!(f, "{:?}", r),
                    ObjectPayload::Block(b) => write!(f, "Block({:?})", b.name),
                    ObjectPayload::Native(_) => write!(f, "Native(<fn>)"),
                    _ => {
                        let name = o_borrow.class.borrow().name.clone();
                        write!(f, "Object({}, {{{:?}}})", name, o_borrow.fields)
                    }
                }
            }
        }
    }
}

impl<'gc> fmt::Display for Value<'gc> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Value::Class(c) => write!(f, "class {}", c.borrow().name),
            Value::ClassMeta(c) => write!(f, "class {} meta", c.borrow().name),
            Value::Object(o) => {
                let o_borrow = o.borrow();
                match &o_borrow.payload {
                    ObjectPayload::Nil => write!(f, "nil"),
                    ObjectPayload::Bool(b) => write!(f, "{}", b),
                    ObjectPayload::Int(i) => write!(f, "{}", i),
                    ObjectPayload::Double(fl) => write!(f, "{}", fl),
                    ObjectPayload::String(s) => write!(f, "{}", **s),
                    _ if o_borrow.class_name() == "List" => {
                        if let Ok(res) = self.with_native_state::<crate::runtime::list::NativeListState, _, _>(|l| {
                            let vec = l.get_vec();
                            let mut s = String::new();
                            s.push_str("#(");
                            for (i, val) in vec.iter().enumerate() {
                                if i > 0 {
                                    s.push(' ');
                                }
                                s.push_str(&format!("{}", val));
                            }
                            s.push(')');
                            s
                        }) {
                            write!(f, "{}", res)
                        } else {
                            write!(f, "List(...)")
                        }
                    }
                    ObjectPayload::Dict(d) => {
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
                    ObjectPayload::Regex(r) => write!(f, "#/{}/", r.0.as_str()),
                    ObjectPayload::Block(b) => {
                        if let Some(ref name) = b.name {
                            write!(f, "<block {}>", name)
                        } else {
                            write!(f, "<block>")
                        }
                    }
                    ObjectPayload::Native(_) => write!(f, "<native fn>"),
                    _ => {
                        let name = o_borrow.class.borrow().name.clone();
                        write!(f, "{}{{", name)?;
                        for (i, (k, v)) in o_borrow.fields.iter().enumerate() {
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
    }
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Block<'gc> {
    pub name: Option<String>,
    pub is_nested_block: bool,
    pub param_names: Vec<String>,
    pub bytecode: Vec<Instruction>,
    pub parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    pub enclosing_method_id: Option<usize>,
}

#[derive(Collect, Debug)]
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
        mc: &Mutation<'gc>,
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

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Class<'gc> {
    pub name: NamespacedName,
    pub parent: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub instance_vars: Vec<String>,
    pub instance_methods: HashMap<String, Value<'gc>>,
    pub class_methods: HashMap<String, Value<'gc>>,
    pub mixin_classes: Vec<Gc<'gc, RefLock<Class<'gc>>>>,
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Object<'gc> {
    pub id: GcUlid,
    pub class: Gc<'gc, RefLock<Class<'gc>>>,
    pub fields: HashMap<String, Value<'gc>>,
    pub payload: ObjectPayload<'gc>,
}

impl<'gc> Object<'gc> {
    pub fn class_name(&self) -> String {
        self.class.borrow().name.to_string()
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
            &Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, BBError>,
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
            &Mutation<'a>,
            Vec<Value<'a>>,
        ) -> Result<Value<'a>, BBError>,
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
