//! Method dispatch: resolution, multimethod scoring, the method-resolution cache,
//! and candidate/error formatting. Extracted verbatim from `vm.rs` — behavior-neutral.

use crate::error::QuoinError;
use crate::ext_sdk::HostCtx;
use crate::runtime::method::NativeMethodState;
use crate::symbol::Symbol;
use crate::value::{
    Block, Class, NamespacedName, NativeArgs, NativeCall, NativeFunc, ObjectPayload, Value,
};
use crate::vm::VmState;

use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use std::mem::transmute;

/// Number of leading arguments whose classes are encoded into a method-cache
/// key. Sends with more arguments than this skip the cache (rare).
const METHOD_CACHE_MAX_ARGS: usize = 4;

/// Key for the method-resolution cache (`VmState::method_cache`). Every field is
/// `Copy`/`'static`, so a lookup builds and probes a key with no allocation.
/// Class identities are raw pointers — sound only because cached lookups never
/// involve an eigenclass (the one class kind with a transient, reusable address);
/// see `Class::is_eigenclass` and `VmState::method_cache_key`. `class_ptr` is the
/// *searched* class (not necessarily the receiver's), since that is what the walk
/// is parameterized by; the receiver only matters for guards, which are uncached.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Collect)]
#[collect(require_static)]
pub struct MethodCacheKey {
    class_ptr: usize,
    selector: Symbol,
    class_side: bool,
    n_args: u8,
    /// Per-argument dispatch class pointers (`get_class_for_lookup`).
    arg_ptrs: [usize; METHOD_CACHE_MAX_ARGS],
    /// Per-argument `Value`-variant discriminant. Necessary because scoring matches
    /// an argument by `type_name()` *and* its class: a `Class` *value* (type
    /// `"Class"`) and an *instance* of that class share one `get_class_for_lookup`
    /// pointer but dispatch differently (`kindOf:Integer` vs `kindOf:5`). The kind
    /// keeps them in distinct cache entries.
    arg_kinds: [u8; METHOD_CACHE_MAX_ARGS],
}

/// A resolved, ready-to-invoke method. A `Copy` enum rather than a boxed trait
/// object, so dispatch resolves and invokes a method without a per-Send heap
/// allocation — each variant carries only `Copy` data (`Gc` handles / a native fn
/// pointer). The callable is transient — built by `lookup_method` and consumed by
/// `call` within a single `Send` step — so it is never stored in a traced struct
/// and needs no `Collect` impl (matching the `Box<dyn Callable>` it replaced).
#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub enum Callable<'gc> {
    /// A user method (a block run as a method on the receiver).
    Block(Gc<'gc, Block<'gc>>),
    /// `Class.meta` — push the class's metaclass.
    Meta(Gc<'gc, RefLock<Class<'gc>>>),
    /// `Class.new:` with no user-defined `new:` — instantiate from the block argument.
    New(Gc<'gc, RefLock<Class<'gc>>>),
    /// `Class.new` with no user-defined `new` — instantiate with no block.
    NewNoBlock(Gc<'gc, RefLock<Class<'gc>>>),
    /// A native (Rust) method.
    Native(NativeFunc),
    /// An extension-backed method (Phase 3): the send dispatches over the socket to `ext` (the
    /// owning `Extension` instance). The class and class-vs-instance side are derived from the
    /// receiver at call time; `selector` is the message to forward.
    ExtMethod { ext: Value<'gc>, selector: Symbol },
    /// An AOT-compiled user method (docs/AOT_ARCH.md): `entry` is the native
    /// code, `block` the ordinary interpreter body it overlays (the fallback if
    /// the argument shapes ever fail to unbox — which dispatch's typed-variant
    /// selection should make impossible).
    AotCall {
        block: Gc<'gc, Block<'gc>>,
        entry: crate::codegen::AotFnRef,
    },
}

impl<'gc> Callable<'gc> {
    /// The callable for a resolved user-method block: the compiled overlay when
    /// this block's template is registered (probed only on this cold path — the
    /// dispatch/inline caches memoize the result), else the interpreted block.
    fn for_block(block: Gc<'gc, Block<'gc>>) -> Callable<'gc> {
        if let Some(id) = block.template.template_id
            && let Some(entry) = crate::codegen::lookup(id)
        {
            return Callable::AotCall {
                block,
                entry: crate::codegen::AotFnRef(entry),
            };
        }
        Callable::Block(block)
    }
}

