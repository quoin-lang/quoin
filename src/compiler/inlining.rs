//! Method inlining (Phase 5): splicing a provably-monomorphic callee body at the call site
//! instead of dispatching. See docs/internal/TYPE_SYSTEM_ARCH.md.

use super::*;

impl Compiler {
    /// Emit a `Send`. (A sealed same-class self-send once emitted a distinct `CallSelfDirect`
    /// marker, but it was a runtime no-op — its planned resolve-and-cache was the ruled-out
    /// per-call-site inline cache — and, being unfused, was slightly *slower* than a plain `Send`,
    /// which the peephole fuses into a `SendLocal`. A provably-fixed self-send is instead handled by
    /// inlining, Phase 5.)
    pub(super) fn emit_call(&self, bytecode: &mut CodeBlock, selector: &str, num_args: usize) {
        bytecode.push(Instruction::Send(Symbol::intern(selector), num_args));
    }

    /// How deep self-send inlining may nest (Phase 5·2). A body may itself self-send, so without a
    /// bound a recursive or fan-out body would expand without limit at compile time; past the bound,
    /// the send stays a normal dispatch.
    pub(super) const MAX_INLINE_DEPTH: usize = 3;

    /// Can this method body be spliced at a call site (Phase 5·1→5·5)? No name / header-decl /
    /// decl-block, no *top-level* local decl (a `var`/`let` would splice a binding into the caller's
    /// scope — needs alpha-renaming, deferred), and — the soundness crux — no **non-local escape**
    /// (`^^`/`^>`) *anywhere*. Params are allowed (5·4); control-flow **blocks** are allowed (5·5),
    /// since `inline_block_body` redirects each `^` (block-return) to the inlined value — only `^^`
    /// (return-from-*method*) and `^>` (fiber-yield) would escape the callee's frame to the caller.
    fn is_inlinable_body(block: &BlockNode) -> bool {
        if block.name.is_some() || block.decl_block.is_some() || !block.decls.is_empty() {
            return false;
        }
        block.statements.iter().all(|s| {
            !matches!(&s.value, NodeValue::Declaration(_)) && !Self::escapes_inlined_frame(s)
        })
    }

    /// Does `node` contain a non-local return that would escape the inlined callee's frame — `^^`
    /// (MethodReturn: an inlined one returns from the *caller's* method) or `^>` (YieldReturn: it
    /// suspends the caller's fiber)? A `^` (BlockReturn) is fine — `inline_block_body` redirects it to
    /// the inlined value. Recurses structurally; an unrecognized node is treated as escaping (a
    /// conservative "don't inline"), so soundness never depends on the walk being exhaustive.
    fn escapes_inlined_frame(node: &Node) -> bool {
        match &node.value {
            NodeValue::MethodReturn(_) | NodeValue::YieldReturn(_) => true,
            NodeValue::BlockReturn(r) => Self::escapes_inlined_frame(&r.value),
            NodeValue::BinaryOperator(op) => {
                Self::escapes_inlined_frame(&op.left) || Self::escapes_inlined_frame(&op.right)
            }
            NodeValue::UnaryOperator(u) => Self::escapes_inlined_frame(&u.right),
            NodeValue::MethodCall(mc) => {
                mc.subject
                    .as_deref()
                    .is_some_and(Self::escapes_inlined_frame)
                    || mc
                        .arguments
                        .expressions
                        .iter()
                        .any(|e| Self::escapes_inlined_frame(e))
            }
            NodeValue::Block(b) => b.statements.iter().any(|s| Self::escapes_inlined_frame(s)),
            NodeValue::Assignment(a) => Self::escapes_inlined_frame(&a.rvalue),
            NodeValue::Declaration(d) => Self::escapes_inlined_frame(&d.rvalue),
            NodeValue::List(l) => l.values.iter().any(|e| Self::escapes_inlined_frame(e)),
            NodeValue::Set(s) => s.values.iter().any(|e| Self::escapes_inlined_frame(e)),
            NodeValue::UserList(u) => u.values.iter().any(|e| Self::escapes_inlined_frame(e)),
            NodeValue::Map(m) => m
                .keys
                .iter()
                .chain(&m.values)
                .any(|e| Self::escapes_inlined_frame(e)),
            NodeValue::Identifier(_)
            | NodeValue::Integer(_)
            | NodeValue::Double(_)
            | NodeValue::Str(_)
            | NodeValue::Symbol(_)
            | NodeValue::Regex(_)
            | NodeValue::UserString(_) => false,
            _ => true, // unrecognized ⇒ assume it might escape (never inline)
        }
    }

    /// The selector a call inlines under: the bare name for a unary send, the joined `foo:bar:` for a
    /// non-variadic keyword send. `None` for a variadic run (whose dispatched selector is `name+:`).
    fn inline_selector(call: &MethodCallNode) -> Option<String> {
        if call.arguments.expressions.is_empty() {
            let idents = &call.arguments.signature.identifiers;
            (idents.len() == 1).then(|| idents[0].name.clone())
        } else {
            Self::call_selector_nonvariadic(call)
        }
    }

