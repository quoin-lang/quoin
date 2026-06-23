use crate::dispatch::{Callable, MethodCacheKey};
use crate::error::QuoinError;
use crate::fiber::{VMYielder, YieldReason};
use crate::highlighter::{HighlightParser, HighlightSpan, format_ansi};
use crate::instruction::{Constant, Instruction};
use crate::io_backend::StreamId;
use crate::packages::{FsResolver, LoadedUnit, PackageResolver};
use crate::parser::parse_quoin_string;
use crate::runtime::fiber::NativeFiberState;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::method::{MethodBody, NativeMethodState};
use crate::runtime::regex::NativeRegexState;
use crate::runtime::runtime::{load_glob, load_unit};
use crate::runtime::set::NativeSetState;
use crate::symbol::{Symbol, self_symbol};
use crate::value::{
    AnyCollect, Block, Class, EnvFrame, NamespacedName, NativeCall, NativeClass, NativeFunc,
    Object, ObjectPayload, Value,
};
use crate::{ansi_colorizer, gc, gcl};

use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use regex::Regex;
use rustc_hash::FxHashMap;
use std::collections::{HashMap, VecDeque};
use std::mem::transmute;
use std::path::Path;
use std::{cmp, fs};

/// A method call queued to run when its frame completes normally (a "defer").
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct DeferredCall<'gc> {
    pub receiver: Value<'gc>,
    #[collect(require_static)]
    pub selector: String,
    pub args: Vec<Value<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Frame<'gc> {
    pub id: usize,
    pub is_nested_block: bool,
    pub enclosing_method_id: Option<usize>,
    pub block: Gc<'gc, Block<'gc>>,
    pub ip: usize,
    pub env: Gc<'gc, RefLock<EnvFrame<'gc>>>,
    pub instantiating_obj: Option<Gc<'gc, RefLock<Object<'gc>>>>,
    pub receiver: Option<Value<'gc>>,
    pub selector: Option<Symbol>,
    pub args: Vec<Value<'gc>>,
    pub stack_base: usize,
    pub return_receiver: bool,
    /// Calls queued (e.g. by `mix:`) to run when this frame returns normally.
    pub defers: Vec<DeferredCall<'gc>>,
    /// If set, and a deferred call throws, remove this global before propagating
    /// (used so a class whose mixin requirements fail is never left registered).
    #[collect(require_static)]
    pub unregister_on_defer_failure: Option<NamespacedName>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct BuiltinCache<'gc> {
    pub nil_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub boolean_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub integer_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub double_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub string_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub list_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub map_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub regex_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub block_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    // `true <-- {…}` / `false <-- {…}` need separate method tables; an immediate
    // carries no per-instance class, so the synthesized singletons live here.
    pub true_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub false_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
}

impl<'gc> BuiltinCache<'gc> {
    pub fn new() -> Self {
        Self {
            nil_class: None,
            boolean_class: None,
            integer_class: None,
            double_class: None,
            string_class: None,
            list_class: None,
            map_class: None,
            regex_class: None,
            block_class: None,
            true_class: None,
            false_class: None,
        }
    }
}

#[derive(Clone, Debug, Default, Collect)]
#[collect(require_static)]
pub struct VmOptions {
    pub arguments: Vec<String>,
    pub supports_color: bool,
    pub console_width: Option<u16>,
}

// The scheduler / task / guest-fiber subsystem lives in `vm_scheduler.rs` (still
// intrinsically VM state); its public types are re-exported here so callers that
// `use crate::vm::{Task, Wake, ...}` are unaffected by the move.
pub use crate::vm_scheduler::{GatherState, Scheduler, Task, TaskId, Wake};

#[derive(Collect)]
#[collect(no_drop)]
pub struct VmState<'gc> {
    pub stack: Vec<Value<'gc>>,
    pub frames: Vec<Frame<'gc>>,
    pub globals: Gc<'gc, RefLock<HashMap<NamespacedName, Value<'gc>>>>,
    /// Intern pool for symbols: one canonical `Symbol` value per name, so symbols
    /// compare by identity. Rooted here and traced as part of `VmState`.
    pub symbol_table: Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>,
    /// Name of the class just created by `DefineClass`, consumed by the next
    /// `ExecuteBlockWithSelf` to mark the class body's frame for unregister-on-
    /// defer-failure. Only a *new* class definition sets this (not an extension).
    #[collect(require_static)]
    pub pending_class_def: Option<NamespacedName>,
    pub next_frame_id: usize,

    pub builtin_cache: Gc<'gc, RefLock<BuiltinCache<'gc>>>,
    pub active_exception: Option<Value<'gc>>,
    /// Arguments of the most recent send that failed *in place* (no callee frame of
    /// its own) — read only by the stack-trace formatter (`annotate_error`). Set on
    /// the in-place-error branches of the `Send` handler / `Callable::call`, not on
    /// every send.
    pub last_send_args: Vec<Value<'gc>>,
    pub active_native_args: Vec<NativeCall<'gc>>,

    pub last_popped_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,

    /// Coroutine / guest-fiber scheduler state, grouped out for legibility (see
    /// the [`Scheduler`] struct). Stored inline by value — no indirection.
    pub sched: Scheduler<'gc>,

    /// Memoized method resolution: `(searched-class ptr, selector, class-side,
    /// arg-class ptrs)` → resolved method (or `None` when the hierarchy has no
    /// match). Populated by `lookup_method_in_class_hierarchy` for guard-free,
    /// non-eigenclass lookups; cleared whenever a class's method table changes
    /// (`invalidate_method_cache`). Traced as part of `VmState` so cached method
    /// `Value`s stay live; the key's class *pointers* are sound because named
    /// classes are globals-rooted (stable address) — eigenclasses are excluded.
    /// Keyed by an all-integer `MethodCacheKey`, so it uses `FxHashMap` (FxHash) —
    /// SipHash's worst case, FxHash's best — for a faster per-send probe.
    pub method_cache: FxHashMap<MethodCacheKey, Option<Value<'gc>>>,
    /// Scratch flag marking the in-progress lookup's result as un-memoizable. Set
    /// by `match_score` when a guarded candidate is examined — a guard's outcome
    /// depends on argument *values*, not just types, so the resolution can't be keyed
    /// on classes alone. Saved/restored around each `lookup_method_in_class_hierarchy`
    /// call for re-entrancy safety (a guard's nested sends run their own lookups
    /// without corrupting this one).
    #[collect(require_static)]
    pub dispatch_uncacheable: bool,

    /// Resolves `use (pkg:)? path` to source — the filesystem-agnostic seam, swappable
    /// per host (FS on the CLI, in-memory on WASM/embedded). See `src/packages.rs`.
    #[collect(require_static)]
    pub resolver: Box<dyn PackageResolver>,
    /// Run-once registry for `use`, in load order (a `Vec`, not a set: run order *is*
    /// load order). A per-entry status breaks cycles. See `USE_ARCH.md`.
    #[collect(require_static)]
    pub loaded: Vec<LoadedUnit>,

    #[collect(require_static)]
    pub options: VmOptions,

    /// fds whose QN `TcpSocket` handle has been closed or collected, awaiting a
    /// synchronous `IoBackend::close` by the driver. A non-GC queue (the handle's
    /// `Drop` can only push a plain `StreamId`); see `docs/ASYNC_ARCH.md` resource
    /// lifecycle. Shared `Rc` clone lives in each socket handle.
    #[collect(require_static)]
    pub socket_reap: std::rc::Rc<std::cell::RefCell<Vec<StreamId>>>,
}