impl<'gc> Callable<'gc> {
    /// `receiver` is passed separately from `args` (which holds only the real
    /// arguments) so the hot path never prepends the receiver into a fresh Vec.
    pub fn call(
        self,
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        receiver: Option<Value<'gc>>,
        args: Vec<Value<'gc>>,
        selector: Option<Symbol>,
        // `Some(start)` when the CALLER keeps `[receiver, args..]` live on
        // the value stack at `stack[start-1..start+args.len()]` for this
        // whole call (the `exec_send` window): the Native/AotCall arms then
        // root via the window instead of cloning `args`. `None` = re-entry
        // paths that own their Vec (the arms clone, as ever).
        args_window: Option<usize>,
    ) -> Result<(), QuoinError> {
        match self {
            Callable::Block(block) => {
                let receiver = receiver.ok_or_else(|| {
                    QuoinError::Other("Method call is missing a receiver".to_string())
                })?;
                // `args` is already the method args (no receiver) — pass it straight through.
                vm.start_block_as_method(mc, block, receiver, args, selector, true);
                Ok(())
            }
            Callable::Meta(class_obj) => {
                vm.push(Value::ClassMeta(class_obj));
                Ok(())
            }
            Callable::New(class_obj) => {
                vm.ensure_instantiable(class_obj)?;
                // `new:` consumes `args` and can error in place — keep them for the
                // stack trace before returning (cold paths; a plain move, no clone).
                if args.len() != 1 {
                    vm.exceptions.last_send_args = args;
                    return Err(QuoinError::Other("new: expects a block".to_string()));
                }
                let block = if let Value::Object(obj) = args[0]
                    && let ObjectPayload::Block(b) = &obj.borrow().payload
                {
                    *b
                } else {
                    let got = args[0].type_name().to_string();
                    vm.exceptions.last_send_args = args;
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got,
                        msg: "new: expects a Block".to_string(),
                    });
                };

                // Create the new object
                let obj = vm.new_object(mc, class_obj);

                vm.start_block_for_instantiation(mc, block, obj, selector);
                Ok(())
            }
            Callable::NewNoBlock(class_obj) => {
                vm.ensure_instantiable(class_obj)?;
                if !args.is_empty() {
                    vm.exceptions.last_send_args = args;
                    return Err(QuoinError::Other("new expects no arguments".to_string()));
                }

                // Create the new object
                let obj = vm.new_object(mc, class_obj);

                vm.push(Value::Object(obj));
                if let Err(e) = vm.run_all_inits(mc, obj) {
                    vm.pop().ok();
                    return Err(e);
                }
                Ok(())
            }
            Callable::Native(func) => {
                let receiver = receiver.ok_or_else(|| {
                    QuoinError::Other("native method called without a receiver".to_string())
                })?;
                // Keep (receiver, args) GC-rooted as one unit so a native fn can re-read
                // them after a nested call that may have collected. One push/pop -> they
                // can never desync. With a caller-kept stack window the root
                // is the window itself — no clone on the hot path.
                vm.active_native_args.push(NativeCall {
                    receiver,
                    args: match args_window {
                        Some(start) => NativeArgs::StackWindow {
                            start,
                            len: args.len(),
                        },
                        None => NativeArgs::Owned(args.clone()),
                    },
                });
                // `Legacy` fns take `&mut VmState` + `mc`; `Sdk` fns take `&mut dyn Host`
                // — a `HostCtx` captures `(vm, mc)` for the call so the SDK never sees `mc`.
                // (`vm` reborrows into the ctx, so it stays usable afterward.)
                let ret = match func {
                    NativeFunc::Legacy(f) => f(vm, mc, receiver, args),
                    NativeFunc::Sdk(f) => {
                        let mut ctx = HostCtx::new(vm, mc);
                        f(&mut ctx, receiver, args)
                    }
                };
                if ret.is_err() {
                    // Native error: the send failed in place (no callee frame), so the
                    // stack-trace formatter wants its args. Materialized only here, on
                    // the cold error path — not on every send.
                    if let Some(call) = vm.active_native_args.last() {
                        vm.exceptions.last_send_args = call.args_vec(&vm.stack);
                    }
                }
                vm.active_native_args.pop();
                let ret = ret?;
                vm.push(ret);
                Ok(())
            }
            Callable::AotCall { block, entry } => {
                let receiver = receiver.ok_or_else(|| {
                    QuoinError::Other("Method call is missing a receiver".to_string())
                })?;
                // Depth gate: past the nesting cap the interpreted body
                // runs instead (flat frames) — deep untyped recursion must
                // not overflow the coroutine stack via per-level outcall
                // re-entries, and must not error where the interpreter works.
                if vm.aot.outcall_nesting >= crate::codegen::spec::MAX_OUTCALL_NESTING {
                    // Interpreter fallback pushes a FRAME: the caller-kept
                    // window (if any) must be consumed first so the frame's
                    // stack_base sits where the send began.
                    if let Some(start) = args_window {
                        vm.stack.truncate(start - 1);
                    }
                    vm.start_block_as_method(mc, block, receiver, args, selector, true);
                    return Ok(());
                }
                // S1 speculation gate: observed-kind preconditions, checked
                // before any state changes. A mismatch Bails to the
                // interpreted body; BAIL_TOMBSTONE consecutive mismatches
                // remove the entry (the speculation was wrong about this
                // program — it runs interpreted from then on).
                if !entry.0.param_preconditions.is_empty() {
                    use std::sync::atomic::Ordering;
                    let holds =
                        entry
                            .0
                            .param_preconditions
                            .iter()
                            .zip(args.iter())
                            .all(|(pre, arg)| match pre {
                                None => true,
                                Some(k) => crate::codegen::scalar_matches(*k, *arg),
                            });
                    if !holds {
                        let bails = entry.0.spec_bails.fetch_add(1, Ordering::Relaxed) + 1;
                        if bails >= crate::codegen::spec::BAIL_TOMBSTONE {
                            crate::codegen::tombstone(entry.0.template_id);
                        }
                        if let Some(start) = args_window {
                            vm.stack.truncate(start - 1);
                        }
                        vm.start_block_as_method(mc, block, receiver, args, selector, true);
                        return Ok(());
                    }
                    entry.0.spec_bails.store(0, Ordering::Relaxed);
                }
                // D1: no `active_native_args` rooting entry — the slot window
                // roots everything across compiled-body suspensions. With a
                // caller window, `invoke` REUSES it as the frame's slot window
                // (one push, not two); without one (re-entry callers), invoke
                // pushes the window itself before any suspension point.
                let outcome = crate::codegen::invoke(
                    vm,
                    mc,
                    entry.0,
                    receiver,
                    &args,
                    block.parent_env,
                    args_window.map(|start| start - 1),
                );
                if matches!(outcome, crate::codegen::AotOutcome::Err(_)) {
                    vm.exceptions.last_send_args = args.clone();
                }
                match outcome {
                    crate::codegen::AotOutcome::Value(v) => {
                        vm.push(v);
                        Ok(())
                    }
                    crate::codegen::AotOutcome::Err(e) => Err(e),
                    // Unboxing mismatch (shouldn't happen: dispatch selected the
                    // typed variant) — run the ordinary interpreted body.
                    crate::codegen::AotOutcome::Bail => {
                        if let Some(start) = args_window {
                            vm.stack.truncate(start - 1);
                        }
                        vm.start_block_as_method(mc, block, receiver, args, selector, true);
                        Ok(())
                    }
                }
            }
            Callable::ExtMethod { ext, selector } => {
                let receiver = receiver.ok_or_else(|| {
                    QuoinError::Other("extension method called without a receiver".to_string())
                })?;
                // Root (receiver, args) as one unit across the socket round-trip (which yields),
                // mirroring the `Native` arm; `ext` stays rooted via the class's method table.
                vm.active_native_args.push(NativeCall {
                    receiver,
                    args: NativeArgs::Owned(args.clone()),
                });
                let ret = crate::runtime::extension::dispatch_ext_method(
                    vm, mc, ext, receiver, selector, args,
                );
                if ret.is_err()
                    && let Some(call) = vm.active_native_args.last()
                {
                    vm.exceptions.last_send_args = call.args_vec(&vm.stack);
                }
                vm.active_native_args.pop();
                let ret = ret?;
                vm.push(ret);
                Ok(())
            }
        }
    }
}

