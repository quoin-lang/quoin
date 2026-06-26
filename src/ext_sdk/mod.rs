//! ext-sdk — the curated host surface that native (builtin) classes are written
//! against, instead of reaching into [`VmState`] directly.
//!
//! [`Host`] is a **dyn-safe** operation surface; native methods registered via the
//! `sdk_*` builders on `NativeClassBuilder` receive `&mut dyn Host` rather than
//! `&mut VmState`, so the compiler enforces that a builtin only touches the
//! curated API.
//!
//! ## No gc_arena in the SDK signatures
//!
//! The surface deliberately keeps gc_arena types out of method signatures:
//!
//! * **`Mutation` (`mc`)** is captured once per native call by [`HostCtx`] — the
//!   short-lived `(vm, mc)` bundle that actually implements `Host` — so SDK
//!   methods never thread it. Write barriers still fire inside the delegations.
//! * **`Gc` / `RefLock`** never appear: class references are the opaque
//!   [`ClassHandle`]; everything else is a [`Value`].
//!
//! What remains is `Value<'gc>` (an opaque enum, built/inspected via `Host` +
//! the `arg!`/`recv!` macros) and the `'gc` brand itself — the last gc_arena
//! thing, removable only by the handle-projection step (the out-of-process tier;
//! see `docs/FUTURE_EXT_ARCH.md` guardrail #6).
//!
//! Generic conveniences that can't live on a `dyn` trait (native-state
//! construction) are provided by the blanket [`HostExt`] trait, implemented for
//! every `Host` — including `dyn Host` — so a `&mut dyn Host` body gets them by
//! importing the prelude.
//!
//! This is Tier 0: an in-tree module today, to be extracted into the
//! `quoin-ext-sdk` crate once every builtin is migrated off `VmState`.

use gc_arena::lock::RefLock;
use gc_arena::{Gc, Mutation};
use indexmap::IndexMap;

use crate::error::QuoinError;
use crate::value::{AnyCollect, Class, NativeCall, ObjectPayload, Value};
use crate::vm::{OutputChunk, StdStream, VmOptions, VmState};

/// An opaque reference to a class. Wraps the underlying GC pointer so SDK authors
/// get class handles from [`Host`] and pass them back without ever naming `Gc`.
#[derive(Clone, Copy)]
pub struct ClassHandle<'gc>(Gc<'gc, RefLock<Class<'gc>>>);

impl<'gc> ClassHandle<'gc> {
    /// Wrap a raw class pointer. Crate-internal: the bridge ([`HostCtx`]) and the
    /// VM construct handles; SDK authors only ever receive them.
    pub(crate) fn from_raw(raw: Gc<'gc, RefLock<Class<'gc>>>) -> Self {
        Self(raw)
    }

    /// The underlying GC pointer, for the bridge to call back into `VmState`.
    pub(crate) fn raw(self) -> Gc<'gc, RefLock<Class<'gc>>> {
        self.0
    }

