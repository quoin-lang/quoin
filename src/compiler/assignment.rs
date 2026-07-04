//! Assignment, declaration, and lvalue/destructuring codegen.

use super::*;

impl Compiler {
    fn collect_lvalue_names(&self, lvalues: &[Arc<Node>], names: &mut Vec<String>) {
        for lval in lvalues {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let id = &ident_lval.identifier;
                    if id.namespace.is_none()
                        && id.identifier_type != IdentifierType::Namespaced
                        && id.identifier_type != IdentifierType::Instance
                    {
                        names.push(id.name.clone());
                    }
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let id = &splat_lval.identifier;
                    if id.namespace.is_none()
                        && id.identifier_type != IdentifierType::Namespaced
                        && id.identifier_type != IdentifierType::Instance
                    {
                        names.push(id.name.clone());
                    }
                }
                NodeValue::SubLValue(sub_lval) => {
                    self.collect_lvalue_names(&sub_lval.lvalues, names);
                }
                _ => {}
            }
        }
    }

    pub(super) fn compile_assignment(
        &mut self,
        assign: &AssignmentNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        if assign.lvalues.is_empty() {
            return Err("Assignment requires at least one target lvalue".to_string());
        }

        // Strict mode: assignment never declares. Plain-local targets must already be in
        // scope (compile_ident_store errors otherwise); a new local is introduced with
        // `var`/`let` (compile_declaration). Globals (`Foo`) and fields (`@x`) are handled
        // per-target in compile_ident_store and are unaffected by this rule.

        // Phase 3a: a reassignment to a *typed* local is checked (and numeric literals promoted)
        // against its declared type — the var's contract. An untyped/unrecorded target resolves to
        // `Any`, so `compile_expecting` compiles it unchecked. Destructuring targets are untyped.
        if let [lval] = assign.lvalues.as_slice()
            && let NodeValue::IdentLValue(l) = &lval.value
            && let Some(expected) = self.declared_type(&l.identifier.name)
        {
            self.compile_expecting(&assign.rvalue, &expected, bytecode)?;
            // Phase 3c: the local now holds the rvalue's type — flow-update its narrowing (a
            // concrete type re-narrows; `Any` widens to gradual). Declared targets only, so the
            // optimizer's inferred type for an untyped `var` is never shadowed.
            let rt = self.static_type(&assign.rvalue);
            self.update_narrowing(NarrowKey::Local(l.identifier.name.clone()), rt);
        } else {
            self.compile_node(&assign.rvalue, bytecode)?;
        }

        if assign.lvalues.len() == 1 {
            let lval = &assign.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_lvalue_store(lval, bytecode, false)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(Symbol::intern(
                &(temp_var.clone()),
            )));
            self.compile_destruct(&assign.lvalues, &temp_var, bytecode, false)?;
        }

        Ok(())
    }

    pub(super) fn compile_declaration(
        &mut self,
        decl: &DeclarationNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        if decl.lvalues.is_empty() {
            return Err("declaration requires at least one target".to_string());
        }
        let mutable = matches!(decl.kind, DeclKind::Var);

        // `var`/`let` declares plain locals only.
        self.validate_decl_targets(&decl.lvalues)?;

        // Introduce the fresh bindings BEFORE compiling the initializer, so a recursive
        // reference resolves — `var f = { … f … }` (a self-recursive block) must see its
        // own name. The name binds in the enclosing env the closure captures; the actual
        // store runs after the value is built, so the captured frame is populated by the
        // time the closure is invoked. (Same-scope redeclaration is an error.)
        let mut names = Vec::new();
        self.collect_lvalue_names(&decl.lvalues, &mut names);
        for name in &names {
            self.declare_local(name, mutable)?;
        }

        // Phase 3a: an annotated `var x: T = expr` resolves `T` (flagging an unknown type) and
        // checks/promotes the initializer against it; un-annotated decls compile plainly.
        let annotated = decl.type_hint.as_ref().map(|h| self.resolve_annotation(h));
        match &annotated {
            Some(expected) => self.compile_expecting(&decl.rvalue, expected, bytecode)?,
            None => self.compile_node(&decl.rvalue, bytecode)?,
        }

        // Record the local's type for the checker + devirt. The annotation is authoritative (and
        // matches a promoted initializer); otherwise infer `Int`/`List` from the initializer —
        // both devirt paths have a runtime fallback, so a stale inferred type is harmless. `Bool`
        // is excluded: the `if:else:` inline for a statically-`Bool` `var` has no fallback, so a
        // reassigned `var` could go stale.
        if decl.lvalues.len() == 1
            && let NodeValue::IdentLValue(l) = &decl.lvalues[0].value
        {
            match &annotated {
                // An explicit annotation is the local's declared type (the reassignment contract).
                // `Bool` is excluded — its `if:else:` inline has no fallback for a stale `var`.
                Some(t) if *t != Type::Bool && *t != Type::Any => {
                    self.record_declared_type(&l.identifier.name, t.clone());
                }
                Some(_) => {}
                None => {
                    // No annotation: record the initializer's type as a *devirt hint* (not a
                    // contract — an untyped `var` may be reassigned to any type; every devirt op
                    // has a runtime fallback, so a stale hint is harmless). Only types with a
                    // devirtualized op path are worth recording — a hint no gate consumes is dead.
                    //
                    // The match is deliberately exhaustive: adding a devirtualized type (a new
                    // `*_devirt_op`) must move its variant here, and a new `Type` variant won't
                    // compile until it's classified — so this can't be silently overlooked.
                    let ty = self.static_type(&decl.rvalue);
                    let has_devirt_path = match &ty {
                        Type::Int | Type::Double | Type::List => true,
                        // `Bool` is excluded even though `if:else:` inlines it — that inline has no
                        // runtime fallback, so a stale `Bool` hint would be unsound.
                        Type::Bool
                        | Type::String
                        | Type::Nil
                        | Type::Map
                        | Type::Set
                        | Type::Block
                        | Type::Instance(_)
                        | Type::Nullable(_)
                        | Type::Any
                        | Type::Never => false,
                    };
                    if has_devirt_path {
                        self.record_local_type(&l.identifier.name, ty);
                    }
                }
            }
        }

        if decl.lvalues.len() == 1 {
            let lval = &decl.lvalues[0];
            bytecode.push(Instruction::Dup);
            self.compile_lvalue_store(lval, bytecode, true)?;
        } else {
            let temp_var = self.new_temp_var();
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(temp_var.clone());
            bytecode.push(Instruction::Dup);
            bytecode.push(Instruction::DefineLocal(Symbol::intern(
                &(temp_var.clone()),
            )));
            self.compile_destruct(&decl.lvalues, &temp_var, bytecode, true)?;
        }

        Ok(())
    }

    /// A `var`/`let` target must be a plain local (or `_` / splat / nested thereof) — not a
    /// global (`Foo`), an instance variable (`@x`), or a namespaced name.
    fn validate_decl_targets(&self, lvalues: &[Arc<Node>]) -> Result<(), String> {
        for lval in lvalues {
            match &lval.value {
                NodeValue::IdentLValue(l) => self.validate_decl_ident(&l.identifier)?,
                NodeValue::SplatLValue(l) => self.validate_decl_ident(&l.identifier)?,
                NodeValue::IgnoredLValue | NodeValue::IgnoredSplatLValue => {}
                NodeValue::SubLValue(s) => self.validate_decl_targets(&s.lvalues)?,
                other => return Err(format!("unsupported `var`/`let` target: {:?}", other)),
            }
        }
        Ok(())
    }

    fn validate_decl_ident(&self, id: &IdentifierNode) -> Result<(), String> {
        if id.identifier_type == IdentifierType::Instance {
            return Err(format!(
                "`var`/`let` cannot declare an instance variable (`@{}`); \
                 declare instance variables in the class header",
                id.name
            ));
        }
        if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
            return Err(format!(
                "`var`/`let` cannot declare a namespaced name (`{}`)",
                id.name
            ));
        }
        if id
            .name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        {
            return Err(format!(
                "`var`/`let` declares locals; `{}` is uppercase — globals/classes use `{} = …`",
                id.name, id.name
            ));
        }
        Ok(())
    }

    fn compile_lvalue_store(
        &mut self,
        lval: &Node,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        match &lval.value {
            NodeValue::IdentLValue(ident_lval) => {
                let id = &ident_lval.identifier;
                if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::StoreGlobal(ns_name, false));
                } else {
                    let name = &id.name;
                    self.compile_ident_store(&id.identifier_type, name, bytecode, declaring)?;
                }
            }
            NodeValue::IgnoredLValue => {
                bytecode.push(Instruction::Pop);
            }
            NodeValue::IgnoredSplatLValue => {
                bytecode.push(Instruction::Pop);
            }
            _ => return Err(format!("Unsupported store target: {:?}", lval.value)),
        }
        Ok(())
    }

    fn compile_ident_store(
        &mut self,
        ident_type: &IdentifierType,
        name: &String,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        // A `var`/`let` declaration introduces a fresh binding. The target was
        // validated as a plain local and inserted into the current scope by
        // `compile_declaration`, so here we just emit the binding instruction.
        if declaring {
            bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
            return Ok(());
        }
        // Reserved identifiers parse as assignable lvalues (`true = false`); emit a store
        // so the runtime raises "Can't modify reserved identifier" (unchanged behavior),
        // rather than the compile-time "undeclared local" error below.
        if matches!(name.as_str(), "true" | "false" | "nil") {
            bytecode.push(Instruction::StoreLocal(Symbol::intern(&(name.clone()))));
            return Ok(());
        }
        let first_char = name.chars().next().unwrap_or('\0');
        if first_char.is_ascii_uppercase() {
            let ns_name = NamespacedName::new(Vec::new(), name.clone());
            bytecode.push(Instruction::StoreGlobal(ns_name, false));
        } else if ident_type == &IdentifierType::Instance {
            if self.value_type_def_depth > 0 {
                return Err(format!(
                    "value types cannot have instance variables (found '@{}')",
                    name
                ));
            }
            bytecode.push(Instruction::StoreField(name.clone()));
        } else if self.is_local(name) {
            if self.is_immutable(name) {
                return Err(format!("cannot reassign `let` binding `{}`", name));
            }
            bytecode.push(Instruction::StoreLocal(Symbol::intern(&(name.clone()))));
        } else if self.scopes.last().map(|s| s.is_init).unwrap_or(false) {
            // Inside an object-initializer block (`X.new:{ … }`), a bare `field = value`
            // binds an instance field — no `var` needed. The instantiating frame binds it
            // into the new object at runtime.
            bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
        } else {
            return Err(format!(
                "undeclared local `{}` — declare it with `var {} = …` \
                 (assignment no longer implicitly declares locals)",
                name, name
            ));
        }
        Ok(())
    }

    fn compile_destruct(
        &mut self,
        lvalues: &[Arc<Node>],
        temp_var: &str,
        bytecode: &mut CodeBlock,
        declaring: bool,
    ) -> Result<(), String> {
        for (i, lval) in lvalues.iter().enumerate() {
            match &lval.value {
                NodeValue::IdentLValue(ident_lval) => {
                    let name = &ident_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("at:"), 1));

                    self.compile_ident_store(
                        &ident_lval.identifier.identifier_type,
                        name,
                        bytecode,
                        declaring,
                    )?;
                }
                NodeValue::SplatLValue(splat_lval) => {
                    let name = &splat_lval.identifier.name;
                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("sliceFrom:"), 1));

                    self.compile_ident_store(
                        &splat_lval.identifier.identifier_type,
                        name,
                        bytecode,
                        declaring,
                    )?;
                }
                NodeValue::IgnoredLValue => {}
                NodeValue::IgnoredSplatLValue => {}
                NodeValue::SubLValue(sub_lval) => {
                    let nested_temp = self.new_temp_var();
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .locals
                        .insert(nested_temp.clone());

                    bytecode.push(Instruction::LoadLocal(Symbol::intern(
                        &(temp_var.to_string()),
                    )));
                    bytecode.push(Instruction::Push(Constant::Int(i as i64)));
                    bytecode.push(Instruction::Send(Symbol::intern("at:"), 1));
                    bytecode.push(Instruction::DefineLocal(Symbol::intern(
                        &(nested_temp.clone()),
                    )));

                    self.compile_destruct(&sub_lval.lvalues, &nested_temp, bytecode, declaring)?;
                }
                _ => {
                    return Err(format!(
                        "Unsupported destructuring element: {:?}",
                        lval.value
                    ));
                }
            }
        }
        Ok(())
    }
}
