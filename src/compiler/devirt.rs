//! Control-flow inlining (`if:`/`whileDo:` → native jumps, Slice 2d) and `List` op
//! devirtualization (Slice 2e).

use super::*;
use std::sync::Arc;

impl Compiler {
    /// The devirtualized `List` op for a keyword send whose receiver is statically a `List`
    /// (Slice 2e), or `None` to fall through to a normal send.
    pub(super) fn collection_devirt_op(
        &self,
        call: &MethodCallNode,
        selector: &str,
        num_args: usize,
    ) -> Option<Instruction> {
        let subject = call.subject.as_ref()?;
        // List/Map/Set are sealed value types (prelude.qn), so their access methods can't be
        // redefined — devirt to a direct op when the receiver is statically that collection. Each
        // op falls back to the real send if the runtime receiver isn't the expected native state.
        // A checked collection (`List(Integer)`) is its bare type at runtime; the
        // ops' interpreter arms carry the tag gate, so tagged receivers keep the
        // fast path (docs/GENERICS_ARCH.md §6).
        match (self.static_type(subject), selector, num_args) {
            (Type::List | Type::ListOf(_), "at:", 1) => Some(Instruction::ListGet),
            (Type::List | Type::ListOf(_), "at:put:", 2) => Some(Instruction::ListSet),
            (Type::List | Type::ListOf(_), "add:", 1) => Some(Instruction::ListPush),
            (Type::Map | Type::MapOf(_), "at:", 1) => Some(Instruction::MapGet),
            (Type::Map | Type::MapOf(_), "at:put:", 2) => Some(Instruction::MapSet),
            // Set has no devirt op — its native `contains?:`/`add:` dispatch `==:` per element
            // (structural/custom equality), which a direct raw-equality op can't replicate.
            _ => None,
        }
    }

    /// Slice 2d — control-flow inlining. If `call` is `recv.if:{…}` or
    /// `recv.if:{…}else:{…}` where `recv` is statically `Boolean` and every block arg is
    /// a literal, parameter-less, declaration-free block, splice the block bodies inline
    /// as `ElseJump`/`Jump` bytecode (no block alloc, no dispatch, no block frame) and
    /// return `true`. Otherwise emit nothing and return `false` so the caller compiles the
    /// normal send.
    ///
    /// Soundness: `Boolean` is sealed (prelude), so `if:`/`if:else:` on a statically-Bool
    /// receiver always resolve to the built-in `True`/`False` methods — treating them as
    /// inlinable built-ins is a language guarantee, matching Smalltalk `ifTrue:ifFalse:`.
    pub(super) fn try_compile_inlined_conditional(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let subject = match &call.subject {
            Some(s) => s,
            None => return Ok(false),
        };
        let idents = &call.arguments.signature.identifiers;
        let exprs = &call.arguments.expressions;

        // Selector shape: `if:` (then only) or `if:else:` (then + else).
        let kws: Vec<&str> = idents.iter().map(|i| i.name.as_str()).collect();
        let has_else = match kws.as_slice() {
            ["if"] => false,
            ["if", "else"] => true,
            _ => return Ok(false),
        };

        // Phase 3c: a nil-guard on a *declared-nullable* path is not inlined — it takes the general
        // send path so its arms narrow (via each arm's `compile_block` scope). Rare and opt-in;
        // untyped guards (the common case) aren't `Nullable`, so they still inline — no perf change.
        if self.guard_narrowing(call).is_some() {
            return Ok(false);
        }

        // Bool receiver → inline directly. A known-non-Bool receiver (Int/List) → normal send
        // (the guard would always miss). Everything else — `Any`, and any other static type we
        // don't specifically reason about — → guarded inline (option C): a runtime Bool-check
        // falls back to the real send for a non-Bool receiver.
        let guarded = match self.static_type(subject) {
            Type::Bool => false,
            Type::Int | Type::List | Type::ListOf(_) => return Ok(false),
            _ => true,
        };

        // Every arg must be a literal, 0-arg block: declaration-free (v1,
        // unconditional) or declaration-carrying via alpha-renamed splicing (v2 —
        // loop-repeated here only when an enclosing fused loop re-executes this site).
        let loop_repeated = self.fused_loop_depth > 0;
        let then_blk = match self.spliceable_arm(&exprs[0], loop_repeated) {
            Some(b) => b,
            None => return Ok(false),
        };
        let else_blk = if has_else {
            match self.spliceable_arm(&exprs[1], loop_repeated) {
                Some(b) => Some(b),
                None => return Ok(false),
            }
        } else {
            None
        };

        // Condition → stack.
        self.compile_node(subject, bytecode)?;

        if !guarded {
            self.emit_inline_conditional_body(then_blk, else_blk, bytecode)?;
            return Ok(true);
        }

        // Guarded (option C): if the receiver isn't a Bool at runtime, jump past the inlined
        // body to a cold path that reissues the real send. The inlined body is
        // self-contained (leaves its value on the stack), so it is wrapped verbatim.
        let mut hot_bc = CodeBlock::new();
        hot_bc.current_source = bytecode.current_source.clone();
        self.emit_inline_conditional_body(then_blk, else_blk, &mut hot_bc)?;

        let mut cold_bc = CodeBlock::new();
        cold_bc.current_source = bytecode.current_source.clone();
        self.compile_block(then_blk, &mut cold_bc)?;
        if let Some(else_blk) = else_blk {
            self.compile_block(else_blk, &mut cold_bc)?;
            self.emit_call(&mut cold_bc, "if:else:", 2);
        } else {
            self.emit_call(&mut cold_bc, "if:", 1);
        }

        let h = hot_bc.len() as isize;
        let k = cold_bc.len() as isize;
        bytecode.push(Instruction::BranchIfNotBool(h + 2));
        bytecode.extend(hot_bc);
        bytecode.push(Instruction::Jump(k + 1));
        bytecode.extend(cold_bc);
        Ok(true)
    }

