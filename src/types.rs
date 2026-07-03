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
        // name; a trailing `?` in a type position means nullable.
        if let Some(base) = name.strip_suffix('?') {
            return Type::Nullable(Box::new(Type::from_annotation_name(base)));
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
            _ => Type::Instance(Arc::from(name)),
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
#[derive(Clone)]
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
