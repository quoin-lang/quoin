//! The compile-time class-signature table for the type checker (Phase 3b).
//!
//! A parallel structure to [`crate::types::SeenTypes`]: where `SeenTypes` answers "is this a known
//! class name?", `ClassTable` answers "does class C respond to selector S?" (compile-time MNU) and
//! "is C a subtype of D?" (`Instance` subtyping). Entries are `'static` (owned names) so the
//! `Compiler` can hold the table without tying itself to `'gc`.
//!
//! **Single source of truth / anti-drift.** For a class already in the VM (stdlib, prior units), a
//! [`ClassSig`] is built from `introspect::describe_class`, whose selector walk (`collect_selectors`)
//! is the *same* class→mixins→parent traversal the VM's dispatch uses
//! (`lookup_method_in_class_hierarchy_rec`). [`ClassTable::responds_to`] reproduces exactly that
//! order over the table, so the checker never forms a second opinion about resolution. A class the
//! checker isn't sure about (absent from the table, or carrying a catch-all handler) yields `None`
//! / stays silent — a missed MNU is fine, a wrong MNU is not.

use crate::types::Type;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// A class's compile-time signature: enough to resolve selectors and subtyping the way dispatch
/// does, and no more.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassSig {
    pub parent: Option<Arc<str>>,
    pub mixins: Vec<Arc<str>>,
    /// Selectors defined *directly* on this class. Inherited/mixed selectors are resolved by
    /// walking `mixins` then `parent` through the table — the exact order dispatch uses.
    pub own_selectors: HashSet<Arc<str>>,
    /// `sealed!` in the class body — its method table is frozen, so a "not found" is authoritative.
    pub sealed: bool,
    /// This class (or an ancestor) can answer *any* selector at runtime (a `doesNotUnderstand:`-style
    /// catch-all). MNU must stay silent for it. (Quoin has none today, so this is always `false`.)
    pub has_catch_all: bool,
    /// Built from the VM's live class object (`introspect::describe_class`) rather than the AST.
    /// Only VM sigs are *complete* — they include native methods and any `Foo <-- {}` extensions
    /// already applied — so **MNU and arg-checks trust only `from_vm` sigs**. Subtyping uses either
    /// (parent/mixins are structural and unaffected by method extensions).
    pub from_vm: bool,
    /// For arg-type checking: `selector → declared param types`, but only for a method that has
    /// exactly ONE variant with every parameter typed. Multi-variant (multimethod) or any untyped
    /// (`Object`) parameter is omitted, so a checkable entry unambiguously fixes the arg types.
    /// Populated only for `from_vm` sigs (a VM sig's variant set is complete).
    pub method_params: HashMap<Arc<str>, Vec<Type>>,
}

impl ClassSig {
    /// Convert `introspect`'s runtime `ClassInfo` into a checker signature. Authoritative
    /// (`from_vm = true`). Instance- and class-side selectors are pooled — being *permissive* about
    /// what a class responds to only ever *suppresses* an MNU, never invents one.
    pub fn from_class_info(info: &crate::introspect::ClassInfo) -> ClassSig {
        let own_selectors = info
            .instance_methods
            .iter()
            .chain(&info.class_methods)
            .map(|m| Arc::from(m.selector.as_str()))
            .collect();
        let mut method_params = HashMap::new();
        for m in info.instance_methods.iter().chain(&info.class_methods) {
            // A single variant with every parameter typed fixes the arg types unambiguously.
            if let [variant] = m.variants.as_slice() {
                if variant.param_types.iter().all(Option::is_some) {
                    let types = variant
                        .param_types
                        .iter()
                        .map(|p| Type::from_annotation_name(p.as_deref().unwrap()))
                        .collect();
                    method_params.insert(Arc::from(m.selector.as_str()), types);
                }
            }
        }
        ClassSig {
            parent: info.parent.as_deref().map(Arc::from),
            mixins: info.mixins.iter().map(|m| Arc::from(m.as_str())).collect(),
            own_selectors,
            sealed: info.is_sealed,
            has_catch_all: false,
            from_vm: true,
            method_params,
        }
    }
}

