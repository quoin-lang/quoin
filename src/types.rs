//! The static `Type` lattice — the shared substrate for the optimizer (devirtualization
//! today) and the resolver/checker (Phases 2–3, see docs/TYPE_SYSTEM_ARCH.md). Gradual:
//! `Any` is the top (an unannotated or un-inferable value — never devirtualized on, never
//! complained about), `Never` the bottom (a diverging expression).
//!
//! Surface syntax: builtins by name, `T?` → `Nullable(T)`, other PascalCase names →
//! `Instance`. Generics (`List(T)` / `Block(args ^Ret)`) and general unions come later.

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
