//! Control-flow inlining (`if:`/`whileDo:` → native jumps, Slice 2d) and `List` op
//! devirtualization (Slice 2e).

use super::*;

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

        // Every arg must be a literal, 0-arg, declaration-free block (v1).
        let then_blk = match Self::inlinable_block(&exprs[0]) {
            Some(b) => b,
            None => return Ok(false),
        };
        let else_blk = if has_else {
            match Self::inlinable_block(&exprs[1]) {
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
        self.inline_block_body(then_blk, &mut then_bc)?;
        let t = then_bc.len() as isize;

        if let Some(else_blk) = else_blk {
            let mut else_bc = CodeBlock::new();
            else_bc.current_source = out.current_source.clone();
            self.inline_block_body(else_blk, &mut else_bc)?;
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

    /// A literal block usable for control-flow inlining: no parameters and no local
    /// declarations. (v1 — declaration-carrying blocks need alpha-renaming, a follow-up.)
    ///
    /// A body `var`/`let` is a `Declaration` *statement*, not a `decls` header entry, so
    /// both must be checked: inlining a block that binds a top-level local would splice
    /// that binding into the method scope, colliding with a sibling branch's same-named
    /// local (they are isolated only by their now-absent block frames).
    fn inlinable_block(node: &Node) -> Option<&BlockNode> {
        if let NodeValue::Block(b) = &node.value {
            let declares_local = b
                .statements
                .iter()
                .any(|s| matches!(&s.value, NodeValue::Declaration(_)));
            if b.arguments.is_empty() && b.decls.is_empty() && !declares_local {
                return Some(b);
            }
        }
        None
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

    /// Slice 2d (v2) — inline `{cond}.whileDo:{body}` when both the receiver (`cond`) and
    /// the body are literal, 0-arg, declaration-free blocks, into a native jump loop.
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
        let cond_blk = match Self::inlinable_block(subject) {
            Some(b) => b,
            None => return Ok(false),
        };
        let body_blk = match Self::inlinable_block(&call.arguments.expressions[0]) {
            Some(b) => b,
            None => return Ok(false),
        };

        // Compile cond/body into their own sub-blocks so their lengths size the jumps.
        let mut cond_bc = CodeBlock::new();
        cond_bc.current_source = bytecode.current_source.clone();
        self.inline_block_body(cond_blk, &mut cond_bc)?;
        let c = cond_bc.len() as isize;

        let mut body_bc = CodeBlock::new();
        body_bc.current_source = bytecode.current_source.clone();
        self.inline_block_body(body_blk, &mut body_bc)?;
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
        let body_res = self.inline_block_body(blk, &mut body_bc);
        self.inline_depth -= 1;
        self.param_override = saved_params;
        body_res?;

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
