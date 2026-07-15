//! Scope and locals machinery: the `Scope` stack, local declaration/lookup,
//! per-local types, provenance, and narrowing state. Extends `Compiler` exactly
//! like the other satellites.

use super::*;

pub(super) struct Scope {
    pub(super) locals: HashSet<String>,
    /// Subset of `locals` declared with `let` — reassigning one is a compile error.
    pub(super) immutable: HashSet<String>,
    /// Declared type of a local/param, when known (Integer/Boolean); absent = Unknown.
    pub(super) types: HashMap<String, Type>,
    /// Subset of `types` that came from an *explicit* annotation, not devirt inference. A
    /// reassignment is checked against the declared type only for these — an inferred type is a
    /// hint, not a contract, so `var x = 0` reassigned to a String is fine, but `var x: Integer`
    /// reassigned to a String is not (Phase 3a).
    pub(super) declared_types: HashMap<String, Type>,
    /// Flow-narrowed types active in this scope (Phase 3c) — a guard refines a local/field here;
    /// `narrowed_type` reads the innermost. Empty until 3c·1 installs the narrowing rules.
    pub(super) narrowed: HashMap<NarrowKey, Type>,
    /// Provenance of each local's recorded type (Phase 4 why-chain): where it was declared/inferred
    /// and a short origin phrase. Keyed like `types`; read when a diagnostic blames the local.
    pub(super) provenance: HashMap<String, TypeProvenance>,
    /// True for the top-level scope of an object-initializer block (`X.new:{ … }`),
    /// where a bare `field = value` binds an instance field (no `var` required).
    pub(super) is_init: bool,
    /// True for a SPLICE scope — pushed around an inlined control-flow arm/body that
    /// carries local declarations (Slice 2d v2). Not a frame: `in_init_frame` skips it,
    /// and `declare_local` alpha-renames declarations made in it (see `renames`).
    pub(super) renaming: bool,
    /// Alpha-rename map for declarations made in a splice scope: original name → the
    /// fresh minted Symbol emitted in instructions. The name is source-unspellable
    /// (contains `·`, outside the identifier grammar), so user code can neither collide
    /// with nor reference it. Keyed by original name; every checker table above stays
    /// original-name-keyed — only instruction emission consults this (`local_symbol`).
    pub(super) renames: HashMap<String, Symbol>,
}

impl Compiler {
    pub(super) fn new_temp_var(&mut self) -> String {
        self.temp_counter += 1;
        format!("__qn_temp_{}", self.temp_counter)
    }

