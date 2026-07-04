use crate::error::QuoinError;
use crate::instruction::{SharedBytecode, SharedSourceMap};
use crate::parser::ast::IdentifierNode;
use crate::runtime::list::NativeListState;
use crate::runtime::map::{NativeKeyValuePairState, NativeMapState};
use crate::runtime::regex::NativeRegexState;
use crate::runtime::set::NativeSetState;
use crate::symbol::Symbol;
use crate::vm::{ICSlot, VmState};

use gc_arena::collect::Trace;
use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use std::any::Any;
use std::cell::RefCell;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::fmt;
use std::fmt::{Debug, Formatter};

pub trait AnyCollect: Debug {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>);
}

unsafe impl<'gc> Collect<'gc> for Box<dyn AnyCollect> {
    const NEEDS_TRACE: bool = true;
    fn trace<T: Trace<'gc>>(&self, cc: &mut T) {
        self.as_ref().trace_gc(cc);
    }
}

pub struct OpaqueState<T>(pub T);

impl<T: 'static> Debug for OpaqueState<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "OpaqueState<{}>", std::any::type_name::<T>())
    }
}

impl<T: 'static> AnyCollect for OpaqueState<T> {
    fn as_any(&self) -> &dyn Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.0
    }

    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

// `SourceInfo` now lives in the standalone `quoin-syntax` crate (its `Collect`
// impl is gated behind that crate's `gc` feature, which the `quoin` crate
// enables). Re-exported here so existing `crate::value::SourceInfo` paths work.
pub use quoin_syntax::SourceInfo;

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

    pub fn from_ast(id: &IdentifierNode) -> Self {
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

/// A legacy native method: takes `&mut VmState` directly. Being migrated to
/// [`SdkFn`](crate::ext_sdk::SdkFn), which takes `&mut dyn Host` and so can only
/// touch the curated SDK surface. Both coexist during the migration.
pub type LegacyNativeFn = for<'a> fn(
    &mut VmState<'a>,
    &Mutation<'a>,
    Value<'a>,
    Vec<Value<'a>>,
) -> Result<Value<'a>, QuoinError>;

/// A native method body. `Legacy` reaches into `VmState`; `Sdk` is written against
/// the `ext_sdk::Host` surface. Dispatch (`Callable::call`) branches on the variant.
#[derive(Clone, Copy, Debug)]
pub enum NativeFunc {
    Legacy(LegacyNativeFn),
    Sdk(crate::ext_sdk::SdkFn),
}

impl NativeFunc {
    pub fn new(f: LegacyNativeFn) -> Self {
        Self::Legacy(f)
    }

    pub fn sdk(f: crate::ext_sdk::SdkFn) -> Self {
        Self::Sdk(f)
    }
}

/// A native method's GC-rooted call context: the receiver and its arguments kept
/// together on `VmState::active_native_args`, so a native fn can re-read them after
/// a nested call that may have collected. One atomic push/pop keeps the pair in sync.
#[derive(Collect)]
#[collect(no_drop)]
pub struct NativeCall<'gc> {
    pub receiver: Value<'gc>,
    pub args: Vec<Value<'gc>>,
}

unsafe impl<'gc> Collect<'gc> for NativeFunc {
    const NEEDS_TRACE: bool = false;
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub enum Value<'gc> {
    /// Immediate value types — no GC allocation. Their class is *derived* from
    /// the variant (see `get_class_for_lookup`), so "numbers are objects" still
    /// holds: they dispatch via `Integer` / `Double` / `Boolean` / `Nil` and
    /// have methods, but no per-instance fields or true eigenclass.
    Int(i64),
    Double(f64),
    Bool(bool),
    Nil,
    Object(Gc<'gc, RefLock<Object<'gc>>>),
    Class(Gc<'gc, RefLock<Class<'gc>>>),
    ClassMeta(Gc<'gc, RefLock<Class<'gc>>>),
}