    /// Splice `body` — an inline-safe body, possibly with `params` (5·4) and control flow (5·5) — at a
    /// call site: bind each arg to a temp (evaluated in the *caller's* context) and compile the body
    /// with params rebound via `param_override`, and `self` → `self_temp` for an explicit receiver.
    /// The body is spliced through `inline_block_body`, so multi-statement bodies and top-level `^`
    /// (redirected to the inlined value) work. Precondition: `is_inlinable_body(body)` and
    /// `body.arguments.len() == args.len()`.
    fn inline_body_with_args(
        &mut self,
        body: &BlockNode,
        args: &[Arc<Node>],
        self_temp: Option<Symbol>,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let mut bindings: HashMap<String, Symbol> = HashMap::new();
        for (param, arg) in body.arguments.iter().zip(args) {
            self.compile_node(arg, bytecode)?;
            let tmp = self.new_temp_var();
            let sym = Symbol::intern(&tmp);
            self.scopes.last_mut().unwrap().locals.insert(tmp);
            bytecode.push(Instruction::DefineLocal(sym));
            bindings.insert(param.identifier.name.clone(), sym);
        }
        let saved_self = self.self_override;
        if self_temp.is_some() {
            self.self_override = self_temp;
        }
        let saved_params = std::mem::replace(&mut self.param_override, bindings);
        self.inline_depth += 1;
        let result = self.inline_block_body(body, bytecode);
        self.inline_depth -= 1;
        self.param_override = saved_params;
        self.self_override = saved_self;
        result?;
        Ok(true)
    }

    /// Inline a self-send to a sealed class's own method with an inline-safe body (Phase 5·1/5·2/5·4):
    /// splice the callee's body instead of dispatching, binding any args to temps. Sound because a
    /// sealed class can't be subclassed, so `self.foo` provably resolves to this class's `foo`, and
    /// the receiver is `self` on both sides. `MAX_INLINE_DEPTH` bounds recursive/nested expansion.
    pub(super) fn try_inline_self_send(
        &mut self,
        call: &MethodCallNode,
        is_self: bool,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        // Under a `self_override` (5·3c), `self` is a rebound receiver, not the caller's `self`, so a
        // bare self-send resolves against *that* class (handled by `try_inline_exact_receiver`), not
        // the class being compiled — never self-inline here.
        if !is_self || self.self_override.is_some() || self.inline_depth >= Self::MAX_INLINE_DEPTH {
            return Ok(false);
        }
        let Some(selector) = Self::inline_selector(call) else {
            return Ok(false);
        };
        let body = match self.class_ctx.last() {
            Some(ctx) if ctx.sealed => ctx.bodies.get(&selector).cloned(),
            _ => None,
        };
        let Some(body) = body else {
            return Ok(false);
        };
        // Check inlinability + arity before emitting anything (a self-send has no receiver to emit,
        // but the arg temps below would be dangling on a bail).
        if !Self::is_inlinable_body(&body)
            || body.arguments.len() != call.arguments.expressions.len()
        {
            return Ok(false);
        }
        self.inline_body_with_args(&body, &call.arguments.expressions, None, bytecode)
    }

    /// If `block` is a bare field accessor (`x -> { @x }` — a single `@field` statement, no
    /// params/decls/name), the field's name. Unlike a general inline-safe body, this needs no
    /// `self`-rebinding: at an explicit-receiver `v.x` it's just "read `v`'s field" (Phase 5·3).
    fn field_accessor_field(block: &BlockNode) -> Option<String> {
        if !block.arguments.is_empty()
            || !block.decls.is_empty()
            || block.decl_block.is_some()
            || block.name.is_some()
        {
            return None;
        }
        let [stmt] = block.statements.as_slice() else {
            return None;
        };
        match &stmt.value {
            NodeValue::Identifier(id) if id.identifier_type == IdentifierType::Instance => {
                Some(id.name.clone())
            }
            _ => None,
        }
    }

    /// Inline an explicit-receiver send `v.foo` (possibly with args) to a sealed in-unit class (Phase
    /// 5·3/5·3b/5·3c/5·4). Sound because a sealed class can't be subclassed, so `v` is exactly that
    /// class; a non-nullable typed receiver is never nil. A no-arg **field accessor** reads `v`'s
    /// field directly (`<eval v>; LoadFieldOf`); any other **inline-safe** body is spliced with
    /// `self` rebound to `v` (a temp) and params rebound to arg temps. Returns `true` if it inlined.
    pub(super) fn try_inline_exact_receiver(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        if self.inline_depth >= Self::MAX_INLINE_DEPTH {
            return Ok(false);
        }
        let Some(subject) = call.subject.as_deref() else {
            return Ok(false); // implicit self-send — handled by try_inline_self_send
        };
        let Some(class) = self.receiver_class(call) else {
            return Ok(false);
        };
        if self.class_table.get(&class).map(|s| s.sealed) != Some(true) {
            return Ok(false);
        }
        let Some(selector) = Self::inline_selector(call) else {
            return Ok(false);
        };
        let Some(body) = self
            .class_bodies
            .get(&class)
            .and_then(|b| b.get(&selector))
            .cloned()
        else {
            return Ok(false);
        };
        let args = &call.arguments.expressions;
        // No-arg field accessor: read the field directly off the receiver — no temp needed.
        if args.is_empty()
            && let Some(field) = Self::field_accessor_field(&body)
        {
            self.compile_node(subject, bytecode)?;
            bytecode.push(Instruction::LoadFieldOf(field));
            return Ok(true);
        }
        // Any other inline-safe body: check BEFORE emitting the receiver, else a bail dangles it.
        if !Self::is_inlinable_body(&body) || body.arguments.len() != args.len() {
            return Ok(false);
        }
        // Evaluate the receiver once into a temp; splice the body with `self` → that temp (and, in
        // `inline_body_with_args`, params → arg temps).
        self.compile_node(subject, bytecode)?;
        let tmp = self.new_temp_var();
        let v_sym = Symbol::intern(&tmp);
        self.scopes.last_mut().unwrap().locals.insert(tmp);
        bytecode.push(Instruction::DefineLocal(v_sym));
        self.inline_body_with_args(&body, args, Some(v_sym), bytecode)
    }
}