    /// Emit the unguarded inlined form of `if:`/`if:else:` (receiver already on the stack)
    /// into `out`: `ElseJump; <then>; Jump; <else | Push(Nil)>`, leaving the construct's
    /// value on the stack. Shared by the Bool-receiver path and the guarded (option C) hot
    /// path.
    fn emit_inline_conditional_body(
        &mut self,
        then_blk: &BlockNode,
        else_blk: Option<&BlockNode>,
        out: &mut CodeBlock,
    ) -> Result<(), String> {
        let mut then_bc = CodeBlock::new();
        then_bc.current_source = out.current_source.clone();
        self.splice_block_body(then_blk, &mut then_bc)?;
        let t = then_bc.len() as isize;

        if let Some(else_blk) = else_blk {
            let mut else_bc = CodeBlock::new();
            else_bc.current_source = out.current_source.clone();
            self.splice_block_body(else_blk, &mut else_bc)?;
            let e = else_bc.len() as isize;
            // cond false → skip the then-body and its trailing Jump, land on the else-body.
            out.push(Instruction::ElseJump(t + 2));
            out.extend(then_bc);
            out.push(Instruction::Jump(e + 1));
            out.extend(else_bc);
        } else {
            // No else: a false condition makes the construct's value `nil`.
            out.push(Instruction::ElseJump(t + 2));
            out.extend(then_bc);
            out.push(Instruction::Jump(2));
            out.push(Instruction::Push(Constant::Nil));
        }
        Ok(())
    }

    /// A literal block usable for v1 control-flow inlining: no parameters and no local
    /// declarations — splicing it can't bind anything into the method scope, so it
    /// qualifies unconditionally. Declaration-carrying blocks go through
    /// `spliceable_arm` (v2 alpha-renaming) instead.
    fn inlinable_block(node: &Node) -> Option<&BlockNode> {
        if let NodeValue::Block(b) = &node.value {
            if b.arguments.is_empty() && b.decls.is_empty() && !Self::block_declares_local(b) {
                return Some(b);
            }
        }
        None
    }

    /// Does this block's top level declare locals (a `var`/`let` *statement*; header
    /// `decls` are separate and keep a block v1-ineligible on their own)?
    fn block_declares_local(b: &BlockNode) -> bool {
        b.statements
            .iter()
            .any(|s| matches!(&s.value, NodeValue::Declaration(_)))
    }