#[derive(Clone, Copy, Collect, Debug)]
#[collect(no_drop)]
pub enum ObjectPayload<'gc> {
    String(Gc<'gc, String>),
    /// An interned symbol (`#foo`). The inner string is shared across all
    /// occurrences of the same name, so symbols compare by pointer identity.
    Symbol(Gc<'gc, String>),
    /// Immutable binary data (the `Bytes` class). A GC leaf like `String`, but raw
    /// `Vec<u8>` rather than UTF-8 — the currency of the socket/TLS/HTTP layers, which
    /// can't be represented as text. Converts to/from `String` at the edges.
    Bytes(Gc<'gc, Vec<u8>>),
    Block(Gc<'gc, Block<'gc>>),
    Instance,
    NativeState(Gc<'gc, RefLock<Box<dyn AnyCollect>>>),
}

impl<'gc> Value<'gc> {
    /// The integer value if this is an `Integer`, else `None`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The value as `f64`, promoting an `Integer` to floating point. `None` if not
    /// numeric. The shared coercion helper for arithmetic operator variants.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Double(d) => Some(*d),
            _ => None,
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub fn is_true(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    pub fn is_false(&self) -> bool {
        matches!(self, Value::Bool(false))
    }

    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn class_name(&self) -> String {
        match self {
            Value::Int(_) => "Integer".to_string(),
            Value::Double(_) => "Double".to_string(),
            Value::Bool(_) => "Boolean".to_string(),
            Value::Nil => "Nil".to_string(),
            Value::Class(_) => "Class".to_string(),
            Value::ClassMeta(_) => "ClassMeta".to_string(),
            Value::Object(obj) => obj.borrow().class_name(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Integer",
            Value::Double(_) => "Double",
            Value::Bool(_) => "Boolean",
            Value::Nil => "Nil",
            Value::Class(_) => "Class",
            Value::ClassMeta(_) => "ClassMeta",
            Value::Object(obj) => {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::String(_) => "String",
                    ObjectPayload::Symbol(_) => "Symbol",
                    ObjectPayload::Bytes(_) => "Bytes",
                    ObjectPayload::Block(_) => "Block",
                    _ => match borrowed.class_name().as_str() {
                        "List" => "List",
                        "Map" => "Map",
                        "Regex" => "Regex",
                        _ => "Object",
                    },
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

    /// Type-erased mutable access to the native-state payload: runs `f` with the
    /// `&mut dyn Any` behind the `NativeState` cell (write-barriered via `mc`),
    /// returning whether this value had a native-state payload at all. This is the
    /// dyn-safe building block the `ext_sdk::Host` trait exposes; the generic,
    /// downcasting `HostExt::with_native_state_mut` is layered on top.
    pub fn with_native_any_mut<R>(
        &self,
        mc: &Mutation<'gc>,
        f: impl FnOnce(&mut dyn Any) -> R,
    ) -> Option<R> {
        if let Value::Object(obj) = self {
            let borrowed = obj.borrow();
            if let ObjectPayload::NativeState(state_cell) = &borrowed.payload {
                let mut state_ref = state_cell.borrow_mut(mc);
                return Some(f((**state_ref).as_any_mut()));
            }
        }
        None
    }
}

impl<'gc> PartialEq for Value<'gc> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Double(x), Value::Double(y)) => x == y,
            (Value::Int(x), Value::Double(y)) => (*x as f64) == *y,
            (Value::Double(x), Value::Int(y)) => *x == (*y as f64),
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Nil, Value::Nil) => true,
            (Value::Class(a), Value::Class(b)) => Gc::ptr_eq(*a, *b),
            (Value::ClassMeta(a), Value::ClassMeta(b)) => Gc::ptr_eq(*a, *b),
            (Value::Object(a), Value::Object(b)) => {
                let a_borrow = a.borrow();
                let b_borrow = b.borrow();
                match (&a_borrow.payload, &b_borrow.payload) {
                    (ObjectPayload::String(x), ObjectPayload::String(y)) => **x == **y,
                    (ObjectPayload::Symbol(x), ObjectPayload::Symbol(y)) => Gc::ptr_eq(*x, *y),
                    (ObjectPayload::Bytes(x), ObjectPayload::Bytes(y)) => **x == **y,
                    (ObjectPayload::Block(x), ObjectPayload::Block(y)) => Gc::ptr_eq(*x, *y),
                    _ => Gc::ptr_eq(*a, *b),
                }
            }
            _ => false,
        }
    }
}

impl<'gc> fmt::Debug for Value<'gc> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "Int({})", i),
            Value::Double(fl) => write!(f, "Float({})", fl),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Nil => write!(f, "Nil"),
            Value::Class(c) => write!(f, "Class({})", c.borrow().name),
            Value::ClassMeta(c) => write!(f, "ClassMeta({})", c.borrow().name),
            Value::Object(o) => {
                let o_borrow = o.borrow();
                match &o_borrow.payload {
                    ObjectPayload::String(s) => write!(f, "String({:?})", *s),
                    ObjectPayload::Symbol(s) => write!(f, "#{}", **s),
                    ObjectPayload::Bytes(b) => write!(f, "Bytes[{}]", b.len()),
                    _ if o_borrow.class_name() == "List" => write!(f, "List(...)"),
                    _ if o_borrow.class_name() == "Map" => write!(f, "Map(...)"),
                    _ if o_borrow.class_name() == "Set" => write!(f, "Set(...)"),
                    _ if o_borrow.class_name() == "Regex" => {
                        if let Ok(res) = self.with_native_state::<NativeRegexState, _, _>(|r| {
                            format!("{:?}", r.regex)
                        }) {
                            write!(f, "Regex({})", res)
                        } else {
                            write!(f, "Regex(...)")
                        }
                    }
                    _ if o_borrow.class_name() == "KeyValuePair" => {
                        if let Ok(res) =
                            self.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| {
                                format!("key={:?} value={:?}", kvp.get_key(), kvp.get_value())
                            })
                        {
                            write!(f, "KeyValuePair{{{}}}", res)
                        } else {
                            write!(f, "KeyValuePair(...)")
                        }
                    }
                    ObjectPayload::Block(b) => write!(f, "Block({:?})", b.name),
                    _ => {
                        let name = o_borrow.class.borrow().name.clone();
                        write!(f, "Object({}, {{{:?}}})", name, o_borrow.fields)
                    }
                }
            }
        }
    }
}

