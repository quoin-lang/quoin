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
    /// Declared return types: `selector → Type`, from a method's `^Ret` header. Recorded from the
    /// AST — both `Foo <- {}` definitions and `Foo <-- {}` reopens (how the core classes add
    /// methods). `insert` *merges* these so a later `from_vm` overwrite (VM sigs carry no returns
    /// today — see Fork-1b) doesn't drop them. Drives the return-covariance check and Object-rooted
    /// send typing (Phase 3c·4).
    pub method_returns: HashMap<Arc<str>, Type>,
    /// Declared class/mixin-header type parameters (`Iterate(T U)`), in order.
    /// The first parameter is the one a tagged-collection receiver binds
    /// (GENERICS_ARCH.md §4.4). AST-recorded; the runtime Class carries none,
    /// so `insert` merges these across a later `from_vm` overwrite. The
    /// builtin collections are seeded (`List(T)`/`Set(T)`/`Map(V)`) — they ARE
    /// the runtime-backed generic classes.
    pub type_params: Vec<Arc<str>>,
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
        let mut method_returns = HashMap::new();
        // The generic builtin collections' type parameters (GENERICS_ARCH.md
        // §3.3): List(T)/Set(T) elements, Map(V) values (keys pinned String).
        // Their native `.returns("T?")`-style declarations parse against these.
        let type_params: Vec<Arc<str>> = match info.name.as_str() {
            "List" | "Set" => vec![Arc::from("T")],
            "Map" => vec![Arc::from("V")],
            _ => Vec::new(),
        };
        // A native method whose CHECKER signature can't ride its dispatch
        // hints: `Block(T ^Any)` as a hint string would make the method
        // dispatch-unreachable (G0's erasure lesson), so the checker-only
        // shape is seeded here, like the collections' `type_params` above.
        // Set's `each:` is the one native the Iterate mixin rides on
        // (List/Map/Range define theirs in qnlib — GENERICS_ARCH.md §11.4).
        if info.name == "Set" {
            method_params.insert(
                Arc::from("each:"),
                vec![Type::parse_annotation_str("Block(T ^Any)", &type_params)],
            );
        }
        for m in info.instance_methods.iter().chain(&info.class_methods) {
            // A single variant fixes the arg types (all typed) and/or the return unambiguously.
            if let [variant] = m.variants.as_slice() {
                if variant.param_types.iter().all(Option::is_some) {
                    let types = variant
                        .param_types
                        .iter()
                        .map(|p| Type::parse_annotation_str(p.as_deref().unwrap(), &type_params))
                        .collect();
                    method_params.insert(Arc::from(m.selector.as_str()), types);
                }
                // Native return declared via `.returns(..)` (Fork-1b native half). `insert`'s
                // per-selector merge lets these coexist with AST-recorded returns.
                if let Some(ret) = &variant.ret_type {
                    method_returns.insert(
                        Arc::from(m.selector.as_str()),
                        Type::parse_annotation_str(ret, &type_params),
                    );
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
            method_returns,
            type_params,
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
        // Every VM-known class is a KNOWN type name — native classes (e.g.
        // `KeyValuePair`) must not draw `unknown type` when used in an
        // annotation. Idempotent set insert; runs before the authoritative
        // skip below so it covers every class on every compile.
        vm.options.seen_types.insert(&g.name);
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

    pub fn insert(&self, name: &str, mut sig: ClassSig) {
        let mut map = self.0.borrow_mut();
        // Return contracts accumulate across inserts: carry over any the new sig doesn't itself
        // provide, so a later `from_vm` overwrite (VM sigs have none) doesn't drop AST-recorded
        // returns. New wins per selector.
        if let Some(existing) = map.get(name) {
            for (sel, ty) in &existing.method_returns {
                sig.method_returns
                    .entry(sel.clone())
                    .or_insert_with(|| ty.clone());
            }
            // Type params survive a from_vm overwrite the same way returns do.
            if sig.type_params.is_empty() && !existing.type_params.is_empty() {
                sig.type_params = existing.type_params.clone();
            }
            if sig.from_vm {
                // A `populate_from_vm` refresh: its param entries derive from runtime
                // dispatch hints — the ERASED signature (`Block(T ^U)` → `"Block"`),
                // strictly poorer than any AST recording already present. Existing wins
                // per selector (found the hard way: `collect:`'s `U` stopped binding
                // after populate shadowed the rich entry). A from_vm entry is never
                // re-populated (the authoritative skip), so this can't go stale.
                for (sel, ps) in &existing.method_params {
                    sig.method_params.insert(sel.clone(), ps.clone());
                }
            } else {
                // An AST (re)definition: new wins per selector; carry over the rest.
                for (sel, ps) in &existing.method_params {
                    sig.method_params
                        .entry(sel.clone())
                        .or_insert_with(|| ps.clone());
                }
            }
        }
        map.insert(Arc::from(name), sig);
    }

    /// Merge declared return types into `name`'s entry without disturbing the rest of its signature
    /// — how a `Foo <-- {}` reopen contributes returns to an already-recorded (often `from_vm`)
    /// class. Upserts a bare entry if the class isn't in the table yet (a later `insert` fills the
    /// structural fields and preserves these returns).
    pub fn add_returns(&self, name: &str, returns: HashMap<Arc<str>, Type>) {
        if returns.is_empty() {
            return;
        }
        let mut map = self.0.borrow_mut();
        let entry = map.entry(Arc::from(name)).or_insert_with(|| ClassSig {
            parent: None,
            mixins: Vec::new(),
            own_selectors: HashSet::new(),
            sealed: false,
            has_catch_all: false,
            from_vm: false,
            method_params: HashMap::new(),
            method_returns: HashMap::new(),
            type_params: Vec::new(),
        });
        entry.method_returns.extend(returns);
    }

    /// Merge declared param types into `name`'s entry — the params-side twin
    /// of `add_returns`, for `Foo <-- {}` reopens (how the core collections
    /// carry their typed `each:` signatures, GENERICS_ARCH.md §11.4).
    pub fn add_params(&self, name: &str, params: HashMap<Arc<str>, Vec<Type>>) {
        if params.is_empty() {
            return;
        }
        let mut map = self.0.borrow_mut();
        let entry = map.entry(Arc::from(name)).or_insert_with(|| ClassSig {
            parent: None,
            mixins: Vec::new(),
            own_selectors: HashSet::new(),
            sealed: false,
            has_catch_all: false,
            from_vm: false,
            method_params: HashMap::new(),
            method_returns: HashMap::new(),
            type_params: Vec::new(),
        });
        entry.method_params.extend(params);
    }

    /// Merge mixin names into `name`'s entry. A `Foo <-- { .mix:Bar }` reopen runs its
    /// `.mix:` at RUNTIME, after the from_vm snapshot was taken — so the reopen's compile
    /// records the mixin here, or the hierarchy walk (typed returns/params on Iterate
    /// methods, GENERICS_ARCH.md §11.3) could never reach the mixin from the receiver.
    pub fn add_mixins(&self, name: &str, mixins: Vec<Arc<str>>) {
        if mixins.is_empty() {
            return;
        }
        let mut map = self.0.borrow_mut();
        let entry = map.entry(Arc::from(name)).or_insert_with(|| ClassSig {
            parent: None,
            mixins: Vec::new(),
            own_selectors: HashSet::new(),
            sealed: false,
            has_catch_all: false,
            from_vm: false,
            method_params: HashMap::new(),
            method_returns: HashMap::new(),
            type_params: Vec::new(),
        });
        for m in mixins {
            if !entry.mixins.contains(&m) {
                entry.mixins.push(m);
            }
        }
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

    /// The return type declared for `selector` by the nearest *ancestor* of `class` — walking mixins
    /// then parent (recursively), not `class` itself — paired with the class that declares it.
    /// `Object`, the universal root, is consulted as an implicit fallback so an override of a core
    /// method (e.g. `defined?`) is checked against its base contract even without an explicit parent
    /// link in the table. `None` when no ancestor declares a return for `selector` (Phase 3c·4b).
    pub fn inherited_return(&self, class: &str, selector: &str) -> Option<(Type, Arc<str>)> {
        let sig = self.get(class)?;
        let mut visited = HashSet::new();
        visited.insert(Arc::from(class));
        for mixin in &sig.mixins {
            if let Some(found) = self.declared_return_rec(mixin, selector, &mut visited) {
                return Some(found);
            }
        }
        if let Some(parent) = &sig.parent {
            if let Some(found) = self.declared_return_rec(parent, selector, &mut visited) {
                return Some(found);
            }
        }
        // Implicit universal root: every class IS-A Object, so its contract always applies.
        if class != "Object" {
            if let Some(t) = self
                .get("Object")
                .and_then(|s| s.method_returns.get(selector).cloned())
            {
                return Some((t, Arc::from("Object")));
            }
        }
        None
    }

    /// The declared return type for `selector` on `class` — the class's *own* contract if it has
    /// one, else the nearest inherited one (mixins, then the parent chain, which for built-ins
    /// reaches `Object`). `None` when no ancestor in the chain declares a return. This is what
    /// statically types a send to a known-class receiver (`list.count → Integer`); the universal
    /// `Object`-rooted fallback for unknown-typed receivers stays a separate path.
    pub fn declared_return(&self, class: &str, selector: &str) -> Option<Type> {
        let mut visited = HashSet::new();
        self.declared_return_rec(class, selector, &mut visited)
            .map(|(t, _)| t)
    }

    /// Like `declared_return`, but also names the DEFINING class — the scope
    /// whose type parameters a variable in the return type belongs to.
    pub fn declared_return_with_source(
        &self,
        class: &str,
        selector: &str,
    ) -> Option<(Type, Arc<str>)> {
        let mut visited = HashSet::new();
        self.declared_return_rec(class, selector, &mut visited)
    }

    /// The defining class's declared type parameters (empty if none/unknown).
    pub fn type_params_of(&self, class: &str) -> Vec<Arc<str>> {
        self.0
            .borrow()
            .get(class)
            .map(|s| s.type_params.clone())
            .unwrap_or_default()
    }

    /// `class`'s OWN declared param types for `selector` (no hierarchy walk —
    /// pass the defining class from `declared_return_with_source`).
    pub fn own_method_params_of(&self, class: &str, selector: &str) -> Option<Vec<Type>> {
        self.0
            .borrow()
            .get(class)
            .and_then(|s| s.method_params.get(selector).cloned())
    }

    /// The declared param types for `selector` on `class` or its nearest
    /// ancestor (mixins, then the parent chain), paired with the DEFINING
    /// class — the scope whose type parameters variables in those params
    /// belong to. The params-side twin of `declared_return_with_source`,
    /// feeding the block-literal expectation channel (GENERICS_ARCH.md §11.3).
    pub fn declared_params_with_source(
        &self,
        class: &str,
        selector: &str,
    ) -> Option<(Vec<Type>, Arc<str>)> {
        let mut visited = HashSet::new();
        self.declared_params_rec(class, selector, &mut visited)
    }

    fn declared_params_rec(
        &self,
        class: &str,
        selector: &str,
        visited: &mut HashSet<Arc<str>>,
    ) -> Option<(Vec<Type>, Arc<str>)> {
        if !visited.insert(Arc::from(class)) {
            return None;
        }
        let sig = self.get(class)?;
        if let Some(p) = sig.method_params.get(selector) {
            return Some((p.clone(), Arc::from(class)));
        }
        for mixin in &sig.mixins {
            if let Some(found) = self.declared_params_rec(mixin, selector, visited) {
                return Some(found);
            }
        }
        match &sig.parent {
            Some(parent) => self.declared_params_rec(parent, selector, visited),
            None => None,
        }
    }

    fn declared_return_rec(
        &self,
        class: &str,
        selector: &str,
        visited: &mut HashSet<Arc<str>>,
    ) -> Option<(Type, Arc<str>)> {
        if !visited.insert(Arc::from(class)) {
            return None;
        }
        let sig = self.get(class)?;
        if let Some(t) = sig.method_returns.get(selector) {
            return Some((t.clone(), Arc::from(class)));
        }
        for mixin in &sig.mixins {
            if let Some(found) = self.declared_return_rec(mixin, selector, visited) {
                return Some(found);
            }
        }
        match &sig.parent {
            Some(parent) => self.declared_return_rec(parent, selector, visited),
            None => None,
        }
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
