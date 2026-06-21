use crate::error::QuoinError;
use crate::fiber::{Fiber, VMYielder, YieldReason};
use crate::highlighter::{format_ansi, HighlightParser, HighlightSpan};
use crate::instruction::{Constant, Instruction};
use crate::parser::parse_quoin_string;
use crate::runtime::fiber::{FiberStatus, NativeFiberState};
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::method::{MethodBody, NativeMethodState};
use crate::runtime::regex::NativeRegexState;
use crate::runtime::set::NativeSetState;
use crate::symbol::{self_symbol, Symbol};
use crate::value::{
    AnyCollect, Block, Class, EnvFrame, NamespacedName, NativeCall, NativeClass, NativeFunc, Object,
    ObjectPayload, Value,
};
use crate::{ansi_colorizer, gc, gcl};

use gc_arena::{lock::RefLock, Collect, Gc, Mutation};
use regex::Regex;
use std::collections::HashMap;
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
    pub last_send_receiver: Option<Value<'gc>>,
    pub last_send_args: Vec<Value<'gc>>,
    pub active_native_args: Vec<NativeCall<'gc>>,

    /// Yielder of the *currently running* coroutine. Set by the driver from the
    /// running fiber's stored slot before every resume, so it can never dangle.
    #[collect(require_static)]
    pub yielder: Option<*const ()>,
    /// The main program coroutine's yielder (the per-fiber slot for fiber #0).
    #[collect(require_static)]
    pub main_yielder: Option<*const ()>,
    pub active_fiber: Option<Gc<'gc, Fiber<'gc>>>,
    pub last_popped_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,

    // --- Guest fiber scheduler state ---
    /// The guest `Fiber` currently executing, or `None` when the main program
    /// (fiber #0) is running. The scheduler in the driver keeps this in sync.
    pub current_fiber: Option<Value<'gc>>,
    /// Chain of resumers: each entry is whoever resumed the fiber above it
    /// (`None` == the main program). A `yield` pops this to find who to return to.
    pub resume_stack: Vec<Option<Value<'gc>>>,
    /// One-slot mailbox for the value handed across a fiber switch (the arg to
    /// `resume:`, or the value out of `yield:`). Written by the scheduler, read
    /// by the resumed coroutine.
    pub fiber_transfer: Option<Value<'gc>>,
    /// Saved execution context for the main program while a guest fiber runs.
    pub main_saved_stack: Vec<Value<'gc>>,
    pub main_saved_frames: Vec<Frame<'gc>>,
    pub main_saved_native_args: Vec<NativeCall<'gc>>,
    /// An error raised inside a guest fiber, delivered to its resumer.
    #[collect(require_static)]
    pub fiber_error: Option<QuoinError>,

    /// Memoized method resolution: `(searched-class ptr, selector, class-side,
    /// arg-class ptrs)` → resolved method (or `None` when the hierarchy has no
    /// match). Populated by `lookup_method_in_class_hierarchy` for guard-free,
    /// non-eigenclass lookups; cleared whenever a class's method table changes
    /// (`invalidate_method_cache`). Traced as part of `VmState` so cached method
    /// `Value`s stay live; the key's class *pointers* are sound because named
    /// classes are globals-rooted (stable address) — eigenclasses are excluded.
    pub method_cache: HashMap<MethodCacheKey, Option<Value<'gc>>>,
    /// Scratch flag marking the in-progress lookup's result as un-memoizable. Set
    /// by `match_score` when a guarded candidate is examined — a guard's outcome
    /// depends on argument *values*, not just types, so the resolution can't be keyed
    /// on classes alone. Saved/restored around each `lookup_method_in_class_hierarchy`
    /// call for re-entrancy safety (a guard's nested sends run their own lookups
    /// without corrupting this one).
    #[collect(require_static)]
    pub dispatch_uncacheable: bool,

    #[collect(require_static)]
    pub options: VmOptions,
}

pub enum VmStatus<'gc> {
    Running,
    Finished(Value<'gc>),
    Yeeted(Value<'gc>), // Uncaught exception
}

/// A resolved, ready-to-invoke method. A `Copy` enum rather than a boxed trait
/// object, so dispatch resolves and invokes a method without a per-Send heap
/// allocation — each variant carries only `Copy` data (`Gc` handles / a native fn
/// pointer). The callable is transient — built by `lookup_method` and consumed by
/// `call` within a single `Send` step — so it is never stored in a traced struct
/// and needs no `Collect` impl (matching the `Box<dyn Callable>` it replaced).
#[derive(Copy, Clone)]
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
                if args.len() != 1 {
                    return Err(QuoinError::Other("new: expects a block".to_string()));
                }
                let block = if let Value::Object(obj) = args[0]
                    && let ObjectPayload::Block(b) = &obj.borrow().payload
                {
                    *b
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: args[0].type_name().to_string(),
                        msg: "new: expects a Block".to_string(),
                    });
                };

                // Create the new object
                let obj = vm.new_object(mc, class_obj);

                vm.start_block_for_instantiation(mc, block, obj, selector);
                Ok(())
            }
            Callable::NewNoBlock(class_obj) => {
                if !args.is_empty() {
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
                // can never desync.
                vm.active_native_args.push(NativeCall {
                    receiver,
                    args: args.clone(),
                });
                let ret = func.0(vm, mc, receiver, args);
                vm.active_native_args.pop();
                let ret = ret?;
                vm.push(ret);
                Ok(())
            }
        }
    }
}

impl<'gc> VmState<'gc> {
    pub unsafe fn get_yielder(&self) -> Option<&VMYielder<'gc>> {
        self.yielder
            .map(|ptr| unsafe { &*(ptr as *const VMYielder<'gc>) })
    }

    /// Record the running coroutine's yielder into the current fiber's slot (or
    /// the main slot) and make it live. Called once at the top of `run_vm_loop`.
    pub fn register_yielder(&mut self, mc: &Mutation<'gc>, ptr: *const ()) {
        match self.current_fiber {
            None => self.main_yielder = Some(ptr),
            Some(f) => {
                let _ =
                    f.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.set_yielder(ptr));
            }
        }
        self.yielder = Some(ptr);
    }

    /// The stored yielder for whichever fiber is current (main if `None`). The
    /// driver loads this into `self.yielder` before resuming, guaranteeing it
    /// always points at the live, GC-rooted coroutine being run.
    pub fn current_fiber_yielder(&self) -> Option<*const ()> {
        match self.current_fiber {
            None => self.main_yielder,
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
            last_send_receiver: None,
            last_send_args: Vec::new(),
            active_native_args: Vec::new(),
            yielder: None,
            main_yielder: None,
            active_fiber: None,
            last_popped_env: None,
            current_fiber: None,
            resume_stack: Vec::new(),
            fiber_transfer: None,
            main_saved_stack: Vec::new(),
            main_saved_frames: Vec::new(),
            main_saved_native_args: Vec::new(),
            fiber_error: None,
            method_cache: HashMap::new(),
            dispatch_uncacheable: false,
            options,
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
    pub fn to_s(&mut self, mc: &Mutation<'gc>, value: Value<'gc>) -> Result<Value<'gc>, QuoinError> {
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
            ObjectPayload::Block(b) => Some(b.param_syms.iter().map(|s| s.as_str().to_string()).collect()),
            ObjectPayload::NativeState(state_cell) => {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                let method_state = any_ref.downcast_ref::<NativeMethodState>()?;
                if let Some(Value::Object(block_obj)) = method_state.get_block()
                    && let ObjectPayload::Block(b) = &block_obj.borrow().payload
                {
                    Some(b.param_syms.iter().map(|s| s.as_str().to_string()).collect())
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
            method.call(self, mc, Some(receiver), args, Some(Symbol::intern(selector)))?;

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

    // =====================================================================
    // Guest fiber support
    //
    // `fiber_resume` / `fiber_yield` run inside native `Fiber` methods (deep in
    // `step`). They bubble a `YieldReason` up to the scheduler in the driver,
    // which performs the actual context switch via the `*_switch` helpers below
    // and re-enters the appropriate coroutine. The transfer value rides in the
    // GC-rooted `fiber_transfer` slot, so nothing is held only on the suspended
    // native stack across the yield.
    // =====================================================================

    /// Resume `fiber_val`, delivering `arg`. Returns the value the fiber yields,
    /// or its final return value when it completes. Called from `f.resume[:]`.
    #[allow(no_gc_across_yield)]
    pub fn fiber_resume(
        &mut self,
        mc: &Mutation<'gc>,
        fiber_val: Value<'gc>,
        arg: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        match self.fiber_status(fiber_val)? {
            FiberStatus::Done => {
                return Err(self.raise_fiber_error(mc, "cannot resume a finished Fiber"));
            }
            FiberStatus::Failed => {
                return Err(self.raise_fiber_error(mc, "cannot resume a failed Fiber"));
            }
            _ => {}
        }
        // The only Running fiber is the current one; ancestors are Suspended.
        if self.current_fiber == Some(fiber_val) {
            return Err(self.raise_fiber_error(mc, "a Fiber cannot resume itself"));
        }
        if self.resume_stack.iter().any(|f| *f == Some(fiber_val)) {
            return Err(self.raise_fiber_error(
                mc,
                "cannot resume a Fiber that is currently resuming this one (would deadlock)",
            ));
        }

        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::ResumeFiber {
                fiber: fiber_val,
                arg,
            });
        } else {
            return Err(QuoinError::Other(
                "Fiber.resume called outside the VM scheduler".to_string(),
            ));
        }
        // On resume the driver has already restored `self.yielder` for us.

        if let Some(err) = self.fiber_error.take() {
            return Err(err);
        }
        Ok(self
            .fiber_transfer
            .take()
            .unwrap_or_else(|| self.new_nil(mc)))
    }

    /// Suspend the running fiber, handing `value` to whoever resumed it. Returns
    /// the value passed to the next `resume:`. Called from `Fiber.yield[:]`.
    #[allow(no_gc_across_yield)]
    pub fn fiber_yield(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        if self.current_fiber.is_none() {
            return Err(self.raise_fiber_error(mc, "Fiber.yield: called outside of a Fiber"));
        }

        if let Some(yielder) = unsafe { self.get_yielder() } {
            yielder.suspend(YieldReason::YieldFiber { value });
        } else {
            return Err(QuoinError::Other(
                "Fiber.yield: called outside the VM scheduler".to_string(),
            ));
        }
        // On resume the driver has already restored `self.yielder` for us.

        if let Some(err) = self.fiber_error.take() {
            return Err(err);
        }
        Ok(self
            .fiber_transfer
            .take()
            .unwrap_or_else(|| self.new_nil(mc)))
    }

    fn fiber_status(&self, fiber_val: Value<'gc>) -> Result<FiberStatus, QuoinError> {
        fiber_val
            .with_native_state::<NativeFiberState, _, _>(|s| s.status)
            .map_err(QuoinError::Other)
    }

    /// Park a structured `FiberError` in `active_exception` and return the
    /// `Thrown` signal, so fiber misuse is catchable by type in Quoin code.
    fn raise_fiber_error(&mut self, mc: &Mutation<'gc>, msg: &str) -> QuoinError {
        let err = self.make_error(mc, "FiberError", msg, None);
        self.active_exception = Some(err);
        QuoinError::Thrown
    }

    fn set_fiber_status(&self, mc: &Mutation<'gc>, fiber_val: Value<'gc>, status: FiberStatus) {
        let _ =
            fiber_val.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.status = status);
    }

    /// Save the live VM execution context into the slot for `who` (`None` = main).
    fn save_fiber_context(
        &mut self,
        mc: &Mutation<'gc>,
        who: Option<Value<'gc>>,
    ) -> Result<(), QuoinError> {
        let stack = std::mem::take(&mut self.stack);
        let frames = std::mem::take(&mut self.frames);
        let native_args = std::mem::take(&mut self.active_native_args);
        match who {
            None => {
                self.main_saved_stack = stack;
                self.main_saved_frames = frames;
                self.main_saved_native_args = native_args;
            }
            Some(f) => {
                f.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| {
                    s.set_context(stack, frames, native_args)
                })
                .map_err(QuoinError::Other)?;
            }
        }
        Ok(())
    }

    /// Load the saved context for `who` (`None` = main) into the live VM fields.
    fn load_fiber_context(
        &mut self,
        mc: &Mutation<'gc>,
        who: Option<Value<'gc>>,
    ) -> Result<(), QuoinError> {
        let (stack, frames, native_args) = match who {
            None => (
                std::mem::take(&mut self.main_saved_stack),
                std::mem::take(&mut self.main_saved_frames),
                std::mem::take(&mut self.main_saved_native_args),
            ),
            Some(f) => f
                .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.take_context())
                .map_err(QuoinError::Other)?,
        };
        self.stack = stack;
        self.frames = frames;
        self.active_native_args = native_args;
        Ok(())
    }

