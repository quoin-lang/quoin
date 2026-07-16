//! Call entry points and nested execution: method-call starts, native re-entry,
//! the hosted-object table, init/defer running, nested drivers (`run_nested`,
//! `execute_block`), REPL line scoping, and block frame setup. Extends `VmState`.

use super::*;

/// The hosted-object table's pin owner (`vm.pins`): one table per VM.
const HOSTED_OWNER: crate::pin_table::PinOwner = crate::pin_table::PinOwner {
    kind: "hosted",
    id: 0,
};

impl<'gc> VmState<'gc> {
    pub fn start_method_call(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<usize, QuoinError> {
        let sel = Symbol::intern(selector);
        let method = self.lookup_method(mc, receiver, sel, &args)?;
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(self, mc, Some(receiver), args, Some(sel), None)?;
            Ok(initial_frame_count)
        } else {
            Err(QuoinError::Other(format!(
                "Method {} not found on receiver",
                selector
            )))
        }
    }

    /// Insert a value into the worker-side hosted-object table, answering its
    /// wire id: pin-slot index + 1, so 0 is never issued (`Call.recv: 0`
    /// stays free to mean "class-side" when hosted manifests arrive). Backed
    /// by `vm.pins` (owner kind "hosted"): identity dedupe — hosting the same
    /// object twice answers the same id (proxy `==` and release refcounts
    /// depend on it) — comes from `pin_or_find`, and the wire ids stay
    /// release-disciplined exactly as before (no generation tags; ids are
    /// sparse across the shared slab, which the wire never cared about).
    pub fn hosted_insert(&mut self, v: Value<'gc>) -> u64 {
        let pin = self.pins.pin_or_find(HOSTED_OWNER, v);
        (crate::pin_table::PinTable::index(pin) + 1) as u64
    }

