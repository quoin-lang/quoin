//! Element tags: the runtime half of checked generic collections
//! (docs/GENERICS_ARCH.md §6). A tagged collection checks every insertion
//! against its tag, so whatever comes out is proven tag-or-nil — the third
//! guarantee source after dispatch-guaranteed params and `sealed!`.
//!
//! Untagged collections (`elem: None`) are the entire pre-existing world:
//! they pay one perfectly-predicted branch per write and nothing else.

use crate::symbol::Symbol;
use crate::types::Type;
use crate::value::{ObjectPayload, Value};

/// A collection's element type. Flat, non-generic types only (v1): anything
/// unenforceable degrades to its base with a compile-time warning — a tag is
/// recorded only when the runtime will actually enforce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElemTag {
    Int,
    Double,
    Bool,
    Str,
    List,
    Map,
    Set,
    /// A user class, by interned name; matched with the same parent/mixin
    /// walk dispatch uses (`List(Shape)` accepts `Circle`s).
    Class(Symbol),
}

impl ElemTag {
    /// The Quoin-facing name, for error messages and `elementType` symbols.
    pub fn name(&self) -> &str {
        match self {
            ElemTag::Int => "Integer",
            ElemTag::Double => "Double",
            ElemTag::Bool => "Boolean",
            ElemTag::Str => "String",
            ElemTag::List => "List",
            ElemTag::Map => "Map",
            ElemTag::Set => "Set",
            ElemTag::Class(s) => s.as_str(),
        }
    }

    /// The tag a checker `Type` mints, if it is runtime-enforceable
    /// (guarantee honesty: nested generics, variables, `Any` → `None`, never
    /// a false guarantee). `T?` tags as `T` — nil always passes anyway.
    pub fn from_type(t: &Type) -> Option<ElemTag> {
        match t {
            Type::Int => Some(ElemTag::Int),
            Type::Double => Some(ElemTag::Double),
            Type::Bool => Some(ElemTag::Bool),
            Type::String => Some(ElemTag::Str),
            Type::List => Some(ElemTag::List),
            Type::Map => Some(ElemTag::Map),
            Type::Set => Some(ElemTag::Set),
            Type::Instance(n) => Some(ElemTag::Class(Symbol::intern(n))),
            Type::Nullable(inner) => ElemTag::from_type(inner),
            Type::ListOf(_)
            | Type::MapOf(_)
            | Type::SetOf(_)
            | Type::BlockOf { .. }
            | Type::Var(_)
            | Type::Nil
            | Type::Block
            | Type::Any
            | Type::Never => None,
        }
    }

    /// The tag for an element-class *value* (`List.of:Integer`, `ensure:`).
    /// `None` for a non-Class value or a class no tag can enforce.
    pub fn from_class_value(v: &Value) -> Option<ElemTag> {
        let Value::Class(c) = v else { return None };
        // Namespaced class names render like dispatch hints do, so the
        // class-walk (`value_matches_type`) resolves them identically.
        let name = c.borrow().name.to_string();
        Some(match name.as_str() {
            "Integer" => ElemTag::Int,
            "Double" => ElemTag::Double,
            "Boolean" => ElemTag::Bool,
            "String" => ElemTag::Str,
            "List" => ElemTag::List,
            "Map" => ElemTag::Map,
            "Set" => ElemTag::Set,
            _ => ElemTag::Class(Symbol::intern(&name)),
        })
    }

    /// Does `v` satisfy this tag, where that's decidable without the VM?
    /// `Some(ok)` for nil (always passes), scalars, strings, and the native
    /// collections; `None` for a `Class` tag on a value that needs the
    /// dispatch class-walk (the caller escalates to `VmState`).
    pub fn matches_value(&self, v: &Value<'_>) -> Option<bool> {
        if matches!(v, Value::Nil) {
            return Some(true);
        }
        match self {
            ElemTag::Int => Some(matches!(v, Value::Int(_))),
            ElemTag::Double => Some(matches!(v, Value::Double(_))),
            ElemTag::Bool => Some(matches!(v, Value::Bool(_))),
            ElemTag::Str => Some(match v {
                Value::Object(o) => matches!(o.borrow().payload, ObjectPayload::String(_)),
                _ => false,
            }),
            ElemTag::List => Some(is_native_state::<crate::runtime::list::NativeListState>(v)),
            ElemTag::Map => Some(is_native_state::<crate::runtime::map::NativeMapState>(v)),
            ElemTag::Set => Some(is_native_state::<crate::runtime::set::NativeSetState>(v)),
            ElemTag::Class(_) => None,
        }
    }
}