    /// Scheduler: switch from the running coroutine to `fiber_val`, delivering
    /// `arg`. Pushes the caller onto the resume stack.
    pub fn do_resume_switch(
        &mut self,
        mc: &Mutation<'gc>,
        fiber_val: Value<'gc>,
        arg: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let outgoing = self.current_fiber;
        self.save_fiber_context(mc, outgoing)?;
        if let Some(of) = outgoing {
            self.set_fiber_status(mc, of, FiberStatus::Suspended);
        }
        self.resume_stack.push(outgoing);
        self.current_fiber = Some(fiber_val);

        let started = fiber_val
            .with_native_state::<NativeFiberState, _, _>(|s| s.started)
            .map_err(QuoinError::Other)?;

        self.load_fiber_context(mc, Some(fiber_val))?;

        if started {
            self.fiber_transfer = Some(arg);
        } else {
            // First activation: bind `arg` to the block's parameters.
            let block_val = fiber_val
                .with_native_state::<NativeFiberState, _, _>(|s| s.block())
                .map_err(QuoinError::Other)?;
            let block_gc = match block_val {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Block(b) => *b,
                    _ => return Err(QuoinError::Other("Fiber target is not a Block".to_string())),
                },
                _ => return Err(QuoinError::Other("Fiber target is not a Block".to_string())),
            };
            self.start_block(mc, block_gc, vec![arg], None, None);
            fiber_val
                .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.started = true)
                .map_err(QuoinError::Other)?;
        }
        self.set_fiber_status(mc, fiber_val, FiberStatus::Running);
        Ok(())
    }

    /// Scheduler: the running fiber yielded `value`; return control to its resumer.
    pub fn do_yield_switch(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let outgoing = self.current_fiber;
        self.save_fiber_context(mc, outgoing)?;
        if let Some(of) = outgoing {
            self.set_fiber_status(mc, of, FiberStatus::Suspended);
        }
        let resumer = self.resume_stack.pop().unwrap_or(None);
        self.current_fiber = resumer;
        self.load_fiber_context(mc, resumer)?;
        if let Some(rf) = resumer {
            self.set_fiber_status(mc, rf, FiberStatus::Running);
        }
        self.fiber_transfer = Some(value);
        Ok(())
    }

    /// Scheduler: the running fiber's block returned (or errored); mark it done
    /// and return control to its resumer with the result.
    pub fn do_fiber_done(
        &mut self,
        mc: &Mutation<'gc>,
        result: Result<Value<'gc>, QuoinError>,
    ) -> Result<(), QuoinError> {
        // Record the outcome on the finished fiber for `result`/`error`/`status`.
        if let Some(finished) = self.current_fiber {
            match &result {
                Ok(val) => {
                    let v = *val;
                    self.set_fiber_status(mc, finished, FiberStatus::Done);
                    let _ = finished
                        .with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.set_result(v));
                }
                Err(e) => {
                    // The error value is the parked Quoin exception, or a converted
                    // internal error. Peek (don't take) so the resumer still sees it.
                    let err_val = match self.active_exception {
                        Some(v) => v,
                        None => self.quoinerror_to_value(mc, e),
                    };
                    self.set_fiber_status(mc, finished, FiberStatus::Failed);
                    let _ = finished.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| {
                        s.set_error(err_val)
                    });
                }
            }
        }
        // The finished fiber's execution context is discarded.
        self.stack.clear();
        self.frames.clear();
        self.active_native_args.clear();

        let resumer = self.resume_stack.pop().unwrap_or(None);
        self.current_fiber = resumer;
        self.load_fiber_context(mc, resumer)?;
        if let Some(rf) = resumer {
            self.set_fiber_status(mc, rf, FiberStatus::Running);
        }
        match result {
            Ok(val) => self.fiber_transfer = Some(val),
            Err(err) => self.fiber_error = Some(err),
        }
        Ok(())
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

    #[allow(no_gc_across_yield)]
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
        let selector_key = NamespacedName::new(Vec::new(), selector.as_str().to_string());
        let method_val = match receiver {
            Value::Class(class_obj) => {
                if let Some(m) =
                    self.lookup_method_in_class_hierarchy(mc, class_obj, receiver, selector, true, args)?
                {
                    Some(m)
                } else {
                    let class_key = NamespacedName::new(Vec::new(), "Class".to_string());
                    if let Some(Value::Class(class_class)) =
                        self.globals.borrow().get(&class_key).copied()
                    {
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
                            self.globals.borrow().get(&selector_key).copied()
                        }
                    } else {
                        self.globals.borrow().get(&selector_key).copied()
                    }
                }
            }
            Value::ClassMeta(class_obj) => {
                if let Some(m) =
                    self.lookup_method_in_class_hierarchy(mc, class_obj, receiver, selector, true, args)?
                {
                    Some(m)
                } else {
                    // A metaclass acts as if it subclasses Object: fall through to
                    // Object's instance methods so it responds to the universal
                    // protocol (can?:, s, ==:, …). We use Object rather than the
                    // "Class" class because Class methods (new, name, …) assume a
                    // real Class receiver.
                    let object_key = NamespacedName::new(Vec::new(), "Object".to_string());
                    if let Some(Value::Class(object_class)) =
                        self.globals.borrow().get(&object_key).copied()
                    {
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
                            self.globals.borrow().get(&selector_key).copied()
                        }
                    } else {
                        self.globals.borrow().get(&selector_key).copied()
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
                        self.globals.borrow().get(&selector_key).copied()
                    }
                } else {
                    self.globals.borrow().get(&selector_key).copied()
                }
            }
        };

        let method_val = match method_val {
            Some(v) => v,
            None => return Ok(None),
        };

        match method_val {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Block(block) => Ok(Some(Callable::Block(*block))),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                        if let Some(func) = method_state.native_func() {
                            Ok(Some(Callable::Native(func)))
                        } else if let Some(Value::Object(block_obj)) = method_state.get_block()
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Ok(Some(Callable::Block(*block)))
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

    #[allow(no_gc_across_yield)]
    /// Drop every memoized method resolution. Called whenever a class's method
    /// table changes (method def/override, native registration, class unregister),
    /// which only happens at class-definition time — never inside a hot loop — so
    /// the cache stays warm during execution. Clearing wholesale (rather than
    /// per-class) is correct because a new/overridden method can shadow cached
    /// resolutions in *derived* classes too.
    pub fn invalidate_method_cache(&mut self) {
        self.method_cache.clear();
    }

    /// Build the cache key for a lookup, or `None` if the lookup must not be cached:
    /// an eigenclass is involved (transient pointer → unsafe to key on), an argument
    /// has no dispatch class, or there are more arguments than the key encodes.
    fn method_cache_key(
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
            if let Some(cached) = self.method_cache.get(&k) {
                return Ok(*cached);
            }
        }

        // Miss: walk, tracking whether any guarded candidate was examined. Save and
        // restore `dispatch_uncacheable` so nested sends fired *by* a guard (which run
        // their own lookups) can't corrupt this lookup's cacheability accounting.
        let saved_uncacheable = self.dispatch_uncacheable;
        self.dispatch_uncacheable = false;
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
        let uncacheable = self.dispatch_uncacheable;
        self.dispatch_uncacheable = saved_uncacheable;

        // Cache only a successful, guard-free resolution (errors and guarded
        // dispatches stay uncached — the latter can depend on argument values).
        if !uncacheable {
            if let (Some(k), Ok(resolved)) = (key, &result) {
                self.method_cache.insert(k, *resolved);
            }
        }
        result
    }

    // `receiver` is the subject of the send (the value `self` resolves to inside a
    // guard block); it's threaded down to `match_score`/`execute_validation_block`.
    #[allow(no_gc_across_yield)]
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
        let method_chain_start = methods.get(selector.as_str()).copied();
        let mixins = class_borrow.mixin_classes.clone();
        let parent = class_borrow.parent;
        drop(class_borrow);

        if let Some(chain_start) = method_chain_start {
            let mut candidates = Vec::new();
            let mut curr = Some(chain_start);
            while let Some(method_val) = curr {
                candidates.push(method_val);
                curr = self.get_next_method_in_chain(method_val);
            }

            // Score every applicable candidate; the lowest `(Σ type_distance,
            // guarded?)` wins. Two distinct candidates sharing the lowest score are
            // equally specific with no tiebreaker -> `AmbiguousMethodError`. A guarded
            // and an unguarded variant never share a score (the guard rank separates
            // them), so the specific-guards-then-unguarded-catch-all idiom stays
            // unambiguous. A signatureless native scores i64::MAX and is a pure
            // fallback, exempt from ambiguity. The hierarchy walk below still lets a
            // derived class override a base regardless of score.
            let mut applicable: Vec<(Value<'gc>, (i64, u8))> = Vec::new();
            for &method_val in &candidates {
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

        for mixin in mixins {
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
                    Some(param_types) => {
                        self.score_param_types(&param_types, args).map(|s| (s, 1))
                    }
                    None => Some((i64::MAX, 1)),
                });
            }
        };
        let param_score = match self.score_param_types(&block.param_types, args) {
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
            self.dispatch_uncacheable = true;
            let res =
                self.execute_validation_block(mc, decl_block, receiver, &block.param_syms, args)?;
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
    fn collect_method_candidates(&self, receiver: Value<'gc>, selector: Symbol) -> Vec<Value<'gc>> {
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
        let chain_start = methods.get(selector.as_str()).copied();
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
    fn format_candidate_signature(&self, method_val: Value<'gc>, selector: Symbol) -> String {
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
    fn candidate_param_types(&self, method_val: Value<'gc>) -> Vec<String> {
        if let Some(block) = self.get_block_from_method(method_val) {
            block.param_types.clone()
        } else {
            self.native_method_param_types(method_val).unwrap_or_default()
        }
    }

    /// A guarded variant's guard for display: its syntax-highlighted source (e.g.
    /// `{x > 5}`), or a colorized `{...}` placeholder when source text is absent.
    /// `None` for an unguarded variant.
    fn candidate_guard_display(&self, method_val: Value<'gc>, supports_color: bool) -> Option<String> {
        let block = self.get_block_from_method(method_val)?;
        let decl = block.decl_block?;
        // `source_info.source_text` is the node's own text (the guard span), so it
        // already holds the guard source — no slicing needed.
        let src = decl
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
    fn format_selector_with_types(selector: &str, types: &[String], supports_color: bool) -> String {
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
        args: &[Value<'gc>],
    ) -> Option<i64> {
        if args.len() < param_types.len() {
            return None;
        }
        let mut score: i64 = 0;
        for (i, hint) in param_types.iter().enumerate() {
            score += self.type_distance(args[i], hint)?;
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

    /// Class-hierarchy distance from `val`'s class to the class named `hint` (0 if
    /// `val` is directly of that type), or `None` if `val` isn't an instance of it.
    /// A mixin counts as one hop from the class that mixes it in.
    fn type_distance(&self, val: Value<'gc>, hint: &str) -> Option<i64> {
        // Fast path / exact match. Also the only thing that matches a `Class` or
        // `ClassMeta` value (whose `get_class_for_lookup` returns the class itself,
        // not a class named "Class") — mirrors matches_type's `type_name == hint`.
        if val.type_name() == hint {
            return Some(0);
        }
        let val_class = self.get_class_for_lookup(val)?;
        // Resolve the hint to a class so we can match by identity; fall back to
        // matching by name when it isn't a known global (mirrors matches_type).
        let target = match self
            .globals
            .borrow()
            .get(&NamespacedName::new(Vec::new(), hint.to_string()))
            .copied()
        {
            Some(Value::Class(c)) => Some(c),
            _ => None,
        };
        let matches = |clz: Gc<'gc, RefLock<Class<'gc>>>| match target {
            Some(t) => Gc::ptr_eq(clz, t),
            None => clz.borrow().name.name == hint,
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

    fn get_block_from_method(&self, method_val: Value<'gc>) -> Option<Gc<'gc, Block<'gc>>> {
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

    fn get_next_method_in_chain(&self, method_val: Value<'gc>) -> Option<Value<'gc>> {
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
            return Err(QuoinError::Other("Invalid method object in chain".to_string()));
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
            match self.globals.borrow().get(&NamespacedName::parse(name)).copied() {
                Some(Value::Class(c)) => Some(c),
                _ => None,
            }
        })
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

    /// Convert an internal `QuoinError` into the Quoin value a `catch:` handler should
    /// receive. Structured variants become typed `Error` objects so guest code
    /// can dispatch on them; everything else stays a descriptive string.
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
            QuoinError::WithSourceInfo { error, .. } => self.quoinerror_to_value(mc, error),
            other => {
                let s = format!("{}", other);
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
        if let Err(e) = res {
            return Err(self.annotate_error(e));
        }
        res
    }

    #[allow(no_gc_across_yield)]
    pub(crate) fn step_internal(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, QuoinError> {
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
            // assert_eq!(self.stack.len(), 0, "Stack is not empty! {:?}", self.stack);
            return Ok(VmStatus::Finished(ret));
        }

        let frame_idx = self.frames.len() - 1;
        let inst = {
            let frame = &self.frames[frame_idx];
            frame.block.bytecode.get(frame.ip).cloned()
        };

        let inst = match inst {
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
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, name).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::DefineLocal(name) => {
                if matches!(name.as_str(), "true" | "false" | "nil") {
                    let err_msg = format!("Can't modify keyword {}", name);
                    self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let val = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                frame.env.borrow_mut(mc).bind(name, val);
                frame.ip += 1;
            }
            Instruction::StoreLocal(name) => {
                if matches!(name.as_str(), "true" | "false" | "nil") {
                    let err_msg = format!("Can't modify keyword {}", name);
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
                    .get(&name)
                    .copied()
                    .unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::StoreGlobal(name, is_define) => {
                let val = self.pop()?;
                if name.name == "true" || name.name == "false" || name.name == "nil" {
                    let err_msg = format!("Can't modify keyword {}", name.name);
                    self.active_exception = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let first_char = name.name.chars().next().unwrap_or('\0');
                if first_char.is_ascii_uppercase() {
                    let exists = self.globals.borrow().contains_key(&name);
                    if is_define {
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
                self.globals.borrow_mut(mc).insert(name, val);
                self.frames[frame_idx].ip += 1;
            }
            Instruction::Push(constant) => {
                let val = match constant {
                    Constant::Nil => self.new_nil(mc),
                    Constant::Bool(b) => self.new_bool(mc, b),
                    Constant::Int(i) => self.new_int(mc, i),
                    Constant::Double(f) => self.new_double(mc, f),
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
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(self.pop()?);
                }
                args.reverse();

                let receiver = self.pop()?;
                self.last_send_receiver = Some(receiver);
                self.last_send_args = args.clone();
                self.frames[frame_idx].ip += 1; // Advance caller frame IP

                if let Value::Object(obj) = receiver
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    if selector.as_str() == "value" || selector.as_str() == "value:" {
                        self.start_block(mc, *block, args, Some(receiver), Some(selector));
                        return Ok(VmStatus::Running);
                    }
                }

                let method_opt = self.lookup_method(mc, receiver, selector, &args)?;
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
                    return Err(QuoinError::MessageNotUnderstood {
                        receiver: receiver.class_name(),
                        selector: selector.as_str().to_string(),
                        args: args.iter().map(|a| a.class_name()).collect(),
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
                let frame = &mut self.frames[frame_idx];
                frame.ip = (frame.ip as isize + offset) as usize;
            }
            Instruction::IfJump(offset) => {
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
            }
            Instruction::ElseJump(offset) => {
                let cond = self.pop()?;
                let frame = &mut self.frames[frame_idx];
                if !cond.is_truthy() {
                    frame.ip = (frame.ip as isize + offset) as usize;
                } else {
                    frame.ip += 1;
                }
            }
            Instruction::NewList(n) => {
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
                let parent = if let Some(p_name) = &parent_name {
                    let val = self
                        .globals
                        .borrow()
                        .get(p_name)
                        .copied()
                        .ok_or_else(|| format!("Parent class {} not found", p_name))?;
                    if let Value::Class(parent_class) = val {
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

                if let Some(existing_val) = self.globals.borrow().get(&name).copied() {
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

                    let method_obj = self.new_method(mc, selector.clone(), block_val, false);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    if is_class_side {
                        if target_class.borrow().class_methods.contains_key(&selector) {
                            let existing_val = target_class
                                .borrow()
                                .class_methods
                                .get(&selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(selector, method_obj);
                        }
                    } else {
                        if target_class
                            .borrow()
                            .instance_methods
                            .contains_key(&selector)
                        {
                            let existing_val = target_class
                                .borrow()
                                .instance_methods
                                .get(&selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(selector, method_obj);
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

                    let method_obj = self.new_method(mc, selector.clone(), block_val, true);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    let exists = self
                        .lookup_in_class_hierarchy(target_class, &selector, is_class_side)
                        .is_some();
                    if !exists {
                        return Err(QuoinError::Other(format!(
                            "Method {} does not exist in hierarchy of Class {} to override",
                            selector,
                            target_class.borrow().name
                        )));
                    }

                    if is_class_side {
                        if target_class.borrow().class_methods.contains_key(&selector) {
                            let existing_val = target_class
                                .borrow()
                                .class_methods
                                .get(&selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(selector, method_obj);
                        }
                    } else {
                        if target_class
                            .borrow()
                            .instance_methods
                            .contains_key(&selector)
                        {
                            let existing_val = target_class
                                .borrow()
                                .instance_methods
                                .get(&selector)
                                .copied()
                                .unwrap();
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(selector, method_obj);
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
                let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
                let val = if let Value::Object(obj) = self_val {
                    let class = obj.borrow().class;
                    // No slot (undeclared) or a slot past this instance's array
                    // (declared on the class after this object was created) => nil.
                    self.field_slot(class, &name)
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
                let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
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
        }

        Ok(VmStatus::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::{Constant, SharedBytecode, SharedSourceMap, StaticBlock};
    use crate::parser::ast::NodeValue;
    use crate::runtime::block::build_block_class;
    use crate::runtime::boolean::build_boolean_class;
    use crate::runtime::class::build_class_class;
    use crate::runtime::double::build_double_class;
    use crate::runtime::integer::build_integer_class;
    use crate::runtime::list::build_list_class;
    use crate::runtime::map::{build_key_value_pair_class, build_map_class};
    use crate::runtime::nil::build_nil_class;
    use crate::runtime::object::build_object_class;
    use crate::runtime::regex::build_regex_class;
    use crate::runtime::string::build_string_class;
    use crate::value::{NativeClassBuilder, OpaqueState};
    use gc_arena::{Arena, Rootable};

    fn native_add<'gc>(
        vm: &mut VmState<'gc>,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let a = receiver
            .as_i64()
            .ok_or_else(|| QuoinError::Other("Invalid types".to_string()))?;
        let b = args[0]
            .as_i64()
            .ok_or_else(|| QuoinError::Other("Invalid types".to_string()))?;
        Ok(vm.new_int(mc, a + b))
    }

    #[derive(Debug, PartialEq, Clone)]
    enum ValueSpec {
        Nil,
        Bool(bool),
        Int(i64),
        Double(f64),
        String(String),
        Symbol(String),
        Class(String),
        ClassMeta(String),
        List(Vec<ValueSpec>),
        Map(HashMap<String, ValueSpec>),
        Regex(String),
        Block(Option<String>),
        Instance(String),
    }

    fn to_spec(val: Value<'_>) -> ValueSpec {
        match val {
            Value::Int(i) => ValueSpec::Int(i),
            Value::Double(d) => ValueSpec::Double(d),
            Value::Bool(b) => ValueSpec::Bool(b),
            Value::Nil => ValueSpec::Nil,
            Value::Class(c) => ValueSpec::Class(c.borrow().name.to_string()),
            Value::ClassMeta(c) => ValueSpec::ClassMeta(c.borrow().name.to_string()),
            Value::Object(obj) => {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::String(s) => ValueSpec::String((**s).clone()),
                    ObjectPayload::Symbol(s) => ValueSpec::Symbol((**s).clone()),
                    _ if borrowed.class_name() == "List" => {
                        let res = val.with_native_state::<NativeListState, _, _>(|l| {
                            let list_specs = l.get_vec().iter().map(|&v| to_spec(v)).collect();
                            ValueSpec::List(list_specs)
                        });
                        res.unwrap_or_else(|_| ValueSpec::Instance("List".to_string()))
                    }
                    _ if borrowed.class_name() == "Map" => {
                        let res = val.with_native_state::<NativeMapState, _, _>(|m| {
                            let map_specs = m
                                .get_map()
                                .iter()
                                .map(|(k, &v)| (k.clone(), to_spec(v)))
                                .collect();
                            ValueSpec::Map(map_specs)
                        });
                        res.unwrap_or_else(|_| ValueSpec::Instance("Map".to_string()))
                    }
                    _ if borrowed.class_name() == "Regex" => {
                        let res = val.with_native_state::<NativeRegexState, _, _>(|r| {
                            ValueSpec::Regex(r.regex.as_str().to_string())
                        });
                        res.unwrap_or_else(|_| ValueSpec::Instance("Regex".to_string()))
                    }
                    ObjectPayload::Block(b) => ValueSpec::Block(b.name.clone()),
                    ObjectPayload::Instance | ObjectPayload::NativeState(_) => {
                        ValueSpec::Instance(borrowed.class.borrow().name.to_string())
                    }
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Clone)]
    enum VmStatusSpec {
        Running,
        Finished(ValueSpec),
        Yeeted(ValueSpec),
    }

    fn to_status_spec(status: VmStatus<'_>) -> VmStatusSpec {
        match status {
            VmStatus::Running => VmStatusSpec::Running,
            VmStatus::Finished(val) => VmStatusSpec::Finished(to_spec(val)),
            VmStatus::Yeeted(val) => VmStatusSpec::Yeeted(to_spec(val)),
        }
    }

    fn stack_spec(vm: &VmState<'_>) -> Vec<ValueSpec> {
        vm.stack.iter().copied().map(to_spec).collect()
    }

    fn run_test_steps<F>(instructions: Vec<Instruction>, check_steps: F)
    where
        F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>),
    {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());

            // Register standard classes first, so that they exist when new_xxx helper methods are called.
            vm.register_native_class(mc, build_object_class());
            vm.register_native_class(mc, build_class_class());
            vm.register_native_class(mc, build_boolean_class());
            vm.register_native_class(mc, build_block_class());
            vm.register_native_class(mc, build_list_class());
            vm.register_native_class(mc, build_double_class());
            vm.register_native_class(mc, build_integer_class());
            vm.register_native_class(mc, build_string_class());
            vm.register_native_class(mc, build_nil_class());
            vm.register_native_class(mc, build_map_class());
            vm.register_native_class(mc, build_key_value_pair_class());
            vm.register_native_class(mc, build_regex_class());

            for t in ["Method"] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }

            // Register standard native functions we might need
            let native_val = vm.new_native_method(mc, "+".to_string(), NativeFunc(native_add), None);
            vm.globals
                .borrow_mut(mc)
                .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

            let static_block = StaticBlock {
                source_info: None,
                name: Some("test_main".to_string()),
                is_nested_block: false,
                param_syms: Vec::new(),
                param_types: Vec::new(),
                bytecode: instructions.into(),
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            };
            let block = gc!(
                mc,
                Block {
                    source_info: None,
                    name: static_block.name.clone(),
                    is_nested_block: static_block.is_nested_block,
                    param_syms: static_block.param_syms.clone(),
                    param_types: static_block.param_types.clone(),
                    bytecode: static_block.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                }
            );
            vm.start_block(mc, block, Vec::new(), None, None);
            vm
        });

        arena.mutate_root(|mc, vm| {
            check_steps(vm, mc);
        });
    }

    #[test]
    fn test_push_pop_dup() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(10)),
                Instruction::Push(Constant::Int(20)),
                Instruction::Pop,
                Instruction::Dup,
            ],
            |vm, mc| {
                // Initial: Stack = []
                assert_eq!(vm.stack.len(), 0);

                // Step 1: Push(10)
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

                // Step 2: Push(20)
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(20)]);

                // Step 3: Pop
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

                // Step 4: Dup
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(10)]);

                // Step 5: Implicit return Nil
                let status = vm.step(mc).unwrap();
                assert_eq!(to_status_spec(status), VmStatusSpec::Running);
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(10), ValueSpec::Int(10), ValueSpec::Nil]
                );

                // Pop the remaining values left on stack for testing to satisfy the stack-empty assertion
                vm.pop().unwrap(); // Nil
                vm.pop().unwrap(); // 10
                vm.pop().unwrap(); // 10
                let nil_val = vm.new_nil(mc);
                vm.push(nil_val); // Push it back as the return value

                // Step 6: Finished
                let status = vm.step(mc).unwrap();
                assert_eq!(
                    to_status_spec(status),
                    VmStatusSpec::Finished(ValueSpec::Nil)
                );
            },
        );
    }

    #[test]
    fn test_symbol_interning_pointer_equality() {
        // Pull the inner interned string out of a symbol value.
        fn inner(v: Value) -> Gc<String> {
            match v {
                Value::Object(obj) => match obj.borrow().payload {
                    ObjectPayload::Symbol(s) => s,
                    _ => panic!("expected a Symbol payload"),
                },
                _ => panic!("expected an Object value"),
            }
        }

        run_test_steps(Vec::new(), |vm, mc| {
            let a = vm.new_symbol(mc, "foo".to_string());
            let b = vm.new_symbol(mc, "foo".to_string());
            let c = vm.new_symbol(mc, "bar".to_string());

            // Same name => the inner Gc<String> is pointer-identical (real interning,
            // not the `id`/content fallbacks in Value::eq).
            assert!(
                Gc::ptr_eq(inner(a), inner(b)),
                "interned symbols of the same name must share the inner Gc<String>"
            );
            // ...and the whole canonical Object is shared too.
            match (a, b) {
                (Value::Object(oa), Value::Object(ob)) => assert!(
                    Gc::ptr_eq(oa, ob),
                    "interned symbols of the same name must be the same Object"
                ),
                _ => panic!("symbols must be Object values"),
            }
            // Different names => distinct pointers.
            assert!(
                !Gc::ptr_eq(inner(a), inner(c)),
                "symbols of different names must not share a pointer"
            );

            // Sanity: Quoin-level equality agrees with identity.
            assert_eq!(a, b);
            assert_ne!(a, c);
        });
    }

    #[test]
    fn test_deferred_call_values_survive_collection() {
        // A `DeferredCall` holds GC `Value`s (receiver + args) in `Frame.defers`.
        // They must be traced so a collection between when a defer is enqueued (e.g.
        // by `mix:`) and when it runs (the Return handler) does not free them. The
        // run loop collects between steps, so this really can happen.
        // (`pending_class_def` / `unregister_on_defer_failure` hold only a 'static
        // `NamespacedName` — no GC pointers — so they need no such guard.)
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, build_object_class());
            vm.register_native_class(mc, build_string_class());

            // Start a frame the defer can attach to (mirrors run_test_steps).
            let static_block = StaticBlock {
                source_info: None,
                name: Some("defer_gc_test".to_string()),
                is_nested_block: false,
                param_syms: Vec::new(),
                param_types: Vec::new(),
                bytecode: Vec::<Instruction>::new().into(),
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            };
            let block = gc!(
                mc,
                Block {
                    source_info: None,
                    name: static_block.name.clone(),
                    is_nested_block: static_block.is_nested_block,
                    param_syms: static_block.param_syms.clone(),
                    param_types: static_block.param_types.clone(),
                    bytecode: static_block.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                }
            );
            vm.start_block(mc, block, Vec::new(), None, None);
            vm
        });

        // Enqueue a deferred call whose receiver and args are freshly-allocated
        // strings reachable ONLY through the defer.
        arena.mutate_root(|mc, vm| {
            let receiver = vm.new_string(mc, "DEFER-RECEIVER".to_string());
            let arg = vm.new_string(mc, "DEFER-ARG".to_string());
            let frame = vm.frames.last_mut().expect("a frame to hold the defer");
            frame.defers.push(DeferredCall {
                receiver,
                selector: "check:".to_string(),
                args: vec![arg],
            });
        });

        // Allocate a pile of unreachable garbage so the collector has real work to
        // sweep, then drive it through full cycles. If `Frame.defers` weren't traced,
        // the deferred strings would be swept right alongside this garbage.
        arena.mutate_root(|mc, vm| {
            for i in 0..512 {
                let _garbage = vm.new_string(mc, format!("garbage-{i}"));
            }
        });
        arena.finish_cycle();
        arena.finish_cycle();

        // After collection the deferred Values must still be the exact strings.
        arena.mutate_root(|_mc, vm| {
            let frame = vm.frames.last().expect("frame still present");
            assert_eq!(frame.defers.len(), 1, "the defer must survive collection");
            let d = &frame.defers[0];
            assert_eq!(d.selector, "check:");
            for (val, expected) in [(d.receiver, "DEFER-RECEIVER"), (d.args[0], "DEFER-ARG")] {
                match val {
                    Value::Object(obj) => match &obj.borrow().payload {
                        ObjectPayload::String(s) => assert_eq!(s.as_str(), expected),
                        _ => panic!("deferred value is no longer a String — collected?"),
                    },
                    _ => panic!("deferred value is not an Object — collected?"),
                }
            }
        });
    }

    #[test]
    fn test_native_methods_are_chainable() {
        // Native methods are `Method` chain nodes (`NativeState` wrapping a native
        // body), so another variant can be appended onto a native method's chain —
        // which previously errored "Invalid method object in chain" (overriding e.g.
        // List#count). (Bare native-function objects no longer exist at all.)
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, build_object_class());
            vm
        });

        arena.mutate_root(|mc, vm| {
            let obj_class = vm.get_or_create_builtin_class(mc, "Object");
            let native_method = obj_class
                .borrow()
                .instance_methods
                .get("can?:")
                .copied()
                .expect("Object should have a native can?: method");

            // A native method is a chainable NativeState node.
            match native_method {
                Value::Object(o) => assert!(
                    matches!(&o.borrow().payload, ObjectPayload::NativeState(_)),
                    "native method should be a chainable NativeState node"
                ),
                _ => panic!("native method should be an object"),
            }

            // Appending another variant onto it succeeds (previously crashed).
            let appended = vm.new_native_method(
                mc,
                "can?:".to_string(),
                NativeFunc(|vm, mc, _receiver, _args| Ok(vm.new_nil(mc))),
                None,
            );
            VmState::append_method_to_chain(mc, native_method, appended)
                .expect("appending onto a native method's chain should succeed");
            assert!(
                vm.get_next_method_in_chain(native_method).is_some(),
                "the native method should now chain to the appended variant"
            );
        });
    }

    #[test]
    fn test_typed_native_method_dispatches_by_type() {
        // Phase 2b: native methods can carry a type signature, so several typed
        // native variants of one selector are routed by argument type, exactly like
        // user multimethod variants.
        use crate::value::NativeClassBuilder;
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, build_object_class());
            vm.register_native_class(mc, build_integer_class());
            vm.register_native_class(mc, build_string_class());
            // Two typed class-side `kind:` variants on one selector.
            let builder = NativeClassBuilder::new("ScoreTest", Some("Object"))
                .typed_class_method("kind:", &["Integer"], |vm, mc, _receiver, _args| {
                    Ok(vm.new_string(mc, "int".to_string()))
                })
                .typed_class_method("kind:", &["String"], |vm, mc, _receiver, _args| {
                    Ok(vm.new_string(mc, "str".to_string()))
                });
            vm.register_native_class(mc, builder);
            vm
        });

        arena.mutate_root(|mc, vm| {
            let test_class = vm.get_or_create_builtin_class(mc, "ScoreTest");
            let recv = Value::Class(test_class);

            let check = |v: Value<'_>, expected: &str| match v {
                Value::Object(o) => match &o.borrow().payload {
                    ObjectPayload::String(s) => assert_eq!(s.as_str(), expected),
                    _ => panic!("expected a String result, got a different payload"),
                },
                _ => panic!("expected a String result"),
            };

            let int_arg = vm.new_int(mc, 5);
            check(
                vm.call_method(mc, recv, "kind:", vec![int_arg]).unwrap(),
                "int",
            );

            let str_arg = vm.new_string(mc, "hi".to_string());
            check(
                vm.call_method(mc, recv, "kind:", vec![str_arg]).unwrap(),
                "str",
            );
        });
    }

    #[test]
    fn test_local_variables() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(42)),
                Instruction::DefineLocal(Symbol::intern("a")),
                Instruction::LoadLocal(Symbol::intern("a")),
                Instruction::Push(Constant::Int(100)),
                Instruction::StoreLocal(Symbol::intern("a")),
                Instruction::LoadLocal(Symbol::intern("a")),
            ],
            |vm, mc| {
                // Step 1: Push(42) -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 2: DefineLocal("a") -> []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 4: Push(100) -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Int(100)]
                );

                // Step 5: StoreLocal("a") -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Step 6: LoadLocal("a") -> [Int(42), Int(100)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Int(100)]
                );
            },
        );
    }

    #[test]
    fn test_global_variables() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(77)),
                Instruction::StoreGlobal(
                    NamespacedName::new(Vec::new(), "g_var".to_string()),
                    false,
                ),
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "g_var".to_string())),
            ],
            |vm, mc| {
                // Step 1: Push(77)
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);

                // Step 2: StoreGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);

                // Step 3: LoadGlobal("g_var")
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);
            },
        );
    }

    #[test]
    fn test_constants() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Nil),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Push(Constant::Double(3.14)),
                Instruction::Push(Constant::String("hello".to_string())),
            ],
            |vm, mc| {
                // Nil
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Nil]);

                // Bool
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Nil, ValueSpec::Bool(true)]);

                // Float
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![
                        ValueSpec::Nil,
                        ValueSpec::Bool(true),
                        ValueSpec::Double(3.14)
                    ]
                );

                // String
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm).len(), 4);
                assert_eq!(stack_spec(vm)[3], ValueSpec::String("hello".to_string()));
            },
        );
    }

    #[test]
    fn test_jump_if_else() {
        run_test_steps(
            vec![
                // 0: Push true
                Instruction::Push(Constant::Bool(true)),
                // 1: IfJump to 4 (offset +3 -> 4)
                Instruction::IfJump(3),
                // 2: Push 99 (should be skipped)
                Instruction::Push(Constant::Int(99)),
                // 3: Jump to 5 (offset +2 -> 5)
                Instruction::Jump(2),
                // 4: Push 42 (target of IfJump)
                Instruction::Push(Constant::Int(42)),
                // 5: Push false
                Instruction::Push(Constant::Bool(false)),
                // 6: ElseJump to 9 (offset +3 -> 9)
                Instruction::ElseJump(3),
                // 7: Push 88 (should be skipped)
                Instruction::Push(Constant::Int(88)),
                // 8: Jump to 10 (offset +2 -> 10)
                Instruction::Jump(2),
                // 9: Push 55 (target of ElseJump)
                Instruction::Push(Constant::Int(55)),
            ],
            |vm, mc| {
                // Push true -> [Bool(true)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);

                // IfJump(3) -> condition true -> jump to index 4 (Push 42). Stack becomes []
                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 0);
                assert_eq!(vm.frames[0].ip, 4);

                // Push 42 -> [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Push false -> [Int(42), Bool(false)]
                vm.step(mc).unwrap();
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::Int(42), ValueSpec::Bool(false)]
                );

                // ElseJump(3) -> condition false -> jump to index 9 (Push 55). Stack becomes [Int(42)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
                assert_eq!(vm.frames[0].ip, 9);

                // Push 55 -> [Int(42), Int(55)]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42), ValueSpec::Int(55)]);
            },
        );
    }

    #[test]
    fn test_list_map_regex() {
        run_test_steps(
            vec![
                // List of 2 elements: Push 1, Push 2, NewList(2)
                Instruction::Push(Constant::Int(1)),
                Instruction::Push(Constant::Int(2)),
                Instruction::NewList(2),
                // Map of 1 pair: Push key "a", Push val 10, NewMap(1)
                Instruction::Push(Constant::String("a".to_string())),
                Instruction::Push(Constant::Int(10)),
                Instruction::NewMap(1),
                // Regex: Push pattern "^ab$", NewRegex
                Instruction::Push(Constant::String("^ab$".to_string())),
                Instruction::NewRegex,
            ],
            |vm, mc| {
                // List creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewList(2)
                assert_eq!(vm.stack.len(), 1);
                assert_eq!(
                    stack_spec(vm),
                    vec![ValueSpec::List(vec![ValueSpec::Int(1), ValueSpec::Int(2)])]
                );

                // Map creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewMap(1)
                assert_eq!(vm.stack.len(), 2);
                let mut expected_map = HashMap::new();
                expected_map.insert("a".to_string(), ValueSpec::Int(10));
                assert_eq!(stack_spec(vm)[1], ValueSpec::Map(expected_map));

                // Regex creation
                vm.step(mc).unwrap();
                vm.step(mc).unwrap(); // NewRegex
                assert_eq!(vm.stack.len(), 3);
                assert_eq!(stack_spec(vm)[2], ValueSpec::Regex("^ab$".to_string()));
            },
        );
    }

    #[test]
    fn test_send_message() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Int(5)),
                Instruction::Push(Constant::Int(10)),
                // Send "+" with 1 argument (selector: "+", receiver: Int(5), arg: Int(10))
                Instruction::Send(Symbol::intern("+"), 1),
            ],
            |vm, mc| {
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                // Send -> receiver 5 + argument 10 -> returns Int(15)
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(15)]);
            },
        );
    }

    #[test]
    fn test_block_execution_and_returns() {
        // We will push a block constant, then send "value" to it.
        // The block bytecode will load its parameter, add 1 to it, and return.
        let block_static = StaticBlock {
            source_info: None,
            name: Some("test_block".to_string()),
            is_nested_block: false,
            param_syms: crate::value::intern_param_syms(&vec!["x".to_string()]),
            param_types: vec!["Object".to_string()],
            bytecode: SharedBytecode::from(vec![
                Instruction::LoadLocal(Symbol::intern("x")),
                Instruction::Push(Constant::Int(1)),
                Instruction::Send(Symbol::intern("+"), 1),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Block(block_static)),
                Instruction::Push(Constant::Int(41)),
                // Send "value:" with 1 arg
                Instruction::Send(Symbol::intern("value:"), 1),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push block -> [Block]
                vm.step(mc).unwrap(); // Push 41 -> [Block, Int(41)]
                assert_eq!(vm.frames.len(), 1);

                // Send -> starts block frame -> [Block]
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(vm.frames[1].block.name, Some("test_block".to_string()));

                // Inside block: LoadLocal("x") -> push 41 -> [41]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41)]);

                // Inside block: Push(1) -> [41, 1]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41), ValueSpec::Int(1)]);

                // Inside block: Send("+", 1) -> [42]
                vm.step(mc).unwrap();
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

                // Inside block: Return -> pops block frame, leaves return value on stack -> [42]
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );
    }

    #[test]
    fn test_yeet_exception() {
        run_test_steps(
            vec![Instruction::Push(Constant::Int(500)), Instruction::Yeet],
            |vm, mc| {
                vm.step(mc).unwrap();
                let status = vm.step(mc).unwrap();
                assert_eq!(
                    to_status_spec(status),
                    VmStatusSpec::Yeeted(ValueSpec::Int(500))
                );
                assert_eq!(vm.frames.len(), 0);
            },
        );
    }

    #[test]
    fn test_method_return() {
        // Block 1 is the method context.
        // Block 2 is a nested block context.
        // If Block 2 executes MethodReturn, it should unwind all frames up to and including the method context (Block 1).

        // Block 2: nested block
        // Bytecode: Push(999), MethodReturn
        let block_nested = StaticBlock {
            source_info: None,
            name: Some("nested".to_string()),
            is_nested_block: true,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::Push(Constant::Int(999)),
                Instruction::MethodReturn,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        // Block 1: method
        // Bytecode: Push(Block(nested)), Send("value", 0), Push(100), Return
        let block_method = StaticBlock {
            source_info: None,
            name: Some("method".to_string()),
            is_nested_block: false, // enclosing_method_id will be this frame's ID
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::Push(Constant::Block(block_nested)),
                Instruction::Send(Symbol::intern("value"), 0),
                Instruction::Push(Constant::Int(100)), // this should be skipped due to MethodReturn
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Block(block_method)),
                Instruction::Send(Symbol::intern("value"), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push block_method
                vm.step(mc).unwrap(); // Send "value" -> starts block_method frame (frame id = 2, enclosing_method_id = 2)
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(vm.frames[1].enclosing_method_id, Some(vm.frames[1].id));

                vm.step(mc).unwrap(); // Inside block_method: Push(block_nested)
                vm.step(mc).unwrap(); // Inside block_method: Send("value", 0) -> starts block_nested frame (frame id = 3, enclosing_method_id = 2)
                assert_eq!(vm.frames.len(), 3);
                assert_eq!(vm.frames[2].enclosing_method_id, Some(vm.frames[1].id));

                vm.step(mc).unwrap(); // Inside block_nested: Push(999) -> Stack has [999]
                // Inside block_nested: MethodReturn.
                // It should pop frame 3 (nested) and frame 2 (method), leaving only the main frame (frame 1),
                // and pushing 999 to the stack.
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(999)]);
            },
        );
    }

    #[test]
    fn test_non_local_return_callback() {
        // block_nested: Push(777), MethodReturn
        let block_nested = StaticBlock {
            source_info: None,
            name: Some("nested".to_string()),
            is_nested_block: true,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::Push(Constant::Int(777)),
                Instruction::MethodReturn,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        // block_bar: blk.value, Push(111), Return
        let block_bar = StaticBlock {
            source_info: None,
            name: Some("bar".to_string()),
            is_nested_block: false,
            param_syms: crate::value::intern_param_syms(&vec!["blk".to_string()]),
            param_types: vec!["Object".to_string()],
            bytecode: SharedBytecode::from(vec![
                Instruction::LoadLocal(Symbol::intern("blk")),
                Instruction::Send(Symbol::intern("value"), 0),
                Instruction::Push(Constant::Int(111)),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        // block_foo: bar.value: block_nested, Push(222), Return
        let block_foo = StaticBlock {
            source_info: None,
            name: Some("foo".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "bar_func".to_string())),
                Instruction::Push(Constant::Block(block_nested)),
                Instruction::Send(Symbol::intern("value:"), 1),
                Instruction::Push(Constant::Int(222)),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            let bar_block = Block {
                source_info: None,
                name: block_bar.name.clone(),
                is_nested_block: block_bar.is_nested_block,
                param_syms: block_bar.param_syms.clone(),
                param_types: block_bar.param_types.clone(),
                bytecode: block_bar.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            };
            let bar_block_val = vm.new_block(mc, bar_block);
            vm.globals.borrow_mut(mc).insert(
                NamespacedName::new(Vec::new(), "bar_func".to_string()),
                bar_block_val,
            );

            let foo_block = gc!(
                mc,
                Block {
                    source_info: None,
                    name: block_foo.name.clone(),
                    is_nested_block: block_foo.is_nested_block,
                    param_syms: block_foo.param_syms.clone(),
                    param_types: block_foo.param_types.clone(),
                    bytecode: block_foo.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                }
            );
            vm.start_block(mc, foo_block, Vec::new(), None, None);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Step 1: Inside foo: LoadGlobal(bar_func)
            vm.step(mc).unwrap();
            // Step 2: Inside foo: Push(block_nested)
            vm.step(mc).unwrap();
            // Step 3: Inside foo: Send(value:) -> starts block_bar frame
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);
            assert_eq!(vm.frames[1].block.name, Some("bar".to_string()));

            // Step 4: Inside bar: LoadLocal(blk)
            vm.step(mc).unwrap();
            // Step 5: Inside bar: Send(value) -> starts block_nested frame
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 3);
            assert_eq!(vm.frames[2].block.name, Some("nested".to_string()));

            // Step 6: Inside nested: Push(777)
            vm.step(mc).unwrap();
            // Step 7: Inside nested: MethodReturn -> unwinds nested, bar, and foo frames!
            vm.step(mc).unwrap();

            // All frames should be unwound.
            assert_eq!(vm.frames.len(), 0);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(777)]);
        });
    }

    #[test]
    fn test_class_and_method_definition_vm() {
        let class_block = StaticBlock {
            source_info: None,
            name: Some("class_block".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                // 1. Define inst method x
                Instruction::Push(Constant::Block(StaticBlock {
                    source_info: None,
                    name: Some("x".to_string()),
                    is_nested_block: false,
                    param_syms: Vec::new(),
                    param_types: Vec::new(),
                    bytecode: vec![
                        Instruction::LoadLocal(Symbol::intern("self")),
                        Instruction::Return,
                    ]
                    .into(),
                    decl_block: None,
                    source_map: Vec::new().into(),
                })),
                Instruction::DefineMethod("x".to_string()),
                // 2. Override inst method x
                Instruction::Push(Constant::Block(StaticBlock {
                    source_info: None,
                    name: Some("x".to_string()),
                    is_nested_block: false,
                    param_syms: Vec::new(),
                    param_types: Vec::new(),
                    bytecode: vec![Instruction::Push(Constant::Int(42)), Instruction::Return]
                        .into(),
                    decl_block: None,
                    source_map: Vec::new().into(),
                })),
                Instruction::OverrideMethod("x".to_string()),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        run_test_steps(
            vec![
                // Define class Point
                Instruction::DefineClass {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent_name: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                },
                // Push class block
                Instruction::Push(Constant::Block(class_block)),
                // Execute block with Point as self
                Instruction::ExecuteBlockWithSelf,
                // Send "meta" to Point
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send(Symbol::intern("meta"), 0),
            ],
            |vm, mc| {
                // Step DefineClass
                vm.step(mc).unwrap();
                let class_val = vm.peek().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Point");
                    assert_eq!(
                        c.borrow().instance_vars,
                        vec!["x".to_string(), "y".to_string()]
                    );
                } else {
                    panic!("Expected Class value");
                }

                // Step Push Block
                vm.step(mc).unwrap();
                // Step ExecuteBlockWithSelf -> frame for class_block starts
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                assert_eq!(EnvFrame::get(vm.frames[1].env, self_symbol()).unwrap(), class_val);

                // Inside class_block: Push(x_block)
                vm.step(mc).unwrap();
                // Inside class_block: DefineMethod("x")
                vm.step(mc).unwrap();

                // Verify method x exists in instance_methods
                if let Value::Class(c) = class_val {
                    assert!(c.borrow().instance_methods.contains_key("x"));
                }

                // Inside class_block: Push(override_x_block)
                vm.step(mc).unwrap();
                // Inside class_block: OverrideMethod("x")
                vm.step(mc).unwrap();

                // Inside class_block: Return -> pops class_block frame, pushes Nil to main stack
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);

                // Step LoadGlobal Point
                vm.step(mc).unwrap();
                // Step Send meta
                vm.step(mc).unwrap();

                // Stack should have [Point, Nil, ClassMeta(Point)]
                let meta_val = vm.peek().unwrap();
                if let Value::ClassMeta(c) = meta_val {
                    assert_eq!(c.borrow().name.to_string(), "Point");
                } else {
                    panic!("Expected ClassMeta, got {:?}", meta_val);
                }
            },
        );
    }

    #[test]
    fn test_class_method_lookup_fallback() {
        run_test_steps(
            vec![
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send(Symbol::intern("name"), 0),
            ],
            |vm, mc| {
                let point_class = gcl!(
                    mc,
                    Class {
                        name: NamespacedName::new(Vec::new(), "Point".to_string()),
                        parent: None,
                        instance_vars: Vec::new(),
                        instance_methods: HashMap::new(),
                        class_methods: HashMap::new(),
                        mixin_classes: Vec::new(),
                        field_slots: HashMap::new(),
                        is_eigenclass: false,
                    }
                );
                vm.globals.borrow_mut(mc).insert(
                    NamespacedName::new(Vec::new(), "Point".to_string()),
                    Value::Class(point_class),
                );

                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 1);

                vm.step(mc).unwrap();
                assert_eq!(vm.stack.len(), 1);

                assert_eq!(stack_spec(vm), vec![ValueSpec::String("Point".to_string())]);
            },
        );
    }

    #[test]
    fn test_primitive_methods_and_overrides() {
        let custom_true_method = StaticBlock {
            source_info: None,
            name: Some("custom_true_method".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::Push(Constant::Int(42)),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        let class_extension_block = StaticBlock {
            source_info: None,
            name: Some("class_extension_block".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: SharedBytecode::from(vec![
                Instruction::Push(Constant::Block(custom_true_method)),
                Instruction::DefineMethod("custom_true".to_string()),
                Instruction::Push(Constant::Nil),
                Instruction::Return,
            ]),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };

        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send(Symbol::intern("class"), 0),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Push(Constant::Block(class_extension_block)),
                Instruction::ExecuteBlockWithSelf,
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send(Symbol::intern("class"), 0),
                Instruction::Push(Constant::Bool(true)),
                Instruction::Send(Symbol::intern("custom_true"), 0),
                Instruction::Push(Constant::Bool(false)),
                Instruction::Send(Symbol::intern("class"), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Boolean");
                } else {
                    panic!("Expected Class Boolean, got {:?}", class_val);
                }

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Bool(true));

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "$TrueClass");
                } else {
                    panic!("Expected Class $TrueClass, got {:?}", class_val);
                }

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 2);
                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                assert_eq!(vm.frames.len(), 1);
                assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Int(42));

                vm.step(mc).unwrap();
                vm.step(mc).unwrap();
                let class_val = vm.pop().unwrap();
                if let Value::Class(c) = class_val {
                    assert_eq!(c.borrow().name.to_string(), "Boolean");
                } else {
                    panic!("Expected Class Boolean, got {:?}", class_val);
                }
            },
        );
    }

    #[test]
    fn test_class_new() {
        run_test_steps(
            vec![
                Instruction::DefineClass {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent_name: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                },
                Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
                Instruction::Send(Symbol::intern("new"), 0),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // DefineClass
                vm.step(mc).unwrap(); // LoadGlobal
                vm.step(mc).unwrap(); // Send "new"
                let obj_val = vm.pop().unwrap();
                if let Value::Object(obj) = obj_val {
                    assert_eq!(obj.borrow().class.borrow().name.to_string(), "Point");
                    let ob = obj.borrow();
                    assert_eq!(ob.fields.len(), 2); // @x, @y
                    assert!(ob.fields.iter().all(|v| matches!(to_spec(*v), ValueSpec::Nil)));
                } else {
                    panic!("Expected Object, got {:?}", obj_val);
                }
            },
        );
    }

    #[test]
    fn test_namespaced_native_class() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());

            // Build a namespaced native class [IO]File
            let file_builder = NativeClassBuilder::new("[IO]File", Some("Object"))
                .instance_method("path", |vm, mc, _receiver, _args| {
                    Ok(vm.new_string(mc, "/etc/passwd".to_string()))
                });
            vm.register_native_class(mc, file_builder);
            vm
        });

        arena.mutate_root(|_mc, vm| {
            // Verify [IO]File class is in globals
            let file_key = NamespacedName::new(vec!["IO".to_string()], "File".to_string());
            let val = vm.globals.borrow().get(&file_key).copied().unwrap();
            if let Value::Class(c) = val {
                assert_eq!(c.borrow().name.to_string(), "[IO]File");
                assert_eq!(c.borrow().name.path, vec!["IO".to_string()]);
                assert_eq!(c.borrow().name.name, "File");
            } else {
                panic!("Expected Class, got {:?}", val);
            }
        });
    }

    #[derive(Debug)]
    struct MyCustomResource {
        counter: i32,
    }

    #[test]
    fn test_native_state_holding_rust_state() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());

            // Build native class [IO]Resource
            let resource_builder = NativeClassBuilder::new("[IO]Resource", Some("Object"))
                .class_method("create", |vm, mc, _receiver, _args| {
                    let class_obj = vm.get_builtin_class("[IO]Resource");
                    let state = OpaqueState(MyCustomResource { counter: 10 });
                    Ok(vm.new_native_state(mc, class_obj, state))
                })
                .instance_method("get", |vm, mc, receiver, _args| {
                    let val = receiver
                        .with_native_state::<MyCustomResource, _, _>(|res| res.counter)
                        .unwrap();
                    Ok(vm.new_int(mc, val as i64))
                })
                .instance_method("inc:", |_vm, mc, receiver, args| {
                    let val = args[0].as_i64().expect("Expected Int") as i32;
                    receiver
                        .with_native_state_mut::<MyCustomResource, _, _>(mc, |res| {
                            res.counter += val;
                        })
                        .unwrap();
                    Ok(receiver)
                });
            vm.register_native_class(mc, resource_builder);
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Instantiate [IO]Resource via sending "create"
            let resource_class = vm.get_builtin_class("[IO]Resource");
            let instance = vm
                .call_method(mc, Value::Class(resource_class), "create", vec![])
                .unwrap();

            // Check counter is 10
            let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
            assert_eq!(to_spec(counter_val), ValueSpec::Int(10));

            // Increment by 5
            let five = vm.new_int(mc, 5);
            vm.call_method(mc, instance, "inc:", vec![five]).unwrap();

            // Check counter is 15
            let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
            assert_eq!(to_spec(counter_val), ValueSpec::Int(15));
        });
    }

    #[test]
    fn test_mixin_method_lookup_and_instance_vars() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let vm = VmState::new(mc, VmOptions::default());
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Define mixin class Point
            let point_class = gcl!(
                mc,
                Class {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent: None,
                    instance_vars: vec!["x".to_string(), "y".to_string()],
                    instance_methods: {
                        let mut m = HashMap::new();
                        m.insert(
                            "name".to_string(),
                            vm.new_native_method(
                                mc,
                                "name".to_string(),
                                NativeFunc::new(|vm, mc, _receiver, _args| {
                                    Ok(vm.new_string(mc, "Point".to_string()))
                                }),
                                None,
                            ),
                        );
                        m
                    },
                    class_methods: HashMap::new(),
                    mixin_classes: Vec::new(),
                    field_slots: HashMap::new(),
                    is_eigenclass: false,
                }
            );

            // Define class PType which mixes in Point
            let ptype_class = gcl!(
                mc,
                Class {
                    name: NamespacedName::new(Vec::new(), "PType".to_string()),
                    parent: None,
                    instance_vars: vec!["z".to_string()],
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                    mixin_classes: vec![point_class],
                    field_slots: HashMap::new(),
                    is_eigenclass: false,
                }
            );

            // Check instance variables (should contain z, x, y)
            let vars = vm.get_all_instance_vars(ptype_class);
            assert!(vars.contains(&"x".to_string()));
            assert!(vars.contains(&"y".to_string()));
            assert!(vars.contains(&"z".to_string()));

            // Instantiate PType
            let obj = vm.new_object(mc, ptype_class);

            // Look up "name" on PType instance -> should find Point's name method
            let _method = vm
                .lookup_method(mc, Value::Object(obj), Symbol::intern("name"), &[])
                .unwrap()
                .unwrap();

            // Execute method
            let ret = vm
                .call_method(mc, Value::Object(obj), "name", vec![])
                .unwrap();
            assert_eq!(to_spec(ret), ValueSpec::String("Point".to_string()));
        });
    }

    #[test]
    fn test_execute_block_helper() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let vm = VmState::new(mc, VmOptions::default());
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Build a block that adds two arguments (a, b) and returns self + a + b
            let block = gc!(
                mc,
                Block {
                    source_info: None,
                    name: Some("test_block".to_string()),
                    is_nested_block: false,
                    param_syms: crate::value::intern_param_syms(&vec!["a".to_string(), "b".to_string()]),
                    param_types: vec!["Object".to_string(), "Object".to_string()],
                    bytecode: SharedBytecode::from(vec![
                        Instruction::LoadLocal(Symbol::intern("self")),
                        Instruction::LoadLocal(Symbol::intern("a")),
                        Instruction::Send(Symbol::intern("+"), 1),
                        Instruction::LoadLocal(Symbol::intern("b")),
                        Instruction::Send(Symbol::intern("+"), 1),
                        Instruction::Return,
                    ]),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                }
            );

            // Register standard native functions we need (+ operator)
            let native_val = vm.new_native_method(mc, "+".to_string(), NativeFunc(native_add), None);
            vm.globals
                .borrow_mut(mc)
                .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

            // Execute block with args [10, 20] and self = 100
            let self_val = vm.new_int(mc, 100);
            let arg1 = vm.new_int(mc, 10);
            let arg2 = vm.new_int(mc, 20);

            let res = vm
                .execute_block(mc, block, vec![arg1, arg2], Some(self_val))
                .unwrap();

            assert_eq!(to_spec(res), ValueSpec::Int(130));

            // Execute block without self: a + b
            let block2 = gc!(
                mc,
                Block {
                    source_info: None,
                    name: Some("test_block_no_self".to_string()),
                    is_nested_block: false,
                    param_syms: crate::value::intern_param_syms(&vec!["a".to_string(), "b".to_string()]),
                    param_types: vec!["Object".to_string(), "Object".to_string()],
                    bytecode: SharedBytecode::from(vec![
                        Instruction::LoadLocal(Symbol::intern("a")),
                        Instruction::LoadLocal(Symbol::intern("b")),
                        Instruction::Send(Symbol::intern("+"), 1),
                        Instruction::Return,
                    ]),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                }
            );

            let res2 = vm
                .execute_block(mc, block2, vec![arg1, arg2], None)
                .unwrap();

            assert_eq!(to_spec(res2), ValueSpec::Int(30));
        });
    }

    #[test]
    fn test_cannot_redefine_existing_class() {
        run_test_steps(
            vec![Instruction::DefineClass {
                name: NamespacedName::new(Vec::new(), "Object".to_string()),
                parent_name: None,
                instance_vars: Vec::new(),
            }],
            |vm, mc| {
                let res = vm.step(mc);
                assert!(res.is_err());
                let err_msg = format!("{}", res.err().unwrap());
                assert!(
                    err_msg.contains("Cannot redefine class [/]Object because it already exists")
                );
            },
        );
    }

    #[test]
    fn test_cannot_extend_non_existent_class() {
        run_test_steps(
            vec![
                Instruction::Push(Constant::Nil),
                Instruction::Push(Constant::Block(StaticBlock {
                    source_info: None,
                    name: Some("ext_block".to_string()),
                    is_nested_block: false,
                    param_syms: Vec::new(),
                    param_types: Vec::new(),
                    bytecode: SharedBytecode::from(vec![
                        Instruction::Push(Constant::Nil),
                        Instruction::Return,
                    ]),
                    decl_block: None,
                    source_map: SharedSourceMap::from(Vec::new()),
                })),
                Instruction::ExecuteBlockWithSelf,
            ],
            |vm, mc| {
                let res = vm.step(mc);
                assert!(res.is_ok());
                let res = vm.step(mc);
                assert!(res.is_ok());
                let res = vm.step(mc);
                assert!(res.is_err());
                let err_msg = format!("{}", res.err().unwrap());
                assert!(err_msg.contains("Cannot extend nil or non-existent class/object"));
            },
        );
    }

    #[test]
    fn test_short_circuit_and_or() {
        // Test: false && (panic!)
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(false)),
                Instruction::Dup,
                Instruction::ElseJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(99)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push false
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // ElseJump (should jump to end, i.e., index 5)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
                assert_eq!(vm.frames[0].ip, 5);
            },
        );

        // Test: true && 42
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Dup,
                Instruction::ElseJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(42)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push true
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // ElseJump (should not jump, IP becomes 3)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
                assert_eq!(vm.frames[0].ip, 3);
                vm.step(mc).unwrap(); // Pop
                assert_eq!(vm.stack.len(), 0);
                vm.step(mc).unwrap(); // Push 42
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );

        // Test: true || (panic!)
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(true)),
                Instruction::Dup,
                Instruction::IfJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(99)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push true
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // IfJump (should jump to end, i.e. index 5)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
                assert_eq!(vm.frames[0].ip, 5);
            },
        );

        // Test: false || 42
        run_test_steps(
            vec![
                Instruction::Push(Constant::Bool(false)),
                Instruction::Dup,
                Instruction::IfJump(3),
                Instruction::Pop,
                Instruction::Push(Constant::Int(42)),
            ],
            |vm, mc| {
                vm.step(mc).unwrap(); // Push false
                vm.step(mc).unwrap(); // Dup
                vm.step(mc).unwrap(); // IfJump (should not jump, IP becomes 3)
                assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
                assert_eq!(vm.frames[0].ip, 3);
                vm.step(mc).unwrap(); // Pop
                assert_eq!(vm.stack.len(), 0);
                vm.step(mc).unwrap(); // Push 42
                assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            },
        );
    }

    #[test]
    fn test_error_annotation_and_display() {
        use crate::compiler::Compiler;
        use crate::parser::parse_quoin_string;

        let code = "1.foo;";
        let ast = parse_quoin_string(code);
        let mut compiler = Compiler::new();
        let compiled = compiler
            .compile_program(match &ast.value {
                NodeValue::Program(p) => p,
                _ => unreachable!(),
            })
            .unwrap();

        let mut arena =
            Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, VmOptions::default()));

        arena.mutate_root(|mc, vm| {
            let decl_block = compiled.decl_block.as_ref().map(|db| {
                gc!(
                    mc,
                    Block {
                        source_info: db.source_info.clone(),
                        name: db.name.clone(),
                        is_nested_block: db.is_nested_block,
                        param_syms: db.param_syms.clone(),
                        param_types: db.param_types.clone(),
                        bytecode: db.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        decl_block: None,
                        source_map: db.source_map.clone(),
                    }
                )
            });
            let block = gc!(
                mc,
                Block {
                    source_info: compiled.source_info.clone(),
                    name: compiled.name.clone(),
                    is_nested_block: compiled.is_nested_block,
                    param_syms: compiled.param_syms.clone(),
                    param_types: compiled.param_types.clone(),
                    bytecode: compiled.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block,
                    source_map: compiled.source_map.clone(),
                }
            );
            vm.start_block(mc, block, Vec::new(), None, None);

            // Run until error. It should fail because Integer/Nil does not have 'foo' method.
            let mut err = None;
            loop {
                match vm.step(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(_) => break,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }

            let err = err.expect("Expected execution error");
            let err_str = err.to_string();

            // Check that the error message displays the source information
            assert!(err_str.contains("at <string>:1:1"));
            assert!(err_str.contains("1.foo"));
        });
    }

    #[test]
    fn test_error_annotation_with_color() {
        use crate::compiler::Compiler;
        use crate::parser::parse_quoin_string;

        let code = "1.foo;";
        let ast = parse_quoin_string(code);
        let mut compiler = Compiler::new();
        let compiled = compiler
            .compile_program(match &ast.value {
                NodeValue::Program(p) => p,
                _ => unreachable!(),
            })
            .unwrap();

        let mut options = VmOptions::default();
        options.supports_color = true;

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, options));

        arena.mutate_root(|mc, vm| {
            let decl_block = compiled.decl_block.as_ref().map(|db| {
                gc!(
                    mc,
                    Block {
                        source_info: db.source_info.clone(),
                        name: db.name.clone(),
                        is_nested_block: db.is_nested_block,
                        param_syms: db.param_syms.clone(),
                        param_types: db.param_types.clone(),
                        bytecode: db.bytecode.clone(),
                        parent_env: None,
                        enclosing_method_id: None,
                        decl_block: None,
                        source_map: db.source_map.clone(),
                    }
                )
            });
            let block = gc!(
                mc,
                Block {
                    source_info: compiled.source_info.clone(),
                    name: compiled.name.clone(),
                    is_nested_block: compiled.is_nested_block,
                    param_syms: compiled.param_syms.clone(),
                    param_types: compiled.param_types.clone(),
                    bytecode: compiled.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block,
                    source_map: compiled.source_map.clone(),
                }
            );
            vm.start_block(mc, block, Vec::new(), None, None);

            // Run until error.
            let mut err = None;
            loop {
                match vm.step(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(_) => break,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }

            let err = err.expect("Expected execution error");
            let err_str = err.to_string();

            // Check that the error message contains the ANSI escape codes
            // Selector is purple (38;2;171;130;255)
            // Filename text has colon gray (38;2;128;128;128)
            // Numbers are light blue (38;2;0;191;255)
            assert!(err_str.contains("\x1b[38;2;171;130;255mfoo\x1b[0;00;22;39;49m"));
            assert!(err_str.contains("\x1b[38;2;0;191;255m1\x1b[0;00;22;39;49m"));
        });
    }

    #[test]
    fn test_error_annotation_with_console_width() {
        use crate::compiler::Compiler;
        use crate::parser::parse_quoin_string;

        let code = "1.foo;";
        let ast = parse_quoin_string(code);
        let mut compiler = Compiler::new();
        let compiled = compiler
            .compile_program(match &ast.value {
                NodeValue::Program(p) => p,
                _ => unreachable!(),
            })
            .unwrap();

        let mut options = VmOptions::default();
        options.console_width = Some(120);

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, options));

        arena.mutate_root(|mc, vm| {
            let block = gc!(
                mc,
                Block {
                    source_info: compiled.source_info.clone(),
                    name: compiled.name.clone(),
                    is_nested_block: compiled.is_nested_block,
                    param_syms: compiled.param_syms.clone(),
                    param_types: compiled.param_types.clone(),
                    bytecode: compiled.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: compiled.source_map.clone(),
                }
            );
            vm.start_block(mc, block, Vec::new(), None, None);

            // Run until error.
            let mut err = None;
            loop {
                match vm.step(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(_) => break,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }

            let err = err.expect("Expected execution error");
            assert!(matches!(err, QuoinError::WithSourceInfo { .. }));
        });
    }

    #[test]
    fn test_vm_to_s() {
        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, VmOptions::default());
            vm.register_native_class(mc, build_object_class());
            vm.register_native_class(mc, build_class_class());
            vm.register_native_class(mc, build_boolean_class());
            vm.register_native_class(mc, build_block_class());
            vm.register_native_class(mc, build_list_class());
            vm.register_native_class(mc, build_double_class());
            vm.register_native_class(mc, build_integer_class());
            vm.register_native_class(mc, build_string_class());
            vm.register_native_class(mc, build_nil_class());
            vm.register_native_class(mc, build_map_class());
            vm.register_native_class(mc, build_key_value_pair_class());
            vm.register_native_class(mc, build_regex_class());
            for t in ["Method"] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Test 1: Value::Class Display Output
            let string_class = vm.get_builtin_class("String");
            let class_val = Value::Class(string_class);
            let result = vm.to_s(mc, class_val).unwrap();
            assert_eq!(
                to_spec(result),
                ValueSpec::String("class String".to_string())
            );

            // Test 2: Value::ClassMeta Display Output
            let class_meta_val = Value::ClassMeta(string_class);
            let result = vm.to_s(mc, class_meta_val).unwrap();
            assert_eq!(
                to_spec(result),
                ValueSpec::String("class String meta".to_string())
            );

            // Test 3: Value::Object (Int / String / Nil / Bool)
            let int_val = vm.new_int(mc, 42);
            let result = vm.to_s(mc, int_val).unwrap();
            assert_eq!(to_spec(result), ValueSpec::String("42".to_string()));

            let bool_val = vm.new_bool(mc, true);
            let result = vm.to_s(mc, bool_val).unwrap();
            assert_eq!(to_spec(result), ValueSpec::String("true".to_string()));

            let nil_val = vm.new_nil(mc);
            let result = vm.to_s(mc, nil_val).unwrap();
            assert_eq!(to_spec(result), ValueSpec::String("nil".to_string()));

            let string_val = vm.new_string(mc, "hello".to_string());
            let result = vm.to_s(mc, string_val).unwrap();
            assert_eq!(to_spec(result), ValueSpec::String("hello".to_string()));
        });
    }

    #[test]
    fn test_vm_options_at_runtime() {
        let options = VmOptions {
            arguments: vec!["foo".to_string(), "bar".to_string()],
            supports_color: true,
            console_width: None,
        };

        let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
            let mut vm = VmState::new(mc, options);
            vm.register_native_class(mc, build_object_class());
            vm.register_native_class(mc, build_class_class());
            vm.register_native_class(mc, build_boolean_class());
            vm.register_native_class(mc, build_block_class());
            vm.register_native_class(mc, build_list_class());
            vm.register_native_class(mc, build_double_class());
            vm.register_native_class(mc, build_integer_class());
            vm.register_native_class(mc, build_string_class());
            vm.register_native_class(mc, build_nil_class());
            vm.register_native_class(mc, build_map_class());
            vm.register_native_class(mc, build_key_value_pair_class());
            vm.register_native_class(mc, build_regex_class());
            vm.register_native_class(mc, crate::runtime::runtime::build_runtime_class());
            for t in ["Method"] {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }
            vm
        });

        arena.mutate_root(|mc, vm| {
            // Check that Runtime.arguments returns the list ["foo", "bar"]
            let runtime_class = vm.get_builtin_class("Runtime");
            let args_val = vm
                .call_method(mc, Value::Class(runtime_class), "arguments", vec![])
                .unwrap();

            // args_val should be a List of strings
            let count_val = vm.call_method(mc, args_val, "count", vec![]).unwrap();
            assert_eq!(to_spec(count_val), ValueSpec::Int(2));

            let idx0 = vm.new_int(mc, 0);
            let arg0 = vm.call_method(mc, args_val, "at:", vec![idx0]).unwrap();
            assert_eq!(to_spec(arg0), ValueSpec::String("foo".to_string()));

            let idx1 = vm.new_int(mc, 1);
            let arg1 = vm.call_method(mc, args_val, "at:", vec![idx1]).unwrap();
            assert_eq!(to_spec(arg1), ValueSpec::String("bar".to_string()));

            // Check options method
            let opts_val = vm
                .call_method(mc, Value::Class(runtime_class), "options", vec![])
                .unwrap();
            // opts_val should be a Map
            let key = vm.new_string(mc, "arguments".to_string());
            let mapped_args = vm.call_method(mc, opts_val, "at:", vec![key]).unwrap();

            let mapped_count = vm.call_method(mc, mapped_args, "count", vec![]).unwrap();
            assert_eq!(to_spec(mapped_count), ValueSpec::Int(2));

            // Check supportsColor method
            let supports_color_val = vm
                .call_method(mc, Value::Class(runtime_class), "supportsColor", vec![])
                .unwrap();
            assert_eq!(to_spec(supports_color_val), ValueSpec::Bool(true));

            // Check options map has supports_color
            let key_color = vm.new_string(mc, "supports_color".to_string());
            let mapped_color = vm
                .call_method(mc, opts_val, "at:", vec![key_color])
                .unwrap();
            assert_eq!(to_spec(mapped_color), ValueSpec::Bool(true));
        });
    }
}
