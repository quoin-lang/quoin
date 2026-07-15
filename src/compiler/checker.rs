//! The best-effort static checker (TYPE_SYSTEM_ARCH.md): expected-type checks,
//! MNU/variant/literal-element diagnostics, Phase-3c flow narrowing and nil-misuse,
//! and static type inference (send return types, covariance). Extends `Compiler`
//! exactly like the other satellites.

use super::*;

/// Where a local's type came from (Phase 4 provenance), for the why-chain note: the declaration
/// span plus a short origin phrase (`declared`, `` inferred from `name` ``, `parameter`).
#[derive(Clone, Debug)]
pub(super) struct TypeProvenance {
    pub(super) span: SourceInfo,
    pub(super) origin: String,
}

/// Unary methods safe to send to `nil` — they don't dereference the receiver, so a possibly-nil
/// receiver isn't flagged for these (Phase 3c nil-misuse check).
const NIL_SAFE_SELECTORS: &[&str] = &["defined?", "s", "pp", "class", "hash", "print"];

/// A flow-narrowable path — what a guard (Phase 3c) can refine the type of. Only locals and
/// instance fields (`@name`) narrow; global, namespaced, and reserved reads do not.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) enum NarrowKey {
    Local(String),
    Field(String),
}

impl NarrowKey {
    /// The narrowable path an identifier read refers to, or `None` if it isn't one (a global,
    /// namespaced, or reserved `nil`/`true`/`false` read).
    pub(super) fn from_ident(id: &IdentifierNode) -> Option<NarrowKey> {
        if id.identifier_type == IdentifierType::Instance {
            Some(NarrowKey::Field(id.name.clone()))
        } else if id.namespace.is_some()
            || id.identifier_type == IdentifierType::Namespaced
            || matches!(id.name.as_str(), "nil" | "true" | "false")
        {
            None
        } else {
            Some(NarrowKey::Local(id.name.clone()))
        }
    }
}

// ---- AST shape matchers (Phase 3c) -----------------------------------------------------------
// Small, shallow structural matchers shared by the checker's recognizers. They match *one* level
// of shape and **bottom out on the semantic helpers** — path classification via
// `NarrowKey::from_ident`, selector reconstruction via `call_selector_*` — so a match can never
// silently disagree with the VM's dispatch (e.g. the variadic-fold selector). Compose these rather
// than re-deriving shapes inline; new checks add matchers here.

/// `RECV.sel` with no arguments → (receiver, selector). `None` for a keyword send or a
/// receiver-less (`self`) send.
fn as_unary_send(node: &Node) -> Option<(&Node, &str)> {
    let NodeValue::MethodCall(mc) = &node.value else {
        return None;
    };
    if !mc.arguments.expressions.is_empty() {
        return None;
    }
    let idents = &mc.arguments.signature.identifiers;
    if idents.len() != 1 {
        return None;
    }
    Some((mc.subject.as_deref()?, idents[0].name.as_str()))
}

/// The narrowable path an expression reads, if it is a bare local or `@field` identifier.
fn as_path(node: &Node) -> Option<NarrowKey> {
    match &node.value {
        NodeValue::Identifier(id) => NarrowKey::from_ident(id),
        _ => None,
    }
}

/// The reserved `nil` literal.
fn is_nil_literal(node: &Node) -> bool {
    matches!(&node.value, NodeValue::Identifier(id) if id.name == "nil")
}

/// A recognized nil-guard's narrowing (Phase 3c): the path it tests and the type it refines to in
/// each arm. For `x.defined?.if:{…} else:{…}` with `x: T?`, `if_arm = T`, `else_arm = Nil`.
pub(super) struct GuardInfo {
    pub(super) key: NarrowKey,
    pub(super) if_arm: Type,
    pub(super) else_arm: Type,
}

impl GuardInfo {
    /// The refinement for the arm reached by keyword `kw` (`if` → true branch, `else` → false).
    pub(super) fn arm_type(&self, kw: &str) -> Option<Type> {
        match kw {
            "if" => Some(self.if_arm.clone()),
            "else" => Some(self.else_arm.clone()),
            _ => None,
        }
    }
}

impl Compiler {
    /// Compile `node` in a position that expects `expected`. A numeric *literal* promotes to
    /// match (`1` where a `Double` is wanted → the Double `1.0`); otherwise it compiles normally
    /// and its synthesized type is checked against `expected`. Phase 3a.
    pub(super) fn compile_expecting(
        &mut self,
        node: &Node,
        expected: &Type,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Value-level promotion: an Integer *literal* where a Double is wanted becomes a Double.
        if *expected == Type::Double
            && let NodeValue::Integer(i) = &node.value
        {
            bytecode.push(Instruction::Push(Constant::Double(i.value as f64)));
            return Ok(());
        }
        self.compile_node(node, bytecode)?;
        self.check_type(node, expected);
        Ok(())
    }

