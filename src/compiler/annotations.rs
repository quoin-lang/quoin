//! Type-annotation and name resolution: `TypeRefNode` -> `Type`, class type-var
//! scoping, and enforceable element tags. Split from the compiler root; extends
//! `Compiler` exactly like the other satellites (`devirt`, `assignment`, ...).

use super::*;

/// Canonical string form of a type-annotation (or class-name) identifier — bare for a
/// root name (`Integer`, `Foo?`), bracket-qualified when namespaced (`[Web]Halt`).
/// Must agree with `NamespacedName`'s `Display`, which keys `globals`, runtime dispatch
/// hints, and `populate_from_vm`'s class-table entries.
pub(crate) fn annotation_name(tr: &TypeRefNode) -> String {
    let base = NamespacedName::from_ast(&tr.ident).to_string();
    if !tr.parenthesized {
        return base;
    }
    let mut parts: Vec<String> = tr.args.iter().map(|a| annotation_name(a)).collect();
    if let Some(r) = &tr.ret {
        parts.push(format!("^{}", annotation_name(r)));
    }
    format!("{}({})", base, parts.join(" "))
}

/// A plain identifier's rendered name (parent classes, mixin targets — the
/// non-type-annotation positions).
pub(crate) fn ident_name(ident: &IdentifierNode) -> String {
    NamespacedName::from_ast(ident).to_string()
}

/// Pure structural `Type` of an annotation — no diagnostics, no type-variable
/// scope (the resolver's `resolve_annotation` layers those on top). Unknown
/// bases become `Instance`; malformed generic arity degrades to the bare base.
pub(crate) fn type_from_ref(tr: &TypeRefNode) -> Type {
    type_from_ref_with_vars(tr, &[])
}

/// `type_from_ref` with the enclosing class's declared type parameters in
/// scope: a bare matching name resolves to `Var` (signature recording for the
/// class table, where the compiler's ctx stack isn't available).
pub(crate) fn type_from_ref_with_vars(tr: &TypeRefNode, vars: &[String]) -> Type {
    let base = NamespacedName::from_ast(&tr.ident).to_string();
    if !tr.parenthesized {
        if tr.ident.namespace.is_none() {
            let (core, nullable) = match base.strip_suffix('?') {
                Some(b) => (b, true),
                None => (base.as_str(), false),
            };
            if vars.iter().any(|v| v == core) {
                let v = Type::Var(Arc::from(core));
                return if nullable {
                    Type::Nullable(Box::new(v))
                } else {
                    v
                };
            }
        }
        return Type::from_annotation_name(&base);
    }
    // `Block(args… ^Ret)`: any arity (zero included — `Block()`); no `^`-tail
    // means an `Any` return (docs/internal/GENERICS_ARCH.md §11).
    if base == "Block" {
        return Type::BlockOf {
            params: tr
                .args
                .iter()
                .map(|a| type_from_ref_with_vars(a, vars))
                .collect(),
            ret: Box::new(
                tr.ret
                    .as_ref()
                    .map(|r| type_from_ref_with_vars(r, vars))
                    .unwrap_or(Type::Any),
            ),
        };
    }
    match (base.as_str(), tr.args.len()) {
        ("List", 1) => Type::ListOf(Box::new(type_from_ref_with_vars(&tr.args[0], vars))),
        ("Set", 1) => Type::SetOf(Box::new(type_from_ref_with_vars(&tr.args[0], vars))),
        ("Map", 2) => Type::MapOf(Box::new(type_from_ref_with_vars(&tr.args[1], vars))),
        _ => Type::from_annotation_name(&base),
    }
}

impl Compiler {
    pub(super) fn resolve_annotation(&mut self, tr: &TypeRefNode) -> Type {
        // A bare name that matches a declared class/mixin-header type parameter
        // is a type variable (`T?` rides the nullable suffix inside the ident,
        // like every annotation).
        if !tr.parenthesized && tr.ident.namespace.is_none() {
            let (base, nullable) = match tr.ident.name.strip_suffix('?') {
                Some(b) => (b, true),
                None => (tr.ident.name.as_str(), false),
            };
            if self.declared_type_var(base) {
                let v = Type::Var(Arc::from(base));
                return if nullable {
                    Type::Nullable(Box::new(v))
                } else {
                    v
                };
            }
        }
        if tr.parenthesized {
            let base = ident_name(&tr.ident);
            // The `^`-marked return tail is block-type syntax only
            // (`Block(Integer ^Boolean)`, GENERICS_ARCH.md §11).
            if tr.ret.is_some() && base != "Block" {
                self.warn(
                    "annotation",
                    format!(
                        "`^` return types belong to `Block(…)` annotations; `{base}` \
                         takes plain type arguments"
                    ),
                    tr.ident.source_info.as_ref(),
                );
            }
            match (base.as_str(), tr.args.len()) {
                // Any arity, zero included (`Block()` = no args, `Any` return).
                ("Block", _) => {}
                ("List", 1) | ("Set", 1) => {}
                ("Map", 2) => {
                    let key = annotation_name(&tr.args[0]);
                    if key != "String" {
                        self.warn(
                            "annotation",
                            format!(
                                "Map keys are String (got `Map({} …)`); only the value \
                                 type is generic for now",
                                key
                            ),
                            tr.ident.source_info.as_ref(),
                        );
                    }
                }
                ("List", n) | ("Set", n) => {
                    self.warn(
                        "annotation",
                        format!("`{base}` takes 1 type argument, got {n}"),
                        tr.ident.source_info.as_ref(),
                    );
                }
                ("Map", n) => {
                    self.warn(
                        "annotation",
                        format!("`Map` takes 2 type arguments (`Map(String V)`), got {n}"),
                        tr.ident.source_info.as_ref(),
                    );
                }
                _ => {
                    self.warn(
                        "annotation",
                        format!("type `{base}` does not take generic arguments"),
                        tr.ident.source_info.as_ref(),
                    );
                }
            }
            for a in &tr.args {
                // Resolve arguments for their own diagnostics (unknown names etc.).
                let _ = self.resolve_annotation(a);
            }
            if let Some(r) = &tr.ret {
                let _ = self.resolve_annotation(r);
            }
            return type_from_ref_with_vars(tr, &self.ctx_type_params());
        }
        let ty = Type::from_annotation_name(&ident_name(&tr.ident));
        // `T?` is unknown iff its base `T` is unknown.
        let base = match &ty {
            Type::Nullable(inner) => inner.as_ref(),
            other => other,
        };
        if let Type::Instance(class) = base
            && !self.seen_types.contains(class)
        {
            self.warn(
                "unknown-type",
                format!("unknown type `{}`", class),
                tr.ident.source_info.as_ref(),
            );
        }
        ty
    }