pub enum VmStatus<'gc> {
    Running,
    Finished(Value<'gc>),
    Yeeted(Value<'gc>), // Uncaught exception
}

impl<'gc> VmState<'gc> {
    pub unsafe fn get_yielder(&self) -> Option<&VMYielder<'gc>> {
        self.sched
            .yielder
            .map(|ptr| unsafe { &*(ptr as *const VMYielder<'gc>) })
    }

    /// Record the running coroutine's yielder into the current fiber's slot (or
    /// the main slot) and make it live. Called once at the top of `run_vm_loop`.
    pub fn register_yielder(&mut self, mc: &Mutation<'gc>, ptr: *const ()) {
        match self.sched.current_fiber {
            None => {
                if let Some(task) = self
                    .sched
                    .tasks
                    .get_mut(self.sched.current_task.0)
                    .and_then(|t| t.as_mut())
                {
                    task.root_yielder = Some(ptr);
                }
            }
            Some(f) => {
                let _ =
                    f.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.set_yielder(ptr));
            }
        }
        self.sched.yielder = Some(ptr);
    }

    /// The stored yielder for whichever fiber is current (main if `None`). The
    /// driver loads this into `self.sched.yielder` before resuming, guaranteeing it
    /// always points at the live, GC-rooted coroutine being run.
    pub fn current_fiber_yielder(&self) -> Option<*const ()> {
        match self.sched.current_fiber {
            None => self
                .sched
                .tasks
                .get(self.sched.current_task.0)
                .and_then(|t| t.as_ref())
                .and_then(|t| t.root_yielder),
            Some(f) => f
                .with_native_state::<NativeFiberState, _, _>(|s| s.yielder())
                .ok()
                .flatten(),
        }
    }

    pub fn new(mc: &Mutation<'gc>, options: VmOptions) -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            globals: gcl!(mc, HashMap::new()),
            symbol_table: gcl!(mc, HashMap::new()),
            pending_class_def: None,
            next_frame_id: 1,
            builtin_cache: gcl!(mc, BuiltinCache::new()),
            active_exception: None,
            last_send_args: Vec::new(),
            active_native_args: Vec::new(),
            last_popped_env: None,
            sched: Scheduler {
                yielder: None,
                tasks: Vec::new(),
                ready: VecDeque::new(),
                current_task: TaskId(0),
                active_fiber: None,
                current_fiber: None,
                resume_stack: Vec::new(),
                fiber_transfer: None,
                main_saved_stack: Vec::new(),
                main_saved_frames: Vec::new(),
                main_saved_native_args: Vec::new(),
                fiber_error: None,
                wake: None,
                cancel_current: false,
            },
            method_cache: FxHashMap::default(),
            dispatch_uncacheable: false,
            resolver: Box::new(FsResolver::new()),
            loaded: Vec::new(),
            options,
            socket_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        }
    }

    pub fn new_object(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, RefLock<Object<'gc>>> {
        let count = self.ensure_field_layout(mc, class_obj);
        let nil_val = self.new_nil(mc);
        let fields = vec![nil_val; count].into_boxed_slice();
        gcl!(
            mc,
            Object {
                class: class_obj,
                fields,
                payload: ObjectPayload::Instance,
            }
        )
    }

    /// Ensure `class.field_slots` covers the full current hierarchy (own + mixins +
    /// parent) and return the field count. Append-only: a newly-seen ivar gets a
    /// fresh trailing slot, so existing slots stay stable across runtime mixins.
    fn ensure_field_layout(
        &self,
        mc: &Mutation<'gc>,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> usize {
        let all = self.get_all_instance_vars(class);
        let mut c = class.borrow_mut(mc);
        for name in all {
            if !c.field_slots.contains_key(&name) {
                let slot = c.field_slots.len();
                c.field_slots.insert(name, slot);
            }
        }
        c.field_slots.len()
    }

    /// The absolute slot of instance variable `name` for instances of `class`
    /// (the layout is populated at instantiation), or `None` if it's not a declared
    /// ivar of the class.
    fn field_slot(&self, class: Gc<'gc, RefLock<Class<'gc>>>, name: &str) -> Option<usize> {
        class.borrow().field_slots.get(name).copied()
    }

    pub fn new_native_state<T: AnyCollect + 'static>(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        state: T,
    ) -> Value<'gc> {
        let payload = ObjectPayload::NativeState(gcl!(mc, Box::new(state) as Box<dyn AnyCollect>));
        let obj = gcl!(
            mc,
            Object {
                class: class_obj,
                fields: Box::default(),
                payload,
            }
        );
        Value::Object(obj)
    }

    // Scalar value types are immediate `Value` variants — no GC allocation. `mc`
    // is kept in the signatures so the many call sites stay unchanged.
    pub fn new_nil(&self, _mc: &Mutation<'gc>) -> Value<'gc> {
        Value::Nil
    }

    pub fn new_bool(&self, _mc: &Mutation<'gc>, b: bool) -> Value<'gc> {
        Value::Bool(b)
    }

    pub fn new_int(&self, _mc: &Mutation<'gc>, i: i64) -> Value<'gc> {
        Value::Int(i)
    }

    pub fn new_double(&self, _mc: &Mutation<'gc>, f: f64) -> Value<'gc> {
        Value::Double(f)
    }

    pub fn new_string(&self, mc: &Mutation<'gc>, s: String) -> Value<'gc> {
        let class = self.builtin_cache.borrow().string_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "String"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::String(gc!(mc, s)),
            }
        ))
    }

    /// Build an immutable `Bytes` value from raw bytes (mirrors `new_string`). One
    /// copy at the native boundary; the inner `Vec<u8>` is a GC leaf.
    pub fn new_bytes(&self, mc: &Mutation<'gc>, bytes: Vec<u8>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Bytes");
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::Bytes(gc!(mc, bytes)),
            }
        ))
    }

    /// Return the interned `Symbol` value for `name`, creating it on first use.
    /// All occurrences of the same name share one value, so symbols compare by
    /// identity.
    pub fn new_symbol(&self, mc: &Mutation<'gc>, name: String) -> Value<'gc> {
        let existing = self.symbol_table.borrow().get(&name).copied();
        if let Some(sym) = existing {
            return sym;
        }
        let class = self.get_or_create_builtin_class(mc, "Symbol");
        let sym = Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::Symbol(gc!(mc, name.clone())),
            }
        ));
        self.symbol_table.borrow_mut(mc).insert(name, sym);
        sym
    }

    #[allow(clippy::wrong_self_convention)]
    #[allow(no_gc_across_yield)]
    pub fn to_s(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        match value {
            Value::Class(_) | Value::ClassMeta(_) => {
                let display = value.to_string();
                Ok(self.new_string(mc, display))
            }
            // Object + immediate value types dispatch their `s` method.
            _ => self.call_method(mc, value, "s", vec![]),
        }
    }

    pub fn new_list(&self, mc: &Mutation<'gc>, list: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().list_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "List"));
        let state = NativeListState::new(list);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_map(&self, mc: &Mutation<'gc>, map: HashMap<String, Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().map_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Map"));
        let boxed_state: Box<dyn AnyCollect> = Box::new(NativeMapState::new(map));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_set(&self, mc: &Mutation<'gc>, set: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Set");
        let boxed_state: Box<dyn AnyCollect> = Box::new(NativeSetState::new(set));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// True if `set_val` already contains a value equal (by Quoin `==:`) to `value`.
    pub fn set_contains(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let len = set_val
            .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
            .map_err(|e| QuoinError::Other(e))?;
        for i in 0..len {
            let elem = set_val
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
                .map_err(|e| QuoinError::Other(e))?;
            if self.call_method(mc, elem, "==:", vec![value])?.is_true() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Insert `value` into `set_val` unless an equal element is already present.
    /// Returns whether a new element was added.
    pub fn set_add(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        if self.set_contains(mc, set_val, value)? {
            Ok(false)
        } else {
            set_val
                .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.get_vec_mut().push(value))
                .map_err(|e| QuoinError::Other(e))?;
            Ok(true)
        }
    }

    /// Remove the first element of `set_val` equal (by `==:`) to `value`.
    /// Returns whether an element was removed.
    pub fn set_remove(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let len = set_val
            .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
            .map_err(|e| QuoinError::Other(e))?;
        for i in 0..len {
            let elem = set_val
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
                .map_err(|e| QuoinError::Other(e))?;
            if self.call_method(mc, elem, "==:", vec![value])?.is_true() {
                set_val
                    .with_native_state_mut::<NativeSetState, _, _>(mc, |s| {
                        s.get_vec_mut().remove(i);
                    })
                    .map_err(|e| QuoinError::Other(e))?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn new_regex(&self, mc: &Mutation<'gc>, regex: Regex) -> Value<'gc> {
        let class = self.builtin_cache.borrow().regex_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Regex"));
        let boxed_state: Box<dyn AnyCollect> = Box::new(NativeRegexState::new(regex));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_block(&self, mc: &Mutation<'gc>, block: Block<'gc>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().block_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Block"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::Block(gc!(mc, block)),
            }
        ))
    }

    pub fn new_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        block: Value<'gc>,
        is_extension: bool,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new(selector, block, is_extension);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Wrap a native fn as a `Method` chain node, so native methods are chainable,
    /// scored, and override-able just like user methods.
    pub fn new_native_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        func: NativeFunc,
        param_types: Option<Vec<String>>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_native(selector, func, param_types);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Box::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    #[allow(no_gc_across_yield)]
    fn finalize_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        env_borrow: &EnvFrame<'gc>,
    ) -> Result<(), QuoinError> {
        let class = obj.borrow().class;
        let vars = self.get_all_instance_vars(class);
        for var in &vars {
            if let Some(val) = env_borrow.lookup_str(var)
                && let Some(slot) = self.field_slot(class, var)
            {
                obj.borrow_mut(mc).fields[slot] = val;
            }
        }

        // Run each class's initializer base->derived (parents, then mixins, then
        // self), mirroring `run_all_inits` for the no-block path. A class that
        // defines `init:` receives the block fields it names (matched by param
        // name); otherwise its zero-arg `init` runs. Running the whole chain means
        // an ancestor or mixin initializer is never skipped just because a more
        // derived class happens to define `init:`.
        let mut classes = Vec::new();
        let mut visited = Vec::new();
        self.collect_classes_for_init(obj.borrow().class, &mut classes, &mut visited);

        let receiver = Value::Object(obj);
        for clz in classes {
            let init_colon = clz.borrow().instance_methods.get("init:").copied();
            if let Some(method_val) = init_colon {
                let param_names = self.init_param_names(method_val).unwrap_or_default();
                let mut init_args = Vec::new();
                for param in &param_names {
                    let val = env_borrow
                        .lookup_str(param)
                        .unwrap_or_else(|| self.new_nil(mc));
                    init_args.push(val);
                }
                self.call_method_value(mc, receiver, method_val, "init:", init_args)?;
            } else if let Some(method_val) = clz.borrow().instance_methods.get("init").copied() {
                self.call_method_value(mc, receiver, method_val, "init", Vec::new())?;
            }
        }

        Ok(())
    }

    /// Parameter names of a method's underlying block, used so `init:` can be fed
    /// the `new:{}` block fields it declares by name. Handles both plain block
    /// methods and native-wrapped method state.
    fn init_param_names(&self, method_val: Value<'gc>) -> Option<Vec<String>> {
        let Value::Object(io) = method_val else {
            return None;
        };
        let io_ref = io.borrow();
        match &io_ref.payload {
            ObjectPayload::Block(b) => Some(
                b.param_syms
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect(),
            ),
            ObjectPayload::NativeState(state_cell) => {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                let method_state = any_ref.downcast_ref::<NativeMethodState>()?;
                if let Some(Value::Object(block_obj)) = method_state.get_block()
                    && let ObjectPayload::Block(b) = &block_obj.borrow().payload
                {
                    Some(
                        b.param_syms
                            .iter()
                            .map(|s| s.as_str().to_string())
                            .collect(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_or_create_builtin_class(
        &self,
        mc: &Mutation<'gc>,
        name: &str,
    ) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            let parent = if name == "Object" {
                None
            } else {
                Some(self.get_or_create_builtin_class(mc, "Object"))
            };
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent,
                    instance_vars: Vec::new(),
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                    mixin_classes: Vec::new(),
                    field_slots: HashMap::new(),
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract: false,
                }
            );
            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
            class_obj
        }
    }

    pub fn get_builtin_class(&self, name: &str) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            panic!("Builtin class {} not found in globals!", name);
        }
    }

    pub fn register_native_class<T: NativeClass>(&mut self, mc: &Mutation<'gc>, native_class: T) {
        let parent_class = if let Some(parent_name) = native_class.parent_name() {
            Some(self.get_or_create_builtin_class(mc, parent_name))
        } else {
            None
        };

        // Several defs may share a selector (typed multimethod variants); chain
        // them in declaration order so the scorer routes by argument type and ties
        // resolve to the first-declared.
        let mut inst_methods: HashMap<String, Value<'gc>> = HashMap::new();
        for def in native_class.instance_methods() {
            let node = self.new_native_method(mc, def.selector.clone(), def.func, def.param_types);
            if let Some(head) = inst_methods.get(&def.selector).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                inst_methods.insert(def.selector, node);
            }
        }

        let mut cls_methods: HashMap<String, Value<'gc>> = HashMap::new();
        for def in native_class.class_methods() {
            let node = self.new_native_method(mc, def.selector.clone(), def.func, def.param_types);
            if let Some(head) = cls_methods.get(&def.selector).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                cls_methods.insert(def.selector, node);
            }
        }

        let name = native_class.name();
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(existing_class)) = existing {
            let mut borrowed = existing_class.borrow_mut(mc);
            borrowed.parent = parent_class;
            borrowed.instance_methods = inst_methods;
            borrowed.class_methods = cls_methods;
            borrowed.instance_vars = Vec::new();
        } else {
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent: parent_class,
                    instance_vars: Vec::new(),
                    instance_methods: inst_methods,
                    class_methods: cls_methods,
                    mixin_classes: Vec::new(),
                    field_slots: HashMap::new(),
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract: false,
                }
            );

            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
        }
        // A class's method tables just changed — drop any memoized resolutions.
        self.invalidate_method_cache();
    }
    #[allow(no_gc_across_yield)]
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
            method.call(self, mc, Some(receiver), args, Some(sel))?;
            Ok(initial_frame_count)
        } else {
            Err(QuoinError::Other(format!(
                "Method {} not found on receiver",
                selector
            )))
        }
    }

    #[allow(no_gc_across_yield)]
    pub fn call_method(
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
            method.call(self, mc, Some(receiver), args, Some(sel))?;

            // let the VM catch up
            if self.frames.len() > initial_frame_count {
                while self.frames.len() > initial_frame_count {
                    match self.step_internal(mc) {
                        Ok(VmStatus::Running) => {
                            if let Some(yielder) = unsafe { self.get_yielder() } {
                                yielder.suspend(YieldReason::CooperativeYield);
                            }
                        }
                        Ok(VmStatus::Finished(_)) => {
                            break;
                        }
                        Ok(VmStatus::Yeeted(val)) => {
                            return Err(QuoinError::Other(format!(
                                "Uncaught exception during method call: {}",
                                val
                            )));
                        }
                        Err(QuoinError::NonLocalReturn) => {
                            if self.frames.len() > initial_frame_count {
                                continue;
                            } else if self.frames.len() == initial_frame_count {
                                break;
                            } else {
                                return Err(QuoinError::NonLocalReturn);
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    #[allow(no_gc_across_yield)]
    pub fn call_method_value(
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
                        if let Some(func) = method_state.native_func() {
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
            )?;

            // let the VM catch up
            if self.frames.len() > initial_frame_count {
                while self.frames.len() > initial_frame_count {
                    match self.step_internal(mc) {
                        Ok(VmStatus::Running) => {
                            if let Some(yielder) = unsafe { self.get_yielder() } {
                                yielder.suspend(YieldReason::CooperativeYield);
                            }
                        }
                        Ok(VmStatus::Finished(_)) => {
                            break;
                        }
                        Ok(VmStatus::Yeeted(val)) => {
                            return Err(QuoinError::Other(format!(
                                "Uncaught exception during method call: {}",
                                val
                            )));
                        }
                        Err(QuoinError::NonLocalReturn) => {
                            if self.frames.len() > initial_frame_count {
                                continue;
                            } else if self.frames.len() == initial_frame_count {
                                break;
                            } else {
                                return Err(QuoinError::NonLocalReturn);
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    fn collect_classes_for_init(
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
    fn run_defers(
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
        let mut classes = Vec::new();
        let mut visited = Vec::new();
        self.collect_classes_for_init(obj.borrow().class, &mut classes, &mut visited);

        let receiver = Value::Object(obj);
        for clz in classes {
            let method_opt = clz.borrow().instance_methods.get("init").copied();
            if let Some(method_val) = method_opt {
                self.call_method_value(mc, receiver, method_val, "init", Vec::new())?;
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
        let initial_frame_count = self.frames.len();
        if let Some(receiver) = self_val {
            self.start_block_as_method(mc, block, receiver, args, None, false);
        } else {
            self.start_block(mc, block, args, None, None);
        }

        if self.frames.len() > initial_frame_count {
            while self.frames.len() > initial_frame_count {
                match self.step_internal(mc) {
                    Ok(VmStatus::Running) => {
                        if let Some(yielder) = unsafe { self.get_yielder() } {
                            yielder.suspend(YieldReason::CooperativeYield);
                        }
                    }
                    Ok(VmStatus::Finished(_)) => {
                        break;
                    }
                    Ok(VmStatus::Yeeted(val)) => {
                        return Err(QuoinError::Other(format!(
                            "Uncaught exception during block execution: {}",
                            val
                        )));
                    }
                    Err(QuoinError::NonLocalReturn) => {
                        if self.frames.len() > initial_frame_count {
                            continue;
                        } else if self.frames.len() == initial_frame_count {
                            break;
                        } else {
                            return Err(QuoinError::NonLocalReturn);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(self.pop()?)
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
            is_nested_block: block.is_nested_block,
            enclosing_method_id: Some(frame_id),
            block,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector: None,
            args: args.to_vec(),
            stack_base: self.stack.len(),
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });

        if self.frames.len() > initial_frame_count {
            while self.frames.len() > initial_frame_count {
                match self.step_internal(mc) {
                    Ok(VmStatus::Running) => {
                        if let Some(yielder) = unsafe { self.get_yielder() } {
                            yielder.suspend(YieldReason::CooperativeYield);
                        }
                    }
                    Ok(VmStatus::Finished(_)) => {
                        break;
                    }
                    Ok(VmStatus::Yeeted(val)) => {
                        return Err(QuoinError::Other(format!(
                            "Uncaught exception during validation block execution: {}",
                            val
                        )));
                    }
                    Err(QuoinError::NonLocalReturn) => {
                        if self.frames.len() > initial_frame_count {
                            continue;
                        } else if self.frames.len() == initial_frame_count {
                            break;
                        } else {
                            return Err(QuoinError::NonLocalReturn);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(self.pop()?)
    }

    pub fn is_subclass_of_clz(
        &self,
        sub: Gc<'gc, RefLock<Class<'gc>>>,
        sup: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> bool {
        let mut curr = Some(sub);
        while let Some(clz) = curr {
            if Gc::ptr_eq(clz, sup) {
                return true;
            }
            for mixin in &clz.borrow().mixin_classes {
                if Gc::ptr_eq(*mixin, sup) {
                    return true;
                }
            }
            curr = clz.borrow().parent;
        }
        false
    }

    pub fn is_instance_of(&self, val: Value<'gc>, class_obj: Gc<'gc, RefLock<Class<'gc>>>) -> bool {
        if let Some(val_class) = self.get_class_for_lookup(val) {
            self.is_subclass_of_clz(val_class, class_obj)
        } else {
            false
        }
    }

    pub fn append_method_to_chain(
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let mut curr = chain_start;
        loop {
            if let Value::Object(obj) = curr {
                let payload = &obj.borrow().payload;
                if let ObjectPayload::NativeState(state_cell) = payload {
                    let mut state_ref = state_cell.borrow_mut(mc);
                    let any_mut = state_ref.as_any_mut();
                    if let Some(method_state) = any_mut.downcast_mut::<NativeMethodState>() {
                        if let Some(next_val) = method_state.next {
                            let next_val_gc: Value<'gc> = unsafe { transmute(next_val) };
                            drop(state_ref);
                            curr = next_val_gc;
                            continue;
                        } else {
                            let new_method_static: Value<'static> =
                                unsafe { transmute(new_method) };
                            method_state.next = Some(new_method_static);
                            return Ok(());
                        }
                    }
                }
            }
            return Err(QuoinError::Other(
                "Invalid method object in chain".to_string(),
            ));
        }
    }

    /// Add `new_method` to a selector's method chain. A plain *unguarded* variant
    /// (no `decl_block`) whose parameter types match an existing unguarded variant
    /// *replaces* that variant's block in place — a true redefinition, so a later
    /// `-->` (or a repeated `->`) overrides instead of silently shadowing. Guarded
    /// and type-differentiated variants are appended, preserving definition order
    /// for multimethod dispatch.
    fn replace_or_append_method_in_chain(
        &self,
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let new_block = self.get_block_from_method(new_method);
        if let Some(nb) = new_block
            && nb.decl_block.is_none()
            && let Some(new_block_val) =
                new_method.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?
        {
            let new_param_types = nb.param_types.clone();
            let mut curr = Some(chain_start);
            while let Some(node) = curr {
                let is_match = self
                    .get_block_from_method(node)
                    .map(|eb| eb.decl_block.is_none() && eb.param_types == new_param_types)
                    .unwrap_or(false);
                if is_match {
                    if let Value::Object(obj) = node {
                        let obj_ref = obj.borrow();
                        if let ObjectPayload::NativeState(state_cell) = &obj_ref.payload {
                            let mut state_ref = state_cell.borrow_mut(mc);
                            if let Some(ms) =
                                state_ref.as_any_mut().downcast_mut::<NativeMethodState>()
                            {
                                ms.body =
                                    MethodBody::UserBlock(unsafe { transmute(new_block_val) });
                            }
                        }
                    }
                    return Ok(());
                }
                curr = self.get_next_method_in_chain(node);
            }
        }
        Self::append_method_to_chain(mc, chain_start, new_method)
    }

    pub fn lookup_in_class_hierarchy(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        let mut visited = Vec::new();
        self.lookup_in_class_hierarchy_rec(class_ref, selector, class_side, &mut visited)
    }

    fn lookup_in_class_hierarchy_rec(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> Option<Value<'gc>> {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return None;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        let methods = if class_side {
            &class_borrow.class_methods
        } else {
            &class_borrow.instance_methods
        };
        if let Some(method) = methods.get(selector).copied() {
            return Some(method);
        }
        for mixin in &class_borrow.mixin_classes {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(*mixin, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        if let Some(parent) = class_borrow.parent {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(parent, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        None
    }

    pub fn get_all_instance_vars(&self, class_ref: Gc<'gc, RefLock<Class<'gc>>>) -> Vec<String> {
        let mut vars = Vec::new();
        let mut visited = Vec::new();
        self.collect_instance_vars(class_ref, &mut vars, &mut visited);
        vars
    }

    fn collect_instance_vars(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        vars: &mut Vec<String>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        for var in &class_borrow.instance_vars {
            if !vars.contains(var) {
                vars.push(var.clone());
            }
        }
        for mixin in &class_borrow.mixin_classes {
            self.collect_instance_vars(*mixin, vars, visited);
        }
        if let Some(parent) = class_borrow.parent {
            self.collect_instance_vars(parent, vars, visited);
        }
    }

    pub fn push(&mut self, val: Value<'gc>) {
        self.stack.push(val);
    }

    pub fn pop(&mut self) -> Result<Value<'gc>, String> {
        self.stack
            .pop()
            .ok_or_else(|| "Stack underflow".to_string())
    }

    pub fn peek(&self) -> Result<Value<'gc>, String> {
        self.stack
            .last()
            .copied()
            .ok_or_else(|| "Stack is empty".to_string())
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
        for (sym, val) in block.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
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
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver,
            selector,
            args,
            stack_base: self.stack.len(),
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

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind self
        env_frame.bind(self_symbol(), receiver);
        // Bind parameters
        for (sym, val) in block.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.is_nested_block;
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
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector,
            args,
            stack_base: self.stack.len(),
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

        let is_nested_block = block.is_nested_block;
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
            ip: 0,
            env: env_ref,
            instantiating_obj: Some(obj),
            receiver: Some(Value::Object(obj)),
            selector,
            args: Vec::new(),
            stack_base: self.stack.len(),
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

    /// Error if `class` is `sealed!` — refuses extension (`<--` / `->` / `-->` /
    /// `.mix:`) and subclassing of a sealed class (or an instance's sealed eigenclass).
    pub(crate) fn ensure_not_sealed(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_sealed {
            return Err(QuoinError::Other(if c.is_eigenclass {
                "Cannot extend a sealed instance".to_string()
            } else {
                format!("Cannot extend sealed class {}", c.name.to_explicit_string())
            }));
        }
        Ok(())
    }

    /// Error if `class` is `abstract!` — refuses `new` / `new:` on the class itself
    /// (concrete subclasses are unaffected, since the flag isn't inherited).
    pub(crate) fn ensure_instantiable(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_abstract {
            return Err(QuoinError::Other(format!(
                "Cannot instantiate abstract class {}",
                c.name.to_explicit_string()
            )));
        }
        Ok(())
    }

    pub fn get_target_class_for_def(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        match receiver {
            Value::Class(c) => Ok(c),
            Value::ClassMeta(c) => Ok(c),
            // Extending a value type (`5 <-- {…}`, `Integer <-- {…}`) extends the
            // type itself — value types have no per-instance eigenclass.
            Value::Int(_) => Ok(self.get_or_create_builtin_class(mc, "Integer")),
            Value::Double(_) => Ok(self.get_or_create_builtin_class(mc, "Double")),
            Value::Nil => Ok(self.get_or_create_builtin_class(mc, "Nil")),
            // `true` and `false` carry distinct methods, so each gets its own
            // singleton class (parent `Boolean`), synthesized once and cached.
            Value::Bool(b) => {
                let existing = if b {
                    self.builtin_cache.borrow().true_class
                } else {
                    self.builtin_cache.borrow().false_class
                };
                if let Some(c) = existing {
                    return Ok(c);
                }
                let boolean = self.get_or_create_builtin_class(mc, "Boolean");
                let name = if b { "$TrueClass" } else { "$FalseClass" };
                let ns = NamespacedName::new(Vec::new(), name.to_string());
                let s = gcl!(
                    mc,
                    Class {
                        name: ns.clone(),
                        parent: Some(boolean),
                        instance_vars: Vec::new(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
                        mixin_classes: Vec::new(),
                        field_slots: HashMap::new(),
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
                    }
                );
                self.globals.borrow_mut(mc).insert(ns, Value::Class(s));
                if b {
                    self.builtin_cache.borrow_mut(mc).true_class = Some(s);
                } else {
                    self.builtin_cache.borrow_mut(mc).false_class = Some(s);
                }
                Ok(s)
            }
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if class_ref.borrow().name.name.starts_with('$') {
                    Ok(class_ref)
                } else {
                    let mut singleton_name = class_ref.borrow().name.clone();
                    singleton_name.name = format!("${}", singleton_name.name);
                    // The eigenclass declares no new ivars, so it shares its base
                    // class's instance layout: it must carry the same field-slot map,
                    // or `@ivar` access on the instance (now of the eigenclass) can't
                    // resolve the inherited slots and reads them as nil.
                    let field_slots = class_ref.borrow().field_slots.clone();
                    let s = gcl!(
                        mc,
                        Class {
                            name: singleton_name,
                            parent: Some(class_ref),
                            instance_vars: Vec::new(),
                            instance_methods: HashMap::new(),
                            class_methods: HashMap::new(),
                            mixin_classes: Vec::new(),
                            field_slots,
                            is_eigenclass: true,
                            is_sealed: false,
                            is_abstract: false,
                        }
                    );
                    obj.borrow_mut(mc).class = s;
                    Ok(s)
                }
            }
        }
    }

    pub fn annotate_error(&self, error: QuoinError) -> QuoinError {
        // An uncaught Quoin throw reaches here as `Thrown`; surface the actual
        // thrown value (which lives in `active_exception`) for display.
        let error = if matches!(error, QuoinError::Thrown) {
            let msg = match self.active_exception {
                Some(v) => format!("{}", v),
                None => "uncaught exception".to_string(),
            };
            QuoinError::Other(msg)
        } else {
            error
        };
        if matches!(error, QuoinError::WithSourceInfo { .. }) {
            return error;
        }
        if let Some(frame) = self.frames.last() {
            let active_ip = if frame.ip > 0 { frame.ip - 1 } else { 0 };
            let active_source_info = frame
                .block
                .source_map
                .get(active_ip)
                .and_then(|opt| opt.as_ref())
                .or(frame.block.source_info.as_ref())
                .cloned();
            if let Some(source_info) = active_source_info {
                let supports_color = self.options.supports_color;

                let colorize_selector = |sel: &str, cls: &str| -> String {
                    if supports_color {
                        format!("$#ab82ff[{}$]$#808080[:$]$#5fd7af[{}$]", sel, cls)
                    } else {
                        format!("{}:{}", sel, cls)
                    }
                };
                let colorize_simple = |sel: &str| -> String {
                    if supports_color {
                        format!("$#ab82ff[{}$]", sel)
                    } else {
                        sel.to_string()
                    }
                };

                let mut frames_info = Vec::new();
                let n = self.frames.len();
                for (i, f) in self.frames.iter().enumerate().rev() {
                    if i == n - 1 {
                        continue;
                    }
                    let frame_ip = if f.ip > 0 { f.ip - 1 } else { 0 };

                    let si_opt = f
                        .block
                        .source_map
                        .get(frame_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(f.block.source_info.as_ref())
                        .cloned();

                    let formatted_selector = if let Some(Instruction::Send(selector, num_args)) =
                        f.block.bytecode.get(frame_ip)
                    {
                        let selector = selector.as_str();
                        let args_vec = if *num_args > 0 {
                            if i == n - 1 {
                                self.last_send_args.clone()
                            } else {
                                self.frames[i + 1].args.clone()
                            }
                        } else {
                            Vec::new()
                        };

                        if !args_vec.is_empty() {
                            let mut parts = Vec::new();
                            let mut current = String::new();
                            for c in selector.chars() {
                                current.push(c);
                                if c == ':' {
                                    parts.push(current);
                                    current = String::new();
                                }
                            }
                            if !current.is_empty() {
                                parts.push(current);
                            }

                            let mut formatted_parts = Vec::new();
                            for (idx, part) in parts.iter().enumerate() {
                                if let Some(arg) = args_vec.get(idx) {
                                    let mut p = part.clone();
                                    if p.ends_with(':') {
                                        p.pop();
                                    }
                                    formatted_parts.push(colorize_selector(&p, &arg.class_name()));
                                } else {
                                    formatted_parts.push(colorize_simple(part));
                                }
                            }
                            formatted_parts.join(" ")
                        } else {
                            colorize_simple(selector)
                        }
                    } else if i == 0 {
                        colorize_simple("(top)")
                    } else {
                        let sel_str = f
                            .selector
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "value".to_string());
                        colorize_simple(&sel_str)
                    };

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " $#808080[in$] {}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "$#808080[at$]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    frames_info.push((prefix_colored, plain_len, si_opt));
                }

                // Always append the (top) frame at the bottom if it was not already the only frame formatted as (top)
                if n > 0 {
                    let first_frame = &self.frames[0];
                    let first_ip = if first_frame.ip > 0 {
                        first_frame.ip - 1
                    } else {
                        0
                    };
                    let si_opt = first_frame
                        .block
                        .source_map
                        .get(first_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(first_frame.block.source_info.as_ref())
                        .cloned();

                    let formatted_selector = colorize_simple("(top)");

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " $#808080[in$] {}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "$#808080[at$]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    // Only push if the last trace element is not already representing (top) at the same location
                    let is_dup = if let Some(last_info) = frames_info.last() {
                        last_info.0 == prefix_colored
                    } else {
                        false
                    };

                    if !is_dup {
                        frames_info.push((prefix_colored, plain_len, si_opt));
                    }
                }

                let max_l = frames_info.iter().map(|info| info.1).max().unwrap_or(0);
                let target_alignment = cmp::max(54, max_l + 2);

                let console_width = self.options.console_width.unwrap_or(80) as usize;
                let available_width = console_width.saturating_sub(target_alignment + 4);
                let show_snippet = available_width >= 15;
                let w = available_width;

                let mut trace = Vec::new();
                for (prefix_colored, plain_len, si_opt) in frames_info {
                    let mut line = if supports_color {
                        ansi_colorizer::colorize(&prefix_colored)
                    } else {
                        prefix_colored
                    };

                    if let Some(si) = si_opt {
                        if show_snippet {
                            if let Some(snippet) = self.get_highlighted_snippet(
                                &si.filename,
                                si.line.saturating_sub(1),
                                si.column,
                                si.start,
                                si.end,
                                si.source_text.as_ref(),
                                w,
                            ) {
                                let padding_len = target_alignment.saturating_sub(plain_len);
                                let padding: String = " ".repeat(padding_len);
                                let separator = if supports_color {
                                    ansi_colorizer::colorize("$#808080[<$]")
                                } else {
                                    "<".to_string()
                                };
                                line = format!("{}{}{} {}", line, padding, separator, snippet);
                            }
                        }
                    }
                    trace.push(line);
                }

                return QuoinError::WithSourceInfo {
                    error: Box::new(error),
                    source_info: source_info.clone(),
                    trace,
                    supports_color,
                };
            }
        }
        error
    }

    /// Build a Quoin `Error` instance of the named class with `message`/`payload`.
    /// Falls back to a plain string if the class isn't registered yet (e.g. an
    /// error fired during bootstrap before the Error hierarchy is defined).
    pub fn make_error(
        &self,
        mc: &Mutation<'gc>,
        class_name: &str,
        message: &str,
        payload: Option<Value<'gc>>,
    ) -> Value<'gc> {
        let key = NamespacedName::new(Vec::new(), class_name.to_string());
        let class_opt = self.globals.borrow().get(&key).copied();
        if let Some(Value::Class(cls)) = class_opt {
            let obj = self.new_object(mc, cls);
            let msg_val = self.new_string(mc, message.to_string());
            if let Some(slot) = self.field_slot(cls, "message") {
                obj.borrow_mut(mc).fields[slot] = msg_val;
            }
            if let Some(p) = payload
                && let Some(slot) = self.field_slot(cls, "payload")
            {
                obj.borrow_mut(mc).fields[slot] = p;
            }
            Value::Object(obj)
        } else {
            self.new_string(mc, message.to_string())
        }
    }

    /// Build a Quoin `IoError` carrying `message` and a `kind` symbol (e.g.
    /// `#connectionRefused`). Same bootstrap-safety as `make_error`: falls back to a
    /// plain string if the `IoError` class isn't registered yet.
    pub fn make_io_error(&self, mc: &Mutation<'gc>, kind: &str, message: &str) -> Value<'gc> {
        let key = NamespacedName::new(Vec::new(), "IoError".to_string());
        let class_opt = self.globals.borrow().get(&key).copied();
        if let Some(Value::Class(cls)) = class_opt {
            let obj = self.new_object(mc, cls);
            let msg_val = self.new_string(mc, message.to_string());
            if let Some(slot) = self.field_slot(cls, "message") {
                obj.borrow_mut(mc).fields[slot] = msg_val;
            }
            let kind_val = self.new_symbol(mc, kind.to_string());
            if let Some(slot) = self.field_slot(cls, "kind") {
                obj.borrow_mut(mc).fields[slot] = kind_val;
            }
            Value::Object(obj)
        } else {
            self.new_string(mc, message.to_string())
        }
    }

    /// Convert an internal `QuoinError` into the Quoin value a `catch:` handler should
    /// receive. Domain variants become typed `Error` objects so guest code can dispatch
    /// on them; control-flow signals and internal errors stay a descriptive string. The
    /// match is exhaustive over domain variants on purpose — a new typed error that
    /// forgets its arm here is then a compile error, not a silent fall-through to string.
    pub fn quoinerror_to_value(&self, mc: &Mutation<'gc>, error: &QuoinError) -> Value<'gc> {
        match error {
            QuoinError::TypeError { msg, .. } => self.make_error(mc, "TypeError", msg, None),
            QuoinError::ArgumentCountMismatch { msg, .. } => {
                self.make_error(mc, "ArgumentError", msg, None)
            }
            QuoinError::ArithmeticError(msg) => self.make_error(mc, "ArithmeticError", msg, None),
            QuoinError::MessageNotUnderstood {
                receiver, selector, ..
            } => {
                let msg = format!("no method '{}' for {}", selector, receiver);
                self.make_error(mc, "MessageNotUnderstood", &msg, None)
            }
            QuoinError::AmbiguousMethod { msg, .. } => {
                self.make_error(mc, "AmbiguousMethodError", msg, None)
            }
            QuoinError::Io { kind, message } => self.make_io_error(mc, kind.symbol(), message),
            QuoinError::WithSourceInfo { error, .. } => self.quoinerror_to_value(mc, error),
            QuoinError::NotCallable(_)
            | QuoinError::StackUnderflow(_)
            | QuoinError::Other(_)
            | QuoinError::Thrown
            | QuoinError::NonLocalReturn
            | QuoinError::Cancelled => {
                let s = format!("{}", error);
                self.new_string(mc, s)
            }
        }
    }

    fn get_highlighted_snippet(
        &self,
        filename: &str,
        line_idx: usize,
        column: usize,
        node_start_offset: usize,
        node_end_offset: usize,
        source_text: Option<&String>,
        w: usize,
    ) -> Option<String> {
        let supports_color = self.options.supports_color;
        let content = match fs::read_to_string(filename) {
            Ok(s) => s,
            Err(_) => {
                if let Some(text) = source_text {
                    let snippet_text = if text.chars().count() > w {
                        let sliced: String = text.chars().take(w).collect();
                        sliced
                    } else {
                        text.clone()
                    };
                    if supports_color {
                        let parse_and_highlight = || -> Option<String> {
                            let program = parse_quoin_string(&snippet_text);
                            let mut parser = HighlightParser::new(&snippet_text);
                            let spans = parser.highlight_program(&program);
                            Some(format_ansi(&snippet_text, spans))
                        };
                        if let Some(hl) = parse_and_highlight() {
                            return Some(hl);
                        }
                    }
                    return Some(snippet_text);
                }
                return None;
            }
        };

        let mut current_line = 0;
        let mut line_start_byte = 0;
        let mut line_end_byte = content.len();
        for (i, c) in content.char_indices() {
            if c == '\n' {
                if current_line == line_idx {
                    line_end_byte = i;
                    break;
                }
                current_line += 1;
                line_start_byte = i + 1;
            }
        }
        if current_line != line_idx {
            if current_line == line_idx && line_start_byte <= content.len() {
                line_end_byte = content.len();
            } else {
                return None;
            }
        }

        if line_end_byte > line_start_byte && content.as_bytes()[line_end_byte - 1] == b'\r' {
            line_end_byte -= 1;
        }

        let line_str = &content[line_start_byte..line_end_byte];
        let line_chars: Vec<(usize, char)> = line_str.char_indices().collect();
        let line_char_count = line_chars.len();

        let node_text = content
            .get(node_start_offset..node_end_offset)
            .unwrap_or("");
        let node_char_count = node_text.chars().count();

        let start_col = cmp::min(column, line_char_count);
        let end_col = cmp::min(start_col + node_char_count, line_char_count);

        let node_center = start_col + (end_col - start_col) / 2;
        let mut win_start = node_center.saturating_sub(w / 2);
        let mut win_end = win_start + w;
        if win_end > line_char_count {
            let overflow = win_end - line_char_count;
            win_start = win_start.saturating_sub(overflow);
            win_end = line_char_count;
        }

        let get_char_byte_offset = |char_idx: usize| -> usize {
            if char_idx >= line_char_count {
                line_end_byte
            } else {
                line_start_byte + line_chars[char_idx].0
            }
        };

        let win_start_byte = get_char_byte_offset(win_start);
        let win_end_byte = get_char_byte_offset(win_end);
        let snippet_text = &content[win_start_byte..win_end_byte];

        if supports_color {
            let parse_and_highlight = || -> Option<String> {
                let program = parse_quoin_string(&content);
                let mut parser = HighlightParser::new(&content);
                let spans = parser.highlight_program(&program);

                let mut snippet_spans = Vec::new();
                for span in spans {
                    let overlap_start = cmp::max(span.start, win_start_byte);
                    let overlap_end = cmp::min(span.end, win_end_byte);
                    if overlap_start < overlap_end {
                        snippet_spans.push(HighlightSpan {
                            start: overlap_start - win_start_byte,
                            end: overlap_end - win_start_byte,
                            htype: span.htype,
                            counter: span.counter,
                        });
                    }
                }
                Some(format_ansi(snippet_text, snippet_spans))
            };

            if let Some(highlighted) = parse_and_highlight() {
                return Some(highlighted);
            }
        }

        Some(snippet_text.to_string())
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
        if let Err(e) = res {
            return Err(self.annotate_error(e));
        }
        res
    }

    #[allow(no_gc_across_yield)]
    pub(crate) fn step_internal(
        &mut self,
        mc: &Mutation<'gc>,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        // Cancellation checkpoint: a pending `cancel` raises here, then clears the live
        // flag so the ensuing `finally` unwind runs to completion uninterrupted. Always
        // `false` in benchmark mode (no task table), so this is a single cheap bool load.
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
            // assert_eq!(self.stack.len(), 0, "Stack is not empty! {:?}", self.stack);
            return Ok(VmStatus::Finished(ret));
        }

        let frame_idx = self.frames.len() - 1;
        // Borrow the current instruction instead of deep-cloning it every step: clone
        // only the `Rc` to the bytecode (a refcount bump, no allocation) into a local,
        // then take a `&Instruction` into it. `inst` borrows this local `Rc`, not
        // `self`, so handlers keep full `&mut self` access; the `Rc` keeps the bytecode
        // alive even if a handler pushes/pops frames. (`Instruction` is `'static` — no
        // `Gc` pointers — so there's no GC-across-step concern.)
        let bytecode = self.frames[frame_idx].block.bytecode.clone();
        let ip = self.frames[frame_idx].ip;
        let inst = match bytecode.0.get(ip) {
            Some(i) => i,
            None => {
                // Implicit return Nil
                let ret_val = self.new_nil(mc);
                let popped = self.frames.pop().unwrap();
                self.last_popped_env = Some(popped.env);
                self.push(ret_val);
                return Ok(VmStatus::Running);
            }
        };

        match inst {
            Instruction::LoadLocal(name) => {
                let name = *name;
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, name).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::DefineLocal(name) => {
                let name = *name;
                if matches!(name.as_str(), "true" | "false" | "nil") {
                    let err_msg = format!("Can't modify reserved identifier {}", name);
                    self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let val = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                frame.env.borrow_mut(mc).bind(name, val);
                frame.ip += 1;
            }
            Instruction::StoreLocal(name) => {
                let name = *name;
                if matches!(name.as_str(), "true" | "false" | "nil") {
                    let err_msg = format!("Can't modify reserved identifier {}", name);
                    self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let val = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                // Assignments inside a `new:{}` block always bind in this frame:
                // they initialize the new object (fields and `init:` args), so they
                // must not walk up the lexical chain and mutate an enclosing
                // variable that happens to share the name. RHS reads still resolve
                // lexically (LoadLocal), so `{ x = x }` copies the outer `x`.
                if frame.instantiating_obj.is_some() {
                    frame.env.borrow_mut(mc).bind(name, val);
                } else if !EnvFrame::set(frame.env, mc, name, val) {
                    frame.env.borrow_mut(mc).bind(name, val);
                }
                frame.ip += 1;
            }
            Instruction::LoadGlobal(name) => {
                let val = self
                    .globals
                    .borrow()
                    .get(name)
                    .copied()
                    .unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreGlobal(name, is_define) => {
                let val = self.pop()?;
                if name.name == "true" || name.name == "false" || name.name == "nil" {
                    let err_msg = format!("Can't modify reserved identifier {}", name.name);
                    self.active_exception = Some(self.new_string(mc, err_msg.clone()));
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
                            self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    } else {
                        if exists {
                            let err_msg = format!(
                                "Can't modify global constant {}",
                                name.to_explicit_string()
                            );
                            self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    }
                }
                self.globals.borrow_mut(mc).insert(name.clone(), val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Push(constant) => {
                let val = match constant {
                    Constant::Nil => self.new_nil(mc),
                    Constant::Bool(b) => self.new_bool(mc, *b),
                    Constant::Int(i) => self.new_int(mc, *i),
                    Constant::Double(f) => self.new_double(mc, *f),
                    Constant::String(s) => self.new_string(mc, s.clone()),
                    Constant::Symbol(s) => self.new_symbol(mc, s.clone()),
                    Constant::Block(sb) => {
                        let parent_env = self.frames.last().map(|f| f.env);
                        let enclosing_method_id =
                            self.frames.last().and_then(|f| f.enclosing_method_id);
                        let decl_block = sb.decl_block.as_ref().map(|db| {
                            gc!(
                                mc,
                                Block {
                                    name: db.name.clone(),
                                    is_nested_block: db.is_nested_block,
                                    param_syms: db.param_syms.clone(),
                                    param_types: db.param_types.clone(),
                                    bytecode: db.bytecode.clone(),
                                    parent_env,
                                    enclosing_method_id,
                                    source_info: db.source_info.clone(),
                                    decl_block: None,
                                    source_map: db.source_map.clone(),
                                }
                            )
                        });
                        let block = Block {
                            name: sb.name.clone(),
                            is_nested_block: sb.is_nested_block,
                            param_syms: sb.param_syms.clone(),
                            param_types: sb.param_types.clone(),
                            bytecode: sb.bytecode.clone(),
                            parent_env,
                            enclosing_method_id,
                            source_info: sb.source_info.clone(),
                            decl_block,
                            source_map: sb.source_map.clone(),
                        };
                        self.new_block(mc, block)
                    }
                };
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Pop => {
                self.pop()?;
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Dup => {
                let val = self.peek()?;
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Send(selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(self.pop()?);
                }
                args.reverse();

                let receiver = self.pop()?;
                self.frames[frame_idx].ip += 1; // Advance caller frame IP

                if let Value::Object(obj) = receiver
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    if selector.as_str() == "value" || selector.as_str() == "value:" {
                        self.start_block(mc, *block, args, Some(receiver), Some(selector));
                        return Ok(VmStatus::Running);
                    }
                }

                // `last_send_args` is read only by the stack-trace formatter, and only
                // for an innermost send that fails *in place* (no callee frame of its
                // own): a failed lookup, a `MessageNotUnderstood`, or a native-method
                // error (the last captured inside `Callable::call`). On success the args
                // move into the callee frame (`Frame.args`), which the formatter reads
                // instead — so we snapshot only on these error branches, not every send.
                let method_opt = match self.lookup_method(mc, receiver, selector, &args) {
                    Ok(m) => m,
                    Err(e) => {
                        self.last_send_args = args;
                        return Err(e);
                    }
                };
                if let Some(callable) = method_opt {
                    callable.call(self, mc, Some(receiver), args, Some(selector))?;
                } else {
                    // The selector may still exist with non-matching signatures;
                    // surface those filtered-out variants as a hint.
                    let candidates = self
                        .collect_method_candidates(receiver, selector)
                        .iter()
                        .map(|&mv| self.format_candidate_signature(mv, selector))
                        .collect();
                    let receiver_name = receiver.class_name();
                    let arg_names = args.iter().map(|a| a.class_name()).collect();
                    self.last_send_args = args;
                    return Err(QuoinError::MessageNotUnderstood {
                        receiver: receiver_name,
                        selector: selector.as_str().to_string(),
                        args: arg_names,
                        candidates,
                    });
                }
            }
            Instruction::Return | Instruction::BlockReturn => {
                // Run calls deferred during this frame (e.g. mixin requirement
                // checks) *before* popping it, so the defer queue — and any Values
                // it references — stays GC-rooted via self.frames even if a defer
                // yields and a collection happens during the suspension. We iterate
                // a clone to satisfy the borrow checker; the originals stay in the
                // (still-live) frame to keep their Values reachable. Defers run only
                // on normal completion; if one throws and this is a new class
                // definition, unregister the class first.
                if !self.frames[frame_idx].defers.is_empty() {
                    let defers = self.frames[frame_idx].defers.clone();
                    if let Err(e) = self.run_defers(mc, &defers) {
                        if let Some(name) =
                            self.frames[frame_idx].unregister_on_defer_failure.clone()
                        {
                            self.globals.borrow_mut(mc).remove(&name);
                            // The class is gone; its pointer could be reused, so drop
                            // any memoized resolutions that might reference it.
                            self.invalidate_method_cache();
                        }
                        return Err(e);
                    }
                }
                let mut ret_val = self.pop()?;
                let popped_frame = self.frames.pop().unwrap();
                self.last_popped_env = Some(popped_frame.env);
                self.stack.truncate(popped_frame.stack_base);
                if let Some(obj) = popped_frame.instantiating_obj {
                    self.push(Value::Object(obj));
                    let env_borrow = popped_frame.env.borrow();
                    self.finalize_instantiation(mc, obj, &env_borrow)?;
                    ret_val = self.pop()?;
                } else if popped_frame.return_receiver {
                    if let Some(rx) = popped_frame.receiver {
                        ret_val = rx;
                    }
                }
                self.push(ret_val);
            }
            Instruction::MethodReturn => {
                let ret_val = self.pop()?;
                let enclosing_id = self.frames[frame_idx].enclosing_method_id;

                return if let Some(target_id) = enclosing_id {
                    let mut ret_val = ret_val;
                    let mut target_stack_base = None;
                    while let Some(f) = self.frames.pop() {
                        self.last_popped_env = Some(f.env);
                        if let Some(obj) = f.instantiating_obj {
                            self.push(Value::Object(obj));
                            let env_borrow = f.env.borrow();
                            self.finalize_instantiation(mc, obj, &env_borrow)?;
                            ret_val = self.pop()?;
                        } else if f.return_receiver {
                            if let Some(rx) = f.receiver {
                                ret_val = rx;
                            }
                        }
                        if f.id == target_id {
                            target_stack_base = Some(f.stack_base);
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
                let frame = &mut self.frames[frame_idx];
                frame.ip = (frame.ip as isize + offset) as usize;
            }
            Instruction::IfJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
            }
            Instruction::ElseJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if !cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
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
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewMap(n) => {
                let n = *n;
                let mut map = HashMap::new();
                for _ in 0..n {
                    let val = self.pop()?;
                    let key_val = self.pop()?;
                    if let Value::Object(obj) = key_val
                        && let ObjectPayload::String(s) = &obj.borrow().payload
                    {
                        map.insert((**s).clone(), val);
                    } else {
                        return Err(QuoinError::TypeError {
                            expected: "String".to_string(),
                            got: key_val.type_name().to_string(),
                            msg: format!("Map keys must be Strings, got: {:?}", key_val),
                        });
                    }
                }
                let map_val = self.new_map(mc, map);
                self.push(map_val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewSet(n) => {
                let n = *n;
                let mut raw = Vec::new();
                for _ in 0..n {
                    raw.push(self.pop()?);
                }
                raw.reverse();
                // Build by inserting through set_add so the literal is deduplicated
                // by `==:`, the same way `add:` enforces uniqueness at runtime.
                let set_val = self.new_set(mc, Vec::new());
                for v in raw {
                    self.set_add(mc, set_val, v)?;
                }
                self.push(set_val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::Object(obj) = pattern_val
                    && let ObjectPayload::String(s) = &obj.borrow().payload
                {
                    let re = Regex::new(&**s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = self.new_regex(mc, re);
                    self.push(regex_val);
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "String".to_string(),
                        got: pattern_val.type_name().to_string(),
                        msg: format!("Regex pattern must be a String, got: {:?}", pattern_val),
                    });
                }
                self.frames[frame_idx].ip += 1;
            }
            Instruction::DefineClass {
                name,
                parent_name,
                instance_vars,
            } => {
                let parent = if let Some(p_name) = parent_name {
                    let val = self
                        .globals
                        .borrow()
                        .get(p_name)
                        .copied()
                        .ok_or_else(|| format!("Parent class {} not found", p_name))?;
                    if let Value::Class(parent_class) = val {
                        if parent_class.borrow().is_sealed {
                            return Err(format!(
                                "Cannot subclass sealed class {}",
                                parent_class.borrow().name.to_explicit_string()
                            )
                            .into());
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

                if let Some(existing_val) = self.globals.borrow().get(name).copied() {
                    if let Value::Class(_) = existing_val {
                        return Err(format!(
                            "Cannot redefine class {} because it already exists",
                            name.to_explicit_string()
                        )
                        .into());
                    }
                }

                let class_obj = gcl!(
                    mc,
                    Class {
                        name: name.clone(),
                        parent,
                        instance_vars: instance_vars.clone(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
                        mixin_classes: Vec::new(),
                        field_slots: HashMap::new(),
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
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
                self.frames[frame_idx].ip += 1;
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
                    self.frames[frame_idx].ip += 1;
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
                        .map_err(|e| QuoinError::Other(e))?;
                    self.ensure_not_sealed(target_class)?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, false);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    if is_class_side {
                        if target_class.borrow().class_methods.contains_key(selector) {
                            let existing_val = target_class
                                .borrow()
                                .class_methods
                                .get(selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(selector.clone(), method_obj);
                        }
                    } else {
                        if target_class
                            .borrow()
                            .instance_methods
                            .contains_key(selector)
                        {
                            let existing_val = target_class
                                .borrow()
                                .instance_methods
                                .get(selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(selector.clone(), method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions.
                    self.invalidate_method_cache();
                    self.push(method_obj);
                    self.frames[frame_idx].ip += 1;
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
                        .map_err(|e| QuoinError::Other(e))?;
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

                    if is_class_side {
                        if target_class.borrow().class_methods.contains_key(selector) {
                            let existing_val = target_class
                                .borrow()
                                .class_methods
                                .get(selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(selector.clone(), method_obj);
                        }
                    } else {
                        if target_class
                            .borrow()
                            .instance_methods
                            .contains_key(selector)
                        {
                            let existing_val = target_class
                                .borrow()
                                .instance_methods
                                .get(selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(selector.clone(), method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions.
                    self.invalidate_method_cache();
                    self.push(method_obj);
                    self.frames[frame_idx].ip += 1;
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("OverrideMethod expects a Block, got {:?}", block_val),
                    });
                }
            }

            Instruction::LoadField(name) => {
                let frame = &self.frames[frame_idx];
                let self_val =
                    EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
                let val = if let Value::Object(obj) = self_val {
                    let class = obj.borrow().class;
                    // No slot (undeclared) or a slot past this instance's array
                    // (declared on the class after this object was created) => nil.
                    self.field_slot(class, name)
                        .and_then(|slot| obj.borrow().fields.get(slot).copied())
                        .unwrap_or_else(|| self.new_nil(mc))
                } else {
                    self.new_nil(mc)
                };
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreField(name) => {
                let val = self.pop()?;
                let frame = &self.frames[frame_idx];
                let self_val =
                    EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
                if let Value::Object(obj) = self_val {
                    let class = obj.borrow().class;
                    match self.field_slot(class, &name) {
                        Some(slot) if slot < obj.borrow().fields.len() => {
                            obj.borrow_mut(mc).fields[slot] = val;
                        }
                        Some(_) => {
                            // Declared on the class, but this instance predates it
                            // (a mixin added the ivar after the object was created);
                            // an object's shape is fixed at construction.
                            return Err(QuoinError::Other(format!(
                                "Instance of '{}' has no '@{}' (it was added after this instance was created)",
                                class.borrow().name,
                                name
                            )));
                        }
                        None => {
                            // You cannot create an instance variable by assigning to
                            // it — it must be declared in the class.
                            return Err(QuoinError::Other(format!(
                                "No instance variable '@{}' declared on '{}'",
                                name,
                                class.borrow().name
                            )));
                        }
                    }
                } else {
                    // Immediate value types (Integer/Double/Boolean/Nil) have no
                    // per-instance fields — setting `@x` on one is an error.
                    return Err(QuoinError::Other(format!(
                        "Cannot set instance variable '@{}' on a value type ({})",
                        name,
                        self_val.type_name()
                    )));
                }
                self.frames[frame_idx].ip += 1;
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
                self.frames[frame_idx].ip += 1;
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

        Ok(VmStatus::Running)
    }
}

#[cfg(test)]
#[path = "vm_tests.rs"]
mod tests;