/// Populate the table with every class currently defined in the VM (stdlib + prior units) via
/// `introspect::describe_class` — whose selector walk *is* the VM's dispatch traversal. Idempotent
/// and cheap on repeat: a class already recorded `from_vm` is skipped. Called at each compile site
/// (where `vm` is in scope) so a unit's checker sees the fully-built classes it can dispatch to.
pub fn populate_from_vm<'gc>(vm: &crate::vm::VmState<'gc>, table: &ClassTable) {
    for g in crate::introspect::globals(vm) {
        if !matches!(g.kind, crate::introspect::GlobalKind::Class) {
            continue;
        }
        let already_authoritative = table
            .0
            .borrow()
            .get(g.name.as_str())
            .is_some_and(|s| s.from_vm);
        if already_authoritative {
            continue;
        }
        if let Some(info) = crate::introspect::describe_class(vm, &g.name) {
            table.insert(&g.name, ClassSig::from_class_info(&info));
        }
    }
}

/// A shared, mutable map `class name → ClassSig`, threaded through every `Compiler` a run spawns
/// (like `SeenTypes`) so a unit sees the classes earlier-compiled units defined.
#[derive(Clone, Default, Debug)]
pub struct ClassTable(Rc<RefCell<HashMap<Arc<str>, ClassSig>>>);

impl ClassTable {
    pub fn new() -> Self {
        ClassTable(Rc::new(RefCell::new(HashMap::new())))
    }

    pub fn insert(&self, name: &str, sig: ClassSig) {
        self.0.borrow_mut().insert(Arc::from(name), sig);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.borrow().contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<ClassSig> {
        self.0.borrow().get(name).cloned()
    }

    /// Does `class_name` respond to `selector`, walking own → mixins → parent exactly as dispatch
    /// does? Returns `None` when the answer can't be trusted — the class (or an ancestor) is absent
    /// from the table, or has a catch-all handler — so the caller stays silent rather than MNU.
    pub fn responds_to(&self, class_name: &str, selector: &str) -> Option<bool> {
        let mut visited = HashSet::new();
        self.responds_rec(class_name, selector, &mut visited)
    }

    fn responds_rec(
        &self,
        class_name: &str,
        selector: &str,
        visited: &mut HashSet<Arc<str>>,
    ) -> Option<bool> {
        if !visited.insert(Arc::from(class_name)) {
            return Some(false); // cycle guard — this branch contributes nothing
        }
        // An unknown class anywhere in the chain means we can't be sure it *doesn't* respond.
        let sig = self.get(class_name)?;
        if sig.has_catch_all {
            return None; // responds to anything → never MNU
        }
        if sig.own_selectors.contains(selector) {
            return Some(true);
        }
        // Same order dispatch uses: mixins first, then parent.
        for mixin in &sig.mixins {
            if self.responds_rec(mixin, selector, visited)? {
                return Some(true);
            }
        }
        match &sig.parent {
            Some(parent) => self.responds_rec(parent, selector, visited),
            None => Some(false),
        }
    }

    /// The declared param types of `selector` defined *directly* on `class_name` — only when it's a
    /// single, fully-typed variant (so the arg types are unambiguous). `None` otherwise. Own methods
    /// only for now (inherited methods aren't arg-checked yet).
    pub fn own_method_params(&self, class_name: &str, selector: &str) -> Option<Vec<Type>> {
        self.get(class_name)?.method_params.get(selector).cloned()
    }

    /// Is `sub` a subtype of `sup` — the same class, or a descendant via the parent/mixin chain?
    /// `None` when either class is unknown to the table (caller stays silent).
    pub fn is_subtype(&self, sub: &str, sup: &str) -> Option<bool> {
        if sub == sup {
            return Some(true);
        }
        let mut visited = HashSet::new();
        self.is_subtype_rec(sub, sup, &mut visited)
    }

    fn is_subtype_rec(
        &self,
        sub: &str,
        sup: &str,
        visited: &mut HashSet<Arc<str>>,
    ) -> Option<bool> {
        if sub == sup {
            return Some(true);
        }
        let key: Arc<str> = Arc::from(sub);
        if !visited.insert(key.clone()) {
            return Some(false);
        }
        let sig = self.get(sub)?;
        for mixin in &sig.mixins {
            if self.is_subtype_rec(mixin, sup, visited)? {
                return Some(true);
            }
        }
        match sig.parent {
            Some(parent) => self.is_subtype_rec(&parent, sup, visited),
            None => Some(false),
        }
    }
}

#[cfg(test)]
#[path = "class_table_tests.rs"]
mod class_table_tests;
