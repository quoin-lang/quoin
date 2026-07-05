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
}