impl ElemTag {
    /// Compact code for embedding in compiled code (AOT `TagCollection`).
    /// `Class` tags have no code — the translator refuses those literals.
    pub fn code(self) -> Option<i64> {
        Some(match self {
            ElemTag::Int => 0,
            ElemTag::Double => 1,
            ElemTag::Bool => 2,
            ElemTag::Str => 3,
            ElemTag::List => 4,
            ElemTag::Map => 5,
            ElemTag::Set => 6,
            ElemTag::Class(_) => return None,
        })
    }

    pub fn from_code(c: i64) -> Option<ElemTag> {
        Some(match c {
            0 => ElemTag::Int,
            1 => ElemTag::Double,
            2 => ElemTag::Bool,
            3 => ElemTag::Str,
            4 => ElemTag::List,
            5 => ElemTag::Map,
            6 => ElemTag::Set,
            _ => return None,
        })
    }
}

/// Single-borrow insertion gate for the hot devirtualized write paths: the
/// caller's closure consults it while already holding the state borrow, writes
/// on `Pass`, and escalates `NeedWalk` (a `Class` tag) to the VM's class walk
/// outside the borrow. Untagged stays one `None` test.
pub enum TagGate {
    Pass,
    Fail(ElemTag),
    NeedWalk(ElemTag),
}

pub fn gate(tag: Option<ElemTag>, v: &Value<'_>) -> TagGate {
    match tag {
        None => TagGate::Pass,
        Some(t) => match t.matches_value(v) {
            Some(true) => TagGate::Pass,
            Some(false) => TagGate::Fail(t),
            None => TagGate::NeedWalk(t),
        },
    }
}

/// The one insertion check. `class_walk` is the dispatch parent/mixin walk
/// (`vm.value_matches_type` / `host.value_matches_type`), consulted only for
/// `Class` tags — everything else is decided by `Value` variant.
pub fn check_insert<'gc>(
    tag: Option<ElemTag>,
    container: &str,
    v: &Value<'gc>,
    index: Option<i64>,
    class_walk: impl FnOnce(&Value<'gc>, &str) -> bool,
) -> Result<(), crate::error::QuoinError> {
    let Some(t) = tag else { return Ok(()) };
    let ok = match t.matches_value(v) {
        Some(ok) => ok,
        None => {
            let ElemTag::Class(sym) = t else {
                unreachable!("only Class tags defer to the walk")
            };
            class_walk(v, sym.as_str())
        }
    };
    if ok {
        Ok(())
    } else {
        Err(elem_type_error(container, t, v, index))
    }
}

/// The value's own element tag, if it is a tagged native collection.
pub fn value_elem_tag(v: &Value<'_>) -> Option<ElemTag> {
    if let Ok(t) = v.with_native_state::<crate::runtime::list::NativeListState, _, _>(|l| l.elem) {
        return t;
    }
    if let Ok(t) = v.with_native_state::<crate::runtime::map::NativeMapState, _, _>(|m| m.elem) {
        return t;
    }
    if let Ok(t) = v.with_native_state::<crate::runtime::set::NativeSetState, _, _>(|s| s.elem) {
        return t;
    }
    None
}

fn is_native_state<S: 'static>(v: &Value<'_>) -> bool {
    v.with_native_state::<S, _, _>(|_| ()).is_ok()
}

/// The house-style insertion `TypeError` (the `Array.ofInts:` precedent):
/// `expected` = the tag, `got` = the value's type name, and the message names
/// the container shape and, where meaningful, the index.
pub fn elem_type_error(
    container: &str,
    tag: ElemTag,
    got: &Value<'_>,
    index: Option<i64>,
) -> crate::error::QuoinError {
    let got_name = got.type_name().to_string();
    let at = match index {
        Some(i) => format!(" at {i}"),
        None => String::new(),
    };
    crate::error::QuoinError::TypeError {
        expected: tag.name().to_string(),
        got: got_name.clone(),
        msg: format!(
            "{container}({tag}): element{at} must be {tag}, got {got_name}",
            tag = tag.name(),
        ),
    }
}
