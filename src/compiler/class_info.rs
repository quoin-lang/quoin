//! Building the compile-time class signature (ClassCtx / ClassSig) from a class body:
//! method returns, sealed flag, mixins, per-selector bodies (Phase 3b/3c·4/5).

use super::*;

impl Compiler {
    /// Selector → declared-return-`Type` map for a class body, from its method
    /// definitions/extensions that carry a return type.
    pub(super) fn collect_class_ctx(
        &mut self,
        name: &str,
        block: &BlockNode,
        type_params: Vec<String>,
    ) -> ClassCtx {
        // Push a stub ctx so the header's type parameters are in scope while the
        // declared returns below resolve (`^T` must be a variable, not an
        // unknown class); the caller pushes the real ctx right after.
        self.class_ctx_counter += 1;
        self.class_ctx.push(ClassCtx {
            id: self.class_ctx_counter,
            name: name.to_string(),
            multi: HashSet::new(),
            type_params: type_params.clone(),
            returns: HashMap::new(),
            bodies: HashMap::new(),
            sealed: false,
        });
        let mut returns = HashMap::new();
        let mut bodies = HashMap::new();
        let mut multi = HashSet::new();
        let mut sealed = false;
        for stmt in &block.statements {
            let (signature, method_block) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                // A direct (unconditional) `sealed!` statement seals the class at compile
                // time, freezing its method table so same-class self-sends devirtualize.
                NodeValue::MethodCall(call) if Self::is_sealed_marker(call) => {
                    sealed = true;
                    continue;
                }
                _ => continue,
            };
            if let Ok(selector) = self.reconstruct_selector(signature) {
                if bodies
                    .insert(selector.clone(), method_block.clone())
                    .is_some()
                {
                    multi.insert(selector.clone());
                }
                if let Some(rt) = &method_block.return_type {
                    returns.insert(selector, self.resolve_annotation(rt));
                }
            }
        }
        // Record this class's bodies unit-wide so an explicit-receiver `v.x` can inline against it
        // from another class (Phase 5·3b). A reopen (`<--`) merges; anonymous targets are skipped.
        if !name.is_empty() {
            self.class_bodies
                .entry(name.to_string())
                .or_default()
                .extend(bodies.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let stub = self.class_ctx.pop().expect("stub ctx pushed above");
        ClassCtx {
            id: stub.id,
            name: name.to_string(),
            multi,
            type_params,
            returns,
            bodies,
            sealed,
        }
    }

    /// A bare `sealed!` self-send (`sealed!` or `self.sealed!`, no args).
    fn is_sealed_marker(call: &MethodCallNode) -> bool {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        is_self
            && call.arguments.expressions.is_empty()
            && call.arguments.signature.identifiers.len() == 1
            && call.arguments.signature.identifiers[0].name == "sealed!"
    }

    /// The class name in a `.mix:X` self-send (a mixin application), if this call is one —
    /// in the qualified form (`[Ns]Name`) the class table is keyed by.
    fn mixin_target(call: &MethodCallNode) -> Option<String> {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        if !is_self
            || call.arguments.signature.identifiers.len() != 1
            || call.arguments.signature.identifiers[0].name != "mix"
            || call.arguments.expressions.len() != 1
        {
            return None;
        }
        match &call.arguments.expressions[0].value {
            NodeValue::Identifier(id) => Some(ident_name(id)),
            _ => None,
        }
    }

    /// Build a `ClassSig` from a class definition's AST — the current-unit source for the class
    /// table (Phase 3b). Selectors come from the same `reconstruct_selector` as `collect_class_ctx`,
    /// so the method set can't drift from it. `has_catch_all` is left `false` here (only MNU uses
    /// it, and MNU consults VM-sourced sigs); the parent comes from the def, mixins from `.mix:`.
    pub(super) fn class_sig_from_def(&self, class_def: &ClassDefinitionNode) -> ClassSig {
        let mut own_selectors = HashSet::new();
        let mut mixins = Vec::new();
        let mut sealed = false;
        for stmt in &class_def.block.statements {
            match &stmt.value {
                NodeValue::MethodDefinition(m) => {
                    if let Ok(sel) = self.reconstruct_selector(&m.signature) {
                        own_selectors.insert(Arc::from(sel.as_str()));
                    }
                }
                NodeValue::MethodExtension(m) => {
                    if let Ok(sel) = self.reconstruct_selector(&m.signature) {
                        own_selectors.insert(Arc::from(sel.as_str()));
                    }
                }
                NodeValue::MethodCall(call) if Self::is_sealed_marker(call) => sealed = true,
                NodeValue::MethodCall(call) => {
                    if let Some(mixin) = Self::mixin_target(call) {
                        mixins.push(Arc::from(mixin.as_str()));
                    }
                }
                _ => {}
            }
        }
        ClassSig {
            parent: class_def
                .parent_identifier
                .as_ref()
                .map(|p| Arc::from(ident_name(p).as_str())),
            mixins,
            own_selectors,
            sealed,
            has_catch_all: false,
            from_vm: false,
            method_params: self.declared_method_params(&class_def.block, &class_def.type_params),
            method_returns: self
                .declared_method_returns_with_vars(&class_def.block, &class_def.type_params),
            type_params: class_def
                .type_params
                .iter()
                .map(|p| Arc::from(p.as_str()))
                .collect(),
        }
    }

    /// Declared return types (`selector → Type`) for the methods written directly in a class body —
    /// only those with a `^Ret` header. Pure (`&self`, no diagnostics): the return-type check
    /// already warns on unknown annotations, so recording resolves names without re-warning. Feeds
    /// `ClassSig::method_returns` for both `Foo <- {}` defs and `Foo <-- {}` reopens (Phase 3c·4).
    pub(super) fn declared_method_returns(&self, block: &BlockNode) -> HashMap<Arc<str>, Type> {
        self.declared_method_returns_with_vars(block, &[])
    }

    /// `declared_method_returns` with the class header's type parameters in
    /// scope, so `^T` records as `Var("T")` rather than an unknown instance.
    pub(super) fn declared_method_returns_with_vars(
        &self,
        block: &BlockNode,
        vars: &[String],
    ) -> HashMap<Arc<str>, Type> {
        let mut out = HashMap::new();
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            if let (Ok(sel), Some(rt)) = (self.reconstruct_selector(sig), &blk.return_type) {
                out.insert(Arc::from(sel.as_str()), type_from_ref_with_vars(rt, vars));
            }
        }
        out
    }

    /// Declared param types per selector, for call-site argument unification
    /// and arg checks: recorded only when a selector has exactly ONE variant
    /// with EVERY parameter annotated (the same rule the VM-side sig uses).
    pub(super) fn declared_method_params(
        &self,
        block: &BlockNode,
        vars: &[String],
    ) -> HashMap<Arc<str>, Vec<Type>> {
        let mut out: HashMap<Arc<str>, Option<Vec<Type>>> = HashMap::new();
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            let Ok(sel) = self.reconstruct_selector(sig) else {
                continue;
            };
            let all_typed: Option<Vec<Type>> = blk
                .arguments
                .iter()
                .map(|a| {
                    a.type_hint
                        .as_ref()
                        .map(|tr| type_from_ref_with_vars(tr, vars))
                })
                .collect();
            let entry = match (all_typed, blk.arguments.is_empty()) {
                (Some(types), false) => Some(types),
                _ => None,
            };
            // A repeated selector (multimethod) is ambiguous — drop it.
            match out.entry(Arc::from(sel.as_str())) {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    o.insert(None);
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(entry);
                }
            }
        }
        out.into_iter()
            .filter_map(|(k, v)| v.map(|types| (k, types)))
            .collect()
    }
}