thread_local! {
    static FORMATTING_OBJECTS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

struct FormattingGuard {
    id: usize,
}

impl Drop for FormattingGuard {
    fn drop(&mut self) {
        FORMATTING_OBJECTS.with(|set| {
            set.borrow_mut().remove(&self.id);
        });
    }
}

impl<'gc> fmt::Display for Value<'gc> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Double(fl) => write!(f, "{}", fl),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Nil => write!(f, "nil"),
            Value::Class(c) => write!(f, "class {}", c.borrow().name),
            Value::ClassMeta(c) => write!(f, "class {} meta", c.borrow().name),
            Value::Object(o) => {
                let id = Gc::as_ptr(*o) as usize;
                let already_formatting =
                    FORMATTING_OBJECTS.with(|set| !set.borrow_mut().insert(id));
                if already_formatting {
                    return write!(f, "{}{{...}}", o.borrow().class.borrow().name);
                }
                let _guard = FormattingGuard { id };

                let o_borrow = o.borrow();
                match &o_borrow.payload {
                    ObjectPayload::String(s) => write!(f, "{}", **s),
                    ObjectPayload::Symbol(s) => write!(f, "#{}", **s),
                    ObjectPayload::Bytes(b) => {
                        // Length + a short hex preview; never dump raw bytes to a terminal.
                        write!(f, "Bytes[{}]", b.len())?;
                        for byte in b.iter().take(16) {
                            write!(f, " {:02x}", byte)?;
                        }
                        if b.len() > 16 {
                            write!(f, " …")?;
                        }
                        Ok(())
                    }
                    _ if o_borrow.class_name() == "List" => {
                        if let Ok(res) = self.with_native_state::<NativeListState, _, _>(|l| {
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
                    _ if o_borrow.class_name() == "Map" => {
                        if let Ok(res) = self.with_native_state::<NativeMapState, _, _>(|m| {
                            let borrowed = m.get_map();
                            let mut parts = Vec::new();
                            for (k, v) in borrowed.iter() {
                                parts.push(format!("{}: {}", k, v));
                            }
                            parts.sort();
                            format!("#{{{}}}", parts.join(" "))
                        }) {
                            write!(f, "{}", res)
                        } else {
                            write!(f, "Map(...)")
                        }
                    }
                    _ if o_borrow.class_name() == "Set" => {
                        if let Ok(res) = self.with_native_state::<NativeSetState, _, _>(|s| {
                            let vec = s.get_vec();
                            let mut out = String::new();
                            out.push_str("#<");
                            for (i, val) in vec.iter().enumerate() {
                                if i > 0 {
                                    out.push(' ');
                                }
                                out.push_str(&format!("{}", val));
                            }
                            out.push('>');
                            out
                        }) {
                            write!(f, "{}", res)
                        } else {
                            write!(f, "Set(...)")
                        }
                    }
                    _ if o_borrow.class_name() == "Regex" => {
                        if let Ok(pattern) = self.with_native_state::<NativeRegexState, _, _>(|r| {
                            r.regex.as_str().to_string()
                        }) {
                            write!(f, "#/{}/", pattern)
                        } else {
                            write!(f, "Regex(...)")
                        }
                    }
                    _ if o_borrow.class_name() == "KeyValuePair" => {
                        if let Ok(res) =
                            self.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| {
                                format!(
                                    "KeyValuePair{{key: {}, value: {}}}",
                                    kvp.get_key(),
                                    kvp.get_value()
                                )
                            })
                        {
                            write!(f, "{}", res)
                        } else {
                            write!(f, "KeyValuePair(...)")
                        }
                    }
                    ObjectPayload::Block(b) => {
                        if let Some(ref name) = b.name {
                            write!(f, "<block {}>", name)
                        } else {
                            write!(f, "<block>")
                        }
                    }
                    _ => {
                        let class = o_borrow.class.borrow();
                        write!(f, "{}{{", class.name)?;
                        // Fields in slot order: `name: value`.
                        let mut by_slot: Vec<(&str, usize)> = class
                            .field_slots
                            .iter()
                            .map(|(n, &s)| (n.as_str(), s))
                            .collect();
                        by_slot.sort_by_key(|&(_, s)| s);
                        let mut first = true;
                        for (n, s) in by_slot {
                            if let Some(v) = o_borrow.fields.get(s) {
                                if !first {
                                    write!(f, " ")?;
                                }
                                first = false;
                                write!(f, "{}: {}", n, v)?;
                            }
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
    /// Parameter names, interned. `Symbol` is the single representation everywhere;
    /// stringify via `as_str()` only for display (`Block#args`, signature output).
    pub param_syms: Vec<Symbol>,
    pub param_types: Vec<String>,
    pub bytecode: SharedBytecode,
    pub parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    pub enclosing_method_id: Option<usize>,
    pub source_info: Option<SourceInfo>,
    pub decl_block: Option<Gc<'gc, Block<'gc>>>,
    pub source_map: SharedSourceMap,
    /// Per-call-site monomorphic inline cache, indexed by `ip` (one slot per instruction). A bare
    /// `RefLock` (not a separate `Gc` alloc) so an uncached block — e.g. a `new:{…}` init block —
    /// costs nothing beyond the inline `None`; the slot array is allocated on the first cacheable
    /// send. Because the executing block roots this array, a cached entry can never be confused
    /// with another block's (no ABA).
    pub inline_cache: RefLock<Option<Box<[ICSlot<'gc>]>>>,
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct EnvFrame<'gc> {
    pub parent: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    /// Local bindings as a small association list keyed by interned [`Symbol`]. A
    /// frame holds only a handful of locals, so a linear scan (comparing `Symbol`s
    /// by pointer) beats a `HashMap`: no per-frame table allocation, no name-string
    /// clone on bind, no SipHash on access. Closures still capture via `parent`.
    pub vars: Vec<(Symbol, Value<'gc>)>,
}

impl<'gc> EnvFrame<'gc> {
    pub fn new(parent: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>) -> Self {
        Self {
            parent,
            vars: Vec::new(),
        }
    }

    /// Read a local by interned name, walking up the lexical (parent) chain.
    pub fn get(frame: Gc<'gc, RefLock<Self>>, name: Symbol) -> Option<Value<'gc>> {
        let borrowed = frame.borrow();
        if let Some(val) = borrowed.lookup(name) {
            Some(val)
        } else if let Some(parent) = borrowed.parent {
            Self::get(parent, name)
        } else {
            None
        }
    }

    /// Assign to the nearest existing binding up the chain; returns whether one was
    /// found (callers bind in the current frame when it wasn't).
    pub fn set(
        frame: Gc<'gc, RefLock<Self>>,
        mc: &Mutation<'gc>,
        name: Symbol,
        val: Value<'gc>,
    ) -> bool {
        let mut current = Some(frame);
        while let Some(curr) = current {
            let pos = curr.borrow().vars.iter().position(|(n, _)| *n == name);
            if let Some(i) = pos {
                curr.borrow_mut(mc).vars[i].1 = val;
                return true;
            }
            current = curr.borrow().parent;
        }
        false
    }

    /// Read a local in *this* frame only, by interned name.
    pub fn lookup(&self, name: Symbol) -> Option<Value<'gc>> {
        self.vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    /// Read a local in *this* frame only, by string name — for callers that hold a
    /// `&str` (instance-var/`init:`-arg population, `bind:` destructuring).
    pub fn lookup_str(&self, name: &str) -> Option<Value<'gc>> {
        self.vars
            .iter()
            .find(|(n, _)| n.as_str() == name)
            .map(|(_, v)| *v)
    }

    /// Bind `name` in this frame: update in place if already present, else append.
    pub fn bind(&mut self, name: Symbol, val: Value<'gc>) {
        match self.vars.iter().position(|(n, _)| *n == name) {
            Some(i) => self.vars[i].1 = val,
            None => self.vars.push((name, val)),
        }
    }
}

/// Intern a block's parameter names to `Symbol`s. Called once per block value when
/// it's created (see `Block::param_syms`), so per-call binding never re-interns.
pub fn intern_param_syms(names: &[String]) -> Vec<Symbol> {
    names.iter().map(|n| Symbol::intern(n)).collect()
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Class<'gc> {
    pub name: NamespacedName,
    pub parent: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub instance_vars: Vec<String>,
    pub instance_methods: FxHashMap<String, Value<'gc>>,
    pub class_methods: FxHashMap<String, Value<'gc>>,
    pub mixin_classes: Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    /// Memoized, append-only instance-variable layout: name -> absolute slot in an
    /// instance's `fields` array. Built lazily from the full hierarchy (own +
    /// mixins + parent) at first instantiation; new ivars only ever append, so
    /// existing slots stay stable across runtime mixins. `len()` is the field count.
    pub field_slots: FxHashMap<String, usize>,
    /// True only for per-instance *eigenclasses* (singletons synthesized by
    /// `get_target_class_for_def` for a `Value::Object` receiver). Named classes —
    /// including the `$TrueClass`/`$FalseClass` boolean singletons, which are
    /// rooted in `globals`/`builtin_cache` — are `false`. The method-dispatch cache
    /// keys on class *pointers*, which is only sound for classes with stable
    /// addresses; eigenclasses are transient (collected when their instance dies →
    /// pointer reuse), so the cache skips any lookup whose receiver or argument
    /// class is an eigenclass.
    pub is_eigenclass: bool,
    /// Set by `sealed!`: the class (or an instance's eigenclass) is frozen — no further
    /// extension (`<--` / `->` / `-->` / `.mix:`) and no subclassing. (The intended
    /// future trigger for devirtualization — a sealed class is a leaf with a fixed
    /// method table.)
    pub is_sealed: bool,
    /// Set by `abstract!`: the class itself can't be instantiated (`new` / `new:`),
    /// though concrete subclasses still can.
    pub is_abstract: bool,
}

/// How many instance fields an `Object` stores inline before spilling to the heap.
/// Small structs (points, pairs, tree nodes — `TreeNode` has 3) fit inline, so
/// constructing one is a *single* GC allocation instead of two (the object plus a
/// separate boxed field slice). Tunable; 4 covers the common small-struct sizes.
const INLINE_FIELD_CAP: usize = 3;

/// Instance-variable storage for an `Object`, indexed by the class's slot layout
/// (`Class::field_slots`). Small field counts live inline in the object itself
/// (`Inline`) — no separate allocation; larger ones spill to a boxed slice (`Boxed`).
/// The count is fixed at construction (a class's slot layout never shrinks), so the
/// variant is chosen once. `Index`/`get`/`len` present a uniform slice view over both.
#[derive(Collect)]
#[collect(no_drop)]
pub enum Fields<'gc> {
    Inline {
        len: u8,
        slots: [Value<'gc>; INLINE_FIELD_CAP],
    },
    Boxed(Box<[Value<'gc>]>),
}

impl<'gc> Fields<'gc> {
    /// Storage for `count` fields, all initialized to `nil`. Inline when it fits.
    pub fn new(count: usize, nil: Value<'gc>) -> Self {
        if count <= INLINE_FIELD_CAP {
            let mut slots = [Value::Nil; INLINE_FIELD_CAP];
            for slot in slots.iter_mut().take(count) {
                *slot = nil;
            }
            Fields::Inline {
                len: count as u8,
                slots,
            }
        } else {
            Fields::Boxed(vec![nil; count].into_boxed_slice())
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Value<'gc>] {
        match self {
            Fields::Inline { len, slots } => &slots[..*len as usize],
            Fields::Boxed(b) => b,
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Value<'gc>] {
        match self {
            Fields::Inline { len, slots } => &mut slots[..*len as usize],
            Fields::Boxed(b) => b,
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> Option<&Value<'gc>> {
        self.as_slice().get(i)
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Fields::Inline { len, .. } => *len as usize,
            Fields::Boxed(b) => b.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Value<'gc>> {
        self.as_slice().iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Value<'gc>> {
        self.as_mut_slice().iter_mut()
    }
}

impl<'gc> Default for Fields<'gc> {
    fn default() -> Self {
        Fields::Inline {
            len: 0,
            slots: [Value::Nil; INLINE_FIELD_CAP],
        }
    }
}

impl<'gc> std::ops::Index<usize> for Fields<'gc> {
    type Output = Value<'gc>;
    #[inline]
    fn index(&self, i: usize) -> &Value<'gc> {
        &self.as_slice()[i]
    }
}

impl<'gc> std::ops::IndexMut<usize> for Fields<'gc> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut Value<'gc> {
        &mut self.as_mut_slice()[i]
    }
}

impl<'gc> std::fmt::Debug for Fields<'gc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Present as a plain slice (unchanged from the old `Box<[Value]>` debug output).
        std::fmt::Debug::fmt(self.as_slice(), f)
    }
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Object<'gc> {
    pub class: Gc<'gc, RefLock<Class<'gc>>>,
    /// Instance-variable storage, indexed by the class's slot layout
    /// (`Class::field_slots`). Sized at construction to the class's field count;
    /// immediate value types have no fields and never allocate an `Object`.
    pub fields: Fields<'gc>,
    pub payload: ObjectPayload<'gc>,
}

impl<'gc> Object<'gc> {
    pub fn class_name(&self) -> String {
        self.class.borrow().name.to_string()
    }
}

/// A native method definition: the fn plus its declared parameter types.
/// `param_types: None` is an untyped/legacy native method (scored as a fallback
/// ranked below any user or typed variant); `Some(types)` participates in scored
/// multimethod dispatch by argument type, exactly like a user method. Several
/// defs may share a selector — `register_native_class` chains them into a
/// multimethod, so the dispatcher routes by type.
#[derive(Clone)]
pub struct NativeMethodDef {
    pub selector: String,
    pub func: NativeFunc,
    pub param_types: Option<Vec<String>>,
}

pub trait NativeClass {
    fn parent_name(&self) -> Option<&'static str>;
    fn name(&self) -> &'static str;
    fn class_methods(&self) -> Vec<NativeMethodDef>;
    fn instance_methods(&self) -> Vec<NativeMethodDef>;
}

pub struct NativeClassBuilder {
    parent_name: Option<&'static str>,
    name: &'static str,
    class_methods: Vec<NativeMethodDef>,
    instance_methods: Vec<NativeMethodDef>,
}

type NativeFn = for<'a> fn(
    &mut VmState<'a>,
    &Mutation<'a>,
    Value<'a>,
    Vec<Value<'a>>,
) -> Result<Value<'a>, QuoinError>;

fn type_hints(param_types: &[&str]) -> Option<Vec<String>> {
    Some(param_types.iter().map(|t| t.to_string()).collect())
}

impl NativeClassBuilder {
    pub fn new(name: &'static str, parent_name: Option<&'static str>) -> Self {
        Self {
            parent_name,
            name,
            class_methods: Vec::new(),
            instance_methods: Vec::new(),
        }
    }

    pub fn class_method(mut self, selector: &str, f: NativeFn) -> Self {
        self.class_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Legacy(f),
            param_types: None,
        });
        self
    }

    /// A class-side native method with a declared type signature (scored by type).
    pub fn typed_class_method(mut self, selector: &str, param_types: &[&str], f: NativeFn) -> Self {
        self.class_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Legacy(f),
            param_types: type_hints(param_types),
        });
        self
    }

    pub fn instance_method(mut self, selector: &str, f: NativeFn) -> Self {
        self.instance_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Legacy(f),
            param_types: None,
        });
        self
    }

    /// An instance native method with a declared type signature (scored by type).
    pub fn typed_instance_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: NativeFn,
    ) -> Self {
        self.instance_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Legacy(f),
            param_types: type_hints(param_types),
        });
        self
    }

    // --- ext-sdk method registration ---------------------------------------
    // The `sdk_*` builders mirror the four above but take an `ext_sdk::SdkFn`
    // (`&mut dyn Host`) instead of a `NativeFn` (`&mut VmState`). Both coexist while
    // builtins migrate class-by-class onto the SDK surface; once all are migrated the
    // legacy `NativeFn` builders above are deleted.

    pub fn sdk_class_method(mut self, selector: &str, f: crate::ext_sdk::SdkFn) -> Self {
        self.class_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Sdk(f),
            param_types: None,
        });
        self
    }

    pub fn sdk_typed_class_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: crate::ext_sdk::SdkFn,
    ) -> Self {
        self.class_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Sdk(f),
            param_types: type_hints(param_types),
        });
        self
    }

    pub fn sdk_instance_method(mut self, selector: &str, f: crate::ext_sdk::SdkFn) -> Self {
        self.instance_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Sdk(f),
            param_types: None,
        });
        self
    }

    pub fn sdk_typed_instance_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: crate::ext_sdk::SdkFn,
    ) -> Self {
        self.instance_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func: NativeFunc::Sdk(f),
            param_types: type_hints(param_types),
        });
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

    fn class_methods(&self) -> Vec<NativeMethodDef> {
        self.class_methods.clone()
    }

    fn instance_methods(&self) -> Vec<NativeMethodDef> {
        self.instance_methods.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opaque_state_debug() {
        struct Dummy;
        let state = OpaqueState(Dummy);
        let debug_str = format!("{:?}", state);
        assert_eq!(
            debug_str,
            "OpaqueState<quoin::value::tests::test_opaque_state_debug::Dummy>"
        );
    }
}
