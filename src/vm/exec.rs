//! The hot path: the step loop and `dispatch_one`'s bytecode arms, send dispatch,
//! the AOT/spec and inline-cache machinery, fused instantiation, cached field
//! load/store, and the arithmetic helpers. One module on purpose: these call each
//! other's private helpers constantly (PERF-SACRED — see git log for the dispatch
//! arms lesson). Extends `VmState`.

use super::*;

impl<'gc> VmState<'gc> {
    /// Materialize a `Constant` into a runtime `Value`. The body of the `Push` handler,
    /// shared with the fused `SendConst` superinstruction.
    fn materialize_constant(&mut self, mc: &Mutation<'gc>, constant: &Constant) -> Value<'gc> {
        match constant {
            Constant::Nil => self.new_nil(mc),
            Constant::Bool(b) => self.new_bool(mc, *b),
            Constant::Int(i) => self.new_int(mc, *i),
            Constant::Double(f) => self.new_double(mc, *f),
            Constant::String(s) => {
                let buf = self.literal_string_buffer(mc, s);
                self.new_string_shared(mc, buf)
            }
            Constant::Symbol(s) => self.new_symbol(mc, s.clone()),
            Constant::Block(sb) => {
                // Constant-closure promotion: a CLOSED template (no captures,
                // no self, no ^^) has one behavioral identity — reuse the
                // per-VM cached closure (shared with compiled make_closure,
                // so baked identity guards stay durable across calls).
                if let Some(tid) = sb.template_id
                    && crate::instruction::template_is_closed(sb)
                    && let Some(&v) = self.aot_closure_cache.get(&tid)
                {
                    return v;
                }
                // A closure is its shared template (Rc bump) plus the captured
                // runtime state — no deep clone of the param vectors.
                let parent_env = self.frames.last().map(|f| f.env);
                let enclosing_method_id = self.frames.last().and_then(|f| f.enclosing_method_id);
                let decl_block = sb.decl_block.as_ref().map(|db| {
                    let inline_cache = self.ic_cell_for(mc, db);
                    gc!(
                        mc,
                        Block {
                            template: db.clone(),
                            parent_env,
                            enclosing_method_id,
                            decl_block: None,
                            inline_cache,
                        }
                    )
                });
                let inline_cache = self.ic_cell_for(mc, sb);
                let block = Block {
                    template: sb.clone(),
                    parent_env,
                    enclosing_method_id,
                    decl_block,
                    inline_cache,
                };
                let v = self.new_block(mc, block);
                if let Some(tid) = sb.template_id
                    && crate::instruction::template_is_closed(sb)
                {
                    self.aot_closure_cache.insert(tid, v);
                }
                v
            }
        }
    }

    /// Read instance field `name` off `self` in the current frame. The body of the
    /// `LoadField` handler, shared with the fused `SendField` superinstruction.
    /// Missing/undeclared field (or a non-object `self`) reads as nil.
    /// `cache_ip`: the call site for the field-slot cache, or `None` to skip caching —
    /// `SendField` must pass `None`, because its *send* entry lives at the same `ip`
    /// (one fused instruction) and a field entry there would thrash the slot.
    /// `load_field` for compiled frames (S3): the receiver comes from the
    /// frame's slot window and the slot cache is the SHARED `(template_id,
    /// ip)` cell — both tiers warm one cache, the B3a outcall lesson applied
    /// to fields. Missing/undeclared/non-object reads are nil, exactly as
    /// interpreted.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // called from AOT-compiled code
    pub(crate) fn field_load_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        self_val: Value<'gc>,
        name: &str,
    ) -> Value<'gc> {
        let ic = self.ic_cell_by_id(mc, tid);
        if let Value::Object(obj) = self_val {
            let borrowed = obj.borrow();
            let class = borrowed.class;
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize) {
                let val = borrowed.fields.get(slot).copied();
                drop(borrowed);
                return val.unwrap_or_else(|| self.new_nil(mc));
            }
            drop(borrowed);
            match self.field_slot(class, name) {
                Some(slot) => {
                    self.field_fill_cell(mc, ic, bc_len, ip, class, slot);
                    obj.borrow()
                        .fields
                        .get(slot)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                }
                None => self.new_nil(mc),
            }
        } else {
            self.new_nil(mc)
        }
    }

    /// `store_field_value` for compiled frames (S3) — same shared-cell cache,
    /// same declared-field errors as interpreted.
    #[allow(clippy::too_many_arguments)] // cached field-store fast path threads receiver/value/cache state
    pub(crate) fn field_store_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        self_val: Value<'gc>,
        name: &str,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let ic = self.ic_cell_by_id(mc, tid);
        if let Value::Object(obj) = self_val {
            let class = obj.borrow().class;
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
                && slot < obj.borrow().fields.len()
            {
                obj.borrow_mut(mc).fields[slot] = val;
                return Ok(());
            }
            match self.field_slot(class, name) {
                Some(slot) if slot < obj.borrow().fields.len() => {
                    self.field_fill_cell(mc, ic, bc_len, ip, class, slot);
                    obj.borrow_mut(mc).fields[slot] = val;
                    Ok(())
                }
                Some(_) => Err(QuoinError::Other(format!(
                    "Instance of '{}' has no '@{}' (it was added after this instance was created)",
                    class.borrow().name,
                    name
                ))),
                None => Err(QuoinError::Other(format!(
                    "No instance variable '@{}' declared on '{}'",
                    name,
                    class.borrow().name
                ))),
            }
        } else {
            Err(QuoinError::Other(format!(
                "Cannot set instance variable '@{}' on a value type ({})",
                name,
                self_val.type_name()
            )))
        }
    }

    fn load_field(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        cache_ip: Option<usize>,
        name: &str,
    ) -> Value<'gc> {
        let frame = &self.frames[frame_idx];
        let block = frame.block;
        let ic = frame.ic;
        let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
        self.field_of(mc, block, ic, cache_ip, self_val, name)
    }

    /// Read instance field `name` off an arbitrary object value (the body of `LoadFieldOf`, and
    /// shared by `load_field` with `self`). Missing/undeclared field, or a non-object value => nil.
    /// `(block, ip)` is the executing call site, for the field-slot cache.
    fn field_of(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ic: InlineCacheCell<'gc>,
        cache_ip: Option<usize>,
        obj_val: Value<'gc>,
        name: &str,
    ) -> Value<'gc> {
        if let Value::Object(obj) = obj_val {
            // Fast path: one object borrow, one cache probe, direct index — no class
            // borrow, no field-name hash.
            let borrowed = obj.borrow();
            let class = borrowed.class;
            if let Some(ip) = cache_ip
                && let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
            {
                let val = borrowed.fields.get(slot).copied();
                drop(borrowed);
                return val.unwrap_or_else(|| self.new_nil(mc));
            }
            drop(borrowed);
            // No slot (undeclared) or a slot past this instance's array (declared on the
            // class after this object was created) => nil.
            match self.field_slot(class, name) {
                Some(slot) => {
                    if let Some(ip) = cache_ip {
                        self.field_fill(mc, block, ip, class, slot);
                    }
                    obj.borrow()
                        .fields
                        .get(slot)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                }
                None => self.new_nil(mc),
            }
        } else {
            self.new_nil(mc)
        }
    }

    /// Execute a send: pop `num_args` then the receiver off the stack and dispatch
    /// `selector`. Shared by the `Send` handler and the fused `Send*` superinstructions
    /// (which push the send's last operand first). Advances the caller frame's ip by one
    /// slot, then either tail-starts a block, invokes the resolved callable, or raises MNU.
    /// Fast path for a devirtualized Integer op (Slice 2a/2f): if the top two stack values
    /// are both `Int`, pop them and return `Some((a, b))`; otherwise leave them in place and
    /// return `None` so the caller falls back to the real send. This optimistic fallback is
    /// what lets `Int` be *inferred* for a mutable `var` (a stale-typed var is handled by the
    /// send) instead of only trusted for annotated params.
    fn take_two_ints(&mut self) -> Option<(i64, i64)> {
        let n = self.stack.len();
        if n < 2 {
            return None;
        }
        if let (Value::Int(a), Value::Int(b)) = (self.stack[n - 2], self.stack[n - 1]) {
            self.stack.truncate(n - 2);
            Some((a, b))
        } else {
            None
        }
    }

    /// Like `take_two_ints`, but for the `Double` devirt arms: pops the top two values iff both
    /// are `Value::Double`, else leaves the stack untouched so the caller can fall back to a send.
    fn take_two_doubles(&mut self) -> Option<(f64, f64)> {
        let n = self.stack.len();
        if n < 2 {
            return None;
        }
        if let (Value::Double(a), Value::Double(b)) = (self.stack[n - 2], self.stack[n - 1]) {
            self.stack.truncate(n - 2);
            Some((a, b))
        } else {
            None
        }
    }

    /// The fused-`Int`-op computation (Slice a1), shared by `IntBinLL`/`IntBinLC`. Matches the
    /// standalone `IntAdd`..`IntNe` arms exactly (arith wraps in release; `/`/`%` raise on a
    /// zero divisor; compares yield a Bool).
    fn int_bin_compute(kind: IntBinKind, a: i64, b: i64) -> Result<Value<'gc>, QuoinError> {
        Ok(match devirt_ops::int_bin(kind, a, b)? {
            devirt_ops::IntBinOut::Int(i) => Value::Int(i),
            devirt_ops::IntBinOut::Bool(b) => Value::Bool(b),
        })
    }

    /// The fused-`Double`-op computation, shared by `DoubleBinLL`/`DoubleBinLC`. Plain IEEE-754
    /// f64 — `/`/`%` yield inf/NaN on a zero divisor (never raise, unlike `int_bin_compute`), so
    /// it returns a `Value` directly rather than a `Result`.
    fn double_bin_compute(kind: IntBinKind, a: f64, b: f64) -> Value<'gc> {
        match devirt_ops::double_bin(kind, a, b) {
            devirt_ops::DoubleBinOut::Double(d) => Value::Double(d),
            devirt_ops::DoubleBinOut::Bool(b) => Value::Bool(b),
        }
    }

    /// Register unit-load AOT candidates (S0): classic annotated methods
    /// compile eagerly, block templates and speculative methods go pending
    /// (blocks tier by invocation count at the vWSOA seams; speculative
    /// methods first OBSERVE their param/return kinds here).
    pub fn register_aot_candidates(&mut self, cands: Vec<crate::codegen::AotCandidate>) {
        use crate::codegen::{AotRole, spec};
        let mut immediate = Vec::new();
        for cand in cands {
            let Some(tid) = cand.block.template_id else {
                continue;
            };
            if cand.role == AotRole::BlockTemplate {
                self.aot_pending_blocks
                    .insert(tid, (0, spec::K_UNKNOWN, cand));
            } else if cand.speculative() {
                cand.block.spec_state.set(spec::OBSERVING);
                let n_params = cand.params.len();
                self.aot_pending_spec.insert(
                    tid,
                    spec::SpecPending {
                        count: 0,
                        param_kinds: vec![spec::K_UNKNOWN; n_params],
                        ret_kind: spec::K_UNKNOWN,
                        cand,
                    },
                );
            } else {
                immediate.push(cand);
            }
        }
        if !immediate.is_empty() {
            crate::codegen::compile_candidates(immediate);
        }
    }

    /// The kind lattice value of a runtime value (spec-AOT observation).
    fn spec_kind(v: Value<'gc>) -> u8 {
        crate::codegen::spec::kind_of(v)
    }

    /// Merge a method entry's arg kinds into its speculative profile and
    /// return the tid for the frame to stash (`Frame.spec_tid`) — so the
    /// pop-side return observation never re-chases the template. Called on
    /// every method-frame push; the common case (template not OBSERVING) is
    /// one bounds-checked byte load.
    /// Cold path: the caller has already checked the template's `spec_state`
    /// Cell (the hot-path gate is inline at the push site). Returns the tid
    /// for `Frame.spec_tid`, or 0.
    #[cold]
    pub(super) fn spec_observe_entry(
        &mut self,
        template: &Arc<StaticBlock>,
        args: &[Value<'gc>],
    ) -> u32 {
        use crate::codegen::spec;
        let Some(tid) = template.template_id else {
            return 0;
        };
        let Some(p) = self.aot_pending_spec.get_mut(&tid) else {
            return 0;
        };
        for (lat, arg) in p.param_kinds.iter_mut().zip(args.iter()) {
            *lat = spec::merge(*lat, Self::spec_kind(*arg));
        }
        p.count += 1;
        self.aot_spec_obs_left -= 1;
        // A speculated RETURN needs at least one observed return before
        // promotion — a recursive method reaches warmth by ENTRIES alone
        // (fib descends past the threshold before its first base case), and
        // promoting with an unknown ret would compile Obj forever. Cap the
        // wait so a genuinely non-returning-yet method still promotes.
        let ret_pending = p.cand.spec_ret && p.ret_kind == spec::K_UNKNOWN;
        if p.count >= crate::codegen::warm_threshold()
            && (!ret_pending || p.count >= spec::OBSERVE_CAP)
        {
            self.spec_promote(tid);
            return 0; // promoted (or refused): no frame stash needed
        }
        tid
    }

    /// S1 promotion: compile a warm speculative method with its OBSERVED
    /// kinds. Scalar observations become the compiled params AND entry
    /// preconditions (checked by the dispatch arm; mismatch Bails to the
    /// interpreted body); `Obj`/unknown observations ride as Obj with no
    /// check. Annotated params were never speculated — dispatch guarantees
    /// them, exactly as before. The method-cache epoch bumps so call sites
    /// whose inline caches hold the interpreted callable re-fill with the
    /// compiled entry.
    fn spec_promote(&mut self, tid: u32) {
        use crate::codegen::spec;
        // Bisection debug hooks (they found every S1 seam bug):
        // QN_AOT_SPEC_MAX=<n> promotes only tids <= n;
        // QN_AOT_SPEC_ONLY=<csv> promotes only the listed tids.
        if let Ok(max) = std::env::var("QN_AOT_SPEC_MAX")
            && max.parse::<u32>().map(|m| tid > m).unwrap_or(false)
        {
            return;
        }
        if let Ok(only) = std::env::var("QN_AOT_SPEC_ONLY")
            && !only.split(',').any(|t| t.trim() == tid.to_string())
        {
            return;
        }
        let Some(pending) = self.aot_pending_spec.remove(&tid) else {
            return;
        };
        let mut cand = pending.cand;
        cand.block.spec_state.set(spec::RESOLVED);
        let mut preconds = vec![None; cand.params.len()];
        // `i` indexes four parallel arrays (spec_params, params, param_kinds,
        // preconds) and mutates `cand.params[i]` through `cand`, so an iterator
        // over one of them would borrow-conflict with the writes.
        #[allow(clippy::needless_range_loop)]
        for i in 0..cand.params.len() {
            if cand.spec_params[i]
                && let Some(kind) = spec::scalar_kind(*pending.param_kinds.get(i).unwrap_or(&0))
            {
                cand.params[i] = crate::codegen::AotParam::Scalar(kind);
                preconds[i] = Some(kind);
            }
        }
        // S2: an observed-scalar RETURN compiles as a scalar too — statically
        // verified (a return path the translator can't prove demotes the ret
        // back to Obj and retries; no runtime narrowing, no wrong-type error
        // the interpreter wouldn't raise).
        if cand.spec_ret
            && let Some(kind) = spec::scalar_kind(pending.ret_kind)
        {
            cand.ret = crate::codegen::AotRet::Scalar(kind);
        }
        if preconds.iter().any(|p| p.is_some()) {
            cand.spec_preconditions = preconds;
        }
        let sel = cand.selector.clone();
        crate::codegen::compile_candidates(vec![cand]);
        if crate::codegen::block_registered(tid) {
            if std::env::var("QN_AOT_VERBOSE").is_ok_and(|v| v == "1") {
                eprintln!("qn aot: promoted {sel} (tid {tid})");
            }
            self.aot_spec_promoted += 1;
            self.invalidate_method_cache();
        }
    }

    /// Merge a method's return kind into its speculative profile. `tid` comes
    /// from the popped frame's `spec_tid` (set at push), so this only runs
    /// for frames that were observing; the state re-check tolerates
    /// saturation between push and pop.
    #[cold]
    fn spec_observe_return(&mut self, tid: u32, ret: Value<'gc>) {
        use crate::codegen::spec;
        if let Some(p) = self.aot_pending_spec.get_mut(&tid) {
            p.ret_kind = spec::merge(p.ret_kind, Self::spec_kind(ret));
        }
    }

    /// One-line profile summary for `QN_AOT_STATS=1`.
    pub fn aot_spec_stats(&self) -> String {
        use crate::codegen::spec;
        let observing = self
            .aot_pending_spec
            .values()
            .filter(|p| p.cand.block.spec_state.get() == spec::OBSERVING)
            .count();
        let (compiled, refused) = crate::codegen::compile_totals();
        let mut lines = vec![format!(
            "spec-aot: {} pending ({} observing), {} promoted; {} compiled, {} refused (QN_AOT_VERBOSE=1 for reasons)",
            self.aot_pending_spec.len(),
            observing,
            self.aot_spec_promoted,
            compiled,
            refused
        )];
        let mut profiled: Vec<_> = self
            .aot_pending_spec
            .values()
            .filter(|p| p.count > 0)
            .collect();
        profiled.sort_by_key(|p| std::cmp::Reverse(p.count));
        for p in profiled.iter().take(12) {
            let kinds: Vec<&str> = p.param_kinds.iter().map(|&k| spec::kind_name(k)).collect();
            lines.push(format!(
                "  {} x{}: ({}) -> {}",
                p.cand.selector,
                p.count,
                kinds.join(", "),
                spec::kind_name(p.ret_kind)
            ));
        }
        lines.join("\n")
    }

    /// Materialize a runtime closure from a compiled template: the thin
    /// `{template, captured state}` pair plus the (possibly registry-shared)
    /// inline-cache cell. Shared by the runner entry points, eval, and string
    /// interpolation; `materialize_constant` inlines the same shape.
    pub fn block_from_template(
        &mut self,
        mc: &Mutation<'gc>,
        template: Arc<StaticBlock>,
        parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
        enclosing_method_id: Option<usize>,
    ) -> Gc<'gc, Block<'gc>> {
        let decl_block = template.decl_block.as_ref().map(|db| {
            let inline_cache = self.ic_cell_for(mc, db);
            gc!(
                mc,
                Block {
                    template: db.clone(),
                    parent_env,
                    enclosing_method_id,
                    decl_block: None,
                    inline_cache,
                }
            )
        });
        let inline_cache = self.ic_cell_for(mc, &template);
        gc!(
            mc,
            Block {
                template,
                parent_env,
                enclosing_method_id,
                decl_block,
                inline_cache,
            }
        )
    }

    /// The inline-cache cell for a closure materialized from `template`: the shared
    /// per-template cell from `ic_registry` when the template has an id (so every
    /// closure of one literal warms the same call sites), or a fresh private cell
    /// for id-less runtime-built blocks.
    pub(crate) fn ic_cell_for(
        &mut self,
        mc: &Mutation<'gc>,
        template: &Arc<StaticBlock>,
    ) -> Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>> {
        match template.template_id {
            Some(id) => {
                if let Some(cell) = self.ic_registry.get(&id) {
                    *cell
                } else {
                    let cell = gcl!(mc, None);
                    self.ic_registry.insert(id, cell);
                    cell
                }
            }
            None => gcl!(mc, None),
        }
    }

    /// The shared IC cell for a template id directly — the compiled-code twin of
    /// `ic_cell_for` (outcall sites pass their `(template_id, ip)`, which is the
    /// same call-site identity the interpreted send at that instruction uses, so
    /// compiled and interpreted execution warm ONE cache).
    pub(crate) fn ic_cell_by_id(
        &mut self,
        mc: &Mutation<'gc>,
        id: u32,
    ) -> Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>> {
        if let Some(cell) = self.ic_registry.get(&id) {
            *cell
        } else {
            let cell = gcl!(mc, None);
            self.ic_registry.insert(id, cell);
            cell
        }
    }

    /// Block-call site peek (D2-for-blocks): the identity is the block's
    /// TEMPLATE id (every closure shares the `Block` class, so the method
    /// cells' receiver-class guard would alias all of them). Returns the
    /// cached entry when live.
    #[inline]
    pub(crate) fn aot_block_site_peek(
        &self,
        site: usize,
        template_id: u32,
    ) -> Option<(&'static crate::codegen::AotEntry, u32)> {
        let cell = self.aot_sites.get(site)?;
        let entry = cell.entry?;
        if cell.epoch != self.dispatch_epoch || entry.template_id != template_id {
            return None;
        }
        Some((entry, cell.hits))
    }

    /// Fill a block-call site cell (entry + epoch only — the template-id
    /// guard lives on the entry itself).
    pub(crate) fn aot_block_site_fill(
        &mut self,
        site: usize,
        entry: &'static crate::codegen::AotEntry,
        recv: Value<'gc>,
    ) {
        if site >= self.aot_sites.len() {
            self.aot_sites.resize(site + 1, AotSiteCell::default());
        }
        let cell = &mut self.aot_sites[site];
        *cell = AotSiteCell::default();
        cell.epoch = self.dispatch_epoch;
        cell.entry = Some(entry);
        cell.recv_val = Some(recv);
    }

    /// Receiver-phase probe of a D2 site cell: live epoch + receiver guard,
    /// checked BEFORE the caller decodes any argument lanes, so a site whose
    /// target is not compiled (a native, a polymorphic receiver) pays a few
    /// loads and nothing else. Returns the cell BY COPY (it is `Copy`; the
    /// `parent_env` Gc stays rooted in the traced `aot_sites` vec) for the
    /// argument-phase check.
    #[inline]
    pub(crate) fn aot_site_peek(
        &self,
        site: usize,
        receiver: Value<'gc>,
        n_args: usize,
    ) -> Option<AotSiteCell<'gc>> {
        let cell = self.aot_sites.get(site)?;
        cell.entry?;
        if cell.epoch != self.dispatch_epoch || cell.n_args as usize != n_args {
            return None;
        }
        let (rk, rp) = value_type_guard(receiver);
        if cell.recv_kind != rk || cell.recv_ptr != rp {
            return None;
        }
        Some(*cell)
    }

    /// Argument-phase check for a peeked D2 cell (see [`Self::aot_site_peek`]).
    #[inline]
    /// One lane of [`aot_site_args_match`] — the D2.5b helper fast path
    /// guards verbatim scalar lanes by lane-kind compare and only routes
    /// GENERAL lanes (Obj / precondition-narrowed) through this shape guard.
    pub(crate) fn aot_site_arg_match_one(cell: &AotSiteCell<'gc>, i: usize, a: Value<'gc>) -> bool {
        let (ak, ap) = value_type_guard(a);
        cell.arg_kinds[i] == ak && cell.arg_ptrs[i] == ap
    }

    /// D3a: count a fast-path hit; crossing the `QN_DIRECT_WARM` threshold
    /// queues the CALLER tid for retranslation (deduped, process-lifetime).
    #[inline(always)]
    pub(crate) fn aot_site_note_hit(&mut self, site: usize, caller_tid: u32) {
        let Some(threshold) = crate::codegen::direct_warm_threshold() else {
            return;
        };
        let Some(cell) = self.aot_sites.get_mut(site) else {
            return;
        };
        // Saturate at the threshold: a warm site's hits become a read-only
        // compare — the unconditional per-hit WRITE dirtied the cell's cache
        // line millions of times on call-heavy programs (measured ~2% on
        // richards even with the counter inlined).
        if cell.hits >= threshold {
            return;
        }
        cell.hits += 1;
        if cell.hits == threshold && self.aot_retranslate_queued.insert(caller_tid) {
            self.aot_retranslate_queue.push(caller_tid);
        }
    }

    /// Drain the retranslation queue (driver-boundary caller).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // AOT-driver-only (native)
    pub(crate) fn take_retranslations(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.aot_retranslate_queue)
    }

    /// D3b activation, the TARGETED form: clear exactly the caches holding
    /// `tid`'s (now replaced) entry — its D2 site cells and interpreted IC
    /// slots — so the next resolution refills from the registry and picks
    /// up the retranslated code. Everything else stays warm, and earlier
    /// batches' baked guards stay LIVE (the wholesale dispatch-epoch bump
    /// this replaces stranded every prior batch's edges and re-warmed the
    /// world per batch — measured btrees +3.2%/richards +3.7%). Runs at the
    /// driver boundary; O(total cached slots), rare.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // AOT-driver-only (native)
    pub(crate) fn invalidate_caches_for_template(&mut self, mc: &Mutation<'gc>, tid: u32) {
        for cell in &mut self.aot_sites {
            if cell.entry.is_some_and(|e| e.template_id == tid) {
                *cell = AotSiteCell::default();
            }
        }
        for ic in self.ic_registry.values() {
            let mut slots = ic.borrow_mut(mc);
            if let Some(slots) = slots.as_mut() {
                for slot in slots.iter_mut() {
                    let stale = matches!(
                        &slot.callable,
                        Some(crate::dispatch::Callable::AotCall { entry, .. })
                            if entry.0.template_id == tid
                    );
                    if stale {
                        *slot = ICSlot::empty();
                    }
                }
            }
        }
    }

    /// D3b: capture baked W0 facts for a caller's retained sites — warm,
    /// current-epoch, monomorphic cells whose entry meets the W0 tier
    /// criteria. Runs in the driver's drain (the translator has no VM).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // AOT-driver-only (native)
    pub(crate) fn bake_w0_sites(
        &self,
        sites: &rustc_hash::FxHashMap<usize, u32>,
        threshold: u32,
    ) -> (
        rustc_hash::FxHashMap<usize, crate::codegen::BakedW0>,
        Vec<Value<'gc>>,
    ) {
        let mut out = rustc_hash::FxHashMap::default();
        let mut roots = Vec::new();
        for (&ip, &site) in sites {
            let Some(cell) = self.aot_sites.get(site as usize) else {
                continue;
            };
            let Some(entry) = cell.entry else { continue };
            let is_block = crate::codegen::block_w0_eligible(entry);
            let eligible = crate::codegen::w0_eligible(entry) || is_block;
            if cell.epoch != self.dispatch_epoch || cell.hits < threshold || !eligible {
                continue;
            }
            if is_block {
                // Identity bake: the guard compares the receiver slot's 16
                // bytes against this exact closure NATIVELY (fixed Value
                // layout). Pin the closure for the code's lifetime — a
                // recycled address must never false-positive the guard.
                let Some(rv) = cell.recv_val else { continue };
                let Value::Object(obj) = rv else { continue };
                roots.push(rv);
                out.insert(
                    ip,
                    crate::codegen::BakedW0 {
                        entry,
                        epoch: self.dispatch_epoch,
                        recv_kind: 4, // Value tag: Object
                        recv_ptr: Gc::as_ptr(obj) as usize,
                    },
                );
                continue;
            }
            out.insert(
                ip,
                crate::codegen::BakedW0 {
                    entry,
                    epoch: self.dispatch_epoch,
                    recv_kind: cell.recv_kind,
                    recv_ptr: cell.recv_ptr,
                },
            );
        }
        (out, roots)
    }

    /// Fill a D2 site cell. The caller gates this on the interpreted IC
    /// having filled for the same resolution (probe-after-fill), which
    /// carries over every cacheability rule (guard-free, non-eigenclass,
    /// arg-count bound) without restating them.
    pub(crate) fn aot_site_fill(
        &mut self,
        site: usize,
        receiver: Value<'gc>,
        args: &[Value<'gc>],
        entry: &'static crate::codegen::AotEntry,
        parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        if site >= self.aot_sites.len() {
            self.aot_sites.resize(site + 1, AotSiteCell::default());
        }
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        self.aot_sites[site] = AotSiteCell {
            epoch: self.dispatch_epoch,
            hits: 0,
            recv_kind,
            recv_ptr,
            n_args: args.len() as u8,
            arg_kinds,
            arg_ptrs,
            entry: Some(entry),
            parent_env,
            recv_val: None,
        };
    }

    /// `call_method`, with the caller's inline cache consulted and filled — the
    /// compiled-code outcall path (B3a): without it every compiled operator send
    /// paid an uncached `lookup_method` while the interpreted body it replaced
    /// had warm ICs, which measurably REGRESSED arithmetic-heavy blocks.
    #[allow(clippy::too_many_arguments)] // inline-cache dispatch fast path; every param is on the hot call boundary
    pub fn call_method_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: Vec<Value<'gc>>,
        site: Option<u32>,
    ) -> Result<Value<'gc>, QuoinError> {
        // No `enter_native_reentry` here (unlike `call_method`): charging the
        // 12-deep hook-recursion budget per outcall made a 12-deep chain of
        // PROMOTED methods (S1: everything unannotated compiles) a spurious
        // "recursion too deep". Instead, `outcall_nesting` counts the REAL
        // hazard — Rust-stack frames per compiled<->interpreted alternation —
        // and dispatch degrades to interpreted bodies past the cap.
        self.aot.outcall_nesting += 1;
        let result =
            self.call_method_cached_inner(mc, tid, ip, bc_len, receiver, selector, args, site);
        self.aot.outcall_nesting = self.aot.outcall_nesting.saturating_sub(1);
        result
    }

    // The IC cell local is a copy of a `Gc` rooted in the traced `ic_registry`
    // for the VM's whole life — safe across `lookup_method`'s guard-predicate
    // yields by that rooting, which the span heuristic can't see.
    #[allow(no_gc_across_yield)]
    #[allow(clippy::too_many_arguments)] // inner half of the IC dispatch fast path
    fn call_method_cached_inner(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: Vec<Value<'gc>>,
        site: Option<u32>,
    ) -> Result<Value<'gc>, QuoinError> {
        let ic = self.ic_cell_by_id(mc, tid);
        let method = match self.ic_probe(ic, ip, receiver, &args) {
            Some(c) => {
                // D2 gap (found by D3b): a caller that TIERS UP mid-run has
                // warm interpreted ICs, so the cold-arm fill below never
                // runs and its site cells stay cold forever — the D2 fast
                // path was inert for every spec-promoted caller. Fill on a
                // probe-hit too, under the same once-per-epoch gate (the
                // polymorphic-flip tax the cold-arm comment guards against
                // stays impossible: a warm cell short-circuits here).
                if let (Some(site), crate::dispatch::Callable::AotCall { block, entry }) =
                    (site, &c)
                    && self.aot_sites.get(site as usize).is_none_or(|cell| {
                        cell.entry.is_none() || cell.epoch != self.dispatch_epoch
                    })
                {
                    self.aot_site_fill(site as usize, receiver, &args, entry.0, block.parent_env);
                }
                Some(c)
            }
            None => {
                let m = self.lookup_method(mc, receiver, selector, &args)?;
                if let Some(c) = &m {
                    self.ic_fill_cell(mc, ic, bc_len, ip, receiver, selector, &args, *c);
                    // D2: mirror the resolution into the site cell — but only
                    // when the IC actually filled (probe-after-fill), so the
                    // site cache inherits ic_fill_cell's cacheability rules;
                    // and only ONCE PER EPOCH per site — a polymorphic site
                    // re-resolves cold on every receiver flip, and re-running
                    // the probe + rewriting the cell each time taxed exactly
                    // the sites that can never benefit.
                    if let (Some(site), crate::dispatch::Callable::AotCall { block, entry }) =
                        (site, c)
                        && self.aot_sites.get(site as usize).is_none_or(|cell| {
                            cell.entry.is_none() || cell.epoch != self.dispatch_epoch
                        })
                        && self.ic_probe(ic, ip, receiver, &args).is_some()
                    {
                        self.aot_site_fill(
                            site as usize,
                            receiver,
                            &args,
                            entry.0,
                            block.parent_env,
                        );
                    }
                }
                m
            }
        };
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            if matches!(
                method,
                crate::dispatch::Callable::Native(_) | crate::dispatch::Callable::AotCall { .. }
            ) {
                // Same stack-window rooting as `exec_send` (A2c): outcall
                // args arrive in an owned Vec (decoded from compiled lanes,
                // never on the value stack), so push them once — two stack
                // writes beat the rooting clone. The frame-count
                // discriminator below is exact: a synchronous call pushes no
                // frame; the AotCall interpreter fallbacks consume the
                // window themselves before pushing theirs.
                let recv_start = self.stack.len();
                self.push(receiver);
                for &a in &args {
                    self.push(a);
                }
                let res = method.call(
                    self,
                    mc,
                    Some(receiver),
                    args,
                    Some(selector),
                    Some(recv_start + 1),
                );
                if let Err(e) = res {
                    // The S1/finish_frame rule, as in `dispatch_send_rooted`:
                    // an escaping `^^` already delivered at (possibly) the
                    // window start — only non-NLR errors tear down here.
                    if !matches!(e, QuoinError::NonLocalReturn) {
                        self.stack.truncate(recv_start.min(self.stack.len()));
                    }
                    return Err(e);
                }
                if self.frames.len() > initial_frame_count {
                    // An interpreter fallback started a frame (window
                    // already consumed by the dispatch arm): drive it.
                    self.run_nested(mc, initial_frame_count, "method call")?;
                    Ok(self.pop()?)
                } else {
                    let result = self.pop()?;
                    self.stack.truncate(recv_start);
                    Ok(result)
                }
            } else {
                method.call(self, mc, Some(receiver), args, Some(selector), None)?;
                self.run_nested(mc, initial_frame_count, "method call")?;
                Ok(self.pop()?)
            }
        } else {
            // No method: raise EXACTLY what the interpreted send raises
            // (candidates included). This arm returned nil since the first
            // outcall shell, which made a warm compiled outcall silently
            // "succeed" where the same interpreted send raised
            // MessageNotUnderstood — a parity hole that hid real errors the
            // moment a block template or promoted method warmed up.
            // (`call_method_inner` — the native `call_method` helper — still
            // has the legacy nil arm: its callers are host-ops and hooks
            // with their own absent-method conventions, out of scope here.)
            let candidates = self
                .collect_method_candidates(receiver, selector)
                .iter()
                .map(|&mv| self.format_candidate_signature(mv, selector))
                .collect();
            let receiver_name = receiver.class_name();
            let arg_names = args.iter().map(|a| a.class_name()).collect();
            self.exceptions.last_send_args = args;
            Err(QuoinError::MessageNotUnderstood {
                receiver: receiver_name,
                selector: selector.as_str().to_string(),
                args: arg_names,
                candidates,
            })
        }
    }

    /// Probe the executing `block`'s inline cache at `ip` for a *field-slot* entry
    /// (see [`IC_FIELD_KIND`]): a hit returns the receiver-class's slot index for the
    /// field named at this instruction, skipping the `field_slots` hash lookup and
    /// the class borrow. Guarded on the exact class pointer — inherited methods run
    /// the same `Gc<Block>` for every subclass, and the same field name maps to a
    /// *different* slot per class, so the guard is load-bearing.
    #[inline]
    fn field_probe(&self, ic: InlineCacheCell<'gc>, ip: usize, class_ptr: usize) -> Option<usize> {
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch
            || slot.recv_kind != IC_FIELD_KIND
            || slot.recv_ptr != class_ptr
        {
            return None;
        }
        Some(slot.arg_ptrs[0])
    }

    /// Memoize `class`'s slot index for the field read/written at `(block, ip)`.
    /// Eigenclasses are never cached (transient pointers — same ABA rule as the
    /// dispatch cache); their accesses just re-run the hash lookup. Slot indices are
    /// append-only per class (see `Class::field_slots`), so a cached entry can't go
    /// stale; the epoch guard is belt-and-braces and gives O(1) invalidation anyway.
    /// `field_fill` for a cell reached by template id (compiled field access,
    /// S3) — same slot-cache protocol, shared with the interpreted site.
    fn field_fill_cell(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        slot_idx: usize,
    ) {
        if class.borrow().is_eigenclass {
            return;
        }
        Self::ic_write_slot(
            mc,
            cell,
            bc_len,
            ip,
            ICSlot {
                epoch: self.dispatch_epoch,
                recv_kind: IC_FIELD_KIND,
                recv_ptr: Gc::as_ptr(class) as usize,
                n_args: 0,
                arg_kinds: [0; IC_MAX_ARGS],
                arg_ptrs: [slot_idx, 0],
                callable: None,
            },
        );
    }

    fn field_fill(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ip: usize,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        slot_idx: usize,
    ) {
        if class.borrow().is_eigenclass {
            return;
        }
        let epoch = self.dispatch_epoch;
        let mut cache = block.inline_cache.borrow_mut(mc);
        if cache.is_none() {
            *cache = Some(vec![ICSlot::empty(); block.template.bytecode.len()].into_boxed_slice());
        }
        if let Some(slot) = cache.as_mut().and_then(|slots| slots.get_mut(ip)) {
            *slot = ICSlot {
                epoch,
                recv_kind: IC_FIELD_KIND,
                recv_ptr: Gc::as_ptr(class) as usize,
                n_args: 0,
                arg_kinds: [0; IC_MAX_ARGS],
                arg_ptrs: [slot_idx, 0],
                callable: None,
            };
        }
    }

    /// Probe a site's cache for a fused-instantiation verdict (`IC_PLAINNEW_KIND`):
    /// a hit is (class-ptr, epoch)-guarded, same protocol as `field_probe`.
    #[inline]
    fn plain_new_probe(
        &self,
        ic: InlineCacheCell<'gc>,
        ip: usize,
        class_ptr: usize,
    ) -> Option<bool> {
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch
            || slot.recv_kind != IC_PLAINNEW_KIND
            || slot.recv_ptr != class_ptr
        {
            return None;
        }
        Some(slot.arg_ptrs[0] != 0)
    }

    /// Does any class in the class-side chain (own, ancestors, mixins —
    /// transitively) define a `new:` method? Over-approximates dispatch on
    /// purpose: a typed user variant that would NOT match a Block argument
    /// still answers true here, which only sends the site to the cold path
    /// (the real send then falls through to `Callable::New` exactly as today).
    fn hierarchy_defines_class_new(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> bool {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class)) {
            return false;
        }
        visited.push(class);
        let c = class.borrow();
        if c.class_methods.contains_key(&new_colon_symbol()) {
            return true;
        }
        if let Some(parent) = c.parent
            && self.hierarchy_defines_class_new(parent, visited)
        {
            return true;
        }
        c.mixin_classes
            .iter()
            .any(|m| self.hierarchy_defines_class_new(*m, visited))
    }

    /// The fused-instantiation verdict (M2, `BranchIfNotPlainNew`): does `new:`
    /// on this receiver resolve to the BUILT-IN `Callable::New` — the fallback
    /// `lookup_method` returns only when NO user `new:` exists anywhere in the
    /// class-side chain — with an instantiable class? False sends the site to
    /// the cold path (the real send), so a conservative false is never wrong.
    fn plain_new_verdict(&self, receiver: Value<'gc>) -> bool {
        let Value::Class(class) = receiver else {
            return false;
        };
        let mut visited = Vec::new();
        if self.hierarchy_defines_class_new(class, &mut visited) {
            return false;
        }
        self.ensure_instantiable(class).is_ok()
    }

    /// Cached `plain_new_verdict`: probe/fill `cell` at `ip` when the receiver
    /// is a (non-eigenclass) class; other receivers recompute (always false).
    pub(crate) fn plain_new_check_cached(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Option<(InlineCacheCell<'gc>, usize)>,
        ip: usize,
        receiver: Value<'gc>,
    ) -> bool {
        if let (Some((cell, _)), Value::Class(class)) = (cell, receiver)
            && let Some(v) = self.plain_new_probe(cell, ip, Gc::as_ptr(class) as usize)
        {
            return v;
        }
        let verdict = self.plain_new_verdict(receiver);
        if let (Some((cell, bc_len)), Value::Class(class)) = (cell, receiver)
            && !class.borrow().is_eigenclass
        {
            Self::ic_write_slot(
                mc,
                cell,
                bc_len,
                ip,
                ICSlot {
                    epoch: self.dispatch_epoch,
                    recv_kind: IC_PLAINNEW_KIND,
                    recv_ptr: Gc::as_ptr(class) as usize,
                    n_args: 0,
                    arg_kinds: [0; IC_MAX_ARGS],
                    arg_ptrs: [usize::from(verdict), 0],
                    callable: None,
                },
            );
        }
        verdict
    }

    /// The fused-instantiation body (M2, `NewWithFields`): the stack holds
    /// `[class, v1..vn]` with the class at `base - 1` and `names[i]` naming
    /// `v(i+1)`'s field; the window is replaced by the finished object.
    /// Reached only through a true `BranchIfNotPlainNew` verdict, so the
    /// receiver was a plain-instantiable class when the field expressions
    /// started evaluating — exactly the point `Callable::New` commits today.
    /// `instantiation_plan` re-derives per epoch, so a field expression that
    /// mutated the class mid-evaluation (adding an `init`) is still honored:
    /// the non-empty-plan path below IS `finalize_instantiation`, fed an env
    /// holding exactly the named bindings (`lookup_str` is local-only, so a
    /// parentless env is indistinguishable from the config frame's).
    pub(crate) fn exec_new_with_fields(
        &mut self,
        mc: &Mutation<'gc>,
        base: usize,
        names: &[Symbol],
    ) -> Result<(), QuoinError> {
        let recv_at = base
            .checked_sub(1)
            .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
        let Value::Class(class) = self.stack[recv_at] else {
            return Err(QuoinError::Other(
                "NewWithFields: receiver is not a class".to_string(),
            ));
        };
        let obj = self.new_object(mc, class);
        let plan = self.instantiation_plan(mc, class);
        if plan.inits.is_empty() {
            // Direct field binds: `finalize_instantiation` with an empty chain
            // reduces to exactly this (unknown names silently dropped there
            // too — it iterates ivar_slots and looks each up in the env).
            for (i, sym) in names.iter().enumerate() {
                let val = self.stack[base + i];
                if let Some((_, slot)) = plan
                    .ivar_slots
                    .iter()
                    .find(|(n, _)| n.as_str() == sym.as_str())
                {
                    obj.borrow_mut(mc).fields[*slot] = val;
                }
            }
            self.stack.truncate(recv_at);
            self.push(Value::Object(obj));
        } else {
            // Root the object in the receiver slot across the init chain (an
            // init can park); the values stay rooted in the window and env.
            self.stack[recv_at] = Value::Object(obj);
            let mut env = EnvFrame::new(None);
            for (i, sym) in names.iter().enumerate() {
                env.bind(*sym, self.stack[base + i]);
            }
            let env = gcl!(mc, env);
            self.finalize_instantiation(mc, obj, env)?;
            let out = self.stack[recv_at];
            self.stack.truncate(recv_at);
            self.push(out);
        }
        Ok(())
    }

    /// Probe the executing `block`'s inline cache at `ip`: a hit requires a live epoch (method
    /// tables unchanged) and matching receiver + argument type-shape guards. Immediates match on
    /// their cheap `Value` discriminant with no class derivation — the whole point. Sound with no
    /// ABA guard: the cache cell is shared per *template*, rooted in `ic_registry` for the VM's
    /// lifetime, and template ids are never reused — `(template, ip)` is a stable call-site
    /// identity. Entries are guard-free resolutions keyed only on receiver/arg type-shape +
    /// epoch, so sharing one array across every closure (and concurrent activation) of the same
    /// literal is sound.
    #[inline]
    fn ic_probe(
        &self,
        ic: InlineCacheCell<'gc>,
        ip: usize,
        receiver: Value<'gc>,
        args: &[Value<'gc>],
    ) -> Option<Callable<'gc>> {
        if args.len() > IC_MAX_ARGS {
            return None;
        }
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch || slot.n_args as usize != args.len() {
            return None;
        }
        let (rk, rp) = value_type_guard(receiver);
        if slot.recv_kind != rk || slot.recv_ptr != rp {
            return None;
        }
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            if slot.arg_kinds[i] != ak || slot.arg_ptrs[i] != ap {
                return None;
            }
        }
        slot.callable
    }

    /// Fill the executing `block`'s inline-cache slot at `ip` — but only for a **guard-free**
    /// resolution, i.e. one the global cache also memoized (a guarded dispatch depends on
    /// argument *values*, not just types, so it must never be inline-cached). The global-cache
    /// lookup here is cold: it runs only on an IC miss, which for a monomorphic site happens
    /// once. The block's per-`ip` array is allocated lazily (sized to its bytecode) on first fill.
    #[allow(clippy::too_many_arguments)] // inline-cache fill threads the resolved site + dispatch context
    fn ic_fill(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ip: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: &[Value<'gc>],
        callable: Callable<'gc>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        let class_side = matches!(receiver, Value::Class(_));
        let Some(class_ref) = self.get_class_for_lookup(receiver) else {
            return;
        };
        let Some(key) = self.method_cache_key(class_ref, selector, class_side, args) else {
            return;
        };
        if !matches!(self.dispatch_cache.entries.get(&key), Some(Some(_))) {
            return; // uncacheable (guarded) or not a hierarchy method — don't inline-cache
        }
        let epoch = self.dispatch_epoch;
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        // The cache cell is its own `Gc<RefLock<…>>` (shared across every closure of
        // the same template via `ic_registry`), so mutate it directly through the
        // write barrier, same idiom as `globals`.
        Self::ic_write_slot(
            mc,
            block.inline_cache,
            block.template.bytecode.len(),
            ip,
            ICSlot {
                epoch,
                recv_kind,
                recv_ptr,
                n_args: args.len() as u8,
                arg_kinds,
                arg_ptrs,
                callable: Some(callable),
            },
        );
    }

    fn ic_write_slot(
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        new_slot: ICSlot<'gc>,
    ) {
        let mut cache = cell.borrow_mut(mc);
        if cache.is_none() {
            *cache = Some(vec![ICSlot::empty(); bc_len].into_boxed_slice());
        }
        if let Some(slot) = cache.as_mut().and_then(|slots| slots.get_mut(ip)) {
            *slot = new_slot;
        }
    }

    /// `ic_fill` for a cell reached by template id (the compiled outcall path) —
    /// the same guards and cacheability rules, no `Gc<Block>` needed.
    #[allow(clippy::too_many_arguments)]
    fn ic_fill_cell(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: &[Value<'gc>],
        callable: Callable<'gc>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        let class_side = matches!(receiver, Value::Class(_));
        let Some(class_ref) = self.get_class_for_lookup(receiver) else {
            return;
        };
        let Some(key) = self.method_cache_key(class_ref, selector, class_side, args) else {
            return;
        };
        if !matches!(self.dispatch_cache.entries.get(&key), Some(Some(_))) {
            return; // uncacheable (guarded/tag-requiring) — never inline-cache
        }
        let epoch = self.dispatch_epoch;
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        Self::ic_write_slot(
            mc,
            cell,
            bc_len,
            ip,
            ICSlot {
                epoch,
                recv_kind,
                recv_ptr,
                n_args: args.len() as u8,
                arg_kinds,
                arg_ptrs,
                callable: Some(callable),
            },
        );
    }

    // GC-rooting: the only yield reachable from here is a *guarded* method's guard
    // predicate (`lookup_method` -> `match_score` -> `execute_validation_block`), and
    // that binds `receiver` as the guard's `self` and each `args` element as a guard
    // parameter into the guard env frame before it steps — so both are rooted through
    // any yield. `caller_block` is a copy of `self.frames[frame_idx].block`, rooted by
    // the live frame stack. Nothing here is held across a yield unrooted.
    fn exec_send(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        selector: Symbol,
        num_args: usize,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        // The operands sit in ORDER at the stack top. Copy the args in one
        // exact-size allocation, but leave `[receiver, args..]` LIVE on the
        // stack: for Native/AotCall callables that window IS the GC root for
        // the whole call (no rooting clone — see `NativeArgs::StackWindow`),
        // torn down in `dispatch_send_rooted` after the call returns. Frame-
        // pushing callables consume the window before their frame instead.
        let args_start = self
            .stack
            .len()
            .checked_sub(num_args)
            .ok_or("Stack underflow")?;
        let recv_start = args_start.checked_sub(1).ok_or("Stack underflow")?;
        let args: Vec<Value<'gc>> = self.stack[args_start..].to_vec();
        let receiver = self.stack[recv_start];
        // Call-site identity for the inline cache: the executing frame's cache cell + the
        // Send's own `ip`, captured before we advance it (the block itself is re-read at
        // fill time — see the note at `ic_fill` below).
        let caller_ic = self.frames[frame_idx].ic;
        let site_ip = self.frames[frame_idx].ip;
        self.frames[frame_idx].ip += 1; // Advance caller frame IP

        if let Value::Object(obj) = receiver
            && let ObjectPayload::Block(block) = &obj.borrow().payload
            && (selector.as_str() == "value" || selector.as_str() == "value:")
        {
            let block = *block;
            self.stack.truncate(recv_start);
            self.start_block(mc, block, args, Some(receiver), Some(selector));
            return Ok(VmStatus::Running);
        }

        // Inline-cache fast path: a hit skips `lookup_method`'s key-build + hash + hashmap.
        if let Some(callable) = self.ic_probe(caller_ic, site_ip, receiver, &args) {
            return self.dispatch_send_rooted(mc, callable, receiver, args, selector, recv_start);
        }

        // `last_send_args` is read only by the stack-trace formatter, and only for an
        // innermost send that fails *in place* (no callee frame of its own): a failed
        // lookup, a `MessageNotUnderstood`, or a native-method error (the last captured
        // inside `Callable::call`). On success the args move into the callee frame
        // (`Frame.args`), which the formatter reads instead — so we snapshot only on
        // these error branches, not every send.
        let method_opt = match self.lookup_method(mc, receiver, selector, &args) {
            Ok(m) => m,
            Err(e) => {
                self.stack.truncate(recv_start);
                self.exceptions.last_send_args = args;
                return Err(e);
            }
        };
        if let Some(callable) = method_opt {
            // Re-read rather than reuse `caller_block`: `lookup_method` above
            // can run guard blocks (yield-capable); the frame itself stays in
            // the traced `self.frames`, so the fresh read is always rooted.
            self.ic_fill(
                mc,
                self.frames[frame_idx].block,
                site_ip,
                receiver,
                selector,
                &args,
                callable,
            );
            self.dispatch_send_rooted(mc, callable, receiver, args, selector, recv_start)
        } else {
            // The selector may still exist with non-matching signatures; surface those
            // filtered-out variants as a hint.
            let candidates = self
                .collect_method_candidates(receiver, selector)
                .iter()
                .map(|&mv| self.format_candidate_signature(mv, selector))
                .collect();
            let receiver_name = receiver.class_name();
            let arg_names = args.iter().map(|a| a.class_name()).collect();
            self.stack.truncate(recv_start);
            self.exceptions.last_send_args = args;
            Err(QuoinError::MessageNotUnderstood {
                receiver: receiver_name,
                selector: selector.as_str().to_string(),
                args: arg_names,
                candidates,
            })
        }
    }

    /// Dispatch a send whose `[receiver, args..]` window is still LIVE on the
    /// value stack at `stack[recv_start..]` (see `exec_send`). Native and
    /// AotCall callables run with the window as their GC root — no rooting
    /// clone — and their pushed result is re-seated over the window
    /// afterwards. Everything else (interpreted methods, guarded variants,
    /// ext methods) consumes the window up front, exactly as before. The
    /// AotCall arm's interpreter fallbacks truncate the window themselves
    /// before pushing their frame, so after an `Ok` the discriminator is the
    /// stack height: above `recv_start` = a synchronous result to re-seat;
    /// at it = a frame was started and there is nothing to move.
    fn dispatch_send_rooted(
        &mut self,
        mc: &Mutation<'gc>,
        callable: crate::dispatch::Callable<'gc>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
        selector: Symbol,
        recv_start: usize,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        use crate::dispatch::Callable;
        match callable {
            Callable::Native(_) | Callable::AotCall { .. } => {
                let res = callable.call(
                    self,
                    mc,
                    Some(receiver),
                    args,
                    Some(selector),
                    Some(recv_start + 1),
                );
                match res {
                    Ok(()) => {
                        if self.stack.len() > recv_start {
                            let result = self.pop()?;
                            self.stack.truncate(recv_start);
                            self.push(result);
                        }
                        Ok(VmStatus::Running)
                    }
                    Err(e) => {
                        // NLR-aware teardown — the S1/finish_frame rule: a
                        // `^^` that escaped through this send has already
                        // truncated to its target's base and pushed the
                        // delivered value there, and that base can sit AT or
                        // ABOVE this window's start (a caller whose operand
                        // stack was empty at the send). Touching the stack
                        // then chops the delivery; every OTHER error tears
                        // the window down here.
                        if !matches!(e, QuoinError::NonLocalReturn) {
                            self.stack.truncate(recv_start.min(self.stack.len()));
                        }
                        Err(e)
                    }
                }
            }
            _ => {
                self.stack.truncate(recv_start);
                callable.call(self, mc, Some(receiver), args, Some(selector), None)?;
                Ok(VmStatus::Running)
            }
        }
    }

    /// Bind `name` in the current frame to an already-obtained `val`. Shared by the
    /// `DefineLocal` handler (pops) and `DefineLocalKeep` (peeks).
    fn store_define_local(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        name: Symbol,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if matches!(name.as_str(), "true" | "false" | "nil") {
            let err_msg = format!("Can't modify reserved identifier {}", name);
            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
            return Err(QuoinError::Other(err_msg));
        }
        self.frames[frame_idx].env.borrow_mut(mc).bind(name, val);
        Ok(())
    }

    /// Assign `name` to an already-obtained `val`: inside a `new:{}` block bind locally
    /// (object init), else set up the lexical chain or bind. Shared by `StoreLocal`
    /// (pops) and `StoreLocalKeep` (peeks).
    fn store_set_local(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        name: Symbol,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if matches!(name.as_str(), "true" | "false" | "nil") {
            let err_msg = format!("Can't modify reserved identifier {}", name);
            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
            return Err(QuoinError::Other(err_msg));
        }
        let frame = &mut self.frames[frame_idx];
        // Init-form binding is STATIC (E): a `new:{...}` config literal's
        // assignments bind into its own frame however it is invoked — the
        // frame flag covers real instantiation, the template flag covers a
        // user-defined `new:` running the block as a plain closure
        // (previously that chain-walked the write: caller-dependent
        // semantics nothing could reason about, the AOT gates included).
        // Bodies are identical on purpose: the `else if` condition itself does the
        // work (`EnvFrame::set` attempts the assignment and reports whether the
        // binding existed), so the branches can't be merged.
        #[allow(clippy::if_same_then_else)]
        if frame.instantiating_obj.is_some() || frame.block.template.is_init_literal {
            frame.env.borrow_mut(mc).bind(name, val);
        } else if !EnvFrame::set(frame.env, mc, name, val) {
            frame.env.borrow_mut(mc).bind(name, val);
        }
        Ok(())
    }

    /// Store an already-obtained `val` into instance field `name` on `self`. Shared by
    /// `StoreField` (pops) and `StoreFieldKeep` (peeks).
    fn store_field_value(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        ip: usize,
        name: &str,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let frame = &self.frames[frame_idx];
        let block = frame.block;
        let ic = frame.ic;
        let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
        if let Value::Object(obj) = self_val {
            let class = obj.borrow().class;
            // Fast path: cached slot for this exact class at this call site. A hit
            // implies the field is declared; the length guard below still applies
            // (an instance can predate a later-added ivar).
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
                && slot < obj.borrow().fields.len()
            {
                obj.borrow_mut(mc).fields[slot] = val;
                return Ok(());
            }
            match self.field_slot(class, name) {
                Some(slot) if slot < obj.borrow().fields.len() => {
                    self.field_fill(mc, block, ip, class, slot);
                    obj.borrow_mut(mc).fields[slot] = val;
                }
                Some(_) => {
                    // Declared on the class, but this instance predates it (a mixin added
                    // the ivar after the object was created); shape is fixed at construction.
                    return Err(QuoinError::Other(format!(
                        "Instance of '{}' has no '@{}' (it was added after this instance was created)",
                        class.borrow().name,
                        name
                    )));
                }
                None => {
                    // You cannot create an instance variable by assigning to it.
                    return Err(QuoinError::Other(format!(
                        "No instance variable '@{}' declared on '{}'",
                        name,
                        class.borrow().name
                    )));
                }
            }
        } else {
            // Immediate value types (Integer/Double/Boolean/Nil) have no per-instance
            // fields — setting `@x` on one is an error.
            return Err(QuoinError::Other(format!(
                "Cannot set instance variable '@{}' on a value type ({})",
                name,
                self_val.type_name()
            )));
        }
        Ok(())
    }

    pub fn step(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, QuoinError> {
        let res = self.step_internal(mc);
        if let Err(QuoinError::NonLocalReturn) = res {
            return Ok(VmStatus::Running);
        }
        // Cancellation bypasses source annotation (like NonLocalReturn) so it reaches
        // the scheduler as a bare `Cancelled` rather than wrapped in `WithSourceInfo`.
        if let Err(QuoinError::Cancelled) = res {
            return Err(QuoinError::Cancelled);
        }
        // A requested process exit likewise stays bare so the driver can match it.
        if let Err(QuoinError::ExitRequested(code)) = res {
            return Err(QuoinError::ExitRequested(code));
        }
        if let Err(e) = res {
            return Err(self.annotate_error(e));
        }
        res
    }

    /// Execute a single VM instruction. The one-step entry point kept for the synchronous
    /// sub-execution loops, the debugger, and `qn benchmark`; it clones the current frame's
    /// bytecode `Rc` per call. The hot path (`run_vm_loop`) uses `run_dispatch`, which hoists
    /// that clone out of the per-instruction path.
    pub(crate) fn step_internal(
        &mut self,
        mc: &Mutation<'gc>,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
            return Ok(VmStatus::Finished(ret));
        }
        let bytecode = self.frames[self.frames.len() - 1]
            .block
            .template
            .bytecode
            .clone();
        self.dispatch_one(mc, &bytecode)
    }

    /// Run up to `budget` instructions in one flat loop, hoisting the current frame's bytecode
    /// `Rc` into a local — cloned only when the frame stack changes (a call pushes / a return
    /// pops), not once per instruction. This is the hot dispatch path driven by `run_vm_loop`.
    /// It folds in the cancellation, empty-stack, and error handling that `step` +
    /// `step_internal` do per instruction, so the result feeds `run_vm_loop` directly. Returns
    /// `Running` once the budget is spent (i.e. "yield now"). The held `Rc` keeps the bytecode
    /// alive across frame changes and GC, exactly as the per-step clone did.
    pub(crate) fn run_dispatch(
        &mut self,
        mc: &Mutation<'gc>,
        budget: u32,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        let mut cached_len = usize::MAX;
        let mut bytecode: Option<SharedBytecode> = None;
        let mut steps = 0u32;
        loop {
            if self.sched.cancel_current {
                return Err(self.take_cancellation());
            }
            if self.frames.is_empty() {
                let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
                return Ok(VmStatus::Finished(ret));
            }
            let flen = self.frames.len();
            if flen != cached_len {
                cached_len = flen;
                bytecode = Some(self.frames[flen - 1].block.template.bytecode.clone());
            }
            let bc = bytecode.as_ref().unwrap();
            match self.dispatch_one(mc, bc) {
                // A completed instruction, or a `^`/`^^` non-local return that unwound frames
                // (`step` maps `NonLocalReturn` to `Running`). Count it; the changed frame
                // stack re-hoists next iteration. An in-flight COMPILED-home `^^`
                // can never surface here: the owning `codegen::invoke` always sits
                // between the `^^` and this top loop (dispatch_one's AotCall arm
                // consumes the delivery) — asserted, because absorbing one would
                // desync the S5 protocol and truncate under a live frame.
                Ok(VmStatus::Running) | Err(QuoinError::NonLocalReturn) => {
                    // (No result binding here: this arm runs once per
                    // interpreted instruction, and binding the Drop-glued
                    // Result cost a measured ~6% on combinators. The assert
                    // holds on BOTH variants — the target must be None
                    // whenever the top loop is running at all.)
                    debug_assert!(
                        self.aot.nlr_target.is_none(),
                        "in-flight compiled-home ^^ surfaced at the top dispatch loop"
                    );
                    steps += 1;
                    if steps >= budget {
                        return Ok(VmStatus::Running);
                    }
                }
                Ok(other) => return Ok(other),
                Err(QuoinError::Cancelled) => return Err(QuoinError::Cancelled),
                Err(QuoinError::ExitRequested(code)) => {
                    return Err(QuoinError::ExitRequested(code));
                }
                Err(e) => return Err(self.annotate_error(e)),
            }
        }
    }

    /// One instruction, hoisted-bytecode form (the giant dispatch `match`). `bytecode` is the
    /// current frame's bytecode `Rc` held by the caller (`step_internal` per-call, or
    /// `run_dispatch` once per frame-entry), so `inst` borrows the caller's local — not
    /// `self` — leaving handlers full `&mut self`. Callers guarantee `self.frames` is
    /// non-empty and no cancellation is pending.
    ///
    /// `ip` is hoisted into a local (Slice b2, the ip-register hoist on top of b1's flat
    /// loop): fall-through arms advance it as `ip += 1` in a register, instead of a
    /// bounds-checked `self.frames[frame_idx].ip += 1` per instruction, and the guarded
    /// write-back at the tail syncs it to the frame. **Invariant:** an arm that advances `ip`
    /// and then leaves via an early `return` (or a value-return like `ExecuteBlockWithSelf`'s
    /// `return if …`), rather than falling through, MUST sync it itself with
    /// `self.frames[frame_idx].ip = ip` — the tail write-back only runs on fall-through. A
    /// violation is never silent: it surfaces immediately as a stack imbalance under the
    /// `.qn` suite.
    /// Run the deferred calls queued on `frames[frame_idx]` (e.g. mixin
    /// requirement checks) *before* popping it, so the defer queue — and any
    /// Values it references — stays GC-rooted via `self.frames` even if a
    /// defer yields and a collection happens during the suspension. Iterates
    /// a clone to satisfy the borrow checker; the originals stay in the
    /// (still-live) frame to keep their Values reachable. Defers run only on
    /// NORMAL completion (the `Return` and implicit end-of-bytecode arms —
    /// never a `^^` unwinding through the frame); if one throws and this is
    /// a new class definition, the class is unregistered first.
    #[inline]
    fn run_frame_defers(&mut self, mc: &Mutation<'gc>, frame_idx: usize) -> Result<(), QuoinError> {
        if self.frames[frame_idx].defers.is_empty() {
            return Ok(());
        }
        let defers = self.frames[frame_idx].defers.clone();
        if let Err(e) = self.run_defers(mc, &defers) {
            if let Some(name) = self.frames[frame_idx].unregister_on_defer_failure.clone() {
                self.globals.borrow_mut(mc).remove(&name);
                // The class is gone; its pointer could be reused, so drop
                // any memoized resolutions that might reference it.
                self.invalidate_method_cache();
            }
            return Err(e);
        }
        Ok(())
    }

    /// Consume a just-popped frame COMPLETELY — the discipline every pop
    /// site shares (used by the `MethodReturn` unwind and the implicit-
    /// return SLOW path; the `Return` arm and the implicit fast path
    /// OPEN-CODE the same steps for speed — their comments say why. Keep
    /// them in lockstep with this): destructure the frame's fields out
    /// first, because `finalize_instantiation` can park (an init that
    /// sleeps) and a collection while parked leaves any Gc pointer still
    /// held on this suspended stack dangling (the S0 segfault). The rooting
    /// contract across that park: the instantiating object rides the VM
    /// stack, and the frame's env rides `last_popped_env`. The
    /// receiver-return applies first and the instantiation pop overwrites
    /// it. Returns the (possibly replaced) return value plus the frame's
    /// `spec_tid` — the CALLER decides whether to observe the return (the
    /// `MethodReturn` unwind observes only at the target frame) and owns
    /// the value-stack truncation policy (per-frame vs once-at-target).
    fn consume_popped_frame(
        &mut self,
        mc: &Mutation<'gc>,
        frame: Frame<'gc>,
        mut ret_val: Value<'gc>,
    ) -> Result<(Value<'gc>, u32), QuoinError> {
        let Frame {
            spec_tid,
            env,
            receiver,
            return_receiver,
            instantiating_obj,
            ..
        } = frame;
        self.last_popped_env = Some(env);
        if return_receiver && let Some(rx) = receiver {
            ret_val = rx;
        }
        if let Some(obj) = instantiating_obj {
            self.push(Value::Object(obj));
            self.finalize_instantiation(mc, obj, env)?;
            ret_val = self.pop()?;
        }
        Ok((ret_val, spec_tid))
    }

    pub(crate) fn dispatch_one(
        &mut self,
        mc: &Mutation<'gc>,
        bytecode: &SharedBytecode,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        let frame_idx = self.frames.len() - 1;
        // Hoisted instruction pointer (Slice b2): read once, advanced in a register by the
        // arms, synced back at the tail. See the invariant on this fn.
        let mut ip = self.frames[frame_idx].ip;
        let inst = match bytecode.0.get(ip) {
            Some(i) => i,
            None => {
                // Implicit return Nil — a NORMAL completion, so the frame-
                // teardown discipline is the `Return` arm's. This arm is HOT
                // (a fused loop's exit jump lands one past the last
                // instruction), so the common plain frame stays on a minimal
                // path; the rare shapes (defers, an instantiation, a
                // receiver-return) take the full shared discipline — they
                // used to be silently SKIPPED here, a divergence waiting for
                // the first such frame to end implicitly.
                let f = &self.frames[frame_idx];
                if !f.defers.is_empty() || f.instantiating_obj.is_some() || f.return_receiver {
                    self.run_frame_defers(mc, frame_idx)?;
                    let ret_val = self.new_nil(mc);
                    let popped = self.frames.pop().unwrap();
                    self.stack.truncate(popped.stack_base);
                    let (ret_val, spec_tid) = self.consume_popped_frame(mc, popped, ret_val)?;
                    if spec_tid != 0 {
                        self.spec_observe_return(spec_tid, ret_val);
                    }
                    self.push(ret_val);
                    return Ok(VmStatus::Running);
                }
                let ret_val = self.new_nil(mc);
                let popped = self.frames.pop().unwrap();
                self.stack.truncate(popped.stack_base);
                if popped.spec_tid != 0 {
                    self.spec_observe_return(popped.spec_tid, ret_val);
                }
                self.last_popped_env = Some(popped.env);
                self.push(ret_val);
                return Ok(VmStatus::Running);
            }
        };

        // Debugger checkpoint: only active while a session is attached (otherwise one bool
        // load). May suspend with `DebugBreak` to hand control to the driver; transparent —
        // execution continues here (then dispatches `inst`) on resume. `inst` borrows the
        // local `bytecode` clone, not `self`, so `&mut self` here is fine.
        if self.instrumentation.debug.is_some() {
            self.debug_checkpoint(frame_idx, ip)?;
        }
        if self.instrumentation.coverage.is_some() {
            self.coverage_tick(frame_idx, ip);
        }

        match inst {
            Instruction::LoadLocal(name) => {
                let name = *name;
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, name).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                ip += 1;
            }
            Instruction::DefineLocal(name) => {
                let name = *name;
                let val = self.pop()?;
                self.store_define_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            // Store-and-keep: store the top of stack without popping it (fused `Dup;
            // DefineLocal`, an assignment used as an expression).
            Instruction::DefineLocalKeep(name) => {
                let name = *name;
                let val = self.peek()?;
                self.store_define_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::StoreLocal(name) => {
                let name = *name;
                let val = self.pop()?;
                self.store_set_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::StoreLocalKeep(name) => {
                let name = *name;
                let val = self.peek()?;
                self.store_set_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::LoadGlobal(name) => {
                // A name bound to nothing is an error, not `nil`. Reading it used to yield
                // `nil`, so a typo propagated silently even though *assigning* to an
                // undeclared local is a compile error. A compile-time check is impossible
                // here — `use` executes at run time, so a unit cannot see the globals its
                // own `use` will define — but by the time this instruction runs, every
                // `use` has run and every class is defined. Ask whether a class exists with
                // `Class.exists?:#Name`.
                let Some(val) = self.globals.borrow().get(name).copied() else {
                    return Err(QuoinError::NameError(format!(
                        "undefined name `{name}` — nothing with that name is in scope"
                    )));
                };
                self.push(val);
                ip += 1;
            }
            Instruction::StoreGlobal(name, is_define) => {
                let val = self.pop()?;
                if name.name == "true" || name.name == "false" || name.name == "nil" {
                    let err_msg = format!("Can't modify reserved identifier {}", name.name);
                    self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let first_char = name.name.chars().next().unwrap_or('\0');
                if first_char.is_ascii_uppercase() {
                    let exists = self.globals.borrow().contains_key(name);
                    if *is_define {
                        if exists {
                            let err_msg = format!(
                                "Global {} is already defined in this scope",
                                name.to_explicit_string()
                            );
                            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    } else {
                        if exists {
                            let err_msg = format!(
                                "Can't modify global constant {}",
                                name.to_explicit_string()
                            );
                            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    }
                }
                self.globals.borrow_mut(mc).insert(name.clone(), val);
                ip += 1;
            }
            Instruction::Push(constant) => {
                let val = self.materialize_constant(mc, constant);
                self.push(val);
                ip += 1;
            }
            Instruction::Pop => {
                self.pop()?;
                ip += 1;
            }
            Instruction::Dup => {
                let val = self.peek()?;
                self.push(val);
                ip += 1;
            }
            // Devirtualized Integer operators (Slice 2a/2f). Fast path when both operands are
            // `Value::Int`: compute directly and push. Semantics match Integer's native ops
            // (`+`/`-`/`*` plain i64, wrap in release; `/`/`%` raise "Division by zero";
            // compares yield a Bool). A non-Int operand (a var whose inferred `Int` type went
            // stale, or an untyped operand) falls back to the real send — so `Int` can be
            // inferred optimistically rather than only trusted for annotated params.
            // The standalone `Int` ops (stack operands — e.g. `1 + 2`). All compute through the
            // shared `int_bin_compute` → `devirt_ops::int_bin`, so they can't drift from the fused
            // ops or the native `Integer` methods. A non-Int operand falls back to the real send.
            Instruction::IntAdd => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Add, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("+:"), 1);
                }
            }
            Instruction::IntSub => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Sub, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("-:"), 1);
                }
            }
            Instruction::IntMul => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Mul, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("*:"), 1);
                }
            }
            Instruction::IntDiv => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Div, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("/:"), 1);
                }
            }
            Instruction::IntMod => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Mod, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("%:"), 1);
                }
            }
            Instruction::IntLt => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Lt, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<:"), 1);
                }
            }
            Instruction::IntLe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Le, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<=:"), 1);
                }
            }
            Instruction::IntGt => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Gt, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">:"), 1);
                }
            }
            Instruction::IntGe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Ge, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">=:"), 1);
                }
            }
            Instruction::IntEq => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Eq, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("==:"), 1);
                }
            }
            Instruction::IntNe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Ne, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("!=:"), 1);
                }
            }
            // Fused Int superinstructions (Slice a1): load the operand(s) directly and compute;
            // on a non-Int operand push the operands and fall back to the real send (matching
            // the standalone `Int` ops' contract, so MNU / user redefinition still work).
            Instruction::IntBinLL(a, b, kind) => {
                let (a, b, kind) = (*a, *b, *kind);
                let (va, vb) = {
                    let frame = &self.frames[frame_idx];
                    (EnvFrame::get(frame.env, a), EnvFrame::get(frame.env, b))
                };
                if let (Some(Value::Int(x)), Some(Value::Int(y))) = (va, vb) {
                    let res = Self::int_bin_compute(kind, x, y)?;
                    self.push(res);
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    let vb = vb.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    self.push(vb);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            Instruction::IntBinLC(a, c, kind) => {
                let (a, kind) = (*a, *kind);
                let va = {
                    let frame = &self.frames[frame_idx];
                    EnvFrame::get(frame.env, a)
                };
                if let (Some(Value::Int(x)), Some(y)) = (va, c.as_int()) {
                    let res = Self::int_bin_compute(kind, x, y)?;
                    self.push(res);
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    let cv = self.materialize_constant(mc, c);
                    self.push(cv);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            // Devirtualized Double operators (mirror of the Integer arms). Plain IEEE-754 f64 —
            // `/`/`%` do NOT check for zero (inf/NaN, matching native Double). A non-Double
            // operand falls back to the real send.
            Instruction::DoubleAdd => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Add, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("+:"), 1);
                }
            }
            Instruction::DoubleSub => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Sub, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("-:"), 1);
                }
            }
            Instruction::DoubleMul => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Mul, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("*:"), 1);
                }
            }
            Instruction::DoubleDiv => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Div, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("/:"), 1);
                }
            }
            Instruction::DoubleMod => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Mod, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("%:"), 1);
                }
            }
            Instruction::DoubleLt => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Lt, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<:"), 1);
                }
            }
            Instruction::DoubleLe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Le, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<=:"), 1);
                }
            }
            Instruction::DoubleGt => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Gt, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">:"), 1);
                }
            }
            Instruction::DoubleGe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Ge, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">=:"), 1);
                }
            }
            Instruction::DoubleEq => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Eq, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("==:"), 1);
                }
            }
            Instruction::DoubleNe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Ne, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("!=:"), 1);
                }
            }
            Instruction::DoubleBinLL(a, b, kind) => {
                let (a, b, kind) = (*a, *b, *kind);
                let (va, vb) = {
                    let frame = &self.frames[frame_idx];
                    (EnvFrame::get(frame.env, a), EnvFrame::get(frame.env, b))
                };
                if let (Some(Value::Double(x)), Some(Value::Double(y))) = (va, vb) {
                    self.push(Self::double_bin_compute(kind, x, y));
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    let vb = vb.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    self.push(vb);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            Instruction::DoubleBinLC(a, c, kind) => {
                let (a, kind) = (*a, *kind);
                let va = {
                    let frame = &self.frames[frame_idx];
                    EnvFrame::get(frame.env, a)
                };
                if let (Some(Value::Double(x)), Some(y)) = (va, c.as_double()) {
                    self.push(Self::double_bin_compute(kind, x, y));
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    let cv = self.materialize_constant(mc, c);
                    self.push(cv);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            // Devirtualized List accessors (Slice 2e). Operands are already on the stack in
            // send order; if the receiver isn't a native list (or the index isn't an
            // Integer, matching the typed native `at:`/`at:put:`), fall back to the real send.
            Instruction::ListGet => {
                let n = self.stack.len();
                let index = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                if let Value::Int(i) = index {
                    let got = receiver.with_native_state::<NativeListState, _, _>(|l| {
                        devirt_ops::list_get(l.get_vec(), i)
                    });
                    if let Ok(elem) = got {
                        self.stack.truncate(n - 2);
                        self.push(elem.unwrap_or(Value::Nil));
                        // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:"), 1);
            }
            Instruction::TagCollection(tag) => {
                let v = *self.stack.last().expect("TagCollection: literal on stack");
                self.tag_fresh_collection(mc, v, *tag)?;
                ip += 1;
            }
            Instruction::ListSet => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let index = self.stack[n - 2];
                let receiver = self.stack[n - 3];
                if let Value::Int(i) = index {
                    // Untagged (the whole pre-generics world): exactly the old
                    // body behind one `is_none`. Tagged lists take the
                    // out-of-line checked path (docs/internal/GENERICS_ARCH.md §6).
                    let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                        match l.elem {
                            None => Some(devirt_ops::list_set(l.get_vec_mut(), i, value)),
                            // Scalar tags decide inside the one borrow; the tag
                            // check precedes the bounds check (the VALUE is
                            // illegal regardless of index). Class tags escalate.
                            Some(t) => match t.matches_value(&value) {
                                Some(true) => Some(devirt_ops::list_set(l.get_vec_mut(), i, value)),
                                Some(false) => {
                                    Some(Err(elem_tag::elem_type_error("List", t, &value, Some(i))))
                                }
                                None => None,
                            },
                        }
                    });
                    if let Ok(fast) = res {
                        let inner = match fast {
                            Some(inner) => inner,
                            None => {
                                let r = self.tagged_list_set(mc, receiver, i, value);
                                self.stack.truncate(n - 3);
                                r?;
                                self.push(receiver);
                                self.frames[frame_idx].ip = ip + 1;
                                return Ok(VmStatus::Running);
                            }
                        };
                        self.stack.truncate(n - 3);
                        inner?; // propagate an IndexError or tag TypeError
                        self.push(receiver); // `at:put:` evaluates to the receiver
                        // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:put:"), 2);
            }
            Instruction::ListPush => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    match l.elem {
                        None => {
                            l.get_vec_mut().push(value);
                            Some(Ok(()))
                        }
                        // Scalar tags decide inside the one borrow (vm-free);
                        // only a Class tag escalates to the dispatch walk.
                        Some(t) => match t.matches_value(&value) {
                            Some(true) => {
                                l.get_vec_mut().push(value);
                                Some(Ok(()))
                            }
                            Some(false) => {
                                Some(Err(elem_tag::elem_type_error("List", t, &value, None)))
                            }
                            None => None,
                        },
                    }
                });
                if let Ok(fast) = res {
                    match fast {
                        Some(inner) => {
                            self.stack.truncate(n - 2);
                            inner?;
                        }
                        None => {
                            let r = self.tagged_list_push(mc, receiver, value);
                            self.stack.truncate(n - 2);
                            r?;
                        }
                    }
                    self.push(receiver); // `add:` evaluates to the receiver
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("add:"), 1);
            }
            // Devirtualized Map accessors (mirror of List). Map is `IndexMap<String, Value>`, so
            // the key must be a String at runtime; a non-String key (or non-Map receiver) falls
            // back to the real send.
            Instruction::MapGet => {
                let n = self.stack.len();
                let key = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                // Inline fast path for ANY scalar-exact key (String, Int,
                // Double, Symbol, …): hash in Rust, no guest dispatch
                // possible. Instance keys (guest hash/==:) fall back to the
                // real `at:` send, which handles dispatch and parking.
                if let Ok(Some(hit)) =
                    receiver.with_native_state::<NativeMapState, _, _>(|m| m.get_scalar(&key))
                {
                    self.stack.truncate(n - 2);
                    self.push(hit.unwrap_or(Value::Nil)); // missing key → nil (native `at:`)
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:"), 1);
            }
            Instruction::MapSet => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let key = self.stack[n - 2];
                let receiver = self.stack[n - 3];
                // Same widening as MapGet: any scalar-exact key inlines;
                // instance keys — and tag checks that need the full
                // type-matcher — fall back to the real `at:put:` send.
                if crate::value::key_native_exact(&key)
                    && crate::value::value_hash_scalar(&key).is_some()
                {
                    let res =
                        receiver.with_native_state_mut::<NativeMapState, _, _>(mc, |m| {
                            match m.elem {
                                None => {
                                    m.insert_scalar(key, value);
                                    Some(Ok(()))
                                }
                                Some(t) => match t.matches_value(&value) {
                                    Some(true) => {
                                        m.insert_scalar(key, value);
                                        Some(Ok(()))
                                    }
                                    Some(false) => Some(Err(elem_tag::elem_type_error(
                                        "Map String",
                                        t,
                                        &value,
                                        None,
                                    ))),
                                    None => None,
                                },
                            }
                        });
                    if let Ok(Some(inner)) = res {
                        self.stack.truncate(n - 3);
                        inner?;
                        self.push(receiver); // `at:put:` evaluates to the receiver
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:put:"), 2);
            }
            Instruction::Send(selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            // Fused superinstructions (see `Instruction::SendLocal` doc): push the last
            // operand the send consumes, then run the identical send path.
            Instruction::SendLocal(var, selector, num_args) => {
                let (var, selector, num_args) = (*var, *selector, *num_args);
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, var).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendConst(constant, selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                let val = self.materialize_constant(mc, constant);
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendField(field, selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                let val = self.load_field(mc, frame_idx, None, field);
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            // 3-instruction sends: push two operands (left-to-right) then dispatch.
            Instruction::SendLocalLocal(a, b, selector, num_args) => {
                let (a, b, selector, num_args) = (*a, *b, *selector, *num_args);
                let env = self.frames[frame_idx].env;
                let va = EnvFrame::get(env, a).unwrap_or_else(|| self.new_nil(mc));
                self.push(va);
                let vb = EnvFrame::get(env, b).unwrap_or_else(|| self.new_nil(mc));
                self.push(vb);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendLocalConst(a, constant, selector, num_args) => {
                let (a, selector, num_args) = (*a, *selector, *num_args);
                let env = self.frames[frame_idx].env;
                let va = EnvFrame::get(env, a).unwrap_or_else(|| self.new_nil(mc));
                self.push(va);
                let vc = self.materialize_constant(mc, constant);
                self.push(vc);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::Return | Instruction::BlockReturn => {
                if !self.frames[frame_idx].defers.is_empty() {
                    self.run_frame_defers(mc, frame_idx)?;
                }
                let mut ret_val = self.pop()?;
                let popped_frame = self.frames.pop().unwrap();
                // Open-coded `consume_popped_frame` (see its doc for the
                // copy-before-park contract): this is the hottest opcode in
                // the interpreter, and routing the frame through the helper
                // cost a measured ~20% on combinators (a fat Result plus a
                // real Frame move per return). Keep the two in lockstep.
                let spec_tid = popped_frame.spec_tid;
                self.last_popped_env = Some(popped_frame.env);
                self.stack.truncate(popped_frame.stack_base);
                if popped_frame.return_receiver
                    && let Some(rx) = popped_frame.receiver
                {
                    ret_val = rx;
                }
                if let Some(obj) = popped_frame.instantiating_obj {
                    self.push(Value::Object(obj));
                    self.finalize_instantiation(mc, obj, popped_frame.env)?;
                    ret_val = self.pop()?;
                }
                if spec_tid != 0 {
                    self.spec_observe_return(spec_tid, ret_val);
                }
                self.push(ret_val);
            }
            Instruction::MethodReturn => {
                let ret_val = self.pop()?;
                let enclosing_id = self.frames[frame_idx].enclosing_method_id;

                return if let Some(target_id) = enclosing_id {
                    // The home may be a live COMPILED invocation (S5): it has
                    // no interpreter frame — its mark says where its outcall
                    // frames and slot window begin. Pop only the frames above
                    // the mark, deliver the value at the window base, and let
                    // the AOT error channel unwind the native frames
                    // (`codegen::invoke` consumes `aot.nlr_target`). A dead
                    // home matches neither a frame nor a mark (ids are never
                    // reused) and drains like an interpreted dead home.
                    let compiled_home = self
                        .aot
                        .frame_marks
                        .iter()
                        .rev()
                        .find(|m| m.id == target_id)
                        .copied();
                    let mut ret_val = ret_val;
                    let mut target_stack_base = None;
                    loop {
                        if let Some(m) = compiled_home
                            && self.frames.len() <= m.frames_len
                        {
                            target_stack_base = Some(m.stack_base);
                            self.aot.nlr_target = Some(target_id);
                            break;
                        }
                        let Some(f) = self.frames.pop() else { break };
                        let f_id = f.id;
                        let f_stack_base = f.stack_base;
                        let (rv, spec_tid) = self.consume_popped_frame(mc, f, ret_val)?;
                        ret_val = rv;
                        if f_id == target_id {
                            if spec_tid != 0 {
                                self.spec_observe_return(spec_tid, ret_val);
                            }
                            target_stack_base = Some(f_stack_base);
                            break;
                        }
                    }
                    if let Some(base) = target_stack_base {
                        self.stack.truncate(base);
                    }
                    self.push(ret_val);
                    Err(QuoinError::NonLocalReturn)
                } else {
                    Err("MethodReturn executed outside of a method context".into())
                };
            }
            Instruction::Yeet => {
                let yeeted_val = self.pop()?;
                self.frames.clear();
                return Ok(VmStatus::Yeeted(yeeted_val));
            }
            Instruction::Jump(offset) => {
                let offset = *offset;
                ip = (ip as isize + offset) as usize;
            }
            Instruction::IfJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                if cond.is_truthy() {
                    ip = (ip as isize + offset) as usize;
                } else {
                    ip += 1;
                }
            }
            Instruction::ElseJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                if !cond.is_truthy() {
                    ip = (ip as isize + offset) as usize;
                } else {
                    ip += 1;
                }
            }
            Instruction::BranchIfNotBool(offset) => {
                let offset = *offset;
                // Peek the receiver (do not pop): a non-Bool takes the cold path (the real
                // send), which needs it on the stack; a Bool falls through to the inlined
                // branch, which consumes it.
                let is_bool = matches!(self.stack.last(), Some(Value::Bool(_)));
                if is_bool {
                    ip += 1;
                } else {
                    ip = (ip as isize + offset) as usize;
                }
            }
            Instruction::RequireBool => match self.stack.last() {
                Some(Value::Bool(_)) => ip += 1,
                other => {
                    let got = other
                        .map(|v| v.class_name())
                        .unwrap_or_else(|| "Nil".to_string());
                    return Err(QuoinError::MessageNotUnderstood {
                        receiver: got,
                        selector: "whileDo: (a loop condition must be a Boolean)".to_string(),
                        args: Vec::new(),
                        candidates: Vec::new(),
                    });
                }
            },
            Instruction::BranchIfNotList(offset, block_tid) => {
                let offset = *offset;
                let block_tid = *block_tid;
                // Peek the `each:` receiver (do not pop): a native List falls through to
                // the fused index loop (which consumes it); anything else takes the cold
                // path (the real `each:` send), which needs it on the stack. One downcast
                // per each: CALL, not per element.
                let list_probe = self.stack.last().and_then(|v| {
                    v.with_native_state::<NativeListState, _, _>(|l| {
                        let v = l.get_vec();
                        (v.len(), v.first().copied())
                    })
                    .ok()
                });
                match list_probe {
                    None => ip = (ip as isize + offset) as usize,
                    Some((len, first)) => {
                        // A COMPILED argument block flips the choice: the cold path's
                        // real send reaches it per element (invoke_block), beating the
                        // interpreted splice ~2x. The guard also feeds the template's
                        // warmth (by element count) and its argument observation (the
                        // elements ARE the args), so splice-only programs tier up.
                        if let Some(tid) = block_tid
                            && crate::tuning::aot_enabled()
                            && crate::codegen::fused_site_prefers_send(self, tid, len, first)
                        {
                            ip = (ip as isize + offset) as usize;
                        } else {
                            ip += 1;
                        }
                    }
                }
            }
            Instruction::BranchIfNotPlainNew(offset) => {
                let offset = *offset;
                // Peek the `new:` receiver (do not pop): a plain-instantiable class falls
                // through to the fused field-expression path; anything else takes the
                // cold path (the real send: user meta `new:`, abstract-class error,
                // non-class MNU), which needs it on the stack.
                let receiver = *self
                    .stack
                    .last()
                    .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
                let (cell, bc_len) = {
                    let frame = &self.frames[frame_idx];
                    (frame.ic, frame.block.template.bytecode.len())
                };
                if self.plain_new_check_cached(mc, Some((cell, bc_len)), ip, receiver) {
                    ip += 1;
                } else {
                    ip = (ip as isize + offset) as usize;
                }
            }
            Instruction::NewWithFields(names) => {
                let names = names.clone();
                let base = self
                    .stack
                    .len()
                    .checked_sub(names.len())
                    .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
                self.exec_new_with_fields(mc, base, &names)?;
                ip += 1;
            }
            Instruction::ListLen => {
                let n = self.stack.len();
                let receiver = self.stack[n - 1];
                let got =
                    receiver.with_native_state::<NativeListState, _, _>(|l| l.get_vec().len());
                if let Ok(len) = got {
                    self.stack.truncate(n - 1);
                    self.push(Value::Int(len as i64));
                    // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("count"), 0);
            }
            Instruction::NewList(n) => {
                let n = *n;
                let mut elements = Vec::new();
                for _ in 0..n {
                    elements.push(self.pop()?);
                }
                elements.reverse();
                let list = self.new_list(mc, elements);
                self.push(list);
                ip += 1;
            }
            Instruction::NewMap(n) => {
                let n = *n;
                // ANY value keys. Same rooting discipline as NewSet below: an
                // instance key's hash/==: can PARK, so the pairs stay rooted
                // in place on the VM stack, the fresh map rides on top, and
                // each insert re-reads through the stack.
                {
                    let map_val = self.new_map(mc, Vec::new());
                    self.push(map_val);
                }
                let base = self.stack.len() - 1 - 2 * n;
                for i in 0..n {
                    let map_val = *self.stack.last().expect("map on top");
                    let key = self.stack[base + 2 * i];
                    let val = self.stack[base + 2 * i + 1];
                    // Duplicate keys: the later entry wins, as before.
                    crate::runtime::map::map_put_any(self, mc, map_val, key, val)?;
                }
                let map_val = self.pop()?;
                self.stack.truncate(base);
                self.push(map_val);
                ip += 1;
            }
            Instruction::NewSet(n) => {
                let n = *n;
                // Build by inserting through set_add so the literal is deduplicated
                // by `==:`, the same way `add:` enforces uniqueness at runtime.
                // A user `==:` can PARK, so nothing GC-managed may live in Rust
                // locals across the inserts: the elements stay rooted in place on
                // the VM stack and the set rides on top, re-read after each
                // insert (popping into a Vec here once left both the elements
                // and the fresh set collectible mid-dedup).
                {
                    let set_val = self.new_set(mc, Vec::new());
                    self.push(set_val);
                }
                let base = self.stack.len() - 1 - n;
                for i in 0..n {
                    let sv = *self.stack.last().expect("set literal under construction");
                    let v = self.stack[base + i];
                    self.set_add(mc, sv, v)?;
                }
                let sv = self.pop()?;
                self.stack.truncate(base);
                self.push(sv);
                ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::Object(obj) = pattern_val
                    && let ObjectPayload::String(s) = &obj.borrow().payload
                {
                    let re = Regex::new(s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = self.new_regex(mc, re);
                    self.push(regex_val);
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "String".to_string(),
                        got: pattern_val.type_name().to_string(),
                        msg: format!("Regex pattern must be a String, got: {:?}", pattern_val),
                    });
                }
                ip += 1;
            }
            Instruction::RecordClassSite { name, source } => {
                self.class_meta
                    .entry(name.clone())
                    .or_default()
                    .extensions
                    .push(source.clone());
                ip += 1;
            }
            Instruction::DefineClass {
                name,
                parent_name,
                instance_vars,
                source,
            } => {
                // A NAMED package's unit may not define a bare-global class — the
                // no-pollution rule extension packages get structurally
                // (`EXT_PACKAGING.md` §4). The load stack's top is the package whose
                // top level is executing right now (`load_unit` pushes the canonical
                // package; bare/`std:` loads sit here as `None`, and `"self"` is the
                // entry project's own units, which may claim bare globals). Checking
                // at the definition site makes this the one enforcement point —
                // including definitions the old load-time AST scan couldn't see,
                // like one inside a block the top level runs. Reopens (`<--`) never
                // reach this instruction, so extending existing classes stays allowed.
                if name.path.is_empty()
                    && let Some(Some(pkg)) = self.modules.load_stack.last()
                    && pkg != "self"
                {
                    return Err(QuoinError::ClassError(format!(
                        "use: package `{pkg}` defines the bare-global class `{cls}` — a \
                         package's classes must live under a namespace (e.g. \
                         `[{ns}]{cls}`); packages cannot claim bare globals",
                        cls = name.name,
                        ns = crate::runtime::extension::pascal_case(pkg),
                    )));
                }
                // Definition wins over any earlier record (a REPL redefinition moves the
                // class); a native class's `.class_doc(..)` set at registration survives.
                if source.is_some() {
                    self.class_meta.entry(name.clone()).or_default().source = source.clone();
                }
                let parent = if let Some(p_name) = parent_name {
                    let val = self
                        .globals
                        .borrow()
                        .get(p_name)
                        .copied()
                        .ok_or_else(|| format!("Parent class {} not found", p_name))?;
                    if let Value::Class(parent_class) = val {
                        if parent_class.borrow().is_sealed {
                            // A typed ClassError, matching the sealed-EXTENSION error above
                            // (ensure_extensible): `catch:{|e:Error|}` must see both. It was
                            // a bare String throw — the F12 family (RELEASE_PREP Tier 4b).
                            return Err(QuoinError::ClassError(format!(
                                "Cannot subclass sealed class {}",
                                parent_class.borrow().name.to_explicit_string()
                            )));
                        }
                        Some(parent_class)
                    } else {
                        return Err(format!("Parent {} is not a Class", p_name).into());
                    }
                } else {
                    if !(name.path.is_empty() && name.name == "Object") {
                        let obj_key = NamespacedName::new(Vec::new(), "Object".to_string());
                        if let Some(Value::Class(obj_class)) =
                            self.globals.borrow().get(&obj_key).copied()
                        {
                            Some(obj_class)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(existing_val) = self.globals.borrow().get(name).copied()
                    && let Value::Class(_) = existing_val
                {
                    return Err(format!(
                        "Cannot redefine class {} because it already exists",
                        name.to_explicit_string()
                    )
                    .into());
                }

                let class_obj = gcl!(
                    mc,
                    Class {
                        name: name.clone(),
                        parent,
                        instance_vars: instance_vars.clone(),
                        instance_methods: FxHashMap::default(),
                        class_methods: FxHashMap::default(),
                        mixin_classes: Vec::new(),
                        field_slots: FxHashMap::default(),
                        init_plan: None,
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
                        native_new_refusal: None,
                    }
                );
                self.globals
                    .borrow_mut(mc)
                    .insert(name.clone(), Value::Class(class_obj));
                // The class is registered now (so it can reference itself), but if
                // the body's deferred mixin checks fail it must be unregistered.
                // Hand the name to the upcoming ExecuteBlockWithSelf (the body).
                self.pending_class_def = Some(name.clone());
                self.push(Value::Class(class_obj));
                ip += 1;
            }
            Instruction::ExecuteBlockWithSelf => {
                let block_val = self.pop()?;
                let self_val = self.pop()?;
                if self_val.is_nil() {
                    return Err(QuoinError::Other(
                        "Cannot extend nil or non-existent class/object".to_string(),
                    ));
                }
                return if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    self.frames[frame_idx].ip = ip + 1;
                    self.start_block_as_method(mc, *block, self_val, Vec::new(), None, false);
                    // A new class definition (DefineClass ran just before) marks its
                    // body frame so a failed deferred mixin check unregisters the class.
                    // Extensions don't set pending_class_def, so they get no marker.
                    let pending = self.pending_class_def.take();
                    let body_frame = self.frames.last_mut().unwrap();
                    body_frame.return_receiver = true;
                    body_frame.unregister_on_defer_failure = pending;
                    Ok(VmStatus::Running)
                } else {
                    Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("ExecuteBlockWithSelf expects a Block, got {:?}", block_val),
                    })
                };
            }
            Instruction::DefineMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, self_symbol())
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(QuoinError::Other)?;
                    self.ensure_not_sealed(target_class)?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, false);
                    let sel_sym = Symbol::intern(selector);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    if is_class_side {
                        if let Some(existing_val) =
                            target_class.borrow().class_methods.get(&sel_sym).copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(sel_sym, method_obj);
                        }
                    } else {
                        if let Some(existing_val) = target_class
                            .borrow()
                            .instance_methods
                            .get(&sel_sym)
                            .copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(sel_sym, method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions
                    // and invalidate compiled direct-self recursion (S2).
                    self.invalidate_method_cache();
                    crate::codegen::bump_redef_epoch();
                    self.push(method_obj);
                    ip += 1;
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("DefineMethod expects a Block, got {:?}", block_val),
                    });
                }
            }
            Instruction::OverrideMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, self_symbol())
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(QuoinError::Other)?;
                    self.ensure_not_sealed(target_class)?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, true);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    let exists = self
                        .lookup_in_class_hierarchy(target_class, selector, is_class_side)
                        .is_some();
                    if !exists {
                        return Err(QuoinError::Other(format!(
                            "Method {} does not exist in hierarchy of Class {} to override",
                            selector,
                            target_class.borrow().name
                        )));
                    }

                    let sel_sym = Symbol::intern(selector);
                    if is_class_side {
                        if let Some(existing_val) =
                            target_class.borrow().class_methods.get(&sel_sym).copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(sel_sym, method_obj);
                        }
                    } else {
                        if let Some(existing_val) = target_class
                            .borrow()
                            .instance_methods
                            .get(&sel_sym)
                            .copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(sel_sym, method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions
                    // and invalidate compiled direct-self recursion (S2).
                    self.invalidate_method_cache();
                    crate::codegen::bump_redef_epoch();
                    self.push(method_obj);
                    ip += 1;
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("OverrideMethod expects a Block, got {:?}", block_val),
                    });
                }
            }

            Instruction::LoadField(name) => {
                let val = self.load_field(mc, frame_idx, Some(ip), name);
                self.push(val);
                ip += 1;
            }
            // Phase 5·3: read a field off the object on top of the stack (an inlined `v.x`).
            Instruction::LoadFieldOf(name) => {
                let obj = self.pop()?;
                let block = self.frames[frame_idx].block;
                let ic = self.frames[frame_idx].ic;
                let val = self.field_of(mc, block, ic, Some(ip), obj, name);
                self.push(val);
                ip += 1;
            }
            Instruction::StoreField(name) => {
                let val = self.pop()?;
                self.store_field_value(mc, frame_idx, ip, name, val)?;
                ip += 1;
            }
            Instruction::StoreFieldKeep(name) => {
                let val = self.peek()?;
                self.store_field_value(mc, frame_idx, ip, name, val)?;
                ip += 1;
            }
            Instruction::Use {
                package,
                path,
                glob,
            } => {
                // Clone out so the `inst` borrow is released before `load_unit` takes
                // `&mut self`. Advance ip first: `load_unit` runs the loaded unit in a
                // nested frame (frame-balanced), so this frame resumes at the next ip.
                let package = package.clone();
                let path = path.clone();
                let glob = *glob;
                ip += 1;
                if glob {
                    load_glob(self, mc, package.as_deref(), &path)?;
                } else {
                    load_unit(self, mc, package.as_deref(), &path)?;
                }
                // A `use` evaluates to nil — push one value so the statement nets +1 on
                // the stack (`compile_program` pops between statements).
                let nil = self.new_nil(mc);
                self.push(nil);
            }
        }

        // Sync the hoisted `ip` (Slice b2) back to the current frame on fall-through. Guarded
        // so a pop-arm (a non-local return that shrank the frame stack) doesn't index a popped
        // frame; early-returning arms that advanced `ip` sync it themselves (see the invariant).
        if frame_idx < self.frames.len() {
            self.frames[frame_idx].ip = ip;
        }
        Ok(VmStatus::Running)
    }
}
