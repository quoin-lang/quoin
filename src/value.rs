use crate::error::QuoinError;
use crate::instruction::StaticBlock;
use crate::parser::ast::IdentifierNode;
use crate::runtime::list::NativeListState;
use crate::runtime::map::{NativeKeyValuePairState, NativeMapState};
use crate::runtime::regex::NativeRegexState;
use crate::runtime::set::NativeSetState;
use crate::symbol::Symbol;
use crate::vm::{ICSlot, VmState};
use std::sync::Arc;

use gc_arena::collect::Trace;
use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use rustc_hash::FxHashMap;
use std::any::Any;
use std::cell::RefCell;
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
        if s.starts_with('[')
            && let Some(close_idx) = s.find(']')
        {
            let ns_part = &s[1..close_idx];
            let name = s[close_idx + 1..].to_string();
            let path = if ns_part == "/" || ns_part.is_empty() {
                Vec::new()
            } else {
                ns_part.split('/').map(|x| x.to_string()).collect()
            };
            return Self { path, name };
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
    pub args: NativeArgs<'gc>,
}

/// How an in-flight native call's args are ROOTED (see
/// `VmState::active_native_args`). The hot `exec_send` path leaves
/// `[receiver, args..]` live on the VALUE STACK for the call's duration and
/// records only the window — no rooting clone per native call; re-entry
/// paths (`call_method` from inside a native) own their Vec as before.
/// Window indices stay valid across parks and nested calls: the stack is
/// per-task, callees only grow it above the window, and truncation back to
/// the window happens in `exec_send` after the call returns.
#[derive(Collect)]
#[collect(no_drop)]
pub enum NativeArgs<'gc> {
    Owned(Vec<Value<'gc>>),
    StackWindow { start: usize, len: usize },
}