    /// Warn if `node`'s statically-known type is confidently incompatible with `expected`. Silent
    /// whenever either side is `Any`, `expected` is an unknown class (already flagged as `unknown
    /// type`), or the actual type can't be pinned down — the gradual "never speak on Any" rule.
    fn check_type(&mut self, node: &Node, expected: &Type) {
        match expected {
            Type::Any => return,
            Type::Instance(n) if !self.seen_types.contains(n) => return,
            _ => {}
        }
        // A matching collection LITERAL in a checked position: check its (statically visible)
        // elements instead — its bare `List` static type would trip the width rule below, a
        // false positive since the literal is constructed into the checked position.
        if Self::generic_literal_decl(expected, node) {
            self.check_literal_elements(node, expected);
            return;
        }
        let actual = self.static_type(node);
        if actual.compatible_with(expected) {
            return;
        }
        // Instance-vs-Instance: the class table may prove a subtype relation that structural
        // `compatible_with` (exact match only) can't. `None` (unknown hierarchy) stays silent.
        if let (Type::Instance(sub), Type::Instance(sup)) = (&actual, expected) {
            match self.class_table.is_subtype(sub, sup) {
                Some(true) | None => return,
                Some(false) => {}
            }
        }
        let notes = self.mismatch_notes(node, &actual);
        self.warn_with_notes(
            "type-mismatch",
            format!(
                "type mismatch: expected `{}`, found `{}`",
                expected.name(),
                actual.name()
            ),
            node.source_info.as_ref(),
            notes,
        );
    }

    /// Why-chain notes for a type mismatch (Phase 4 provenance): if the offending expression is a
    /// local read, point back at where that local got its type (`` `x` is `String` (inferred from
    /// `name`) ``). Empty for literals/other expressions — their type is self-evident at the site.
    fn mismatch_notes(&self, node: &Node, actual: &Type) -> Vec<Note> {
        if let NodeValue::Identifier(id) = &node.value
            && let Some(NarrowKey::Local(name)) = NarrowKey::from_ident(id)
            && let Some(prov) = self.local_provenance(&name)
        {
            return vec![Note {
                message: format!("`{}` is `{}` ({})", name, actual.name(), prov.origin),
                span: Some(prov.span.clone()),
            }];
        }
        Vec::new()
    }

    /// Compile a returned value (`^expr` / `^^expr`), checked and promoted against the innermost
    /// declared return type on `return_type_stack`. `None` → compile normally, unchecked.
    pub(super) fn compile_return_value(
        &mut self,
        value: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        match self.return_type_stack.last().cloned().flatten() {
            Some(expected) => self.compile_expecting(value, &expected, bytecode),
            None => self.compile_node(value, bytecode),
        }
    }

    /// Compile-time MessageNotUnderstood: warn when a send targets a selector the receiver's class
    /// provably doesn't respond to. Sound only for an authoritative (`from_vm`), `sealed`, catch-all-
    /// free class — otherwise a future extension or dynamic handler could resolve it, so we stay
    /// silent (a missed MNU is fine; a wrong one is not). Resolution reuses `responds_to`, which is
    /// the VM's own dispatch walk.
    pub(super) fn check_mnu(&mut self, call: &MethodCallNode) {
        let Some(class) = self.receiver_class(call) else {
            return;
        };
        let Some(sig) = self.class_table.get(&class) else {
            return;
        };
        if !sig.from_vm || !sig.sealed || sig.has_catch_all {
            return;
        }
        let Some(selector) = Self::call_selector_simple(call) else {
            return;
        };
        if self.class_table.responds_to(&class, &selector) == Some(false) {
            self.warn(
                "mnu",
                format!("`{class}` does not respond to `{selector}`"),
                call.subject.as_deref().and_then(|n| n.source_info.as_ref()),
            );
        }
    }

    /// The receiver's concrete class name, if statically known. Only a user-class `Instance`:
    /// MNU claims ABSENCE, which the table can only prove inside a user-class hierarchy —
    /// builtins inherit from the open (hence stale-in-table) `Object`, whose qnlib reopens
    /// (`case:`, matchers) the walk would miss, and a Boolean value's `if:`/`not` live on the
    /// `true`/`false` eigenclasses, not the class. Builtin receivers feed only the arg-type
    /// checks (`arg_check_receiver_class`), which key on selector EXISTENCE.
    pub(super) fn receiver_class(&self, call: &MethodCallNode) -> Option<String> {
        match self.static_type(call.subject.as_ref()?) {
            Type::Instance(c) => Some(c.to_string()),
            _ => None,
        }
    }

    /// The receiver class the ARG-TYPE checks may use: a user-class `Instance`, or a builtin
    /// scalar/collection mapped to its class-table entry (the `from_vm` + `sealed` gates
    /// downstream still drop open classes like `String`). These checks warn only on a
    /// selector the class provably HAS with mismatching args, so trust matters more than
    /// coverage: for a LOCAL receiver, only flow narrowing and an explicit annotation count —
    /// never the inferred devirt hint a plain `var x = 5` records, which is deliberately
    /// allowed to go stale across reassignment (a devirt gate has a runtime fallback; a
    /// diagnostic does not). `Bool` is excluded: a Boolean value dispatches through the
    /// `true`/`false` eigenclasses, which the class's table entry doesn't model.
    fn arg_check_receiver_class(&self, call: &MethodCallNode) -> Option<String> {
        let subject = call.subject.as_ref()?;
        let t = match &subject.value {
            NodeValue::Identifier(id) => match NarrowKey::from_ident(id) {
                Some(key) => self.narrowed_type(&key).or_else(|| match &key {
                    NarrowKey::Local(name) => self.declared_type(name),
                    NarrowKey::Field(_) => None,
                })?,
                // `nil`/`true`/`false` receivers dispatch via eigenclasses; globals unknown.
                None => return None,
            },
            _ => self.static_type(subject),
        };
        match t {
            Type::Instance(c) => Some(c.to_string()),
            Type::Int => Some("Integer".to_string()),
            Type::Double => Some("Double".to_string()),
            Type::String => Some("String".to_string()),
            Type::List | Type::ListOf(_) => Some("List".to_string()),
            Type::Map | Type::MapOf(_) => Some("Map".to_string()),
            Type::Set | Type::SetOf(_) => Some("Set".to_string()),
            _ => None,
        }
    }

