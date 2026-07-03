//! Building the compile-time class signature (ClassCtx / ClassSig) from a class body:
//! method returns, sealed flag, mixins, per-selector bodies (Phase 3b/3c·4/5).

use super::*;

impl Compiler {
    /// Selector → declared-return-`Type` map for a class body, from its method
    /// definitions/extensions that carry a return type.
    pub(super) fn collect_class_ctx(&mut self, name: &str, block: &BlockNode) -> ClassCtx {
        let mut returns = HashMap::new();
        let mut bodies = HashMap::new();
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
                bodies.insert(selector.clone(), method_block.clone());
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
        ClassCtx {
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

    /// The class name in a `.mix:X` self-send (a mixin application), if this call is one.
    fn mixin_target(call: &MethodCallNode) -> Option<&str> {
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
            NodeValue::Identifier(id) => Some(id.name.as_str()),
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
                        mixins.push(Arc::from(mixin));
                    }
                }
                _ => {}
            }
        }
        ClassSig {
            parent: class_def
                .parent_identifier
                .as_ref()
                .map(|p| Arc::from(p.name.as_str())),
            mixins,
            own_selectors,
            sealed,
            has_catch_all: false,
            from_vm: false,
            method_params: HashMap::new(),
            method_returns: self.declared_method_returns(&class_def.block),
        }
    }

    /// Declared return types (`selector → Type`) for the methods written directly in a class body —
    /// only those with a `^Ret` header. Pure (`&self`, no diagnostics): the return-type check
    /// already warns on unknown annotations, so recording resolves names without re-warning. Feeds
    /// `ClassSig::method_returns` for both `Foo <- {}` defs and `Foo <-- {}` reopens (Phase 3c·4).
    pub(super) fn declared_method_returns(&self, block: &BlockNode) -> HashMap<Arc<str>, Type> {
        let mut out = HashMap::new();
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            if let (Ok(sel), Some(rt)) = (self.reconstruct_selector(sig), &blk.return_type) {
                out.insert(
                    Arc::from(sel.as_str()),
                    Type::from_annotation_name(&rt.name),
                );
            }
        }
        out
    }
}