impl<'gc> NativeCall<'gc> {
    /// The i-th argument; `stack` must be the owning VM's value stack (the
    /// window variant indexes into it). Bounds-clamped: a `^^` unwinding
    /// BELOW the window truncates it away mid-call, and the error paths that
    /// then snapshot args must see "gone", not a panic.
    pub fn arg(&self, stack: &[Value<'gc>], i: usize) -> Option<Value<'gc>> {
        match &self.args {
            NativeArgs::Owned(v) => v.get(i).copied(),
            NativeArgs::StackWindow { start, len } => {
                if i < *len {
                    stack.get(start + i).copied()
                } else {
                    None
                }
            }
        }
    }

    /// All arguments as an owned Vec (error-path snapshots only — the hot
    /// path never materializes this). Bounds-clamped like [`Self::arg`].
    pub fn args_vec(&self, stack: &[Value<'gc>]) -> Vec<Value<'gc>> {
        match &self.args {
            NativeArgs::Owned(v) => v.clone(),
            NativeArgs::StackWindow { start, len } => {
                let lo = (*start).min(stack.len());
                let hi = (start + len).min(stack.len());
                stack[lo..hi].to_vec()
            }
        }
    }
}

unsafe impl<'gc> Collect<'gc> for NativeFunc {
    const NEEDS_TRACE: bool = false;
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
/// FIXED LAYOUT (the window-arena contract, docs/internal/WINDOW_ARENA_ARCH.md §2.1):
/// `#[repr(C, u64)]` — tag qword at offset 0, payload qword at offset 8,
/// 16 bytes total (pinned by `value_layout_facts`). Compiled code reads and
/// writes slots natively against this layout; the scalar discriminants
/// deliberately COINCIDE with the helper lane kinds (`helpers::KIND_*`), so
/// a scalar lane→slot store is tag=kind, payload=bits verbatim. Object/
/// Class payloads are Gc pointers: native code only ever copies those
/// whole (16-byte slot-to-slot), never fabricates them — and the store
/// order invariant (payload before tag) is what keeps every intermediate
/// state traceable. Discriminant values are API for the JIT: do not
/// reorder or renumber without updating codegen's emission.
#[repr(C, u64)]
pub enum Value<'gc> {
    /// Immediate value types — no GC allocation. Their class is *derived* from
    /// the variant (see `get_class_for_lookup`), so "numbers are objects" still
    /// holds: they dispatch via `Integer` / `Double` / `Boolean` / `Nil` and
    /// have methods, but no per-instance fields or true eigenclass.
    Int(i64) = 0,
    Double(f64) = 1,
    Bool(bool) = 2,
    Nil = 3,
    Object(Gc<'gc, RefLock<Object<'gc>>>) = 4,
    Class(Gc<'gc, RefLock<Class<'gc>>>) = 5,
    ClassMeta(Gc<'gc, RefLock<Class<'gc>>>) = 6,
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
    /// The hot builtin collections carry DEDICATED variants — still one thin
    /// `Gc` pointer each, but the state struct lives directly in the
    /// `Gc<RefLock<…>>` node, collapsing the former Gc → Box → Vec triple hop
    /// to Gc → Vec and dropping the `Box` allocation per collection. The long
    /// tail of native types (sockets, workers, times, …) stays in
    /// `NativeState`; `new_native_state_boxed` re-routes any boxed collection
    /// state here, so a collection can never end up on the Box path.
    List(Gc<'gc, RefLock<NativeListState>>),
    Map(Gc<'gc, RefLock<NativeMapState>>),
    Set(Gc<'gc, RefLock<NativeSetState>>),
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
            match &borrowed.payload {
                ObjectPayload::List(cell) => {
                    let state = cell.borrow();
                    if let Some(concrete) = (&*state as &dyn Any).downcast_ref::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::Map(cell) => {
                    let state = cell.borrow();
                    if let Some(concrete) = (&*state as &dyn Any).downcast_ref::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::Set(cell) => {
                    let state = cell.borrow();
                    if let Some(concrete) = (&*state as &dyn Any).downcast_ref::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(concrete) = any_ref.downcast_ref::<T>() {
                        return Ok(f(concrete));
                    }
                }
                _ => {}
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
            match &borrowed.payload {
                ObjectPayload::List(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    if let Some(concrete) = (&mut *state as &mut dyn Any).downcast_mut::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::Map(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    if let Some(concrete) = (&mut *state as &mut dyn Any).downcast_mut::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::Set(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    if let Some(concrete) = (&mut *state as &mut dyn Any).downcast_mut::<T>() {
                        return Ok(f(concrete));
                    }
                }
                ObjectPayload::NativeState(state_cell) => {
                    let mut state_ref = state_cell.borrow_mut(mc);
                    let any_mut = (**state_ref).as_any_mut();
                    if let Some(concrete) = any_mut.downcast_mut::<T>() {
                        return Ok(f(concrete));
                    }
                }
                _ => {}
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
            match &borrowed.payload {
                ObjectPayload::List(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    return Some(f(&mut *state as &mut dyn Any));
                }
                ObjectPayload::Map(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    return Some(f(&mut *state as &mut dyn Any));
                }
                ObjectPayload::Set(cell) => {
                    let mut state = cell.borrow_mut(mc);
                    return Some(f(&mut *state as &mut dyn Any));
                }
                ObjectPayload::NativeState(state_cell) => {
                    let mut state_ref = state_cell.borrow_mut(mc);
                    return Some(f((**state_ref).as_any_mut()));
                }
                _ => {}
            }
        }
        None
    }
}

/// Hash a value WITHOUT dispatching guest code, or `None` if the value is a
/// user instance (whose hash is its `hash` method — dispatched by
/// `map_hash_key`, never from inside a Rust `Hash` impl).
///
/// The contract mirrors equality: equal values must hash equal. So Doubles
/// with integral values hash as their Int (native `==` coerces Int↔Double);
/// Strings/Bytes hash by content; Symbols/Blocks/Classes by their stable
/// pointer (gc-arena is non-moving); BigInteger/BigDecimal structurally
/// (their guest `==:` is structural even though native `==` is identity);
/// other native-state values by identity, matching their native `==`.
pub fn value_hash_scalar(v: &Value<'_>) -> Option<u64> {
    Some(match v {
        Value::Int(i) => hash_i64(*i),
        Value::Double(d) => {
            if d.fract() == 0.0 && *d >= i64::MIN as f64 && *d <= i64::MAX as f64 {
                hash_i64(*d as i64)
            } else {
                hash_i64(d.to_bits() as i64) ^ 0x9e37
            }
        }
        Value::Bool(b) => {
            if *b {
                0x9e3779b97f4a7c15
            } else {
                0x517cc1b727220a95
            }
        }
        Value::Nil => 0x2545f4914f6cdd1d,
        Value::Class(c) => hash_i64(Gc::as_ptr(*c) as i64),
        Value::ClassMeta(c) => hash_i64(Gc::as_ptr(*c) as i64) ^ 0x5bd1,
        Value::Object(obj) => {
            let borrowed = obj.borrow();
            match &borrowed.payload {
                ObjectPayload::String(s) => hash_bytes(s.as_bytes()),
                ObjectPayload::Bytes(b) => hash_bytes(b.as_slice()) ^ 0x1f83,
                ObjectPayload::Symbol(sym) => hash_i64(sym.as_str().as_ptr() as i64),
                ObjectPayload::Block(b) => hash_i64(Gc::as_ptr(*b) as i64),
                ObjectPayload::Instance => return None,
                // Mutable builtin collections key by IDENTITY, matching their
                // native `==` (content-hashing a mutable key is the classic
                // footgun) — same as the NativeState fallback below.
                ObjectPayload::List(_) | ObjectPayload::Map(_) | ObjectPayload::Set(_) => {
                    hash_i64(Gc::as_ptr(*obj) as i64)
                }
                ObjectPayload::NativeState(cell) => {
                    let state = cell.borrow();
                    let any = (**state).as_any();
                    if let Some(bi) =
                        any.downcast_ref::<crate::runtime::big_integer::NativeBigInteger>()
                    {
                        match num_traits::ToPrimitive::to_i64(&bi.0) {
                            Some(i) => hash_i64(i),
                            None => hash_bytes(bi.0.to_string().as_bytes()),
                        }
                    } else if let Some(bd) =
                        any.downcast_ref::<crate::runtime::big_decimal::NativeBigDecimal>()
                    {
                        if bd.0.fract().is_zero() {
                            match num_traits::ToPrimitive::to_i64(&bd.0) {
                                Some(i) => hash_i64(i),
                                None => hash_bytes(bd.0.normalize().to_string().as_bytes()),
                            }
                        } else {
                            hash_bytes(bd.0.normalize().to_string().as_bytes())
                        }
                    } else {
                        // Other native-state values (channels, workers, arrays,
                        // …) key by IDENTITY, matching their native `==`.
                        hash_i64(Gc::as_ptr(*obj) as i64)
                    }
                }
            }
        }
    })
}

/// True when native `==` is AUTHORITATIVE for this value (scalars and
/// content-compared payloads): two such values that are not natively equal
/// are definitively unequal — no guest `==:` dispatch needed.
pub fn key_native_exact(v: &Value<'_>) -> bool {
    match v {
        Value::Int(_)
        | Value::Double(_)
        | Value::Bool(_)
        | Value::Nil
        | Value::Class(_)
        | Value::ClassMeta(_) => true,
        Value::Object(obj) => matches!(
            &obj.borrow().payload,
            ObjectPayload::String(_)
                | ObjectPayload::Bytes(_)
                | ObjectPayload::Symbol(_)
                | ObjectPayload::Block(_)
        ),
    }
}

/// FxHash over bytes — the single content-hash used by both String VALUES
/// and native `&str` lookups, so they can never disagree.
pub fn hash_bytes(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    b.hash(&mut h);
    h.finish()
}

pub fn hash_i64(i: i64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    i.hash(&mut h);
    h.finish()
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
                    ObjectPayload::Block(b) => write!(f, "Block({:?})", b.template.name),
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
                            let mut parts = Vec::new();
                            for (_, k, v) in m.entries().iter() {
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
                            let vec = s.values();
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
                        if let Some(ref name) = b.template.name {
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
    /// The immutable compile-time half (name, params, bytecode, source map),
    /// shared by every closure materialized from the same block literal — a
    /// closure creation is an `Rc` bump plus the captured runtime state below,
    /// not a deep clone of the param vectors.
    #[collect(require_static)]
    pub template: Arc<StaticBlock>,
    pub parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    pub enclosing_method_id: Option<usize>,
    pub decl_block: Option<Gc<'gc, Block<'gc>>>,
    /// Per-call-site monomorphic inline cache, indexed by `ip` (one slot per
    /// instruction), allocated lazily on the first cacheable send. When the
    /// template has an id this cell is shared via `VmState::ic_registry`, so call
    /// sites stay warm across re-materialization of the same literal; the registry
    /// roots it for the VM's lifetime and ids are never reused, so
    /// `(template_id, ip)` is a stable call-site identity (no ABA). Id-less
    /// (runtime-built) blocks get a private cell.
    pub inline_cache: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
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

/// The memoized per-class instantiation recipe (field-fill + init chain),
/// built once per (class, dispatch_epoch) by `VmState::instantiation_plan`
/// and cached on the class — `new:`/`new` used to re-derive all of it per
/// instantiation (two hierarchy walks, a Vec per walk, a String clone per
/// ivar name, and an `init:` param-name Vec per init). IMMUTABLE once built:
/// users iterate a copied `Gc` (rooted via `VmState::active_init_plans`
/// across the user init calls, which can park — a stale plan replaced
/// mid-chain must stay alive for its running iteration).
#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct InitPlan<'gc> {
    /// Deduped ivar names paired with their resolved field slots, in the
    /// same self-then-mixins-then-parent order the per-call walk produced.
    #[collect(require_static)]
    pub ivar_slots: Vec<(String, usize)>,
    /// The base->derived init chain, one entry per class that defines any
    /// initializer. `finalize_instantiation` (the `new:{}` path) runs
    /// `init_colon` when present, else `init_plain`; `run_all_inits` (the
    /// plain `new` path) runs `init_plain` ONLY — mirroring the two
    /// pre-plan walks exactly.
    pub inits: Vec<InitEntry<'gc>>,
}

/// One class's resolved initializers in an [`InitPlan`].
#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct InitEntry<'gc> {
    /// `init:` plus the param names it is fed from the `new:{}` block.
    pub init_colon: Option<(Value<'gc>, Vec<String>)>,
    pub init_plain: Option<Value<'gc>>,
}

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct Class<'gc> {
    pub name: NamespacedName,
    pub parent: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub instance_vars: Vec<String>,
    /// Method tables keyed by interned selector: hot lookups already hold a
    /// `Symbol`, so probing is a pointer-hash, never a byte-hash of the name.
    pub instance_methods: FxHashMap<Symbol, Value<'gc>>,
    pub class_methods: FxHashMap<Symbol, Value<'gc>>,
    pub mixin_classes: Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    /// Memoized, append-only instance-variable layout: name -> absolute slot in an
    /// instance's `fields` array. Built lazily from the full hierarchy (own +
    /// mixins + parent) at first instantiation; new ivars only ever append, so
    /// existing slots stay stable across runtime mixins. `len()` is the field count.
    pub field_slots: FxHashMap<String, usize>,
    /// Memoized instantiation recipe (see [`InitPlan`]): valid only while
    /// the stamped `dispatch_epoch` matches — any method/mixin/hierarchy
    /// mutation bumps the epoch and the next instantiation rebuilds.
    pub init_plan: Option<(u64, Gc<'gc, InitPlan<'gc>>)>,
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
    /// For builder-registered native classes: refuse the generic `new` / `new:{}`
    /// instantiation fallback with this constructor hint. The fallback mints a plain
    /// object with no `NativeState` payload — a poison shell whose first native
    /// method fails with an internal error — so a native class is constructible only
    /// through its own class-side constructors. `None` for user-defined classes (and
    /// the natives whose explicit class-side `new` wins lookup before the fallback).
    pub native_new_refusal: Option<&'static str>,
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
    /// Declared checker return type (Fork-1b native half), e.g. `Some("String")`. A pure
    /// compile-time annotation — the VM never reads it; it flows to `ClassSig.method_returns`
    /// via `describe_class`. Set opt-in through the `.returns(..)` builder modifier.
    pub ret_type: Option<String>,
    /// Reference-doc text (docs/internal/DOCS_ARCH.md §5): first line is the summary, the rest the
    /// body. Never consulted at dispatch; surfaced via `describe_class` and `qn doc`. Set
    /// through the `.doc(..)` builder modifier — the native counterpart of the `"*` block a
    /// Quoin method carries in its source.
    pub doc: Option<String>,
}

/// Which method table a builder call last appended to — so `.returns(..)` knows whose
/// most-recently-registered method to annotate.
#[derive(Clone, Copy)]
enum LastSide {
    Instance,
    Class,
}

pub trait NativeClass {
    fn parent_name(&self) -> Option<&'static str>;
    fn name(&self) -> &'static str;
    /// Class-level reference doc (`.class_doc(..)` on the builder). Recorded in
    /// `VmState::class_meta` at registration.
    fn class_doc(&self) -> Option<&str> {
        None
    }
    fn class_methods(&self) -> Vec<NativeMethodDef>;
    fn instance_methods(&self) -> Vec<NativeMethodDef>;
    fn new_policy(&self) -> NativeNewPolicy {
        NativeNewPolicy::Refuse(None)
    }
}

/// How the generic `new`/`new:{}` instantiation fallback treats a native class.
/// The default is `Refuse(None)`: the fallback would mint a payload-less shell, so
/// a native class must declare how it is constructed — a namespace-style class
/// marks itself [`Abstract`](NativeNewPolicy::Abstract); anything with real
/// instances names its constructors in a [`Refuse`](NativeNewPolicy::Refuse) hint.
/// (A class whose explicit class-side `new` wins lookup — `List`, `Map`, `Set`,
/// `Bytes`, `Channel` — never reaches the fallback for bare `new`; its policy
/// still governs `new:{}`.)
#[derive(Clone, Copy)]
pub enum NativeNewPolicy {
    /// Mark the class `abstract!`: a namespace-style class (`Math`, `JSON`, …)
    /// whose instances are meaningless.
    Abstract,
    /// Refuse `new`/`new:` with a constructor hint, e.g.
    /// `"use UUID.generateV4 / UUID.parse:"`; `None` uses a generic message.
    Refuse(Option<&'static str>),
}

pub struct NativeClassBuilder {
    parent_name: Option<&'static str>,
    name: &'static str,
    class_methods: Vec<NativeMethodDef>,
    instance_methods: Vec<NativeMethodDef>,
    last_side: Option<LastSide>,
    new_policy: NativeNewPolicy,
    class_doc: Option<String>,
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
            last_side: None,
            new_policy: NativeNewPolicy::Refuse(None),
            class_doc: None,
        }
    }

    /// Mark this class `abstract!`: a namespace-style class (`Math`, `JSON`, …)
    /// whose instances are meaningless — `new`/`new:` raise the abstract-class error.
    pub fn abstract_class(mut self) -> Self {
        self.new_policy = NativeNewPolicy::Abstract;
        self
    }

    /// Name the class's real constructors in the `new`/`new:` refusal message,
    /// e.g. `"use UUID.generateV4 / UUID.parse:"`.
    pub fn construct_with(mut self, hint: &'static str) -> Self {
        self.new_policy = NativeNewPolicy::Refuse(Some(hint));
        self
    }

    /// Append a class-side method def and remember the side for a following `.returns(..)`.
    fn add_class(&mut self, selector: &str, func: NativeFunc, param_types: Option<Vec<String>>) {
        self.class_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func,
            param_types,
            ret_type: None,
            doc: None,
        });
        self.last_side = Some(LastSide::Class);
    }

    /// Append an instance-side method def and remember the side for a following `.returns(..)`.
    fn add_instance(&mut self, selector: &str, func: NativeFunc, param_types: Option<Vec<String>>) {
        self.instance_methods.push(NativeMethodDef {
            selector: selector.to_string(),
            func,
            param_types,
            ret_type: None,
            doc: None,
        });
        self.last_side = Some(LastSide::Instance);
    }

    /// Declare the checker return type of the most-recently-registered method (the native half of
    /// Fork-1b). A pure compile-time annotation — the VM ignores it; it flows to
    /// `ClassSig.method_returns` via `describe_class`. Composes with any builder, typed or not
    /// (a return is orthogonal to arg-typing: `Object#s` has untyped args but a `String` return).
    /// No-op if no method was registered yet.
    pub fn returns(mut self, ret_type: &str) -> Self {
        let last = match self.last_side {
            Some(LastSide::Instance) => self.instance_methods.last_mut(),
            Some(LastSide::Class) => self.class_methods.last_mut(),
            None => None,
        };
        if let Some(def) = last {
            def.ret_type = Some(ret_type.to_string());
        }
        self
    }

    /// Attach reference-doc text to the most-recently-registered method
    /// (docs/internal/DOCS_ARCH.md §5) — the native counterpart of the `"*` block above a Quoin
    /// method. First line is the summary; the rest is the body. Composes with `.returns(..)`
    /// in either order; no-op if no method was registered yet.
    pub fn doc(mut self, text: &str) -> Self {
        let last = match self.last_side {
            Some(LastSide::Instance) => self.instance_methods.last_mut(),
            Some(LastSide::Class) => self.class_methods.last_mut(),
            None => None,
        };
        if let Some(def) = last {
            def.doc = Some(text.to_string());
        }
        self
    }

    /// Attach reference-doc text to the class itself. Recorded in `VmState::class_meta` at
    /// registration; Quoin classes get theirs from the `"*` block above the definition.
    pub fn class_doc(mut self, text: &str) -> Self {
        self.class_doc = Some(text.to_string());
        self
    }

    pub fn class_method(mut self, selector: &str, f: NativeFn) -> Self {
        self.add_class(selector, NativeFunc::Legacy(f), None);
        self
    }

    /// A class-side native method with a declared type signature (scored by type).
    pub fn typed_class_method(mut self, selector: &str, param_types: &[&str], f: NativeFn) -> Self {
        self.add_class(selector, NativeFunc::Legacy(f), type_hints(param_types));
        self
    }

    pub fn instance_method(mut self, selector: &str, f: NativeFn) -> Self {
        self.add_instance(selector, NativeFunc::Legacy(f), None);
        self
    }

    /// An instance native method with a declared type signature (scored by type).
    pub fn typed_instance_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: NativeFn,
    ) -> Self {
        self.add_instance(selector, NativeFunc::Legacy(f), type_hints(param_types));
        self
    }

    // --- ext-sdk method registration ---------------------------------------
    // The `sdk_*` builders mirror the four above but take an `ext_sdk::SdkFn`
    // (`&mut dyn Host`) instead of a `NativeFn` (`&mut VmState`). Both coexist while
    // builtins migrate class-by-class onto the SDK surface; once all are migrated the
    // legacy `NativeFn` builders above are deleted.

    pub fn sdk_class_method(mut self, selector: &str, f: crate::ext_sdk::SdkFn) -> Self {
        self.add_class(selector, NativeFunc::Sdk(f), None);
        self
    }

    pub fn sdk_typed_class_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: crate::ext_sdk::SdkFn,
    ) -> Self {
        self.add_class(selector, NativeFunc::Sdk(f), type_hints(param_types));
        self
    }

    pub fn sdk_instance_method(mut self, selector: &str, f: crate::ext_sdk::SdkFn) -> Self {
        self.add_instance(selector, NativeFunc::Sdk(f), None);
        self
    }

    pub fn sdk_typed_instance_method(
        mut self,
        selector: &str,
        param_types: &[&str],
        f: crate::ext_sdk::SdkFn,
    ) -> Self {
        self.add_instance(selector, NativeFunc::Sdk(f), type_hints(param_types));
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

    fn class_doc(&self) -> Option<&str> {
        self.class_doc.as_deref()
    }

    fn class_methods(&self) -> Vec<NativeMethodDef> {
        self.class_methods.clone()
    }

    fn instance_methods(&self) -> Vec<NativeMethodDef> {
        self.instance_methods.clone()
    }

    fn new_policy(&self) -> NativeNewPolicy {
        self.new_policy
    }
}
/// The VM's slot stack (docs/internal/WINDOW_ARENA_ARCH.md §2.2): a Vec with a
/// `#[repr(C)]` HEAD at a stable address — compiled code reads `(ptr, len)`
/// through the head (passed via the raw ABI beside fuel/depth/epoch) and
/// does native bounds-checked slot loads/stores against `Value`'s fixed
/// layout. Growth stays in Rust: every mutation routes through methods
/// that re-sync the head, so Vec reallocation remains legal — native code
/// re-reads the head per access and never pushes.
#[repr(C)]
pub struct SlotHead {
    /// Read by compiled code at offset 0.
    pub ptr: *mut u8,
    /// Read by compiled code at offset 8 (in Values, not bytes).
    pub len: usize,
}

pub struct SlotStack<'gc> {
    head: SlotHead,
    vec: Vec<Value<'gc>>,
}

impl<'gc> SlotStack<'gc> {
    pub fn new() -> Self {
        let mut s = SlotStack {
            head: SlotHead {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
            vec: Vec::new(),
        };
        s.sync_head();
        s
    }

    /// LAZY head discipline (the A2 lesson: syncing on every push/truncate
    /// sat on the interpreter's two hottest operations and gave back the
    /// whole A1 win — +6-8% measured). Only compiled code reads the head,
    /// so it is refreshed explicitly at the compiled-call boundary
    /// (`invoke`/`invoke_block` before the raw call) and at the exit of any
    /// helper that can GROW the stack while native code holds slot
    /// addresses derived from it. Interpreter mutations are plain Vec ops.
    #[inline(always)]
    pub fn sync_head(&mut self) {
        self.head.ptr = self.vec.as_mut_ptr() as *mut u8;
        self.head.len = self.vec.len();
    }

    /// The stable head address for the raw ABI.
    pub fn head_addr(&mut self) -> *mut SlotHead {
        &raw mut self.head
    }

    /// The A5 canary: every extern helper that can grow the stack must
    /// sync before returning into native code (docs/internal/WINDOW_ARENA_ARCH.md
    /// §5). `slot_write` asserts this in debug builds on every compiled
    /// slot write, so a missed exit-sync fails the corpus loudly instead
    /// of reading a reallocated-away buffer.
    #[cfg(debug_assertions)]
    pub fn head_is_fresh(&self) -> bool {
        self.head.ptr as *const Value<'gc> == self.vec.as_ptr() && self.head.len == self.vec.len()
    }

    /// Task-switch boundary (the scheduler parks a task's stack as a plain
    /// Vec): O(1) moves either way, head re-synced on entry.
    pub fn from_vec(vec: Vec<Value<'gc>>) -> Self {
        let mut s = SlotStack {
            head: SlotHead {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
            vec,
        };
        s.sync_head();
        s
    }

    pub fn into_vec(self) -> Vec<Value<'gc>> {
        self.vec
    }

    #[inline(always)]
    pub fn push(&mut self, v: Value<'gc>) {
        self.vec.push(v);
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Option<Value<'gc>> {
        self.vec.pop()
    }

    #[inline(always)]
    pub fn truncate(&mut self, n: usize) {
        self.vec.truncate(n);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.vec.clear();
    }
}

impl Default for SlotStack<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> std::ops::Deref for SlotStack<'gc> {
    type Target = [Value<'gc>];
    #[inline(always)]
    fn deref(&self) -> &[Value<'gc>] {
        &self.vec
    }
}

impl<'gc> std::ops::DerefMut for SlotStack<'gc> {
    // In-place writes through the slice never move or resize the storage,
    // so the head stays valid.
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [Value<'gc>] {
        &mut self.vec
    }
}

// The head is plain data (no GC content); the slots trace like the Vec did.
unsafe impl<'gc> gc_arena::Collect<'gc> for SlotStack<'gc> {
    const NEEDS_TRACE: bool = true;
    fn trace<T: gc_arena::collect::Trace<'gc>>(&self, cc: &mut T) {
        self.vec.trace(cc);
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
