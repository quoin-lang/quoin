//! The static `Type` lattice — the shared substrate for the optimizer (devirtualization
//! today) and the resolver/checker (Phases 2–3, see docs/TYPE_SYSTEM_ARCH.md). Gradual:
//! `Any` is the top (an unannotated or un-inferable value — never devirtualized on, never
//! complained about), `Never` the bottom (a diverging expression).
//!
//! Surface syntax: builtins by name, `T?` → `Nullable(T)`, other PascalCase names →
//! `Instance`. Generics (`List(T)` / `Block(args ^Ret)`) and general unions come later.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    // Scalar builtins.
    Int,
    Double,
    Bool,
    String,
    Nil,
    // Collection / callable builtins (generics come later: `List(T)`, `Block(args ^Ret)`).
    List,
    Map,
    Set,
    Block,
    /// An instance of a user-defined class, identified by name.
    Instance(Arc<str>),
    /// `T?` — `T` or `nil`.
    Nullable(Box<Type>),
    /// Gradual top: an unannotated or un-inferable value. The optimizer never devirtualizes
    /// on `Any`; the checker never complains about it.
    Any,
    /// Bottom: an expression that never yields a value (diverges). For Phase 3 control-flow.
    Never,
}

impl Type {
    /// Resolve a type-annotation identifier to a `Type`. Best-effort (Phase 1): builtins map
    /// by name, a trailing `?` yields `Nullable`, and any other name is taken as a user-class
    /// `Instance`. Diagnostics and validation against real classes are Phase 2.
    pub fn from_annotation_name(name: &str) -> Type {
        // The settled `Integer?` rule: `?` is an identifier char, so it arrives glued to the
        // name; a trailing `?` in a type position means nullable. Nullable of the top type is
        // still the top type (`Object?` ⇒ `Any`, not `Nullable(Any)`).
        if let Some(base) = name.strip_suffix('?') {
            let inner = Type::from_annotation_name(base);
            return if matches!(inner, Type::Any) {
                Type::Any
            } else {
                Type::Nullable(Box::new(inner))
            };
        }
        match name {
            "Integer" => Type::Int,
            "Double" => Type::Double,
            "Boolean" => Type::Bool,
            "String" => Type::String,
            "Nil" => Type::Nil,
            "List" => Type::List,
            "Map" => Type::Map,
            "Set" => Type::Set,
            "Block" => Type::Block,
            // `Object` is the universal supertype — as a static annotation it constrains nothing,
            // so it resolves to the gradual top `Any` (never a concrete `Instance("Object")`, which
            // would false-positive `expected Object, found …`). The `Object` *string* is still the
            // runtime dispatch default; that path doesn't come through here.
            "Object" => Type::Any,
            _ => Type::Instance(Arc::from(name)),
        }
    }

    /// Is a value of `self` usable where `expected` is wanted (the subtype direction)? Strict —
    /// signatures never auto-widen (`Int` is NOT compatible with `Double`; numeric *literals* are
    /// promoted at the value level by the checker instead). Gradual: `Any` on either side always
    /// fits, so untyped code is never flagged. `Instance` matches by name only — subtyping arrives
    /// with the class table (Phase 3b).
    pub fn compatible_with(&self, expected: &Type) -> bool {
        match (self, expected) {
            (Type::Any, _) | (_, Type::Any) => true,
            // Bottom: a diverging expression satisfies any expected type.
            (Type::Never, _) => true,
            // `T?` expected: `nil` fits, else the actual (unwrapped) must fit the inner type.
            (_, Type::Nullable(inner)) => match self {
                Type::Nil => true,
                Type::Nullable(a) => a.compatible_with(inner),
                other => other.compatible_with(inner),
            },
            // A nullable actual can't satisfy a non-nullable expected (it could be `nil`).
            (Type::Nullable(_), _) => false,
            (a, b) => a == b,
        }
    }