    /// v2 (alpha-renaming): a literal 0-arg block acceptable for control-flow splicing.
    /// A declaration-free block qualifies exactly as v1, unconditionally. A block whose
    /// top level DECLARES locals is spliced with those declarations alpha-renamed to
    /// fresh source-unspellable names (`Compiler::declare_local`), and qualifies only
    /// when:
    /// - it is shape-simple: unnamed, unguarded, no header decls;
    /// - the splice would not run in an object-initializer frame — under (E) every
    ///   binding a config frame holds becomes an instance FIELD, so a spliced `var`
    ///   would pollute the new object (the real block frame isolates it today);
    /// - if the splice is LOOP-REPEATED (`loop_repeated`: `whileDo:` cond/body always;
    ///   an `if:` arm compiled inside an enclosing fused loop), no surviving nested
    ///   literal captures a declared name — a real block frame mints a fresh cell per
    ///   execution, a splice rebinds ONE frame cell, and only a closure that outlives
    ///   the iteration can observe the difference (the binding-generation hazard).
    fn spliceable_arm<'a>(&self, node: &'a Node, loop_repeated: bool) -> Option<&'a BlockNode> {
        if let Some(b) = Self::inlinable_block(node) {
            return Some(b);
        }
        let NodeValue::Block(b) = &node.value else {
            return None;
        };
        if !b.arguments.is_empty()
            || !b.decls.is_empty()
            || b.name.is_some()
            || b.decl_block.is_some()
        {
            return None;
        }
        if self.in_init_frame() {
            return None;
        }
        if loop_repeated && !self.splice_hazard_free(b) {
            return None;
        }
        Some(b)
    }

    /// The plain-local names a block's top-level declarations introduce.
    fn declared_local_names(&self, b: &BlockNode) -> Vec<String> {
        let mut names = Vec::new();
        for s in &b.statements {
            if let NodeValue::Declaration(d) = &s.value {
                self.collect_lvalue_names(&d.lvalues, &mut names);
            }
        }
        names
    }

    /// TRUE if a loop-repeated splice of `b` is free of the binding-generation hazard:
    /// no nested literal that SURVIVES as a runtime block mentions a name declared at
    /// `b`'s top level. Nested `whileDo:` constructs that will themselves fuse
    /// (`while_will_fuse`, recursive) are transparent — their cond/body statements
    /// splice into the same frame rather than surviving; `if:` arms are conservatively
    /// treated as surviving (a guarded inline keeps a real cold copy regardless).
    /// Mentions match the ORIGINAL name anywhere inside the literal — over-approximate
    /// under shadowing, which only ever refuses a fusible shape, never fuses a
    /// hazardous one.
    fn splice_hazard_free(&self, b: &BlockNode) -> bool {
        let names = self.declared_local_names(b);
        if names.is_empty() {
            return true;
        }
        !b.statements.iter().any(|s| self.survival_hazard(s, &names))
    }

    /// Walk one spliced statement: does any surviving nested literal mention a tracked
    /// name? Unrecognized node kinds conservatively count as hazards — soundness never
    /// depends on this walk being exhaustive (the `escapes_inlined_frame` discipline).
    fn survival_hazard(&self, node: &Node, names: &[String]) -> bool {
        match &node.value {
            NodeValue::Block(lit) => Self::block_mentions_any(lit, names),
            NodeValue::MethodCall(mc) => {
                if self.while_will_fuse(mc) {
                    let cond = mc.subject.as_deref().unwrap();
                    let (NodeValue::Block(cb), NodeValue::Block(bb)) =
                        (&cond.value, &mc.arguments.expressions[0].value)
                    else {
                        return true; // will_fuse guarantees blocks; defensive
                    };
                    cb.statements.iter().any(|s| self.survival_hazard(s, names))
                        || bb.statements.iter().any(|s| self.survival_hazard(s, names))
                } else {
                    mc.subject
                        .as_deref()
                        .is_some_and(|s| self.survival_hazard(s, names))
                        || mc
                            .arguments
                            .expressions
                            .iter()
                            .any(|e| self.survival_hazard(e, names))
                }
            }
            NodeValue::Assignment(a) => {
                a.lvalues.iter().any(|l| self.survival_hazard(l, names))
                    || self.survival_hazard(&a.rvalue, names)
            }
            NodeValue::Declaration(d) => self.survival_hazard(&d.rvalue, names),
            NodeValue::SubLValue(s) => s.lvalues.iter().any(|l| self.survival_hazard(l, names)),
            NodeValue::BinaryOperator(op) => {
                self.survival_hazard(&op.left, names) || self.survival_hazard(&op.right, names)
            }
            NodeValue::UnaryOperator(u) => self.survival_hazard(&u.right, names),
            NodeValue::MethodReturn(r) => self.survival_hazard(&r.value, names),
            NodeValue::BlockReturn(r) => self.survival_hazard(&r.value, names),
            NodeValue::YieldReturn(r) => self.survival_hazard(&r.value, names),
            NodeValue::List(l) => l.values.iter().any(|e| self.survival_hazard(e, names)),
            NodeValue::Set(s) => s.values.iter().any(|e| self.survival_hazard(e, names)),
            NodeValue::Map(m) => m
                .keys
                .iter()
                .chain(&m.values)
                .any(|e| self.survival_hazard(e, names)),
            NodeValue::Identifier(_)
            | NodeValue::IdentLValue(_)
            | NodeValue::SplatLValue(_)
            | NodeValue::IgnoredLValue
            | NodeValue::IgnoredSplatLValue
            | NodeValue::Integer(_)
            | NodeValue::Double(_)
            | NodeValue::Str(_)
            | NodeValue::Symbol(_)
            | NodeValue::Regex(_) => false,
            _ => true,
        }
    }

    /// Does any plain identifier matching one of `names` appear anywhere in this block
    /// (its own body and nested literals alike)? Shadowing inside nested blocks is
    /// deliberately ignored — over-approximation is the sound direction here.
    fn block_mentions_any(b: &BlockNode, names: &[String]) -> bool {
        b.statements.iter().any(|s| Self::mentions_any(s, names))
    }

    fn mentions_any(node: &Node, names: &[String]) -> bool {
        match &node.value {
            NodeValue::Identifier(id) => {
                id.identifier_type != IdentifierType::Instance
                    && id.namespace.is_none()
                    && names.iter().any(|n| *n == id.name)
            }
            NodeValue::Block(b) => Self::block_mentions_any(b, names),
            NodeValue::MethodCall(mc) => {
                mc.subject
                    .as_deref()
                    .is_some_and(|s| Self::mentions_any(s, names))
                    || mc
                        .arguments
                        .expressions
                        .iter()
                        .any(|e| Self::mentions_any(e, names))
            }
            NodeValue::Assignment(a) => {
                a.lvalues.iter().any(|l| Self::mentions_any(l, names))
                    || Self::mentions_any(&a.rvalue, names)
            }
            NodeValue::Declaration(d) => Self::mentions_any(&d.rvalue, names),
            NodeValue::IdentLValue(l) => {
                l.identifier.identifier_type != IdentifierType::Instance
                    && l.identifier.namespace.is_none()
                    && names.iter().any(|n| *n == l.identifier.name)
            }
            NodeValue::SplatLValue(l) => names.iter().any(|n| *n == l.identifier.name),
            NodeValue::SubLValue(s) => s.lvalues.iter().any(|l| Self::mentions_any(l, names)),
            NodeValue::BinaryOperator(op) => {
                Self::mentions_any(&op.left, names) || Self::mentions_any(&op.right, names)
            }
            NodeValue::UnaryOperator(u) => Self::mentions_any(&u.right, names),
            NodeValue::MethodReturn(r) => Self::mentions_any(&r.value, names),
            NodeValue::BlockReturn(r) => Self::mentions_any(&r.value, names),
            NodeValue::YieldReturn(r) => Self::mentions_any(&r.value, names),
            NodeValue::List(l) => l.values.iter().any(|e| Self::mentions_any(e, names)),
            NodeValue::Set(s) => s.values.iter().any(|e| Self::mentions_any(e, names)),
            NodeValue::Map(m) => m
                .keys
                .iter()
                .chain(&m.values)
                .any(|e| Self::mentions_any(e, names)),
            NodeValue::Integer(_)
            | NodeValue::Double(_)
            | NodeValue::Str(_)
            | NodeValue::Symbol(_)
            | NodeValue::Regex(_)
            | NodeValue::IgnoredLValue
            | NodeValue::IgnoredSplatLValue => false,
            _ => true,
        }
    }

    /// Mirrors `try_compile_inlined_while`'s decision closely enough to
    /// UNDER-approximate it (a `false` for a loop that later fuses only over-refuses
    /// the outer splice — sound; a `true` for one that later refuses would be a missed
    /// hazard, so the predicates here are exactly the ones the real fusion uses).
    fn while_will_fuse(&self, mc: &MethodCallNode) -> bool {
        let kws: Vec<&str> = mc
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if kws.as_slice() != ["whileDo"] || mc.arguments.expressions.len() != 1 {
            return false;
        }
        let Some(subject) = mc.subject.as_deref() else {
            return false;
        };
        self.spliceable_arm(subject, true).is_some()
            && self
                .spliceable_arm(&mc.arguments.expressions[0], true)
                .is_some()
    }

    /// Splice a block body inline. A declaration-carrying block (v2) gets a splice
    /// scope so its declarations alpha-rename and its checker state stays arm-scoped;
    /// a declaration-free block splices exactly as v1 (no scope).
    fn splice_block_body(&mut self, block: &BlockNode, out: &mut CodeBlock) -> Result<(), String> {
        if Self::block_declares_local(block) {
            self.push_splice_scope();
            let r = self.inline_block_body(block, out);
            self.pop_scope();
            r
        } else {
            self.inline_block_body(block, out)
        }
    }

    /// M2 fused instantiation (docs/MATERIALIZATION_ARCH.md): compile
    /// `X.new:{ f1=e1; …; fn=en }` on the plain-config shape into the guarded
    /// dual form — the option-C pattern applied to the instantiation seam:
    ///
    /// ```text
    /// <receiver>
    /// BranchIfNotPlainNew(→cold)
    /// <e1> … <en>              // field rvalues inline in the METHOD frame
    /// NewWithFields([f1…fn])
    /// Jump(→end)
    /// cold: Push(Block(config)); Send(new:, 1)
    /// ```
    ///
    /// The guard runs BEFORE the rvalues evaluate, so the cold path (a user
    /// meta `new:`, an abstract class, a non-class receiver) re-evaluates them
    /// inside the real config closure — no double evaluation on either path.
    /// Returns false (emitting nothing) for any shape `fusable_config` refuses.
    pub(super) fn try_compile_fused_instantiation(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if kws.as_slice() != ["new"] || call.arguments.expressions.len() != 1 {
            return Ok(false);
        }
        let Some(subject) = &call.subject else {
            return Ok(false);
        };
        let Some(pairs) = Self::fusable_config(&call.arguments.expressions[0]) else {
            return Ok(false);
        };

        // Receiver first — evaluated once; both paths read it from the stack.
        self.compile_node(subject, bytecode)?;

        let mut hot = CodeBlock::new();
        hot.current_source = bytecode.current_source.clone();
        for (_, rvalue) in &pairs {
            self.compile_node(rvalue, &mut hot)?;
        }
        let names: Vec<Symbol> = pairs.iter().map(|(s, _)| *s).collect();
        hot.push(Instruction::NewWithFields(Arc::new(names)));

        let mut cold = CodeBlock::new();
        cold.current_source = bytecode.current_source.clone();
        self.next_block_is_init = true;
        self.compile_node(&call.arguments.expressions[0], &mut cold)?;
        self.emit_call(&mut cold, "new:", 1);

        let h = hot.len() as isize;
        let k = cold.len() as isize;
        bytecode.push(Instruction::BranchIfNotPlainNew(h + 2));
        bytecode.extend(hot);
        bytecode.push(Instruction::Jump(k + 1));
        bytecode.extend(cold);
        Ok(true)
    }

    /// A config literal whose fused evaluation is indistinguishable from
    /// running it in an instantiation frame: 0-arg, unnamed, unguarded, no
    /// header decls; every top-level statement a single-target plain
    /// assignment to a bare lowercase name; every rvalue self-free (config
    /// `self` IS the new object), field-free, bare-send-free, literal-free (a
    /// nested literal's captures of config-bound names would resolve
    /// differently without the config frame), fiber-yield-free — and reading
    /// no name a PRIOR statement in the same config stored (such a read sees
    /// the config-local binding today when the name shadows an outer local).
    /// Returns the (field, rvalue) pairs in source order.
    fn fusable_config(node: &Node) -> Option<Vec<(Symbol, &Node)>> {
        let NodeValue::Block(b) = &node.value else {
            return None;
        };
        if !b.arguments.is_empty()
            || !b.decls.is_empty()
            || b.name.is_some()
            || b.decl_block.is_some()
        {
            return None;
        }
        let mut pairs = Vec::with_capacity(b.statements.len());
        let mut stored: Vec<String> = Vec::new();
        for stmt in &b.statements {
            let NodeValue::Assignment(a) = &stmt.value else {
                return None;
            };
            let [lval] = a.lvalues.as_slice() else {
                return None;
            };
            let NodeValue::IdentLValue(l) = &lval.value else {
                return None;
            };
            let id = &l.identifier;
            if id.namespace.is_some()
                || id.identifier_type != IdentifierType::Local
                || !id
                    .name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
            {
                return None;
            }
            if !Self::fusable_field_rvalue(&a.rvalue) || Self::mentions_any(&a.rvalue, &stored) {
                return None;
            }
            stored.push(id.name.clone());
            pairs.push((Symbol::intern(&id.name), &*a.rvalue));
        }
        Some(pairs)
    }

    /// An rvalue safe to evaluate in the METHOD frame instead of the config
    /// frame (see `fusable_config`). Unrecognized node kinds conservatively
    /// refuse — the `escapes_inlined_frame` discipline.
    fn fusable_field_rvalue(node: &Node) -> bool {
        match &node.value {
            NodeValue::Identifier(id) => {
                id.identifier_type != IdentifierType::Instance && id.name != "self"
            }
            NodeValue::MethodCall(mc) => {
                mc.subject
                    .as_deref()
                    .is_some_and(Self::fusable_field_rvalue)
                    && mc
                        .arguments
                        .expressions
                        .iter()
                        .all(|e| Self::fusable_field_rvalue(e))
            }
            NodeValue::BinaryOperator(op) => {
                Self::fusable_field_rvalue(&op.left) && Self::fusable_field_rvalue(&op.right)
            }
            NodeValue::UnaryOperator(u) => Self::fusable_field_rvalue(&u.right),
            NodeValue::List(l) => l.values.iter().all(|e| Self::fusable_field_rvalue(e)),
            NodeValue::Set(s) => s.values.iter().all(|e| Self::fusable_field_rvalue(e)),
            NodeValue::Map(m) => m
                .keys
                .iter()
                .chain(&m.values)
                .all(|e| Self::fusable_field_rvalue(e)),
            NodeValue::Integer(_)
            | NodeValue::Double(_)
            | NodeValue::Str(_)
            | NodeValue::Symbol(_)
            | NodeValue::Regex(_) => true,
            _ => false,
        }
    }

    /// Compile an inlined control-flow block body into `out`: its statements spliced
    /// inline (value-on-stack like a block, but no frame and no trailing `Return`), with
    /// each top-level `^expr` redirected to a `Jump` past the body (patched here). `^^`
    /// (MethodReturn) is left untouched — it still returns from the enclosing method.
    pub(super) fn inline_block_body(
        &mut self,
        block: &BlockNode,
        out: &mut CodeBlock,
    ) -> Result<(), String> {
        let saved = self.inline_carets.replace(Vec::new());
        let len = block.statements.len();
        for (idx, stmt) in block.statements.iter().enumerate() {
            out.current_source = stmt.source_info.clone();
            self.compile_node(stmt, out)?;
            // Discard the value of every statement but the last (the block's value).
            if idx + 1 < len {
                out.push(Instruction::Pop);
            }
        }
        if len == 0 {
            out.push(Instruction::Push(Constant::Nil));
        }
        // Patch each top-level `^` to jump to just past the body (falls through to the
        // construct's merge point).
        let carets = self.inline_carets.take().unwrap_or_default();
        let end = out.len() as isize;
        for pos in carets {
            set_jump_offset(&mut out.bytecode[pos], end - pos as isize);
        }
        self.inline_carets = saved;
        Ok(())
    }

    /// Slice 2d — inline `{cond}.whileDo:{body}` when both the receiver (`cond`) and
    /// the body are spliceable 0-arg literal blocks (declaration-free, or
    /// declaration-carrying via alpha-renaming — `spliceable_arm`), into a native jump
    /// loop.
    /// Eliminates the per-iteration block allocation, dispatch, and frame — and the
    /// recursion, since the bootstrap `whileDo:` recurses once per iteration
    /// (`^^s.whileDo:block`). Returns `true` if inlined. Evaluates to `nil`, matching the
    /// bootstrap (the terminating `if:` has no else). `^` in `cond`/`body` ends that block
    /// (redirected by `inline_block_body`); `^^` still returns from the enclosing method.
    pub(super) fn try_compile_inlined_while(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let subject = match &call.subject {
            Some(s) => s,
            None => return Ok(false),
        };
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if kws.as_slice() != ["whileDo"] {
            return Ok(false);
        }
        // Cond and body both re-execute per iteration, so a declaring block here is
        // always loop-repeated (v2 hazard gating applies unconditionally).
        let cond_blk = match self.spliceable_arm(subject, true) {
            Some(b) => b,
            None => return Ok(false),
        };
        let body_blk = match self.spliceable_arm(&call.arguments.expressions[0], true) {
            Some(b) => b,
            None => return Ok(false),
        };

        // Compile cond/body into their own sub-blocks so their lengths size the jumps.
        // The depth is raised across BOTH so any `if:` arm spliced inside them knows it
        // is loop-repeated.
        let mut cond_bc = CodeBlock::new();
        cond_bc.current_source = bytecode.current_source.clone();
        let mut body_bc = CodeBlock::new();
        body_bc.current_source = bytecode.current_source.clone();
        self.fused_loop_depth += 1;
        let compiled = self
            .splice_block_body(cond_blk, &mut cond_bc)
            .and_then(|()| self.splice_block_body(body_blk, &mut body_bc));
        self.fused_loop_depth -= 1;
        compiled?;
        let c = cond_bc.len() as isize;
        let b = body_bc.len() as isize;

        // Layout (each jump offset is relative to its own position):
        //   [start] <cond>          (c instrs; leaves the condition on the stack)
        //           ElseJump(b+3)    cond false → exit to the trailing nil
        //           <body>          (b instrs; leaves the body value)
        //           Pop              discard the body value
        //           Jump(-(c+b+2))   back to [start]
        //   [end]   Push(Nil)        whileDo: evaluates to nil
        bytecode.extend(cond_bc);
        bytecode.push(Instruction::ElseJump(b + 3));
        bytecode.extend(body_bc);
        bytecode.push(Instruction::Pop);
        bytecode.push(Instruction::Jump(-(c + b + 2)));
        bytecode.push(Instruction::Push(Constant::Nil));
        Ok(true)
    }

    /// A literal block acceptable for `each:` fusion (B1, docs/BLOCK_AOT_ARCH.md §3): at
    /// most one parameter, no name / header decls / decl-block, no top-level local
    /// declaration (it would splice a binding into the method scope), and nothing
    /// anywhere in its tree the fusion would mis-bind or escape (`each_fusion_blocker`).
    fn fusable_each_block(node: &Node) -> Option<&BlockNode> {
        let NodeValue::Block(b) = &node.value else {
            return None;
        };
        if b.name.is_some()
            || b.decl_block.is_some()
            || !b.decls.is_empty()
            || b.arguments.len() > 1
        {
            return None;
        }
        let top_declares = b
            .statements
            .iter()
            .any(|s| matches!(&s.value, NodeValue::Declaration(_)));
        if top_declares || b.statements.iter().any(|s| Self::each_fusion_blocker(s)) {
            return None;
        }
        Some(b)
    }

    /// Would this node be mis-compiled by splicing the `each:` block into the method
    /// frame? `each:` invokes its block via `valueWithSelfOrArg:`, which rebinds `self`
    /// to the ELEMENT — so any `self` reference (explicit, a bare `.foo` send, an
    /// `@field` read or write) must keep the real block frame; `^>` suspends the block's
    /// own fiber context. `^^` is FINE: spliced, its lexical target is the very method
    /// frame it runs in. Nested block literals keep their own frames at runtime but
    /// still resolve `self` through the shared chain, so the walk recurses into them.
    /// An unrecognized node conservatively blocks — soundness never depends on this
    /// walk being exhaustive (the `escapes_inlined_frame` discipline).
    fn each_fusion_blocker(node: &Node) -> bool {
        match &node.value {
            NodeValue::YieldReturn(_) => true,
            NodeValue::Identifier(id) => {
                id.name == "self" || id.identifier_type == IdentifierType::Instance
            }
            NodeValue::MethodCall(mc) => match mc.subject.as_deref() {
                None => true, // a bare `.foo` send targets the rebound self
                Some(s) => {
                    Self::each_fusion_blocker(s)
                        || mc
                            .arguments
                            .expressions
                            .iter()
                            .any(|e| Self::each_fusion_blocker(e))
                }
            },
            NodeValue::MethodReturn(r) => Self::each_fusion_blocker(&r.value),
            NodeValue::BlockReturn(r) => Self::each_fusion_blocker(&r.value),
            NodeValue::BinaryOperator(op) => {
                Self::each_fusion_blocker(&op.left) || Self::each_fusion_blocker(&op.right)
            }
            NodeValue::UnaryOperator(u) => Self::each_fusion_blocker(&u.right),
            NodeValue::Assignment(a) => {
                // `@field = …` writes through the rebound self; destructuring targets
                // are out of scope (conservative). Plain-local targets are the shared-
                // capture case fusion preserves exactly (one frame, same slot).
                a.lvalues.iter().any(|lv| match &lv.value {
                    NodeValue::IdentLValue(l) => {
                        l.identifier.identifier_type == IdentifierType::Instance
                    }
                    _ => true,
                }) || Self::each_fusion_blocker(&a.rvalue)
            }
            NodeValue::Declaration(d) => Self::each_fusion_blocker(&d.rvalue),
            NodeValue::Block(b) => {
                b.statements.iter().any(|s| Self::each_fusion_blocker(s))
                    || b.decl_block.as_deref().is_some_and(|db| {
                        db.statements.iter().any(|s| Self::each_fusion_blocker(s))
                    })
            }
            NodeValue::List(l) => l.values.iter().any(|e| Self::each_fusion_blocker(e)),
            NodeValue::Set(s) => s.values.iter().any(|e| Self::each_fusion_blocker(e)),
            NodeValue::UserList(u) => u.values.iter().any(|e| Self::each_fusion_blocker(e)),
            NodeValue::Map(m) => m
                .keys
                .iter()
                .chain(&m.values)
                .any(|e| Self::each_fusion_blocker(e)),
            NodeValue::Integer(_)
            | NodeValue::Double(_)
            | NodeValue::Str(_)
            | NodeValue::Symbol(_)
            | NodeValue::Regex(_)
            | NodeValue::UserString(_) => false,
            _ => true, // unrecognized ⇒ assume it needs the real block frame
        }
    }

    /// B1 (docs/BLOCK_AOT_ARCH.md §3): fuse `recv.each:{ |x| … }` into a guarded native
    /// index loop. The hot path — a native-List receiver, for which sealed `List#each:`
    /// fully determines dispatch — runs the block body spliced INLINE in the method
    /// frame: no closure, no per-element frame/env/send, captures are the frame's own
    /// locals (exact shared-cell semantics, since there is only one frame), `^` ends the
    /// element, `^^` returns from the method it lexically targets anyway. The cold path
    /// re-materializes the literal and performs the real send (custom `each:`
    /// implementations, Set/Map/Generator receivers, MNU — exact semantics), so the
    /// receiver's static type is irrelevant: the GUARD is the dispatch, and bare
    /// `.each:{…}` self-sends fuse too (how Iterate's combinators go closure-free on
    /// native lists). Loop semantics mirror `List#each:` exactly: the bound is read
    /// once (a body that mutates the list sees the stale bound; `ListGet` reads
    /// elements fresh, OOB → nil), and the expression's value is nil.
    /// Does this template (or any nested literal) read or write local
    /// `sym`? Used to detect loop-variable capture by closures created in a
    /// fused `each:` body (BUGS.md Finding 10).
    fn block_references(sb: &crate::instruction::StaticBlock, sym: Symbol) -> bool {
        use crate::instruction::{Constant, Instruction};
        for inst in sb.bytecode.iter() {
            let hit = match inst {
                Instruction::LoadLocal(s)
                | Instruction::StoreLocal(s)
                | Instruction::StoreLocalKeep(s) => *s == sym,
                Instruction::SendLocal(v, _, _) => *v == sym,
                Instruction::SendLocalLocal(a, b, _, _) => *a == sym || *b == sym,
                Instruction::SendLocalConst(a, _, _, _) => *a == sym,
                Instruction::IntBinLL(a, b, _) | Instruction::DoubleBinLL(a, b, _) => {
                    *a == sym || *b == sym
                }
                Instruction::IntBinLC(a, _, _) | Instruction::DoubleBinLC(a, _, _) => *a == sym,
                _ => false,
            };
            if hit {
                return true;
            }
            if let Instruction::Push(Constant::Block(inner)) = inst
                && Self::block_references(inner, sym)
            {
                return true;
            }
            if let Some((_, _, Some(Constant::Block(inner)))) = inst.send_parts()
                && Self::block_references(inner, sym)
            {
                return true;
            }
        }
        false
    }

    /// Does compiled body bytecode materialize any block literal that
    /// references `sym`?
    fn body_captures(body: &CodeBlock, sym: Symbol) -> bool {
        use crate::instruction::{Constant, Instruction};
        for inst in body.bytecode.iter() {
            if let Instruction::Push(Constant::Block(inner)) = inst
                && Self::block_references(inner, sym)
            {
                return true;
            }
            if let Some((_, _, Some(Constant::Block(inner)))) = inst.send_parts()
                && Self::block_references(inner, sym)
            {
                return true;
            }
        }
        false
    }

    pub(super) fn try_compile_inlined_each(
        &mut self,
        call: &MethodCallNode,
        bytecode: &mut CodeBlock,
    ) -> Result<bool, String> {
        let kws: Vec<&str> = call
            .arguments
            .signature
            .identifiers
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        if kws.as_slice() != ["each"] || call.arguments.expressions.len() != 1 {
            return Ok(false);
        }
        if self.inline_depth >= Self::MAX_INLINE_DEPTH {
            return Ok(false);
        }
        let Some(blk) = Self::fusable_each_block(&call.arguments.expressions[0]) else {
            return Ok(false);
        };

        // Receiver first — evaluated once; both paths read it from the stack.
        match &call.subject {
            Some(s) => self.compile_node(s, bytecode)?,
            None => bytecode.push(Instruction::LoadLocal(
                self.self_override.unwrap_or_else(|| Symbol::intern("self")),
            )),
        }

        // Frame-local temps (fresh names — no collision with user locals).
        fn temp(c: &mut Compiler) -> Symbol {
            let t = c.new_temp_var();
            let sym = Symbol::intern(&t);
            c.scopes.last_mut().unwrap().locals.insert(t);
            sym
        }
        let recv_t = temp(self);
        let n_t = temp(self);
        let i_t = temp(self);
        let x_t = blk.arguments.first().map(|_| temp(self));

        // The body, spliced with the param rebound to its temp. `param_override` is
        // EXTENDED, not replaced: an `each:` block's free names belong to the same
        // scope as the call site (unlike a spliced method body's, whose free names
        // are the callee's).
        let mut body_bc = CodeBlock::new();
        body_bc.current_source = bytecode.current_source.clone();
        let saved_params = self.param_override.clone();
        if let (Some(x), Some(arg)) = (x_t, blk.arguments.first()) {
            self.param_override.insert(arg.identifier.name.clone(), x);
        }
        self.inline_depth += 1;
        // The body re-executes per element: a declaring `if:` arm spliced inside it is
        // loop-repeated (the body itself is declaration-free per `fusable_each_block`).
        self.fused_loop_depth += 1;
        let body_res = self.inline_block_body(blk, &mut body_bc);
        self.fused_loop_depth -= 1;
        self.inline_depth -= 1;
        self.param_override = saved_params;
        body_res?;

        // BUGS.md Finding 10: the fused loop shares ONE param cell across
        // iterations (DefineLocal hoisted, StoreLocal per element), so a
        // closure created in the body captures that single cell and every
        // stashed closure sees the FINAL element. When the body materializes
        // any block literal referencing the param, fall back to the real
        // `each:` send — per-invocation frames give each closure its own
        // binding. The receiver is already on the stack.
        if let Some(x) = x_t
            && Self::body_captures(&body_bc, x)
        {
            self.compile_node(&call.arguments.expressions[0], bytecode)?;
            self.emit_call(bytecode, "each:", 1);
            return Ok(true);
        }

        // Hot path. Layout (jump offsets relative to their own position):
        //   DefineLocal $recv; LoadLocal $recv; ListLen; DefineLocal $n
        //   Push 0; DefineLocal $i; [Push Nil; DefineLocal $x]
        //   HDR: IntBinLL($i < $n)
        //        ElseJump(→EXIT)
        //        [LoadLocal $recv; LoadLocal $i; ListGet; StoreLocal $x]
        //        <body>; Pop
        //        IntBinLC($i + 1); StoreLocal $i; Jump(→HDR)
        //   EXIT: Push Nil; Jump(→JOIN, over the cold path)
        let mut hot = CodeBlock::new();
        hot.current_source = bytecode.current_source.clone();
        hot.push(Instruction::DefineLocal(recv_t));
        hot.push(Instruction::LoadLocal(recv_t));
        hot.push(Instruction::ListLen);
        hot.push(Instruction::DefineLocal(n_t));
        hot.push(Instruction::Push(Constant::Int(0)));
        hot.push(Instruction::DefineLocal(i_t));
        if let Some(x) = x_t {
            hot.push(Instruction::Push(Constant::Nil));
            hot.push(Instruction::DefineLocal(x));
        }
        let hdr = hot.len() as isize;
        hot.push(Instruction::IntBinLL(i_t, n_t, IntBinKind::Lt));
        let elem = if x_t.is_some() { 4isize } else { 0 };
        let per = elem + body_bc.len() as isize + 1 + 3; // + Pop + (IntBinLC, StoreLocal, Jump)
        hot.push(Instruction::ElseJump(per + 1));
        if let Some(x) = x_t {
            hot.push(Instruction::LoadLocal(recv_t));
            hot.push(Instruction::LoadLocal(i_t));
            hot.push(Instruction::ListGet);
            hot.push(Instruction::StoreLocal(x));
        }
        hot.extend(body_bc);
        hot.push(Instruction::Pop);
        hot.push(Instruction::IntBinLC(
            i_t,
            Constant::Int(1),
            IntBinKind::Add,
        ));
        hot.push(Instruction::StoreLocal(i_t));
        let jump_pos = hot.len() as isize;
        hot.push(Instruction::Jump(hdr - jump_pos));
        hot.push(Instruction::Push(Constant::Nil));

        // Cold path: the literal materialized, then the real send.
        let mut cold = CodeBlock::new();
        cold.current_source = bytecode.current_source.clone();
        self.compile_node(&call.arguments.expressions[0], &mut cold)?;
        self.emit_call(&mut cold, "each:", 1);

        hot.push(Instruction::Jump(cold.len() as isize + 1));

        bytecode.push(Instruction::BranchIfNotList(hot.len() as isize + 1));
        bytecode.extend(hot);
        bytecode.extend(cold);
        Ok(true)
    }
}