    /// Identity comparison (two handles to the same class).
    pub fn same(self, other: ClassHandle<'gc>) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

/// A native method written against the SDK surface. Mirrors the legacy
/// `NativeFn`/`LegacyNativeFn` fn-pointer, but takes `&mut dyn Host` (no `mc`)
/// instead of `&mut VmState` + `&Mutation`.
pub type SdkFn =
    for<'a> fn(&mut dyn Host<'a>, Value<'a>, Vec<Value<'a>>) -> Result<Value<'a>, QuoinError>;

/// The curated host operation surface available to native (builtin) classes.
///
/// `dyn`-safe by construction: no generic methods (those live on [`HostExt`]) and
/// no associated types, so `&mut dyn Host` is the argument every SDK method takes.
/// No method takes a `&Mutation` — it's captured by the implementor ([`HostCtx`]).
///
/// The surface grows as builtin classes migrate onto it — async/socket/channel/
/// fiber operations are added in their respective migration batches, against real
/// call sites, rather than guessed up front.
pub trait Host<'gc> {
    // --- value constructors -------------------------------------------------
    fn new_nil(&self) -> Value<'gc>;
    fn new_bool(&self, b: bool) -> Value<'gc>;
    fn new_int(&self, i: i64) -> Value<'gc>;
    fn new_double(&self, f: f64) -> Value<'gc>;
    fn new_string(&self, s: String) -> Value<'gc>;
    fn new_bytes(&self, bytes: Vec<u8>) -> Value<'gc>;
    fn new_symbol(&self, name: String) -> Value<'gc>;
    fn new_list(&self, list: Vec<Value<'gc>>) -> Value<'gc>;
    fn new_map(&self, map: IndexMap<String, Value<'gc>>) -> Value<'gc>;
    /// A fresh instance of `class` (fields nil-initialized), wrapped as a `Value`.
    fn new_object(&self, class: ClassHandle<'gc>) -> Value<'gc>;
    /// The dyn-safe core of native-state construction. Prefer the generic
    /// [`HostExt::new_native_state`], which boxes for you.
    fn new_native_state_boxed(
        &self,
        class: ClassHandle<'gc>,
        state: Box<dyn AnyCollect>,
    ) -> Value<'gc>;

    // --- dispatch -----------------------------------------------------------
    fn call_method(
        &mut self,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError>;
    /// Run `block` (a `Value` holding a block; errors otherwise) with `args` and an
    /// optional `self`.
    fn execute_block(
        &mut self,
        block: Value<'gc>,
        args: Vec<Value<'gc>>,
        self_val: Option<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError>;

    // --- args / config / output --------------------------------------------
    /// The GC-rooted receiver+args of the in-flight native call, kept live across
    /// nested calls that may collect (re-read after calling back into the VM).
    fn active_native_args(&self) -> &[NativeCall<'gc>];
    fn options(&self) -> &VmOptions;
    fn write_std(&mut self, stream: StdStream, bytes: &[u8]) -> std::io::Result<()>;
    fn take_program_output(&mut self) -> Vec<OutputChunk>;

    // --- class registry / type queries -------------------------------------
    fn get_or_create_builtin_class(&self, name: &str) -> ClassHandle<'gc>;
    fn get_builtin_class(&self, name: &str) -> ClassHandle<'gc>;
    fn is_instance_of(&self, val: Value<'gc>, class: ClassHandle<'gc>) -> bool;
    fn is_subclass_of(&self, sub: ClassHandle<'gc>, sup: ClassHandle<'gc>) -> bool;
    fn lookup_in_class_hierarchy(
        &self,
        class: ClassHandle<'gc>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>>;
    fn class_of(&self, receiver: Value<'gc>) -> Option<ClassHandle<'gc>>;
    fn value_matches_type(&self, val: Value<'gc>, hint: &str) -> bool;
}

/// Generic conveniences over [`Host`] that can't be `dyn`-safe methods. Blanket-
/// implemented for every `Host` (the `?Sized` bound includes `dyn Host`), so a
/// native method body with `&mut dyn Host` gets them for free.
pub trait HostExt<'gc>: Host<'gc> {
    /// Box `state` and wrap it as an opaque native-state instance of `class`.
    fn new_native_state<T: AnyCollect + 'static>(
        &self,
        class: ClassHandle<'gc>,
        state: T,
    ) -> Value<'gc> {
        self.new_native_state_boxed(class, Box::new(state))
    }
}

impl<'gc, H: Host<'gc> + ?Sized> HostExt<'gc> for H {}

/// The per-native-call bridge that implements [`Host`]: a short-lived bundle of
/// the VM and the live `Mutation` token. Built once at the native dispatch site
/// (`Callable::call`) and handed to the `SdkFn` as `&mut dyn Host`, so SDK methods
/// never see `mc`. Stays VM-side; the abstract `Host`/`ClassHandle` surface is
/// what eventually moves to the `quoin-ext-sdk` crate.
pub struct HostCtx<'a, 'gc> {
    vm: &'a mut VmState<'gc>,
    mc: &'a Mutation<'gc>,
}

impl<'a, 'gc> HostCtx<'a, 'gc> {
    pub fn new(vm: &'a mut VmState<'gc>, mc: &'a Mutation<'gc>) -> Self {
        Self { vm, mc }
    }
}

fn not_a_block(v: Value<'_>) -> QuoinError {
    QuoinError::TypeError {
        expected: "Block".to_string(),
        got: v.type_name().to_string(),
        msg: "execute_block expects a Block".to_string(),
    }
}

/// Every method is a thin delegation to the inherent `VmState` op of the same
/// role, supplying the captured `mc`.
impl<'gc> Host<'gc> for HostCtx<'_, 'gc> {
    fn new_nil(&self) -> Value<'gc> {
        self.vm.new_nil(self.mc)
    }
    fn new_bool(&self, b: bool) -> Value<'gc> {
        self.vm.new_bool(self.mc, b)
    }
    fn new_int(&self, i: i64) -> Value<'gc> {
        self.vm.new_int(self.mc, i)
    }
    fn new_double(&self, f: f64) -> Value<'gc> {
        self.vm.new_double(self.mc, f)
    }
    fn new_string(&self, s: String) -> Value<'gc> {
        self.vm.new_string(self.mc, s)
    }
    fn new_bytes(&self, bytes: Vec<u8>) -> Value<'gc> {
        self.vm.new_bytes(self.mc, bytes)
    }
    fn new_symbol(&self, name: String) -> Value<'gc> {
        self.vm.new_symbol(self.mc, name)
    }
    fn new_list(&self, list: Vec<Value<'gc>>) -> Value<'gc> {
        self.vm.new_list(self.mc, list)
    }
    fn new_map(&self, map: IndexMap<String, Value<'gc>>) -> Value<'gc> {
        self.vm.new_map(self.mc, map)
    }
    fn new_object(&self, class: ClassHandle<'gc>) -> Value<'gc> {
        Value::Object(self.vm.new_object(self.mc, class.raw()))
    }
    fn new_native_state_boxed(
        &self,
        class: ClassHandle<'gc>,
        state: Box<dyn AnyCollect>,
    ) -> Value<'gc> {
        self.vm.new_native_state_boxed(self.mc, class.raw(), state)
    }

    fn call_method(
        &mut self,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        self.vm.call_method(self.mc, receiver, selector, args)
    }
    fn execute_block(
        &mut self,
        block: Value<'gc>,
        args: Vec<Value<'gc>>,
        self_val: Option<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let Value::Object(obj) = block else {
            return Err(not_a_block(block));
        };
        let blk = match &obj.borrow().payload {
            ObjectPayload::Block(b) => *b,
            _ => return Err(not_a_block(block)),
        };
        self.vm.execute_block(self.mc, blk, args, self_val)
    }

    fn active_native_args(&self) -> &[NativeCall<'gc>] {
        &self.vm.active_native_args
    }
    fn options(&self) -> &VmOptions {
        &self.vm.options
    }
    fn write_std(&mut self, stream: StdStream, bytes: &[u8]) -> std::io::Result<()> {
        self.vm.write_std(stream, bytes)
    }
    fn take_program_output(&mut self) -> Vec<OutputChunk> {
        self.vm.take_program_output()
    }

    fn get_or_create_builtin_class(&self, name: &str) -> ClassHandle<'gc> {
        ClassHandle::from_raw(self.vm.get_or_create_builtin_class(self.mc, name))
    }
    fn get_builtin_class(&self, name: &str) -> ClassHandle<'gc> {
        ClassHandle::from_raw(self.vm.get_builtin_class(name))
    }
    fn is_instance_of(&self, val: Value<'gc>, class: ClassHandle<'gc>) -> bool {
        self.vm.is_instance_of(val, class.raw())
    }
    fn is_subclass_of(&self, sub: ClassHandle<'gc>, sup: ClassHandle<'gc>) -> bool {
        self.vm.is_subclass_of_clz(sub.raw(), sup.raw())
    }
    fn lookup_in_class_hierarchy(
        &self,
        class: ClassHandle<'gc>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        self.vm
            .lookup_in_class_hierarchy(class.raw(), selector, class_side)
    }
    fn class_of(&self, receiver: Value<'gc>) -> Option<ClassHandle<'gc>> {
        self.vm
            .get_class_for_lookup(receiver)
            .map(ClassHandle::from_raw)
    }
    fn value_matches_type(&self, val: Value<'gc>, hint: &str) -> bool {
        self.vm.value_matches_type(val, hint)
    }
}

/// Re-exports for native-class authors: `use crate::ext_sdk::prelude::*;`.
pub mod prelude {
    pub use super::{ClassHandle, Host, HostExt, SdkFn};
}