    /// Least upper bound on the nil lattice — the type of a value that is `self` on one control-flow
    /// path and `other` on another (a narrowing *join*, Phase 3c). `T ⊔ T = T`, `T ⊔ Nil = T?`,
    /// `T ⊔ T? = T?`; two different concrete cores widen to `Any` (we have no general unions yet —
    /// this *is* the union constructor, kept nil-scoped). `Any` is absorbing (unknown on either path
    /// ⇒ unknown); `Never` (a diverging path) contributes nothing.
    pub fn join(&self, other: &Type) -> Type {
        match (self, other) {
            (Type::Any, _) | (_, Type::Any) => return Type::Any,
            (Type::Never, t) | (t, Type::Never) => return t.clone(),
            _ => {}
        }
        if self == other {
            return self.clone();
        }
        // Split each side into (non-nil core, may-be-nil?).
        fn split(t: &Type) -> (Option<&Type>, bool) {
            match t {
                Type::Nil => (None, true),
                Type::Nullable(inner) => (Some(inner), true),
                other => (Some(other), false),
            }
        }
        let (a, a_nil) = split(self);
        let (b, b_nil) = split(other);
        let nullable = a_nil || b_nil;
        let core = match (a, b) {
            (None, None) => None,
            (Some(t), None) | (None, Some(t)) => Some(t.clone()),
            (Some(x), Some(y)) if x == y => Some(x.clone()),
            _ => return Type::Any, // two different concrete cores — no union available
        };
        match core {
            None => Type::Nil,
            Some(t) if nullable => Type::Nullable(Box::new(t)),
            Some(t) => t,
        }
    }

    /// The Quoin-facing class name, for diagnostics (`Integer`, `Boolean`, `Foo?`, `Any`, …).
    pub fn name(&self) -> String {
        match self {
            Type::Int => "Integer".to_string(),
            Type::Double => "Double".to_string(),
            Type::Bool => "Boolean".to_string(),
            Type::String => "String".to_string(),
            Type::Nil => "Nil".to_string(),
            Type::List => "List".to_string(),
            Type::Map => "Map".to_string(),
            Type::Set => "Set".to_string(),
            Type::Block => "Block".to_string(),
            Type::Instance(n) => n.to_string(),
            Type::Nullable(inner) => format!("{}?", inner.name()),
            Type::Any => "Any".to_string(),
            Type::Never => "Never".to_string(),
        }
    }
}

/// Every built-in class *name* — a superset of the enum's dedicated variants (the extras
/// resolve to `Instance` but are still "known", so annotating with them is not flagged).
pub const BUILTIN_CLASS_NAMES: &[&str] = &[
    "Integer", "Double", "Boolean", "String", "Nil", "List", "Map", "Set", "Block", "Object",
    "Symbol", "Range", "Regex", "Bytes", "Method", "Class",
];

/// The set of class names known so far during compilation: the builtins plus every class a
/// unit compiled up to this point has defined. Shared (via `Rc`) across every `Compiler` the
/// runner and VM spawn, so a later unit "sees" the classes earlier units defined — the basis
/// for the resolver's `unknown type Foo` diagnostic (docs/TYPE_SYSTEM_ARCH.md Phase 2). The VM
/// is single-threaded (gc_arena), so `Rc<RefCell<…>>` is sufficient.
#[derive(Clone, Debug)]
pub struct SeenTypes(Rc<RefCell<HashSet<Arc<str>>>>);

impl SeenTypes {
    /// A fresh set seeded with the builtin class names.
    pub fn with_builtins() -> Self {
        let set: HashSet<Arc<str>> = BUILTIN_CLASS_NAMES.iter().map(|s| Arc::from(*s)).collect();
        SeenTypes(Rc::new(RefCell::new(set)))
    }

    /// Record a class name as known (idempotent).
    pub fn insert(&self, name: &str) {
        self.0.borrow_mut().insert(Arc::from(name));
    }

    /// Is this a known type name (a builtin, or a class seen so far)?
    pub fn contains(&self, name: &str) -> bool {
        self.0.borrow().contains(name)
    }
}

impl Default for SeenTypes {
    fn default() -> Self {
        Self::with_builtins()
    }
}