    /// Is `name` a type parameter declared by any enclosing class/mixin header?
    fn declared_type_var(&self, name: &str) -> bool {
        self.class_ctx
            .iter()
            .any(|c| c.type_params.iter().any(|p| p == name))
    }

    /// Every type parameter in scope (all enclosing class/mixin headers).
    pub(super) fn ctx_type_params(&self) -> Vec<String> {
        self.class_ctx
            .iter()
            .flat_map(|c| c.type_params.iter().cloned())
            .collect()
    }

    /// The element-tag *requirement* a generic param annotation places on
    /// dispatch: `|l: List(Integer)|` matches only Integer-tagged lists
    /// (GENERICS_ARCH.md §5). `None` = no requirement (bare or unenforceable).
    pub(super) fn param_elem_tag(&mut self, tr: &TypeRefNode) -> Option<ElemTag> {
        if tr.args.is_empty() {
            return None;
        }
        let inner = match (ident_name(&tr.ident).as_str(), tr.args.len()) {
            ("List", 1) | ("Set", 1) => &tr.args[0],
            ("Map", 2) => &tr.args[1],
            _ => return None,
        };
        self.enforceable_elem_tag_of_ref(inner)
    }

    /// Is this decl a collection literal whose declared type is generic —
    /// the tagged-literal construction pattern?
    pub(super) fn generic_literal_decl(expected: &Type, rvalue: &Node) -> bool {
        matches!(
            (expected, &rvalue.value),
            (Type::ListOf(_), NodeValue::List(_))
                | (Type::MapOf(_), NodeValue::Map(_))
                | (Type::SetOf(_), NodeValue::Set(_))
        )
    }

    /// `enforceable_elem_tag_of_ref`, but from a resolved `Type` (the decl
    /// path, where the annotation is already resolved). Same honesty rules.
    pub(super) fn enforceable_elem_tag_of_type(
        &mut self,
        inner: &Type,
        decl: &DeclarationNode,
    ) -> Option<ElemTag> {
        match ElemTag::from_type(inner) {
            Some(tag) => Some(tag),
            None => {
                let base = match inner {
                    Type::ListOf(_) => Some(ElemTag::List),
                    Type::MapOf(_) => Some(ElemTag::Map),
                    Type::SetOf(_) => Some(ElemTag::Set),
                    _ => None, // Var/Any/…: checker-only, no tag
                };
                if let Some(b) = base {
                    self.warn(
                        "annotation",
                        format!(
                            "nested element types are checker-only; `{}` is enforced as \
                             `{}` at runtime",
                            inner.name(),
                            b.name(),
                        ),
                        decl.rvalue.source_info.as_ref(),
                    );
                }
                base
            }
        }
    }

    /// The runtime-enforceable tag for an element annotation, with the
    /// guarantee-honesty degradations: a nested generic degrades to its base
    /// (with a warning — `List(List(Integer))` is enforced as `List(List)`),
    /// and a type variable or `Any` yields no tag at all (checker-only).
    fn enforceable_elem_tag_of_ref(&mut self, tr: &TypeRefNode) -> Option<ElemTag> {
        if tr.args.is_empty() && tr.ident.namespace.is_none() {
            let base = tr.ident.name.strip_suffix('?').unwrap_or(&tr.ident.name);
            if self.declared_type_var(base) {
                return None;
            }
        }
        let t = type_from_ref(tr);
        match ElemTag::from_type(&t) {
            Some(tag) => Some(tag),
            None => {
                let base = match t {
                    Type::ListOf(_) => Some(ElemTag::List),
                    Type::MapOf(_) => Some(ElemTag::Map),
                    Type::SetOf(_) => Some(ElemTag::Set),
                    _ => None,
                };
                if let Some(b) = base {
                    self.warn(
                        "annotation",
                        format!(
                            "nested element types are checker-only; `{}` is enforced as \
                             `{}` at runtime",
                            annotation_name(tr),
                            b.name(),
                        ),
                        tr.ident.source_info.as_ref(),
                    );
                }
                base
            }
        }
    }

    /// The runtime dispatch signature for a param annotation: generic arguments
    /// erase to the base class (the tag requirement rides separately in
    /// `param_elem_tags`), and a declared
    /// type variable erases to `Object` (variables never dispatch;
    /// GENERICS_ARCH.md §4.4/§5).
    pub(super) fn dispatch_type_name(&self, tr: &TypeRefNode) -> String {
        if tr.args.is_empty() && tr.ident.namespace.is_none() {
            let base = tr.ident.name.strip_suffix('?').unwrap_or(&tr.ident.name);
            if self.declared_type_var(base) {
                return "Object".to_string();
            }
        }
        ident_name(&tr.ident)
    }
}