    /// The multimethod face of compile-time MNU: for an authoritative, sealed receiver whose
    /// selector is recorded with 2+ fully-typed variants (`method_param_variants`), warn when
    /// every argument's static type is confident and NO variant accepts them — dispatch then
    /// provably raises MessageNotUnderstood at runtime (`5.pow:'x'`). The single-variant case
    /// is `call_param_types`' (which also promotes literals); an unconfident argument
    /// (`Any` / nullable / type variable) keeps this silent, per the no-false-positives rule.
    pub(super) fn check_variant_mismatch(&mut self, call: &MethodCallNode) {
        let Some(class) = self.arg_check_receiver_class(call) else {
            return;
        };
        let Some(sig) = self.class_table.get(&class) else {
            return;
        };
        if !sig.from_vm || !sig.sealed || sig.has_catch_all {
            return;
        }
        let Some(selector) = Self::call_selector_nonvariadic(call) else {
            return;
        };
        let Some(variants) = sig.method_param_variants.get(selector.as_str()) else {
            return;
        };
        let args = &call.arguments.expressions;
        let arg_types: Vec<Type> = args.iter().map(|a| self.static_type(a)).collect();
        if arg_types
            .iter()
            .any(|t| matches!(t, Type::Any | Type::Nullable(_)) || t.contains_var())
        {
            return;
        }
        let accepted = variants.iter().any(|params| {
            params.len() == arg_types.len()
                && arg_types
                    .iter()
                    .zip(params)
                    .all(|(a, p)| a.compatible_with(p))
        });
        if !accepted {
            let got = arg_types
                .iter()
                .map(|t| format!("`{}`", t.name()))
                .collect::<Vec<_>>()
                .join(", ");
            let takes = variants
                .iter()
                .map(|v| {
                    format!(
                        "({})",
                        v.iter().map(|t| t.name()).collect::<Vec<_>>().join(" ")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.warn(
                "no-variant",
                format!(
                    "no `{selector}` variant on `{class}` accepts {got} — \
                     declared: {takes}; this raises MessageNotUnderstood at runtime"
                ),
                call.subject.as_deref().and_then(|n| n.source_info.as_ref()),
            );
        }
    }

    /// A collection LITERAL in a `List(T)`-checked position: its elements are statically
    /// visible, so check THEM against `T` — the same contract `check_generic_insertion`
    /// enforces on inserts — instead of letting the literal's bare `List` static type trip
    /// the width rule (a bare collection never satisfies a checked one, but THIS literal is
    /// constructed into the checked position, tagged on the decl path). Map literals check
    /// their values (keys are pinned String). Nil elements pass, matching the insert check.
    pub(super) fn check_literal_elements(&mut self, node: &Node, expected: &Type) {
        let (elem, items): (Type, Vec<&Arc<Node>>) = match (expected, &node.value) {
            (Type::ListOf(t), NodeValue::List(l)) => ((**t).clone(), l.values.iter().collect()),
            (Type::SetOf(t), NodeValue::Set(s)) => ((**t).clone(), s.values.iter().collect()),
            (Type::MapOf(t), NodeValue::Map(m)) => ((**t).clone(), m.values.iter().collect()),
            _ => return,
        };
        if elem.contains_var() {
            return;
        }
        let allowed = Type::Nullable(Box::new(elem));
        for item in items {
            let actual = self.static_type(item);
            if actual.compatible_with(&allowed) {
                continue;
            }
            // Instance subtyping may rescue (a Circle literal element into List(Shape)).
            if let (Type::Instance(sub), Type::Nullable(sup)) = (&actual, &allowed)
                && let Type::Instance(sup) = sup.as_ref()
                && self.class_table.is_subtype(sub, sup) != Some(false)
            {
                continue;
            }
            self.warn(
                "element-type",
                format!(
                    "`{}` rejects a `{}` element — this raises a TypeError at runtime",
                    expected.name(),
                    actual.name(),
                ),
                item.source_info.as_ref().or(node.source_info.as_ref()),
            );
        }
    }

    /// The canonical dispatched selector for a call — but only for the unambiguous shapes (unary, or
    /// a single keyword with one argument). Multi-keyword and variadic runs (which fold to `name+:`)
    /// return `None`, so MNU never reconstructs a selector that could differ from dispatch's.
    fn call_selector_simple(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if call.arguments.expressions.is_empty() {
            return idents.first().map(|i| i.name.clone());
        }
        if idents.len() == 1 && call.arguments.expressions.len() == 1 {
            return Some(format!("{}:", idents[0].name));
        }
        None
    }

    /// The canonical non-variadic selector of a call *with* args (`foo:` / `foo:bar:`). `None` for a
    /// no-arg call, or any variadic run (a repeated consecutive keyword, which folds to `name+:`).
    pub(super) fn call_selector_nonvariadic(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if call.arguments.expressions.is_empty() || idents.len() != call.arguments.expressions.len()
        {
            return None;
        }
        if idents.windows(2).any(|w| w[0].name == w[1].name) {
            return None; // a variadic run — its dispatched selector is `name+:`
        }
        Some(idents.iter().map(|i| format!("{}:", i.name)).collect())
    }

    /// The declared parameter types for a call, when they're checkable: the receiver is an
    /// authoritative (`from_vm`), `sealed` class, and the (non-variadic) selector resolves to a
    /// single fully-typed method whose arity matches. `None` → args compile unchecked (gradual).
    pub(super) fn call_param_types(&self, call: &MethodCallNode) -> Option<Vec<Type>> {
        let class = self.arg_check_receiver_class(call)?;
        let sig = self.class_table.get(&class)?;
        if !sig.from_vm || !sig.sealed {
            return None;
        }
        let selector = Self::call_selector_nonvariadic(call)?;
        let params = self.class_table.own_method_params(&class, &selector)?;
        (params.len() == call.arguments.expressions.len()).then_some(params)
    }

    /// The call's declared parameter types with the receiver's element type already bound
    /// (PARTIAL substitution — unbound variables stay variables). The front half of
    /// `typed_receiver_return_type`, run BEFORE the args compile so a block-literal argument
    /// can carry its declared `Block(…)` shape into `compile_block` as the expectation
    /// (G4b, GENERICS_ARCH.md §11.3). `None` = no declaration found (gradual).
    pub(super) fn receiver_bound_param_types(&self, call: &MethodCallNode) -> Option<Vec<Type>> {
        let subject = call.subject.as_ref()?;
        let recv_t = self.static_type(subject);
        let (class_name, recv_elem) = match &recv_t {
            Type::Any | Type::Never | Type::Nullable(_) => return None,
            Type::ListOf(e) => ("List".to_string(), Some((**e).clone())),
            Type::MapOf(e) => ("Map".to_string(), Some((**e).clone())),
            Type::SetOf(e) => ("Set".to_string(), Some((**e).clone())),
            concrete => (concrete.name(), None),
        };
        let selector = Self::reconstruct_send_selector(call)?;
        let (params, defining) = self
            .class_table
            .declared_params_with_source(&class_name, &selector)?;
        if params.len() != call.arguments.expressions.len() {
            return None;
        }
        let def_params = self.class_table.type_params_of(&defining);
        let mut bindings: std::collections::HashMap<Arc<str>, Type> =
            std::collections::HashMap::new();
        // The same Map nuance as `typed_receiver_return_type`: a Map's iteration
        // element is a key/value pair, so a MapOf receiver binds only Map's own methods.
        let elem_binds = !(matches!(recv_t, Type::MapOf(_)) && defining.as_ref() != "Map");
        if let (true, Some(elem), Some(p0)) = (elem_binds, recv_elem, def_params.first()) {
            bindings.insert(p0.clone(), elem);
        }
        Some(
            params
                .iter()
                .map(|p| p.substitute_bound(&bindings))
                .collect(),
        )
    }

    /// Recognize a nil-condition on a narrowable path (Phase 3c): `RECV.defined?`, or `RECV == nil`
    /// / `RECV != nil` (either operand order). Returns the path and whether a *true* result means
    /// RECV is non-nil (`.defined?` and `!= nil` → `true`; `== nil` → `false`).
    fn match_nil_condition(node: &Node) -> Option<(NarrowKey, bool)> {
        // `RECV.defined?` → a true result means RECV is non-nil.
        if let Some((recv, "defined?")) = as_unary_send(node) {
            return Some((as_path(recv)?, true));
        }
        // `RECV == nil` (⇒ nil) / `RECV != nil` (⇒ non-nil), either operand order.
        if let NodeValue::BinaryOperator(op) = &node.value
            && matches!(
                op.operator,
                BinaryOperatorType::Eq | BinaryOperatorType::NotEq
            )
        {
            return Some((
                Self::nil_comparison_key(&op.left, &op.right)?,
                op.operator == BinaryOperatorType::NotEq,
            ));
        }
        None
    }

    /// One operand is the reserved `nil`, the other a narrowable path → that path.
    fn nil_comparison_key(a: &Node, b: &Node) -> Option<NarrowKey> {
        if is_nil_literal(a) {
            as_path(b)
        } else if is_nil_literal(b) {
            as_path(a)
        } else {
            None
        }
    }

    /// A path's type at the current point: its flow-narrowed type if any, else the recorded local
    /// type (a field carries none → `Any`).
    fn path_type(&self, key: &NarrowKey) -> Type {
        self.narrowed_type(key).unwrap_or_else(|| match key {
            NarrowKey::Local(name) => self.local_type(name),
            NarrowKey::Field(_) => Type::Any,
        })
    }

    /// If `call` is a nil-guard conditional (`RECV.defined?` composed with `.if:`/`.if:else:`/
    /// `.else:`) whose path is currently `Nullable(T)`, the per-arm refinement. `None` otherwise —
    /// so narrowing only fires on a declared-nullable path, never on the optimizer's inferred types.
    pub(super) fn guard_narrowing(&self, call: &MethodCallNode) -> Option<GuardInfo> {
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if !matches!(kws.as_slice(), ["if"] | ["if", "else"] | ["else"]) {
            return None;
        }
        let (key, true_is_nonnil) = Self::match_nil_condition(call.subject.as_deref()?)?;
        let Type::Nullable(inner) = self.path_type(&key) else {
            return None;
        };
        let non_nil = *inner;
        let (if_arm, else_arm) = if true_is_nonnil {
            (non_nil, Type::Nil)
        } else {
            (Type::Nil, non_nil)
        };
        Some(GuardInfo {
            key,
            if_arm,
            else_arm,
        })
    }

    /// Does this arm expression always exit non-locally (its tail is `^^`/`^`)? Used for post-guard
    /// narrowing: when the nil-arm diverges, the surviving arm's refinement holds afterward.
    fn expr_diverges(node: &Node) -> bool {
        let NodeValue::Block(b) = &node.value else {
            return false;
        };
        matches!(
            b.statements.last().map(|s| &s.value),
            Some(NodeValue::MethodReturn(_)) | Some(NodeValue::BlockReturn(_))
        )
    }

    /// After a guard send, merge the arms' exit states into the enclosing scope (Phase 3c
    /// join/merge). The conditional has two paths — condition true (the `if:` block, or a straight
    /// fall-through with the guard's true refinement when there's no `if:`) and condition false
    /// (the `else:` block, or a fall-through with the false refinement). A path whose arm diverges
    /// (`^^`/`^`) drops out; the guarded key's type afterward is the **join** of the surviving
    /// paths' exit types (`if_exit`/`else_exit` are those arms' captured exits, defaulting to the
    /// bare refinement). Both diverging ⇒ the code after is unreachable, so nothing is installed.
    pub(super) fn apply_guard_join(
        &mut self,
        call: &MethodCallNode,
        g: &GuardInfo,
        if_exit: Option<Type>,
        else_exit: Option<Type>,
    ) {
        let idents = &call.arguments.signature.identifiers;
        let arm = |kw: &str| idents.iter().position(|i| i.name == kw);
        let diverges = |k: usize| Self::expr_diverges(&call.arguments.expressions[k]);

        let true_exit = match arm("if") {
            Some(k) if diverges(k) => None,
            Some(_) => Some(if_exit.unwrap_or_else(|| g.if_arm.clone())),
            None => Some(g.if_arm.clone()), // no `if:` block ⇒ true path falls through
        };
        let false_exit = match arm("else") {
            Some(k) if diverges(k) => None,
            Some(_) => Some(else_exit.unwrap_or_else(|| g.else_arm.clone())),
            None => Some(g.else_arm.clone()), // no `else:` block ⇒ false path falls through
        };

        let joined = match (true_exit, false_exit) {
            (Some(a), Some(b)) => Some(a.join(&b)),
            (Some(t), None) | (None, Some(t)) => Some(t),
            (None, None) => None,
        };
        if let Some(ty) = joined {
            self.update_narrowing(g.key.clone(), ty);
        }
    }

    /// Flow-update a *declared* path's narrowing after a (re)assignment (Phase 3c): a concrete
    /// rvalue type re-narrows it; an `Any` (unknown) rvalue drops to gradual. Only called for
    /// declared targets, so an untyped `var`'s inferred type is never shadowed.
    pub(super) fn update_narrowing(&mut self, key: NarrowKey, ty: Type) {
        let scope = self.scopes.last_mut().unwrap();
        if ty == Type::Any {
            scope.narrowed.remove(&key);
        } else {
            scope.narrowed.insert(key, ty);
        }
    }

    /// G2: warn when an insertion into a statically-checked collection would
    /// raise the runtime tag TypeError — `xs.add:'s'` where `xs: List(Integer)`.
    /// Mirrors the runtime check exactly: nil always passes (the element
    /// position is honestly `T?`), and a variable-typed element claims nothing.
    pub(super) fn check_generic_insertion(&mut self, call: &MethodCallNode) {
        let Some(subject) = call.subject.as_deref() else {
            return;
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return;
        };
        let recv_t = self.static_type(subject);
        let (elem, arg_idx) = match (&recv_t, selector.as_str()) {
            (Type::ListOf(e), "add:" | "push:") => ((**e).clone(), 0),
            (Type::ListOf(e), "at:put:") => ((**e).clone(), 1),
            (Type::SetOf(e), "add:") => ((**e).clone(), 0),
            (Type::MapOf(e), "at:put:") => ((**e).clone(), 1),
            _ => return,
        };
        if elem.contains_var() {
            return;
        }
        let Some(arg) = call.arguments.expressions.get(arg_idx) else {
            return;
        };
        let actual = self.static_type(arg);
        let allowed = Type::Nullable(Box::new(elem.clone()));
        if actual.compatible_with(&allowed) {
            return;
        }
        // Instance subtyping may rescue (a Circle into List(Shape)).
        if let (Type::Instance(sub), Type::Instance(sup)) = (&actual, &elem)
            && self.class_table.is_subtype(sub, sup) != Some(false)
        {
            return;
        }
        self.warn(
            "element-type",
            format!(
                "`{}` rejects a `{}` element — this raises a TypeError at runtime",
                recv_t.name(),
                actual.name(),
            ),
            arg.source_info.as_ref(),
        );
    }

    /// Phase 3c: warn on a non-nil-safe send to a receiver whose current (narrowed) type is
    /// confidently `Nullable(T)` — `nil.<sel>` would fail at runtime. Gated to explicit `T?` /
    /// narrowed paths (silent on `Any`), so it speaks only on opt-in nullable annotations, and a
    /// guarded (narrowed non-nil) receiver reads as `T` here and is silent.
    pub(super) fn check_nil_misuse(&mut self, call: &MethodCallNode) {
        let Some(subject) = call.subject.as_deref() else {
            return; // a self-send has no nullable receiver
        };
        if !matches!(self.static_type(subject), Type::Nullable(_)) {
            return;
        }
        let idents = &call.arguments.signature.identifiers;
        // A nil-safe unary method (`s`, `class`, `defined?`, …) doesn't dereference the receiver.
        if call.arguments.expressions.is_empty()
            && let Some(first) = idents.first()
            && NIL_SAFE_SELECTORS.contains(&first.name.as_str())
        {
            return;
        }
        let selector = if call.arguments.expressions.is_empty() {
            idents.first().map(|i| i.name.clone()).unwrap_or_default()
        } else {
            Self::call_selector_nonvariadic(call).unwrap_or_else(|| {
                format!(
                    "{}:",
                    idents.first().map(|i| i.name.as_str()).unwrap_or("?")
                )
            })
        };
        self.warn(
            "nil-receiver",
            format!("receiver of `{selector}` may be nil"),
            subject.source_info.as_ref(),
        );
    }

    /// Phase 3c: warn on a nil-dereferencing binary op whose left operand is confidently nullable
    /// (`x + 1` where `x: Integer?`). Equality and logical ops tolerate a `nil` left and are exempt.
    pub(super) fn check_binop_nil_misuse(&mut self, op: &BinaryOperatorNode) {
        use BinaryOperatorType::*;
        if matches!(op.operator, Eq | NotEq | And | Or | Unknown) {
            return;
        }
        if matches!(self.static_type(&op.left), Type::Nullable(_)) {
            self.warn(
                "nil-receiver",
                format!(
                    "left operand of `{}` may be nil",
                    Self::binop_symbol(&op.operator)
                ),
                op.left.source_info.as_ref(),
            );
        }
    }

    fn binop_symbol(op: &BinaryOperatorType) -> &'static str {
        use BinaryOperatorType::*;
        match op {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Mod => "%",
            Gt => ">",
            GtEq => ">=",
            Lt => "<",
            LtEq => "<=",
            Range => "..",
            Match => "=~",
            Eq => "==",
            NotEq => "!=",
            And => "&&",
            Or => "||",
            Unknown => "?",
        }
    }

    /// Install the *true*-branch refinement of a nil-condition into the current scope, returning
    /// what to restore. Used for `&&` short-circuit narrowing (`RECV.defined? && EXPR`).
    pub(super) fn push_true_narrowing(&mut self, cond: &Node) -> Option<(NarrowKey, Option<Type>)> {
        let (key, true_is_nonnil) = Self::match_nil_condition(cond)?;
        let Type::Nullable(inner) = self.path_type(&key) else {
            return None;
        };
        let refined = if true_is_nonnil { *inner } else { Type::Nil };
        let scope = self.scopes.last_mut().unwrap();
        let saved = scope.narrowed.get(&key).cloned();
        scope.narrowed.insert(key.clone(), refined);
        Some((key, saved))
    }

    pub(super) fn pop_narrowing(&mut self, restore: Option<(NarrowKey, Option<Type>)>) {
        if let Some((key, saved)) = restore {
            let scope = self.scopes.last_mut().unwrap();
            match saved {
                Some(t) => scope.narrowed.insert(key, t),
                None => scope.narrowed.remove(&key),
            };
        }
    }

    /// Statically infer an expression's type for devirtualization. Conservative: only
    /// literals, typed locals/params, and numeric operators on them are known; anything
    /// else is `Unknown` and compiles to a normal dynamic `Send`.
    pub(super) fn static_type(&self, node: &Node) -> Type {
        match &node.value {
            // Literals synthesize their builtin type. (Only `Int`/`List`/`Bool` drive devirt;
            // the rest are inert there but let the checker see real mismatches — Phase 3a.)
            NodeValue::Integer(_) => Type::Int,
            NodeValue::Double(_) => Type::Double,
            NodeValue::Str(_) => Type::String,
            NodeValue::List(_) => Type::List,
            NodeValue::Map(_) => Type::Map,
            NodeValue::Set(_) => Type::Set,
            // A block literal is bare `Block` until its body compiles; after that, its sharpened
            // `Block(args ^Ret)` shape if it has one (G4b block-literal inference, §11.3).
            NodeValue::Block(b) => self
                .block_literal_types
                .get(&(b as *const BlockNode as usize))
                .cloned()
                .unwrap_or(Type::Block),
            NodeValue::Identifier(id) => match NarrowKey::from_ident(id) {
                // A narrowable read (local or `@field`): its flow-narrowed type if any (Phase 3c),
                // else the recorded local type (a field carries none → `Any`).
                Some(key) => self.path_type(&key),
                // Not narrowable: `nil`/`true`/`false` are reserved names (they parse as plain
                // idents, so match by name); everything else (globals/namespaced) is unknown here.
                None => match id.name.as_str() {
                    "nil" => Type::Nil,
                    "true" | "false" => Type::Bool,
                    _ => Type::Any,
                },
            },
            NodeValue::BinaryOperator(op) => self.binop_result_type(op),
            // A send's static type: a self-send to a current-class method, else the receiver's
            // own/inherited declared return (known-typed receiver), else an Object-rooted return
            // (universal, any receiver). Each is a safe miss → `Any`, so they layer by confidence.
            NodeValue::MethodCall(call) => match self.self_send_return_type(call) {
                Type::Any => match self.construction_return_type(call) {
                    Type::Any => match self.typed_receiver_return_type(call) {
                        Type::Any => self.object_rooted_return_type(call),
                        t => t,
                    },
                    t => t,
                },
                t => t,
            },
            _ => Type::Any,
        }
    }

    /// A self-send (`.sel:(…)` — no explicit receiver, or an explicit `self`) to a
    /// current-class method with a declared return type is statically that type. Non-self
    /// sends, unknown selectors, and variadic sends stay `Unknown` (a safe miss).
    fn self_send_return_type(&self, call: &MethodCallNode) -> Type {
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };
        if !is_self {
            return Type::Any;
        }
        let Some(ctx) = self.class_ctx.last() else {
            return Type::Any;
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        ctx.returns.get(&selector).cloned().unwrap_or(Type::Any)
    }

    /// Reconstruct a send's selector from its arguments — the bare name for a unary send, the
    /// joined `name:` parts for a keyword send. `None` for an empty signature. A variadic run
    /// (a keyword repeated, dispatched as `name+:`) isn't reconstructed, so such a send simply
    /// misses — a safe `Any` rather than a mismatched selector.
    fn reconstruct_send_selector(call: &MethodCallNode) -> Option<String> {
        let idents = &call.arguments.signature.identifiers;
        if idents.is_empty() {
            return None;
        }
        Some(if call.arguments.expressions.is_empty() {
            idents[0].name.clone()
        } else {
            idents.iter().map(|i| format!("{}:", i.name)).collect()
        })
    }

    /// The static return type of a send whose *receiver* has a known concrete type: the receiver
    /// class's own or inherited declared return for the selector (`list.count` → `Integer`,
    /// `d.floor` → `Integer`, `set.contains?:x` → `Boolean`). `Any` when the receiver's type is
    /// unknown/nullable or no return is declared. Sound like the Object-rooted path — return
    /// covariance guarantees any override returns a compatible type, so the declared return
    /// bounds the actual one.
    fn typed_receiver_return_type(&self, call: &MethodCallNode) -> Type {
        let Some(subject) = &call.subject else {
            return Type::Any;
        };
        // Only a receiver with a known concrete class qualifies; a nullable receiver's send is the
        // nil-misuse check's concern, not typed here. A checked collection looks up under its BASE
        // class and carries its element type into type-variable binding (GENERICS_ARCH.md §4.4).
        let recv_t = self.static_type(subject);
        let (class_name, recv_elem) = match &recv_t {
            Type::Any | Type::Never | Type::Nullable(_) => return Type::Any,
            Type::ListOf(e) => ("List".to_string(), Some((**e).clone())),
            Type::MapOf(e) => ("Map".to_string(), Some((**e).clone())),
            Type::SetOf(e) => ("Set".to_string(), Some((**e).clone())),
            concrete => (concrete.name(), None),
        };
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        let Some((ret, defining)) = self
            .class_table
            .declared_return_with_source(&class_name, &selector)
        else {
            return Type::Any;
        };
        if !ret.contains_var() {
            return ret;
        }
        // Bind the defining class's variables at this call site: the receiver's
        // element type binds the FIRST header parameter; declared param types
        // (if recorded) unify structurally against the arguments' static types.
        let def_params = self.class_table.type_params_of(&defining);
        let mut bindings: std::collections::HashMap<Arc<str>, Type> =
            std::collections::HashMap::new();
        // A Map's tag is its VALUE type, but its ITERATION element is a
        // key/value pair — so a MapOf receiver binds only methods Map itself
        // defines (`at:` → V?); an inherited/mixin method (Iterate's
        // combinators) must not claim the value type for pair elements.
        let elem_binds = !(matches!(recv_t, Type::MapOf(_)) && defining.as_ref() != "Map");
        if let (true, Some(elem), Some(p0)) = (elem_binds, recv_elem, def_params.first()) {
            bindings.insert(p0.clone(), elem);
        }
        if let Some(decl_params) = self.class_table.own_method_params_of(&defining, &selector) {
            let args = &call.arguments.expressions;
            for (decl, arg) in decl_params.iter().zip(args.iter()) {
                Type::unify_into(decl, &self.static_type(arg), &mut bindings);
            }
        }
        Self::normalize_any_elems(ret.substitute(&bindings))
    }

    /// `List(Any)` (an unbound variable after substitution) is just `List` —
    /// don't let inference mint element claims out of nothing.
    fn normalize_any_elems(t: Type) -> Type {
        match t {
            Type::ListOf(e) if *e == Type::Any => Type::List,
            Type::MapOf(e) if *e == Type::Any => Type::Map,
            Type::SetOf(e) if *e == Type::Any => Type::Set,
            Type::Nullable(inner) => match Self::normalize_any_elems(*inner) {
                Type::Any => Type::Any,
                t => Type::Nullable(Box::new(t)),
            },
            other => other,
        }
    }

    /// Construction inference for the checked-conversion surface: `List.of:X`,
    /// `Map.of:X`, `Set.of:X`, and `recv.ensure:X` — the element class is a
    /// statically-visible Identifier argument, so the result types as the
    /// checked collection (GENERICS_ARCH.md §7.1's static sources).
    fn construction_return_type(&self, call: &MethodCallNode) -> Type {
        let Some(selector) = Self::reconstruct_send_selector(call) else {
            return Type::Any;
        };
        if selector != "of:" && selector != "ensure:" {
            return Type::Any;
        }
        let Some(subject) = &call.subject else {
            return Type::Any;
        };
        let [arg] = call.arguments.expressions.as_slice() else {
            return Type::Any;
        };
        let NodeValue::Identifier(elem_id) = &arg.value else {
            return Type::Any;
        };
        let elem = Type::from_annotation_name(&ident_name(elem_id));
        if matches!(elem, Type::Any | Type::Nil | Type::Never) {
            return Type::Any;
        }
        let base = if selector == "of:" {
            // `List.of:X` — the receiver is the collection class itself.
            match &subject.value {
                NodeValue::Identifier(id) => ident_name(id),
                _ => return Type::Any,
            }
        } else {
            // `xs.ensure:X` — the receiver is a collection value.
            match self.static_type(subject) {
                Type::List | Type::ListOf(_) => "List".to_string(),
                Type::Map | Type::MapOf(_) => "Map".to_string(),
                Type::Set | Type::SetOf(_) => "Set".to_string(),
                _ => return Type::Any,
            }
        };
        match base.as_str() {
            "List" => Type::ListOf(Box::new(elem)),
            "Map" => Type::MapOf(Box::new(elem)),
            "Set" => Type::SetOf(Box::new(elem)),
            _ => Type::Any,
        }
    }

    /// The static return type of a no-arg send whose selector is declared on `Object`, the
    /// universal root — e.g. `x.defined?` → `Boolean`. Sound for *any* receiver because the
    /// return-covariance check (Phase 3c·4b) guarantees every override returns a compatible type.
    /// This is what lets narrowing/nil-misuse see through a `.defined?` guard and lets the guard
    /// devirt-inline as a real Bool conditional (Phase 3c·4c). Only `Object`-rooted selectors
    /// qualify, so it can't misjudge an unrelated same-named method on some other class.
    fn object_rooted_return_type(&self, call: &MethodCallNode) -> Type {
        if !call.arguments.expressions.is_empty() {
            return Type::Any;
        }
        let [sel] = call.arguments.signature.identifiers.as_slice() else {
            return Type::Any;
        };
        self.class_table
            .get("Object")
            .and_then(|s| s.method_returns.get(sel.name.as_str()).cloned())
            .unwrap_or(Type::Any)
    }

    /// Return-type covariance (the Liskov rule): a method that overrides an ancestor's method must
    /// return a type usable where the ancestor's *declared* return is expected — a subtype is fine,
    /// a widened or unrelated type is not. Warns on a confident violation, pointing at the override's
    /// `^Ret` annotation. Gradual: silent unless both returns are known and the mismatch can't be
    /// explained by class subtyping. This is what makes `Object#defined? : Boolean` a contract every
    /// override must honor, so `x.defined?` is soundly `Boolean` (Phase 3c·4b). `class_name` and its
    /// ancestors must already be in the class table (true at the class's compile site).
    pub(super) fn check_return_covariance(&mut self, class_name: &str, block: &BlockNode) {
        for stmt in &block.statements {
            let (sig, blk) = match &stmt.value {
                NodeValue::MethodDefinition(m) => (&m.signature, &m.block),
                NodeValue::MethodExtension(m) => (&m.signature, &m.block),
                _ => continue,
            };
            let Some(rt) = &blk.return_type else { continue };
            let Ok(selector) = self.reconstruct_selector(sig) else {
                continue;
            };
            let Some((base, from)) = self.class_table.inherited_return(class_name, &selector)
            else {
                continue;
            };
            let over = type_from_ref_with_vars(rt, &self.ctx_type_params());
            if self.override_return_violates(&over, &base) {
                self.warn(
                    "return-type",
                    format!(
                        "override of `{}` returns `{}`, incompatible with `{}` from `{}`",
                        selector,
                        over.name(),
                        base.name(),
                        from,
                    ),
                    rt.ident.source_info.as_ref(),
                );
            }
        }
    }

    /// Is an override returning `over` a *confident* covariance violation against a base return
    /// `base`? Only speaks when sure — a scalar mismatch (no class subtyping can rescue it) or a
    /// *proven* non-subtype between two bare classes. Anything the type/class lattice can't
    /// adjudicate (mixed class/scalar, nullable-of-class, unknown classes) stays silent (no FP).
    fn override_return_violates(&self, over: &Type, base: &Type) -> bool {
        if over.compatible_with(base) {
            return false; // Any/Never/exact/nullable-rules all fit
        }
        if Self::type_is_class_free(over) && Self::type_is_class_free(base) {
            return true; // e.g. `String` where `Boolean` is declared
        }
        if let (Type::Instance(o), Type::Instance(b)) = (over, base) {
            // Covariant returns permit a subtype; only a proven non-subtype is a violation.
            return self.class_table.is_subtype(o, b) == Some(false);
        }
        false
    }

    /// Does `ty` mention no class name (recursing through `Nullable`)? Such types have no subtype
    /// relation beyond `compatible_with`, so an incompatibility between two of them is definite.
    fn type_is_class_free(ty: &Type) -> bool {
        match ty {
            Type::Instance(_) => false,
            Type::Nullable(inner) => Self::type_is_class_free(inner),
            _ => true,
        }
    }

    /// Static result type of a binary operator. Comparison/equality operators yield `Bool`
    /// for *any* operands (Slice 2d, option B) — a language guarantee that they return
    /// `Boolean`, which lets `(a < b).if:…` / `(x == y).if:…` inline even when the operands
    /// aren't statically typed. Arithmetic yields `Int` only when *both* operands are
    /// statically `Int` — the soundness condition for devirtualizing to the direct i64 ops.
    /// Everything else (incl. `~`/`..`, and `&&`/`||`, which return an operand value not a
    /// `Bool`) stays `Unknown`.
    fn binop_result_type(&self, op: &BinaryOperatorNode) -> Type {
        use BinaryOperatorType::*;
        match op.operator {
            // A comparison is statically `Bool` ONLY when both operands are
            // native scalars (Int/Double): those devirtualize to direct i64/
            // f64 compares that bypass dispatch, so no user override can
            // intervene. For any other operand types the comparison goes
            // through `==:`/`<:`/… dispatch, which a user class may override
            // to return a non-Boolean — so the result type is Unknown and
            // the inlined `if:` uses its GUARDED form (BUGS.md Finding 1).
            Lt | LtEq | Gt | GtEq | Eq | NotEq
                if matches!(self.static_type(&op.left), Type::Int | Type::Double)
                    && matches!(self.static_type(&op.right), Type::Int | Type::Double) =>
            {
                Type::Bool
            }
            Add | Sub | Mul | Div | Mod
                if self.static_type(&op.left) == Type::Int
                    && self.static_type(&op.right) == Type::Int =>
            {
                Type::Int
            }
            Add | Sub | Mul | Div | Mod
                if self.static_type(&op.left) == Type::Double
                    && self.static_type(&op.right) == Type::Double =>
            {
                Type::Double
            }
            _ => Type::Any,
        }
    }
}