    pub fn hosted_get(&self, id: u64) -> Option<Value<'gc>> {
        self.pins
            .get_at((id as usize).checked_sub(1)?, HOSTED_OWNER.kind)
    }

    pub fn hosted_release(&mut self, id: u64) {
        if let Some(i) = (id as usize).checked_sub(1) {
            self.pins.unpin_at(i, HOSTED_OWNER.kind);
        }
    }

    /// Like [`Self::call_method`], but a lookup miss raises `MessageNotUnderstood`
    /// instead of answering nil. Remote dispatch (hosted objects; anything
    /// forwarding a real SEND) wants send semantics; `call_method`'s nil-on-miss
    /// is hook semantics ("call it if it's there").
    pub fn call_method_mnu(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let sel = Symbol::intern(selector);
        if self.lookup_method(mc, receiver, sel, &args)?.is_none() {
            return Err(QuoinError::MessageNotUnderstood {
                receiver: format!("{receiver}"),
                selector: selector.to_string(),
                args: args.iter().map(|a| format!("{a}")).collect(),
                candidates: Vec::new(),
            });
        }
        self.call_method(mc, receiver, selector, args)
    }

    pub fn call_method(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        // Bound native → Quoin re-entry so a self-referential hook (a `==:` that re-adds
        // to the set it's a key of, a comparator that re-sorts, …) fails catchably rather
        // than overflowing the machine stack. The `?` returns before incrementing on the
        // over-limit case; otherwise the guard decrements on every exit path.
        self.enter_native_reentry()?;
        let result = self.call_method_inner(mc, receiver, selector, args);
        self.native_reentry_depth = self.native_reentry_depth.saturating_sub(1);
        result
    }

    /// The catchable ceiling on native → Quoin re-entry depth (see `native_reentry_depth`).
    /// Well above any legitimate nesting of custom hooks, low enough to fault before the
    /// coroutine stack overflows (each re-entry frame drives a nested `step` loop).
    const MAX_NATIVE_REENTRY: usize = 12;

    /// Headroom `execute_block` insists on before re-entering the VM: refuse once fewer than
    /// this many bytes of the 16 MiB coroutine stack remain.
    ///
    /// A *depth* cap is the wrong instrument here (and is why `execute_block` was left
    /// unguarded): lazy generator pipelines legitimately compose blocks deeper than any
    /// machine-stack-safe fixed count, so a counter cannot tell them from a block that
    /// re-enters itself. Measuring the stack itself separates the two — deep-but-finite
    /// pipelines keep their real ceiling, minus this margin.
    ///
    /// 2 MiB is sized to cover the deepest single frame we can add after the check passes:
    /// `dispatch_one` + a compiled outcall + a native method, several times over.
    const STACK_MARGIN: usize = 2 * 1024 * 1024;

    /// Claim one level of native re-entry, or return a catchable error at the ceiling.
    fn enter_native_reentry(&mut self) -> Result<(), QuoinError> {
        if self.native_reentry_depth >= Self::MAX_NATIVE_REENTRY {
            return Err(QuoinError::StackExhausted(format!(
                "native call recursion too deep (> {}): a custom ==:/hash/comparator/render \
                 hook is re-entering a native operation without bound",
                Self::MAX_NATIVE_REENTRY
            )));
        }
        self.native_reentry_depth += 1;
        Ok(())
    }

    /// Refuse to re-enter the VM when this coroutine's stack is nearly spent.
    ///
    /// Each `execute_block` level stacks *real Rust frames* (the `valueWithSelfOrArg:`
    /// combinator seam, the `catch:` family), so an `each:` body that re-iterates its own
    /// receiver — or a `catch:` whose protected block re-enters itself — walks off the end of
    /// the 16 MiB coroutine stack and aborts the process with SIGBUS, uncatchably. The check
    /// is a load, a subtract and a compare against the address of a stack local.
    ///
    /// `stack_limit == 0` disables it: the benchmark harness steps the VM on the OS thread
    /// stack, where we have no extent to measure and no re-entry to bound.
    #[inline]
    fn ensure_stack_headroom(&self) -> Result<(), QuoinError> {
        if self.stack_limit == 0 {
            return Ok(());
        }
        let probe = 0u8;
        let sp = &probe as *const u8 as usize;
        if sp.saturating_sub(self.stack_limit) >= Self::STACK_MARGIN {
            return Ok(());
        }
        Err(QuoinError::StackExhausted(
            "block re-entry exhausted the task stack: a block is re-entering itself without \
             bound (an each:/collect: body that re-iterates its own receiver, or a catch: \
             whose protected block re-enters it)"
                .to_string(),
        ))
    }

    fn call_method_inner(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let sel = Symbol::intern(selector);
        let method = self.lookup_method(mc, receiver, sel, &args)?;
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(self, mc, Some(receiver), args, Some(sel), None)?;

            // let the VM catch up (batched — B0)
            self.run_nested(mc, initial_frame_count, "method call")?;

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    pub fn call_method_value(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        method_val: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        self.enter_native_reentry()?;
        let result = self.call_method_value_inner(mc, receiver, method_val, selector, args);
        self.native_reentry_depth = self.native_reentry_depth.saturating_sub(1);
        result
    }

    fn call_method_value_inner(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        method_val: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let method: Option<Callable<'gc>> = match method_val {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Block(block) => Some(Callable::Block(*block)),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                        if let Some(ext) = method_state.ext_dispatch() {
                            Some(Callable::ExtMethod {
                                ext,
                                selector: Symbol::intern(selector),
                            })
                        } else if let Some(service) = method_state.service_dispatch() {
                            Some(Callable::ServiceMethod {
                                service,
                                selector: Symbol::intern(selector),
                            })
                        } else if let Some(func) = method_state.native_func() {
                            Some(Callable::Native(func))
                        } else if let Some(Value::Object(block_obj)) = method_state.get_block()
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Some(Callable::Block(*block))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        };

        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(
                self,
                mc,
                Some(receiver),
                args,
                Some(Symbol::intern(selector)),
                None,
            )?;

            // let the VM catch up (batched — B0)
            self.run_nested(mc, initial_frame_count, "method call")?;

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    pub(super) fn collect_classes_for_init(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        classes: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        if let Some(parent) = class_borrow.parent {
            self.collect_classes_for_init(parent, classes, visited);
        }
        for mixin in &class_borrow.mixin_classes {
            self.collect_classes_for_init(*mixin, classes, visited);
        }

        if !classes.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            classes.push(class_ref);
        }
    }

    /// Run a frame's deferred calls in order. Each is a plain method send; the
    /// first one that errors aborts and returns the error.
    pub(super) fn run_defers(
        &mut self,
        mc: &Mutation<'gc>,
        defers: &[DeferredCall<'gc>],
    ) -> Result<(), QuoinError> {
        for d in defers {
            self.call_method(mc, d.receiver, &d.selector, d.args.clone())?;
        }
        Ok(())
    }

    pub fn run_all_inits(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
    ) -> Result<(), QuoinError> {
        let class = obj.borrow().class;
        let plan = self.instantiation_plan(mc, class);
        let receiver = Value::Object(obj);
        self.active_init_plans.push(plan);
        let result = self.run_init_chain_planned(mc, receiver, plan, None);
        self.active_init_plans.pop();
        result
    }

    /// Drive nested execution (a native-initiated block or method call) until the frame
    /// stack returns to `initial_frame_count` — the BATCHED form (B0,
    /// docs/internal/BLOCK_AOT_ARCH.md §3). One flat loop with the current frame's bytecode `Rc`
    /// hoisted exactly like `run_dispatch` (re-cloned only when the frame stack changes),
    /// yielding to the driver every `step_batch()` instructions instead of after every
    /// one. This gives nested block bodies — every `each:`-family combinator element —
    /// the same observable scheduling granularity as top-level code; before B0 they paid
    /// a full coroutine suspend→driver→resume round-trip plus a bytecode-`Rc` clone per
    /// instruction. Under the stress modes `step_batch()` is 1, so their per-instruction
    /// coverage is unchanged. Errors are returned raw (un-annotated), exactly as the
    /// per-step loops returned them; `context` names the caller in the uncaught-throw
    /// message, byte-identical to the old per-site strings.
    /// An in-flight `^^` MUST keep unwinding past this loop — either its
    /// target frame is strictly below the loop's baseline, or its home is a
    /// live COMPILED frame (`aot.nlr_target` set): a compiled frame owns no
    /// interpreter frame of its own to pop, so its delivery stops the unwind
    /// EXACTLY AT nested baselines, where "all callee frames gone" must read
    /// as delivery, not completion — only the owning `codegen::invoke` may
    /// consume it (the S5 absorb-at-baseline abort). Every loop that absorbs
    /// `NonLocalReturn` decides through this ONE predicate.
    #[inline(always)]
    pub(crate) fn nlr_must_propagate(&self, baseline: usize) -> bool {
        self.frames.len() < baseline || self.aot.nlr_target.is_some()
    }

    pub(super) fn run_nested(
        &mut self,
        mc: &Mutation<'gc>,
        initial_frame_count: usize,
        context: &str,
    ) -> Result<(), QuoinError> {
        let budget = crate::tuning::step_batch();
        let mut steps: u32 = 0;
        let mut cached_len = usize::MAX;
        let mut bytecode: Option<SharedBytecode> = None;
        while self.frames.len() > initial_frame_count {
            // The cancellation check `step_internal` performed per step — including
            // immediately after a resume from the suspend below.
            if self.sched.cancel_current {
                return Err(self.take_cancellation());
            }
            let flen = self.frames.len();
            if flen != cached_len {
                cached_len = flen;
                bytecode = Some(self.frames[flen - 1].block.template.bytecode.clone());
            }
            match self.dispatch_one(mc, bytecode.as_ref().unwrap()) {
                Ok(VmStatus::Running) => {}
                // A `^`/`^^` unwound frames: below the baseline it belongs to an
                // enclosing loop; at/above it, the loop head re-evaluates. Counted
                // as a step, like `run_dispatch`.
                Err(QuoinError::NonLocalReturn) => {
                    if self.nlr_must_propagate(initial_frame_count) {
                        return Err(QuoinError::NonLocalReturn);
                    }
                }
                Ok(VmStatus::Finished(_)) => break,
                Ok(VmStatus::Yeeted(val)) => {
                    return Err(QuoinError::Other(format!(
                        "Uncaught exception during {}: {}",
                        context, val
                    )));
                }
                Err(e) => return Err(e),
            }
            steps += 1;
            if steps >= budget {
                steps = 0;
                if let Some(yielder) = unsafe { self.get_yielder() } {
                    yielder.suspend(YieldReason::CooperativeYield);
                }
            }
        }
        Ok(())
    }

    pub fn execute_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
        self_val: Option<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        // NOTE: deliberately *not* guarded by `enter_native_reentry`. Lazy generator
        // pipelines legitimately compose blocks many levels deep on the native stack
        // (each stage's `execute_block` nests inside the next), so a low machine-stack
        // cap here would break real programs. The native-recursion guard lives on the
        // method-dispatch paths (`call_method`/`call_method_value`), where the
        // pathological self-referential hooks (a `==:` that re-adds to its own set)
        // actually recurse. What bounds *this* path is the remaining stack itself, which
        // costs those pipelines nothing while still refusing unbounded self-re-entry.
        self.ensure_stack_headroom()?;
        let initial_frame_count = self.frames.len();
        if let Some(receiver) = self_val {
            self.start_block_as_method(mc, block, receiver, args, None, false);
        } else {
            self.start_block(mc, block, args, None, None);
        }

        self.run_nested(mc, initial_frame_count, "block execution")?;

        Ok(self.pop()?)
    }

    /// Start a REPL line's top-level `block` in the persistent `repl_env`, returning the
    /// `(frame, stack)` depths to restore once the line finishes. The frame's env *is* the
    /// reused `repl_env` (not a fresh child), so top-level `x = 5` binds there and persists
    /// across lines. Transient scheduler state is reset first so a line that errored mid-fiber
    /// can't corrupt this one. The caller installs this as scheduler task #0 and drives it
    /// (via the shared `drive_main_task`), so the line gets async I/O, sleep, tasks, and
    /// fibers — which the old synchronous path could not. `repl_env` must be `Some`.
    pub fn begin_repl_line(&mut self, block: Gc<'gc, Block<'gc>>) -> (usize, usize) {
        let env = self
            .repl_env
            .expect("begin_repl_line called without a repl_env");
        let base_frames = self.frames.len();
        let base_stack = self.stack.len();
        self.reset_scheduler();

        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        self.frames.push(Frame {
            id: frame_id,
            is_nested_block: false,
            enclosing_method_id: Some(frame_id),
            block,
            ic: block.inline_cache,
            ip: 0,
            env,
            instantiating_obj: None,
            receiver: None,
            selector: None,
            args: Vec::new(),
            stack_base: base_stack,
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
        (base_frames, base_stack)
    }

    /// Finish a REPL line driven by the scheduler: take its result off the stack (or `nil` on
    /// error / an empty stack), then restore the `(frame, stack)` baseline and clear any
    /// pending exception so the next line starts clean. `succeeded` reflects whether the drive
    /// finished without a runtime error; the error itself is already source-annotated by `step`
    /// and surfaced by the caller. The returned value is meaningful only when `succeeded`.
    pub fn end_repl_line(
        &mut self,
        mc: &Mutation<'gc>,
        base_frames: usize,
        base_stack: usize,
        succeeded: bool,
    ) -> Value<'gc> {
        let result = if succeeded {
            self.pop().unwrap_or_else(|_| self.new_nil(mc))
        } else {
            self.new_nil(mc)
        };
        self.frames.truncate(base_frames);
        self.stack.truncate(base_stack);
        self.exceptions.active = None;
        result
    }

    pub fn execute_validation_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        receiver: Value<'gc>,
        outer_param_syms: &[Symbol],
        args: &[Value<'gc>],
    ) -> Result<Value<'gc>, QuoinError> {
        let initial_frame_count = self.frames.len();

        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);

        // A guard is a predicate over the method's arguments: every argument is bound
        // by its (method) parameter name, so the guard references them directly
        // (`|x:Integer { x > 5 }|`) without re-declaring them. `self` is the method's
        // receiver (the subject of the call), so a guard can also use the rest of the
        // class's functionality — other methods, instance variables, etc.
        env_frame.bind(self_symbol(), receiver);

        for (sym, val) in outer_param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }

        let env_ref = gcl!(mc, env_frame);

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block: block.template.is_nested_block,
            enclosing_method_id: Some(frame_id),
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector: None,
            args: args.to_vec(),
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });

        self.run_nested(mc, initial_frame_count, "validation block execution")?;

        Ok(self.pop()?)
    }

    pub fn start_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
        receiver: Option<Value<'gc>>,
        selector: Option<Symbol>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind parameters
        for (sym, val) in block.template.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver,
            selector,
            args,
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn start_block_as_method(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
        selector: Option<Symbol>,
        is_method_call: bool,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let spec_tid = if is_method_call
            && self.aot_spec_obs_left != 0
            && block.template.spec_state.get() == crate::codegen::spec::OBSERVING
        {
            self.spec_observe_entry(&block.template, &args)
        } else {
            0
        };

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind self
        env_frame.bind(self_symbol(), receiver);
        // Bind parameters
        for (sym, val) in block.template.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_method_call {
            Some(frame_id)
        } else if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector,
            args,
            stack_base: self.stack.len(),
            spec_tid,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn start_block_for_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        selector: Option<Symbol>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        // The block runs in a fresh frame over its lexical parent only. Instance
        // variables are deliberately NOT pre-bound here: an empty `new:{}` block
        // must leave fields at their default (nil) rather than silently capturing
        // a same-named variable from the surrounding scope. A bare instance-var
        // name therefore reads up the lexical chain, and an explicit assignment is
        // what binds the field (see StoreLocal's instantiation-frame handling).
        let env_frame = EnvFrame::new(block.parent_env);
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: Some(obj),
            receiver: Some(Value::Object(obj)),
            selector,
            args: Vec::new(),
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn get_class_for_lookup(
        &self,
        receiver: Value<'gc>,
    ) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
        match receiver {
            Value::Int(_) | Value::Double(_) | Value::Bool(_) | Value::Nil => {
                self.immediate_class(receiver)
            }
            Value::Object(obj) => Some(obj.borrow().class),
            Value::Class(c) => Some(c),
            Value::ClassMeta(c) => Some(c),
        }
    }

    /// The dispatch class for an immediate value type, read from `builtin_cache`
    /// (populated at native-class registration) with a globals fallback. The
    /// booleans use their per-value singleton class once `true`/`false` have been
    /// extended, otherwise the shared `Boolean` class.
    fn immediate_class(&self, receiver: Value<'gc>) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
        let (cached, name) = {
            let c = self.builtin_cache.borrow();
            match receiver {
                Value::Int(_) => (c.integer_class, "Integer"),
                Value::Double(_) => (c.double_class, "Double"),
                Value::Bool(true) => (c.true_class.or(c.boolean_class), "Boolean"),
                Value::Bool(false) => (c.false_class.or(c.boolean_class), "Boolean"),
                Value::Nil => (c.nil_class, "Nil"),
                _ => return None,
            }
        };
        cached.or_else(|| {
            match self
                .globals
                .borrow()
                .get(&NamespacedName::parse(name))
                .copied()
            {
                Some(Value::Class(c)) => Some(c),
                _ => None,
            }
        })
    }
}