impl<'gc> VmState<'gc> {
    /// Resolve a bare selector as a global (the legacy global-function fallback,
    /// reached only when no method matched in the class hierarchy). Builds the key
    /// lazily so the hot, method-found path never allocates it.
    fn lookup_selector_in_globals(&self, selector: Symbol) -> Option<Value<'gc>> {
        let key = NamespacedName::new(Vec::new(), selector.as_str().to_string());
        self.globals.borrow().get(&key).copied()
    }

    pub fn lookup_method(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: Symbol,
        args: &[Value<'gc>],
    ) -> Result<Option<Callable<'gc>>, QuoinError> {
        if selector.as_str() == "meta" {
            if let Value::Class(c) = receiver {
                return Ok(Some(Callable::Meta(c)));
            }
        }
        if let Value::Class(c) = receiver {
            if self
                .lookup_method_in_class_hierarchy(mc, c, receiver, selector, true, args)?
                .is_none()
            {
                if selector.as_str() == "new:" {
                    return Ok(Some(Callable::New(c)));
                }
                if selector.as_str() == "new" {
                    return Ok(Some(Callable::NewNoBlock(c)));
                }
            }
        }
        let method_val = match receiver {
            Value::Class(class_obj) => {
                if let Some(m) = self.lookup_method_in_class_hierarchy(
                    mc, class_obj, receiver, selector, true, args,
                )? {
                    Some(m)
                } else {
                    let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
                    // Hoisted out of the `if let` scrutinee: the hierarchy lookup below
                    // can run guard blocks (yield-capable), and a scrutinee temporary —
                    // here the globals borrow — would stay alive through the branch.
                    let class_class = self.globals.borrow().get(&class_key).copied();
                    if let Some(Value::Class(class_class)) = class_class {
                        if let Some(m) = self.lookup_method_in_class_hierarchy(
                            mc,
                            class_class,
                            receiver,
                            selector,
                            false,
                            args,
                        )? {
                            Some(m)
                        } else {
                            self.lookup_selector_in_globals(selector)
                        }
                    } else {
                        self.lookup_selector_in_globals(selector)
                    }
                }
            }
            Value::ClassMeta(class_obj) => {
                if let Some(m) = self.lookup_method_in_class_hierarchy(
                    mc, class_obj, receiver, selector, true, args,
                )? {
                    Some(m)
                } else {
                    // A metaclass acts as if it subclasses Object: fall through to
                    // Object's instance methods so it responds to the universal
                    // protocol (can?:, s, ==:, …). We use Object rather than the
                    // "Class" class because Class methods (new, name, …) assume a
                    // real Class receiver.
                    let object_key = NamespacedName::new(Vec::new(), "Object".to_string());
                    // Hoisted like the Class fallback above (globals borrow must not
                    // live through the yield-capable hierarchy lookup).
                    let object_class = self.globals.borrow().get(&object_key).copied();
                    if let Some(Value::Class(object_class)) = object_class {
                        if let Some(m) = self.lookup_method_in_class_hierarchy(
                            mc,
                            object_class,
                            receiver,
                            selector,
                            false,
                            args,
                        )? {
                            Some(m)
                        } else {
                            self.lookup_selector_in_globals(selector)
                        }
                    } else {
                        self.lookup_selector_in_globals(selector)
                    }
                }
            }
            // Object + immediate value types: look up via the receiver's class.
            _ => {
                if let Some(class_obj) = self.get_class_for_lookup(receiver) {
                    if let Some(m) = self.lookup_method_in_class_hierarchy(
                        mc, class_obj, receiver, selector, false, args,
                    )? {
                        Some(m)
                    } else {
                        self.lookup_selector_in_globals(selector)
                    }
                } else {
                    self.lookup_selector_in_globals(selector)
                }
            }
        };

        let method_val = match method_val {
            Some(v) => v,
            None => return Ok(None),
        };

        match method_val {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Block(block) => Ok(Some(Callable::for_block(*block))),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                        if let Some(ext) = method_state.ext_dispatch() {
                            Ok(Some(Callable::ExtMethod { ext, selector }))
                        } else if let Some(func) = method_state.native_func() {
                            Ok(Some(Callable::Native(func)))
                        } else if let Some(Value::Object(block_obj)) = method_state.get_block()
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Ok(Some(Callable::for_block(*block)))
                        } else {
                            Ok(None)
                        }
                    } else {
                        Ok(None)
                    }
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Drop every memoized method resolution. Called whenever a class's method
    /// table changes (method def/override, native registration, class unregister),
    /// which only happens at class-definition time — never inside a hot loop — so
    /// the cache stays warm during execution. Clearing wholesale (rather than
    /// per-class) is correct because a new/overridden method can shadow cached
    /// resolutions in *derived* classes too.
    pub fn invalidate_method_cache(&mut self) {
        self.dispatch_cache.entries.clear();
        // Bumping the epoch invalidates every inline-cache slot at once (a slot is used
        // only when its stored epoch matches the current one), so they self-evict.
        self.dispatch_epoch = self.dispatch_epoch.wrapping_add(1);
    }

    /// Build the cache key for a lookup, or `None` if the lookup must not be cached:
    /// an eigenclass is involved (transient pointer → unsafe to key on), an argument
    /// has no dispatch class, or there are more arguments than the key encodes.
    pub(crate) fn method_cache_key(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: Symbol,
        class_side: bool,
        args: &[Value<'gc>],
    ) -> Option<MethodCacheKey> {
        if args.len() > METHOD_CACHE_MAX_ARGS || class_ref.borrow().is_eigenclass {
            return None;
        }
        let mut arg_ptrs = [0usize; METHOD_CACHE_MAX_ARGS];
        let mut arg_kinds = [0u8; METHOD_CACHE_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let ac = self.get_class_for_lookup(*a)?;
            if ac.borrow().is_eigenclass {
                return None;
            }
            arg_ptrs[i] = Gc::as_ptr(ac) as usize;
            arg_kinds[i] = match a {
                Value::Int(_) => 0,
                Value::Double(_) => 1,
                Value::Bool(_) => 2,
                Value::Nil => 3,
                Value::Object(_) => 4,
                Value::Class(_) => 5,
                Value::ClassMeta(_) => 6,
            };
        }
        Some(MethodCacheKey {
            class_ptr: Gc::as_ptr(class_ref) as usize,
            selector,
            class_side,
            n_args: args.len() as u8,
            arg_ptrs,
            arg_kinds,
        })
    }

    pub fn lookup_method_in_class_hierarchy(
        &mut self,
        mc: &Mutation<'gc>,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        receiver: Value<'gc>,
        selector: Symbol,
        class_side: bool,
        args: &[Value<'gc>],
    ) -> Result<Option<Value<'gc>>, QuoinError> {
        // Fast path: a memoized, guard-free resolution skips the whole walk + scoring.
        let key = self.method_cache_key(class_ref, selector, class_side, args);
        if let Some(k) = key {
            if let Some(cached) = self.dispatch_cache.entries.get(&k) {
                return Ok(*cached);
            }
        }

        // Miss: walk, tracking whether any guarded candidate was examined. Save and
        // restore `dispatch_uncacheable` so nested sends fired *by* a guard (which run
        // their own lookups) can't corrupt this lookup's cacheability accounting.
        let saved_uncacheable = self.dispatch_cache.uncacheable;
        self.dispatch_cache.uncacheable = false;
        let mut visited = Vec::new();
        let result = self.lookup_method_in_class_hierarchy_rec(
            mc,
            class_ref,
            receiver,
            selector,
            class_side,
            args,
            &mut visited,
        );
        let uncacheable = self.dispatch_cache.uncacheable;
        self.dispatch_cache.uncacheable = saved_uncacheable;

        // Cache only a successful, guard-free resolution (errors and guarded
        // dispatches stay uncached — the latter can depend on argument values).
        if !uncacheable {
            if let (Some(k), Ok(resolved)) = (key, &result) {
                self.dispatch_cache.entries.insert(k, *resolved);
            }
        }
        result
    }

    // `receiver` is the subject of the send (the value `self` resolves to inside a
    // guard block); it's threaded down to `match_score`/`execute_validation_block`.
    fn lookup_method_in_class_hierarchy_rec(
        &mut self,
        mc: &Mutation<'gc>,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        receiver: Value<'gc>,
        selector: Symbol,
        class_side: bool,
        args: &[Value<'gc>],
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> Result<Option<Value<'gc>>, QuoinError> {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return Ok(None);
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        let methods = if class_side {
            &class_borrow.class_methods
        } else {
            &class_borrow.instance_methods
        };
        let method_chain_start = methods.get(&selector).copied();
        let mixins = class_borrow.mixin_classes.clone();
        let parent = class_borrow.parent;
        drop(class_borrow);

        let mut candidates = Vec::new();
        let mut curr = method_chain_start;
        while let Some(method_val) = curr {
            candidates.push(method_val);
            curr = self.get_next_method_in_chain(method_val);
        }

        // Root our CLONES of the class's edges for the rest of the resolution:
        // scoring runs guard blocks and the walk recurses into more of them,
        // and a guard that REOPENS this class can drop the class's own
        // reference to a candidate method, a mixin, or the parent — leaving
        // these locals the only (unrooted, collectible) holders mid-dispatch.
        let root_base = self.stack.len();
        for &c in &candidates {
            self.push(c);
        }
        for &m in &mixins {
            self.push(Value::Class(m));
        }
        if let Some(p) = parent {
            self.push(Value::Class(p));
        }
        let result = self.resolve_in_class(
            mc,
            receiver,
            selector,
            class_side,
            args,
            visited,
            class_ref,
            &candidates,
            &mixins,
            parent,
        );
        self.stack.truncate(root_base);
        result
    }

    /// The scoring + hierarchy-walk tail of
    /// [`Self::lookup_method_in_class_hierarchy_rec`], split out so the caller
    /// can root `candidates`/`mixins`/`parent` around ALL of its early
    /// returns with one `truncate`.
    // The caller has pushed `candidates`/`mixins`/`parent` onto the VM stack
    // for this whole call — everything held here across guard-block yields
    // (including the `applicable` copies) is rooted by that contract.
    #[allow(clippy::too_many_arguments)]
    #[allow(no_gc_across_yield)]
    fn resolve_in_class(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: Symbol,
        class_side: bool,
        args: &[Value<'gc>],
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        candidates: &[Value<'gc>],
        mixins: &[Gc<'gc, RefLock<Class<'gc>>>],
        parent: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> Result<Option<Value<'gc>>, QuoinError> {
        if !candidates.is_empty() {
            // Score every applicable candidate; the lowest `(Σ type_distance,
            // guarded?)` wins. Two distinct candidates sharing the lowest score are
            // equally specific with no tiebreaker -> `AmbiguousMethodError`. A guarded
            // and an unguarded variant never share a score (the guard rank separates
            // them), so the specific-guards-then-unguarded-catch-all idiom stays
            // unambiguous. A signatureless native scores i64::MAX and is a pure
            // fallback, exempt from ambiguity. The hierarchy walk below still lets a
            // derived class override a base regardless of score.
            let mut applicable: Vec<(Value<'gc>, (i64, u8))> = Vec::new();
            for &method_val in candidates {
                if let Some(score) = self.match_score(mc, receiver, method_val, args)? {
                    applicable.push((method_val, score));
                }
            }
            if let Some(min_score) = applicable.iter().map(|(_, s)| *s).min() {
                let at_min: Vec<Value<'gc>> = applicable
                    .iter()
                    .filter(|(_, s)| *s == min_score)
                    .map(|(mv, _)| *mv)
                    .collect();
                if at_min.len() >= 2 && min_score.0 != i64::MAX {
                    return Err(self.ambiguous_method_error(selector, class_ref, &at_min, args));
                }
                return Ok(Some(at_min[0]));
            }
        }

        for &mixin in mixins {
            if let Some(method) = self.lookup_method_in_class_hierarchy_rec(
                mc, mixin, receiver, selector, class_side, args, visited,
            )? {
                return Ok(Some(method));
            }
        }
        if let Some(p) = parent {
            if let Some(method) = self.lookup_method_in_class_hierarchy_rec(
                mc, p, receiver, selector, class_side, args, visited,
            )? {
                return Ok(Some(method));
            }
        }
        Ok(None)
    }

    /// Score how well a method variant applies to `args` — lower is more specific.
    /// Returns `None` if it doesn't apply (a typed parameter's argument isn't
    /// assignable, a guard fails, or there are too few arguments).
    ///
    /// The score sums, over parameters: a typed parameter's class-hierarchy
    /// distance from the argument's class to the declared type (exact match = 0,
    /// +1 per hop up); an untyped parameter contributes a large constant so a typed
    /// parameter always beats an untyped one. Parameter types and guard are read
    /// through `get_block_from_method`, so this is agnostic to how a method is
    /// stored: a legacy native method (no block) is treated as an untyped fallback
    /// that matches anything — once native methods carry signatures they will score
    /// by type like any other variant. (This replaces the old pairwise
    /// `compare_specificity`, which wasn't a total order.)
    fn match_score(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        method_val: Value<'gc>,
        args: &[Value<'gc>],
    ) -> Result<Option<(i64, u8)>, QuoinError> {
        let block = match self.get_block_from_method(method_val) {
            Some(b) => b,
            None => {
                // No user block: a native method (always unguarded — rank 1). Score
                // by its declared signature if it has one; a signatureless legacy
                // native makes no specificity claim and ranks last as a pure fallback.
                return Ok(match self.native_method_param_types(method_val) {
                    Some(param_types) => self
                        .score_param_types(&param_types, &[], args)
                        .map(|s| (s, 1)),
                    None => Some((i64::MAX, 1)),
                });
            }
        };
        // A tag-requiring variant makes the whole resolution uncacheable: the
        // (kind, class-ptr) guards can't distinguish an Integer-tagged list
        // from a String-tagged or untagged one, so a cached selection would be
        // wrong for the next differently-tagged argument (GENERICS_ARCH.md §5).
        // Same mechanism guarded variants use; legacy chains never hit this.
        if block.template.param_elem_tags.iter().any(|t| t.is_some()) {
            self.dispatch_cache.uncacheable = true;
        }
        let param_score = match self.score_param_types(
            &block.template.param_types,
            &block.template.param_elem_tags,
            args,
        ) {
            Some(s) => s,
            None => return Ok(None),
        };
        // A guard *refines* specificity within a parameter-type level: a guarded
        // variant whose guard passes (rank 0) outranks an otherwise-equal unguarded
        // variant (rank 1). A failing guard makes the variant inapplicable.
        let guard_rank = if let Some(decl_block) = block.decl_block {
            // This selector has a guarded variant on the class being examined, so the
            // resolution can depend on argument *values*, not just types — mark the
            // in-progress lookup uncacheable. Set before running the guard so a throw
            // (or a failing-guard early return) still disables caching, and so it
            // holds regardless of which candidate ultimately wins.
            self.dispatch_cache.uncacheable = true;
            let res = self.execute_validation_block(
                mc,
                decl_block,
                receiver,
                &block.template.param_syms,
                args,
            )?;
            if !res.is_true() {
                return Ok(None);
            }
            0
        } else {
            1
        };
        Ok(Some((param_score, guard_rank)))
    }

    /// Build an `AmbiguousMethod` error naming the equally-specific candidates that
    /// tied for `selector` on `class_ref` given `args`. The candidate signatures
    /// render one-per-line at the error site (see `QuoinError` Display).
    fn ambiguous_method_error(
        &self,
        selector: Symbol,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        candidates: &[Value<'gc>],
        args: &[Value<'gc>],
    ) -> QuoinError {
        let class_name = class_ref.borrow().name.to_string();
        let arg_types: Vec<String> = args.iter().map(|a| a.class_name()).collect();
        let msg = format!(
            "ambiguous dispatch for '{}' on {} with argument type(s) ({}): {} equally-specific candidates",
            selector,
            class_name,
            arg_types.join(", "),
            candidates.len(),
        );
        QuoinError::AmbiguousMethod {
            selector: selector.as_str().to_string(),
            msg,
            candidates: candidates
                .iter()
                .map(|&mv| self.format_candidate_signature(mv, selector))
                .collect(),
        }
    }

    /// Every variant sharing `selector` reachable from `receiver`'s class hierarchy
    /// (instance- or class-side as appropriate), in hierarchy order, regardless of
    /// whether it applies to the current arguments. Used to enrich a
    /// `MessageNotUnderstood` with the candidates dispatch filtered out.
    pub(crate) fn collect_method_candidates(
        &self,
        receiver: Value<'gc>,
        selector: Symbol,
    ) -> Vec<Value<'gc>> {
        let class_side = matches!(receiver, Value::Class(_) | Value::ClassMeta(_));
        let mut out = Vec::new();
        if let Some(class) = self.get_class_for_lookup(receiver) {
            let mut visited = Vec::new();
            self.collect_candidates_rec(class, selector, class_side, &mut visited, &mut out);
        }
        out
    }

    fn collect_candidates_rec(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: Symbol,
        class_side: bool,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
        out: &mut Vec<Value<'gc>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);
        let class_borrow = class_ref.borrow();
        let methods = if class_side {
            &class_borrow.class_methods
        } else {
            &class_borrow.instance_methods
        };
        let chain_start = methods.get(&selector).copied();
        let mixins = class_borrow.mixin_classes.clone();
        let parent = class_borrow.parent;
        drop(class_borrow);
        let mut curr = chain_start;
        while let Some(mv) = curr {
            out.push(mv);
            curr = self.get_next_method_in_chain(mv);
        }
        for mixin in mixins {
            self.collect_candidates_rec(mixin, selector, class_side, visited, out);
        }
        if let Some(p) = parent {
            self.collect_candidates_rec(p, selector, class_side, visited, out);
        }
    }

    /// Format a candidate's signature in the stack-trace style: the selector's
    /// keywords interleaved with the variant's *declared* parameter types (e.g.
    /// `foo:Integer bar:Object`), plus the guard appended for a guarded variant —
    /// its syntax-highlighted source if available, else a `{...}` placeholder.
    pub(crate) fn format_candidate_signature(
        &self,
        method_val: Value<'gc>,
        selector: Symbol,
    ) -> String {
        let supports_color = self.options.supports_color;
        let types = self.candidate_param_types(method_val);
        let mut sig = Self::format_selector_with_types(selector.as_str(), &types, supports_color);
        if let Some(guard) = self.candidate_guard_display(method_val, supports_color) {
            sig.push(' ');
            sig.push_str(&guard);
        }
        sig
    }

    /// The declared parameter types of a candidate (a user block carries them
    /// directly; a native method via its signature, empty if signatureless).
    pub(crate) fn candidate_param_types(&self, method_val: Value<'gc>) -> Vec<String> {
        if let Some(block) = self.get_block_from_method(method_val) {
            block.template.param_types.clone()
        } else {
            self.native_method_param_types(method_val)
                .unwrap_or_default()
        }
    }

    /// The declared checker return type of a candidate, or `None`. Native methods carry it via
    /// `.returns(..)` (Fork-1b native half); user blocks return `None` here — their declared
    /// returns reach the checker via the AST-recording path, not runtime introspection (the
    /// compiler half of Fork-1b, still deferred).
    pub(crate) fn candidate_ret_type(&self, method_val: Value<'gc>) -> Option<String> {
        if self.get_block_from_method(method_val).is_some() {
            None
        } else {
            self.native_method_ret_type(method_val)
        }
    }

    /// A guarded variant's guard for display: its syntax-highlighted source (e.g.
    /// `{x > 5}`), or a colorized `{...}` placeholder when source text is absent.
    /// `None` for an unguarded variant.
    fn candidate_guard_display(
        &self,
        method_val: Value<'gc>,
        supports_color: bool,
    ) -> Option<String> {
        let block = self.get_block_from_method(method_val)?;
        let decl = block.decl_block?;
        // `source_info.source_text` is the node's own text (the guard span), so it
        // already holds the guard source — no slicing needed.
        let src = decl
            .template
            .source_info
            .as_ref()
            .and_then(|si| si.source_text.as_ref())
            .map(|s| s.trim().to_string());
        let display = match src {
            Some(s) if !s.is_empty() => {
                let braced = if s.starts_with('{') {
                    s
                } else {
                    format!("{{{}}}", s)
                };
                if supports_color {
                    crate::highlighter::highlight_to_ansi(&braced)
                        .trim_end()
                        .to_string()
                } else {
                    braced
                }
            }
            _ => {
                if supports_color {
                    crate::ansi_colorizer::colorize("$#808080[{...}$]")
                } else {
                    "{...}".to_string()
                }
            }
        };
        Some(display)
    }

    /// Interleave a selector's keywords with `types` (e.g. `foo:Integer bar:Object`),
    /// matching the stack-trace rendering style. A keyword with no corresponding type
    /// (a no-arg selector, or a native signature shorter than the selector) prints
    /// bare. Colorized to match traces when `supports_color`.
    fn format_selector_with_types(
        selector: &str,
        types: &[String],
        supports_color: bool,
    ) -> String {
        // Split into keyword parts, each keeping its trailing ':'.
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        for c in selector.chars() {
            current.push(c);
            if c == ':' {
                parts.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            parts.push(current);
        }
        let mut out = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let mut keyword = part.clone();
            let has_colon = keyword.ends_with(':');
            if has_colon {
                keyword.pop();
            }
            match types.get(i) {
                Some(ty) if has_colon => {
                    if supports_color {
                        out.push(crate::ansi_colorizer::colorize(&format!(
                            "$#ab82ff[{}$]$#808080[:$]$#5fd7af[{}$]",
                            keyword, ty
                        )));
                    } else {
                        out.push(format!("{}:{}", keyword, ty));
                    }
                }
                _ => {
                    let text = part.clone();
                    if supports_color {
                        out.push(crate::ansi_colorizer::colorize(&format!(
                            "$#ab82ff[{}$]",
                            text
                        )));
                    } else {
                        out.push(text);
                    }
                }
            }
        }
        out.join(" ")
    }

    /// Distance assigned to `:Object` when a value can't physically walk up to
    /// Object (metaclasses) — large, so such matches rank last while still matching.
    const OBJECT_UNIVERSAL_DISTANCE: i64 = 1_000_000;

    /// Sum of per-parameter class-hierarchy distances (`type_distance`), or `None`
    /// if any parameter's argument isn't assignable to its type (or there are too
    /// few arguments). Every parameter carries a type: a user block defaults an
    /// unannotated parameter to `Object` (the universal supertype) at compile time,
    /// so a more-specific type always beats it.
    ///
    /// Native methods differ *slightly*: their signature is supplied directly by the
    /// builder (`typed_instance_method`, …) and may be **shorter** than the argument
    /// list — only the leading args are typed, and any trailing args are left
    /// unconstrained (not defaulted to `Object`). E.g. `List#at:put:` declares
    /// `["Integer"]`, constraining the index but leaving the value free. (A native
    /// method with no signature at all is handled by `match_score` as a fallback.)
    fn score_param_types(
        &self,
        param_types: &[String],
        elem_tags: &[Option<crate::runtime::elem_tag::ElemTag>],
        args: &[Value<'gc>],
    ) -> Option<i64> {
        if args.len() < param_types.len() {
            return None;
        }
        // Base distances are doubled so a satisfied tag requirement can sit
        // between them as a discount: legacy scoring (empty `elem_tags` — every
        // pre-generics block, normalized at compile time) is a pure monotonic
        // scaling with ZERO tag probes, identical orderings everywhere.
        let mut score: i64 = 0;
        if elem_tags.is_empty() {
            for (i, hint) in param_types.iter().enumerate() {
                score += 2 * self.type_distance(args[i], hint)?;
            }
            return Some(score);
        }
        for (i, hint) in param_types.iter().enumerate() {
            score += 2 * self.type_distance(args[i], hint)?;
            // Tag-aware dispatch (GENERICS_ARCH.md §5): a `List(Integer)` param
            // matches only an Integer-tagged list — untagged or differently
            // tagged falls through to a bare-`List`/`Object` variant or MNU. A
            // satisfied requirement DISCOUNTS the param (strictly more specific
            // than the requirement-free variant at the same base distance).
            if let Some(required) = elem_tags.get(i).copied().flatten() {
                if crate::runtime::elem_tag::value_elem_tag(&args[i]) != Some(required) {
                    return None;
                }
                score -= 1;
            }
        }
        Some(score)
    }

    /// Declared parameter types of a native method variant, or `None` for a user
    /// block, an untyped (legacy) native method, or a non-method.
    fn native_method_param_types(&self, method_val: Value<'gc>) -> Option<Vec<String>> {
        method_val
            .with_native_state::<NativeMethodState, _, _>(|m| m.native_param_types())
            .ok()
            .flatten()
    }

    fn native_method_ret_type(&self, method_val: Value<'gc>) -> Option<String> {
        method_val
            .with_native_state::<NativeMethodState, _, _>(|m| m.native_ret_type())
            .ok()
            .flatten()
    }

    /// A native candidate's `.doc(..)` text, or `None`. User blocks answer `None` here — their
    /// doc lives in source, in the `"*` block above the definition (docs/DOCS_ARCH.md §4), and
    /// is extracted lazily from `MethodVariant.source` rather than carried at runtime.
    pub(crate) fn candidate_doc(&self, method_val: Value<'gc>) -> Option<String> {
        if self.get_block_from_method(method_val).is_some() {
            None
        } else {
            method_val
                .with_native_state::<NativeMethodState, _, _>(|m| m.native_doc())
                .ok()
                .flatten()
        }
    }

    /// Class-hierarchy distance from `val`'s class to the class named `hint` (0 if
    /// `val` is directly of that type), or `None` if `val` isn't an instance of it.
    /// A mixin counts as one hop from the class that mixes it in.
    fn type_distance(&self, val: Value<'gc>, hint: &str) -> Option<i64> {
        // Fast path / exact match. Also the only thing that matches a `Class` or
        // `ClassMeta` value (whose `get_class_for_lookup` returns the class itself,
        // not a class named "Class"). `Object` is exempt: `type_name()` reports
        // "Object" for every plain user object, so taking this shortcut would score
        // an untyped catch-all (`|x|` ⇒ `:Object`) at 0 — tying with, instead of
        // losing to, an exact class match. Let the walk below rank `Object` by its
        // real distance (or the universal fallback).
        if val.type_name() == hint && hint != "Object" {
            return Some(0);
        }
        let val_class = self.get_class_for_lookup(val)?;
        // Resolve the hint to a class so we can match by identity; fall back to
        // matching by name when it isn't a known global. The hint is a rendered
        // `NamespacedName` (`Foo`, `[Web]Halt`), so parse it back for the lookup:
        // a bare hint means the root namespace, never a leaf-name match against
        // some `[X]Foo` (annotations resolve exactly like expression-position names).
        let hint_name = NamespacedName::parse(hint);
        let target = match self.globals.borrow().get(&hint_name).copied() {
            Some(Value::Class(c)) => Some(c),
            _ => None,
        };
        let matches = |clz: Gc<'gc, RefLock<Class<'gc>>>| match target {
            Some(t) => Gc::ptr_eq(clz, t),
            None => clz.borrow().name == hint_name,
        };
        let mut curr = Some(val_class);
        let mut dist: i64 = 0;
        while let Some(clz) = curr {
            if matches(clz) {
                return Some(dist);
            }
            let (mixins, parent) = {
                let b = clz.borrow();
                (b.mixin_classes.clone(), b.parent)
            };
            if mixins.iter().any(|m| matches(*m)) {
                return Some(dist + 1);
            }
            curr = parent;
            dist += 1;
        }
        // `Object` is the universal supertype: it matches every value. Some values
        // (notably metaclasses — see QUOIN_TODO about making Class/ClassMeta subclass
        // Object directly) don't physically reach Object via `parent`, so they fall
        // here and rank last via this large fallback distance.
        if hint == "Object" {
            return Some(Self::OBJECT_UNIVERSAL_DISTANCE);
        }
        None
    }

    /// Whether `val`'s class is (or descends from) the type named `hint` — the same subtype
    /// test method dispatch uses, as a bool. Used by typed `catch:`/`catch+:` to decide whether
    /// a handler catches a thrown value.
    pub(crate) fn value_matches_type(&self, val: Value<'gc>, hint: &str) -> bool {
        self.type_distance(val, hint).is_some()
    }

    pub(crate) fn get_block_from_method(
        &self,
        method_val: Value<'gc>,
    ) -> Option<Gc<'gc, Block<'gc>>> {
        if let Value::Object(obj) = method_val {
            match &obj.borrow().payload {
                ObjectPayload::Block(block) => Some(*block),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                        if let Some(Value::Object(block_obj)) = method_state.get_block()
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Some(*block)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        }
    }

    pub(crate) fn get_next_method_in_chain(&self, method_val: Value<'gc>) -> Option<Value<'gc>> {
        if let Value::Object(obj) = method_val {
            let payload = &obj.borrow().payload;
            if let ObjectPayload::NativeState(state_cell) = payload {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                    return method_state.next.map(|n| unsafe { transmute(n) });
                }
            }
        }
        None
    }
}
