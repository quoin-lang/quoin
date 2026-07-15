//! The lowering core: node dispatch, method-call/operator/interpolation/block
//! compilation, and selector reconstruction. Extends `Compiler` exactly like the
//! other satellites (which it delegates into: `devirt`, `inlining`, `assignment`).

use super::*;

impl Compiler {
    pub(super) fn compile_node_internal(
        &mut self,
        node: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        match &node.value {
            NodeValue::Integer(n) => {
                bytecode.push(Instruction::Push(Constant::Int(n.value)));
            }
            NodeValue::Double(d) => {
                bytecode.push(Instruction::Push(Constant::Double(d.value)));
            }
            NodeValue::Str(s) => {
                bytecode.push(Instruction::Push(Constant::String(s.value.clone())));
            }
            NodeValue::Symbol(s) => {
                bytecode.push(Instruction::Push(Constant::Symbol(s.value.clone())));
            }
            NodeValue::Identifier(id) => {
                if id.identifier_type == IdentifierType::Instance {
                    if self.value_type_def_depth > 0 {
                        return Err(format!(
                            "value types cannot have instance variables (found '@{}')",
                            id.name
                        ));
                    }
                    // Phase 5·3c: inside a spliced computed body, `@x` reads the override receiver's
                    // field, not the caller's `self`.
                    if let Some(over) = self.self_override {
                        bytecode.push(Instruction::LoadLocal(over));
                        bytecode.push(Instruction::LoadFieldOf(id.name.clone()));
                    } else {
                        bytecode.push(Instruction::LoadField(id.name.clone()));
                    }
                } else if id.name == "nil" || id.name == "true" || id.name == "false" {
                    match id.name.as_str() {
                        "nil" => bytecode.push(Instruction::Push(Constant::Nil)),
                        "true" => bytecode.push(Instruction::Push(Constant::Bool(true))),
                        "false" => bytecode.push(Instruction::Push(Constant::Bool(false))),
                        _ => unreachable!(),
                    }
                } else if let Some(&sym) = self.param_override.get(&id.name) {
                    // Phase 5·4: inside a spliced body, a param reference loads its bound-arg temp.
                    bytecode.push(Instruction::LoadLocal(sym));
                } else if id.namespace.is_some() || id.identifier_type == IdentifierType::Namespaced
                {
                    let ns_name = NamespacedName::from_ast(id);
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                } else if self.is_local(&id.name) {
                    // Phase 5·3c: inside a spliced computed body, a bare `self` is the override.
                    let sym = match self.self_override {
                        Some(over) if id.name == "self" => over,
                        _ => self.local_symbol(&id.name),
                    };
                    bytecode.push(Instruction::LoadLocal(sym));
                } else {
                    let ns_name = NamespacedName::new(Vec::new(), id.name.clone());
                    bytecode.push(Instruction::LoadGlobal(ns_name));
                }
            }
            NodeValue::Assignment(assign) => {
                self.compile_assignment(assign, bytecode)?;
            }
            NodeValue::Declaration(decl) => {
                self.compile_declaration(decl, bytecode)?;
            }
            NodeValue::MethodCall(call) => {
                self.compile_method_call(call, bytecode)?;
            }
            NodeValue::BinaryOperator(op) => {
                self.compile_binary_operator(op, bytecode)?;
            }
            NodeValue::UnaryOperator(op) => {
                self.compile_unary_operator(op, bytecode)?;
            }
            NodeValue::Block(block) => {
                self.next_block_is_expression = true;
                self.compile_block(block, bytecode)?;
                // B3a: a block LITERAL is a block-template candidate (method
                // bodies are collected as Method candidates at their def site).
                if let Some(Instruction::Push(Constant::Block(rc))) = bytecode.bytecode.last() {
                    let rc = rc.clone();
                    self.maybe_collect_block_candidate(&rc);
                }
            }
            NodeValue::BlockReturn(ret) => {
                self.compile_return_value(&ret.value, bytecode)?;
                // Inside an inlined control-flow block (Slice 2d), `^expr` yields the
                // block's value and jumps past the inlined region rather than popping a
                // (now-absent) block frame; `inline_block_body` patches the placeholder.
                if let Some(positions) = self.inline_carets.as_mut() {
                    positions.push(bytecode.len());
                    bytecode.push(Instruction::Jump(0));
                } else {
                    // G4b: a real `^` is one of the enclosing block's return values —
                    // join it into the return harvest (§11.3). An inlined-region `^`
                    // (above) is the *conditional's* value, not a block return.
                    let t = self.static_type(&ret.value);
                    if let Some(h) = self.block_ret_harvest.last_mut() {
                        let joined = h.join(&t);
                        *h = joined;
                    }
                    bytecode.push(Instruction::BlockReturn);
                }
            }
            NodeValue::MethodReturn(ret) => {
                self.compile_return_value(&ret.value, bytecode)?;
                bytecode.push(Instruction::MethodReturn);
            }
            NodeValue::YieldReturn(ret) => {
                // `^> expr` is sugar for `Fiber.yield:expr`: suspend the current
                // fiber, hand `expr` out to the resumer, and evaluate to whatever
                // the next `resume:` passes back in.
                bytecode.push(Instruction::LoadGlobal(NamespacedName::new(
                    Vec::new(),
                    "Fiber".to_string(),
                )));
                self.compile_node(&ret.value, bytecode)?;
                bytecode.push(Instruction::Send(Symbol::intern("yield:"), 1));
            }
            NodeValue::List(list) => {
                for item in &list.values {
                    self.compile_node(item, bytecode)?;
                }
                bytecode.push(Instruction::NewList(list.values.len()));
            }
            NodeValue::Map(map) => {
                if map.keys.len() != map.values.len() {
                    return Err("Map keys and values count mismatch".to_string());
                }
                for i in 0..map.keys.len() {
                    self.compile_node(&map.keys[i], bytecode)?;
                    self.compile_node(&map.values[i], bytecode)?;
                }
                bytecode.push(Instruction::NewMap(map.keys.len()));
            }
            NodeValue::Set(set) => {
                for item in &set.values {
                    self.compile_node(item, bytecode)?;
                }
                bytecode.push(Instruction::NewSet(set.values.len()));
            }
            NodeValue::Regex(re) => {
                let mut pattern = re.value.clone();
                if pattern.starts_with("#/") && pattern.ends_with('/') {
                    pattern = pattern[2..pattern.len() - 1].to_string();
                }
                bytecode.push(Instruction::Push(Constant::String(pattern)));
                bytecode.push(Instruction::NewRegex);
            }
            NodeValue::ClassDefinition(class_def) => {
                let name = NamespacedName::from_ast(&class_def.identifier);
                // Checker/class-table key: the qualified form (`[Web]Halt`), matching the
                // `populate_from_vm` keying so AST- and VM-sourced sigs can't diverge.
                let class_name = name.to_string();
                // Record the class as known as soon as it's defined — covers classes in nested
                // blocks the top-level pre-scan can't reach (a def-before-use in any scope).
                self.seen_types.insert(&class_name);
                self.class_table
                    .insert(&class_name, self.class_sig_from_def(class_def));
                self.check_return_covariance(&class_name, &class_def.block);
                let parent_name = class_def
                    .parent_identifier
                    .as_ref()
                    .map(|id| NamespacedName::from_ast(id));
                let mut instance_vars = Vec::new();
                for arg in &class_def.block.arguments {
                    instance_vars.push(arg.identifier.name.clone());
                }
                let is_value_type = matches!(
                    class_name.as_str(),
                    "Integer" | "Double" | "Boolean" | "Nil"
                );
                if is_value_type && !instance_vars.is_empty() {
                    return Err(format!(
                        "value type '{}' cannot declare instance variables (@{})",
                        class_name, instance_vars[0]
                    ));
                }
                bytecode.push(Instruction::DefineClass {
                    name,
                    parent_name,
                    instance_vars,
                    source: node.source_info.clone(),
                });
                if is_value_type {
                    self.value_type_def_depth += 1;
                }
                let ctx = self.collect_class_ctx(
                    &class_name,
                    &class_def.block,
                    class_def.type_params.clone(),
                );
                self.class_ctx.push(ctx);
                let r = self.compile_block(&class_def.block, bytecode);
                self.class_ctx.pop();
                if is_value_type {
                    self.value_type_def_depth -= 1;
                }
                r?;
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::ClassExtension(class_ext) => {
                // A `Foo <-- {}` reopen contributes its methods' declared returns AND params to
                // `Foo`'s signature — how the core classes (`Object <-- {}`, `List <-- {}`, …)
                // carry their contracts, since they're reopened rather than defined with `<-`
                // (Phase 3c·4; params for the G4b expectation channel, §11.3/§11.4). The
                // target's declared type parameters come from the table (a reopen header can't
                // declare them), so `^Set(T)` records as `Var("T")`, not a bogus `Instance`.
                if let NodeValue::Identifier(target) = &class_ext.expression.value {
                    let target_name = ident_name(target);
                    let vars: Vec<String> = self
                        .class_table
                        .type_params_of(&target_name)
                        .iter()
                        .map(|p| p.to_string())
                        .collect();
                    self.class_table.add_returns(
                        &target_name,
                        self.declared_method_returns_with_vars(&class_ext.block, &vars),
                    );
                    self.class_table.add_params(
                        &target_name,
                        self.declared_method_params(&class_ext.block, &vars),
                    );
                    // A reopen's `.mix:` runs at runtime, after the from_vm snapshot — record
                    // it now or the hierarchy walk can't reach the mixin's typed signatures.
                    let mixins: Vec<Arc<str>> = class_ext
                        .block
                        .statements
                        .iter()
                        .filter_map(|stmt| match &stmt.value {
                            NodeValue::MethodCall(call) => {
                                Self::mixin_target(call).map(|m| Arc::from(m.as_str()))
                            }
                            _ => None,
                        })
                        .collect();
                    self.class_table.add_mixins(&target_name, mixins);
                    self.check_return_covariance(&target_name, &class_ext.block);
                }
                self.compile_node(&class_ext.expression, bytecode)?;
                let is_value_type = Self::is_value_type_target(&class_ext.expression);
                if is_value_type {
                    if let Some(arg) = class_ext
                        .block
                        .arguments
                        .iter()
                        .find(|a| a.identifier.identifier_type == IdentifierType::Instance)
                    {
                        return Err(format!(
                            "value type cannot declare instance variables (@{})",
                            arg.identifier.name
                        ));
                    }
                    self.value_type_def_depth += 1;
                }
                let ext_name = match &class_ext.expression.value {
                    NodeValue::Identifier(id) => ident_name(id),
                    _ => String::new(),
                };
                let ext_params: Vec<String> = self
                    .class_table
                    .type_params_of(&ext_name)
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                let ctx = self.collect_class_ctx(&ext_name, &class_ext.block, ext_params);
                self.class_ctx.push(ctx);
                let r = self.compile_block(&class_ext.block, bytecode);
                self.class_ctx.pop();
                if is_value_type {
                    self.value_type_def_depth -= 1;
                }
                r?;
                if let NodeValue::Identifier(id) = &class_ext.expression.value
                    && let Some(si) = node.source_info.clone()
                {
                    bytecode.push(Instruction::RecordClassSite {
                        name: NamespacedName::from_ast(id),
                        source: si,
                    });
                }
                bytecode.push(Instruction::ExecuteBlockWithSelf);
            }
            NodeValue::MethodDefinition(method_def) => {
                let selector = self.reconstruct_selector(&method_def.signature)?;
                self.reject_top_level_method(&selector)?;
                self.compile_block(&method_def.block, bytecode)?;
                self.maybe_collect_aot_candidate(&selector, &method_def.block, bytecode);
                bytecode.push(Instruction::DefineMethod(selector));
            }
            NodeValue::MethodExtension(method_ext) => {
                let selector = self.reconstruct_selector(&method_ext.signature)?;
                self.reject_top_level_method(&selector)?;
                self.compile_block(&method_ext.block, bytecode)?;
                bytecode.push(Instruction::OverrideMethod(selector));
            }
            NodeValue::ConstDefinition(const_def) => {
                let ns_name = NamespacedName::from_ast(&const_def.identifier);
                self.compile_node(&const_def.rvalue, bytecode)?;
                bytecode.push(Instruction::Dup);
                bytecode.push(Instruction::StoreGlobal(ns_name, true));
            }
            NodeValue::Use(use_node) => {
                bytecode.push(Instruction::Use {
                    package: use_node.package.clone(),
                    path: use_node.path.clone(),
                    glob: use_node.glob,
                });
            }
            NodeValue::UserString(user_str) => {
                let ns_name = NamespacedName::from_ast(&user_str.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                bytecode.push(Instruction::Push(Constant::String(user_str.value.clone())));
                bytecode.push(Instruction::Send(Symbol::intern("newUserString:"), 1));
            }
            NodeValue::UserList(user_list) => {
                let ns_name = NamespacedName::from_ast(&user_list.identifier);
                bytecode.push(Instruction::LoadGlobal(ns_name));
                for val in &user_list.values {
                    self.compile_node(val, bytecode)?;
                }
                bytecode.push(Instruction::NewList(user_list.values.len()));
                bytecode.push(Instruction::Send(Symbol::intern("newUserList:"), 1));
            }
            // The placeholder statements (statement-position only, by grammar).
            // Each desugars to ordinary sends, so traces, typed `catch:`, the
            // DAP stderr capture, and AOT outcalls all just work.
            NodeValue::Dot3 => {
                // `...` — "not written yet", the todo!() of Quoin: throws a
                // typed NotImplementedError (bootstrap.qn).
                bytecode.push(Instruction::LoadGlobal(NamespacedName::new(
                    Vec::new(),
                    "NotImplementedError".to_string(),
                )));
                bytecode.push(Instruction::Push(Constant::String(
                    "not implemented".to_string(),
                )));
                bytecode.push(Instruction::Send(Symbol::intern("throw:"), 1));
            }
            NodeValue::Bang3 => {
                // `!!!` — "can NEVER execute": throws a typed UnreachableError.
                bytecode.push(Instruction::LoadGlobal(NamespacedName::new(
                    Vec::new(),
                    "UnreachableError".to_string(),
                )));
                bytecode.push(Instruction::Push(Constant::String(
                    "reached unreachable code".to_string(),
                )));
                bytecode.push(Instruction::Send(Symbol::intern("throw:"), 1));
            }
            NodeValue::Huh3 => {
                // `???` — "shouldn't get here, but keep going": a plain
                // `Log.warn:` send, then nil (the statement's value). Nothing
                // bespoke: the entry's `file:line:col` arrives through Log's
                // uniform caller-location capture (this send site IS the
                // caller), warn coloring through Log's default sink, and
                // `Log.level:` / `Log.sink:` govern it like any other warning.
                bytecode.push(Instruction::LoadGlobal(NamespacedName::new(
                    Vec::new(),
                    "Log".to_string(),
                )));
                bytecode.push(Instruction::Push(Constant::String(
                    "reached `???` placeholder".to_string(),
                )));
                bytecode.push(Instruction::Send(Symbol::intern("warn:"), 1));
            }
            NodeValue::Unknown => {
                return Err("Encountered Unknown NodeValue (ast_visitor bug)".to_string());
            }
            _ => {
                return Err(format!("Unsupported NodeValue: {:?}", node.value));
            }
        }
        Ok(())
    }

    fn compile_method_call(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Phase 3b: compile-time MNU (a pure analysis, before any inlining/lowering).
        self.check_mnu(call);
        // The multimethod face of MNU: args that provably match no recorded variant.
        self.check_variant_mismatch(call);
        // Phase 3c: a non-nil-safe send to a confidently-nullable, un-narrowed receiver.
        self.check_nil_misuse(call);
        self.check_generic_insertion(call);
        // Portability: a block literal shipped by a boundary send registers for
        // the shape scan once its template exists (`classify_block_literal`).
        self.note_boundary_send(call);
        let args = &call.arguments;
        // A self-send (no explicit receiver, or an explicit `self`) — eligible for
        // devirtualization when the enclosing class is sealed (see `emit_call`).
        let is_self = match &call.subject {
            None => true,
            Some(s) => matches!(&s.value, NodeValue::Identifier(id) if id.name == "self"),
        };

        // Slice 2d: inline `if:`/`if:else:` on a statically-Bool receiver with literal,
        // 0-arg, declaration-free block args into native jumps — no block allocation, no
        // dispatch, no block-invocation frame. Falls through to a normal send otherwise.
        if self.try_compile_inlined_conditional(call, bytecode)? {
            return Ok(());
        }
        if self.try_compile_inlined_while(call, bytecode)? {
            return Ok(());
        }
        // B1 (docs/internal/BLOCK_AOT_ARCH.md §3): fuse `recv.each:{ |x| … }` into a guarded
        // native index loop — closure-free per element on any native-List receiver,
        // with the real send as the cold path (the guard IS the dispatch).
        if self.try_compile_inlined_each(call, bytecode)? {
            return Ok(());
        }
        // M2 (docs/internal/MATERIALIZATION_ARCH.md): fuse `X.new:{ f=e; … }` on the plain-config
        // shape into a guarded inline instantiation — no config closure, no config frame,
        // no interpreted stores, with the real send as the cold path.
        if self.try_compile_fused_instantiation(call, bytecode)? {
            return Ok(());
        }
        // Phase 5·1/5·2: inline a self-send to a sealed class's own method with an inline-safe body
        // (`self.width` → the field load; `self.area` → `.width * .height`) — no receiver push, no
        // dispatch. Before the receiver is evaluated, since the inline replaces it entirely.
        if self.try_inline_self_send(call, is_self, bytecode)? {
            return Ok(());
        }
        // Phase 5·3/5·3b/5·3c: inline an explicit-receiver `v.foo` (field accessor, or a computed
        // body with `self` rebound to `v`) to a sealed in-unit class. Before the receiver push, since
        // the inline evaluates `v` itself.
        if self.try_inline_exact_receiver(call, bytecode)? {
            return Ok(());
        }

        // Evaluate receiver. Inside a spliced computed body (5·3c), a bare self-send targets the
        // override receiver, not the caller's `self`.
        if let Some(ref subject) = call.subject {
            self.compile_node(subject, bytecode)?;
        } else {
            bytecode.push(Instruction::LoadLocal(
                self.self_override.unwrap_or_else(|| Symbol::intern("self")),
            ));
        }

        // No-argument selector (unary / bang / symbol): a single component, no args.
        if args.expressions.is_empty() {
            if args.signature.identifiers.is_empty() {
                return Err("No identifiers found in method call selector".to_string());
            }
            let selector = args.signature.identifiers[0].name.clone();
            self.emit_call(bytecode, &selector, 0);
            return Ok(());
        }

        // Keyword send. Keywords and argument expressions are 1:1 here (the parser builds them in
        // lockstep). A run of the *same* consecutive keyword is a variadic group: its arguments
        // fold into one `List` and the keyword interns as `name+:`, matching a `name+:` method
        // definition. A lone keyword stays `name:`. This is resolved entirely at compile time, so
        // dispatch only ever sees a canonical interned selector — no runtime collapse.
        // Phase 3b arg-checks: when the receiver + method params are known, args are checked and
        // numeric literals promoted against them; otherwise compiled unchecked (gradual). `Some`
        // only for fully non-variadic calls, so `i + j` indexes `params` directly.
        let param_types = self.call_param_types(call);
        // G4b: the declared param types from the class-table walk, receiver-bound — feeds a
        // block-literal argument its declared `Block(…)` shape (§11.3). Computed once per call;
        // consulted only for literal block args below.
        let has_block_arg = call
            .arguments
            .expressions
            .iter()
            .any(|a| matches!(a.value, NodeValue::Block(_)));
        let block_expectations = has_block_arg
            .then(|| self.receiver_bound_param_types(call))
            .flatten();
        // Phase 3c: if this is a nil-guard conditional (`RECV.defined?.if:`/`.else:`), the per-arm
        // narrowing to install while compiling each arm, and post-guard on divergence.
        let guard = self.guard_narrowing(call);
        let idents = &args.signature.identifiers;
        debug_assert_eq!(idents.len(), args.expressions.len());
        let mut selector = String::new();
        let mut num_components = 0usize;
        // Phase 3c join/merge: each guard arm's captured exit narrowing for the guarded key.
        let mut if_exit: Option<Type> = None;
        let mut else_exit: Option<Type> = None;
        let mut i = 0;
        while i < idents.len() {
            // Extent of the run of the keyword at `i`.
            let mut run = 1;
            while i + run < idents.len() && idents[i + run].name == idents[i].name {
                run += 1;
            }
            // Evaluate this component's argument expression(s); a run folds into one list value.
            for j in 0..run {
                let arg = &args.expressions[i + j];
                // `X.new:{ … }` — the block argument is an object-initializer block, in
                // which a bare `field = value` binds an instance field (see compile_block
                // / Scope::is_init). Only a literal block gets the flag, and it's consumed
                // immediately by that block's compile_block, so it can't leak.
                if run == 1 && idents[i].name == "new" && matches!(arg.value, NodeValue::Block(_)) {
                    self.next_block_is_init = true;
                }
                // G4b: a literal block argument whose declared param is a `Block(…)` shape
                // compiles with that shape as its expectation — seeding its unannotated
                // params and closing the loop for `U`-binding (§11.3). One-shot, consumed
                // by the literal's own `compile_block`.
                if matches!(arg.value, NodeValue::Block(_))
                    && let Some(dp) = &block_expectations
                    && let Some(Type::BlockOf { params, .. }) = dp.get(i + j)
                {
                    self.next_block_expected = Some(params.clone());
                }
                // Phase 3c: narrow the guarded path inside this arm's block (`if` → non-nil arm,
                // `else` → nil arm). One-shot, consumed by the arm's `compile_block`. Also request a
                // snapshot of the arm's exit narrowing for the join/merge after the loop.
                let capture_this_arm = if let Some(g) = &guard
                    && matches!(arg.value, NodeValue::Block(_))
                    && let Some(arm_ty) = g.arm_type(&idents[i].name)
                {
                    self.next_block_narrowing = Some((g.key.clone(), arm_ty));
                    self.next_block_capture = Some(g.key.clone());
                    true
                } else {
                    false
                };
                match &param_types {
                    Some(params) => self.compile_expecting(arg, &params[i + j], bytecode)?,
                    None => self.compile_node(arg, bytecode)?,
                }
                if capture_this_arm {
                    let exit = self.captured_arm_exit.take();
                    match idents[i].name.as_str() {
                        "if" => if_exit = exit,
                        "else" => else_exit = exit,
                        _ => {}
                    }
                }
            }
            if run > 1 {
                bytecode.push(Instruction::NewList(run));
            }
            selector.push_str(&idents[i].name);
            if run > 1 {
                selector.push('+');
            }
            selector.push(':');
            num_components += 1;
            i += run;
        }

        // Phase 3c: after a guard send, merge the arms' exit states into the enclosing scope —
        // a diverging arm drops out (`x.defined?.else:{ ^^… }`), the surviving/fall-through paths
        // join. Both diverging ⇒ unreachable, no narrowing.
        if let Some(g) = &guard {
            self.apply_guard_join(call, g, if_exit, else_exit);
        }

        // Slice 2e: devirtualize `at:`/`at:put:`/`add:` when the receiver is statically a
        // `List`. The operands a send would consume are already on the stack in send order,
        // so the op is a drop-in replacement.
        if let Some(op) = self.collection_devirt_op(call, &selector, num_components) {
            bytecode.push(op);
            return Ok(());
        }

        self.emit_call(bytecode, &selector, num_components);
        Ok(())
    }

    fn compile_binary_operator(
        &mut self,
        op: &BinaryOperatorNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Phase 3c: a nil-dereferencing binop on a confidently-nullable left operand.
        self.check_binop_nil_misuse(op);

        if op.operator == BinaryOperatorType::And {
            self.compile_node(&op.left, bytecode)?;
            bytecode.push(Instruction::Dup);

            // Phase 3c: `RECV.defined? && EXPR` narrows RECV non-nil within EXPR (short-circuit).
            let restore = self.push_true_narrowing(&op.left);
            let mut right_bytecode = CodeBlock::new();
            right_bytecode.current_source = bytecode.current_source.clone();
            self.compile_node(&op.right, &mut right_bytecode)?;
            self.pop_narrowing(restore);

            let offset = 2 + right_bytecode.len() as isize;
            bytecode.push(Instruction::ElseJump(offset));
            bytecode.push(Instruction::Pop);
            bytecode.extend(right_bytecode);
            return Ok(());
        }

        if op.operator == BinaryOperatorType::Or {
            self.compile_node(&op.left, bytecode)?;
            bytecode.push(Instruction::Dup);

            let mut right_bytecode = CodeBlock::new();
            right_bytecode.current_source = bytecode.current_source.clone();
            self.compile_node(&op.right, &mut right_bytecode)?;

            let offset = 2 + right_bytecode.len() as isize;
            bytecode.push(Instruction::IfJump(offset));
            bytecode.push(Instruction::Pop);
            bytecode.extend(right_bytecode);
            return Ok(());
        }

        // Devirtualize when both operands are statically Integer: emit the direct i64 op
        // instead of a method send. Computed from the AST before compiling the operands
        // (no side effects). Integer is a sealed value type (see prelude.qn), so its
        // arithmetic operators can't be redefined — this is sound.
        // Integer and Double are sealed value types (prelude.qn), so their arithmetic operators
        // can't be redefined — devirt to a direct op when both operands are statically that same
        // type. Types computed from the AST before compiling the operands (no side effects); a
        // runtime type mismatch (stale inference) falls back to the real send.
        let (lt, rt) = (self.static_type(&op.left), self.static_type(&op.right));

        self.compile_node(&op.left, bytecode)?;
        self.compile_node(&op.right, bytecode)?;

        let devirt_op = if lt == Type::Int && rt == Type::Int {
            Self::int_devirt_op(&op.operator)
        } else if lt == Type::Double && rt == Type::Double {
            Self::double_devirt_op(&op.operator)
        } else {
            None
        };
        if let Some(op_instr) = devirt_op {
            bytecode.push(op_instr);
            return Ok(());
        }

        let selector = match op.operator {
            BinaryOperatorType::Add => "+:",
            BinaryOperatorType::Sub => "-:",
            BinaryOperatorType::Mul => "*:",
            BinaryOperatorType::Div => "/:",
            BinaryOperatorType::Eq => "==:",
            BinaryOperatorType::NotEq => "!=:",
            BinaryOperatorType::Lt => "<:",
            BinaryOperatorType::Gt => ">:",
            BinaryOperatorType::LtEq => "<=:",
            BinaryOperatorType::GtEq => ">=:",
            BinaryOperatorType::Mod => "%:",
            BinaryOperatorType::Match => "~:",
            BinaryOperatorType::Range => "..:",
            _ => {
                return Err(format!(
                    "Unsupported binary operator type: {:?}",
                    op.operator
                ));
            }
        };

        bytecode.push(Instruction::Send(Symbol::intern(selector), 1));
        Ok(())
    }

    fn compile_unary_operator(
        &mut self,
        op: &UnaryOperatorNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // `%` on a string LITERAL lowers to a `+` concatenation chain here at
        // compile time. Only a computed string (`%t`) takes the runtime
        // reflective path via the `%` send below.
        if op.operator == UnaryOperatorType::Mod
            && let NodeValue::Str(s) = &op.right.value
        {
            let template = s.value.clone();
            return self.compile_interpolated_literal(&template, &op.right, bytecode);
        }

        // Compile operand (receiver)
        self.compile_node(&op.right, bytecode)?;

        match op.operator {
            UnaryOperatorType::Bang => {
                bytecode.push(Instruction::Send(Symbol::intern("!"), 0));
            }
            UnaryOperatorType::Sub => {
                bytecode.push(Instruction::Send(Symbol::intern("-"), 0));
            }
            UnaryOperatorType::Add => {
                bytecode.push(Instruction::Send(Symbol::intern("+"), 0));
            }
            UnaryOperatorType::Mod => {
                bytecode.push(Instruction::Send(Symbol::intern("%"), 0));
            }
            _ => {
                return Err(format!(
                    "Unsupported unary operator type: {:?}",
                    op.operator
                ));
            }
        }
        Ok(())
    }

    /// Lower `%'…%{expr}…'` to `'…' + (expr) + '…'`: each fragment compiles
    /// inline in the enclosing scope, so locals resolve lexically and
    /// `@ivars` work (the runtime path reads `self` off the env chain, which
    /// its synthesized block never binds — BUGS.md-era `%{@ivar}` renders
    /// empty there). The chain is anchored on a leading String constant so it
    /// dispatches `String#+:` throughout, whose argument coercion is the same
    /// `.s` the runtime path applies.
    fn compile_interpolated_literal(
        &mut self,
        template: &str,
        lit_node: &Node,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        let parts = split_interpolation(template);
        if !parts.iter().any(|p| matches!(p, InterpPart::Expr(_))) {
            // Nothing to interpolate (including an unterminated `%{`, which
            // stays literal): `%` is the identity here, skip the send.
            return self.compile_node(lit_node, bytecode);
        }

        let si = &lit_node.source_info;
        let mk_str = |v: String| Node {
            source_info: si.clone(),
            value: NodeValue::Str(StringNode { value: v }),
        };
        let mk_add = |left: Node, right: Node| Node {
            source_info: si.clone(),
            value: NodeValue::BinaryOperator(BinaryOperatorNode {
                operator: BinaryOperatorType::Add,
                left: Arc::new(left),
                right: Arc::new(right),
            }),
        };

        let mut chain: Option<Node> = None;
        for part in parts {
            let node = match part {
                InterpPart::Lit(l) => mk_str(l),
                InterpPart::Expr(src) => self.parse_interpolation_fragment(&src, si)?,
            };
            chain = Some(match chain {
                Some(left) => mk_add(left, node),
                // Anchor on a String even when the template begins with a
                // fragment, so the whole chain is String concatenation.
                None if matches!(node.value, NodeValue::Str(_)) => node,
                None => mk_add(mk_str(String::new()), node),
            });
        }
        // `parts` contains at least the one Expr checked above.
        self.compile_node(&chain.unwrap(), bytecode)
    }

    /// Parse one `%{…}` fragment into a node that compiles in expression
    /// position. A single non-declaration statement inlines directly;
    /// anything else (multi-statement, `var`/`let`, comment-only) becomes
    /// `{ … }.value` so declarations stay fragment-local, matching the
    /// runtime path's synthesized block.
    fn parse_interpolation_fragment(
        &mut self,
        src: &str,
        si: &Option<SourceInfo>,
    ) -> Result<Node, String> {
        let at = si
            .as_ref()
            .map(|s| format!(" at {}:{}", s.filename, s.line))
            .unwrap_or_default();
        let single = |parsed: &Node| -> Option<Node> {
            let NodeValue::Program(program) = &parsed.value else {
                return None;
            };
            match program.expressions.as_slice() {
                [stmt] if !matches!(stmt.value, NodeValue::Declaration(_)) => {
                    Some(stmt.as_ref().clone())
                }
                _ => None,
            }
        };

        let parsed = crate::parser::try_parse_quoin_string_named(src, "<interpolation>")
            .map_err(|e| format!("in %{{…}} interpolation{at}: {e}"))?;
        if let Some(node) = single(&parsed) {
            return Ok(node);
        }
        // The newlines keep a trailing line comment in the fragment from
        // swallowing the wrapper.
        let wrapped = format!("{{\n{src}\n}}.value");
        let parsed = crate::parser::try_parse_quoin_string_named(&wrapped, "<interpolation>")
            .map_err(|e| format!("in %{{…}} interpolation{at}: {e}"))?;
        single(&parsed).ok_or_else(|| format!("in %{{…}} interpolation{at}: not an expression"))
    }

    pub(super) fn compile_block(
        &mut self,
        block: &BlockNode,
        bytecode: &mut CodeBlock,
    ) -> Result<(), String> {
        // Consume the one-shot init-block flag (set by `compile_method_call` for a
        // `X.new:{ … }` argument) before anything can reset it; nested blocks compiled
        // within read it as `false`.
        let is_init = std::mem::take(&mut self.next_block_is_init);
        // Consume the expression-literal flag likewise: nested literals set it
        // for themselves; definition bodies arrive with it unset.
        let is_expression = std::mem::take(&mut self.next_block_is_expression);
        // Phase 3c: a guard arm's narrowing, installed into this block's scope below. Taken here
        // (one-shot) so nested blocks don't inherit it.
        let block_narrowing = std::mem::take(&mut self.next_block_narrowing);
        // G4b: the declared `Block(…)` shape this literal is being passed to, receiver-bound —
        // one-shot like the narrowing so nested blocks don't inherit it (§11.3).
        let expected_params = std::mem::take(&mut self.next_block_expected);
        // Phase 3c join/merge: the key whose exit narrowing this arm should snapshot. Taken at
        // entry (bound to THIS block) and read from its scope just before `pop_scope`, so the
        // snapshot reflects the arm's straight-line effect (guard refinement + top-level
        // reassignments); nested blocks pop first and don't consume it.
        let capture_key = std::mem::take(&mut self.next_block_capture);
        // A real block gets its own frame, so any enclosing inlined-region caret
        // redirection (Slice 2d) must not leak into it: a `^` here is a genuine
        // `BlockReturn` for this block. Cleared on entry, restored on exit.
        let saved_inline = self.inline_carets.take();
        let mut param_names = Vec::new();
        let mut param_types = Vec::new();
        let mut param_elem_tags: Vec<Option<ElemTag>> = Vec::new();
        let mut locals = HashSet::new();

        for arg in &block.arguments {
            let name = arg.identifier.name.clone();
            param_names.push(name.clone());
            // An unannotated parameter defaults to `Object` (the universal supertype),
            // so `|x|` and `|x:Object|` are the same signature everywhere downstream.
            let type_name = arg
                .type_hint
                .as_ref()
                .map(|tr| self.dispatch_type_name(tr))
                .unwrap_or_else(|| "Object".to_string());
            param_types.push(type_name);
            param_elem_tags.push(
                arg.type_hint
                    .as_ref()
                    .and_then(|tr| self.param_elem_tag(tr)),
            );
            locals.insert(name);
        }

        // All-None normalizes to empty: legacy blocks share one shape, dispatch
        // scoring skips tag work entirely on `is_empty`, and variant identity
        // compares equal across pre- and post-generics compiles.
        if param_elem_tags.iter().all(Option::is_none) {
            param_elem_tags.clear();
        }

        let mut decls_names = Vec::new();
        for decl in &block.decls {
            let name = decl.identifier.name.clone();
            decls_names.push(name.clone());
            locals.insert(name);
        }

        self.push_scope(locals);
        self.scopes.last_mut().unwrap().is_init = is_init;
        if let Some((key, ty)) = block_narrowing {
            self.scopes.last_mut().unwrap().narrowed.insert(key, ty);
        }

        // Seed declared param types so arithmetic on a typed param devirtualizes, and so the
        // annotation acts as a *contract*: a reassignment is checked against it and flow-updates the
        // param's narrowing (Phase 3c), exactly like a `var x: T` local. In the METHOD role,
        // dispatch only selects a typed method when the arg matches, so the param is provably that
        // type on entry — no runtime guard needed; a `value:`-invoked bare literal gets no such
        // check, and its seeding stays operationally safe only because the devirt ops it feeds are
        // value-guarded (GENERICS_ARCH.md §11.1). An *un-annotated* param is `Any` (gradual,
        // unchecked), NOT `Object` — the `Object` default above is only the runtime dispatch
        // signature, not a static type.
        //
        // `param_beliefs` doubles as this literal's outward param shape (§11.3): the explicit
        // annotation where present, else the expectation's seed, else `Any`.
        let mut param_beliefs: Vec<Type> = Vec::with_capacity(block.arguments.len());
        for (i, arg) in block.arguments.iter().enumerate() {
            if let Some(hint) = &arg.type_hint {
                let ty = self.resolve_annotation(hint);
                let prov = Self::provenance_from(
                    arg.identifier.source_info.clone(),
                    "parameter".to_string(),
                );
                self.record_declared_type(&arg.identifier.name, ty.clone(), prov);
                param_beliefs.push(ty);
                continue;
            }
            // G4b: an UNANNOTATED param seeds from the declared `Block(…)` shape this literal is
            // being passed to — a narrowing-grade belief: read by `static_type`/warnings/
            // nil-narrowing, dissolved by any reassignment, never a contract and never devirt
            // (§11.1). `T` not `T?`: elements present during iteration are never the OOB nil
            // (§10.3). A type still mentioning an unbound variable claims nothing.
            let seed = expected_params
                .as_ref()
                .and_then(|e| e.get(i))
                .filter(|t| !matches!(t, Type::Any) && !t.contains_var())
                .cloned();
            if let Some(ty) = &seed {
                self.scopes
                    .last_mut()
                    .unwrap()
                    .narrowed
                    .insert(NarrowKey::Local(arg.identifier.name.clone()), ty.clone());
            }
            param_beliefs.push(seed.unwrap_or(Type::Any));
        }

        let mut block_bytecode = CodeBlock::new();
        block_bytecode.current_source = block.source_info.clone();

        for name in &decls_names {
            block_bytecode.push(Instruction::Push(Constant::Nil));
            block_bytecode.push(Instruction::DefineLocal(Symbol::intern(&(name.clone()))));
        }

        // Phase 3a: check/promote returns against this block's declared return type (`|args ^T|`).
        let expected_ret = block
            .return_type
            .as_ref()
            .map(|rt| type_from_ref_with_vars(rt, &self.ctx_type_params()));
        self.return_type_stack.push(expected_ret.clone());
        // G4b: accumulate the body's ACTUAL return type — the tail expression joined with every
        // real `^` return (the `BlockReturn` arm joins in; `^^` diverges the block and adds
        // nothing). Starts at `Never`, the join identity (§11.3).
        self.block_ret_harvest.push(Type::Never);

        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            block_bytecode.current_source = stmt.source_info.clone();
            // The final statement is the block's implicit return value; check it against the
            // declared return type. Explicit `^`/`^^` returns are handled by their own arms.
            let is_tail_expr = idx == len - 1
                && !matches!(
                    &stmt.value,
                    NodeValue::BlockReturn(_) | NodeValue::MethodReturn(_)
                );
            if let (true, Some(expected)) = (is_tail_expr, &expected_ret) {
                self.compile_expecting(stmt, expected, &mut block_bytecode)?;
            } else {
                self.compile_node(stmt, &mut block_bytecode)?;
            }
            if is_tail_expr {
                let t = self.static_type(stmt);
                if let Some(h) = self.block_ret_harvest.last_mut() {
                    let joined = h.join(&t);
                    *h = joined;
                }
            }
            if idx < len - 1 {
                self.check_discarded_caret_arm(stmt);
                block_bytecode.push(Instruction::Pop);
            }
        }
        self.return_type_stack.pop();
        let harvested = self.block_ret_harvest.pop().unwrap_or(Type::Never);
        // An empty body yields nil.
        let harvested = if len == 0 { Type::Nil } else { harvested };

        block_bytecode.current_source = block.source_info.clone();
        if len == 0 {
            block_bytecode.push(Instruction::Push(Constant::Nil));
        }

        block_bytecode.push(Instruction::Return);

        let decl_block = if let Some(db) = &block.decl_block {
            let mut db_bytecode = CodeBlock::new();
            db_bytecode.current_source = db.source_info.clone();
            self.compile_block(db, &mut db_bytecode)?;
            if let Some(Instruction::Push(Constant::Block(sb))) = db_bytecode.pop() {
                Some(sb)
            } else {
                None
            }
        } else {
            None
        };

        // Phase 3c join/merge: snapshot the guarded key's narrowed type at the arm's exit before
        // its scope is discarded. Absent from the overlay ⇒ the arm widened it to `Any`.
        if let Some(key) = &capture_key {
            let exit = self
                .scopes
                .last()
                .unwrap()
                .narrowed
                .get(key)
                .cloned()
                .unwrap_or(Type::Any);
            self.captured_arm_exit = Some(exit);
        }

        self.pop_scope();

        // G4b: record the literal's sharpened outward type (§11.3) — its header with the names
        // stripped, inference filling what the header leaves blank: params from annotations or
        // expectation seeds, the return from the declared `^Ret` or the harvested join. Recorded
        // only when it says something (all-`Any` stays bare `Block`, minting no claims). This is
        // what `static_type` answers for the literal from here on — and what call-site
        // unification binds `U` from (`collect:`'s `Block(T ^U)`).
        let ret_belief = expected_ret.unwrap_or(harvested);
        if param_beliefs.iter().any(|t| *t != Type::Any) || ret_belief != Type::Any {
            self.block_literal_types.insert(
                block as *const BlockNode as usize,
                Type::BlockOf {
                    params: param_beliefs,
                    ret: Box::new(ret_belief),
                },
            );
        }

        let block_name = block.name.as_ref().map(|s| s.value.clone());

        let (fused_bytecode, fused_source_map) =
            fuse_bytecode(block_bytecode.bytecode, block_bytecode.source_map);
        let static_block = StaticBlock {
            spec_state: Default::default(),
            uses_self: Default::default(),
            is_closed: Default::default(),
            name: block_name,
            is_nested_block: true,
            is_init_literal: is_init,
            param_syms: crate::value::intern_param_syms(&param_names),
            param_types,
            param_elem_tags,
            bytecode: SharedBytecode(Arc::new(fused_bytecode)),
            source_info: block.source_info.clone(),
            decl_block,
            source_map: SharedSourceMap(Arc::new(fused_source_map)),
            // Every closure of this literal shares one inline-cache array via this id.
            template_id: self
                .mint_template_ids
                .then(crate::instruction::fresh_template_id),
        };

        let template = Arc::new(static_block);
        // Boundary warning + (opted-in) IDE classification — here, after
        // `pop_scope`, so capture names resolve in the ENCLOSING scope.
        self.classify_block_literal(block as *const BlockNode as usize, &template, is_expression);
        bytecode.push(Instruction::Push(Constant::Block(template)));
        self.inline_carets = saved_inline;
        Ok(())
    }

    pub(super) fn reconstruct_selector(&self, sig: &MethodSelectorNode) -> Result<String, String> {
        if sig.identifiers.is_empty() {
            return Err("No identifiers found in method selector".to_string());
        }
        // The wildcard-selector rule: a definition may not write the same keyword twice in a row.
        // Consecutive repetition is the call-site idiom for a variadic component, so a literal
        // repeat (`foo:foo:`) is almost certainly a missing `+` — reject it so call-site folding
        // stays unambiguous. `+` is the only way to declare a repeated keyword.
        fn base(n: &str) -> &str {
            n.trim_end_matches(':').trim_end_matches('+')
        }
        for pair in sig.identifiers.windows(2) {
            if base(&pair[0].name) == base(&pair[1].name) {
                let kw = base(&pair[0].name);
                return Err(format!(
                    "selector repeats keyword '{kw}:'; declare it variadic with '{kw}+:' instead"
                ));
            }
        }
        let mut s = String::new();
        for ident in &sig.identifiers {
            s.push_str(&ident.name);
        }
        Ok(s)
    }
}