    pub(super) fn is_local(&self, name: &str) -> bool {
        if name == "self" {
            return true;
        }
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return true;
            }
        }
        false
    }

    pub(super) fn push_scope(&mut self, locals: HashSet<String>) {
        self.scopes.push(Scope {
            locals,
            immutable: HashSet::new(),
            types: HashMap::new(),
            declared_types: HashMap::new(),
            narrowed: HashMap::new(),
            provenance: HashMap::new(),
            is_init: false,
            renaming: false,
            renames: HashMap::new(),
        });
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Push a SPLICE scope (Slice 2d v2) around an inlined declaration-carrying arm/body:
    /// declarations made in it are alpha-renamed (`declare_local`), and it is transparent
    /// to `in_init_frame` (a splice is not a frame).
    pub(super) fn push_splice_scope(&mut self) {
        self.push_scope(HashSet::new());
        self.scopes.last_mut().unwrap().renaming = true;
    }

    /// Is the nearest REAL frame scope an object-initializer (`X.new:{…}`) scope?
    /// Splice scopes are transparent — an arm spliced inside a config literal still
    /// executes in the instantiating frame.
    pub(super) fn in_init_frame(&self) -> bool {
        for scope in self.scopes.iter().rev() {
            if !scope.renaming {
                return scope.is_init;
            }
        }
        false
    }

    /// The Symbol to EMIT for a plain-local `name`: the innermost scope that binds it
    /// decides — its alpha-rename if it is a splice scope, else the name itself. All
    /// checker tables stay original-name-keyed; only instruction emission calls this.
    pub(super) fn local_symbol(&self, name: &str) -> Symbol {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                if let Some(&sym) = scope.renames.get(name) {
                    return sym;
                }
                return Symbol::intern(name);
            }
        }
        Symbol::intern(name)
    }

    /// Declare a fresh local in the current (innermost) scope. Errors if the name is
    /// already declared *in this scope* (redeclaration); shadowing an outer scope is
    /// allowed. `let` bindings are recorded as immutable. In a splice scope (an inlined
    /// declaration-carrying arm, Slice 2d v2) the emitted Symbol is alpha-renamed to a
    /// source-unspellable `name·N` — the block frame that used to isolate this binding
    /// is gone, so the fresh name is what prevents collision with same-named siblings
    /// and the enclosing frame.
    pub(super) fn declare_local(&mut self, name: &str, mutable: bool) -> Result<(), String> {
        let minted = if self.scopes.last().unwrap().renaming {
            let n = self.splice_rename_counter;
            self.splice_rename_counter += 1;
            Some(Symbol::intern(&format!("{name}\u{b7}{n}")))
        } else {
            None
        };
        let scope = self.scopes.last_mut().unwrap();
        if scope.locals.contains(name) {
            return Err(format!("`{}` is already declared in this scope", name));
        }
        scope.locals.insert(name.to_string());
        if !mutable {
            scope.immutable.insert(name.to_string());
        }
        if let Some(sym) = minted {
            scope.renames.insert(name.to_string(), sym);
        }
        Ok(())
    }

    /// Was `name` declared with `let`? Resolves to the nearest scope that binds it
    /// (matching `is_local`'s innermost-first walk).
    pub(super) fn is_immutable(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.immutable.contains(name);
            }
        }
        false
    }

    /// Declared `Type` of a local/param — the nearest binding's recorded type,
    /// or `Unknown` (untyped, or not a plain local).
    pub(super) fn local_type(&self, name: &str) -> Type {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.types.get(name).cloned().unwrap_or(Type::Any);
            }
        }
        Type::Any
    }

    /// Record a known type for a local just declared in the innermost scope.
    pub(super) fn record_local_type(
        &mut self,
        name: &str,
        ty: Type,
        provenance: Option<TypeProvenance>,
    ) {
        if ty != Type::Any {
            let scope = self.scopes.last_mut().unwrap();
            scope.types.insert(name.to_string(), ty);
            if let Some(p) = provenance {
                scope.provenance.insert(name.to_string(), p);
            }
        }
    }

    /// Record a local's *declared* (annotated) type — into both `types` (devirt) and
    /// `declared_types` (the reassignment check, which enforces only explicit contracts).
    pub(super) fn record_declared_type(
        &mut self,
        name: &str,
        ty: Type,
        provenance: Option<TypeProvenance>,
    ) {
        if ty != Type::Any {
            let scope = self.scopes.last_mut().unwrap();
            scope.types.insert(name.to_string(), ty.clone());
            scope.declared_types.insert(name.to_string(), ty);
            if let Some(p) = provenance {
                scope.provenance.insert(name.to_string(), p);
            }
        }
    }

    /// Build a [`TypeProvenance`] pointing at `node`'s span with origin phrase `origin`, or `None`
    /// if `node` carries no source location (nothing useful to point at).
    pub(super) fn provenance_at(node: &Node, origin: String) -> Option<TypeProvenance> {
        Self::provenance_from(node.source_info.clone(), origin)
    }

    /// Build a [`TypeProvenance`] from a raw span (e.g. a param's `IdentifierNode`), or `None`.
    pub(super) fn provenance_from(
        span: Option<SourceInfo>,
        origin: String,
    ) -> Option<TypeProvenance> {
        span.map(|span| TypeProvenance { span, origin })
    }

    /// The provenance of a local's recorded type — where it was declared/inferred (Phase 4).
    pub(super) fn local_provenance(&self, name: &str) -> Option<&TypeProvenance> {
        self.scopes
            .iter()
            .rev()
            .find(|s| s.locals.contains(name))
            .and_then(|s| s.provenance.get(name))
    }

    /// The explicitly-declared type of a local, if any — `None` for an untyped local even when a
    /// type was *inferred* for it (an inferred type is a devirt hint, not a reassignment contract).
    pub(super) fn declared_type(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if scope.locals.contains(name) {
                return scope.declared_types.get(name).cloned();
            }
        }
        None
    }

    /// The flow-narrowed type of a path at the current point, if any — innermost scope wins
    /// (Phase 3c). Empty until 3c·1 installs narrowing, so today this always returns `None`.
    pub(super) fn narrowed_type(&self, key: &NarrowKey) -> Option<Type> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.narrowed.get(key).cloned())
    }
}
